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
    assert!(
        subs.contains_key(topic),
        "Subscription should be registered"
    );
    drop(subs);

    // Cleanup
    handle.abort();
}
