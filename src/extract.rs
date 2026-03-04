use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cpio::CpioArchive;

/// Parse /proc/self/status and return (effective_uid, effective_capabilities).
fn proc_status() -> Option<(u32, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let mut euid = None;
    let mut cap_eff = None;

    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            euid = rest.split_whitespace().nth(1).and_then(|s| s.parse().ok());
        }
        if let Some(rest) = line.strip_prefix("CapEff:") {
            cap_eff = u64::from_str_radix(rest.trim(), 16).ok();
        }
    }

    Some((euid?, cap_eff.unwrap_or(0)))
}

/// Check if the current process can change file ownership.
/// Returns true if running as effective UID 0 or has CAP_CHOWN (bit 0).
fn can_chown() -> bool {
    match proc_status() {
        Some((euid, cap_eff)) => euid == 0 || (cap_eff & 1) != 0,
        None => false,
    }
}

/// Check if the current process can create device nodes.
/// Returns true if running as effective UID 0 or has CAP_MKNOD (bit 27).
fn can_mknod() -> bool {
    match proc_status() {
        Some((euid, cap_eff)) => euid == 0 || (cap_eff & (1 << 27)) != 0,
        None => false,
    }
}

/// Create a device node at the given path.
fn create_device_node(
    target: &Path,
    mode: u32,
    rdevmajor: u32,
    rdevminor: u32,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // Remove existing node if present
    let _ = std::fs::remove_file(target);

    let c_path = CString::new(target.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let dev = libc::makedev(rdevmajor, rdevminor);
    let ret = unsafe { libc::mknod(c_path.as_ptr(), mode, dev) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Extract all entries from a CPIO archive to the given directory.
pub fn extract_archive(archive: &CpioArchive, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)
        .with_context(|| format!("failed to create destination: {}", dest.display()))?;

    let chown_ok = can_chown();
    let mknod_ok = can_mknod();

    let has_devices = archive
        .entries
        .iter()
        .any(|e| e.is_block_device() || e.is_char_device());

    // Collect missing capabilities for warning
    let mut missing_caps: Vec<&str> = Vec::new();
    if !chown_ok {
        missing_caps.push("chown");
    }
    if !mknod_ok && has_devices {
        missing_caps.push("mknod");
    }

    if !missing_caps.is_empty() {
        let args: Vec<String> = std::env::args().collect();
        let cmd = args.join(" ");
        let caps = missing_caps.join(",+");
        if !chown_ok {
            eprintln!(
                "warning: not running as root and missing CAP_CHOWN; file ownership will not be preserved"
            );
        }
        if !mknod_ok && has_devices {
            eprintln!(
                "warning: not running as root and missing CAP_MKNOD; device nodes will be skipped"
            );
        }
        eprintln!("hint: re-run with: setpriv --ambient-caps +{caps} -- {cmd}");
    }

    for entry in &archive.entries {
        let target = dest.join(&entry.name);

        if entry.is_dir() {
            std::fs::create_dir_all(&target)
                .with_context(|| format!("failed to create dir: {}", target.display()))?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(entry.permissions()))
                .ok(); // best effort
            if chown_ok {
                std::os::unix::fs::chown(&target, Some(entry.uid), Some(entry.gid)).ok();
            }
        } else if entry.is_symlink() {
            let link_target =
                std::str::from_utf8(&entry.data).context("symlink target is not valid UTF-8")?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            // Remove existing file/symlink if present
            let _ = std::fs::remove_file(&target);
            std::os::unix::fs::symlink(link_target, &target)
                .with_context(|| format!("failed to create symlink: {}", target.display()))?;
            if chown_ok {
                std::os::unix::fs::lchown(&target, Some(entry.uid), Some(entry.gid)).ok();
            }
        } else if entry.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&target, &entry.data)
                .with_context(|| format!("failed to write file: {}", target.display()))?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(entry.permissions()))
                .ok(); // best effort
            if chown_ok {
                std::os::unix::fs::chown(&target, Some(entry.uid), Some(entry.gid)).ok();
            }
        } else if entry.is_block_device() || entry.is_char_device() {
            if mknod_ok {
                create_device_node(&target, entry.mode, entry.rdevmajor, entry.rdevminor)
                    .with_context(|| {
                        format!(
                            "failed to create device node: {} ({}:{}) ",
                            target.display(),
                            entry.rdevmajor,
                            entry.rdevminor
                        )
                    })?;
                if chown_ok {
                    std::os::unix::fs::chown(&target, Some(entry.uid), Some(entry.gid)).ok();
                }
            } else {
                eprintln!(
                    "skipping device node '{}': {} ({}:{})",
                    entry.file_type_char(),
                    entry.name,
                    entry.rdevmajor,
                    entry.rdevminor
                );
            }
        } else {
            eprintln!(
                "skipping unsupported entry type '{}': {}",
                entry.file_type_char(),
                entry.name
            );
        }
    }

    Ok(())
}
