//! GossipSub Pub/Sub for CRDT Synchronization
//!
//! Provides automatic CRDT replication over libp2p's GossipSub protocol.
//! When a CRDT is synced on a topic, local changes are broadcast to all
//! subscribers, and remote changes are received and merged automatically.
//!
//! # Thread Safety
//!
//! Subscription state is managed via a global `SUBSCRIPTIONS` map protected
//! by a tokio Mutex. Multiple topics can be subscribed concurrently.
//!
//! # Features
//!
//! Requires the `networking` feature.
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_system::network::gossip;
//! # use serde::{Serialize, Deserialize};
//!
//! # #[derive(Serialize, Deserialize)]
//! # struct MyMessage { data: i32 }
//! # fn main() {}
//! # async fn example() {
//! // Subscribe to a topic and receive raw messages
//! let mut rx = gossip::subscribe("my-topic").await;
//! while let Some(bytes) = rx.recv().await {
//!     println!("Received {} bytes", bytes.len());
//! }
//!
//! // Publish a message
//! gossip::publish("my-topic", &MyMessage { data: 42 }).await;
//! # }
//! ```

use logicaffeine_data::crdt::Merge;
use crate::network::wire;
use once_cell::sync::Lazy;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Topic subscriptions: topic -> channel for incoming messages
static SUBSCRIPTIONS: Lazy<Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Subscribe to a topic. Returns a receiver for incoming messages.
///
/// This registers the subscription locally and forwards it to the mesh node.
/// The returned receiver will receive raw message bytes.
pub async fn subscribe(topic: &str) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(256);

    // Register subscription
    {
        let mut subs = SUBSCRIPTIONS.lock().await;
        subs.insert(topic.to_string(), tx);
    }

    // Forward subscription to mesh node
    crate::network::gossip_subscribe(topic).await;

    rx
}

/// Publish a message to a GossipSub topic.
///
/// The message is serialized with bincode and broadcast to all subscribers
/// on the mesh network.
pub async fn publish<T: Serialize>(topic: &str, data: &T) {
    let bytes = match wire::encode(data) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[gossip] Serialization failed: {:?}", e);
            return;
        }
    };

    // Forward to mesh node's gossipsub behaviour
    crate::network::gossip_publish(topic, bytes).await;
}

/// Publishes raw bytes (already encoded) to avoid double-encoding.
///
/// This is used internally by [`Distributed<T>`](crate::distributed::Distributed)
/// which serializes once for both journaling and network broadcast.
///
/// # Arguments
///
/// * `topic` - The topic name
/// * `data` - Pre-serialized bytes to publish
pub async fn publish_raw(topic: &str, data: Vec<u8>) -> Result<(), String> {
    crate::network::gossip_publish(topic, data).await;
    Ok(())
}

/// Returns the local peer ID as a string.
///
/// Used for echo detection to filter out self-published messages.
///
/// # Returns
///
/// `Some(peer_id)` if the mesh node is initialized, `None` otherwise.
pub async fn local_peer_id() -> Option<String> {
    match crate::network::local_peer_id().await {
        Ok(peer_id) => Some(peer_id.to_string()),
        Err(_) => None,
    }
}

/// Subscribe to a topic and auto-merge incoming messages.
///
/// This function blocks until the subscription is cancelled.
/// Incoming messages are deserialized and merged into the target.
pub async fn subscribe_and_merge<T: Merge + DeserializeOwned + Send + 'static>(
    topic: &str,
    target: Arc<Mutex<T>>,
) {
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(256);

    // Register subscription
    {
        let mut subs = SUBSCRIPTIONS.lock().await;
        subs.insert(topic.to_string(), tx);
    }

    // Forward subscription to mesh node
    crate::network::gossip_subscribe(topic).await;

    // Process incoming messages
    while let Some(bytes) = rx.recv().await {
        match wire::decode::<T>(&bytes) {
            Ok(incoming) => {
                let mut guard = target.lock().await;
                guard.merge(&incoming);
            }
            Err(e) => {
                eprintln!("[gossip] Deserialization failed: {:?}", e);
            }
        }
    }
}

/// Called by mesh node when a GossipSub message arrives.
///
/// Routes the message to the appropriate subscription channel.
pub async fn on_message(topic: &str, data: Vec<u8>) {
    // Test hook: log received messages
    #[cfg(test)]
    test_hooks::log_received(topic, &data);

    let subs = SUBSCRIPTIONS.lock().await;
    if let Some(tx) = subs.get(topic) {
        if tx.send(data).await.is_err() {
            eprintln!("[gossip] Failed to forward message to subscriber");
        }
    }
}

/// Unsubscribe from a topic.
///
/// This removes the subscription and stops receiving messages.
#[allow(dead_code)]
pub async fn unsubscribe(topic: &str) {
    let mut subs = SUBSCRIPTIONS.lock().await;
    subs.remove(topic);
    // Note: Should also tell mesh node to unsubscribe from gossipsub
}

// =============================================================================
// Test infrastructure (compiles out in release)
// =============================================================================

#[cfg(test)]
pub mod test_hooks {
    use once_cell::sync::Lazy;
    use std::sync::Mutex;

    pub struct MessageLog {
        pub received: Vec<(String, Vec<u8>)>,
    }

    static LOG: Lazy<Mutex<MessageLog>> = Lazy::new(|| {
        Mutex::new(MessageLog {
            received: Vec::new(),
        })
    });

    pub fn log_received(topic: &str, data: &[u8]) {
        if let Ok(mut log) = LOG.lock() {
            log.received.push((topic.to_string(), data.to_vec()));
        }
    }

    pub fn get_received() -> Vec<(String, Vec<u8>)> {
        LOG.lock().map(|l| l.received.clone()).unwrap_or_default()
    }

    pub fn clear_log() {
        if let Ok(mut log) = LOG.lock() {
            log.received.clear();
        }
    }

    pub fn received_count() -> usize {
        LOG.lock().map(|l| l.received.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logicaffeine_data::crdt::GCounter;

    #[tokio::test]
    async fn test_subscriptions_registry() {
        let counter = Arc::new(Mutex::new(GCounter::new()));

        // Spawn a subscription task
        let topic = "test-sub";
        let counter_clone = Arc::clone(&counter);
        let handle = tokio::spawn(async move {
            // This would block forever in real use, but we'll cancel it
            tokio::select! {
                _ = subscribe_and_merge::<GCounter>(topic, counter_clone) => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        });

        // Wait a bit for subscription to register
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Check subscription exists
        let subs = SUBSCRIPTIONS.lock().await;
        assert!(subs.contains_key(topic), "Subscription should be registered");
        drop(subs);

        // Cleanup
        handle.abort();
    }
}
