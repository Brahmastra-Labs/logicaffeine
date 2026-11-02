//! QA — the streaming codec knobs, end to end. The streaming format's job is to be SMALL: the wire
//! codec frames a structured value compactly, and the compressor squeezes redundant state (CRDT
//! deltas, rosters, pose keypoints) further. For an E2E-encrypted stream the squeeze has to happen
//! BEFORE encryption (you cannot compress ciphertext), so `compressBytes`/`decompressBytes` are
//! byte-level knobs — the codec's existing deflate/lz4/zstd (smallest-kept, self-describing), not
//! reinvented. These lock: round-trip identity, that compression actually shrinks redundant data,
//! that it is SAFE (never corrupts) on incompressible/already-encoded data, and that the whole
//! compress→encrypt→FEC→lose→restore→decrypt→decompress→unwire pipeline is byte-exact.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

fn ints(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Run a program ending in two `Show` lines and assert they are byte-identical, on BOTH tiers.
fn assert_roundtrip_all_tiers(program: &str, ctx: &str) {
    for (tier, r) in [("tw", tw_outcome(program)), ("vm", vm_outcome(program))] {
        assert_eq!(
            r.error, None,
            "{ctx} [{tier}]: must run clean, got {:?}\n{program}",
            r.error
        );
        let lines: Vec<&str> = r.output.trim().lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "{ctx} [{tier}]: expected two lines\n{program}"
        );
        assert_eq!(
            lines[0], lines[1],
            "{ctx} [{tier}]: round-trip must be byte-exact\n{program}"
        );
    }
}

/// INVARIANT: `compress ∘ decompress` is the identity on arbitrary bytes — the non-negotiable
/// contract before it can carry any stream.
#[test]
fn compress_then_decompress_is_the_identity() {
    let mut rng = R(0xC0DEC_5EED_0001);
    for &len in &[0usize, 1, 7, 64, 200, 1000] {
        // A mix of a redundant run (compressible) and random bytes (incompressible) in one payload.
        let mut payload: Vec<u8> = (0..len / 2).map(|i| (i % 4) as u8).collect();
        payload.extend((0..len - len / 2).map(|_| rng.byte()));
        let program = format!(
            "## Main\n\
             Let frame be [{}].\n\
             Let packed be compressBytes(frame).\n\
             Let restored be decompressBytes(packed).\n\
             Show frame.\n\
             Show restored.\n",
            ints(&payload)
        );
        assert_roundtrip_all_tiers(&program, &format!("compress round-trip len={len}"));
    }
}

/// THE STREAMING WIN: redundant structured state compresses SMALLER than its raw framing — this is
/// what makes CRDT-delta / roster / keypoint streams cheap on a potato link. A 256-byte repeating
/// frame must pack to well under its raw size.
#[test]
fn compression_shrinks_redundant_state() {
    let redundant: Vec<u8> = (0..256).map(|i| (i % 4) as u8).collect(); // heavily repetitive
    let program = format!(
        "## Main\n\
         Let frame be [{}].\n\
         Let packed be compressBytes(frame).\n\
         Let packedLen be length of packed.\n\
         Let rawLen be length of frame.\n\
         If packedLen is less than rawLen:\n\
         \x20   Show \"smaller\".\n\
         Otherwise:\n\
         \x20   Show \"not-smaller\".\n",
        ints(&redundant)
    );
    for (tier, r) in [("tw", tw_outcome(&program)), ("vm", vm_outcome(&program))] {
        assert_eq!(r.error, None, "[{tier}] runs clean: {:?}", r.error);
        assert_eq!(
            r.output.trim(),
            "smaller",
            "[{tier}] redundant state must compress smaller\n{program}"
        );
    }
}

/// SAFETY: compressing already-encrypted / already-encoded (incompressible) data must NEVER corrupt
/// it and must never balloon it — at worst a one-byte codec tag (the "stored" fallback). This is the
/// knob discipline: media chunks are pre-compressed, so `compress` on them is a safe no-op-ish pass.
#[test]
fn compression_is_safe_and_bounded_on_incompressible_data() {
    let mut rng = R(0xC0DEC_5EED_0002);
    let random: Vec<u8> = (0..256).map(|_| rng.byte()).collect();
    // round-trips byte-exact...
    let rt = format!(
        "## Main\n\
         Let frame be [{}].\n\
         Let restored be decompressBytes(compressBytes(frame)).\n\
         Show frame.\n\
         Show restored.\n",
        ints(&random)
    );
    assert_roundtrip_all_tiers(&rt, "compress incompressible round-trip");
    // ...and never grows by more than the single self-describing tag byte.
    let bound = format!(
        "## Main\n\
         Let frame be [{}].\n\
         Let rawLen be length of frame.\n\
         Let packedLen be length of compressBytes(frame).\n\
         Let ceiling be rawLen + 1.\n\
         If packedLen is at most ceiling:\n\
         \x20   Show \"bounded\".\n\
         Otherwise:\n\
         \x20   Show \"ballooned\".\n",
        ints(&random)
    );
    for (tier, r) in [("tw", tw_outcome(&bound)), ("vm", vm_outcome(&bound))] {
        assert_eq!(r.error, None, "[{tier}] runs clean: {:?}", r.error);
        assert_eq!(
            r.output.trim(),
            "bounded",
            "[{tier}] compress must not balloon incompressible data\n{bound}"
        );
    }
}

/// SAFETY: a malformed / unknown-codec compressed frame is rejected with a clean error — never a
/// panic and never a decompression bomb (the bound is inherited from the codec). Network-edge safety
/// a receiver needs when a peer sends garbage.
#[test]
fn decompress_rejects_a_malformed_frame_cleanly() {
    // Byte 0 is the codec tag; 200 is not a known codec, so this must error rather than fabricate.
    let program = "## Main\n\
        Let junk be [200, 1, 2, 3, 4].\n\
        Let out be decompressBytes(junk).\n\
        Show out.\n";
    for (tier, r) in [("tw", tw_outcome(program)), ("vm", vm_outcome(program))] {
        assert!(
            r.error.is_some(),
            "[{tier}]: a malformed compressed frame must be refused, not silently decoded → {:?}",
            r.output
        );
    }
}

/// THE FULL STREAMING FORMAT, correct knob ORDER: a structured frame is codec-framed, COMPRESSED,
/// encrypted, FEC-protected, loses a packet, is restored, decrypted, decompressed, and unframed —
/// byte-exact. Compress-before-encrypt is the whole reason the byte-level knob exists.
#[test]
fn full_streaming_pipeline_compress_before_encrypt_is_byte_exact() {
    let key =
        "[word32(1), word32(2), word32(3), word32(4), word32(5), word32(6), word32(7), word32(8)]";
    let nonce = "[word32(9), word32(10), word32(11)]";
    // A redundant structured frame (the kind CRDT deltas / rosters produce), k=3 m=2, drop packet 2.
    let frame: Vec<u8> = (0..48).map(|i| (i % 3) as u8).collect();
    let program = format!(
        "## Main\n\
         Let frame be [{}].\n\
         Let payload be wireBytes(frame).\n\
         Let squeezed be compressBytes(payload).\n\
         Let sealed be encrypt({key}, {nonce}, squeezed, word32(0)).\n\
         Let packets be fecProtect(sealed, 3, 2).\n\
         Let p1 be item 1 of packets.\n\
         Let p3 be item 3 of packets.\n\
         Let p4 be item 4 of packets.\n\
         Let p5 be item 5 of packets.\n\
         Let received be [p1, nothing, p3, p4, p5].\n\
         Let recovered be fecRestore(received, 3).\n\
         Let opened be decrypt({key}, {nonce}, recovered, word32(0)).\n\
         Let unpacked be decompressBytes(opened).\n\
         Show payload.\n\
         Show unpacked.\n",
        ints(&frame)
    );
    assert_roundtrip_all_tiers(&program, "full compress-before-encrypt pipeline");
}

// =============================================================================
// SEALED FRAMES — the E2E streaming primitive: `sealBytes` authenticated-encrypts the plaintext
// (ChaCha20-Poly1305) in one step; `openBytes` verifies the tag and decrypts. Nothing is compressed
// first, so a frame's length gives away nothing of its plaintext but the plaintext's length.
// =============================================================================

fn key_lit() -> String {
    ints(
        &(0..32u32)
            .map(|i| (i.wrapping_mul(7).wrapping_add(1)) as u8)
            .collect::<Vec<_>>(),
    )
}
fn nonce_lit() -> String {
    ints(
        &(0..12u32)
            .map(|i| (i.wrapping_mul(5).wrapping_add(3)) as u8)
            .collect::<Vec<_>>(),
    )
}

/// INVARIANT: `sealBytes` then `openBytes` is the identity — the sealed frame decrypts back to
/// exactly the plaintext, on both tiers, for redundant AND random payloads.
#[test]
fn seal_then_open_is_the_identity() {
    let mut rng = R(0x5EA1_ED_0001);
    for &len in &[1usize, 16, 100, 512] {
        let mut payload: Vec<u8> = (0..len / 2).map(|i| (i % 5) as u8).collect();
        payload.extend((0..len - len / 2).map(|_| rng.byte()));
        let program = format!(
            "## Main\n\
             Let key be [{}].\n\
             Let nonce be [{}].\n\
             Let plaintext be [{}].\n\
             Let sealed be sealBytes(key, nonce, plaintext).\n\
             Let opened be openBytes(key, nonce, sealed).\n\
             Show plaintext.\n\
             Show opened.\n",
            key_lit(),
            nonce_lit(),
            ints(&payload)
        );
        assert_roundtrip_all_tiers(&program, &format!("seal/open round-trip len={len}"));
    }
}

/// AUTHENTICATION: `openBytes` REFUSES a frame that fails the Poly1305 tag check — a wrong key or a
/// truncated frame is rejected with an error, never silently decrypted to garbage. This is what a
/// raw stream cipher cannot do; it's why the sealed frame is the secure primitive.
#[test]
fn open_rejects_a_wrong_key_and_a_truncated_frame() {
    // Wrong key: seal with `key`, open with a different key → tag mismatch → error.
    let wrong = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let badKey be [{}].\n\
         Let sealed be sealBytes(key, nonce, [1, 2, 3, 4, 5]).\n\
         Let opened be openBytes(badKey, nonce, sealed).\n\
         Show opened.\n",
        key_lit(),
        nonce_lit(),
        ints(
            &(0..32u32)
                .map(|i| (i.wrapping_mul(3)) as u8)
                .collect::<Vec<_>>()
        )
    );
    for (tier, r) in [("tw", tw_outcome(&wrong)), ("vm", vm_outcome(&wrong))] {
        assert!(
            r.error.is_some(),
            "[{tier}]: a wrong key must fail authentication → {:?}",
            r.output
        );
    }
    // Truncated: a frame shorter than the 16-byte tag can't authenticate → error.
    let trunc = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let opened be openBytes(key, nonce, [1, 2, 3]).\n\
         Show opened.\n",
        key_lit(),
        nonce_lit()
    );
    for (tier, r) in [("tw", tw_outcome(&trunc)), ("vm", vm_outcome(&trunc))] {
        assert!(
            r.error.is_some(),
            "[{tier}]: a truncated frame must be refused → {:?}",
            r.output
        );
    }
}

/// A sealed frame is exactly its plaintext and the 16-byte tag: a redundant plaintext and a random one
/// of the same length seal to the same length. Compressing before encrypting would make the frame's
/// length tell how compressible the plaintext was, and a frame that carries a secret beside anything an
/// attacker chooses would give the secret away a guess at a time (CRIME, BREACH).
#[test]
fn a_sealed_frame_is_its_plaintext_and_a_tag() {
    let mut rng = R(0x5EA1_ED_0002);
    let redundant: Vec<u8> = (0..512).map(|i| (i % 4) as u8).collect();
    let random: Vec<u8> = (0..512).map(|_| rng.byte()).collect();
    let program = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Show length of sealBytes(key, nonce, [{}]).\n\
         Show length of sealBytes(key, nonce, [{}]).\n",
        key_lit(),
        nonce_lit(),
        ints(&redundant),
        ints(&random)
    );
    for (tier, r) in [("tw", tw_outcome(&program)), ("vm", vm_outcome(&program))] {
        assert_eq!(r.error, None, "[{tier}] runs clean: {:?}", r.error);
        assert_eq!(
            r.output.trim(),
            "528\n528",
            "[{tier}] each sealed frame is its 512-byte plaintext plus the 16-byte tag\n{program}"
        );
    }
}

/// THE STREAMING FRAME, end to end: a structured frame is codec-framed, SEALED (authenticated-encrypted
/// in one call), FEC-protected, loses a packet on the potato link, is restored, and OPENED (verified +
/// decrypted) — byte-exact.
#[test]
fn full_secure_streaming_frame_survives_loss_byte_exact() {
    let frame: Vec<u8> = (0..60).map(|i| (i % 3) as u8).collect();
    let program = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let frame be [{}].\n\
         Let payload be wireBytes(frame).\n\
         Let sealed be sealBytes(key, nonce, payload).\n\
         Let packets be fecProtect(sealed, 3, 2).\n\
         Let p1 be item 1 of packets.\n\
         Let p2 be item 2 of packets.\n\
         Let p4 be item 4 of packets.\n\
         Let p5 be item 5 of packets.\n\
         Let received be [p1, p2, nothing, p4, p5].\n\
         Let recovered be fecRestore(received, 3).\n\
         Let opened be openBytes(key, nonce, recovered).\n\
         Show payload.\n\
         Show opened.\n",
        key_lit(),
        nonce_lit(),
        ints(&frame)
    );
    assert_roundtrip_all_tiers(&program, "full secure streaming frame with loss");
}
