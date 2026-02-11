//! Distributed CRDT with Unified Persistence and Network Synchronization
//!
//! [`Distributed<T>`] is the mesh-journal bridge: every network event is journaled,
//! and every local change is broadcast. This solves the "data loss on restart" bug
//! where in-memory sync loses remote updates on page reload.
//!
//! # Data Flow
//!
//! ```text
//! Local mutation:   RAM → Journal → Network
//! Remote update:    Network → RAM → Journal
//! ```
//!
//! # Features
//!
//! Requires both `networking` and `persistence` features.
//!
//! # Platform Support
//!
//! - **Native**: Full support with GossipSub network sync
//! - **WASM**: Disk-only mode (network sync via WebSocket relay is future work)
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_system::distributed::Distributed;
//! use logicaffeine_data::crdt::GCounter;
//! # use logicaffeine_system::fs::NativeVfs;
//! # use std::sync::Arc;
//!
//! # fn main() {}
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let vfs: Arc<dyn logicaffeine_system::fs::Vfs + Send + Sync> = Arc::new(NativeVfs::new("/data"));
//! // Disk-only (same as Persistent<T>)
//! let counter = Distributed::<GCounter>::mount(vfs.clone(), "counter.lsf", None).await?;
//!
//! // Disk + Network sync
//! let counter = Distributed::<GCounter>::mount(
//!     vfs,
//!     "counter.lsf",
//!     Some("game-scores".into())
//! ).await?;
//!
//! // Mutations are persisted AND broadcast
//! counter.mutate(|c| c.increment(1)).await?;
//! # Ok(())
//! # }
//! ```

use logicaffeine_data::crdt::Merge;
use crate::fs::{Vfs, VfsResult, VfsError};
use async_lock::Mutex;
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;

/// Compaction threshold: auto-compact when journal exceeds this many entries.
const COMPACT_THRESHOLD: u64 = 1000;

/// Operation recorded in the journal (shared with storage module).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum JournalOp<T> {
    /// Full state snapshot (for compaction)
    Snapshot(T),
    /// Delta operation (for incremental updates)
    Delta(T),
}

/// Journal entry header format: [4 bytes: length][4 bytes: crc32][N bytes: payload]
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

/// Replay journal entries to reconstruct state.
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

// =============================================================================
// Native Implementation (with network support)
// =============================================================================

/// A distributed, persistent CRDT wrapper.
///
/// Combines:
/// 1. In-Memory State (RAM)
/// 2. Append-Only Journal (Disk)
/// 3. Pub/Sub Synchronization (Network) - optional
///
/// When `topic` is Some, the value is synchronized via GossipSub.
/// When `topic` is None, it behaves identically to `Persistent<T>`.
#[cfg(not(target_arch = "wasm32"))]
pub struct Distributed<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + Send + 'static,
{
    inner: Arc<Mutex<T>>,
    vfs: Arc<dyn Vfs + Send + Sync>,
    journal_path: String,
    topic: Option<String>,
    entry_count: Arc<Mutex<u64>>,
    local_peer_id: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> Distributed<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + Send + 'static,
{
    /// Mount from disk, optionally sync to network.
    ///
    /// # Arguments
    /// * `vfs` - Virtual filesystem for persistence
    /// * `path` - Journal file path
    /// * `topic` - GossipSub topic (None for disk-only)
    ///
    /// # Example
    /// ```no_run
    /// # use logicaffeine_system::distributed::Distributed;
    /// # use logicaffeine_data::crdt::GCounter;
    /// # use logicaffeine_system::fs::NativeVfs;
    /// # use std::sync::Arc;
    /// # fn main() {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let vfs: Arc<dyn logicaffeine_system::fs::Vfs + Send + Sync> = Arc::new(NativeVfs::new("/data"));
    /// // Disk-only (same as Persistent<T>)
    /// let counter = Distributed::<GCounter>::mount(vfs.clone(), "counter.lsf", None).await?;
    ///
    /// // Disk + Network
    /// let counter = Distributed::<GCounter>::mount(vfs, "counter.lsf", Some("game-scores".into())).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn mount(
        vfs: Arc<dyn Vfs + Send + Sync>,
        path: &str,
        topic: Option<String>,
    ) -> VfsResult<Self> {
        // 1. Replay journal to reconstruct state
        let (state, entry_count) = if vfs.exists(path).await.unwrap_or(false) {
            let data = vfs.read(path).await?;
            replay_journal(&data)?
        } else {
            (T::default(), 0)
        };

        // 2. Get local peer ID for echo detection
        let local_peer_id = crate::network::gossip::local_peer_id().await;

        let dist = Self {
            inner: Arc::new(Mutex::new(state)),
            vfs,
            journal_path: path.to_string(),
            topic: topic.clone(),
            entry_count: Arc::new(Mutex::new(entry_count)),
            local_peer_id,
        };

        // 3. Subscribe to network and spawn receive loop
        if let Some(ref topic_name) = topic {
            dist.start_receive_loop(topic_name.clone());
        }

        Ok(dist)
    }

    /// Local mutation: RAM → Journal → Network
    ///
    /// After applying the mutation:
    /// 1. Writes a delta to the journal
    /// 2. Broadcasts the state to network peers (if synced)
    /// 3. Auto-compacts if entry count exceeds threshold
    pub async fn mutate<F, R>(&self, f: F) -> VfsResult<R>
    where
        F: FnOnce(&mut T) -> R + Send,
    {
        // 1. Apply to RAM
        let mut guard = self.inner.lock().await;
        let result = f(&mut *guard);
        let snapshot = (*guard).clone();
        drop(guard);

        // 2. Persist delta to Journal
        self.append_to_journal(&snapshot).await?;

        // 3. Broadcast to Network (if synced)
        if let Some(topic) = &self.topic {
            let bytes = bincode::serialize(&snapshot)
                .map_err(|e| VfsError::SerializationError(e.to_string()))?;
            // Fire and forget - network failures shouldn't block local operations
            let _ = crate::network::gossip::publish_raw(topic, bytes).await;
        }

        // 4. Auto-compact if needed
        self.maybe_compact().await?;

        Ok(result)
    }

    /// Get current state (clone).
    pub async fn get(&self) -> T {
        self.inner.lock().await.clone()
    }

    /// Manual compaction.
    pub async fn compact(&self) -> VfsResult<()> {
        let snapshot = self.inner.lock().await.clone();
        self.do_compact(&snapshot).await
    }

    /// Get the number of journal entries.
    pub async fn entry_count(&self) -> u64 {
        *self.entry_count.lock().await
    }

    // === Internal ===

    /// Start the background receive loop for network messages.
    fn start_receive_loop(&self, topic: String) {
        let inner = self.inner.clone();
        let vfs = self.vfs.clone();
        let path = self.journal_path.clone();
        let local_id = self.local_peer_id.clone();
        let entry_count = self.entry_count.clone();

        tokio::spawn(async move {
            let mut rx = crate::network::gossip::subscribe(&topic).await;

            while let Some(bytes) = rx.recv().await {
                // Deserialize
                let delta: T = match bincode::deserialize(&bytes) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("[Distributed] Deserialize failed: {:?}", e);
                        continue;
                    }
                };

                // Merge to RAM
                inner.lock().await.merge(&delta);

                // CRITICAL: Persist to Journal
                // This ensures remote updates survive restarts
                let op = JournalOp::Delta(delta);
                let payload = match bincode::serialize(&op) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("[Distributed] Serialize failed: {:?}", e);
                        continue;
                    }
                };

                let header = JournalHeader::encode(&payload);
                let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
                entry.extend_from_slice(&header);
                entry.extend_from_slice(&payload);

                if let Err(e) = vfs.append(&path, &entry).await {
                    eprintln!("[Distributed] Journal append failed: {:?}", e);
                } else {
                    *entry_count.lock().await += 1;
                }
            }

            // Suppress unused variable warning
            let _ = local_id;
        });
    }

    /// Append state as delta to journal.
    async fn append_to_journal(&self, state: &T) -> VfsResult<()> {
        let op = JournalOp::Delta(state.clone());
        let payload = bincode::serialize(&op)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        let header = JournalHeader::encode(&payload);
        let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
        entry.extend_from_slice(&header);
        entry.extend_from_slice(&payload);

        self.vfs.append(&self.journal_path, &entry).await?;
        *self.entry_count.lock().await += 1;

        Ok(())
    }

    /// Auto-compact if entry count exceeds threshold.
    async fn maybe_compact(&self) -> VfsResult<()> {
        let count = *self.entry_count.lock().await;
        if count >= COMPACT_THRESHOLD {
            let snapshot = self.inner.lock().await.clone();
            self.do_compact(&snapshot).await?;
        }
        Ok(())
    }

    /// Write snapshot and reset journal.
    async fn do_compact(&self, snapshot: &T) -> VfsResult<()> {
        let op = JournalOp::Snapshot(snapshot.clone());
        let payload = bincode::serialize(&op)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        let header = JournalHeader::encode(&payload);
        let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
        entry.extend_from_slice(&header);
        entry.extend_from_slice(&payload);

        // Atomic: write temp → rename
        let temp_path = format!("{}.tmp", self.journal_path);
        self.vfs.write(&temp_path, &entry).await?;
        self.vfs.rename(&temp_path, &self.journal_path).await?;

        *self.entry_count.lock().await = 1;
        Ok(())
    }
}

// =============================================================================
// WASM Implementation (disk-only, no network)
// =============================================================================

/// WASM version of Distributed<T>.
///
/// On WASM, network sync is not available (libp2p requires native).
/// This behaves identically to `Persistent<T>` - topic is ignored.
#[cfg(target_arch = "wasm32")]
pub struct Distributed<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + 'static,
{
    inner: Arc<Mutex<T>>,
    vfs: Arc<dyn Vfs>,
    journal_path: String,
    entry_count: Arc<Mutex<u64>>,
}

#[cfg(target_arch = "wasm32")]
impl<T> Distributed<T>
where
    T: Merge + Serialize + DeserializeOwned + Clone + Default + 'static,
{
    /// Mount from disk (network sync not available on WASM).
    ///
    /// The `topic` parameter is ignored - WASM doesn't support libp2p.
    /// For WASM network sync, use WebSocket relay (future work).
    pub async fn mount(
        vfs: Arc<dyn Vfs>,
        path: &str,
        _topic: Option<String>,  // Ignored on WASM
    ) -> VfsResult<Self> {
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
        })
    }

    /// Local mutation: RAM → Journal (no network on WASM).
    pub async fn mutate<F, R>(&self, f: F) -> VfsResult<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut guard = self.inner.lock().await;
        let result = f(&mut *guard);
        let snapshot = (*guard).clone();
        drop(guard);

        // Persist delta to Journal
        self.append_to_journal(&snapshot).await?;

        // Auto-compact if needed
        self.maybe_compact().await?;

        Ok(result)
    }

    /// Get current state (clone).
    pub async fn get(&self) -> T {
        self.inner.lock().await.clone()
    }

    /// Manual compaction.
    pub async fn compact(&self) -> VfsResult<()> {
        let snapshot = self.inner.lock().await.clone();
        self.do_compact(&snapshot).await
    }

    /// Get the number of journal entries.
    pub async fn entry_count(&self) -> u64 {
        *self.entry_count.lock().await
    }

    async fn append_to_journal(&self, state: &T) -> VfsResult<()> {
        let op = JournalOp::Delta(state.clone());
        let payload = bincode::serialize(&op)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        let header = JournalHeader::encode(&payload);
        let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
        entry.extend_from_slice(&header);
        entry.extend_from_slice(&payload);

        self.vfs.append(&self.journal_path, &entry).await?;
        *self.entry_count.lock().await += 1;

        Ok(())
    }

    async fn maybe_compact(&self) -> VfsResult<()> {
        let count = *self.entry_count.lock().await;
        if count >= COMPACT_THRESHOLD {
            let snapshot = self.inner.lock().await.clone();
            self.do_compact(&snapshot).await?;
        }
        Ok(())
    }

    async fn do_compact(&self, snapshot: &T) -> VfsResult<()> {
        let op = JournalOp::Snapshot(snapshot.clone());
        let payload = bincode::serialize(&op)
            .map_err(|e| VfsError::SerializationError(e.to_string()))?;

        let header = JournalHeader::encode(&payload);
        let mut entry = Vec::with_capacity(JournalHeader::SIZE + payload.len());
        entry.extend_from_slice(&header);
        entry.extend_from_slice(&payload);

        // Atomic: write temp → rename
        let temp_path = format!("{}.tmp", self.journal_path);
        self.vfs.write(&temp_path, &entry).await?;
        self.vfs.rename(&temp_path, &self.journal_path).await?;

        *self.entry_count.lock().await = 1;
        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use logicaffeine_data::crdt::GCounter;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_distributed_mount_empty() {
        let temp = TempDir::new().unwrap();
        let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

        let counter = Distributed::<GCounter>::mount(vfs, "counter.lsf", None).await.unwrap();

        assert_eq!(counter.get().await.value(), 0);
    }

    #[tokio::test]
    async fn test_distributed_mutate() {
        let temp = TempDir::new().unwrap();
        let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

        let counter = Distributed::<GCounter>::mount(vfs, "counter.lsf", None).await.unwrap();

        counter.mutate(|c| c.increment(5)).await.unwrap();
        counter.mutate(|c| c.increment(3)).await.unwrap();

        assert_eq!(counter.get().await.value(), 8);
    }

    #[tokio::test]
    async fn test_distributed_persist_and_reload() {
        let temp = TempDir::new().unwrap();
        let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

        // Create and mutate
        {
            let counter = Distributed::<GCounter>::mount(vfs.clone(), "counter.lsf", None).await.unwrap();
            counter.mutate(|c| c.increment(10)).await.unwrap();
            assert_eq!(counter.get().await.value(), 10);
        }

        // Reload and verify
        {
            let counter = Distributed::<GCounter>::mount(vfs.clone(), "counter.lsf", None).await.unwrap();
            assert_eq!(counter.get().await.value(), 10);
        }
    }

    #[tokio::test]
    async fn test_distributed_compaction() {
        let temp = TempDir::new().unwrap();
        let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

        let counter = Distributed::<GCounter>::mount(vfs.clone(), "counter.lsf", None).await.unwrap();

        // Mutate multiple times
        for _ in 0..5 {
            counter.mutate(|c| c.increment(1)).await.unwrap();
        }
        assert_eq!(counter.entry_count().await, 5);

        // Compact
        counter.compact().await.unwrap();
        assert_eq!(counter.entry_count().await, 1);

        // Value should be preserved
        assert_eq!(counter.get().await.value(), 5);

        // Reload should still work
        let counter2 = Distributed::<GCounter>::mount(vfs, "counter.lsf", None).await.unwrap();
        assert_eq!(counter2.get().await.value(), 5);
    }
}
