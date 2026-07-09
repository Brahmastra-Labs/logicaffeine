//! ════════════════════════════════════════════════════════════════════════════════════════════
//! VMNET-4 — CROSS-TIER NETWORKING LOCK: peer networking runs byte-identically on the BYTECODE VM
//! and the TREE-WALKER. Both tiers compile the SAME program, dial the SAME loopback relay, and go
//! through the SAME shared `NetInbox` (lifted in VMNET-2) — so `Connect`/`Listen`/`Send`/`Stream`/
//! `Await` MUST produce identical output. The VM path is `run_vm_net_async` (opcodes `Op::Net*` →
//! `VmBlock::Net*` → the async net driver); the tree-walker path is `interpret_for_ui`.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  A divergence here means networking drifted between tiers — fix the TIER (`run_vm_net_async` /
//!  the VM net opcodes / the shared `NetInbox`), never relax the `assert_eq!`. Strictly monotone:
//!  add directions/shapes, never remove them. This is the lock that retires the interim
//!  `emit_fail` / `vm_loudly_refuses` — the VM HANDLES networking now, it does not refuse it.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use std::time::Duration;

use logicaffeine_compile::concurrency::marshal::{
    deframe_stream_message, frame_stream_message, make_handshake_frame, message_from_wire,
    parse_handshake_frame, with_type_registry, PeerProfile, WireTypeRegistry,
};
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::{interpret_for_ui, run_vm_net_async};
use logicaffeine_system::addr::canonical_topic;
use logicaffeine_system::relay::{serve, RelayClient};

/// Run `program` on one tier — the VM net driver or the tree-walker — under a hang guard.
async fn run_tier(program: &str, use_vm: bool) -> logicaffeine_compile::interpreter::InterpreterResult {
    let fut = async {
        if use_vm {
            run_vm_net_async(program).await
        } else {
            interpret_for_ui(program).await
        }
    };
    tokio::time::timeout(Duration::from_secs(10), fut)
        .await
        .expect("the program must not hang")
}

/// RECEIVE side: the program `Await stream`s a batch a peer injects, and reads its first element.
/// Returns the program's output lines on the given tier.
async fn await_stream_first_element(use_vm: bool) -> Vec<String> {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let peer = RelayClient::connect(&url).await.expect("peer dials");

    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Await stream from \"sensor\" into batch.\n\
         \x20   Show item 1 of batch.\n"
    );

    let run = run_tier(&program, use_vm);
    let inject = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let blob = frame_stream_message(
            &canonical_topic("sensor"),
            &[RuntimeValue::Int(11), RuntimeValue::Int(22), RuntimeValue::Int(33)],
        )
        .unwrap();
        peer.publish(&canonical_topic("me"), blob).expect("peer injects the batch");
    };

    let (result, ()) = tokio::join!(run, inject);
    assert!(result.error.is_none(), "tier (vm={use_vm}) Await errored: {:?}", result.error);
    result.lines
}

/// SEND side: the program `Stream`s a batch to a peer; returns the values the peer deframes.
async fn stream_batch_to_peer(use_vm: bool) -> Vec<RuntimeValue> {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe(&canonical_topic("sink")).await.expect("peer subscribes");

    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Let items be [11, 22, 33].\n\
         \x20   Stream items to \"sink\".\n"
    );

    let run = run_tier(&program, use_vm);
    let recv = async {
        let (_topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
            .await
            .expect("the streamed batch arrives")
            .expect("event present");
        data
    };

    let (result, data) = tokio::join!(run, recv);
    assert!(result.error.is_none(), "tier (vm={use_vm}) Stream errored: {:?}", result.error);
    deframe_stream_message(&data).expect("the peer deframed the batch")
}

/// SEND side (plain `Send`): the program `Send`s a scalar to a peer; returns the decoded value.
async fn send_scalar_to_peer(use_vm: bool) -> Option<(String, RuntimeValue)> {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe(&canonical_topic("sink")).await.expect("peer subscribes");

    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Send 42 to \"sink\".\n"
    );

    let run = run_tier(&program, use_vm);
    let recv = async {
        let (_topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
            .await
            .expect("the message arrives")
            .expect("event present");
        data
    };

    let (result, data) = tokio::join!(run, recv);
    assert!(result.error.is_none(), "tier (vm={use_vm}) Send errored: {:?}", result.error);
    message_from_wire(&data)
}

/// SEND side via a `PeerAgent` handle: the program declares a peer (`Let p be a PeerAgent at …`)
/// and `Send`s to it; returns the decoded value the peer receives.
async fn send_via_peeragent(use_vm: bool) -> Option<(String, RuntimeValue)> {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe(&canonical_topic("sink")).await.expect("peer subscribes");

    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Let p be a PeerAgent at \"sink\".\n\
         \x20   Send 99 to p.\n"
    );

    let run = run_tier(&program, use_vm);
    let recv = async {
        let (_topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
            .await
            .expect("the message arrives")
            .expect("event present");
        data
    };

    let (result, data) = tokio::join!(run, recv);
    assert!(result.error.is_none(), "tier (vm={use_vm}) PeerAgent Send errored: {:?}", result.error);
    message_from_wire(&data)
}

/// `Sync` (CRDT sync point): the program syncs a counter on a topic with no other peer publishing,
/// then shows it — proving `Sync` runs end-to-end (subscribe + publish + drain + merge). Returns the
/// program's output lines.
async fn sync_counter_output(use_vm: bool) -> Vec<String> {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();

    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Let count be 5.\n\
         \x20   Sync count on \"room\".\n\
         \x20   Show count.\n"
    );

    let result = run_tier(&program, use_vm).await;
    assert!(result.error.is_none(), "tier (vm={use_vm}) Sync errored: {:?}", result.error);
    result.lines
}

#[tokio::test]
async fn vm_await_stream_is_byte_identical_to_treewalker() {
    let tree_walker = await_stream_first_element(false).await;
    let vm = await_stream_first_element(true).await;
    assert_eq!(
        tree_walker, vm,
        "CROSS-TIER NETWORKING REGRESSION: `Await stream` output diverged between the VM \
         ({vm:?}) and the tree-walker ({tree_walker:?}). Fix the tier, never this lock."
    );
    assert!(
        tree_walker.iter().any(|l| l == "11"),
        "both tiers must deframe the batch and read its first element (11): {tree_walker:?}"
    );
}

#[tokio::test]
async fn vm_stream_send_is_byte_identical_to_treewalker() {
    let tree_walker = stream_batch_to_peer(false).await;
    let vm = stream_batch_to_peer(true).await;
    assert_eq!(
        tree_walker, vm,
        "CROSS-TIER NETWORKING REGRESSION: `Stream` send delivered different values from the VM \
         ({vm:?}) than the tree-walker ({tree_walker:?}). Fix the tier, never this lock."
    );
    assert_eq!(
        vm,
        vec![RuntimeValue::Int(11), RuntimeValue::Int(22), RuntimeValue::Int(33)],
        "the VM streamed exactly the values the program built"
    );
}

#[tokio::test]
async fn vm_send_scalar_is_byte_identical_to_treewalker() {
    let tree_walker = send_scalar_to_peer(false).await;
    let vm = send_scalar_to_peer(true).await;
    assert_eq!(
        tree_walker, vm,
        "CROSS-TIER NETWORKING REGRESSION: `Send` delivered a different value/sender from the VM \
         ({vm:?}) than the tree-walker ({tree_walker:?}). Fix the tier, never this lock."
    );
    assert!(
        matches!(vm, Some((_, RuntimeValue::Int(42)))),
        "the VM sent the scalar 42 the program built: {vm:?}"
    );
}

#[tokio::test]
async fn vm_peeragent_send_is_byte_identical_to_treewalker() {
    let tree_walker = send_via_peeragent(false).await;
    let vm = send_via_peeragent(true).await;
    assert_eq!(
        tree_walker, vm,
        "CROSS-TIER NETWORKING REGRESSION: a `PeerAgent`-addressed `Send` diverged between the VM \
         ({vm:?}) and the tree-walker ({tree_walker:?}). `LetPeerAgent` must resolve identically on \
         both tiers. Fix the tier, never this lock."
    );
    assert!(
        matches!(vm, Some((_, RuntimeValue::Int(99)))),
        "the VM resolved the PeerAgent and sent 99: {vm:?}"
    );
}

#[tokio::test]
async fn vm_sync_is_byte_identical_to_treewalker() {
    let tree_walker = sync_counter_output(false).await;
    let vm = sync_counter_output(true).await;
    assert_eq!(
        tree_walker, vm,
        "CROSS-TIER NETWORKING REGRESSION: `Sync` produced different output on the VM ({vm:?}) than \
         the tree-walker ({tree_walker:?}). Fix the tier, never this lock."
    );
    assert!(
        tree_walker.iter().any(|l| l == "5"),
        "both tiers ran the CRDT sync point and kept the counter (5): {tree_walker:?}"
    );
}

/// HANDSHAKE EXCHANGE LOCK: on FIRST contact a program advertises its acceptance profile to the peer's
/// dedicated handshake sub-topic (`<peer>#hs`), NEVER the data topic — identically on BOTH tiers. So a
/// peer learns our surface and can negotiate back, while a raw consumer on the data topic is unaffected
/// (that separation is exactly why the byte-identical data locks above still hold).
async fn handshake_advertised_to_peer(
    use_vm: bool,
) -> Option<(String, logicaffeine_compile::concurrency::marshal::PeerProfile)> {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    let peer_data = canonical_topic("peer");
    let peer_hs = format!("{peer_data}#hs");
    peer.subscribe(&peer_data).await.expect("peer subscribes data");
    peer.subscribe(&peer_hs).await.expect("peer subscribes hs");

    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Send 7 to \"peer\".\n"
    );
    let run = run_tier(&program, use_vm);
    let recv = async {
        let mut found = None;
        // The send emits two frames: the handshake (on `#hs`) and the data (on the data topic).
        for _ in 0..2 {
            match tokio::time::timeout(Duration::from_secs(5), peer.next_event()).await {
                Ok(Some((topic, data))) if topic == peer_hs => {
                    found = logicaffeine_compile::concurrency::marshal::parse_handshake_frame(&data);
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        found
    };
    let (result, found) = tokio::join!(run, recv);
    assert!(result.error.is_none(), "tier(vm={use_vm}) Send errored: {:?}", result.error);
    found
}

#[tokio::test]
async fn program_advertises_its_profile_on_first_contact_on_both_tiers() {
    use logicaffeine_compile::concurrency::marshal::PeerProfile;
    for use_vm in [false, true] {
        let (from, profile) = handshake_advertised_to_peer(use_vm).await.unwrap_or_else(|| {
            panic!("tier(vm={use_vm}) must advertise a handshake on the peer's #hs topic")
        });
        assert_eq!(from, canonical_topic("me"), "advertises our inbox identity");
        assert_eq!(profile, PeerProfile::default(), "advertises our full capability profile");
    }
}

/// True iff `needle` appears as a contiguous byte run inside `haystack` — used to assert that field /
/// type NAMES are (or are not) physically present on the wire.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// TYPE-ID NAME-ELISION END-TO-END (the D2b capstone): two Logos peers that run the SAME program
/// derive the identical content-addressed type registry, so once they have traded handshakes a
/// struct's type/field NAMES never travel again — only a tiny registry id plus the field values.
///
/// We model the far peer as a raw relay client that MIRRORS BACK exactly the registry epoch the
/// program advertised on first contact (the real two-same-program-peers condition, with NO hardcoded
/// epoch — the peer learns it from the program itself). The program sends the same struct twice:
///   1. FIRST contact (peer still unknown) → the conservative, self-describing form: field names inline.
///   2. After absorbing the peer's matching handshake → type-id elided: names GONE, strictly smaller.
/// Returns `(first_send_bytes, second_send_bytes)` as the peer observed them on the wire.
async fn typeid_elided_struct_sends(use_vm: bool) -> (Vec<u8>, Vec<u8>) {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    let peer_data = canonical_topic("peer");
    let peer_hs = format!("{peer_data}#hs");
    let me_data = canonical_topic("me");
    peer.subscribe(&peer_data).await.expect("peer subscribes data");
    peer.subscribe(&peer_hs).await.expect("peer subscribes hs");

    let program = format!(
        "## A Record has:\n\
         \x20   An alpha: Int.\n\
         \x20   A beta: Int.\n\
         \n\
         ## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Let r be a new Record with alpha 1 and beta 2.\n\
         \x20   Send r to \"peer\".\n\
         \x20   Await stream from \"peer\" into ready.\n\
         \x20   Send r to \"peer\".\n"
    );

    let run = run_tier(&program, use_vm);
    let drive = async {
        // Phase 1 — collect the program's first-contact handshake (carrying its registry epoch) and the
        // FIRST, self-describing struct send. They arrive on two topics in either order.
        let mut program_epoch: Option<u64> = None;
        let mut first_struct: Option<Vec<u8>> = None;
        while program_epoch.is_none() || first_struct.is_none() {
            let (topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
                .await
                .expect("the program's first contact arrives")
                .expect("event present");
            if topic == peer_hs {
                program_epoch = parse_handshake_frame(&data).map(|(_, p)| p.registry_epoch);
            } else if topic == peer_data {
                first_struct.get_or_insert(data);
            }
        }
        let epoch = program_epoch.expect("the program advertised a registry epoch");
        assert_ne!(epoch, 0, "a program with a struct type advertises a NON-zero registry epoch");

        // Phase 2 — mirror that exact epoch back (a second peer running the same program), then unblock
        // the Await. BOTH frames go to OUR data topic in order: the handshake is absorbed (never
        // delivered as data), the stream frame unblocks the Await — so by the time the program's SECOND
        // send runs it has negotiated type-id against our matching, advertised surface.
        let mirror = PeerProfile { registry_epoch: epoch, ..PeerProfile::default() };
        peer.publish(&me_data, make_handshake_frame(&peer_data, &mirror)).expect("mirror handshake");
        let unblock = frame_stream_message(&peer_data, &[RuntimeValue::Int(1)]).unwrap();
        peer.publish(&me_data, unblock).expect("unblock the Await");

        // Phase 3 — the SECOND struct send, now type-id elided.
        let second_struct = loop {
            let (topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
                .await
                .expect("the second struct arrives")
                .expect("event present");
            if topic == peer_data {
                break data;
            }
        };
        (first_struct.unwrap(), second_struct)
    };

    let (result, pair) = tokio::join!(run, drive);
    assert!(result.error.is_none(), "tier(vm={use_vm}) type-id program errored: {:?}", result.error);
    pair
}

#[tokio::test]
async fn typeid_name_elision_fires_end_to_end_on_both_tiers() {
    let (tw_first, tw_second) = typeid_elided_struct_sends(false).await;
    let (vm_first, vm_second) = typeid_elided_struct_sends(true).await;

    // First contact is self-describing on BOTH tiers — the type and field NAMES are physically on the wire.
    for first in [&tw_first, &vm_first] {
        assert!(
            contains(first, b"Record") && contains(first, b"alpha") && contains(first, b"beta"),
            "first contact is self-describing — type/field names travel: {first:?}"
        );
    }

    // The SECOND send (the peer's matching registry is now known) ELIDES every name and is strictly smaller.
    for (first, second) in [(&tw_first, &tw_second), (&vm_first, &vm_second)] {
        assert!(
            !contains(second, b"alpha") && !contains(second, b"beta"),
            "type-id elides the field NAMES from the wire: {second:?}"
        );
        assert!(
            !contains(second, b"Record"),
            "type-id elides the type NAME from the wire: {second:?}"
        );
        assert!(
            second.len() < first.len(),
            "the elided struct is strictly smaller than the self-describing one ({} vs {} bytes)",
            second.len(),
            first.len()
        );
    }

    // CROSS-TIER: the negotiated, name-elided wire is byte-identical between the VM and the tree-walker.
    // The cross-tier lock holds for the FULL crush, not merely the conservative first-contact fallback.
    assert_eq!(
        tw_second, vm_second,
        "CROSS-TIER REGRESSION: the type-id-elided struct send diverged between the VM ({vm_second:?}) \
         and the tree-walker ({tw_second:?}). Fix the tier, never this lock."
    );
    assert_eq!(
        tw_first, vm_first,
        "CROSS-TIER REGRESSION: the first-contact struct send diverged between the tiers."
    );

    // A receiver that SHARES the program's registry resolves the elided id back to the exact struct —
    // names absent from the wire, recovered from the shared type table.
    let reg = WireTypeRegistry::new(vec![(
        "Record".to_string(),
        vec!["alpha".to_string(), "beta".to_string()],
    )]);
    let (_from, value) =
        with_type_registry(reg, || message_from_wire(&tw_second)).expect("decodes with the shared registry");
    match value {
        RuntimeValue::Struct(s) => {
            assert_eq!(s.type_name, "Record", "the shared registry recovers the type name");
            assert_eq!(s.fields.get("alpha"), Some(&RuntimeValue::Int(1)));
            assert_eq!(s.fields.get("beta"), Some(&RuntimeValue::Int(2)));
        }
        other => panic!("the elided struct must decode to a Record struct, got {other:?}"),
    }
}
