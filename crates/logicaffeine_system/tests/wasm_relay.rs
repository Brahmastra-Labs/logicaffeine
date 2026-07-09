//! Browser relay round-trip test (Phase 9c).
//!
//! Runs the REAL browser relay client ([`RelayBrowserClient`], a `web-sys`
//! WebSocket) and round-trips a message through a native relay host. A browser
//! cannot host a relay, so a native one must be running first —
//! `scripts/test-wasm-node.sh` starts one on `127.0.0.1:9944` and then runs this
//! under node with a `WebSocket` polyfill (no headless browser needed); CI can
//! still force a real browser with `wasm-pack test --headless --chrome`.
//!
//! This is a genuine two-client browser→relay→browser path: a *subscriber* and a
//! separate *publisher*. The relay deliberately does not echo a publish back to
//! its own connection (so a CRDT `Sync` cannot double-count itself), which is
//! exactly why a single self-publishing client is the wrong shape here — two
//! connections are. The subscriber awaits its `SubAck` before the publisher
//! publishes, so the registration cannot lose the race across the two sockets.
//!
//! Only builds + runs on `wasm32`; inert on a normal `cargo test`.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use futures::StreamExt;
use logicaffeine_system::relay_browser::RelayBrowserClient;
use wasm_bindgen_test::*;

// No `run_in_browser`: this runs under node (a `WebSocket` polyfill supplies the
// browser global), so the suite exercises it WITHOUT a headless browser.

/// The relay host `scripts/test-wasm-node.sh` starts before the test.
const RELAY_URL: &str = "ws://127.0.0.1:9944";

/// Dial the relay and resolve once the socket is open, forwarding events and
/// subscribe acks onto channels the test can await.
async fn dial(
) -> (RelayBrowserClient, futures::channel::mpsc::UnboundedReceiver<(String, Vec<u8>)>, futures::channel::mpsc::UnboundedReceiver<String>) {
    let (open_tx, open_rx) = futures::channel::oneshot::channel::<()>();
    let open_tx = Rc::new(RefCell::new(Some(open_tx)));
    let (ev_tx, ev_rx) = futures::channel::mpsc::unbounded::<(String, Vec<u8>)>();
    let (ack_tx, ack_rx) = futures::channel::mpsc::unbounded::<String>();

    let opener = open_tx.clone();
    let client = RelayBrowserClient::connect(
        RELAY_URL,
        move || {
            if let Some(tx) = opener.borrow_mut().take() {
                let _ = tx.send(());
            }
        },
        move |topic, data| {
            let _ = ev_tx.unbounded_send((topic, data));
        },
    )
    .expect("dial the relay host");
    client.on_suback(move |topic| {
        let _ = ack_tx.unbounded_send(topic);
    });
    open_rx.await.expect("the socket opens");
    (client, ev_rx, ack_rx)
}

/// A subscriber receives what a *separate* publisher sends through the native
/// relay — the real browser→relay→browser path over real WebSockets.
#[wasm_bindgen_test]
async fn browser_two_clients_roundtrip_through_relay() {
    let (subscriber, mut sub_events, mut sub_acks) = dial().await;
    subscriber.subscribe("room").expect("subscribe");
    // The relay's SubAck guarantees the registration is live before we publish
    // from the other connection, so the publish cannot race ahead of it.
    let acked = sub_acks.next().await.expect("the relay acks the subscribe");
    assert_eq!(acked, "room");

    let (publisher, _pub_events, _pub_acks) = dial().await;
    publisher
        .publish("room", b"hello-from-the-other-tab".to_vec())
        .expect("publish");

    let (topic, data) = sub_events.next().await.expect("the relay delivers the event");
    assert_eq!(topic, "room");
    assert_eq!(data, b"hello-from-the-other-tab");
}

/// The relay must NOT echo a publish back to its own connection — the property
/// the CRDT `Sync` relies on to avoid double-counting itself. Two clients both
/// subscribe to the same topic; the publisher sends "mine" first, then a peer
/// sends "theirs". With self-skip the publisher's own "mine" is suppressed, so
/// the *first* event it ever receives is the peer's "theirs". If self-skip were
/// broken, "mine" — published first, on the relay's arrival order — would reach
/// the publisher before "theirs", and this assertion would catch it.
#[wasm_bindgen_test]
async fn browser_publisher_does_not_receive_its_own_message() {
    let (publisher, mut pub_events, mut pub_acks) = dial().await;
    publisher.subscribe("solo").expect("publisher subscribes");
    assert_eq!(pub_acks.next().await.expect("publisher ack"), "solo");

    let (peer, _peer_events, mut peer_acks) = dial().await;
    peer.subscribe("solo").expect("peer subscribes");
    assert_eq!(peer_acks.next().await.expect("peer ack"), "solo");

    publisher.publish("solo", b"mine".to_vec()).expect("publisher publishes first");
    peer.publish("solo", b"theirs".to_vec()).expect("peer publishes second");

    // The publisher's first delivered event is the peer's, never its own.
    let (topic, data) = pub_events.next().await.expect("the publisher receives the peer's message");
    assert_eq!(topic, "solo");
    assert_eq!(
        data, b"theirs",
        "publisher must skip its own 'mine' and receive only the peer's 'theirs'"
    );
}
