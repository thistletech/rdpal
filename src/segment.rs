use anyhow::{bail, Result};

use crate::cpio;

/// Compression format of a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Zstd,
}

impl std::fmt::Display for Compression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Compression::None => write!(f, "none"),
            Compression::Gzip => write!(f, "gzip"),
            Compression::Bzip2 => write!(f, "bzip2"),
            Compression::Zstd => write!(f, "zstd"),
        }
    }
}

impl std::str::FromStr for Compression {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "none" => Ok(Compression::None),
            "gzip" | "gz" => Ok(Compression::Gzip),
            "bzip2" | "bz2" => Ok(Compression::Bzip2),
            "zstd" | "zst" => Ok(Compression::Zstd),
            _ => bail!("unknown compression: {s} (expected: none, gzip, bzip2, zstd)"),
        }
    }
}

/// A raw segment from a concatenated initramfs file.
#[derive(Debug)]
pub struct RawSegment {
    /// Byte offset in the original file.
    pub offset: usize,
    /// Raw bytes (still compressed if the segment was compressed).
    pub data: Vec<u8>,
    /// Detected compression type.
    pub compression: Compression,
}

/// Detect compression from magic bytes.
pub fn detect_compression(data: &[u8]) -> Option<Compression> {
    if data.len() >= 6 && &data[..6] == b"070701" {
        Some(Compression::None)
    } else if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        Some(Compression::Gzip)
    } else if data.len() >= 3 && &data[..3] == b"BZh" {
        Some(Compression::Bzip2)
    } else if data.len() >= 4 && data[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
        Some(Compression::Zstd)
    } else {
        None
    }
}

/// Determine how many compressed bytes a compressed stream consumes.
/// Uses format-specific low-level APIs to find exact boundaries.
fn compressed_size(data: &[u8], comp: Compression) -> Result<usize> {
    match comp {
        Compression::None => Ok(data.len()),
        Compression::Gzip => gzip_compressed_size(data),
        Compression::Bzip2 => bzip2_compressed_size(data),
        Compression::Zstd => zstd_compressed_size(data),
    }
}

/// Find the exact size of a gzip stream by parsing the header, using
/// low-level inflate to find deflate end, then adding the 8-byte footer.
fn gzip_compressed_size(data: &[u8]) -> Result<usize> {
    anyhow::ensure!(data.len() >= 10, "gzip data too short");
    anyhow::ensure!(data[0] == 0x1f && data[1] == 0x8b, "not gzip");

    let flg = data[3];
    let mut pos: usize = 10;

    // FEXTRA
    if flg & 4 != 0 {
        anyhow::ensure!(pos + 2 <= data.len(), "truncated gzip FEXTRA");
        let xlen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2 + xlen;
    }
    // FNAME
    if flg & 8 != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1; // skip null terminator
    }
    // FCOMMENT
    if flg & 16 != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    // FHCRC
    if flg & 2 != 0 {
        pos += 2;
    }

    // Inflate the deflate stream to find how many bytes it consumes
    let mut decomp = flate2::Decompress::new(false);
    let mut out_buf = vec![0u8; 64 * 1024];
    loop {
        let in_before = decomp.total_in() as usize;
        let _ = in_before; // suppress warning
        let status = decomp.decompress(
            &data[pos + decomp.total_in() as usize..],
            &mut out_buf,
            flate2::FlushDecompress::None,
        )?;
        if status == flate2::Status::StreamEnd {
            break;
        }
    }

    let total = pos + decomp.total_in() as usize + 8; // +8 for CRC32 + ISIZE
    Ok(total)
}

/// Find the exact size of a bzip2 stream. Bzip2 streams end with a
/// 48-bit magic (0x177245385090) that we can search for after decompression
/// tells us the approximate boundary.
fn bzip2_compressed_size(data: &[u8]) -> Result<usize> {
    // Decompress to find the approximate end, then use the fact that
    // bzip2 data must end at a byte boundary after the end-of-stream marker.
    use std::io::Read;
    let mut decoder = bzip2::read::BzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    // BzDecoder doesn't read past the bzip2 stream's actual end in terms
    // of underlying data. But it wraps in no BufReader, so we can check
    // how many bytes the internal decompressor consumed.
    // Unfortunately the bzip2 crate doesn't expose consumed byte count.
    // Fallback: scan forward from start for the next valid segment magic
    // after the decompressed data suggests the stream ended.
    // Use a heuristic: decompress with progressively larger slices.
    //
    // Better approach: the bzip2 crate's raw decompressor reports total_in.
    let mut decomp = bzip2::Decompress::new(false);
    let mut out_buf = vec![0u8; 64 * 1024];
    loop {
        let status = decomp.decompress(
            &data[decomp.total_in() as usize..],
            &mut out_buf,
        )?;
        if status == bzip2::Status::MemNeeded && decomp.total_out() > 0 {
            continue;
        }
        if (status == bzip2::Status::StreamEnd || status == bzip2::Status::Ok)
            && decomp.total_out() > 0 {
                // Check if we're done
                let remaining = &data[decomp.total_in() as usize..];
                if remaining.is_empty() || remaining[0] == 0 || detect_compression(remaining).is_some() {
                    break;
                }
            }
        if status == bzip2::Status::StreamEnd {
            break;
        }
    }
    Ok(decomp.total_in() as usize)
}

/// Find the exact size of a zstd stream using frame-level API.
fn zstd_compressed_size(data: &[u8]) -> Result<usize> {
    let mut pos = 0;
    loop {
        if pos >= data.len() {
            break;
        }
        if data.len() - pos < 4 || data[pos..pos + 4] != [0x28, 0xb5, 0x2f, 0xfd] {
            break;
        }
        let frame_size = zstd::zstd_safe::find_frame_compressed_size(&data[pos..])
            .map_err(|code| anyhow::anyhow!("zstd frame error: {code}"))?;
        pos += frame_size;
    }
    Ok(pos)
}

/// Split an initramfs file into its constituent raw segments.
pub fn split_segments(data: &[u8]) -> Result<Vec<RawSegment>> {
    let mut segments = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        // Skip null padding between segments
        while pos < data.len() && data[pos] == 0 {
            pos += 1;
        }
        if pos >= data.len() {
            break;
        }

        let comp = detect_compression(&data[pos..])
            .ok_or_else(|| anyhow::anyhow!("unknown format at offset {pos}"))?;

        match comp {
            Compression::None => {
                // Uncompressed CPIO: scan to find end
                let end = cpio::scan_archive_end(&data[pos..])
                    .map(|len| pos + len)
                    .unwrap_or(data.len());
                segments.push(RawSegment {
                    offset: pos,
                    data: data[pos..end].to_vec(),
                    compression: Compression::None,
                });
                pos = end;
            }
            comp => {
                // Compressed: decompress to find how many compressed bytes
                // were consumed, then take only those bytes as the segment.
                let consumed = compressed_size(&data[pos..], comp)?;
                let end = pos + consumed;
                segments.push(RawSegment {
                    offset: pos,
                    data: data[pos..end].to_vec(),
                    compression: comp,
                });
                pos = end;
            }
        }
    }

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_cpio() {
        assert_eq!(detect_compression(b"070701"), Some(Compression::None));
    }

    #[test]
    fn detect_gzip() {
        assert_eq!(
            detect_compression(&[0x1f, 0x8b, 0x08]),
            Some(Compression::Gzip)
        );
    }

    #[test]
    fn detect_bzip2() {
        assert_eq!(detect_compression(b"BZh9"), Some(Compression::Bzip2));
    }

    #[test]
    fn detect_zstd() {
        assert_eq!(
            detect_compression(&[0x28, 0xb5, 0x2f, 0xfd]),
            Some(Compression::Zstd)
        );
    }

    #[test]
    fn parse_compression_str() {
        assert_eq!("gzip".parse::<Compression>().unwrap(), Compression::Gzip);
        assert_eq!("bz2".parse::<Compression>().unwrap(), Compression::Bzip2);
        assert_eq!("zstd".parse::<Compression>().unwrap(), Compression::Zstd);
        assert_eq!("none".parse::<Compression>().unwrap(), Compression::None);
    }
}
