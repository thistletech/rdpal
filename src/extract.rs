use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cpio::CpioArchive;

/// Parsed process status from /proc/self/status.
struct ProcStatus {
    real_uid: u32,
    effective_uid: u32,
    real_gid: u32,
    cap_eff: u64,
}

/// Parse /proc/self/status for uid, gid, and effective capabilities.
fn proc_status() -> Option<ProcStatus> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let mut real_uid = None;
    let mut effective_uid = None;
    let mut real_gid = None;
    let mut cap_eff = None;

    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let mut fields = rest.split_whitespace();
            real_uid = fields.next().and_then(|s| s.parse().ok());
            effective_uid = fields.next().and_then(|s| s.parse().ok());
        }
        if let Some(rest) = line.strip_prefix("Gid:") {
            real_gid = rest.split_whitespace().next().and_then(|s| s.parse().ok());
        }
        if let Some(rest) = line.strip_prefix("CapEff:") {
            cap_eff = u64::from_str_radix(rest.trim(), 16).ok();
        }
    }

    Some(ProcStatus {
        real_uid: real_uid?,
        effective_uid: effective_uid?,
        real_gid: real_gid?,
        cap_eff: cap_eff.unwrap_or(0),
    })
}

impl ProcStatus {
    /// Whether the process can change file ownership (root or CAP_CHOWN, bit 0).
    fn can_chown(&self) -> bool {
        self.effective_uid == 0 || (self.cap_eff & 1) != 0
    }

    /// Whether the process can create device nodes (root or CAP_MKNOD, bit 27).
    fn can_mknod(&self) -> bool {
        self.effective_uid == 0 || (self.cap_eff & (1 << 27)) != 0
    }
}

/// Create a special filesystem node (device, FIFO, or socket) at the given path.
fn create_special_node(
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

/// Escape arguments for embedding in a shell command string.
fn shell_escape_args(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.contains(|c: char| c.is_whitespace() || "\"'\\$`!#&|;(){}".contains(c)) {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract all entries from a CPIO archive to the given directory.
pub fn extract_archive(archive: &CpioArchive, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)
        .with_context(|| format!("failed to create destination: {}", dest.display()))?;

    let status = proc_status();
    let chown_ok = status.as_ref().is_some_and(|s| s.can_chown());
    let mknod_ok = status.as_ref().is_some_and(|s| s.can_mknod());

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
        let cmd = shell_escape_args(&args);
        let inh_caps = missing_caps.iter().map(|c| format!("+{c}")).collect::<Vec<_>>().join(",");
        let amb_caps = &inh_caps;
        let (uid, gid) = status
            .as_ref()
            .map(|s| (s.real_uid, s.real_gid))
            .unwrap_or((1000, 1000));

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
        eprintln!(
            "hint: re-run with: sudo -E setpriv --reuid={uid} --regid={gid} --init-groups --inh-caps {inh_caps} --ambient-caps {amb_caps} -- env PATH=\"$PATH\" bash -c \"{cmd}\""
        );
    }

    // Deferred directory chown: collect (path, uid, gid) and apply after all
    // entries are extracted, deepest-first, so that chowning a parent to root
    // doesn't prevent writing into it.
    let mut dir_chowns: Vec<(std::path::PathBuf, u32, u32)> = Vec::new();

    for entry in &archive.entries {
        let target = dest.join(&entry.name);

        if entry.is_dir() {
            std::fs::create_dir_all(&target)
                .with_context(|| format!("failed to create dir: {}", target.display()))?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(entry.permissions()))
                .ok(); // best effort
            if chown_ok {
                dir_chowns.push((target, entry.uid, entry.gid));
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
                create_special_node(&target, entry.mode, entry.rdevmajor, entry.rdevminor)
                    .with_context(|| {
                        format!(
                            "failed to create device node: {} ({}:{})",
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
        } else if entry.is_fifo() || entry.is_socket() {
            create_special_node(&target, entry.mode, 0, 0)
                .with_context(|| {
                    format!(
                        "failed to create {}: {}",
                        if entry.is_fifo() { "FIFO" } else { "socket" },
                        target.display()
                    )
                })?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(entry.permissions()))
                .ok(); // best effort — override umask
            if chown_ok {
                std::os::unix::fs::chown(&target, Some(entry.uid), Some(entry.gid)).ok();
            }
        } else {
            eprintln!(
                "skipping unsupported entry type '{}': {}",
                entry.file_type_char(),
                entry.name
            );
        }
    }

    // Apply deferred directory chowns deepest-first so that parent directories
    // remain writable while their children are being chowned.
    dir_chowns.sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));
    for (path, uid, gid) in &dir_chowns {
        std::os::unix::fs::chown(path, Some(*uid), Some(*gid)).ok();
    }

    Ok(())
}
