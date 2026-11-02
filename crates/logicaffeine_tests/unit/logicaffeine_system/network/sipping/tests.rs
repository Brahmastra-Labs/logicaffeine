use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

fn create_test_file(size: usize) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
    file.write_all(&data).unwrap();
    file.flush().unwrap();
    file
}

#[test]
fn test_chunk_count_empty() {
    // Create empty file
    let file = NamedTempFile::new().unwrap();
    let zone = Zone::new_mapped(file.path()).unwrap();
    let sipper = FileSipper::from_zone(&zone);
    assert_eq!(sipper.chunk_count(), 0);
}

#[test]
fn test_chunk_count_small_file() {
    let file = create_test_file(100);
    let zone = Zone::new_mapped(file.path()).unwrap();
    let sipper = FileSipper::from_zone(&zone);
    assert_eq!(sipper.chunk_count(), 1); // 100 bytes < 1 MB
}

#[test]
fn test_manifest_has_correct_size() {
    let file = create_test_file(1000);
    let zone = Zone::new_mapped(file.path()).unwrap();
    let sipper = FileSipper::from_zone(&zone);
    let manifest = sipper.manifest();

    assert_eq!(manifest.total_size, 1000);
    assert_eq!(manifest.chunk_count, 1);
    assert_eq!(manifest.chunk_hashes.len(), 1);
}

#[test]
fn test_chunk_hash_is_consistent() {
    let file = create_test_file(500);
    let zone = Zone::new_mapped(file.path()).unwrap();
    let sipper = FileSipper::from_zone(&zone);

    let hash1 = sipper.hash_chunk(0);
    let hash2 = sipper.hash_chunk(0);
    assert_eq!(hash1, hash2, "Same chunk should have same hash");
}

#[test]
fn test_custom_chunk_size() {
    let file = create_test_file(1000);
    let zone = Zone::new_mapped(file.path()).unwrap();
    let sipper = FileSipper::with_chunk_size(&zone, 100);

    assert_eq!(sipper.chunk_count(), 10); // 1000 / 100 = 10 chunks

    let manifest = sipper.manifest();
    assert_eq!(manifest.chunk_count, 10);
    assert_eq!(manifest.chunk_hashes.len(), 10);
}
