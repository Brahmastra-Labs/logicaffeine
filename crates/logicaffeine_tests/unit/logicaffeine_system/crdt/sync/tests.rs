use super::*;
use logicaffeine_data::crdt::GCounter;

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
    let counter = GCounter::with_replica_id(1);
    let synced = Synced::new(counter, "test-get").await;

    synced.mutate(|c| c.increment(5)).await;
    synced.mutate(|c| c.increment(3)).await;

    let value = synced.get().await;
    assert_eq!(value.value(), 8);
}
