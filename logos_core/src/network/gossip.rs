//! Phase 52: GossipSub pub/sub for CRDT synchronization
//!
//! This module provides automatic CRDT replication over GossipSub.
//! When a CRDT is synced on a topic:
//! 1. Local changes are broadcast to all subscribers
//! 2. Remote changes are received and merged automatically

use crate::crdt::Merge;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::GCounter;

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
