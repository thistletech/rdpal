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

## CPIO Entry Type Support
All CPIO entry types supported on extract: directories (`0o040000`), regular files (`0o100000`), symlinks (`0o120000`), block devices (`0o060000`), character devices (`0o020000`), FIFOs (`0o010000`), sockets (`0o140000`).

Device nodes require root or CAP_MKNOD — created via `libc::mknod`. Skipped with a warning when unprivileged. FIFOs and sockets are created via `mknod` and do not require special privileges.

Build from directory (`build_archive_from_dir`): supports dirs, files, symlinks. Device nodes on disk are skipped by walkdir.

## Privilege Detection
`extract.rs` parses `/proc/self/status` for effective UID and CapEff bitmask. Checks CAP_CHOWN (bit 0) for ownership and CAP_MKNOD (bit 27) for device nodes. Prints `setpriv --ambient-caps` hint when missing.

## Mode Handling
Full 16-bit mode (file type + SUID/SGID/sticky + rwx) preserved through parse/write. On extract, permissions restored via `PermissionsExt::from_mode()` for dirs and files. Symlink permissions not set (Linux ignores them). Device node modes passed directly to `mknod`.

## Test Data
- `test-data/boot-initrd` — 88MB, 4 archives: 3 uncompressed + 1 zstd-compressed.

## Dependencies
anyhow, clap (derive), flate2, bzip2, zstd, walkdir. Edition 2024.
