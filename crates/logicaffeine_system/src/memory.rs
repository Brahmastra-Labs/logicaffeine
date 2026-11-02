//! Zone-based Memory Management
//!
//! Zones provide region-based allocation with O(1) allocation and bulk deallocation.
//! This is the LOGOS implementation of memory arenas. Every reference a zone hands out borrows
//! the zone, so none can outlive it, and dropping or resetting the zone frees the whole region
//! at once.
//!
//! # Backing Strategies
//!
//! - **Heap**: Fast arena allocation via bumpalo for temporary allocations
//! - **Mapped**: Zero-copy file mapping via memmap2 for read-only file access
//!
//! # Features
//!
//! - `concurrency`: Always required (this module is gated on it)
//! - `persistence`: Required for memory-mapped file support
//!
//! # Safety
//!
//! Memory-mapped zones (`Zone::Mapped`) have standard mmap safety caveats:
//! the underlying file should not be modified by other processes while mapped.
//! Concurrent modification may cause undefined behavior.
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_system::memory::Zone;
//!
//! # fn main() -> Result<(), std::io::Error> {
//! // Heap zone for temporary allocations
//! let zone = Zone::new_heap(1024 * 1024); // 1 MB arena
//! let x = zone.alloc(42);
//! let slice = zone.alloc_slice(&[1, 2, 3, 4, 5]);
//!
//! // Mapped zone for zero-copy file access (requires persistence feature)
//! let zone = Zone::new_mapped("data.bin")?;
//! let bytes = zone.as_slice();
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "persistence")]
use std::fs::File;
#[cfg(feature = "persistence")]
use std::io;
#[cfg(feature = "persistence")]
use std::path::Path;

/// A memory region for batch allocation and bulk deallocation.
///
/// Every reference a zone hands out borrows the zone, so none can outlive it, and the whole
/// region is freed at once when the zone goes out of scope.
pub enum Zone {
    /// Dynamic heap-allocated arena (Scratchpad).
    /// Use for temporary allocations that can be bulk-freed.
    Heap(bumpalo::Bump),
    /// Memory-mapped file (Zero-copy IO).
    /// Provides read-only access to file contents without loading into memory.
    /// Only available with the `persistence` feature.
    #[cfg(feature = "persistence")]
    Mapped(memmap2::Mmap),
}

impl Zone {
    /// Create a new empty zone on the heap with pre-sized capacity.
    ///
    /// # Example
    /// ```
    /// use logicaffeine_system::memory::Zone;
    ///
    /// let zone = Zone::new_heap(1024 * 1024); // 1 MB arena
    /// let x = zone.alloc(42);
    /// assert_eq!(*x, 42);
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
    /// ```no_run
    /// use logicaffeine_system::memory::Zone;
    ///
    /// # fn main() -> Result<(), std::io::Error> {
    /// let zone = Zone::new_mapped("data.bin")?;
    /// let bytes = zone.as_slice();
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Only available with the `persistence` feature.
    #[cfg(feature = "persistence")]
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
            #[cfg(feature = "persistence")]
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
            #[cfg(feature = "persistence")]
            Zone::Mapped(_) => panic!(
                "Cannot allocate into a read-only Mapped Zone. \
                 Use Zone::new_heap() for allocations."
            ),
        }
    }

    /// Get a reference to the mapped memory as a byte slice.
    ///
    /// Only valid for Mapped zones. Only available with `persistence` feature.
    ///
    /// # Panics
    /// Panics if called on a Heap zone.
    #[cfg(feature = "persistence")]
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
    #[cfg(feature = "persistence")]
    pub fn is_mapped(&self) -> bool {
        matches!(self, Zone::Mapped(_))
    }

    /// Returns true if this is a Mapped zone.
    /// Always returns false when persistence feature is disabled.
    #[cfg(not(feature = "persistence"))]
    pub fn is_mapped(&self) -> bool {
        false
    }

    /// Returns the current allocated bytes for Heap zones.
    /// Returns the file size for Mapped zones.
    pub fn allocated_bytes(&self) -> usize {
        match self {
            Zone::Heap(bump) => bump.allocated_bytes(),
            #[cfg(feature = "persistence")]
            Zone::Mapped(mmap) => mmap.len(),
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/memory/tests.rs"]
mod tests;
