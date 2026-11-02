//! QA invariant locks — property-based tests that pin the pit-of-success invariants across the
//! whole streaming stack over MANY generated inputs. A hand-picked case proves a feature works
//! once; these prove the INVARIANT holds — FEC round-trips are identity, the cipher round-trips,
//! the full pipeline recovers byte-exact, and the AIMD controller never leaves its bounds — for a
//! broad matrix of sizes, keys, parameters, and loss patterns. A regression anywhere trips them.

use logicaffeine_compile::compile::tw_outcome;

/// Deterministic splitmix64 — reproducible generation, no external RNG.
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

/// Run a program that ends in `Show a.` `Show b.` and assert the two lines are byte-identical.
fn assert_roundtrip(program: &str, ctx: &str) {
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
    assert_eq!(
        lines[0], lines[1],
        "{ctx}: round-trip must be byte-exact\n---\n{program}"
    );
}

/// Pick `m` distinct 1-based positions out of `n` to drop (Fisher-Yates).
fn lost_positions(rng: &mut R, n: usize, m: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (1..=n).collect();
    for i in (1..n).rev() {
        idx.swap(i, rng.upto(i + 1));
    }
    idx.truncate(m);
    idx
}

/// Build a `received` packet list (`nothing` for the lost indices) plus the binding lines.
fn received(lost: &[usize], n: usize) -> (String, String) {
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

/// INVARIANT: `fecProtect ∘ fecRestore` is the identity on the payload for EVERY valid (payload, k,
/// m) after losing ANY m packets — swept over a broad matrix of lengths, codes, and loss patterns.
#[test]
fn fec_transport_round_trip_is_identity_over_a_broad_matrix() {
    let mut rng = R(0x00C0_FFEE_1234_5678);
    for &len in &[1usize, 2, 7, 8, 15, 16, 64, 100, 255] {
        for &k in &[2usize, 3, 4, 5] {
            for &m in &[1usize, 2, 3] {
                let n = k + m;
                let payload: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
                let lost = lost_positions(&mut rng, n, m);
                let (binds, slots) = received(&lost, n);
                let program = format!(
                    "## Main\n\
                     Let payload be [{}].\n\
                     Let packets be fecProtect(payload, {k}, {m}).\n\
                     {binds}\
                     Let received be [{slots}].\n\
                     Let restored be fecRestore(received, {k}).\n\
                     Show payload.\n\
                     Show restored.\n",
                    ints(&payload)
                );
                assert_roundtrip(
                    &program,
                    &format!("fec len={len} k={k} m={m} lost={lost:?}"),
                );
            }
        }
    }
}

/// INVARIANT: `encrypt ∘ decrypt` is the identity for random keys, nonces, counters, and message
/// lengths across the ChaCha20 block boundary.
#[test]
fn chacha20_round_trip_is_identity_over_random_keys_and_lengths() {
    let mut rng = R(0x5EED_1234_ABCD_9999);
    for &len in &[1usize, 5, 63, 64, 65, 127, 128, 200] {
        for _ in 0..4 {
            let key: Vec<u32> = (0..8).map(|_| rng.next() as u32).collect();
            let nonce: Vec<u32> = (0..3).map(|_| rng.next() as u32).collect();
            let msg: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
            let ctr = rng.next() as u32;
            let words = |w: &[u32]| {
                w.iter()
                    .map(|x| format!("word32({x})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let program = format!(
                "## Main\n\
                 Let key be [{}].\n\
                 Let nonce be [{}].\n\
                 Let msg be [{}].\n\
                 Let c be encrypt(key, nonce, msg, word32({ctr})).\n\
                 Let d be decrypt(key, nonce, c, word32({ctr})).\n\
                 Show msg.\n\
                 Show d.\n",
                words(&key),
                words(&nonce),
                ints(&msg)
            );
            assert_roundtrip(&program, &format!("chacha20 len={len}"));
        }
    }
}

/// INVARIANT: the FULL potato pipeline — wire-codec compact → encrypt → FEC-protect → lose m of n
/// → restore → decrypt — recovers the exact compact payload, for random frames, codes, and losses.
#[test]
fn full_potato_pipeline_is_identity_over_random_frames_and_losses() {
    let mut rng = R(0x9077_A70_5EED_0001);
    let key =
        "[word32(1), word32(2), word32(3), word32(4), word32(5), word32(6), word32(7), word32(8)]";
    let nonce = "[word32(9), word32(10), word32(11)]";
    for _ in 0..24 {
        let frame_len = 1 + rng.upto(24);
        let frame: Vec<u8> = (0..frame_len).map(|_| rng.byte()).collect();
        let k = 2 + rng.upto(4); // 2..5
        let m = 1 + rng.upto(3); // 1..3
        let n = k + m;
        let ctr = rng.next() as u32;
        let lost = lost_positions(&mut rng, n, m);
        let (binds, slots) = received(&lost, n);
        let program = format!(
            "## Main\n\
             Let frame be [{}].\n\
             Let payload be wireBytes(frame).\n\
             Let sealed be encrypt({key}, {nonce}, payload, word32({ctr})).\n\
             Let packets be fecProtect(sealed, {k}, {m}).\n\
             {binds}\
             Let received be [{slots}].\n\
             Let recovered be fecRestore(received, {k}).\n\
             Let opened be decrypt({key}, {nonce}, recovered, word32({ctr})).\n\
             Show payload.\n\
             Show opened.\n",
            ints(&frame)
        );
        assert_roundtrip(
            &program,
            &format!("pipeline frame_len={frame_len} k={k} m={m} lost={lost:?}"),
        );
    }
}

/// INVARIANT: the AIMD controller never leaves its bounds — the rate is always at least the 8 kbps
/// trickle floor and never exceeds 90% of the measured link — for random conditions.
#[test]
fn adapt_rate_never_leaves_its_bounds_over_random_conditions() {
    let mut rng = R(0xA1_3DAB_0B0_1234);
    let mut program = String::from("## Main\n");
    let mut expected: Vec<String> = Vec::new();
    for _ in 0..50 {
        let link = 100 + rng.upto(50_000);
        let current = 8 + rng.upto(link);
        let loss = rng.upto(101);
        let cap = (link * 9) / 10;
        program.push_str(&format!(
            "Show adaptRate({current}, {link}, {loss}) is at least 8.\n"
        ));
        program.push_str(&format!(
            "Show adaptRate({current}, {link}, {loss}) is less than {}.\n",
            cap + 1
        ));
        expected.push("true".into());
        expected.push("true".into());
    }
    let r = tw_outcome(&program);
    assert_eq!(
        r.error, None,
        "adaptRate bounds program runs clean: {:?}",
        r.error
    );
    let lines: Vec<&str> = r.output.trim().lines().collect();
    assert_eq!(
        lines, expected,
        "adaptRate must always stay within [8, 0.9*link]"
    );
}

/// INVARIANT: scalable layers are MONOTONE — a receiver with more bandwidth never gets FEWER layers
/// than one with less, for any ladder and any pair of rates.
#[test]
fn scalable_layers_are_monotone_in_rate() {
    let mut rng = R(0x1A_4E12_5EED_7777);
    let mut program = String::from("## Main\nLet ladder be [80, 250, 700, 1800, 4500, 12000].\n");
    let mut expected: Vec<String> = Vec::new();
    for _ in 0..40 {
        let lo = rng.upto(15000);
        let hi = lo + rng.upto(15000);
        program.push_str(&format!(
            "Show layersFor({hi}, ladder) is at least layersFor({lo}, ladder).\n"
        ));
        expected.push("true".into());
    }
    let r = tw_outcome(&program);
    assert_eq!(
        r.error, None,
        "layers monotonicity program runs clean: {:?}",
        r.error
    );
    let lines: Vec<&str> = r.output.trim().lines().collect();
    assert_eq!(
        lines, expected,
        "layersFor must be monotone non-decreasing in rate"
    );
}
