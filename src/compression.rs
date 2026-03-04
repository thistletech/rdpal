use anyhow::Result;
use std::io::Read;

use crate::segment::Compression;

/// Decompress data according to the given compression type.
pub fn decompress(data: &[u8], compression: Compression) -> Result<Vec<u8>> {
    match compression {
        Compression::None => Ok(data.to_vec()),
        Compression::Gzip => {
            let mut decoder = flate2::read::GzDecoder::new(data);
            let mut out = Vec::new();
            decoder.read_to_end(&mut out)?;
            Ok(out)
        }
        Compression::Bzip2 => {
            let mut decoder = bzip2::read::BzDecoder::new(data);
            let mut out = Vec::new();
            decoder.read_to_end(&mut out)?;
            Ok(out)
        }
        Compression::Zstd => {
            let out = zstd::stream::decode_all(data)?;
            Ok(out)
        }
    }
}

/// Compress data with the given compression type.
pub fn compress(data: &[u8], compression: Compression) -> Result<Vec<u8>> {
    match compression {
        Compression::None => Ok(data.to_vec()),
        Compression::Gzip => {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            std::io::Write::write_all(&mut encoder, data)?;
            Ok(encoder.finish()?)
        }
        Compression::Bzip2 => {
            let mut encoder =
                bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
            std::io::Write::write_all(&mut encoder, data)?;
            Ok(encoder.finish()?)
        }
        Compression::Zstd => {
            let out = zstd::stream::encode_all(data, 0)?;
            Ok(out)
        }
    }
}
