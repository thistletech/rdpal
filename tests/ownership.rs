use std::os::unix::fs::MetadataExt;
use std::path::Path;

use rdpal::{cpio, extract, update};

/// Create a temp directory with a file and a symlink.
fn create_test_dir(base: &Path) {
    std::fs::create_dir_all(base).unwrap();
    std::fs::write(base.join("file.txt"), b"hello").unwrap();
    std::os::unix::fs::symlink("file.txt", base.join("link.txt")).unwrap();
}

/// Build a CPIO archive from a directory, then manually override uid/gid
/// on all entries to simulate an archive with different ownership.
fn build_archive_with_ownership(src: &Path, uid: u32, gid: u32) -> cpio::CpioArchive {
    let mut archive = update::build_archive_from_dir(src, None).unwrap();
    for entry in &mut archive.entries {
        entry.uid = uid;
        entry.gid = gid;
    }
    archive
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

fn has_cap_chown() -> bool {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("CapEff:") {
            if let Ok(mask) = u64::from_str_radix(rest.trim(), 16) {
                return mask & 1 != 0;
            }
        }
    }
    false
}

#[test]
fn extract_preserves_ownership_when_privileged() {
    if !is_root() && !has_cap_chown() {
        eprintln!("skipping ownership test: not root and no CAP_CHOWN");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    // Build archive with uid/gid = 0 (root)
    let archive = build_archive_with_ownership(&src, 0, 0);

    let dest = dir.path().join("extracted");
    extract::extract_archive(&archive, &dest).unwrap();

    // Verify ownership was set to root
    let file_meta = std::fs::metadata(dest.join("file.txt")).unwrap();
    assert_eq!(file_meta.uid(), 0, "file uid should be 0");
    assert_eq!(file_meta.gid(), 0, "file gid should be 0");

    let link_meta = std::fs::symlink_metadata(dest.join("link.txt")).unwrap();
    assert_eq!(link_meta.uid(), 0, "symlink uid should be 0");
    assert_eq!(link_meta.gid(), 0, "symlink gid should be 0");

    let dir_meta = std::fs::metadata(&dest.join(".")).unwrap();
    assert_eq!(dir_meta.uid(), 0, "dir uid should be 0");
    assert_eq!(dir_meta.gid(), 0, "dir gid should be 0");
}

#[test]
fn extract_without_privileges_still_succeeds() {
    // This test always runs — it verifies extraction doesn't fail
    // when ownership can't be set (the common unprivileged case).
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    // Build archive claiming uid/gid = 0
    let archive = build_archive_with_ownership(&src, 0, 0);

    let dest = dir.path().join("extracted");
    extract::extract_archive(&archive, &dest).unwrap();

    // Files should exist regardless of ownership outcome
    assert!(dest.join("file.txt").exists());
    assert!(dest.join("link.txt").is_symlink());
    assert_eq!(std::fs::read(dest.join("file.txt")).unwrap(), b"hello");
    assert_eq!(
        std::fs::read_link(dest.join("link.txt")).unwrap().to_str().unwrap(),
        "file.txt"
    );
}

#[test]
fn ownership_roundtrip_preserves_current_user() {
    // Without privileges, a roundtrip should at least preserve the
    // current user's uid/gid consistently.
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    let archive = update::build_archive_from_dir(&src, None).unwrap();
    let cpio_bytes = cpio::write_archive(&archive);
    let (parsed, _) = cpio::parse_archive(&cpio_bytes).unwrap();

    let src_meta = std::fs::metadata(src.join("file.txt")).unwrap();
    let file_entry = parsed.entries.iter().find(|e| e.name == "file.txt").unwrap();
    assert_eq!(file_entry.uid, src_meta.uid());
    assert_eq!(file_entry.gid, src_meta.gid());
}

#[test]
fn cpio_roundtrip_preserves_arbitrary_uid_gid() {
    // Verify uid/gid survives CPIO write + parse without corruption.
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    let archive = build_archive_with_ownership(&src, 1234, 5678);
    let cpio_bytes = cpio::write_archive(&archive);
    let (parsed, _) = cpio::parse_archive(&cpio_bytes).unwrap();

    for entry in &parsed.entries {
        assert_eq!(entry.uid, 1234, "uid mismatch on {}", entry.name);
        assert_eq!(entry.gid, 5678, "gid mismatch on {}", entry.name);
    }
}
