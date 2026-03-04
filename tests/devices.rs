use std::os::unix::fs::MetadataExt;

use rdpal::cpio::{CpioArchive, CpioEntry};
use rdpal::{cpio, extract};

fn make_device_entry(name: &str, block: bool, major: u32, minor: u32) -> CpioEntry {
    let mode = if block { 0o060000 } else { 0o020000 } | 0o660;
    CpioEntry {
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        nlink: 1,
        mtime: 0,
        devmajor: 0,
        devminor: 0,
        rdevmajor: major,
        rdevminor: minor,
        name: name.to_string(),
        data: Vec::new(),
    }
}

fn is_root() -> bool {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            if let Some(euid) = rest.split_whitespace().nth(1) {
                return euid == "0";
            }
        }
    }
    false
}

fn has_cap_mknod() -> bool {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("CapEff:") {
            if let Ok(mask) = u64::from_str_radix(rest.trim(), 16) {
                return (mask & (1 << 27)) != 0;
            }
        }
    }
    false
}

#[test]
fn cpio_roundtrip_block_device() {
    let archive = CpioArchive {
        entries: vec![make_device_entry("sda", true, 8, 0)],
    };
    let bytes = cpio::write_archive(&archive);
    let (parsed, _) = cpio::parse_archive(&bytes).unwrap();

    assert_eq!(parsed.entries.len(), 1);
    let entry = &parsed.entries[0];
    assert!(entry.is_block_device());
    assert!(!entry.is_char_device());
    assert_eq!(entry.name, "sda");
    assert_eq!(entry.rdevmajor, 8);
    assert_eq!(entry.rdevminor, 0);
    assert_eq!(entry.permissions(), 0o660);
    assert_eq!(entry.file_type_char(), 'b');
}

#[test]
fn cpio_roundtrip_char_device() {
    let archive = CpioArchive {
        entries: vec![make_device_entry("null", false, 1, 3)],
    };
    let bytes = cpio::write_archive(&archive);
    let (parsed, _) = cpio::parse_archive(&bytes).unwrap();

    assert_eq!(parsed.entries.len(), 1);
    let entry = &parsed.entries[0];
    assert!(entry.is_char_device());
    assert!(!entry.is_block_device());
    assert_eq!(entry.name, "null");
    assert_eq!(entry.rdevmajor, 1);
    assert_eq!(entry.rdevminor, 3);
    assert_eq!(entry.permissions(), 0o660);
    assert_eq!(entry.file_type_char(), 'c');
}

#[test]
fn cpio_roundtrip_mixed_with_devices() {
    let archive = CpioArchive {
        entries: vec![
            CpioEntry {
                ino: 0,
                mode: 0o040755,
                uid: 0,
                gid: 0,
                nlink: 2,
                mtime: 0,
                devmajor: 0,
                devminor: 0,
                rdevmajor: 0,
                rdevminor: 0,
                name: "dev".to_string(),
                data: Vec::new(),
            },
            make_device_entry("dev/null", false, 1, 3),
            make_device_entry("dev/zero", false, 1, 5),
            make_device_entry("dev/sda", true, 8, 0),
            CpioEntry {
                ino: 4,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                nlink: 1,
                mtime: 0,
                devmajor: 0,
                devminor: 0,
                rdevmajor: 0,
                rdevminor: 0,
                name: "dev/readme.txt".to_string(),
                data: b"hello".to_vec(),
            },
        ],
    };

    let bytes = cpio::write_archive(&archive);
    let (parsed, _) = cpio::parse_archive(&bytes).unwrap();

    assert_eq!(parsed.entries.len(), 5);
    assert!(parsed.entries[0].is_dir());
    assert!(parsed.entries[1].is_char_device());
    assert!(parsed.entries[2].is_char_device());
    assert!(parsed.entries[3].is_block_device());
    assert!(parsed.entries[4].is_file());
}

#[test]
fn extract_devices_when_privileged() {
    if !is_root() && !has_cap_mknod() {
        eprintln!("skipping device extraction test: not root and no CAP_MKNOD");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("extracted");

    let archive = CpioArchive {
        entries: vec![
            CpioEntry {
                ino: 0,
                mode: 0o040755,
                uid: 0,
                gid: 0,
                nlink: 2,
                mtime: 0,
                devmajor: 0,
                devminor: 0,
                rdevmajor: 0,
                rdevminor: 0,
                name: ".".to_string(),
                data: Vec::new(),
            },
            make_device_entry("test_null", false, 1, 3),
            make_device_entry("test_blk", true, 7, 0),
        ],
    };

    extract::extract_archive(&archive, &dest).unwrap();

    // Verify char device
    let null_meta = std::fs::symlink_metadata(dest.join("test_null")).unwrap();
    let ft = null_meta.file_type();
    assert!(
        !ft.is_file() && !ft.is_dir() && !ft.is_symlink(),
        "test_null should be a device node"
    );
    assert_eq!(null_meta.rdev(), libc::makedev(1, 3));

    // Verify block device
    let blk_meta = std::fs::symlink_metadata(dest.join("test_blk")).unwrap();
    assert_eq!(blk_meta.rdev(), libc::makedev(7, 0));
}

#[test]
fn extract_skips_devices_without_privileges() {
    if is_root() || has_cap_mknod() {
        eprintln!("skipping unprivileged device test: running with mknod capability");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("extracted");

    let archive = CpioArchive {
        entries: vec![
            CpioEntry {
                ino: 0,
                mode: 0o040755,
                uid: 0,
                gid: 0,
                nlink: 2,
                mtime: 0,
                devmajor: 0,
                devminor: 0,
                rdevmajor: 0,
                rdevminor: 0,
                name: ".".to_string(),
                data: Vec::new(),
            },
            make_device_entry("skipped_dev", false, 1, 3),
            CpioEntry {
                ino: 2,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                nlink: 1,
                mtime: 0,
                devmajor: 0,
                devminor: 0,
                rdevmajor: 0,
                rdevminor: 0,
                name: "regular.txt".to_string(),
                data: b"still works".to_vec(),
            },
        ],
    };

    // Should succeed — devices skipped, regular files extracted
    extract::extract_archive(&archive, &dest).unwrap();

    assert!(!dest.join("skipped_dev").exists(), "device should not be created");
    assert!(dest.join("regular.txt").exists(), "regular file should exist");
    assert_eq!(
        std::fs::read(dest.join("regular.txt")).unwrap(),
        b"still works"
    );
}

#[test]
fn device_entry_rdev_preserved_through_cpio() {
    // Test a range of major/minor values survive serialization.
    let entries: Vec<CpioEntry> = vec![
        make_device_entry("null", false, 1, 3),
        make_device_entry("zero", false, 1, 5),
        make_device_entry("tty0", false, 4, 0),
        make_device_entry("sda", true, 8, 0),
        make_device_entry("sda1", true, 8, 1),
        make_device_entry("loop0", true, 7, 0),
    ];

    let archive = CpioArchive {
        entries: entries.clone(),
    };
    let bytes = cpio::write_archive(&archive);
    let (parsed, _) = cpio::parse_archive(&bytes).unwrap();

    for (orig, parsed) in entries.iter().zip(parsed.entries.iter()) {
        assert_eq!(orig.name, parsed.name);
        assert_eq!(orig.rdevmajor, parsed.rdevmajor, "rdevmajor mismatch: {}", orig.name);
        assert_eq!(orig.rdevminor, parsed.rdevminor, "rdevminor mismatch: {}", orig.name);
        assert_eq!(orig.mode, parsed.mode, "mode mismatch: {}", orig.name);
    }
}
