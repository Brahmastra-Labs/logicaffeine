use super::*;

/// Two clients on one relay: a subscriber receives a publisher's message —
/// the browser↔browser-via-relay path, end to end.
#[tokio::test]
async fn relay_pubsub_roundtrip() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();

    let mut sub = RelayClient::connect(&url).await.expect("subscriber dials");
    let pubr = RelayClient::connect(&url).await.expect("publisher dials");

    sub.subscribe("chat").await.expect("subscribe acked");
    pubr.publish("chat", b"hello".to_vec()).expect("publish");

    let (topic, data) = tokio::time::timeout(std::time::Duration::from_secs(5), sub.next_event())
        .await
        .expect("event arrives in time")
        .expect("event present");
    assert_eq!(topic, "chat");
    assert_eq!(data, b"hello");
}

/// A message on a topic nobody subscribes to reaches no one, and a second
/// topic is isolated from the first.
#[tokio::test]
async fn relay_topics_are_isolated() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut a = RelayClient::connect(&url).await.unwrap();
    let b = RelayClient::connect(&url).await.unwrap();

    a.subscribe("alpha").await.unwrap();
    b.publish("beta", b"nope".to_vec()).unwrap(); // different topic
    b.publish("alpha", b"yes".to_vec()).unwrap();

    let (topic, data) = tokio::time::timeout(std::time::Duration::from_secs(5), a.next_event())
        .await
        .expect("event arrives")
        .unwrap();
    // The first event a receives must be the `alpha` one — `beta` was never
    // delivered to it.
    assert_eq!(topic, "alpha");
    assert_eq!(data, b"yes");
}

/// The server-side `inject` hook (used by the gossip bridge) delivers to WS
/// subscribers exactly like a client publish.
#[tokio::test]
async fn relay_inject_reaches_subscribers() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let mut sub = RelayClient::connect(&relay.url()).await.unwrap();
    sub.subscribe("mesh").await.unwrap();

    let delivered = relay.inject("mesh", b"from-gossip").await;
    assert_eq!(
        delivered, 1,
        "one WS subscriber received the injected message"
    );

    let (topic, data) = tokio::time::timeout(std::time::Duration::from_secs(5), sub.next_event())
        .await
        .expect("event arrives")
        .unwrap();
    assert_eq!(topic, "mesh");
    assert_eq!(data, b"from-gossip");
}
