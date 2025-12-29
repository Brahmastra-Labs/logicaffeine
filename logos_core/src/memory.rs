//! Phase 8.5 & 8.6: Zone-based Memory Management
//!
//! Zones provide region-based allocation with O(1) allocation and bulk deallocation.
//! Two backing strategies are supported:
//! - Heap: Fast arena allocation via bumpalo
//! - Mapped: Zero-copy file mapping via memmap2

use std::fs::File;
use std::io;
use std::path::Path;

/// A memory region for batch allocation and bulk deallocation.
///
/// Zones implement the "Hotel California" rule: values can enter but cannot
/// escape. This enables safe O(1) deallocation when the zone goes out of scope.
pub enum Zone {
    /// Dynamic heap-allocated arena (Scratchpad).
    /// Use for temporary allocations that can be bulk-freed.
    Heap(bumpalo::Bump),
    /// Memory-mapped file (Zero-copy IO).
    /// Provides read-only access to file contents without loading into memory.
    Mapped(memmap2::Mmap),
}

impl Zone {
    /// Create a new empty zone on the heap with pre-sized capacity.
    ///
    /// # Example
    /// ```ignore
    /// let zone = Zone::new_heap(1024 * 1024); // 1 MB arena
    /// let x = zone.alloc(42);
    /// ```
    pub fn new_heap(capacity_bytes: usize) -> Self {
        Zone::Heap(bumpalo::Bump::with_capacity(capacity_bytes))
    }

    /// Create a new zone backed by a memory-mapped file.
    ///
    /// # Safety
    /// The file should not be modified by other processes while mapped.
    /// Standard mmap safety caveats apply.
    ///
    /// # Example
    /// ```ignore
    /// let zone = Zone::new_mapped("data.bin")?;
    /// let bytes = zone.as_slice();
    /// ```
    pub fn new_mapped<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        // SAFETY: We assume the file is not concurrently modified by other
        // processes in a way that causes undefined behavior.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Ok(Zone::Mapped(mmap))
    }

    /// Allocate a value inside the zone.
    ///
    /// Returns a reference with lifetime tied to the zone.
    /// Only valid for Heap zones; Mapped zones are read-only.
    ///
    /// # Panics
    /// Panics if called on a Mapped zone.
    pub fn alloc<T>(&self, val: T) -> &T {
        match self {
            Zone::Heap(bump) => bump.alloc(val),
            Zone::Mapped(_) => panic!(
                "Cannot allocate into a read-only Mapped Zone. \
                 Use Zone::new_heap() for allocations."
            ),
        }
    }

    /// Allocate a slice inside the zone.
    ///
    /// Only valid for Heap zones.
    ///
    /// # Panics
    /// Panics if called on a Mapped zone.
    pub fn alloc_slice<T: Copy>(&self, vals: &[T]) -> &[T] {
        match self {
            Zone::Heap(bump) => bump.alloc_slice_copy(vals),
            Zone::Mapped(_) => panic!(
                "Cannot allocate into a read-only Mapped Zone. \
                 Use Zone::new_heap() for allocations."
            ),
        }
    }

    /// Get a reference to the mapped memory as a byte slice.
    ///
    /// Only valid for Mapped zones.
    ///
    /// # Panics
    /// Panics if called on a Heap zone.
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Zone::Heap(_) => panic!(
                "Heap zones do not have a flat byte slice representation. \
                 Use Zone::new_mapped() for file access."
            ),
            Zone::Mapped(mmap) => &mmap[..],
        }
    }

    /// Reset the zone, deallocating all allocations.
    ///
    /// For Heap zones, this resets the bump allocator.
    /// For Mapped zones, this is a no-op.
    pub fn reset(&mut self) {
        if let Zone::Heap(bump) = self {
            bump.reset();
        }
    }

    /// Returns true if this is a Heap zone.
    pub fn is_heap(&self) -> bool {
        matches!(self, Zone::Heap(_))
    }

    /// Returns true if this is a Mapped zone.
    pub fn is_mapped(&self) -> bool {
        matches!(self, Zone::Mapped(_))
    }

    /// Returns the current allocated bytes for Heap zones.
    /// Returns the file size for Mapped zones.
    pub fn allocated_bytes(&self) -> usize {
        match self {
            Zone::Heap(bump) => bump.allocated_bytes(),
            Zone::Mapped(mmap) => mmap.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    #[should_panic(expected = "Cannot allocate into a read-only Mapped Zone")]
    fn test_mapped_zone_alloc_panics() {
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        temp.write_all(b"test").unwrap();
        temp.flush().unwrap();

        let zone = Zone::new_mapped(temp.path()).unwrap();
        let _ = zone.alloc(42); // Should panic
    }

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

        let mut temp = tempfile::NamedTempFile::new().unwrap();
        temp.write_all(b"test").unwrap();
        temp.flush().unwrap();

        let mapped = Zone::new_mapped(temp.path()).unwrap();
        assert!(mapped.is_mapped());
        assert!(!mapped.is_heap());
    }
}
