use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cpio::CpioArchive;

/// Check if the current process can change file ownership.
/// Returns true if running as effective UID 0 or has CAP_CHOWN.
fn can_chown() -> bool {
    let status = match std::fs::read_to_string("/proc/self/status") {
        Ok(s) => s,
        Err(_) => return false,
    };

    for line in status.lines() {
        // Uid line: Real, Effective, Saved, FS — check effective (2nd field)
        if let Some(rest) = line.strip_prefix("Uid:") {
            if let Some(euid_str) = rest.split_whitespace().nth(1) {
                if euid_str == "0" {
                    return true;
                }
            }
        }
        // CapEff: hex bitmask — CAP_CHOWN is bit 0
        if let Some(rest) = line.strip_prefix("CapEff:") {
            if let Ok(mask) = u64::from_str_radix(rest.trim(), 16) {
                if mask & 1 != 0 {
                    return true;
                }
            }
        }
    }

    false
}

/// Extract all entries from a CPIO archive to the given directory.
pub fn extract_archive(archive: &CpioArchive, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)
        .with_context(|| format!("failed to create destination: {}", dest.display()))?;

    let chown_ok = can_chown();
    if !chown_ok {
        let args: Vec<String> = std::env::args().collect();
        let cmd = args.join(" ");
        eprintln!(
            "warning: not running as root and missing CAP_CHOWN; file ownership will not be preserved"
        );
        eprintln!("hint: re-run with: setpriv --ambient-caps +chown -- {cmd}");
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
