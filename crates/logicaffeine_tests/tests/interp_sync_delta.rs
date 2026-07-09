//! ════════════════════════════════════════════════════════════════════════════════════════════
//! G7 δ-CRDT `Sync` END-TO-END — `Sync <shared-struct> on <topic>` now ships only the CHANGE since
//! the last sync, not the whole collection. A program fills a `SharedSet` of 8, syncs (the first
//! sync ships the full state), adds ONE more element, and syncs again — the second publish is a tiny
//! delta, orders of magnitude smaller than re-broadcasting the set. Proven over a live loopback relay
//! through the running interpreter.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use std::time::Duration;

use logicaffeine_compile::interpret_for_ui;
use logicaffeine_system::addr::canonical_topic;
use logicaffeine_system::relay::{serve, RelayClient};

/// Run a program that fills a `SharedSet`, `Sync`s it, adds one element, and `Sync`s again. A raw
/// peer on the topic captures the two published payloads → `(full_state, incremental_delta)`.
async fn capture_two_syncs() -> (Vec<u8>, Vec<u8>) {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe(&canonical_topic("room")).await.expect("peer subscribes to the room");

    let program = format!(
        "## Definition\n\
         A Room is Shared and has:\n\
         \x20   a members, which is a SharedSet of Int.\n\
         \n\
         ## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Let mutable r be a new Room.\n\
         \x20   Add 1 to r's members.\n\
         \x20   Add 2 to r's members.\n\
         \x20   Add 3 to r's members.\n\
         \x20   Add 4 to r's members.\n\
         \x20   Add 5 to r's members.\n\
         \x20   Add 6 to r's members.\n\
         \x20   Add 7 to r's members.\n\
         \x20   Add 8 to r's members.\n\
         \x20   Sync r on \"room\".\n\
         \x20   Add 999 to r's members.\n\
         \x20   Sync r on \"room\".\n"
    );

    let run = interpret_for_ui(&program);
    let recv = async {
        let (_t1, full) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
            .await
            .expect("the first sync (full state) arrives")
            .expect("event present");
        let (_t2, delta) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
            .await
            .expect("the second sync (delta) arrives")
            .expect("event present");
        (full, delta)
    };

    let (result, pair) = tokio::join!(run, recv);
    assert!(result.error.is_none(), "the sync program ran: {:?}", result.error);
    pair
}

#[tokio::test]
async fn interp_sync_ships_a_small_delta_on_the_second_round() {
    let (full, delta) = capture_two_syncs().await;
    assert!(
        delta.len() * 2 < full.len(),
        "the 2nd `Sync` must ship a small delta ({} B) for the one new element, not re-broadcast the \
         whole 8-element set ({} B)",
        delta.len(),
        full.len()
    );
}
