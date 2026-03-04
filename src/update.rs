use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::cpio::{CpioArchive, CpioEntry};

/// Build a CPIO archive from a directory tree.
pub fn build_archive_from_dir(source: &Path) -> Result<CpioArchive> {
    let mut entries = Vec::new();
    let mut ino: u32 = 0;

    for result in WalkDir::new(source).follow_links(false).sort_by_file_name() {
        let dir_entry = result.with_context(|| format!("walking {}", source.display()))?;
        let path = dir_entry.path();
        let rel_path = path
            .strip_prefix(source)
            .unwrap_or(path);

        let name = if rel_path == Path::new("") {
            ".".to_string()
        } else {
            rel_path.to_string_lossy().to_string()
        };

        let meta = dir_entry
            .metadata()
            .with_context(|| format!("reading metadata for {}", path.display()))?;

        let file_type = dir_entry.file_type();
        let mut data = Vec::new();
        let mode;

        if file_type.is_dir() {
            mode = 0o040000 | (meta.mode() & 0o7777);
        } else if file_type.is_symlink() {
            let target = std::fs::read_link(path)
                .with_context(|| format!("reading symlink {}", path.display()))?;
            data = target.to_string_lossy().as_bytes().to_vec();
            mode = 0o120000 | (meta.mode() & 0o7777);
        } else if file_type.is_file() {
            data = std::fs::read(path)
                .with_context(|| format!("reading file {}", path.display()))?;
            mode = 0o100000 | (meta.mode() & 0o7777);
        } else {
            eprintln!("skipping unsupported file type: {}", path.display());
            continue;
        };

        entries.push(CpioEntry {
            ino,
            mode,
            uid: meta.uid(),
            gid: meta.gid(),
            nlink: meta.nlink() as u32,
            mtime: meta.mtime() as u32,
            devmajor: 0,
            devminor: 0,
            rdevmajor: 0,
            rdevminor: 0,
            name,
            data,
        });

        ino += 1;
    }

    Ok(CpioArchive { entries })
}

/// Reassemble an initramfs file, replacing one segment.
/// Returns the new file contents.
pub fn reassemble(
    segments: &[crate::segment::RawSegment],
    index: usize,
    new_segment_data: Vec<u8>,
) -> Vec<u8> {
    let mut out = Vec::new();

    for (i, seg) in segments.iter().enumerate() {
        if i == index {
            out.extend_from_slice(&new_segment_data);
        } else {
            out.extend_from_slice(&seg.data);
        }

        // Pad with nulls to 512-byte boundary between segments
        if i + 1 < segments.len() {
            let remainder = out.len() % 512;
            if remainder != 0 {
                out.resize(out.len() + (512 - remainder), 0);
            }
        }
    }

    out
}
