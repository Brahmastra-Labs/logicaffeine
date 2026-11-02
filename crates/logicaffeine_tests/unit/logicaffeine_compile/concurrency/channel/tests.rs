use super::super::marshal::{message_to_wire_with, WireCodec, WireIntegrity};
use super::*;
use crate::interpreter::RuntimeValue;

#[test]
fn null_envelope_round_trips_and_rejects_malformed() {
    let blob = message_to_wire_with(
        "alice",
        &RuntimeValue::Int(7),
        WireCodec::Native,
        WireIntegrity::Checked,
    )
    .expect("encode");

    let sealed = seal(SUITE_NULL, &blob);

    // Round-trip: the null envelope returns the exact blob.
    assert_eq!(
        open(&sealed).as_deref(),
        Some(blob.as_slice()),
        "null suite round-trip"
    );

    // Truncated header → None.
    assert!(open(&sealed[..2]).is_none(), "truncated envelope rejected");

    // Unknown suite id (0xFFFF) → None.
    let mut bad = sealed.clone();
    bad[2] = 0xFF;
    bad[3] = 0xFF;
    assert!(open(&bad).is_none(), "unknown suite rejected");
}

/// A small deterministic PRNG so the fuzz lock is reproducible (house style).
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[test]
fn null_round_trips_arbitrary_blobs() {
    let mut s = 0x1234_5678_9ABC_DEF0u64;
    for _ in 0..2000 {
        let len = (splitmix64(&mut s) % 257) as usize; // 0..=256, incl. empty
        let blob: Vec<u8> = (0..len).map(|_| splitmix64(&mut s) as u8).collect();
        let sealed = seal(SUITE_NULL, &blob);
        assert_eq!(
            open(&sealed),
            Some(blob),
            "null suite round-trip, len {len}"
        );
    }
}

#[test]
fn rejects_bad_magic_version_and_empty() {
    let sealed = seal(SUITE_NULL, b"payload");
    assert!(open(&[]).is_none(), "empty input rejected");
    assert!(
        open(&sealed[..CHAN_HEADER_LEN - 1]).is_none(),
        "header-short rejected"
    );

    let mut bad_magic = sealed.clone();
    bad_magic[0] ^= 0xFF;
    assert!(open(&bad_magic).is_none(), "bad magic rejected");

    let mut bad_ver = sealed.clone();
    bad_ver[1] = bad_ver[1].wrapping_add(1);
    assert!(open(&bad_ver).is_none(), "bad version rejected");
}

#[test]
fn seal_is_framed_not_raw() {
    let blob = b"the relay sees only the envelope";
    let sealed = seal(SUITE_NULL, blob);
    assert_eq!(
        &sealed[..2],
        &[CHAN_MAGIC, CHAN_VER],
        "header magic+version present"
    );
    assert_eq!(
        &sealed[2..4],
        &SUITE_NULL.to_le_bytes(),
        "suite id bound little-endian"
    );
    assert_eq!(
        sealed.len(),
        CHAN_HEADER_LEN + blob.len(),
        "null body length-exact"
    );
    assert_ne!(
        sealed.as_slice(),
        blob.as_slice(),
        "sealed bytes differ from the raw blob"
    );
}

#[test]
fn null_suite_trait_dispatch() {
    let suite = suite_for(SUITE_NULL).expect("null suite registered");
    assert_eq!(suite.id(), SUITE_NULL);
    let body = suite.seal_body(b"abc");
    assert_eq!(
        suite.open_body(&body).as_deref(),
        Some(&b"abc"[..]),
        "identity body"
    );
    assert!(
        suite_for(0xFFFF).is_none(),
        "unknown suite id is not registered"
    );
}

#[test]
fn no_active_suite_is_passthrough() {
    assert_eq!(active_suite(), None, "default: channel disengaged");
    let blob = b"plain".to_vec();
    assert_eq!(seal_active(blob.clone()), blob, "seal is identity when off");
    assert_eq!(
        open_active(blob.clone()),
        Some(blob),
        "open is identity when off"
    );
}

#[test]
fn with_suite_scopes_and_restores() {
    assert_eq!(active_suite(), None);
    with_suite(Some(SUITE_NULL), || {
        assert_eq!(
            active_suite(),
            Some(SUITE_NULL),
            "suite active inside scope"
        );
        let blob = b"secret payload".to_vec();
        let sealed = seal_active(blob.clone());
        assert_ne!(sealed, blob, "sealed under an active suite is framed");
        assert_eq!(
            open_active(sealed),
            Some(blob),
            "round-trip under the active suite"
        );
    });
    assert_eq!(active_suite(), None, "prior suite restored after scope");
}

#[test]
fn open_active_drops_foreign_frame_under_active_suite() {
    with_suite(Some(SUITE_NULL), || {
        assert!(
            open_active(b"not an envelope".to_vec()).is_none(),
            "foreign frame dropped"
        );
    });
}

#[test]
fn pq_channel_seals_a_real_wire_message_end_to_end() {
    // Full post-quantum handshake + AEAD seal over an actual marshalled wire blob.
    // Initiator generates an ML-KEM-768 keypair and publishes ek; responder encapsulates to it
    // and returns the ciphertext; both derive the same directional ChaCha20-Poly1305 suites.
    let hs = pq_handshake_initiator(&[0xA1; 32], &[0xA2; 32]);
    let (ct, resp_i2r, resp_r2i) =
        pq_handshake_responder(&hs.ek, &[0xB0; 32]).expect("in-contract arguments");
    let (init_i2r, init_r2i) = pq_handshake_finish(&hs, &ct).expect("in-contract arguments");

    // The actual payload is a real codec-encoded wire blob, exactly what the transport carries.
    let blob = message_to_wire_with(
        "responder",
        &RuntimeValue::Int(42),
        WireCodec::Native,
        WireIntegrity::Checked,
    )
    .expect("encode");

    // Responder→initiator: responder seals, initiator opens. End-to-end post-quantum secrecy.
    let sealed = seal_with(&resp_r2i, &blob);
    assert_eq!(
        &sealed[..2],
        &[CHAN_MAGIC, CHAN_VER],
        "channel header present"
    );
    assert_eq!(
        &sealed[2..4],
        &SUITE_PQ.to_le_bytes(),
        "PQ suite id bound in the envelope"
    );
    assert_ne!(
        sealed[CHAN_HEADER_LEN..].to_vec(),
        blob,
        "body is ciphertext, not the raw blob"
    );
    assert_eq!(
        open_with(&init_r2i, &sealed).as_deref(),
        Some(blob.as_slice()),
        "initiator opens the responder's PQ-sealed wire message"
    );

    // Initiator→responder direction works independently (distinct key).
    let s2 = seal_with(&init_i2r, b"ack");
    assert_eq!(
        open_with(&resp_i2r, &s2).as_deref(),
        Some(&b"ack"[..]),
        "i2r direction opens"
    );

    // The counter nonce makes every seal unique, and each still opens.
    let sealed_again = seal_with(&resp_r2i, &blob);
    assert_ne!(
        sealed, sealed_again,
        "fresh counter nonce ⇒ a distinct frame each time"
    );
    assert_eq!(
        open_with(&init_r2i, &sealed_again).as_deref(),
        Some(blob.as_slice())
    );

    // A tampered tag is rejected; a foreign session key (failed handshake) cannot open.
    let mut tampered = sealed.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(
        open_with(&init_r2i, &tampered).is_none(),
        "tampered PQ frame rejected"
    );
    let eve = PqSuite::new(derive_aead_key(&[0u8; 32], b"r2i"));
    assert!(
        open_with(&eve, &sealed).is_none(),
        "wrong session key cannot open"
    );
}

#[test]
fn pq_handshake_disagrees_on_a_corrupted_ciphertext() {
    // If the KEM ciphertext is corrupted in flight, ML-KEM's implicit reject makes the two
    // sides derive DIFFERENT secrets, so the initiator cannot open the responder's frames.
    let hs = pq_handshake_initiator(&[1; 32], &[2; 32]);
    let (ct, _resp_i2r, resp_r2i) =
        pq_handshake_responder(&hs.ek, &[3; 32]).expect("in-contract arguments");
    let mut bad_ct = ct.clone();
    bad_ct[0] ^= 1;
    let (_init_i2r, init_r2i) = pq_handshake_finish(&hs, &bad_ct).expect("in-contract arguments");
    let sealed = seal_with(&resp_r2i, b"top secret");
    assert!(
        open_with(&init_r2i, &sealed).is_none(),
        "a corrupted handshake yields divergent keys ⇒ frames don't open"
    );
}

#[test]
fn the_responder_refuses_a_malformed_encapsulation_key_from_the_wire() {
    // The initiator's ek arrives from the peer: a short, long or unreduced one is a refusal, never a
    // panic of the responder and never a key derived from it.
    use logicaffeine_system::mlkem::{KemError, EK_BYTES};
    let hs = pq_handshake_initiator(&[0x41; 32], &[0x42; 32]);
    for len in [0usize, 31, EK_BYTES - 1, EK_BYTES + 1] {
        let mut ek = hs.ek.clone();
        ek.resize(len, 0);
        assert_eq!(
            pq_handshake_responder(&ek, &[0x43; 32]).err(),
            Some(KemError::EncapsulationKeyLength { len }),
            "a {len}-byte ek"
        );
    }
    let mut unreduced = hs.ek.clone();
    unreduced[0] = 0xFF;
    unreduced[1] |= 0x0F;
    assert_eq!(
        pq_handshake_responder(&unreduced, &[0x43; 32]).err(),
        Some(KemError::EncapsulationKeyNotReduced)
    );
}

#[test]
fn the_initiator_refuses_a_ciphertext_of_the_wrong_length_from_the_wire() {
    use logicaffeine_system::mlkem::{KemError, CT_BYTES};
    let hs = pq_handshake_initiator(&[0x51; 32], &[0x52; 32]);
    let (ct, _, _) = pq_handshake_responder(&hs.ek, &[0x53; 32]).expect("an honest ek");
    for len in [0usize, 1, CT_BYTES - 1, CT_BYTES + 1, 2 * CT_BYTES] {
        let mut wrong = ct.clone();
        wrong.resize(len, 0);
        assert_eq!(
            pq_handshake_finish(&hs, &wrong).err(),
            Some(KemError::CiphertextLength { len }),
            "a {len}-byte ciphertext"
        );
    }
}

/// The two ends of one direction of a PQ session: the suite that seals and the suite that opens.
fn pq_pair() -> (PqSuite, PqSuite) {
    let key = derive_aead_key(&[0x5A; 32], b"i2r");
    (PqSuite::new(key), PqSuite::new(key))
}

#[test]
fn a_pq_frame_opens_once() {
    let (tx, rx) = pq_pair();
    let frame = seal_with(&tx, b"pay 10 to bob");
    assert_eq!(open_with(&rx, &frame).as_deref(), Some(&b"pay 10 to bob"[..]));
    assert_eq!(open_with(&rx, &frame), None, "a replayed frame is refused");
    assert_eq!(open_with(&rx, &frame), None, "and refused again");
}

#[test]
fn pq_frames_open_in_any_order_within_the_window() {
    let (tx, rx) = pq_pair();
    let frames: Vec<Vec<u8>> = (0..5u8).map(|i| seal_with(&tx, &[i])).collect();
    for i in [3usize, 1, 4, 0, 2] {
        assert_eq!(open_with(&rx, &frames[i]).as_deref(), Some(&[i as u8][..]), "frame {i}");
    }
    for (i, frame) in frames.iter().enumerate() {
        assert_eq!(open_with(&rx, frame), None, "frame {i} again");
    }
}

#[test]
fn a_pq_frame_older_than_the_window_is_refused() {
    let (tx, rx) = pq_pair();
    let frames: Vec<Vec<u8>> = (0..PQ_REPLAY_WINDOW + 1).map(|i| seal_with(&tx, &i.to_le_bytes())).collect();
    let newest = frames.last().unwrap();
    assert!(open_with(&rx, newest).is_some());
    assert_eq!(
        open_with(&rx, &frames[0]),
        None,
        "a frame {PQ_REPLAY_WINDOW} behind the newest can no longer be told from a replay"
    );
    assert!(open_with(&rx, &frames[1]).is_some(), "the oldest frame inside the window still opens");
}

#[test]
fn a_frame_that_does_not_authenticate_spends_nothing() {
    let (tx, rx) = pq_pair();
    let frame = seal_with(&tx, b"genuine");
    let mut tampered = frame.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x80;
    assert_eq!(open_with(&rx, &tampered), None);
    let mut far_ahead = frame.clone();
    far_ahead[CHAN_HEADER_LEN..CHAN_HEADER_LEN + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(open_with(&rx, &far_ahead), None, "a forged sequence number does not authenticate");
    assert_eq!(
        open_with(&rx, &frame).as_deref(),
        Some(&b"genuine"[..]),
        "neither forgery moved the window or spent the genuine frame's number"
    );
}

#[test]
fn a_frame_opened_on_two_threads_at_once_opens_on_one() {
    let (tx, rx) = pq_pair();
    for round in 0..200u32 {
        let frame = seal_with(&tx, &round.to_le_bytes());
        let opened = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    if open_with(&rx, &frame).is_some() {
                        opened.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            }
        });
        assert_eq!(opened.into_inner(), 1, "round {round}: exactly one opener accepts the frame");
    }
}

#[test]
fn the_replay_window_answers_as_the_set_of_every_number_it_opened() {
    for span in [1u64, 2, 3, 63, 64, 65, 100, PQ_REPLAY_WINDOW] {
        let mut window = ReplayWindow::new(span);
        let mut opened = std::collections::BTreeSet::new();
        let mut newest: Option<u64> = None;
        let mut state = 0x243F_6A88_85A3_08D3u64 ^ span;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut base = 0u64;
        for step in 0..50_000u32 {
            let r = next();
            base += match r % 100 {
                0 => 3 * span,
                1..=4 => span / 2 + 1,
                _ => r % 3,
            };
            let seq = (base + (r >> 32) % (2 * span + 2)).saturating_sub(span);
            let expected = !opened.contains(&seq)
                && newest.is_none_or(|n| seq > n || n - seq < span);
            assert_eq!(window.admits(seq), expected, "span {span}, step {step}: seq {seq}, newest {newest:?}");
            assert_eq!(window.accept(seq), expected, "span {span}, step {step}: seq {seq}, newest {newest:?}");
            if expected {
                opened.insert(seq);
                newest = Some(newest.map_or(seq, |n| n.max(seq)));
            }
        }
        assert!(opened.len() > 5_000, "span {span}: the walk opened a healthy share ({})", opened.len());
    }
}

#[test]
fn a_resumed_window_refuses_everything_below_its_mark_and_opens_what_follows() {
    let mut window = ReplayWindow::resumed_at(8, 100);
    for seq in [0u64, 50, 92, 93, 99] {
        assert!(!window.admits(seq), "{seq} may have opened before the restart");
    }
    assert!(window.accept(100));
    assert!(window.accept(103));
    assert!(window.accept(101), "a number after the mark opens out of order");
    assert!(!window.accept(100), "and once");
    let mut fresh = ReplayWindow::resumed_at(8, 0);
    assert!(fresh.accept(0), "a mark of 0 remembers nothing");
}
