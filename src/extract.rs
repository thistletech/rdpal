use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cpio::CpioArchive;

/// Extract all entries from a CPIO archive to the given directory.
pub fn extract_archive(archive: &CpioArchive, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)
        .with_context(|| format!("failed to create destination: {}", dest.display()))?;

    for entry in &archive.entries {
        let target = dest.join(&entry.name);

        if entry.is_dir() {
            std::fs::create_dir_all(&target)
                .with_context(|| format!("failed to create dir: {}", target.display()))?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(entry.permissions()))
                .ok(); // best effort
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
        } else if entry.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&target, &entry.data)
                .with_context(|| format!("failed to write file: {}", target.display()))?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(entry.permissions()))
                .ok(); // best effort
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
