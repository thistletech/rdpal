use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use rdpal::segment::Compression;
use rdpal::{compression, cpio, extract, info, segment, update};

#[derive(Parser)]
#[command(name = "rdpal", about = format!("Linux initramfs/ramdisk inspection and manipulation tool\nVersion: {}", env!("CARGO_PKG_VERSION")))]//, disable_version_flag = true)]
struct Cli {
    /// Path to the initramfs/ramdisk file
    file: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print information about all archives in the ramdisk
    Info {
        /// Print file paths and sizes for each entry in each archive
        #[arg(short, long)]
        verbose: bool,
    },

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

        /// Root path prefix for entries in the archive (default: ".")
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Add a new CPIO archive section to an existing initramfs file
    Add {
        /// Source directory to build the CPIO archive from
        #[arg(short, long)]
        source: PathBuf,

        /// Compression to apply (none, gzip, bzip2, zstd)
        #[arg(short, long, default_value = "none")]
        compression: String,

        /// Position to insert at (default: append to end)
        #[arg(short, long)]
        index: Option<usize>,

        /// Output file path (defaults to overwriting the input file)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Root path prefix for entries in the archive (default: ".")
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Create a new initramfs file from a directory
    Create {
        /// Source directory to build the CPIO archive from
        #[arg(short, long)]
        source: PathBuf,

        /// Compression to apply (none, gzip, bzip2, zstd)
        #[arg(short, long, default_value = "none")]
        compression: String,

        /// Overwrite the output file if it already exists
        #[arg(short, long)]
        force: bool,

        /// Root path prefix for entries in the archive (default: ".")
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Info { verbose } => {
            let data = std::fs::read(&cli.file)
                .with_context(|| format!("failed to read {}", cli.file.display()))?;
            let segments = segment::split_segments(&data)?;
            let file_str = cli.file.to_string_lossy();
            info::print_info(&file_str, data.len(), &segments, verbose)?;
        }

        Command::Extract { index, dest } => {
            let data = std::fs::read(&cli.file)
                .with_context(|| format!("failed to read {}", cli.file.display()))?;
            let segments = segment::split_segments(&data)?;

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
            root,
        } => {
            let data = std::fs::read(&cli.file)
                .with_context(|| format!("failed to read {}", cli.file.display()))?;
            let segments = segment::split_segments(&data)?;

            if index >= segments.len() {
                bail!(
                    "archive index {index} out of range (file has {} archive{})",
                    segments.len(),
                    if segments.len() == 1 { "" } else { "s" }
                );
            }

            let comp: Compression = comp_str.parse()?;
            let archive = update::build_archive_from_dir(&source, root.as_deref())?;
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

        Command::Add {
            source,
            compression: comp_str,
            index,
            output,
            root,
        } => {
            let data = std::fs::read(&cli.file)
                .with_context(|| format!("failed to read {}", cli.file.display()))?;
            let segments = segment::split_segments(&data)?;

            let insert_at = index.unwrap_or(segments.len());

            if insert_at > segments.len() {
                bail!(
                    "index {insert_at} out of range (file has {} archive{}, max insert index is {})",
                    segments.len(),
                    if segments.len() == 1 { "" } else { "s" },
                    segments.len(),
                );
            }

            if index.is_some() && insert_at == segments.len() {
                eprintln!(
                    "warning: index {insert_at} does not exist, appending to end"
                );
            }

            let comp: Compression = comp_str.parse()?;
            let archive = update::build_archive_from_dir(&source, root.as_deref())?;
            let cpio_bytes = cpio::write_archive(&archive);
            let compressed = compression::compress(&cpio_bytes, comp)?;

            let new_data = update::insert_segment(&segments, insert_at, compressed);

            let out_path = output.unwrap_or_else(|| cli.file.clone());
            std::fs::write(&out_path, &new_data)
                .with_context(|| format!("failed to write {}", out_path.display()))?;

            println!(
                "Added archive at index {insert_at} ({} entries, {comp}) -> {}",
                archive.entries.len(),
                out_path.display()
            );
        }

        Command::Create {
            source,
            compression: comp_str,
            force,
            root,
        } => {
            if !force && cli.file.exists() {
                bail!(
                    "{} already exists (use --force to overwrite)",
                    cli.file.display()
                );
            }

            let comp: Compression = comp_str.parse()?;
            let archive = update::build_archive_from_dir(&source, root.as_deref())?;
            let cpio_bytes = cpio::write_archive(&archive);
            let compressed = compression::compress(&cpio_bytes, comp)?;

            std::fs::write(&cli.file, &compressed)
                .with_context(|| format!("failed to write {}", cli.file.display()))?;

            println!(
                "Created {} ({} entries, {comp})",
                cli.file.display(),
                archive.entries.len(),
            );
        }
    }

    Ok(())
}
