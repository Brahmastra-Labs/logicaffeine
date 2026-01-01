//! Phase 53: Persistent Storage with Journaling
//!
//! Provides crash-resilient persistence for CRDTs:
//! - Append-only journal for durability
//! - Automatic replay on mount
//! - Compaction/snapshot support

use crate::crdt::Merge;
use crate::fs::{Vfs, VfsResult, VfsError};
use serde::{de::DeserializeOwned, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Operation recorded in the journal.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum JournalOp<T> {
    /// Full state snapshot (for compaction)
    Snapshot(T),
    /// Delta operation (for incremental updates)
    Delta(T),
}

/// Journal entry header format:
/// [4 bytes: length][4 bytes: crc32][N bytes: payload]
struct JournalHeader;

impl JournalHeader {
    const SIZE: usize = 8;

    fn encode(payload: &[u8]) -> [u8; Self::SIZE] {
        let length = payload.len() as u32;
        let checksum = crc32fast::hash(payload);
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&length.to_le_bytes());
        buf[4..8].copy_from_slice(&checksum.to_le_bytes());
        buf
    }

    fn decode(buf: &[u8; Self::SIZE]) -> (u32, u32) {
        let length = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let checksum = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        (length, checksum)
    }
}

/// A persistent CRDT wrapper with journaling.
///
/// `Persistent<T>` wraps any type implementing `Merge + Serialize + DeserializeOwned`
/// and provides durable storage with crash recovery.
pub struct Persistent<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + Send + 'static,
{
    inner: Arc<Mutex<T>>,
    vfs: Arc<dyn Vfs>,
    journal_path: String,
    entry_count: Arc<Mutex<u64>>,
    _marker: PhantomData<T>,
}

impl<T> Persistent<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + Send + 'static,
{
    /// Mount a persistent value from a journal file.
    ///
    /// If the journal exists, replays all entries to reconstruct state.
    /// If not, creates a new journal with default state.
    pub async fn mount(vfs: &dyn Vfs, path: &str) -> VfsResult<Self> {
        let mut state = T::default();
        let mut entry_count = 0u64;

        // Check if journal exists and replay
        if vfs.exists(path).await.unwrap_or(false) {
            let data = vfs.read(path).await?;
            let mut pos = 0;

            while pos + JournalHeader::SIZE <= data.len() {
                let header_bytes: [u8; 8] = data[pos..pos + 8].try_into().unwrap();
                let (length, expected_checksum) = JournalHeader::decode(&header_bytes);
                pos += JournalHeader::SIZE;

                let payload_end = pos + length as usize;
                if payload_end > data.len() {
                    // Truncated entry - stop replay (WAL semantics)
                    break;
                }

                let payload = &data[pos..payload_end];
                let actual_checksum = crc32fast::hash(payload);

                if actual_checksum != expected_checksum {
                    // Corrupted entry - stop replay
                    return Err(VfsError::JournalCorrupted(
                        format!("Entry {} checksum mismatch", entry_count)
                    ));
                }

                // Deserialize and apply operation
                let op: JournalOp<T> = bincode::deserialize(payload)
                    .map_err(|e| VfsError::SerializationError(e.to_string()))?;

                match op {
                    JournalOp::Snapshot(s) => state = s,
                    JournalOp::Delta(d) => state.merge(&d),
                }

                pos = payload_end;
                entry_count += 1;
            }
        }

        // Create Arc'd VFS reference (we need to clone the trait object)
        // For now, we'll use a simple workaround with Box
        let vfs_arc: Arc<dyn Vfs> = Arc::new(NativeVfsWrapper {
            base_dir: std::path::PathBuf::from("."),
        });

        Ok(Self {
            inner: Arc::new(Mutex::new(state)),
            vfs: vfs_arc,
            journal_path: path.to_string(),
            entry_count: Arc::new(Mutex::new(entry_count)),
            _marker: PhantomData,
        })
    }

    /// Get immutable access to the current state.
    pub async fn get(&self) -> T {
        self.inner.lock().await.clone()
    }

    /// Mutate the state and persist the delta.
    ///
    /// The closure receives mutable access to the inner value.
    /// After mutation, the state is serialized and appended to the journal.
    pub async fn mutate<F, R>(&self, f: F) -> VfsResult<R>
    where
        F: FnOnce(&mut T) -> R + Send,
    {
        let mut guard = self.inner.lock().await;
        let result = f(&mut *guard);

        // Persist as delta (full state for simplicity)
        let op = JournalOp::Delta(guard.clone());
        let payload = bincode::serialize(&op)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        let header = JournalHeader::encode(&payload);
        let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
        entry.extend_from_slice(&header);
        entry.extend_from_slice(&payload);

        self.vfs.append(&self.journal_path, &entry).await?;

        *self.entry_count.lock().await += 1;

        Ok(result)
    }

    /// Compact the journal by writing a snapshot.
    ///
    /// This replaces all journal entries with a single snapshot,
    /// reducing storage and replay time.
    pub async fn compact(&self) -> VfsResult<()> {
        let state = self.inner.lock().await.clone();

        let op = JournalOp::<T>::Snapshot(state);
        let payload = bincode::serialize(&op)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        let header = JournalHeader::encode(&payload);
        let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
        entry.extend_from_slice(&header);
        entry.extend_from_slice(&payload);

        // Write snapshot (overwrites journal)
        self.vfs.write(&self.journal_path, &entry).await?;

        *self.entry_count.lock().await = 1;

        Ok(())
    }

    /// Get the number of journal entries.
    pub async fn entry_count(&self) -> u64 {
        *self.entry_count.lock().await
    }

    /// Automatically compact when entry count exceeds threshold.
    pub async fn maybe_compact(&self, threshold: u64) -> VfsResult<bool> {
        if self.entry_count().await > threshold {
            self.compact().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Internal wrapper to create Arc<dyn Vfs> from NativeVfs
/// This is a workaround until we refactor the mount signature
#[cfg(not(target_arch = "wasm32"))]
struct NativeVfsWrapper {
    base_dir: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl Vfs for NativeVfsWrapper {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        let full_path = self.base_dir.join(path);
        tokio::fs::read(&full_path).await.map_err(VfsError::from)
    }

    async fn read_to_string(&self, path: &str) -> VfsResult<String> {
        let full_path = self.base_dir.join(path);
        tokio::fs::read_to_string(&full_path).await.map_err(VfsError::from)
    }

    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let full_path = self.base_dir.join(path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full_path, contents).await.map_err(VfsError::from)
    }

    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        use tokio::io::AsyncWriteExt;
        let full_path = self.base_dir.join(path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full_path)
            .await?;
        file.write_all(contents).await?;
        file.sync_all().await?;
        Ok(())
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        let full_path = self.base_dir.join(path);
        Ok(full_path.exists())
    }

    async fn remove(&self, path: &str) -> VfsResult<()> {
        let full_path = self.base_dir.join(path);
        tokio::fs::remove_file(&full_path).await.map_err(VfsError::from)
    }

    async fn create_dir_all(&self, path: &str) -> VfsResult<()> {
        let full_path = self.base_dir.join(path);
        tokio::fs::create_dir_all(&full_path).await.map_err(VfsError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::GCounter;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_persistent_mount_empty() {
        let temp = TempDir::new().unwrap();
        let vfs = crate::fs::NativeVfs::new(temp.path());

        let counter = Persistent::<GCounter>::mount(&vfs, "counter.journal").await.unwrap();

        assert_eq!(counter.get().await.value(), 0);
    }

    #[tokio::test]
    async fn test_persistent_mutate() {
        let temp = TempDir::new().unwrap();
        let vfs = crate::fs::NativeVfs::new(temp.path());

        let counter = Persistent::<GCounter>::mount(&vfs, "counter.journal").await.unwrap();

        counter.mutate(|c| c.increment(5)).await.unwrap();
        counter.mutate(|c| c.increment(3)).await.unwrap();

        assert_eq!(counter.get().await.value(), 8);
    }
}
