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

## Unsupported CPIO Entry Types
The following CPIO entry types are parsed but skipped (with a warning) during extract and build:
- Block devices (`0o060000`) — requires `mknod` + root privileges
- Character devices (`0o020000`) — requires `mknod` + root privileges
- Sockets (`0o140000`) — cannot be created from archive data
- FIFOs/named pipes (`0o010000`) — could be supported with `mkfifo` but currently skipped

Supported types: directories (`0o040000`), regular files (`0o100000`), symlinks (`0o120000`).

Mode handling: full 16-bit mode (file type + SUID/SGID/sticky + rwx) is preserved through parse/write. On extract, permissions (including special bits) are restored via `PermissionsExt::from_mode()` for dirs and files. Symlink permissions are not set (Linux ignores them).

## Test Data
- `test-data/boot-initrd` — 88MB, 4 archives: 3 uncompressed + 1 zstd-compressed.

## Dependencies
anyhow, clap (derive), flate2, bzip2, zstd, walkdir. Edition 2024.
