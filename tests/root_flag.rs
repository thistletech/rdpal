use std::path::Path;

use rdpal::{compression, cpio, segment, update};

/// Create a temp directory with some test files and subdirectories.
fn create_test_dir(base: &Path) {
    std::fs::create_dir_all(base.join("subdir")).unwrap();
    std::fs::write(base.join("hello.txt"), b"hello world").unwrap();
    std::fs::write(base.join("subdir/nested.txt"), b"nested content").unwrap();
}

/// Parse a CPIO archive from raw (uncompressed) bytes and return entry names.
fn entry_names(data: &[u8]) -> Vec<String> {
    let (archive, _) = cpio::parse_archive(data).unwrap();
    archive.entries.iter().map(|e| e.name.clone()).collect()
}

#[test]
fn create_without_root_uses_dot() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    let archive = update::build_archive_from_dir(&src, None).unwrap();
    let names: Vec<&str> = archive.entries.iter().map(|e| e.name.as_str()).collect();

    assert_eq!(names[0], ".");
    assert!(names.contains(&"hello.txt"));
    assert!(names.contains(&"subdir"));
    assert!(names.contains(&"subdir/nested.txt"));
}

#[test]
fn create_with_root_prefixes_all_entries() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    let root = Path::new("myroot");
    let archive = update::build_archive_from_dir(&src, Some(root)).unwrap();
    let names: Vec<&str> = archive.entries.iter().map(|e| e.name.as_str()).collect();

    assert_eq!(names[0], "myroot");
    assert!(names.contains(&"myroot/hello.txt"));
    assert!(names.contains(&"myroot/subdir"));
    assert!(names.contains(&"myroot/subdir/nested.txt"));
}

#[test]
fn create_with_nested_root() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    let root = Path::new("kernel/x86/microcode");
    let archive = update::build_archive_from_dir(&src, Some(root)).unwrap();
    let names: Vec<&str> = archive.entries.iter().map(|e| e.name.as_str()).collect();

    assert_eq!(names[0], "kernel/x86/microcode");
    assert!(names.contains(&"kernel/x86/microcode/hello.txt"));
    assert!(names.contains(&"kernel/x86/microcode/subdir"));
    assert!(names.contains(&"kernel/x86/microcode/subdir/nested.txt"));
}

#[test]
fn roundtrip_create_and_parse_with_root() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    let root = Path::new("prefix");
    let archive = update::build_archive_from_dir(&src, Some(root)).unwrap();
    let cpio_bytes = cpio::write_archive(&archive);

    // Parse back and verify
    let names = entry_names(&cpio_bytes);
    assert_eq!(names[0], "prefix");
    assert!(names.contains(&"prefix/hello.txt".to_string()));
    assert!(names.contains(&"prefix/subdir/nested.txt".to_string()));
}

#[test]
fn update_segment_with_root() {
    let dir = tempfile::tempdir().unwrap();
    let src1 = dir.path().join("src1");
    let src2 = dir.path().join("src2");
    create_test_dir(&src1);
    std::fs::create_dir_all(&src2).unwrap();
    std::fs::write(src2.join("replacement.txt"), b"replaced").unwrap();

    // Build initial file with one segment
    let archive1 = update::build_archive_from_dir(&src1, None).unwrap();
    let initial_bytes = cpio::write_archive(&archive1);

    // Parse as segments
    let segments = segment::split_segments(&initial_bytes).unwrap();
    assert_eq!(segments.len(), 1);

    // Update with root prefix
    let root = Path::new("updated");
    let archive2 = update::build_archive_from_dir(&src2, Some(root)).unwrap();
    let new_cpio = cpio::write_archive(&archive2);
    let new_data = update::reassemble(&segments, 0, new_cpio);

    // Parse the result
    let new_segments = segment::split_segments(&new_data).unwrap();
    assert_eq!(new_segments.len(), 1);

    let names = entry_names(&new_segments[0].data);
    assert_eq!(names[0], "updated");
    assert!(names.contains(&"updated/replacement.txt".to_string()));
}

#[test]
fn add_segment_with_root() {
    let dir = tempfile::tempdir().unwrap();
    let src1 = dir.path().join("src1");
    let src2 = dir.path().join("src2");
    create_test_dir(&src1);
    std::fs::create_dir_all(&src2).unwrap();
    std::fs::write(src2.join("added.txt"), b"new section").unwrap();

    // Build initial file with one segment
    let archive1 = update::build_archive_from_dir(&src1, None).unwrap();
    let initial_bytes = cpio::write_archive(&archive1);

    let segments = segment::split_segments(&initial_bytes).unwrap();
    assert_eq!(segments.len(), 1);

    // Add a second segment with root prefix
    let root = Path::new("extra");
    let archive2 = update::build_archive_from_dir(&src2, Some(root)).unwrap();
    let new_cpio = cpio::write_archive(&archive2);
    let new_data = update::insert_segment(&segments, 1, new_cpio);

    // Parse the result — should have 2 segments
    let new_segments = segment::split_segments(&new_data).unwrap();
    assert_eq!(new_segments.len(), 2);

    // First segment unchanged
    let names0 = entry_names(&new_segments[0].data);
    assert_eq!(names0[0], ".");

    // Second segment has root prefix
    let names1 = entry_names(&new_segments[1].data);
    assert_eq!(names1[0], "extra");
    assert!(names1.contains(&"extra/added.txt".to_string()));
}

#[test]
fn add_segment_at_index_zero_with_root() {
    let dir = tempfile::tempdir().unwrap();
    let src1 = dir.path().join("src1");
    let src2 = dir.path().join("src2");
    create_test_dir(&src1);
    std::fs::create_dir_all(&src2).unwrap();
    std::fs::write(src2.join("first.txt"), b"inserted first").unwrap();

    // Build initial file
    let archive1 = update::build_archive_from_dir(&src1, None).unwrap();
    let initial_bytes = cpio::write_archive(&archive1);
    let segments = segment::split_segments(&initial_bytes).unwrap();

    // Insert at index 0
    let root = Path::new("prepended");
    let archive2 = update::build_archive_from_dir(&src2, Some(root)).unwrap();
    let new_cpio = cpio::write_archive(&archive2);
    let new_data = update::insert_segment(&segments, 0, new_cpio);

    let new_segments = segment::split_segments(&new_data).unwrap();
    assert_eq!(new_segments.len(), 2);

    // First segment is the newly inserted one
    let names0 = entry_names(&new_segments[0].data);
    assert_eq!(names0[0], "prepended");
    assert!(names0.contains(&"prepended/first.txt".to_string()));

    // Second segment is the original
    let names1 = entry_names(&new_segments[1].data);
    assert_eq!(names1[0], ".");
}

#[test]
fn compressed_roundtrip_with_root() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    create_test_dir(&src);

    let root = Path::new("zroot");
    let archive = update::build_archive_from_dir(&src, Some(root)).unwrap();
    let cpio_bytes = cpio::write_archive(&archive);
    let compressed =
        compression::compress(&cpio_bytes, segment::Compression::Zstd).unwrap();

    // Decompress and verify
    let decompressed =
        compression::decompress(&compressed, segment::Compression::Zstd).unwrap();
    let names = entry_names(&decompressed);
    assert_eq!(names[0], "zroot");
    assert!(names.contains(&"zroot/hello.txt".to_string()));
}
