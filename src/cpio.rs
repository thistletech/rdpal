use anyhow::{Context, Result, ensure};

const CPIO_MAGIC: &[u8] = b"070701";
const HEADER_LEN: usize = 110;
const TRAILER_NAME: &str = "TRAILER!!!";

/// A single entry in a CPIO newc archive.
#[derive(Debug, Clone)]
pub struct CpioEntry {
    pub ino: u32,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub mtime: u32,
    pub devmajor: u32,
    pub devminor: u32,
    pub rdevmajor: u32,
    pub rdevminor: u32,
    pub name: String,
    pub data: Vec<u8>,
}

impl CpioEntry {
    pub fn is_dir(&self) -> bool {
        (self.mode & 0o170000) == 0o040000
    }

    pub fn is_file(&self) -> bool {
        (self.mode & 0o170000) == 0o100000
    }

    pub fn is_symlink(&self) -> bool {
        (self.mode & 0o170000) == 0o120000
    }

    pub fn is_block_device(&self) -> bool {
        (self.mode & 0o170000) == 0o060000
    }

    pub fn is_char_device(&self) -> bool {
        (self.mode & 0o170000) == 0o020000
    }

    pub fn is_fifo(&self) -> bool {
        (self.mode & 0o170000) == 0o010000
    }

    pub fn is_socket(&self) -> bool {
        (self.mode & 0o170000) == 0o140000
    }

    pub fn permissions(&self) -> u32 {
        self.mode & 0o7777
    }

    pub fn file_type_char(&self) -> char {
        match self.mode & 0o170000 {
            0o040000 => 'd',
            0o100000 => '-',
            0o120000 => 'l',
            0o060000 => 'b',
            0o020000 => 'c',
            0o010000 => 'p',
            0o140000 => 's',
            _ => '?',
        }
    }
}

/// A parsed CPIO archive (TRAILER entry excluded).
#[derive(Debug)]
pub struct CpioArchive {
    pub entries: Vec<CpioEntry>,
}

/// Round up to next 4-byte boundary.
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Parse an 8-character hex ASCII field into u32.
fn parse_hex_field(bytes: &[u8]) -> Result<u32> {
    let s = std::str::from_utf8(bytes).context("CPIO header field is not valid UTF-8")?;
    u32::from_str_radix(s, 16).with_context(|| format!("invalid hex in CPIO header: {s:?}"))
}

/// Parse a complete CPIO archive from decompressed bytes.
/// Returns the archive and the number of bytes consumed.
pub fn parse_archive(data: &[u8]) -> Result<(CpioArchive, usize)> {
    let mut entries = Vec::new();
    let mut pos = 0;

    loop {
        ensure!(
            pos + HEADER_LEN <= data.len(),
            "unexpected end of data at offset {pos}"
        );
        ensure!(
            &data[pos..pos + 6] == CPIO_MAGIC,
            "bad CPIO magic at offset {pos}"
        );

        let ino = parse_hex_field(&data[pos + 6..pos + 14])?;
        let mode = parse_hex_field(&data[pos + 14..pos + 22])?;
        let uid = parse_hex_field(&data[pos + 22..pos + 30])?;
        let gid = parse_hex_field(&data[pos + 30..pos + 38])?;
        let nlink = parse_hex_field(&data[pos + 38..pos + 46])?;
        let mtime = parse_hex_field(&data[pos + 46..pos + 54])?;
        let filesize = parse_hex_field(&data[pos + 54..pos + 62])? as usize;
        let devmajor = parse_hex_field(&data[pos + 62..pos + 70])?;
        let devminor = parse_hex_field(&data[pos + 70..pos + 78])?;
        let rdevmajor = parse_hex_field(&data[pos + 78..pos + 86])?;
        let rdevminor = parse_hex_field(&data[pos + 86..pos + 94])?;
        let namesize = parse_hex_field(&data[pos + 94..pos + 102])? as usize;
        let _check = parse_hex_field(&data[pos + 102..pos + 110])?;

        let name_start = pos + HEADER_LEN;
        let name_end = name_start + namesize;
        ensure!(name_end <= data.len(), "name extends past end of data");
        // namesize includes the null terminator
        let name =
            std::str::from_utf8(&data[name_start..name_end - 1]).context("invalid entry name")?;

        let data_start = align4(name_end);
        let data_end = data_start + filesize;
        ensure!(data_end <= data.len(), "file data extends past end of data");

        pos = align4(data_end);

        if name == TRAILER_NAME {
            break;
        }

        entries.push(CpioEntry {
            ino,
            mode,
            uid,
            gid,
            nlink,
            mtime,
            devmajor,
            devminor,
            rdevmajor,
            rdevminor,
            name: name.to_string(),
            data: data[data_start..data_end].to_vec(),
        });
    }

    Ok((CpioArchive { entries }, pos))
}

/// Lightweight scan to find the end offset of a CPIO archive without
/// allocating entry data. Returns bytes consumed including TRAILER + padding.
pub fn scan_archive_end(data: &[u8]) -> Result<usize> {
    let mut pos = 0;

    loop {
        ensure!(
            pos + HEADER_LEN <= data.len(),
            "unexpected end of data during scan at offset {pos}"
        );
        ensure!(
            &data[pos..pos + 6] == CPIO_MAGIC,
            "bad CPIO magic during scan at offset {pos}"
        );

        let filesize = parse_hex_field(&data[pos + 54..pos + 62])? as usize;
        let namesize = parse_hex_field(&data[pos + 94..pos + 102])? as usize;

        let name_start = pos + HEADER_LEN;
        let name_end = name_start + namesize;
        ensure!(name_end <= data.len(), "name extends past end of data");
        let name = std::str::from_utf8(&data[name_start..name_end - 1])
            .context("invalid entry name during scan")?;

        let data_start = align4(name_end);
        let data_end = data_start + filesize;
        pos = align4(data_end);

        if name == TRAILER_NAME {
            break;
        }
    }

    Ok(pos)
}

/// Serialize a CpioArchive into CPIO newc format bytes with TRAILER.
pub fn write_archive(archive: &CpioArchive) -> Vec<u8> {
    let mut buf = Vec::new();

    for entry in &archive.entries {
        write_entry(&mut buf, entry);
    }

    // Write TRAILER
    let trailer = CpioEntry {
        ino: 0,
        mode: 0,
        uid: 0,
        gid: 0,
        nlink: 1,
        mtime: 0,
        devmajor: 0,
        devminor: 0,
        rdevmajor: 0,
        rdevminor: 0,
        name: TRAILER_NAME.to_string(),
        data: Vec::new(),
    };
    write_entry(&mut buf, &trailer);

    buf
}

fn write_entry(buf: &mut Vec<u8>, entry: &CpioEntry) {
    use std::fmt::Write;

    let namesize = entry.name.len() + 1; // include null terminator
    let filesize = entry.data.len();

    let mut header = String::with_capacity(HEADER_LEN);
    write!(header, "070701").unwrap();
    write!(header, "{:08X}", entry.ino).unwrap();
    write!(header, "{:08X}", entry.mode).unwrap();
    write!(header, "{:08X}", entry.uid).unwrap();
    write!(header, "{:08X}", entry.gid).unwrap();
    write!(header, "{:08X}", entry.nlink).unwrap();
    write!(header, "{:08X}", entry.mtime).unwrap();
    write!(header, "{:08X}", filesize).unwrap();
    write!(header, "{:08X}", entry.devmajor).unwrap();
    write!(header, "{:08X}", entry.devminor).unwrap();
    write!(header, "{:08X}", entry.rdevmajor).unwrap();
    write!(header, "{:08X}", entry.rdevminor).unwrap();
    write!(header, "{:08X}", namesize).unwrap();
    write!(header, "{:08X}", 0u32).unwrap(); // check

    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(entry.name.as_bytes());
    buf.push(0); // null terminator

    // Pad header+name to 4-byte boundary
    let total = HEADER_LEN + namesize;
    let padded = align4(total);
    buf.resize(buf.len() + (padded - total), 0);

    buf.extend_from_slice(&entry.data);

    // Pad data to 4-byte boundary
    let data_padded = align4(filesize);
    buf.resize(buf.len() + (data_padded - filesize), 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let archive = CpioArchive {
            entries: vec![
                CpioEntry {
                    ino: 0,
                    mode: 0o040755,
                    uid: 0,
                    gid: 0,
                    nlink: 2,
                    mtime: 1000,
                    devmajor: 0,
                    devminor: 0,
                    rdevmajor: 0,
                    rdevminor: 0,
                    name: ".".to_string(),
                    data: Vec::new(),
                },
                CpioEntry {
                    ino: 1,
                    mode: 0o100644,
                    uid: 0,
                    gid: 0,
                    nlink: 1,
                    mtime: 1000,
                    devmajor: 0,
                    devminor: 0,
                    rdevmajor: 0,
                    rdevminor: 0,
                    name: "hello.txt".to_string(),
                    data: b"Hello, world!\n".to_vec(),
                },
            ],
        };

        let bytes = write_archive(&archive);
        let (parsed, consumed) = parse_archive(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].name, ".");
        assert!(parsed.entries[0].is_dir());
        assert_eq!(parsed.entries[1].name, "hello.txt");
        assert_eq!(parsed.entries[1].data, b"Hello, world!\n");
    }

    #[test]
    fn scan_end_matches_parse() {
        let archive = CpioArchive {
            entries: vec![CpioEntry {
                ino: 0,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                nlink: 1,
                mtime: 0,
                devmajor: 0,
                devminor: 0,
                rdevmajor: 0,
                rdevminor: 0,
                name: "test".to_string(),
                data: b"data".to_vec(),
            }],
        };

        let bytes = write_archive(&archive);
        let scan_end = scan_archive_end(&bytes).unwrap();
        let (_, parse_end) = parse_archive(&bytes).unwrap();
        assert_eq!(scan_end, parse_end);
    }
}
