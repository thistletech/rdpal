# rdpal Project Context

## Project
Linux initramfs/ramdisk CLI tool + library. Parses concatenated CPIO archives (newc format), each optionally compressed.

## Architecture
- `src/cpio.rs` — CPIO newc parser/writer (from scratch). `CpioEntry`, `CpioArchive`, `parse_archive`, `scan_archive_end`, `write_archive`.
- `src/segment.rs` — Splits concatenated initramfs into `RawSegment`s. `Compression` enum (None/Gzip/Bzip2/Zstd). Magic byte detection. Compressed size detection uses format-specific low-level APIs.
- `src/compression.rs` — Thin wrappers: `decompress`/`compress` dispatching to flate2/bzip2/zstd.
- `src/extract.rs` — `extract_archive` writes dirs, files, symlinks to disk.
- `src/update.rs` — `build_archive_from_dir` (walkdir), `reassemble` (replace segment + 512-byte null padding).
- `src/info.rs` — `print_info` table output.
- `src/main.rs` — clap CLI: `info`, `extract`, `update`, `create` subcommands.

## Key Technical Decisions
- CPIO parsing implemented from scratch (not using `cpio` crate) for full control over concatenated archive handling.
- Compressed segment boundary detection is tricky:
  - **zstd**: Uses `zstd::zstd_safe::find_frame_compressed_size` (frame-level API). Loops to handle multi-frame streams.
  - **gzip**: Manually parses gzip header, uses `flate2::Decompress` (raw inflate) to find deflate end, adds 8 bytes for CRC32+ISIZE footer.
  - **bzip2**: Uses `bzip2::Decompress` low-level API with `total_in()`.
  - TrackingReader approach was tried and abandoned — decoders with internal BufReader (zstd) or chunked reads (flate2) read past the actual compressed boundary.
- Reassembly pads between segments with nulls to 512-byte boundaries.

## Test Data
- `test-data/boot-initrd` — 88MB, 4 archives: 3 uncompressed + 1 zstd-compressed.

## Dependencies
anyhow, clap (derive), flate2, bzip2, zstd, walkdir. Edition 2024.
