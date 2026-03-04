use anyhow::Result;

use crate::compression;
use crate::cpio;
use crate::segment::RawSegment;

/// Print information about all segments in a ramdisk file.
pub fn print_info(
    file_path: &str,
    file_size: usize,
    segments: &[RawSegment],
    verbose: bool,
) -> Result<()> {
    println!(
        "Ramdisk: {file_path} ({} bytes, {} archive{})",
        file_size,
        segments.len(),
        if segments.len() == 1 { "" } else { "s" }
    );
    println!();
    println!(
        "  {:>3}  {:>12}  {:>12}  {:>11}  {:>12}  {:>7}  First Entry",
        "#", "Offset", "Comp Size", "Compression", "Decomp Size", "Entries"
    );
    println!("  {}", "-".repeat(80));

    for (i, seg) in segments.iter().enumerate() {
        let decompressed = compression::decompress(&seg.data, seg.compression)?;
        let (archive, _) = cpio::parse_archive(&decompressed)?;

        let first_entry = archive
            .entries
            .first()
            .map(|e| e.name.as_str())
            .unwrap_or("");

        println!(
            "  {:>3}  {:>12}  {:>12}  {:>11}  {:>12}  {:>7}  {}",
            i,
            seg.offset,
            seg.data.len(),
            seg.compression.to_string(),
            decompressed.len(),
            archive.entries.len(),
            first_entry,
        );

        if verbose {
            println!(
                "         {:<6} {:>10}  {}",
                "Type", "Size", "Path"
            );
            println!("         {}", "-".repeat(60));
            for entry in &archive.entries {
                let kind = if entry.is_dir() {
                    "dir"
                } else if entry.is_symlink() {
                    "link"
                } else {
                    "file"
                };
                let name = if entry.is_symlink() {
                    let target = String::from_utf8_lossy(&entry.data);
                    format!("{} -> {}", entry.name, target)
                } else {
                    entry.name.clone()
                };
                println!(
                    "         {:<6} {:>10}  {}",
                    kind,
                    entry.data.len(),
                    name,
                );
            }
            println!();
        }
    }

    Ok(())
}
