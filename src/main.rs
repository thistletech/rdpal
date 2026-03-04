use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use rdpal::segment::Compression;
use rdpal::{compression, cpio, extract, info, segment, update};

#[derive(Parser)]
#[command(name = "rdpal", about = "Linux initramfs/ramdisk inspection and manipulation tool", long_about = format!("Linux initramfs/ramdisk inspection and manipulation tool\nVersion: {}", env!("CARGO_PKG_VERSION")), disable_version_flag = true)]
struct Cli {
    /// Path to the initramfs/ramdisk file
    file: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print information about all archives in the ramdisk
    Info,

    /// Extract a single CPIO archive to a directory
    Extract {
        /// 0-based index of the archive to extract
        #[arg(short, long)]
        index: usize,

        /// Destination directory
        #[arg(short, long)]
        dest: PathBuf,
    },

    /// Update a single CPIO archive from a directory
    Update {
        /// 0-based index of the archive to replace
        #[arg(short, long)]
        index: usize,

        /// Source directory to build the new archive from
        #[arg(short, long)]
        source: PathBuf,

        /// Compression to apply (none, gzip, bzip2, zstd)
        #[arg(short, long, default_value = "none")]
        compression: String,

        /// Output file path (defaults to overwriting the input file)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let data = std::fs::read(&cli.file)
        .with_context(|| format!("failed to read {}", cli.file.display()))?;
    let segments = segment::split_segments(&data)?;

    match cli.command {
        Command::Info => {
            let file_str = cli.file.to_string_lossy();
            info::print_info(&file_str, data.len(), &segments)?;
        }

        Command::Extract { index, dest } => {
            if index >= segments.len() {
                bail!(
                    "archive index {index} out of range (file has {} archive{})",
                    segments.len(),
                    if segments.len() == 1 { "" } else { "s" }
                );
            }

            let seg = &segments[index];
            let decompressed = compression::decompress(&seg.data, seg.compression)?;
            let (archive, _) = cpio::parse_archive(&decompressed)?;

            extract::extract_archive(&archive, &dest)?;
            println!(
                "Extracted archive {index} ({} entries) to {}",
                archive.entries.len(),
                dest.display()
            );
        }

        Command::Update {
            index,
            source,
            compression: comp_str,
            output,
        } => {
            if index >= segments.len() {
                bail!(
                    "archive index {index} out of range (file has {} archive{})",
                    segments.len(),
                    if segments.len() == 1 { "" } else { "s" }
                );
            }

            let comp: Compression = comp_str.parse()?;
            let archive = update::build_archive_from_dir(&source)?;
            let cpio_bytes = cpio::write_archive(&archive);
            let compressed = compression::compress(&cpio_bytes, comp)?;

            let new_data = update::reassemble(&segments, index, compressed);

            let out_path = output.unwrap_or_else(|| cli.file.clone());
            std::fs::write(&out_path, &new_data)
                .with_context(|| format!("failed to write {}", out_path.display()))?;

            println!(
                "Updated archive {index} ({} entries, {comp}) -> {}",
                archive.entries.len(),
                out_path.display()
            );
        }
    }

    Ok(())
}
