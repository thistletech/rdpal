# rdpal

A CLI tool for inspecting and manipulating Linux initramfs/ramdisk files.

Linux initramfs images are composed of one or more CPIO archives (newc format) concatenated together, each optionally compressed. rdpal can split these apart, inspect them, extract individual archives, and rebuild them with different compression.

This project was co-written in association with Opus 4.6.

## Features

- Parses concatenated CPIO archives within a single initramfs file
- Automatically detects compression per-segment: gzip, bzip2, zstd, or uncompressed
- Extracts individual archives to disk (directories, files, symlinks)
- Rebuilds archives from a directory with optional compression
- CPIO newc format implemented from scratch for full control over parsing and writing

## Building

```
cargo build --release
```

## Usage

### Inspect a ramdisk

```
rdpal <file> info
```

Shows a summary of all CPIO archives in the file, including offset, compressed/decompressed size, compression type, entry count, and first entry name.

```
$ rdpal boot-initrd info
Ramdisk: boot-initrd (88086613 bytes, 4 archives)

  #    Offset       Comp Size      Compression  Decomp Size    Entries  First Entry
  --------------------------------------------------------------------------------
  0    0            148112         none         148112         5        .
  1    148480       13126444       none         13126444       5        kernel
  2    13275136     53583708       none         53583708       1893     .
  3    66859008     21227605       zstd         54261248       465      .
```

### Extract a single archive

```
rdpal <file> extract --index <N> --dest <directory>
```

Extracts the archive at the given 0-based index to the destination directory. Handles directories, regular files, and symlinks. Permissions are preserved.

```
$ rdpal boot-initrd extract --index 0 --dest /tmp/archive0
Extracted archive 0 (5 entries) to /tmp/archive0
```

### Update a single archive

```
rdpal <file> update --index <N> --source <directory> --compression <type> [--output <file>]
```

Replaces the archive at the given index with a new CPIO archive built from the source directory. The remaining archives are preserved unchanged.

Supported compression types: `none`, `gzip`, `bzip2`, `zstd`

If `--output` is not specified, the input file is overwritten.

```
$ rdpal boot-initrd update --index 0 --source /tmp/archive0 --compression zstd --output modified-initrd
Updated archive 0 (5 entries, zstd) -> modified-initrd
```

## Library

rdpal is also usable as a library. The public modules are:

- `rdpal::cpio` -- CPIO newc parsing and writing (`CpioEntry`, `CpioArchive`, `parse_archive`, `write_archive`)
- `rdpal::segment` -- Segment splitting and compression detection (`split_segments`, `Compression`)
- `rdpal::compression` -- Compress/decompress helpers
- `rdpal::extract` -- Extract archive entries to filesystem
- `rdpal::update` -- Build archive from directory, reassemble initramfs
- `rdpal::info` -- Print segment information
