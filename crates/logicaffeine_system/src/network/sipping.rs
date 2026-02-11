//! Sipping Protocol: Zero-copy file chunking for resumable transfers.
//!
//! The Sipping protocol slices memory-mapped files into chunks with SHA256 hashes,
//! enabling resumable, verifiable file transfers over unreliable networks.

use crate::memory::Zone;
use sha2::{Sha256, Digest};
use serde::{Serialize, Deserialize};

/// Default chunk size: 1 MB
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

/// Manifest describing a file's chunks for resumable transfer.
///
/// The manifest contains:
/// - A unique file ID for this transfer session
/// - Total file size and chunk count
/// - SHA256 hashes for each chunk (enables verification and deduplication)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileManifest {
    pub file_id: String,
    pub total_size: u64,
    pub chunk_size: usize,
    pub chunk_count: usize,
    pub chunk_hashes: Vec<[u8; 32]>,
}

/// A single chunk of file data with its hash for verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunk {
    pub file_id: String,
    pub index: usize,
    pub data: Vec<u8>,
    pub hash: [u8; 32],
}

/// Zero-copy file chunking using memory-mapped zones.
///
/// FileSipper wraps a memory-mapped Zone and provides:
/// - Zero-copy chunk access via slicing
/// - SHA256 hashing for verification
/// - Manifest generation for resumable transfers
///
/// # Example
/// ```no_run
/// # use logicaffeine_system::memory::Zone;
/// # use logicaffeine_system::network::FileSipper;
/// # fn main() -> Result<(), std::io::Error> {
/// let zone = Zone::new_mapped("large_file.bin")?;
/// let sipper = FileSipper::from_zone(&zone);
/// let manifest = sipper.manifest();
/// let chunk = sipper.get_chunk(0);
/// # Ok(())
/// # }
/// ```
pub struct FileSipper<'a> {
    zone: &'a Zone,
    chunk_size: usize,
    file_id: String,
}

impl<'a> FileSipper<'a> {
    /// Create sipper from a mapped zone with default chunk size (1 MB).
    pub fn from_zone(zone: &'a Zone) -> Self {
        Self {
            zone,
            chunk_size: DEFAULT_CHUNK_SIZE,
            file_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// Create sipper with custom chunk size.
    pub fn with_chunk_size(zone: &'a Zone, chunk_size: usize) -> Self {
        Self {
            zone,
            chunk_size,
            file_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// Get the file ID for this sipper session.
    pub fn file_id(&self) -> &str {
        &self.file_id
    }

    /// Get number of chunks in the file.
    pub fn chunk_count(&self) -> usize {
        let size = self.zone.allocated_bytes();
        if size == 0 {
            0
        } else {
            (size + self.chunk_size - 1) / self.chunk_size
        }
    }

    /// Zero-copy slice of chunk at index (0-indexed).
    ///
    /// Returns the raw bytes of the chunk without copying.
    /// The last chunk may be smaller than chunk_size.
    pub fn get_chunk(&self, index: usize) -> &[u8] {
        let slice = self.zone.as_slice();
        let start = index * self.chunk_size;
        let end = (start + self.chunk_size).min(slice.len());
        if start >= slice.len() {
            &[]
        } else {
            &slice[start..end]
        }
    }

    /// Compute SHA256 hash of a specific chunk.
    pub fn hash_chunk(&self, index: usize) -> [u8; 32] {
        let chunk = self.get_chunk(index);
        let mut hasher = Sha256::new();
        hasher.update(chunk);
        hasher.finalize().into()
    }

    /// Generate manifest with all chunk hashes.
    ///
    /// The manifest enables:
    /// - Resumable transfers (client can request missing chunks)
    /// - Verification (client can verify each chunk's hash)
    /// - Deduplication (identical chunks have identical hashes)
    pub fn manifest(&self) -> FileManifest {
        let slice = self.zone.as_slice();
        let chunk_count = self.chunk_count();
        let hashes: Vec<[u8; 32]> = (0..chunk_count)
            .map(|i| self.hash_chunk(i))
            .collect();

        FileManifest {
            file_id: self.file_id.clone(),
            total_size: slice.len() as u64,
            chunk_size: self.chunk_size,
            chunk_count,
            chunk_hashes: hashes,
        }
    }

    /// Get chunk as FileChunk struct (includes hash for verification).
    ///
    /// This copies the data into the FileChunk. Use `get_chunk()` for
    /// zero-copy access when you don't need the hash included.
    pub fn get_chunk_with_hash(&self, index: usize) -> FileChunk {
        let data = self.get_chunk(index).to_vec();
        let hash = self.hash_chunk(index);
        FileChunk {
            file_id: self.file_id.clone(),
            index,
            data,
            hash,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
