use super::*;
#[cfg(feature = "persistence")]
use std::io::Write;

#[test]
fn test_heap_zone_alloc() {
    let zone = Zone::new_heap(1024);
    let x = zone.alloc(42i64);
    assert_eq!(*x, 42);

    let y = zone.alloc(String::from("hello"));
    assert_eq!(y, "hello");
}

#[test]
fn test_heap_zone_alloc_slice() {
    let zone = Zone::new_heap(1024);
    let data = [1, 2, 3, 4, 5];
    let slice = zone.alloc_slice(&data);
    assert_eq!(slice, &[1, 2, 3, 4, 5]);
}

#[test]
fn test_heap_zone_reset() {
    let mut zone = Zone::new_heap(1024);
    let _ = zone.alloc(42);
    let before = zone.allocated_bytes();
    assert!(before > 0);

    zone.reset();
    // After reset, we can allocate again from the beginning
    let _ = zone.alloc(42);
}

#[cfg(feature = "persistence")]
#[test]
fn test_mapped_zone() {
    // Create a temp file
    let mut temp = tempfile::NamedTempFile::new().unwrap();
    temp.write_all(b"Hello, Zone!").unwrap();
    temp.flush().unwrap();

    let zone = Zone::new_mapped(temp.path()).unwrap();
    assert!(zone.is_mapped());
    assert_eq!(zone.as_slice(), b"Hello, Zone!");
}

#[cfg(feature = "persistence")]
#[test]
#[should_panic(expected = "Cannot allocate into a read-only Mapped Zone")]
fn test_mapped_zone_alloc_panics() {
    let mut temp = tempfile::NamedTempFile::new().unwrap();
    temp.write_all(b"test").unwrap();
    temp.flush().unwrap();

    let zone = Zone::new_mapped(temp.path()).unwrap();
    let _ = zone.alloc(42); // Should panic
}

#[cfg(feature = "persistence")]
#[test]
#[should_panic(expected = "Heap zones do not have a flat byte slice")]
fn test_heap_zone_as_slice_panics() {
    let zone = Zone::new_heap(1024);
    let _ = zone.as_slice(); // Should panic
}

#[test]
fn test_zone_type_checks() {
    let heap = Zone::new_heap(1024);
    assert!(heap.is_heap());
    assert!(!heap.is_mapped());
}

#[cfg(feature = "persistence")]
#[test]
fn test_mapped_zone_type_checks() {
    let mut temp = tempfile::NamedTempFile::new().unwrap();
    temp.write_all(b"test").unwrap();
    temp.flush().unwrap();

    let mapped = Zone::new_mapped(temp.path()).unwrap();
    assert!(mapped.is_mapped());
    assert!(!mapped.is_heap());
}
