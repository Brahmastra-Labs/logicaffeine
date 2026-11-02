use super::super::marshal::{message_to_wire_with, WireCodec, WireIntegrity};
use super::*;
use crate::interpreter::RuntimeValue;

/// The same deterministic PRNG the sibling channel tests use — reproducible "true-random" pad
/// material with full byte entropy (the classifier rates it incompressible).
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn random_pad(len: usize, seed: u64) -> Vec<u8> {
    let mut s = seed;
    (0..len).map(|_| splitmix64(&mut s) as u8).collect()
}

const PAYLOAD_START: usize = CHAN_HEADER_LEN + PREAMBLE;

/// The ciphertext/payload bytes carried in a PNP frame body.
fn ciphertext_of(frame: &[u8]) -> Vec<u8> {
    let off_at = CHAN_HEADER_LEN + KIND_LEN + EPOCH_LEN + OFF_LEN;
    let len = u32::from_le_bytes(frame[off_at..off_at + LEN_LEN].try_into().unwrap()) as usize;
    frame[PAYLOAD_START..PAYLOAD_START + len].to_vec()
}

/// The `[offset, offset + len)` keystream a frame draws, half-local.
fn cover_range(frame: &[u8]) -> (u64, u64) {
    let off = frame_offset(frame).unwrap();
    let len = ciphertext_of(frame).len();
    (off, off + len as u64)
}

/// The half-local pad range a frame's MAC key comes from, in a half of `half` bytes: its key slot,
/// counted down from the top.
fn key_range(frame: &[u8], half: usize) -> (u64, u64) {
    let slot = frame_key_slot(frame).unwrap();
    let top = half as u64 - MAC_KEY_LEN as u64 * slot;
    (top - MAC_KEY_LEN as u64, top)
}

fn ranges_disjoint(ranges: &[(u64, u64)]) -> bool {
    for i in 0..ranges.len() {
        for j in (i + 1)..ranges.len() {
            let (a0, a1) = ranges[i];
            let (b0, b1) = ranges[j];
            if a0 < b1 && b0 < a1 {
                return false;
            }
        }
    }
    true
}

#[test]
fn roundtrip_seal_open() {
    let pool =
        PadPool::shared(random_pad(4096, 0x1111_2222_3333_4444)).expect("incompressible pad");
    let sender = pool.send_suite(Role::Initiator);
    let receiver = pool.recv_suite(Role::Responder);
    let msg = b"attack at dawn";
    let frame = sender.seal(msg).expect("pad available");
    assert_eq!(
        receiver.open(&frame).as_deref(),
        Some(&msg[..]),
        "cover round-trips exactly"
    );
}

#[test]
fn ciphertext_is_plaintext_xor_pad() {
    let keystream = [0xA5u8, 0x5A, 0xFF, 0x00, 0x13, 0x37, 0xBE, 0xEF];
    let mut pad = keystream.to_vec();
    pad.extend(std::iter::repeat(7u8).take(MAC_KEY_LEN + 64));
    let pool = PadPool::shared_unchecked(pad);
    let sender = pool.send_suite(Role::Initiator);
    let plaintext = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let frame = sender.seal(&plaintext).expect("pad available");
    let ct = ciphertext_of(&frame);
    let expect: Vec<u8> = plaintext
        .iter()
        .zip(&keystream)
        .map(|(p, k)| p ^ k)
        .collect();
    assert_eq!(ct, expect, "ciphertext is the exact plaintext ⊕ pad");
    assert_ne!(ct, plaintext.to_vec(), "ciphertext is not the plaintext");
}

#[test]
fn seal_matches_independent_reference() {
    // Reconstruct the exact wire bytes with a wholly independent implementation and demand
    // byte-for-byte equality — pins the frame format and the XOR/MAC construction.
    let pad = random_pad(1024, 0xCAFE_F00D_1234_5678);
    let pool = PadPool::shared_unchecked(pad.clone()).with_epoch(0);
    let sender = pool.send_suite(Role::Initiator); // first half, base 0
    let msg = b"reference vector";
    let frame = sender.seal(msg).expect("pad");

    // Independent reference over the first half (base 0, 512 bytes): keystream from its bottom at
    // offset 0, the MAC key from key slot 0 at its top.
    let mac_key: [u8; 32] = pad[512 - 32..512].try_into().unwrap();
    let ks = &pad[0..msg.len()];
    let ct: Vec<u8> = msg.iter().zip(ks).map(|(m, k)| m ^ k).collect();
    let mut expect = Vec::new();
    expect.push(CHAN_MAGIC);
    expect.push(CHAN_VER);
    expect.extend_from_slice(&SUITE_PNP.to_le_bytes());
    expect.push(KIND_DATA);
    expect.extend_from_slice(&0u32.to_le_bytes()); // epoch
    expect.extend_from_slice(&0u64.to_le_bytes()); // keystream offset
    expect.extend_from_slice(&(msg.len() as u32).to_le_bytes());
    expect.extend_from_slice(&0u64.to_le_bytes()); // key slot
    expect.extend_from_slice(&ct);
    let tag = logicaffeine_system::aead::poly1305(&mac_key, &expect[CHAN_HEADER_LEN..]);
    expect.extend_from_slice(&tag);
    assert_eq!(
        frame, expect,
        "sealed frame matches the independent reference byte-for-byte"
    );
}

#[test]
fn never_reuses_pad_bytes() {
    let pool = PadPool::shared(random_pad(8192, 0xDEAD_BEEF_0000_0001)).expect("pad");
    let half = pool.direction_len();
    let sender = pool.send_suite(Role::Initiator);
    let (mut streams, mut keys) = (Vec::new(), Vec::new());
    let mut s = 0xABCD_1234u64;
    for _ in 0..200 {
        let len = (splitmix64(&mut s) % 17) as usize;
        let msg: Vec<u8> = (0..len).map(|_| splitmix64(&mut s) as u8).collect();
        if let Some(frame) = sender.seal(&msg) {
            streams.push(cover_range(&frame));
            keys.push(key_range(&frame, half));
        }
    }
    assert!(!streams.is_empty(), "some covers were sealed");
    let every: Vec<(u64, u64)> = streams
        .iter()
        .chain(&keys)
        .copied()
        .filter(|(a, b)| a < b)
        .collect();
    assert!(
        ranges_disjoint(&every),
        "no pad byte serves two covers, or both keystream and a MAC key"
    );
    for w in streams.windows(2) {
        assert_eq!(w[0].1, w[1].0, "keystream is laid down contiguously from the bottom");
    }
    for w in keys.windows(2) {
        assert_eq!(w[0].0, w[1].1, "key slots are laid down contiguously from the top");
    }
}

#[test]
fn concurrent_seals_never_overlap() {
    let pool = PadPool::shared(random_pad(1 << 16, 0x0F0F_0F0F_1234_5678)).expect("pad");
    let half = pool.direction_len();
    let sender = Arc::new(pool.send_suite(Role::Initiator));
    let mut handles = Vec::new();
    for t in 0..8 {
        let s = sender.clone();
        handles.push(std::thread::spawn(move || {
            let mut out = Vec::new();
            for i in 0..64 {
                let msg = [(t as u8), (i as u8), 0xEE];
                if let Some(frame) = s.seal(&msg) {
                    out.push(cover_range(&frame));
                    out.push(key_range(&frame, half));
                }
            }
            out
        }));
    }
    let mut ranges = Vec::new();
    for h in handles {
        ranges.extend(h.join().unwrap());
    }
    assert_eq!(ranges.len(), 2 * 8 * 64, "every concurrent seal succeeded");
    assert!(
        ranges_disjoint(&ranges),
        "concurrent covers are pairwise disjoint"
    );
}

#[test]
fn directional_split_no_overlap() {
    let mut pad = random_pad(2048, 0x5555_6666_7777_8888);
    let mid = pad.len() / 2;
    for b in &mut pad[..mid] {
        *b |= 0x01;
    }
    for b in &mut pad[mid..] {
        *b &= 0xFE;
    }
    let pool = PadPool::shared_unchecked(pad);

    let init_send = pool.send_suite(Role::Initiator);
    let resp_send = pool.send_suite(Role::Responder);
    let msg = b"same plaintext";
    let f_i = init_send.seal(msg).expect("pad");
    let f_r = resp_send.seal(msg).expect("pad");
    assert_ne!(
        ciphertext_of(&f_i),
        ciphertext_of(&f_r),
        "different halves ⇒ different ciphertext"
    );

    assert_eq!(
        pool.recv_suite(Role::Responder).open(&f_i).as_deref(),
        Some(&msg[..]),
        "matching direction opens"
    );
    assert!(
        pool.recv_suite(Role::Initiator).open(&f_i).is_none(),
        "wrong direction cannot open"
    );
}

#[test]
fn bitflip_detected() {
    let pool = PadPool::shared(random_pad(4096, 0x9999_AAAA_BBBB_CCCC)).expect("pad");
    let sender = pool.send_suite(Role::Initiator);
    let frame = sender.seal(b"integrity matters").expect("pad");

    let mut ct_tamper = frame.clone();
    ct_tamper[PAYLOAD_START] ^= 0x01;
    assert!(
        pool.recv_suite(Role::Responder).open(&ct_tamper).is_none(),
        "flipped ciphertext bit caught"
    );

    let mut tag_tamper = frame.clone();
    let last = tag_tamper.len() - 1;
    tag_tamper[last] ^= 0x80;
    assert!(
        pool.recv_suite(Role::Responder).open(&tag_tamper).is_none(),
        "flipped tag bit caught"
    );

    assert!(
        pool.recv_suite(Role::Responder).open(&frame).is_some(),
        "untampered cover opens"
    );
}

#[test]
fn every_single_byte_corruption_is_rejected() {
    // Absurd-robustness: for a batch of covers, flipping ANY single byte anywhere in the frame
    // must make a fresh receiver reject it — no malleability, no accidental re-parse into a valid
    // (offset,len) that still authenticates.
    let pool = PadPool::shared(random_pad(1 << 15, 0x0BADF00D_5EED_1234)).expect("pad");
    let sender = pool.send_suite(Role::Initiator);
    let mut s = 0x1357_9BDF_2468_ACE0u64;
    for _ in 0..40 {
        let len = (splitmix64(&mut s) % 24) as usize;
        let msg: Vec<u8> = (0..len).map(|_| splitmix64(&mut s) as u8).collect();
        let frame = sender.seal(&msg).expect("pad");
        // sanity: pristine opens
        assert!(pool.recv_suite(Role::Responder).open(&frame).is_some());
        for pos in 0..frame.len() {
            for bit in [0x01u8, 0x80] {
                let mut bad = frame.clone();
                bad[pos] ^= bit;
                assert!(
                    pool.recv_suite(Role::Responder).open(&bad).is_none(),
                    "corrupting byte {pos} (bit {bit:#x}) must be rejected"
                );
            }
        }
    }
}

#[test]
fn cross_pad_frame_is_rejected() {
    let a = PadPool::shared(random_pad(4096, 0xAAAA_0000_1111_2222)).expect("pad A");
    let b = PadPool::shared(random_pad(4096, 0xBBBB_3333_4444_5555)).expect("pad B");
    let frame = a
        .send_suite(Role::Initiator)
        .seal(b"for A only")
        .expect("pad");
    assert!(
        a.recv_suite(Role::Responder).open(&frame).is_some(),
        "A's receiver opens it"
    );
    assert!(
        b.recv_suite(Role::Responder).open(&frame).is_none(),
        "a foreign pad cannot open it"
    );
}

#[test]
fn fuzz_roundtrip_many_sizes() {
    let pool = PadPool::shared(random_pad(1 << 20, 0xF1F2_F3F4_F5F6_F7F8)).expect("pad");
    let sender = pool.send_suite(Role::Initiator);
    let receiver = pool.recv_suite(Role::Responder);
    let mut s = 0x2222_4444_6666_8888u64;
    let mut sealed = 0;
    for _ in 0..3000 {
        let len = (splitmix64(&mut s) % 300) as usize;
        let msg: Vec<u8> = (0..len).map(|_| splitmix64(&mut s) as u8).collect();
        match sender.seal(&msg) {
            Some(frame) => {
                assert_eq!(
                    receiver.open(&frame),
                    Some(msg),
                    "fuzz round-trip, len {len}"
                );
                sealed += 1;
            }
            None => break, // exhausted
        }
    }
    assert!(
        sealed > 100,
        "a healthy number of covers round-tripped ({sealed})"
    );
}

#[test]
fn mac_key_is_one_time() {
    let pool = PadPool::shared(random_pad(8192, 0x1212_3434_5656_7878)).expect("pad");
    let half = pool.direction_len();
    let sender = pool.send_suite(Role::Initiator);
    let (mut mac_ranges, mut streams) = (Vec::new(), Vec::new());
    for i in 0..100u8 {
        let frame = sender.seal(&[i, i, i]).expect("pad");
        mac_ranges.push(key_range(&frame, half));
        streams.push(cover_range(&frame));
    }
    assert!(
        ranges_disjoint(&mac_ranges),
        "MAC keys are drawn from pairwise-disjoint pad ranges"
    );
    let lowest_key = mac_ranges.iter().map(|r| r.0).min().unwrap();
    let highest_stream = streams.iter().map(|r| r.1).max().unwrap();
    assert!(
        highest_stream <= lowest_key,
        "no MAC key is pad any cover's keystream was drawn from"
    );
}

#[test]
fn replay_rejected() {
    let pool = PadPool::shared(random_pad(4096, 0x2468_ACE0_1357_9BDF)).expect("pad");
    let sender = pool.send_suite(Role::Initiator);
    let receiver = pool.recv_suite(Role::Responder);
    let frame = sender.seal(b"once and only once").expect("pad");
    assert!(receiver.open(&frame).is_some(), "first delivery accepted");
    assert!(
        receiver.open(&frame).is_none(),
        "exact replay of a consumed cover rejected"
    );
}

#[test]
fn reorder_within_window_ok_beyond_window_rejected() {
    let pool = PadPool::shared(random_pad(4096, 0x3141_5926_5358_9793)).expect("pad");
    let sender = pool.send_suite(Role::Initiator);
    let f0 = sender.seal(&[0u8; 8]).unwrap();
    let f1 = sender.seal(&[1u8; 8]).unwrap();
    let f2 = sender.seal(&[2u8; 8]).unwrap();
    assert_eq!(frame_offset(&f0), Some(0));
    assert_eq!(frame_offset(&f1), Some(8));
    assert_eq!(frame_offset(&f2), Some(16));
    assert_eq!(
        [frame_key_slot(&f0), frame_key_slot(&f1), frame_key_slot(&f2)],
        [Some(0), Some(1), Some(2)]
    );

    let receiver = pool.recv_suite(Role::Responder).with_recv_window(2);
    assert!(
        receiver.open(&f2).is_some(),
        "newest cover accepted (slot 2)"
    );
    assert!(
        receiver.open(&f1).is_some(),
        "in-window reorder (slot 1, one behind the newest) accepted"
    );
    assert!(
        receiver.open(&f0).is_none(),
        "beyond-window cover (slot 0, two behind the newest) rejected"
    );
}

#[test]
fn drop_then_resync() {
    let pool = PadPool::shared(random_pad(4096, 0xF00D_BABE_1234_5678)).expect("pad");
    let sender = pool.send_suite(Role::Initiator);
    let f0 = sender.seal(b"first").unwrap();
    let _dropped = sender.seal(b"lost in flight").unwrap();
    let f2 = sender.seal(b"third").unwrap();

    let receiver = pool.recv_suite(Role::Responder);
    assert_eq!(
        receiver.open(&f0).as_deref(),
        Some(&b"first"[..]),
        "first arrives"
    );
    assert_eq!(
        receiver.open(&f2).as_deref(),
        Some(&b"third"[..]),
        "third decodes despite the gap"
    );
}

#[test]
fn exhaustion_fails_closed() {
    let pool = PadPool::shared_unchecked(random_pad(2 * (MAC_KEY_LEN + 8), 0xC0DE_F00D_0BAD_BEEF));
    let sender = pool.send_suite(Role::Initiator);
    assert_eq!(sender.send_remaining(), MAC_KEY_LEN + 8);
    assert!(
        sender.seal(&[0u8; 8]).is_some(),
        "the one cover the half holds is sealed"
    );
    assert_eq!(sender.send_remaining(), 0, "the half is now spent");
    assert!(sender.is_low(1), "low-water tripped");
    assert!(
        sender.seal(&[0u8; 1]).is_none(),
        "exhausted pad fails closed"
    );
    assert!(
        sender.seal(&[]).is_none(),
        "even an empty message needs a MAC key ⇒ still closed"
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn cursor_persists_across_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("pnp.cursor");
    let pad = random_pad(8192, 0x0BEE_F00D_1234_ABCD);

    let end1 = {
        let pool = PadPool::shared(pad.clone()).expect("pad");
        let sender = pool
            .send_suite(Role::Initiator)
            .with_ledger(Arc::new(FileLedger::new(&path)));
        let f = sender.seal(b"before the crash").expect("pad");
        cover_range(&f).1
    };

    let pool = PadPool::shared(pad).expect("pad");
    let sender = pool
        .send_suite(Role::Initiator)
        .with_ledger(Arc::new(FileLedger::new(&path)));
    let f2 = sender.seal(b"after the restart").expect("pad");
    let (off2, _) = cover_range(&f2);
    assert!(
        off2 >= end1,
        "post-restart cover starts at or beyond the durable high-water ({off2} ≥ {end1})"
    );
}

#[test]
fn seal_is_deterministic_no_runtime_randomness() {
    let pad = random_pad(4096, 0xAAAA_5555_AAAA_5555);
    let a = PadPool::shared(pad.clone())
        .unwrap()
        .send_suite(Role::Initiator);
    let b = PadPool::shared(pad).unwrap().send_suite(Role::Initiator);
    let m = b"deterministic";
    assert_eq!(
        a.seal(m),
        b.seal(m),
        "seal is a pure function of (pad, cursor, plaintext)"
    );
    let f1 = a.seal(m).unwrap();
    let f2 = a.seal(m).unwrap();
    assert_ne!(f1, f2, "successive covers draw fresh pad");
}

#[test]
fn pad_quality_gate_rejects_compressible() {
    assert_eq!(
        PadPool::shared(vec![0u8; 4096]).err(),
        Some(PadError::Compressible),
        "zero pad refused"
    );
    let counter: Vec<u8> = (0..4096).map(|i| i as u8).collect();
    assert_eq!(
        PadPool::shared(counter).err(),
        Some(PadError::Compressible),
        "counter pad refused"
    );
    assert_eq!(
        PadPool::shared(vec![1]).err(),
        Some(PadError::TooSmall),
        "tiny pool refused"
    );
    assert!(
        PadPool::shared(random_pad(4096, 0x7777_8888_9999_0000)).is_ok(),
        "true-random pad accepted"
    );
}

#[test]
fn perfect_secrecy_leaks_only_length() {
    let target_ct = [0x11u8, 0x22, 0x33, 0x44];
    let p1 = [0xDEu8, 0xAD, 0xBE, 0xEF];
    let p2 = [0x00u8, 0x01, 0x02, 0x03];

    let build = |plaintext: &[u8]| {
        let mut pad: Vec<u8> = plaintext.iter().zip(&target_ct).map(|(p, c)| p ^ c).collect();
        pad.extend(std::iter::repeat(0x5Au8).take(MAC_KEY_LEN + 64));
        let pool = PadPool::shared_unchecked(pad);
        let frame = pool.send_suite(Role::Initiator).seal(plaintext).unwrap();
        ciphertext_of(&frame)
    };
    assert_eq!(
        build(&p1),
        target_ct.to_vec(),
        "plaintext 1 seals to the target ciphertext under its pad"
    );
    assert_eq!(
        build(&p2),
        target_ct.to_vec(),
        "a different plaintext seals to the SAME ciphertext under its pad"
    );
    assert_ne!(p1, p2, "the two plaintexts are genuinely different");
}

#[test]
fn end_to_end_two_peers_real_wire_message() {
    let material = random_pad(1 << 14, 0x1BAD_C0DE_F00D_FACE);
    let initiator = PadPool::shared(material.clone()).expect("pad");
    let responder = PadPool::shared(material).expect("pad");

    let blob = message_to_wire_with(
        "responder",
        &RuntimeValue::Int(42),
        WireCodec::Native,
        WireIntegrity::Checked,
    )
    .expect("encode");

    let sealed = initiator
        .send_suite(Role::Initiator)
        .seal(&blob)
        .expect("pad");
    assert_eq!(
        &sealed[..2],
        &[CHAN_MAGIC, CHAN_VER],
        "channel header present"
    );
    assert_eq!(
        &sealed[2..4],
        &SUITE_PNP.to_le_bytes(),
        "PNP suite id bound in the envelope"
    );
    assert_eq!(frame_kind(&sealed), Some(KIND_DATA), "a data cover");
    assert_ne!(
        ciphertext_of(&sealed),
        blob,
        "body is ciphertext, not the raw blob"
    );
    assert_eq!(
        responder
            .recv_suite(Role::Responder)
            .open(&sealed)
            .as_deref(),
        Some(blob.as_slice()),
        "responder opens the initiator's one-time-pad-sealed wire message"
    );

    let ack = responder
        .send_suite(Role::Responder)
        .seal(b"ack")
        .expect("pad");
    assert_eq!(
        initiator.recv_suite(Role::Initiator).open(&ack).as_deref(),
        Some(&b"ack"[..]),
        "reverse direction opens"
    );
    assert_eq!(frame_offset(&sealed), Some(0));
    assert_eq!(frame_offset(&ack), Some(0));
}

// ---- The next pad: epoch tagging + authenticated rollover handoff -----------------------------

#[test]
fn epoch_mismatch_rejected() {
    // A cover sealed under epoch 2 is refused by a receiver on epoch 1 — pad epochs never mix.
    let material = random_pad(4096, 0x0E0E_0E0E_1234_5678);
    let e1 = PadPool::shared(material.clone()).unwrap().with_epoch(1);
    let e2 = PadPool::shared(material).unwrap().with_epoch(2);
    let frame = e2
        .send_suite(Role::Initiator)
        .seal(b"epoch two")
        .expect("pad");
    assert_eq!(frame_kind(&frame), Some(KIND_DATA));
    assert!(
        e1.recv_suite(Role::Responder).open(&frame).is_none(),
        "epoch-1 receiver rejects an epoch-2 cover"
    );
    assert!(
        e2.recv_suite(Role::Responder).open(&frame).is_some(),
        "the matching epoch opens it"
    );
}

#[test]
fn pad_commitment_binds_bytes_and_epoch() {
    let m = random_pad(4096, 0x9090_9090_ABAB_ABAB);
    let a = PadPool::shared(m.clone()).unwrap().with_epoch(7);
    let a2 = PadPool::shared(m.clone()).unwrap().with_epoch(7);
    let diff_epoch = PadPool::shared(m).unwrap().with_epoch(8);
    let diff_bytes = PadPool::shared(random_pad(4096, 0x1111_2222_3333_4444))
        .unwrap()
        .with_epoch(7);
    assert_eq!(
        a.commitment(),
        a2.commitment(),
        "same bytes + epoch ⇒ identical commitment"
    );
    assert_ne!(
        a.commitment(),
        diff_epoch.commitment(),
        "epoch is bound into the commitment"
    );
    assert_ne!(
        a.commitment(),
        diff_bytes.commitment(),
        "pad bytes are bound into the commitment"
    );
}

#[test]
fn roll_handoff_roundtrip_and_authenticated() {
    // Peers on epoch 0 provision epoch 1 out of band; the sender announces it via an authenticated
    // roll cover; the receiver recovers the next epoch id + commitment and confirms it holds the
    // identical next pad.
    let pad0 = random_pad(1 << 13, 0x0000_1111_2222_3333);
    let next_material = random_pad(1 << 13, 0x4444_5555_6666_7777);
    let init0 = PadPool::shared(pad0.clone()).unwrap().with_epoch(0);
    let resp0 = PadPool::shared(pad0).unwrap().with_epoch(0);
    let next_on_init = PadPool::shared(next_material.clone())
        .unwrap()
        .with_epoch(1);
    let next_on_resp = PadPool::shared(next_material).unwrap().with_epoch(1);

    let roll = init0
        .send_suite(Role::Initiator)
        .seal_roll(1, &next_on_init.commitment())
        .expect("pad");
    assert_eq!(frame_kind(&roll), Some(KIND_ROLL), "a roll cover");
    assert!(
        resp0.recv_suite(Role::Responder).open(&roll).is_none(),
        "a roll cover is NOT a data cover"
    );

    let (next_epoch, commitment) = resp0
        .recv_suite(Role::Responder)
        .open_roll(&roll)
        .expect("roll opens");
    assert_eq!(next_epoch, 1, "announced the next epoch");
    assert_eq!(
        commitment,
        next_on_resp.commitment(),
        "the receiver confirms it holds the identical next pad"
    );

    // Tamper ⇒ the handoff is rejected (no unauthenticated pad switch).
    let mut bad = roll.clone();
    bad[PAYLOAD_START] ^= 0x01;
    assert!(
        resp0.recv_suite(Role::Responder).open_roll(&bad).is_none(),
        "a tampered roll is rejected"
    );
}

#[test]
fn chain_rollover_end_to_end() {
    // Deplete epoch 0, hand off to epoch 1, keep sending — a seamless "next pad".
    let pad0 = random_pad(2 * (MAC_KEY_LEN + 16), 0xEE11_EE22_EE33_EE44); // one small cover per half
    let next_material = random_pad(1 << 12, 0x55AA_55AA_55AA_55AA);
    let s_init0 = PadPool::shared_unchecked(pad0.clone()).with_epoch(0);
    let s_resp0 = PadPool::shared_unchecked(pad0).with_epoch(0);
    let e1_init = PadPool::shared(next_material.clone())
        .unwrap()
        .with_epoch(1);
    let e1_resp = PadPool::shared(next_material).unwrap().with_epoch(1);

    let send0 = s_init0.send_suite(Role::Initiator);
    let recv0 = s_resp0.recv_suite(Role::Responder);

    let f = send0.seal(b"epoch0 payload!!").expect("first cover fits");
    assert_eq!(recv0.open(&f).as_deref(), Some(&b"epoch0 payload!!"[..]));
    assert!(send0.is_low(MAC_KEY_LEN + 1), "epoch 0 is now low");
    // Announce + hand off (authenticated by the last of epoch 0 is not possible here since it is
    // spent, so in practice the roll is sent while pad remains; we assert the commitments match).
    assert_eq!(
        e1_init.commitment(),
        e1_resp.commitment(),
        "both peers hold the identical epoch 1"
    );

    // Switch to epoch 1 and continue.
    let send1 = e1_init.send_suite(Role::Initiator);
    let recv1 = e1_resp.recv_suite(Role::Responder);
    let f1 = send1
        .seal(b"epoch1 continues after the roll")
        .expect("epoch 1 has pad");
    assert_eq!(
        recv1.open(&f1).as_deref(),
        Some(&b"epoch1 continues after the roll"[..])
    );
}

// ---- Wired into the live send/receive seam (the exact fns the interpreter + net_inbox call) ---

#[test]
fn wired_through_the_send_and_receive_seam() {
    use super::super::channel::{
        active_session, open_active, seal_active_checked, with_session, ActiveSession,
    };
    use std::rc::Rc;

    let material = random_pad(1 << 14, 0x5EA1_D00D_0FEE_1234);
    let initiator = PadPool::shared(material.clone()).expect("pad");
    let responder = PadPool::shared(material).expect("pad");
    let blob = b"through the very functions the interpreter Send/receive path calls".to_vec();

    // No session installed ⇒ byte-identical passthrough (today's default behavior, unchanged).
    assert!(active_session().is_none());
    assert_eq!(
        seal_active_checked(blob.clone()),
        Some(blob.clone()),
        "passthrough seal"
    );
    assert_eq!(
        open_active(blob.clone()),
        Some(blob.clone()),
        "passthrough open"
    );

    // Initiator installs its session; seal via the SAME fn the interpreter Send path calls.
    let send: Rc<dyn ActiveSession> = Rc::new(initiator.session(Role::Initiator));
    let frame =
        with_session(Some(send), || seal_active_checked(blob.clone())).expect("pad available");
    assert_eq!(frame_kind(&frame), Some(KIND_DATA), "a PNP data cover");
    assert_eq!(
        &frame[2..4],
        &SUITE_PNP.to_le_bytes(),
        "PNP suite id bound in the envelope"
    );
    assert_ne!(frame, blob, "sealed under the one-time pad, not plaintext");

    // Responder installs its session; open via the SAME fn net_inbox's receive path calls.
    let recv: Rc<dyn ActiveSession> = Rc::new(responder.session(Role::Responder));
    let opened = with_session(Some(recv), || open_active(frame.clone()));
    assert_eq!(
        opened.as_deref(),
        Some(blob.as_slice()),
        "opened through the wired receive seam"
    );
    assert!(
        active_session().is_none(),
        "session scope restored after with_session"
    );
}

#[test]
fn wired_open_active_drops_foreign_frame_under_session() {
    use super::super::channel::{open_active, with_session, ActiveSession};
    use std::rc::Rc;
    let pool = PadPool::shared(random_pad(4096, 0xF0E1_D2C3_B4A5_9687)).unwrap();
    let recv: Rc<dyn ActiveSession> = Rc::new(pool.session(Role::Responder));
    with_session(Some(recv), || {
        assert!(
            open_active(b"not a pnp frame at all".to_vec()).is_none(),
            "foreign frame dropped"
        );
    });
}

#[test]
fn wired_seal_active_fails_closed_on_exhaustion() {
    use super::super::channel::{seal_active_checked, with_session, ActiveSession};
    use std::rc::Rc;
    // The send half holds exactly one small cover; the next seal must fail closed, which the
    // interpreter Send path turns into a send error — never a plaintext leak.
    let pool = PadPool::shared_unchecked(random_pad(2 * (MAC_KEY_LEN + 8), 0x0B0E_0A0D_0C0F_0E0D));
    let send: Rc<dyn ActiveSession> = Rc::new(pool.session(Role::Initiator));
    with_session(Some(send), || {
        assert!(
            seal_active_checked(vec![0u8; 8]).is_some(),
            "first cover fits"
        );
        assert!(
            seal_active_checked(vec![0u8; 1]).is_none(),
            "exhausted ⇒ fail-closed None (no plaintext)"
        );
    });
}

// ---- Throughput (run with: cargo test --release -p logicaffeine-compile --lib pnp -- --ignored --nocapture)
#[test]
#[ignore = "benchmark: prints seal/open throughput; run in --release"]
fn throughput_seal_open() {
    use std::hint::black_box;
    use std::time::Instant;
    let half: usize = 64 << 20; // 64 MiB of pad per direction
    let pad = random_pad(2 * half, 0x7E57_7E57_7E57_7E57);
    let pool = PadPool::shared_unchecked(pad);

    // Component ceilings @1 MiB: where does the cost go — the XOR, or the authenticator?
    {
        let n = 1usize << 20;
        let x = random_pad(n, 0x1111_1111_1111_1111);
        let y = random_pad(n, 0x2222_2222_2222_2222);
        let reps = 200;
        let mut z = vec![0u8; n];
        let t = Instant::now();
        for _ in 0..reps {
            for ((o, a), b) in z.iter_mut().zip(&x).zip(&y) {
                *o = a ^ b;
            }
            black_box(&z);
        }
        let xor_gibs = (reps * n) as f64 / t.elapsed().as_secs_f64() / (1u64 << 30) as f64;
        let key = [7u8; 32];
        let t = Instant::now();
        for _ in 0..reps {
            black_box(poly1305(&key, &x));
        }
        let mac_gibs = (reps * n) as f64 / t.elapsed().as_secs_f64() / (1u64 << 30) as f64;
        eprintln!("components @1MiB: raw XOR {xor_gibs:.2} GiB/s | Poly1305 one-time MAC {mac_gibs:.2} GiB/s (this is the ceiling for authenticated OTP)");
    }

    eprintln!("PNP throughput (fused XOR + Poly1305 one-time MAC), 64 MiB pad/direction:");
    for &size in &[256usize, 4096, 65536, 1 << 20] {
        let msg = vec![0xABu8; size];
        let iters = (half / (size + MAC_KEY_LEN)).min(50_000).max(1);

        let sender = pool.send_suite(Role::Initiator);
        let mut frames: Vec<Vec<u8>> = Vec::with_capacity(iters);
        let t = Instant::now();
        for _ in 0..iters {
            frames.push(sender.seal(&msg).expect("pad"));
        }
        let seal_s = t.elapsed().as_secs_f64();

        let receiver = pool.recv_suite(Role::Responder);
        let t = Instant::now();
        let mut ok = 0usize;
        for f in &frames {
            if receiver.open(f).is_some() {
                ok += 1;
            }
        }
        let open_s = t.elapsed().as_secs_f64();
        assert_eq!(ok, iters, "all opened");

        let bytes = (iters * size) as f64;
        let seal_mbps = bytes / seal_s / (1 << 20) as f64;
        let open_mbps = bytes / open_s / (1 << 20) as f64;
        eprintln!(
            "  size {:>8}B  x{:>6}  seal {:>8.1} MiB/s ({:>6.0} ns/op)  open {:>8.1} MiB/s ({:>6.0} ns/op)",
            size,
            iters,
            seal_mbps,
            seal_s / iters as f64 * 1e9,
            open_mbps,
            open_s / iters as f64 * 1e9,
        );
    }
}

// ---- What an attacker and a crash can and cannot do --------------------------------------------

/// A `KIND_DATA` frame, `kind ‖ epoch ‖ offset ‖ len ‖ slot ‖ ct ‖ tag`, MAC'd under `key`.
fn forged_frame(epoch: u32, offset: u64, slot: u64, ct: &[u8], key: &[u8; 32]) -> Vec<u8> {
    let mut f = vec![CHAN_MAGIC, CHAN_VER];
    f.extend_from_slice(&SUITE_PNP.to_le_bytes());
    f.push(KIND_DATA);
    f.extend_from_slice(&epoch.to_le_bytes());
    f.extend_from_slice(&offset.to_le_bytes());
    f.extend_from_slice(&(ct.len() as u32).to_le_bytes());
    f.extend_from_slice(&slot.to_le_bytes());
    f.extend_from_slice(ct);
    let tag = poly1305(key, &f[CHAN_HEADER_LEN..]);
    f.extend_from_slice(&tag);
    f
}

#[test]
fn a_cover_keyed_by_keystream_an_earlier_cover_exposed_does_not_authenticate() {
    // The attacker sees a cover whose plaintext it knows (a protocol header, a status line), so it
    // knows that cover's keystream. It withholds the cover and sends its own, placed so the 32 pad
    // bytes the receiver keys the MAC with are keystream it knows, followed by keystream it knows.
    let pad = random_pad(4096, 0xA77A_C4E5_0000_0001);
    let pool = PadPool::shared(pad.clone()).expect("pad");
    let half = pool.direction_len();
    let sender = pool.send_suite(Role::Initiator);
    let known = vec![0x41u8; 200];
    let seen = sender.seal(&known).expect("pad");
    let keystream: Vec<u8> = ciphertext_of(&seen).iter().zip(&known).map(|(c, p)| c ^ p).collect();
    let start = frame_offset(&seen).unwrap();
    let wanted = b"pay mallory";
    let mut authenticated = Vec::new();
    for key_at in 0..=(keystream.len() - MAC_KEY_LEN - wanted.len()) {
        let key: [u8; 32] = keystream[key_at..key_at + MAC_KEY_LEN].try_into().unwrap();
        let stream_at = key_at + MAC_KEY_LEN;
        let ct: Vec<u8> = wanted
            .iter()
            .zip(&keystream[stream_at..])
            .map(|(p, k)| p ^ k)
            .collect();
        for slot in 0..4u64 {
            let frame = forged_frame(0, start + stream_at as u64, slot, &ct, &key);
            if pool.recv_suite(Role::Responder).open(&frame).is_some() {
                authenticated.push((key_at, slot));
            }
        }
    }
    assert!(
        authenticated.is_empty(),
        "a cover keyed by exposed keystream authenticated, at (keystream position, slot) {authenticated:?}"
    );
    // The forger's frames are well formed: the same construction under a key slot's own key opens.
    let slot = 1u64;
    let top = half - MAC_KEY_LEN * slot as usize;
    let key: [u8; 32] = pad[top - MAC_KEY_LEN..top].try_into().unwrap();
    let ct: Vec<u8> = wanted.iter().zip(&keystream).map(|(p, k)| p ^ k).collect();
    assert_eq!(
        pool.recv_suite(Role::Responder)
            .open(&forged_frame(0, start, slot, &ct, &key))
            .as_deref(),
        Some(&wanted[..]),
        "a frame of this construction keyed from its own slot opens"
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn two_ledgers_on_one_file_never_hand_out_one_byte_twice() {
    // Two processes holding one pad and one role share its ledger file and nothing else.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("shared.ledger");
    let pad = random_pad(1 << 14, 0x2222_1111_0000_FFFF);
    let a = PadPool::shared(pad.clone())
        .expect("pad")
        .send_suite(Role::Initiator)
        .with_ledger(Arc::new(FileLedger::new(&path)));
    let b = PadPool::shared(pad)
        .expect("pad")
        .send_suite(Role::Initiator)
        .with_ledger(Arc::new(FileLedger::new(&path)));
    let mut ranges = Vec::new();
    for i in 0..20u8 {
        let sender = if i % 2 == 0 { &a } else { &b };
        ranges.push(cover_range(&sender.seal(&[i; 24]).expect("pad")));
    }
    assert!(ranges_disjoint(&ranges), "two holders of one ledger drew one pad byte twice: {ranges:?}");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_ledger_that_cannot_be_read_closes_the_pad() {
    // A ledger it cannot read is not a fresh pad: starting again from the bottom would reuse
    // every byte the lost ledger had handed out.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("torn.ledger");
    std::fs::write(&path, [1u8, 2, 3]).expect("write a torn ledger");
    let sender = PadPool::shared(random_pad(4096, 0x7041_0000_1111_2222))
        .expect("pad")
        .send_suite(Role::Initiator)
        .with_ledger(Arc::new(FileLedger::new(&path)));
    assert_eq!(sender.seal(b"anything"), None, "a torn ledger closes the pad");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_cover_opened_before_a_restart_is_refused_after_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("recv.ledger");
    let pad = random_pad(4096, 0x0BE0_0BE0_1234_9876);
    let frame = PadPool::shared(pad.clone())
        .expect("pad")
        .send_suite(Role::Initiator)
        .seal(b"deliver once")
        .expect("pad");
    let receiver = || {
        PadPool::shared(pad.clone())
            .expect("pad")
            .recv_suite(Role::Responder)
            .with_ledger(Arc::new(FileLedger::new(&path)))
    };
    assert_eq!(receiver().open(&frame).as_deref(), Some(&b"deliver once"[..]));
    assert_eq!(receiver().open(&frame), None, "the receiver remembers it across a restart");
}
