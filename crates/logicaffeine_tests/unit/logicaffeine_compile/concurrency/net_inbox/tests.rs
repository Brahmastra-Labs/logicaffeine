use super::*;
use crate::concurrency::marshal::{
    make_handshake_frame, PeerProfile, ReceiveLimits, WireCompression, FEAT_COMPUTED, FEAT_LZ4,
    FEAT_TYPE_ID, FEAT_ZSTD,
};

#[test]
fn absorbs_a_peer_handshake_and_negotiates_to_its_exposed_surface() {
    let mut inbox = NetInbox::new();
    // I speak zstd+lz4+type-id+computed and carry type-registry epoch 5.
    inbox.my_profile.registry_epoch = 5;
    inbox.my_profile.features = FEAT_ZSTD | FEAT_LZ4 | FEAT_TYPE_ID | FEAT_COMPUTED;

    // Before any handshake, a peer is treated CONSERVATIVELY (no optional capabilities).
    assert_eq!(inbox.peer_profile("bob"), PeerProfile::conservative());
    // …so a send to an unheard peer negotiates to a plain, uncompressed, self-describing form.
    let cold = inbox.negotiation_for("bob");
    assert!(!cold.use_type_id && !cold.may_send_computed);
    assert_eq!(cold.compression, WireCompression::None);

    // Bob advertises a RESTRICTIVE surface: lz4 only (no zstd), type-id, same epoch, declines code,
    // small byte budget.
    let bob = PeerProfile {
        limits: ReceiveLimits {
            max_bytes: 2048,
            accept_computed: false,
            ..Default::default()
        },
        registry_epoch: 5,
        features: FEAT_LZ4 | FEAT_TYPE_ID,
    };
    inbox.enqueue_received(make_handshake_frame("bob", &bob));

    // The handshake was absorbed — stored, NOT queued as a data message.
    assert!(
        inbox.received.is_empty(),
        "a handshake is absorbed, never delivered as data"
    );
    assert_eq!(inbox.peer_profile("bob"), bob);

    // Sending to bob now negotiates to exactly his exposed surface.
    let n = inbox.negotiation_for("bob");
    assert!(
        n.use_type_id,
        "matching epochs + both type-id → names elided"
    );
    assert!(
        !n.may_send_computed,
        "bob declined computed → never ship code to him"
    );
    assert_eq!(
        n.compression,
        WireCompression::Lz4,
        "lz4 is the strongest compression both speak"
    );
    assert_eq!(n.peer_max_bytes, 2048, "stay under bob's byte budget");
}

#[test]
fn null_suite_seal_engages_through_enqueue_and_off_is_byte_identical() {
    use crate::concurrency::channel::{seal_active, with_suite, SUITE_NULL};
    use crate::concurrency::marshal::{message_to_wire_with, WireCodec, WireIntegrity};

    let blob = message_to_wire_with(
        "bob",
        &RuntimeValue::Int(7),
        WireCodec::Native,
        WireIntegrity::Checked,
    )
    .expect("encode");

    // OFF: no active suite → the wire is the raw blob and the receive path is unchanged.
    let mut off = NetInbox::new();
    let wire_off = seal_active(blob.clone());
    assert_eq!(wire_off, blob, "no suite → byte-identical wire");
    off.enqueue_received(wire_off);
    assert_eq!(
        off.received.len(),
        1,
        "message delivered with the channel disengaged"
    );

    // ON (null suite): the wire is the framed envelope and the receiver opens it back to the
    // same message — the seam engages through the real receive path.
    with_suite(Some(SUITE_NULL), || {
        let mut on = NetInbox::new();
        let wire_on = seal_active(blob.clone());
        assert_ne!(wire_on, blob, "active suite → framed envelope on the wire");
        on.enqueue_received(wire_on);
        assert_eq!(
            on.received.len(),
            1,
            "sealed message opened + delivered under the null suite"
        );

        // A body byte flipped after sealing: open() succeeds (null framing carries no tag of its
        // own) but the inner FNV checksum fails, so the message is dropped — the transitional
        // tamper-detection the plan relies on until an AEAD tag lands.
        let mut tampered = seal_active(blob.clone());
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF;
        on.enqueue_received(tampered);
        assert_eq!(
            on.received.len(),
            1,
            "tampered frame dropped, not delivered"
        );
    });
}

#[test]
fn my_handshake_advertises_my_inbox_identity_and_profile() {
    let mut inbox = NetInbox::new();
    inbox.inbox = Some(Rc::new("alice".to_string()));
    inbox.my_profile.registry_epoch = 9;
    let frame = inbox.my_handshake();
    let (from, prof) = marshal::parse_handshake_frame(&frame).expect("my handshake parses");
    assert_eq!(from, "alice", "advertises our inbox identity");
    assert_eq!(prof, inbox.my_profile, "advertises our exact profile");
}
