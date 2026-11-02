//! QA safety locks — the "refuse to lie" invariants. A round-trip test proves the happy path; these
//! prove the DANGEROUS paths are safe: the FEC codec must REFUSE to fabricate a payload it cannot
//! recover (one packet past the MDS bound), reject an impossible code, and the cipher must genuinely
//! depend on EVERY input (key, nonce, counter) — a guard against a silent miscompile that quietly
//! ignores one and turns encryption into a predictable no-op. These are the guardrails that keep a
//! regression from shipping a codec that lies or a cipher that doesn't encrypt.

use logicaffeine_compile::compile::tw_outcome;

/// Deterministic splitmix64 — reproducible generation.
struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn upto(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
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

fn words(w: &[u32]) -> String {
    w.iter()
        .map(|x| format!("word32({x})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Assert the program aborts with a runtime error (a clean refusal, not a fabricated result).
fn assert_fails(program: &str, ctx: &str) {
    let r = tw_outcome(program);
    assert!(
        r.error.is_some(),
        "{ctx}: MUST refuse (error), but ran clean → {:?}\n---\n{program}",
        r.output
    );
}

/// Assert the two trailing `Show` lines DIFFER — the value genuinely changed with the input.
fn assert_lines_differ(program: &str, ctx: &str) {
    let r = tw_outcome(program);
    assert_eq!(
        r.error, None,
        "{ctx}: must run clean, got {:?}\n---\n{program}",
        r.error
    );
    let lines: Vec<&str> = r.output.trim().lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "{ctx}: expected two Show lines\n---\n{program}"
    );
    assert_ne!(
        lines[0], lines[1],
        "{ctx}: the two outputs must differ\n---\n{program}"
    );
}

/// Pick `count` distinct 1-based positions out of `n`.
fn pick_positions(rng: &mut R, n: usize, count: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (1..=n).collect();
    for i in (1..n).rev() {
        idx.swap(i, rng.upto(i + 1));
    }
    idx.truncate(count);
    idx
}

fn received_with_lost(lost: &[usize], n: usize) -> (String, String) {
    let mut binds = String::new();
    let mut slots = Vec::new();
    for idx in 1..=n {
        if lost.contains(&idx) {
            slots.push("nothing".to_string());
        } else {
            binds.push_str(&format!("Let p{idx} be item {idx} of packets.\n"));
            slots.push(format!("p{idx}"));
        }
    }
    (binds, slots.join(", "))
}

/// SAFETY INVARIANT (the MDS boundary): losing exactly `m` packets recovers, but losing `m + 1` is
/// one past the code's capacity — `fecRestore` MUST refuse (error), never fabricate a wrong payload.
/// A codec that silently returns garbage on unrecoverable loss is a data-corruption bug.
#[test]
fn fec_restore_refuses_to_fabricate_one_packet_past_the_mds_bound() {
    let mut rng = R(0xDEAD_BEEF_0BAD_F00D);
    for &len in &[1usize, 8, 32, 100] {
        for &k in &[2usize, 3, 4, 5] {
            for &m in &[1usize, 2, 3] {
                let n = k + m;
                let payload: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
                // Drop m+1 packets — strictly beyond what any k-of-n code can recover.
                let lost = pick_positions(&mut rng, n, m + 1);
                let (binds, slots) = received_with_lost(&lost, n);
                let program = format!(
                    "## Main\n\
                     Let payload be [{}].\n\
                     Let packets be fecProtect(payload, {k}, {m}).\n\
                     {binds}\
                     Let received be [{slots}].\n\
                     Let restored be fecRestore(received, {k}).\n\
                     Show restored.\n",
                    ints(&payload)
                );
                assert_fails(
                    &program,
                    &format!("fec over-loss len={len} k={k} m={m} lost={lost:?}"),
                );
            }
        }
    }
}

/// SAFETY INVARIANT: an impossible code (more shards than the GF(256) field can index) is rejected
/// up front, not silently truncated into a code that can't be decoded.
#[test]
fn fec_protect_rejects_a_code_that_overflows_the_field() {
    // k + m must be ≤ 256; 200 data + 100 parity = 300 is impossible over GF(256).
    let program = "## Main\n\
        Let payload be [1, 2, 3, 4].\n\
        Let packets be fecProtect(payload, 200, 100).\n\
        Show packets.\n";
    assert_fails(program, "fec impossible code 200+100");
}

/// SAFETY INVARIANT: the cipher genuinely depends on the KEY — two different keys produce different
/// ciphertext for the same plaintext. If they matched, the key would be ignored (a catastrophic
/// silent miscompile turning E2E encryption into a fixed, breakable transform).
#[test]
fn chacha20_ciphertext_depends_on_the_key() {
    let mut rng = R(0x0BADC0DE_5AFE_0001);
    for _ in 0..12 {
        let msg: Vec<u8> = (0..32).map(|_| rng.byte()).collect();
        let key_a: Vec<u32> = (0..8).map(|_| rng.next() as u32).collect();
        let mut key_b = key_a.clone();
        key_b[3] ^= 1; // differ in a single bit
        let nonce = "[word32(1), word32(2), word32(3)]";
        let program = format!(
            "## Main\n\
             Let msg be [{}].\n\
             Let a be encrypt([{}], {nonce}, msg, word32(0)).\n\
             Let b be encrypt([{}], {nonce}, msg, word32(0)).\n\
             Show a.\n\
             Show b.\n",
            ints(&msg),
            words(&key_a),
            words(&key_b)
        );
        assert_lines_differ(&program, "chacha20 key-sensitivity");
    }
}

/// SAFETY INVARIANT: the keystream depends on the NONCE — reusing a key with a fresh nonce yields a
/// different keystream (the whole point of a nonce; ignoring it would be a keystream-reuse disaster).
#[test]
fn chacha20_ciphertext_depends_on_the_nonce() {
    let mut rng = R(0x0BADC0DE_5AFE_0002);
    for _ in 0..12 {
        let msg: Vec<u8> = (0..32).map(|_| rng.byte()).collect();
        let key = words(&(0..8).map(|_| rng.next() as u32).collect::<Vec<_>>());
        let program = format!(
            "## Main\n\
             Let msg be [{}].\n\
             Let a be encrypt([{key}], [word32(1), word32(2), word32(3)], msg, word32(0)).\n\
             Let b be encrypt([{key}], [word32(1), word32(2), word32(4)], msg, word32(0)).\n\
             Show a.\n\
             Show b.\n",
            ints(&msg)
        );
        assert_lines_differ(&program, "chacha20 nonce-sensitivity");
    }
}

/// SAFETY INVARIANT: the keystream depends on the COUNTER — the block counter must advance the
/// keystream, or every block would be encrypted identically (another keystream-reuse disaster).
#[test]
fn chacha20_ciphertext_depends_on_the_counter() {
    let mut rng = R(0x0BADC0DE_5AFE_0003);
    for _ in 0..12 {
        let msg: Vec<u8> = (0..32).map(|_| rng.byte()).collect();
        let key = words(&(0..8).map(|_| rng.next() as u32).collect::<Vec<_>>());
        let nonce = "[word32(7), word32(8), word32(9)]";
        let program = format!(
            "## Main\n\
             Let msg be [{}].\n\
             Let a be encrypt([{key}], {nonce}, msg, word32(0)).\n\
             Let b be encrypt([{key}], {nonce}, msg, word32(1)).\n\
             Show a.\n\
             Show b.\n",
            ints(&msg)
        );
        assert_lines_differ(&program, "chacha20 counter-sensitivity");
    }
}

/// SAFETY INVARIANT: encryption is not accidentally the identity — ciphertext differs from plaintext
/// for a non-trivial message. Catches an all-zero keystream or a dropped XOR.
#[test]
fn chacha20_ciphertext_is_not_the_plaintext() {
    let mut rng = R(0x0BADC0DE_5AFE_0004);
    for _ in 0..12 {
        let msg: Vec<u8> = (0..32).map(|_| rng.byte()).collect();
        let key = words(&(0..8).map(|_| rng.next() as u32).collect::<Vec<_>>());
        let nonce = "[word32(5), word32(6), word32(7)]";
        let program = format!(
            "## Main\n\
             Let msg be [{}].\n\
             Let c be encrypt([{key}], {nonce}, msg, word32(0)).\n\
             Show msg.\n\
             Show c.\n",
            ints(&msg)
        );
        assert_lines_differ(&program, "chacha20 non-triviality");
    }
}
