//! Persistent Storage with Journaling
//!
//! Provides crash-resilient persistence for CRDTs using an append-only journal
//! with automatic replay on mount and compaction support.
//!
//! # Architecture
//!
//! The journal uses a simple binary format:
//!
//! ```text
//! ┌─────────────┬─────────────┬─────────────────┐
//! │ Length (4B) │ CRC32 (4B)  │ Payload (N B)   │
//! └─────────────┴─────────────┴─────────────────┘
//! ```
//!
//! Entries are either:
//! - **Snapshot**: Full state replacement (written during compaction)
//! - **Delta**: Incremental update (written on each mutation)
//!
//! # Recovery
//!
//! On mount, the journal is replayed entry-by-entry:
//! - Snapshots replace the current state
//! - Deltas are merged via the CRDT's `Merge` trait
//! - Truncated entries are ignored (WAL semantics)
//! - Checksum failures return an error
//!
//! # Features
//!
//! Requires the `persistence` feature.
//!
//! # Example
//!
//! ```rust,ignore
//! use logicaffeine_system::storage::Persistent;
//! use logicaffeine_data::crdt::GCounter;
//!
//! let vfs = Arc::new(NativeVfs::new("/data"));
//! let counter = Persistent::<GCounter>::mount(vfs, "counter.lsf").await?;
//!
//! counter.mutate(|c| c.increment(5)).await?;
//! counter.compact().await?; // Reduce journal size
//! ```

use logicaffeine_data::crdt::Merge;
use crate::fs::{Vfs, VfsResult, VfsError};
use serde::{de::DeserializeOwned, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;
use async_lock::Mutex;

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

/// A persistent CRDT wrapper with journaling (native platform).
///
/// Wraps any type implementing `Merge + Serialize + DeserializeOwned`
/// and provides durable storage with crash recovery.
///
/// This is the native version with `Send` bounds required for thread-safe access.
/// The WASM version relaxes these bounds since JavaScript is single-threaded.
#[cfg(not(target_arch = "wasm32"))]
pub struct Persistent<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + Send + 'static,
{
    inner: Arc<Mutex<T>>,
    vfs: Arc<dyn Vfs + Send + Sync>,
    journal_path: String,
    entry_count: Arc<Mutex<u64>>,
    _marker: PhantomData<T>,
}

/// WASM version without Send bounds (JS is single-threaded).
#[cfg(target_arch = "wasm32")]
pub struct Persistent<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + 'static,
{
    inner: Arc<Mutex<T>>,
    vfs: Arc<dyn Vfs>,
    journal_path: String,
    entry_count: Arc<Mutex<u64>>,
    _marker: PhantomData<T>,
}

/// Helper function to replay journal entries (shared between platforms).
fn replay_journal<T>(data: &[u8]) -> Result<(T, u64), VfsError>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default,
{
    let mut state = T::default();
    let mut entry_count = 0u64;
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
            return Err(VfsError::JournalCorrupted(
                format!("Entry {} checksum mismatch", entry_count)
            ));
        }

        let op: JournalOp<T> = bincode::deserialize(payload)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        match op {
            JournalOp::Snapshot(s) => state = s,
            JournalOp::Delta(d) => state.merge(&d),
        }

        pos = payload_end;
        entry_count += 1;
    }

    Ok((state, entry_count))
}

/// Native implementation with Send bounds.
#[cfg(not(target_arch = "wasm32"))]
impl<T> Persistent<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + Send + 'static,
{
    /// Mount a persistent value from a journal file.
    ///
    /// If the journal exists, replays all entries to reconstruct state.
    /// If not, creates a new journal with default state.
    ///
        pub async fn mount(vfs: Arc<dyn Vfs + Send + Sync>, path: &str) -> VfsResult<Self> {
        let (state, entry_count) = if vfs.exists(path).await.unwrap_or(false) {
            let data = vfs.read(path).await?;
            replay_journal(&data)?
        } else {
            (T::default(), 0)
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(state)),
            vfs,
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
    pub async fn mutate<F, R>(&self, f: F) -> VfsResult<R>
    where
        F: FnOnce(&mut T) -> R + Send,
    {
        let mut guard = self.inner.lock().await;
        let result = f(&mut *guard);

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
    pub async fn compact(&self) -> VfsResult<()> {
        let state = self.inner.lock().await.clone();

        let op = JournalOp::<T>::Snapshot(state);
        let payload = bincode::serialize(&op)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        let header = JournalHeader::encode(&payload);
        let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
        entry.extend_from_slice(&header);
        entry.extend_from_slice(&payload);

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

/// WASM implementation without Send bounds (single-threaded).
#[cfg(target_arch = "wasm32")]
impl<T> Persistent<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + 'static,
{
    /// Mount a persistent value from a journal file.
    pub async fn mount(vfs: Arc<dyn Vfs>, path: &str) -> VfsResult<Self> {
        let (state, entry_count) = if vfs.exists(path).await.unwrap_or(false) {
            let data = vfs.read(path).await?;
            replay_journal(&data)?
        } else {
            (T::default(), 0)
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(state)),
            vfs,
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
    pub async fn mutate<F, R>(&self, f: F) -> VfsResult<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut guard = self.inner.lock().await;
        let result = f(&mut *guard);

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
    pub async fn compact(&self) -> VfsResult<()> {
        let state = self.inner.lock().await.clone();

        let op = JournalOp::<T>::Snapshot(state);
        let payload = bincode::serialize(&op)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        let header = JournalHeader::encode(&payload);
        let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
        entry.extend_from_slice(&header);
        entry.extend_from_slice(&payload);

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

// The storage module directly accepts Arc<dyn Vfs> for flexibility.

#[cfg(test)]
mod tests {
    use super::*;
    use logicaffeine_data::crdt::GCounter;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_persistent_mount_empty() {
        let temp = TempDir::new().unwrap();
        let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

        let counter = Persistent::<GCounter>::mount(vfs, "counter.journal").await.unwrap();

        assert_eq!(counter.get().await.value(), 0);
    }

    #[tokio::test]
    async fn test_persistent_mutate() {
        let temp = TempDir::new().unwrap();
        let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

        let counter = Persistent::<GCounter>::mount(vfs, "counter.journal").await.unwrap();

        counter.mutate(|c| c.increment(5)).await.unwrap();
        counter.mutate(|c| c.increment(3)).await.unwrap();

        assert_eq!(counter.get().await.value(), 8);
    }
}
