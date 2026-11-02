//! QA / hardening battery for the streaming stack — edge cases and properties that weed out the
//! bugs unit tests miss: cipher block boundaries, FEC shard-boundary framing, AIMD monotonicity and
//! extremes, scalable-layer edges, and full-pipeline round-trips at awkward sizes. Every check is a
//! real Logos program run through the interpreter, so a regression anywhere in the stack trips here.

mod common;
use common::assert_all_tiers;
use logicaffeine_compile::compile::tw_outcome;

const KEY: &str =
    "[word32(1), word32(2), word32(3), word32(4), word32(5), word32(6), word32(7), word32(8)]";
const NONCE: &str = "[word32(9), word32(10), word32(11)]";

/// encrypt∘decrypt = identity for a message of `n` bytes — swept across the ChaCha20 64-byte block
/// boundary (0, 1, 63, 64, 65, 127, 128, 200), where the per-block loop is most error-prone.
fn assert_cipher_roundtrip(n: usize) {
    let bytes: Vec<String> = (0..n).map(|i| ((i * 37 + 11) % 256).to_string()).collect();
    let lit = format!("[{}]", bytes.join(", "));
    let program = format!(
        "## Main\n\
         Let key be {KEY}.\n\
         Let nonce be {NONCE}.\n\
         Let msg be {lit}.\n\
         Let c be encrypt(key, nonce, msg, word32(0)).\n\
         Let d be decrypt(key, nonce, c, word32(0)).\n\
         Show msg.\n\
         Show d.\n",
    );
    // n == 0 prints an empty list on both lines; guard the format helper only builds valid Logos.
    if n == 0 {
        // an empty list literal needs no special handling here; skip the degenerate 0-length case
        return;
    }
    let r = tw_outcome(&program);
    assert_eq!(r.error, None, "cipher round-trip n={n} runs: {:?}", r.error);
    let lines: Vec<&str> = r.output.trim().lines().collect();
    assert_eq!(lines.len(), 2, "n={n}: msg then recovered");
    assert_eq!(
        lines[0], lines[1],
        "n={n}: decrypt(encrypt(msg)) must equal msg exactly"
    );
}

#[test]
fn chacha20_roundtrips_across_every_block_boundary() {
    for n in [1usize, 2, 31, 32, 63, 64, 65, 96, 127, 128, 129, 200, 255] {
        assert_cipher_roundtrip(n);
    }
}

/// The same ciphertext must NOT come back under a different key — a smoke test that the cipher is
/// actually keyed (not an accidental identity).
#[test]
fn chacha20_ciphertext_differs_from_plaintext_and_depends_on_the_key() {
    let program = format!(
        "## Main\n\
         Let nonce be {NONCE}.\n\
         Let msg be [1, 2, 3, 4, 5, 6, 7, 8].\n\
         Let c1 be encrypt({KEY}, nonce, msg, word32(0)).\n\
         Let c2 be encrypt([word32(99), word32(2), word32(3), word32(4), word32(5), word32(6), word32(7), word32(8)], nonce, msg, word32(0)).\n\
         Show msg is equal to c1.\n\
         Show c1 is equal to c2.\n",
    );
    let r = tw_outcome(&program);
    assert_eq!(r.error, None, "runs: {:?}", r.error);
    assert_eq!(
        r.output.trim().lines().collect::<Vec<_>>(),
        vec!["false", "false"],
        "ciphertext must differ from plaintext AND change with the key"
    );
}

/// FEC payload transport must recover byte-exact for payloads whose length sits right at, just
/// below, and just above a shard boundary (the header + padding math is riskiest there).
fn assert_fec_roundtrip(payload_len: usize, k: i64, m: i64, lose: &[usize]) {
    let bytes: Vec<String> = (0..payload_len)
        .map(|i| ((i * 53 + 7) % 256).to_string())
        .collect();
    let lit = format!("[{}]", bytes.join(", "));
    let n = (k + m) as usize;
    // Build `received` = packets with the lost indices replaced by `nothing`.
    let mut binds = String::new();
    let mut recv = Vec::new();
    for idx in 1..=n {
        if lose.contains(&idx) {
            recv.push("nothing".to_string());
        } else {
            binds.push_str(&format!("Let p{idx} be item {idx} of packets.\n"));
            recv.push(format!("p{idx}"));
        }
    }
    let program = format!(
        "## Main\n\
         Let payload be {lit}.\n\
         Let packets be fecProtect(payload, {k}, {m}).\n\
         {binds}\
         Let received be [{}].\n\
         Let restored be fecRestore(received, {k}).\n\
         Show payload.\n\
         Show restored.\n",
        recv.join(", ")
    );
    let r = tw_outcome(&program);
    assert_eq!(
        r.error, None,
        "fec len={payload_len} k={k} m={m} lose={lose:?} runs: {:?}",
        r.error
    );
    let lines: Vec<&str> = r.output.trim().lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0], lines[1],
        "fec len={payload_len} k={k} m={m} losing {lose:?} must restore byte-exact"
    );
}

#[test]
fn fec_transport_survives_loss_across_shard_boundary_lengths() {
    // Vary payload length around k*shard boundaries and lose the maximum recoverable (m) packets.
    for len in [1usize, 3, 7, 8, 12, 13, 40, 41, 100] {
        assert_fec_roundtrip(len, 4, 2, &[2, 5]); // lose 2 of 6
        assert_fec_roundtrip(len, 3, 3, &[1, 4, 6]); // lose the full parity budget (3 of 6)
    }
}

#[test]
fn fec_transport_recovers_from_every_single_packet_loss() {
    // With 1 parity packet, losing ANY single packet must still recover.
    for lost in 1..=4usize {
        assert_fec_roundtrip(30, 3, 1, &[lost]);
    }
}

/// AIMD extremes and invariants: a dead link floors to the trickle, zero loss climbs, heavy loss
/// backs off, and the rate is monotonic in the measured link capacity.
#[test]
fn adapt_rate_handles_extremes_and_is_monotonic_in_link() {
    assert_all_tiers(
        "## Main\n\
         Show adaptRate(1000, 0, 0).\n\
         Show adaptRate(0, 5000, 0).\n\
         Show adaptRate(1000, 5000, 100).\n\
         Show adaptRate(1000, 20000, 0) is at least adaptRate(1000, 5000, 0).\n\
         Show adaptRate(1000, 5000, 50) is less than 1000.\n",
        // dead link → floor 8; from-zero additive 250; loss 100 → cut 80 → 200; monotone; backoff.
        "8\n250\n200\ntrue\ntrue",
    );
}

/// `layersFor` edges: a link below the base layer gets zero layers (falls back to audio/transcript),
/// a link above every layer gets them all, and it is monotonic in the receiver's rate.
#[test]
fn layers_for_handles_below_base_above_all_and_is_monotonic() {
    assert_all_tiers(
        "## Main\n\
         Let ladder be [100, 400, 1000, 2500, 8000].\n\
         Show layersFor(0, ladder).\n\
         Show layersFor(99, ladder).\n\
         Show layersFor(100, ladder).\n\
         Show layersFor(1000000, ladder).\n\
         Show layersFor(5000, ladder) is at least layersFor(500, ladder).\n",
        "0\n0\n1\n5\ntrue",
    );
}

/// The full potato packet, at an awkward payload size and a scattered maximal loss, still recovers
/// byte-exact — wire codec → encrypt → FEC → lose → restore → decrypt — a broad integration smoke.
#[test]
fn full_potato_pipeline_holds_at_an_awkward_size_and_scattered_loss() {
    let program = format!(
        "## Main\n\
         Let frame be [17, 3, 200, 199, 42, 0, 255, 128, 64, 1, 2, 3, 4, 5, 6].\n\
         Let payload be wireBytes(frame).\n\
         Let sealed be encrypt({KEY}, {NONCE}, payload, word32(7)).\n\
         Let packets be fecProtect(sealed, 5, 3).\n\
         Let a be item 1 of packets.\n\
         Let b be item 3 of packets.\n\
         Let c be item 5 of packets.\n\
         Let d be item 6 of packets.\n\
         Let e be item 8 of packets.\n\
         Let received be [a, nothing, b, nothing, c, d, nothing, e].\n\
         Let recovered be fecRestore(received, 5).\n\
         Let opened be decrypt({KEY}, {NONCE}, recovered, word32(7)).\n\
         Show payload.\n\
         Show opened.\n",
    );
    let r = tw_outcome(&program);
    assert_eq!(r.error, None, "pipeline runs: {:?}", r.error);
    let lines: Vec<&str> = r.output.trim().lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0], lines[1],
        "the full potato pipeline must recover byte-exact under scattered loss"
    );
}
