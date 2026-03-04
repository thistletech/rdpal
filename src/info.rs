use anyhow::Result;

use crate::compression;
use crate::cpio;
use crate::segment::RawSegment;

/// Print information about all segments in a ramdisk file.
pub fn print_info(file_path: &str, file_size: usize, segments: &[RawSegment]) -> Result<()> {
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
    }

    Ok(())
}
