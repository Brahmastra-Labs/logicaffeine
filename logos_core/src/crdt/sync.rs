//! Phase 52: Synced wrapper for automatic CRDT replication
//!
//! The `Synced<T>` wrapper provides automatic GossipSub-based replication
//! for any type that implements `Merge + Serialize + DeserializeOwned`.
//!
//! When a `Synced<T>` is mutated, the change is automatically broadcast
//! to all subscribers on the same topic. When a message is received,
//! it's automatically merged into the local state.

use super::Merge;
use crate::network::gossip;
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

/// A synced CRDT that automatically replicates over GossipSub.
///
/// # Example
///
/// ```ignore
/// let counter = GCounter::new();
/// let synced = Synced::new(counter, "game-scores").await;
///
/// // Mutations are automatically broadcast
/// synced.mutate(|c| c.increment(5)).await;
/// ```
pub struct Synced<T: Merge + Serialize + DeserializeOwned + Clone + Send + 'static> {
    inner: Arc<Mutex<T>>,
    topic: String,
}

impl<T: Merge + Serialize + DeserializeOwned + Clone + Send + 'static> Synced<T> {
    /// Create a new synced wrapper and subscribe to the topic.
    ///
    /// This spawns a background task that:
    /// 1. Subscribes to the GossipSub topic
    /// 2. Listens for incoming messages
    /// 3. Deserializes and merges them into the local state
    pub async fn new(initial: T, topic: &str) -> Self {
        let inner = Arc::new(Mutex::new(initial));
        let topic_str = topic.to_string();

        // Spawn background merge task
        let inner_clone = Arc::clone(&inner);
        let topic_clone = topic_str.clone();
        tokio::spawn(async move {
            gossip::subscribe_and_merge::<T>(&topic_clone, inner_clone).await;
        });

        Self {
            inner,
            topic: topic_str,
        }
    }

    /// Get mutable access to the inner value, publishing after mutation.
    ///
    /// The closure receives a mutable reference to the inner value.
    /// After the closure returns, the full state is broadcast to the topic.
    pub async fn mutate<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut guard = self.inner.lock().await;
        let result = f(&mut *guard);

        // Publish full state after mutation
        let state = guard.clone();
        drop(guard); // Release lock before async publish

        gossip::publish(&self.topic, &state).await;

        result
    }

    /// Get immutable access to the current state.
    ///
    /// Returns a clone of the current state. For frequent reads,
    /// consider using `mutate` to batch operations.
    pub async fn get(&self) -> T {
        self.inner.lock().await.clone()
    }

    /// Get the topic this CRDT is synchronized on.
    pub fn topic(&self) -> &str {
        &self.topic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::GCounter;

    #[tokio::test]
    async fn test_synced_creation() {
        let counter = GCounter::new();
        let synced = Synced::new(counter, "test-topic").await;
        assert_eq!(synced.topic(), "test-topic");
    }

    #[tokio::test]
    async fn test_synced_mutate() {
        let counter = GCounter::new();
        let synced = Synced::new(counter, "test-mutate").await;

        synced.mutate(|c| c.increment(10)).await;

        let value = synced.get().await;
        assert_eq!(value.value(), 10);
    }

    #[tokio::test]
    async fn test_synced_get() {
        let counter = GCounter::with_replica_id("node1".to_string());
        let synced = Synced::new(counter, "test-get").await;

        synced.mutate(|c| c.increment(5)).await;
        synced.mutate(|c| c.increment(3)).await;

        let value = synced.get().await;
        assert_eq!(value.value(), 8);
    }
}
