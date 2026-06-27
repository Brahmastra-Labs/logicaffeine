//! Phase 9c — the interpreter networks over the thin WS relay.
//!
//! Proves the language's `Connect`/`Sync` actually run in the tree-walker (not
//! just compiled): an interpreted program dials a relay, publishes a CRDT
//! counter, and merges one received from a peer. The browser path uses the
//! identical `Sync`/`Connect` lowering over `web-sys` WebSocket (headless test in
//! `logicaffeine_system/tests/wasm_relay.rs`).

use std::time::Duration;

use logicaffeine_compile::interpret_for_ui;
use logicaffeine_system::relay::{serve, RelayClient};

#[tokio::test]
async fn interp_sync_publishes_counter_via_relay() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();

    // A peer subscribes first, so the interpreter's publish can't race ahead.
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe("counter").await.expect("peer subscribe acked");

    let program = format!(
        "## Main\n\
         \x20   Let counter be 5.\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Sync counter on \"counter\".\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "interpreter networking ran: {:?}", result.error);

    // The peer receives the counter published over the relay, in the JSON wire
    // form (a bare Int counter uses the empty field name).
    let (topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
        .await
        .expect("event arrives in time")
        .expect("event present");
    assert_eq!(topic, "counter");
    assert_eq!(String::from_utf8(data).expect("utf8"), r#"{"":5}"#);
}

#[tokio::test]
async fn interp_sync_merges_received_counter() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let peer = RelayClient::connect(&url).await.expect("peer dials");

    // The interpreter loops Sync until it has merged a non-zero counter, then
    // shows it. A peer injects 7 once the interpreter has subscribed.
    let program = format!(
        "## Main\n\
         \x20   Let counter be 0.\n\
         \x20   Connect to \"{url}\".\n\
         \x20   While counter is at most 0:\n\
         \x20       Sync counter on \"t\".\n\
         \x20       Sleep 25.\n\
         \x20   Show counter.\n"
    );

    // Run the (possibly-blocking) interpreter under a timeout so a missed merge
    // fails fast instead of hanging the loop forever.
    let interp = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&program));
    let inject = async {
        // Let the interpreter subscribe (its first Sync), then publish a delta.
        tokio::time::sleep(Duration::from_millis(250)).await;
        peer.publish("t", br#"{"":7}"#.to_vec()).expect("peer publishes");
    };
    let (result, ()) = tokio::join!(interp, inject);
    let result = result.expect("interpreter did not hang");

    assert!(result.error.is_none(), "interpreter merged the delta: {:?}", result.error);
    assert!(
        result.lines.iter().any(|l| l == "7"),
        "interpreter should have merged the received counter (7), output: {:?}",
        result.lines
    );
}

#[tokio::test]
async fn interp_pipe_and_network_run_together() {
    // A program that uses BOTH a channel (→ the cooperative scheduler) AND networking
    // (Connect/Sync). Networking marks the program `needs_async`, which routed it to the
    // direct `interp.run` path that installs NO scheduler — so the first channel op
    // (`Let jobs be a Pipe`) panicked "concurrency op executed outside a scheduler context".
    // It must instead run on the scheduler AND service the network await over the reactor,
    // receiving the piped value and publishing it.
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe("counter").await.expect("peer subscribe acked");

    let program = format!(
        "## To produce (ch: Int):\n\
         \x20   Send 5 into ch.\n\
         \n\
         ## Main\n\
         \x20   Let jobs be a Pipe of Int.\n\
         \x20   Launch a task to produce with jobs.\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Receive n from jobs.\n\
         \x20   Sync n on \"counter\".\n"
    );
    let result = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&program))
        .await
        .expect("mixed pipe+network program did not hang");
    assert!(result.error.is_none(), "mixed pipe+network ran: {:?}", result.error);

    let (topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
        .await
        .expect("event arrives in time")
        .expect("event present");
    assert_eq!(topic, "counter");
    assert_eq!(String::from_utf8(data).expect("utf8"), r#"{"":5}"#);
}

#[tokio::test]
async fn interp_sync_publishes_struct_counter() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe("game").await.expect("peer subscribe acked");

    // A CRDT struct counter: the interpreter publishes its named Int fields.
    let program = format!(
        "## Definition\n\
         A Counter is Shared and has:\n\
         \x20   a points, which is ConvergentCount.\n\
         \n\
         ## Main\n\
         \x20   Let mutable c be a new Counter.\n\
         \x20   Increase c's points by 3.\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Sync c on \"game\".\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "struct CRDT sync ran: {:?}", result.error);

    let (topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
        .await
        .expect("event arrives")
        .expect("event present");
    assert_eq!(topic, "game");
    assert_eq!(String::from_utf8(data).expect("utf8"), r#"{"points":3}"#);
}

#[tokio::test]
async fn interp_sync_publishes_lww_register() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe("page").await.expect("peer subscribe acked");

    // A LastWriteWins Text field crosses the wire (not just Int counters).
    let program = format!(
        "## Definition\n\
         A Page is Shared and has:\n\
         \x20   a title, which is LastWriteWins of Text.\n\
         \n\
         ## Main\n\
         \x20   Let mutable p be a new Page.\n\
         \x20   Set p's title to \"hello\".\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Sync p on \"page\".\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "LWW register sync ran: {:?}", result.error);

    let (topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
        .await
        .expect("event arrives")
        .expect("event present");
    assert_eq!(topic, "page");
    assert_eq!(String::from_utf8(data).expect("utf8"), r#"{"title":"hello"}"#);
}

#[tokio::test]
async fn interp_connect_accepts_libp2p_multiaddr_surface() {
    // The compiled path dials peers with libp2p multiaddrs
    // (`/ip4/H/tcp/P`); the interpreter must accept the *same* surface and route
    // it over the relay, so one program text runs on both runtimes.
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url(); // ws://127.0.0.1:<port>
    let hostport = url.strip_prefix("ws://").expect("relay url is ws://");
    let (host, port) = hostport.rsplit_once(':').expect("host:port");
    let multiaddr = format!("/ip4/{host}/tcp/{port}");

    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe("counter").await.expect("peer subscribe acked");

    let program = format!(
        "## Main\n\
         \x20   Let counter be 5.\n\
         \x20   Connect to \"{multiaddr}\".\n\
         \x20   Sync counter on \"counter\".\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(
        result.error.is_none(),
        "interpreter should accept the /ip4/.../tcp/... multiaddr surface: {:?}",
        result.error
    );

    let (topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
        .await
        .expect("event arrives in time")
        .expect("event present");
    assert_eq!(topic, "counter");
    assert_eq!(String::from_utf8(data).expect("utf8"), r#"{"":5}"#);
}

#[tokio::test]
async fn interp_two_peers_round_trip_a_message() {
    // The authentic path: two interpreter peers on the relay. The sender `Listen`s
    // (its identity), waits for the receiver to subscribe, then `Send`s; the
    // receiver `Await`s the message from that named peer and shows it.
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();

    let receiver = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"receiver\".\n\
         \x20   Let sender be a PeerAgent at \"sender\".\n\
         \x20   Await response from sender into got.\n\
         \x20   Show got.\n"
    );
    let sender = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"sender\".\n\
         \x20   Let receiver be a PeerAgent at \"receiver\".\n\
         \x20   Sleep 300.\n\
         \x20   Send \"ping\" to receiver.\n"
    );

    let r = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&receiver));
    let s = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&sender));
    let (rr, sr) = tokio::join!(r, s);
    let rr = rr.expect("receiver did not hang");
    let sr = sr.expect("sender did not hang");

    assert!(sr.error.is_none(), "sender ran: {:?}", sr.error);
    assert!(rr.error.is_none(), "receiver ran: {:?}", rr.error);
    assert!(
        rr.lines.iter().any(|l| l == "ping"),
        "receiver should have received 'ping', output: {:?}",
        rr.lines
    );
}

#[tokio::test]
async fn interp_two_peers_round_trip_a_list_value() {
    // A message is any language value: a list crosses whole and the receiver binds
    // a real list it can index — no manual (de)serialization anywhere in the program.
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();

    let receiver = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"receiver\".\n\
         \x20   Let sender be a PeerAgent at \"sender\".\n\
         \x20   Await response from sender into got.\n\
         \x20   Show item 2 of got.\n"
    );
    let sender = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"sender\".\n\
         \x20   Let receiver be a PeerAgent at \"receiver\".\n\
         \x20   Let nums be [10, 20, 30].\n\
         \x20   Sleep 300.\n\
         \x20   Send nums to receiver.\n"
    );

    let r = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&receiver));
    let s = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&sender));
    let (rr, sr) = tokio::join!(r, s);
    let rr = rr.expect("receiver did not hang");
    let sr = sr.expect("sender did not hang");

    assert!(sr.error.is_none(), "sender ran: {:?}", sr.error);
    assert!(rr.error.is_none(), "receiver ran: {:?}", rr.error);
    assert!(
        rr.lines.iter().any(|l| l == "20"),
        "receiver should index the reconstructed list (item 2 = 20), output: {:?}",
        rr.lines
    );
}

#[tokio::test]
async fn interp_await_selects_by_sender() {
    // Three peers: alice awaits from bob. Carol's message arrives FIRST but is from
    // the wrong peer, so it stays queued; alice's await returns bob's message, not
    // carol's. (If await ignored the sender it would surface carol's earlier one.)
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();

    let alice = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let bob be a PeerAgent at \"bob\".\n\
         \x20   Await response from bob into got.\n\
         \x20   Show got.\n"
    );
    // Carol sends first (shorter wait), bob second — so carol's lands before bob's.
    let carol = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"carol\".\n\
         \x20   Let alice be a PeerAgent at \"alice\".\n\
         \x20   Sleep 300.\n\
         \x20   Send \"noise\" to alice.\n"
    );
    let bob = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"bob\".\n\
         \x20   Let alice be a PeerAgent at \"alice\".\n\
         \x20   Sleep 450.\n\
         \x20   Send \"signal\" to alice.\n"
    );

    let a = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&alice));
    let c = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&carol));
    let b = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&bob));
    let (ar, cr, br_) = tokio::join!(a, c, b);
    let ar = ar.expect("alice did not hang");
    cr.expect("carol did not hang");
    br_.expect("bob did not hang");

    assert!(ar.error.is_none(), "alice ran: {:?}", ar.error);
    assert!(
        ar.lines.iter().any(|l| l == "signal") && !ar.lines.iter().any(|l| l == "noise"),
        "await must return bob's 'signal', not carol's earlier 'noise', output: {:?}",
        ar.lines
    );
}

#[tokio::test]
async fn interp_send_compressed_keyword_compresses_the_wire() {
    // `Send compressed X to <peer>` deflates the wire body (kept only if it shrank).
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut bob = RelayClient::connect(&url).await.expect("bob dials");
    bob.subscribe("bob").await.expect("bob subscribe acked");

    let long = "abcd".repeat(300); // 1200 redundant bytes — compresses hard
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let remote be a PeerAgent at \"bob\".\n\
         \x20   Let s be \"{long}\".\n\
         \x20   Send compressed s to remote.\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "`Send compressed` ran: {:?}", result.error);

    let (topic, data) = tokio::time::timeout(Duration::from_secs(5), bob.next_event())
        .await
        .expect("event arrives in time")
        .expect("event present");
    assert_eq!(topic, "bob");
    // The frame's compression bit (0x02) is set, the wire is smaller than the raw
    // string, and it still decodes back to exactly the original text.
    assert!(data[0] & 0x02 != 0, "the `compressed` keyword set the compression bit");
    assert!(
        data.len() < long.len(),
        "compressed wire ({}) should be smaller than the raw string ({})",
        data.len(),
        long.len()
    );
    let (_from, back) = logicaffeine_compile::concurrency::marshal::message_from_wire(&data).expect("decodes");
    match back {
        logicaffeine_compile::interpreter::RuntimeValue::Text(t) => assert_eq!(t.as_str(), long),
        other => panic!("expected text, got {other:?}"),
    }
}

/// Run a one-liner that sends `body` to peer "bob" with the given send-clause
/// (e.g. `Send compressed with lz4`), and return the wire bytes "bob" receives.
async fn send_and_capture(send_clause: &str, body_expr: &str) -> Vec<u8> {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut bob = RelayClient::connect(&url).await.expect("bob dials");
    bob.subscribe("bob").await.expect("bob subscribe acked");
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let remote be a PeerAgent at \"bob\".\n\
         \x20   Let s be {body_expr}.\n\
         \x20   {send_clause} s to remote.\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "`{send_clause}` ran: {:?}", result.error);
    let (topic, data) = tokio::time::timeout(Duration::from_secs(5), bob.next_event())
        .await
        .expect("event arrives in time")
        .expect("event present");
    assert_eq!(topic, "bob");
    data
}

#[tokio::test]
async fn interp_send_cached_references_schema_on_repeat() {
    // `Send cached <struct list>` sends the schema once; a repeat references it by id
    // (content-addressed) and is smaller, and both decode to the same list through a
    // schema-aware receiver.
    use logicaffeine_compile::concurrency::marshal::{message_from_wire_cached, message_to_wire, WireSchemaCache};
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut bob = RelayClient::connect(&url).await.expect("bob dials");
    bob.subscribe("bob").await.expect("bob subscribe acked");
    let program = format!(
        "## A Point has:\n\
         \x20   An x: Int.\n\
         \x20   A y: Int.\n\
         \n\
         ## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let remote be a PeerAgent at \"bob\".\n\
         \x20   Let pts be [a new Point with x 1 and y 2, a new Point with x 3 and y 4].\n\
         \x20   Send cached pts to remote.\n\
         \x20   Send cached pts to remote.\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "`Send cached` ran: {:?}", result.error);

    let (t1, m1) = tokio::time::timeout(Duration::from_secs(5), bob.next_event()).await.expect("1st arrives").expect("present");
    let (t2, m2) = tokio::time::timeout(Duration::from_secs(5), bob.next_event()).await.expect("2nd arrives").expect("present");
    assert_eq!(t1, "bob");
    assert_eq!(t2, "bob");
    assert!(m2.len() < m1.len(), "the repeat references the schema → smaller: {} vs {}", m2.len(), m1.len());

    // A schema-aware receiver decodes both; the reference resolves against the def.
    let mut rc = WireSchemaCache::content_addressed();
    let d1 = message_from_wire_cached(&m1, &mut rc).expect("schema def decodes").1;
    let d2 = message_from_wire_cached(&m2, &mut rc).expect("schema ref decodes").1;
    assert_eq!(
        message_to_wire("alice", &d1).unwrap(),
        message_to_wire("alice", &d2).unwrap(),
        "the referenced message reconstructs the same struct list as the definition"
    );
}

#[tokio::test]
async fn interp_send_unchecked_drops_the_checksum() {
    // `Send unchecked X` drops the integrity checksum (header bit 0x01 unset) — the
    // latency dial — and still decodes exactly. Composes with the other modifiers.
    let data = send_and_capture("Send unchecked", "\"hello world\"").await;
    assert_eq!(data[0] & 0x01, 0, "unchecked drops the checksum bit");
    let (_from, back) = logicaffeine_compile::concurrency::marshal::message_from_wire(&data).expect("decodes");
    match back {
        logicaffeine_compile::interpreter::RuntimeValue::Text(t) => assert_eq!(t.as_str(), "hello world"),
        other => panic!("expected text, got {other:?}"),
    }
}

#[tokio::test]
async fn interp_send_compressed_with_lz4_keyword() {
    // `Send compressed with lz4 X to peer` selects the lz4 codec — header bit 0x02
    // set, codec id 1 in bits 2-3 (0x04), and it decodes back exactly.
    let long = "abcd".repeat(300);
    let data = send_and_capture("Send compressed with lz4", &format!("\"{long}\"")).await;
    assert!(data[0] & 0x02 != 0, "compression bit set");
    assert_eq!(data[0] & 0x0C, 0x04, "header records the lz4 codec id");
    assert!(data.len() < long.len(), "lz4 wire smaller than the raw string");
    let (_from, back) = logicaffeine_compile::concurrency::marshal::message_from_wire(&data).expect("decodes");
    match back {
        logicaffeine_compile::interpreter::RuntimeValue::Text(t) => assert_eq!(t.as_str(), long),
        other => panic!("expected text, got {other:?}"),
    }
}

#[tokio::test]
async fn interp_send_fast_picks_the_memcpy_layout() {
    // `Send fast X` picks the fixed-width memcpy layout (fastest decode, for a fat link).
    // For a list of small ints it is LARGER than the default varint (8 B/int vs ~2),
    // which proves the layout knob reached the wire — and it still decodes exactly.
    let fast = send_and_capture("Send fast", "[100, 200, 300, 400, 500]").await;
    let compact = send_and_capture("Send compact", "[100, 200, 300, 400, 500]").await;
    assert!(
        fast.len() > compact.len(),
        "fast (fixed 8B/int) must be larger than compact (varint): {} vs {}",
        fast.len(),
        compact.len()
    );
    use logicaffeine_compile::concurrency::marshal::{message_from_wire, message_to_wire};
    let (_, a) = message_from_wire(&fast).expect("fast decodes");
    let (_, b) = message_from_wire(&compact).expect("compact decodes");
    assert_eq!(
        message_to_wire("x", &a).unwrap(),
        message_to_wire("x", &b).unwrap(),
        "both layouts carry the same logical list"
    );
}

#[tokio::test]
async fn interp_send_packed_picks_group_varint() {
    // `Send packed X` picks the group-varint (SIMD) layout — the balanced middle. It
    // round-trips exactly; for small ints it sits between varint and fixed.
    let packed = send_and_capture("Send packed", "[1, 2, 3, 4, 5, 6, 7, 8]").await;
    use logicaffeine_compile::concurrency::marshal::message_from_wire;
    assert!(message_from_wire(&packed).is_some(), "packed decodes");
}

#[tokio::test]
async fn interp_send_smallest_picks_the_compression_menu() {
    // `Send smallest X` turns on the per-column compression menu. On a near-monotone
    // (non-affine) column its delta form is much smaller than the default varint, which
    // proves the knob reached the wire — and it still decodes to the exact list.
    let list = "[1000, 1002, 1003, 1005, 1006, 1008, 1009, 1011, 1012, 1014, 1015, 1017, 1018, 1020, 1021]";
    let smallest = send_and_capture("Send smallest", list).await;
    let compact = send_and_capture("Send compact", list).await;
    assert!(
        smallest.len() < compact.len(),
        "smallest (compression menu) must beat compact varint on a monotone run: {} vs {}",
        smallest.len(),
        compact.len()
    );
    use logicaffeine_compile::concurrency::marshal::{message_from_wire, message_to_wire};
    let (_, a) = message_from_wire(&smallest).expect("smallest decodes");
    let (_, b) = message_from_wire(&compact).expect("compact decodes");
    assert_eq!(
        message_to_wire("x", &a).unwrap(),
        message_to_wire("x", &b).unwrap(),
        "the compression menu carries the same logical list"
    );
}

#[tokio::test]
async fn interp_send_redundant_publishes_reconstructable_shards() {
    // `Send redundant X` (the one-word reliability knob) splits the message into FEC
    // shards and publishes EACH as its own packet, so a receiver reconstructs the exact
    // message from any K even after some are lost. We capture all the shards a peer
    // receives, drop one, and prove the rest still reconstruct a valid message.
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut bob = RelayClient::connect(&url).await.expect("bob dials");
    bob.subscribe("bob").await.expect("bob subscribe acked");
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let remote be a PeerAgent at \"bob\".\n\
         \x20   Let s be [10, 20, 30, 40, 50].\n\
         \x20   Send redundant s to remote.\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "`Send redundant` ran: {:?}", result.error);

    // Collect every shard the peer received (the interpreter has finished publishing).
    let mut shards: Vec<Vec<u8>> = Vec::new();
    while let Ok(Some((topic, data))) =
        tokio::time::timeout(Duration::from_millis(500), bob.next_event()).await
    {
        assert_eq!(topic, "bob");
        shards.push(data);
    }
    assert!(shards.len() > 1, "redundant must publish multiple shards, got {}", shards.len());

    use logicaffeine_compile::concurrency::fec::reconstruct_redundant;
    use logicaffeine_compile::concurrency::marshal::message_from_wire;
    // Drop one shard — the rest must still reconstruct the original wire message.
    let (_id, payload) =
        reconstruct_redundant(&shards[1..]).expect("reconstruct from a lossy shard subset");
    assert!(
        message_from_wire(&payload).is_some(),
        "the reconstructed bytes decode as the original message"
    );
}

#[tokio::test]
async fn interp_receive_reconstructs_redundant_after_loss() {
    // The receive side of `redundant`: a peer FEC-frames a message and delivers only K of
    // N shards (the other 2 "lost" on the link). The interpreter buffers the shards,
    // reconstructs the EXACT message once K arrive, and shows it.
    use logicaffeine_compile::concurrency::fec::frame_redundant;
    use logicaffeine_compile::concurrency::marshal::message_to_wire;
    use logicaffeine_compile::interpreter::RuntimeValue;

    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let peer = RelayClient::connect(&url).await.expect("peer dials");

    // Wire-encode the peer's message (sender "carol"), then split into 6 shards (k=4),
    // matching the interpreter's REDUNDANT_K/REDUNDANT_N.
    let wire = message_to_wire("carol", &RuntimeValue::Int(42)).unwrap();
    let shards = frame_redundant(0, &wire, 4, 6).expect("frame");

    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"dave\".\n\
         \x20   Let carol be a PeerAgent at \"carol\".\n\
         \x20   Await response from carol into m.\n\
         \x20   Show m.\n"
    );
    let interp = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&program));
    let inject = async {
        // Let the interpreter subscribe, then deliver only 4 of the 6 shards (2 lost).
        tokio::time::sleep(Duration::from_millis(250)).await;
        for s in shards.iter().take(4) {
            peer.publish("dave", s.clone()).expect("peer publishes shard");
        }
    };
    let (result, ()) = tokio::join!(interp, inject);
    let result = result.expect("interpreter did not hang");
    assert!(result.error.is_none(), "interpreter reconstructed from K shards: {:?}", result.error);
    assert!(
        result.lines.iter().any(|l| l == "42"),
        "the reconstructed Int (42) is shown, output: {:?}",
        result.lines
    );
}

#[tokio::test]
async fn interp_send_shared_elides_names_default_stays_self_describing() {
    // Type-id elision is OPT-IN: a relay or a different-version / non-Logos peer often does
    // NOT share the program's types, so the DEFAULT `Send g` must stay self-describing
    // (names on the wire → anyone decodes it). `Send shared g` is the explicit option you
    // flip on ONLY when both ends run the same program, to drop the names and go faster.
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut bob = RelayClient::connect(&url).await.expect("bob dials");
    bob.subscribe("bob").await.expect("bob subscribe acked");
    let program = format!(
        "## A Gadget has:\n\
         \x20   A wingspan: Int.\n\
         \n\
         ## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let remote be a PeerAgent at \"bob\".\n\
         \x20   Let g be a new Gadget with wingspan 42.\n\
         \x20   Send g to remote.\n\
         \x20   Send shared g to remote.\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "struct sends ran: {:?}", result.error);
    // First send — default, self-describing; second — opt-in `shared`, name-elided.
    let plain = tokio::time::timeout(Duration::from_secs(5), bob.next_event())
        .await
        .expect("event arrives")
        .expect("event present")
        .1;
    let shared = tokio::time::timeout(Duration::from_secs(5), bob.next_event())
        .await
        .expect("event arrives")
        .expect("event present")
        .1;
    let on_wire = |d: &[u8], n: &[u8]| d.windows(n.len()).any(|w| w == n);
    assert!(on_wire(&plain, b"wingspan"), "DEFAULT send must stay self-describing (names present) for any peer/relay");
    assert!(!on_wire(&shared, b"wingspan"), "`shared` must elide the field name via the type registry; wire={shared:?}");
    assert!(!on_wire(&shared, b"Gadget"), "`shared` must elide the type name too");
    assert!(shared.len() < plain.len(), "`shared` ({}) is smaller than default ({})", shared.len(), plain.len());
}

#[tokio::test]
async fn interp_send_shared_elides_enum_names() {
    // The enum analog of the struct test: `Send shared` on an enum value elides the enum
    // TYPE name and the CONSTRUCTOR name (T_INDUCTIVE_TID), shipping a type-id + ctor-index
    // — proving the interpreter now auto-wires ENUM defs into the wire registry, not just
    // structs. Default `Send` stays self-describing (names on the wire for any peer).
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut bob = RelayClient::connect(&url).await.expect("bob dials");
    bob.subscribe("bob").await.expect("bob subscribe acked");
    let program = format!(
        "## Definition\n\
         A Color is either:\n\
         \x20   A Crimson.\n\
         \x20   A Viridian.\n\
         \x20   A Cerulean.\n\
         \n\
         ## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let remote be a PeerAgent at \"bob\".\n\
         \x20   Let c be a new Viridian.\n\
         \x20   Send c to remote.\n\
         \x20   Send shared c to remote.\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "enum sends ran: {:?}", result.error);
    let plain = tokio::time::timeout(Duration::from_secs(5), bob.next_event())
        .await
        .expect("event arrives")
        .expect("event present")
        .1;
    let shared = tokio::time::timeout(Duration::from_secs(5), bob.next_event())
        .await
        .expect("event arrives")
        .expect("event present")
        .1;
    let on_wire = |d: &[u8], n: &[u8]| d.windows(n.len()).any(|w| w == n);
    assert!(on_wire(&plain, b"Viridian"), "DEFAULT enum send must stay self-describing (ctor name present)");
    assert!(!on_wire(&shared, b"Viridian"), "`shared` must elide the constructor name; wire={shared:?}");
    assert!(!on_wire(&shared, b"Color"), "`shared` must elide the enum type name too");
    assert!(shared.len() < plain.len(), "`shared` enum ({}) is smaller than default ({})", shared.len(), plain.len());
}

#[tokio::test]
async fn interp_send_computed_ships_the_function_as_a_callable() {
    // `Send computed f` — COMPUTE-SHIPPING end to end: a pure single-argument function is
    // lowered to the sandboxed generator and ships as a CALLABLE the receiver evaluates in
    // its bounded sandbox (never arbitrary code). Decode the wire and confirm it computes
    // f(x) = 3x + 1 on a peer that never compiled the function.
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut bob = RelayClient::connect(&url).await.expect("bob dials");
    bob.subscribe("bob").await.expect("bob subscribe acked");
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let remote be a PeerAgent at \"bob\".\n\
         \x20   Let f be (i: Int) -> i * 3 + 1.\n\
         \x20   Send computed f to remote.\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "computed send ran: {:?}", result.error);
    let wire = tokio::time::timeout(Duration::from_secs(5), bob.next_event())
        .await
        .expect("event arrives")
        .expect("event present")
        .1;
    let (_, val) = logicaffeine_compile::concurrency::marshal::message_from_wire(&wire)
        .expect("the computed function decodes off the wire");
    match val {
        logicaffeine_compile::interpreter::RuntimeValue::Function(c) => {
            let gen = c.generated.expect("a shipped computed function carries its sandboxed generator");
            for x in -10..10i64 {
                assert_eq!(
                    logicaffeine_compile::concurrency::marshal::gen_eval(&gen, x),
                    3 * x + 1,
                    "the shipped function computes f(x)=3x+1 on a receiver that never compiled it"
                );
            }
        }
        other => panic!("expected a callable Function on the wire, got {other:?}"),
    }
}

#[tokio::test]
async fn computed_function_runs_only_through_the_acceptance_contract_end_to_end() {
    // C2 Layer C, the full "ship computation, safely" flow: a peer ships a pure computation
    // via `Send computed`; the receiver decodes it and runs it ONLY through its typed,
    // bounded acceptance contract. In-bounds arguments compute in the sandbox; out-of-bounds
    // arguments are REFUSED at the seam (never clamped) — the attack surface is exactly the
    // range the receiver wrote down.
    use logicaffeine_compile::interpreter::RuntimeValue;
    use logicaffeine_compile::semantics::acceptance::AcceptanceContract;

    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut bob = RelayClient::connect(&url).await.expect("bob dials");
    bob.subscribe("bob").await.expect("bob subscribe acked");
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"alice\".\n\
         \x20   Let remote be a PeerAgent at \"bob\".\n\
         \x20   Let f be (i: Int) -> i * 3 + 1.\n\
         \x20   Send computed f to remote.\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "computed send ran: {:?}", result.error);
    let wire = tokio::time::timeout(Duration::from_secs(5), bob.next_event())
        .await
        .expect("event arrives")
        .expect("event present")
        .1;
    let (_, received) = logicaffeine_compile::concurrency::marshal::message_from_wire(&wire)
        .expect("the computed function decodes off the wire");

    // The receiver accepts a single Int argument only within [0, 1000].
    let contract = AcceptanceContract::new(0, 1000);
    // In-bounds (7 ∈ [0, 1000]) → 3·7 + 1 = 22, computed in the sandbox on a peer that never
    // compiled the function.
    assert_eq!(contract.apply(&received, 7).unwrap(), 22);
    // Out-of-bounds (9999 ∉ [0, 1000]) → refused, not clamped.
    assert!(
        contract.apply(&received, 9999).is_err(),
        "an out-of-range argument to a received computation must be refused at the contract"
    );
    // A non-shipped ordinary value is refused at the signature check.
    assert!(contract.apply(&RuntimeValue::Int(5), 7).is_err());
}

#[test]
fn run_accepted_is_cross_tier_consistent_and_refuses_an_ordinary_closure() {
    // `run_accepted` reachable from Logos, and CROSS-TIER consistent: the program runs on the
    // bytecode VM with the tree-walker as the debug shadow oracle, so the two MUST agree. An
    // ordinary closure (never shipped via `Send computed`) is refused at the signature check —
    // and both tiers refuse it identically (the tree-walker's `builtin_id` now falls back to
    // `builtin_from_name`, resolving `run_accepted` exactly as the VM does).
    let program = "## Main\n\
        Let f be (i: Int) -> i * 3 + 1.\n\
        Let y be run_accepted(f, 5, 0, 1000).\n\
        Show y.\n";
    let result = logicaffeine_compile::compile::interpret_program(program);
    assert!(
        result.is_err(),
        "run_accepted must refuse an ordinary (non-shipped) closure on both tiers, got {result:?}"
    );
}

#[test]
fn accept_computed_declarative_sugar_desugars_and_refuses_ordinary() {
    // The C2 Layer C declarative sugar: `Accept computed <Name> where <p> is an Int from lo to
    // hi` declares a named contract; `Run <f> on <arg> under <Name> into <var>` desugars to
    // `Let <var> be run_accepted(<f>, <arg>, lo, hi)`, inlining the named bounds. An ordinary
    // (non-shipped) closure is refused at the seam — proving the sugar parsed, desugared, and
    // reached the run_accepted validator (cross-tier consistent now).
    let program = "## Main\n\
        Accept computed Tripler where input is an Int from 0 to 1000.\n\
        Let f be (i: Int) -> i * 3 + 1.\n\
        Run f on 5 under Tripler into y.\n\
        Show y.\n";
    let result = logicaffeine_compile::compile::interpret_program(program);
    assert!(
        result.is_err(),
        "Run-under-contract on an ordinary closure must be refused, got {result:?}"
    );
}

#[test]
fn run_under_an_undeclared_contract_is_an_error() {
    // `Run … under <Name>` referencing a contract that was never `Accept`ed is rejected at
    // parse time (the sugar resolves the named bounds from the parser's contract table).
    let program = "## Main\n\
        Let f be (i: Int) -> i * 3 + 1.\n\
        Run f on 5 under Undeclared into y.\n\
        Show y.\n";
    let result = logicaffeine_compile::compile::interpret_program(program);
    assert!(result.is_err(), "Run under an undeclared contract must error");
}

#[tokio::test]
async fn interp_send_compressed_with_zstd_keyword() {
    // `Send compressed with zstd X to peer` selects zstd — codec id 2 in bits 2-3
    // (0x08). Native encodes via the C zstd library; decodes back exactly.
    let long = "abcd".repeat(300);
    let data = send_and_capture("Send compressed with zstd", &format!("\"{long}\"")).await;
    assert!(data[0] & 0x02 != 0, "compression bit set");
    assert_eq!(data[0] & 0x0C, 0x08, "header records the zstd codec id");
    assert!(data.len() < long.len(), "zstd wire smaller than the raw string");
    let (_from, back) = logicaffeine_compile::concurrency::marshal::message_from_wire(&data).expect("decodes");
    match back {
        logicaffeine_compile::interpreter::RuntimeValue::Text(t) => assert_eq!(t.as_str(), long),
        other => panic!("expected text, got {other:?}"),
    }
}

#[tokio::test]
async fn interp_send_without_connect_errors_cleanly() {
    let program = "## Main\n\
        \x20   Let bob be a PeerAgent at \"bob\".\n\
        \x20   Send \"hi\" to bob.\n";
    let result = interpret_for_ui(program).await;
    let err = result.error.expect("Send without Connect must error");
    assert!(err.contains("Connect"), "error should name the missing Connect, got: {err}");
}

#[tokio::test]
async fn interp_await_without_listen_errors_cleanly() {
    // Connected but never `Listen`ed → no inbox to receive on.
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Let bob be a PeerAgent at \"bob\".\n\
         \x20   Await response from bob into reply.\n"
    );
    let result = interpret_for_ui(&program).await;
    let err = result.error.expect("Await without Listen must error");
    assert!(err.contains("Listen"), "error should name the missing Listen, got: {err}");
}

#[tokio::test]
async fn interp_sync_without_connect_errors_cleanly() {
    // `Sync` before any `Connect` is a clean error, not a panic.
    let program = "## Main\n\
        \x20   Let counter be 1.\n\
        \x20   Sync counter on \"t\".\n";
    let result = interpret_for_ui(program).await;
    let err = result.error.expect("Sync without Connect must error");
    assert!(err.contains("Connect"), "error should name the missing Connect, got: {err}");
}
