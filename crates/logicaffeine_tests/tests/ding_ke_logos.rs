//! Ding Key Exchange written ENTIRELY in LOGOS — ring arithmetic, signal, and reconciliation all
//! in `assets/std/crypto.lg`, with no native kernel. Because nothing is native, the whole exchange
//! runs identically on the tree-walker, the register VM, and native (AOT) code; these tests hold it
//! to that byte-identical, cross-tier contract with NO external oracle.
//!
//! Jintai Ding, Xiang Xie, and Xiaodong Lin introduced RLWE key exchange with error reconciliation
//! (ePrint 2012/688); the NIST Round-1 submission fixes `R_q = Z_q[x]/(x^n + 1)`, `q = 120833`. The
//! signal `Sig` and reconciliation `Mod2` here are the DING512 semantics extracted verbatim from the
//! reference `crypto_kem_enc`/`crypto_kem_dec`. Every expected value comes from an independent
//! reimplementation in this file, so a bug in the LOGOS code cannot hide.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{run_all_tiers, run_logos_with_args};

const Q: i64 = 120833;

// ── An independent reference (distinct from the LOGOS code under test) ────────────────

fn ref_signal(v: i64, q: i64) -> i64 {
    if v < q / 4 || v > 3 * q / 4 {
        1
    } else {
        0
    }
}

fn ref_mod2(v: i64, w: i64, q: i64) -> i64 {
    (((v + w * ((q - 1) / 2)) % q) % 2 + 2) % 2
}

fn ref_negacyclic(a: &[i64], b: &[i64], q: i64) -> Vec<i64> {
    let n = a.len();
    let qi = q as i128;
    let mut acc = vec![0i128; n];
    for i in 0..n {
        for j in 0..n {
            let prod = a[i] as i128 * b[j] as i128;
            if i + j < n {
                acc[i + j] += prod;
            } else {
                acc[i + j - n] -= prod; // x^n ≡ −1
            }
        }
    }
    acc.iter().map(|&x| (((x % qi) + qi) % qi) as i64).collect()
}

fn ref_add_scaled(x: &[i64], e: &[i64], scale: i64, q: i64) -> Vec<i64> {
    x.iter()
        .zip(e)
        .map(|(&xi, &ei)| ((xi + scale * ei) % q + q) % q)
        .collect()
}

/// The independent reference for the two reconciled shared keys.
fn expected_shared_keys(
    a: &[i64],
    s1: &[i64],
    e1: &[i64],
    s2: &[i64],
    e2: &[i64],
) -> (Vec<i64>, Vec<i64>) {
    let p1 = ref_add_scaled(&ref_negacyclic(a, s1, Q), e1, 2, Q);
    let p2 = ref_add_scaled(&ref_negacyclic(a, s2, Q), e2, 2, Q);
    let k2 = ref_negacyclic(&p1, s2, Q); // responder
    let k1 = ref_negacyclic(&p2, s1, Q); // initiator
    let signal: Vec<i64> = k2.iter().map(|&v| ref_signal(v, Q)).collect();
    let ss_b: Vec<i64> = k2
        .iter()
        .map(|&v| ref_mod2(v, ref_signal(v, Q), Q))
        .collect();
    let ss_a: Vec<i64> = k1
        .iter()
        .zip(&signal)
        .map(|(&v, &w)| ref_mod2(v, w, Q))
        .collect();
    (ss_a, ss_b)
}

// ── Helpers ──────────────────────────────────────────────────────────────────────────

fn lit(v: &[i64]) -> String {
    v.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Run on the tree-walker AND the VM, assert they agree, and parse the `Show` output as ints.
fn tw_vm_ints(prog: &str) -> Vec<i64> {
    let agreed = run_all_tiers(prog);
    assert!(
        agreed.error.is_none(),
        "program errored: {}",
        agreed.error.unwrap_or_default()
    );
    parse_ints(&agreed.output)
}

/// Compile the full AOT way and parse the `Show` output as ints.
fn aot_ints(prog: &str) -> Vec<i64> {
    let aot = run_logos_with_args(prog, &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    parse_ints(&aot.stdout)
}

fn parse_ints(s: &str) -> Vec<i64> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse::<i64>().expect("int line"))
        .collect()
}

/// Deterministic splitmix64 — reproducible key-exchange inputs, no external RNG.
struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn uniform_q(&mut self) -> i64 {
        (self.next() % Q as u64) as i64
    }
    /// A ternary secret/error coefficient encoded in [0, q): −1 becomes q−1.
    fn ternary_encoded(&mut self) -> i64 {
        match self.next() % 3 {
            0 => 0,
            1 => 1,
            _ => Q - 1, // ≡ −1 (mod q)
        }
    }
}

fn kex_inputs(seed: u64, n: usize) -> (Vec<i64>, Vec<i64>, Vec<i64>, Vec<i64>, Vec<i64>) {
    let mut r = R(seed);
    let a: Vec<i64> = (0..n).map(|_| r.uniform_q()).collect();
    let s1: Vec<i64> = (0..n).map(|_| r.ternary_encoded()).collect();
    let e1: Vec<i64> = (0..n).map(|_| r.ternary_encoded()).collect();
    let s2: Vec<i64> = (0..n).map(|_| r.ternary_encoded()).collect();
    let e2: Vec<i64> = (0..n).map(|_| r.ternary_encoded()).collect();
    (a, s1, e1, s2, e2)
}

/// The end-to-end key exchange, orchestrated in LOGOS: both parties derive their shared key and the
/// program shows the responder's key then the initiator's. Uses only pure-LOGOS crypto functions.
fn kex_program(a: &[i64], s1: &[i64], e1: &[i64], s2: &[i64], e2: &[i64]) -> String {
    format!(
        "## Main\n\
         Let q be {q}.\n\
         Let a be [{a}].\n\
         Let s1 be [{s1}].\n\
         Let e1 be [{e1}].\n\
         Let s2 be [{s2}].\n\
         Let e2 be [{e2}].\n\
         Let p1 be dingPublicKey(a, s1, e1, q).\n\
         Let p2 be dingPublicKey(a, s2, e2, q).\n\
         Let k2 be dingNegacyclic(p1, s2, q).\n\
         Let k1 be dingNegacyclic(p2, s1, q).\n\
         Let signal be dingSignalBits(k2, q).\n\
         Let ssB be dingSharedKey(k2, signal, q).\n\
         Let ssA be dingSharedKey(k1, signal, q).\n\
         Repeat for i from 1 to length of ssB:\n    Show item i of ssB.\n\
         Repeat for i from 1 to length of ssA:\n    Show item i of ssA.\n",
        q = Q,
        a = lit(a),
        s1 = lit(s1),
        e1 = lit(e1),
        s2 = lit(s2),
        e2 = lit(e2),
    )
}

fn check_exchange_output(got: &[i64], want_a: &[i64], want_b: &[i64], where_: &str) {
    let n = want_a.len();
    let (got_b, got_a) = got.split_at(n);
    assert_eq!(
        got_b, want_b,
        "{where_}: responder key must match the reference"
    );
    assert_eq!(
        got_a, want_a,
        "{where_}: initiator key must match the reference"
    );
    assert_eq!(
        got_a, got_b,
        "{where_}: the two parties must reconcile to one key"
    );
}

// ── The scheme's own agreement property (independent reference, no LOGOS) ──────────────

#[test]
fn reference_key_exchange_always_reconciles() {
    // With ternary secrets/errors the reconciliation error stays far below the tolerance δ = q/4−2,
    // so both parties MUST derive the identical key, for every seed and both ring sizes we test.
    for &n in &[64usize, 512] {
        for trial in 0..30u64 {
            let (a, s1, e1, s2, e2) = kex_inputs(0x0D16_0000 ^ (n as u64) ^ (trial << 8), n);
            let (a_key, b_key) = expected_shared_keys(&a, &s1, &e1, &s2, &e2);
            assert_eq!(
                a_key, b_key,
                "n={n} trial={trial}: initiator and responder must agree"
            );
        }
    }
}

// ── The exact reconciliation tolerance (Ding–Xie–Lin Lemma 3: δ = q/4 − 2; exact threshold q/4) ──

#[test]
fn ding_reconciliation_is_correct_up_to_the_exact_error_tolerance() {
    // The load-bearing correctness property. Given the responder's signal w = Sig(k2), both parties
    // recover the same key bit iff their key coefficients differ by an EVEN amount (the difference is
    // 2·(e₂s₁ − e₁s₂)) that does not push the reconciliation value across the ring boundary.
    // Ding–Xie–Lin Lemma 3 gives the SUFFICIENT bound δ = q/4 − 2; here we pin the EXACT threshold and
    // prove it EXHAUSTIVELY over the whole ring — no sampling: reconciliation holds for every even
    // shift up to q/4, the first even shift that can disagree is q/4 + 2, and Ding's δ is therefore
    // conservative by exactly 2.
    let q = Q;
    assert_eq!(q % 2, 1, "the parity-flips-on-wrap argument requires q odd");
    let quarter = q / 4; // 30208 — the exact tight tolerance, proven below
    let delta = quarter - 2; // 30206 — Ding–Xie–Lin Lemma 3's sufficient bound, inside q/4

    // (0) WHY the tolerance exists — the whole purpose of Ding's signal: it places the reconciliation
    //     value u = (k2 + w·(q−1)/2) mod q into the safe middle band [q/4, 3q/4] for EVERY k2, i.e. at
    //     least q/4 from either ring boundary.
    for k2 in 0..q {
        let w = ref_signal(k2, q);
        let u = (k2 + w * ((q - 1) / 2)) % q;
        assert!(
            quarter <= u && u <= 3 * q / 4,
            "k2={k2}: reconciliation value {u} left the safe band [q/4, 3q/4]"
        );
    }

    // (1) EXACT tolerance, EXHAUSTIVE: for every k2, even shifts of magnitude δ (Lemma 3) AND q/4 (the
    //     tight bound) in either direction reconcile — and hence so does every smaller even shift,
    //     since with u in [q/4, 3q/4] a magnitude ≤ q/4 never wraps and (q odd) a non-wrapping even
    //     shift preserves parity.
    for k2 in 0..q {
        let w = ref_signal(k2, q);
        let key_b = ref_mod2(k2, w, q);
        for m in [delta, quarter] {
            assert_eq!(
                ref_mod2((k2 + m) % q, w, q),
                key_b,
                "k2={k2}: even shift +{m} (≤ q/4) must reconcile"
            );
            assert_eq!(
                ref_mod2((k2 - m + q) % q, w, q),
                key_b,
                "k2={k2}: even shift −{m} (≤ q/4) must reconcile"
            );
        }
    }

    // (2) The threshold is SHARP: the first even magnitude beyond q/4, namely q/4 + 2, disagrees for a
    //     band-edge key (u = 3q/4). Pinned deterministically — so this proves not merely "some
    //     disagreement exists somewhere past δ" but EXACTLY where agreement stops.
    let k2 = 3 * q / 4; // 90624; Sig = 0, so u = k2 sits exactly at the upper band edge
    let w = ref_signal(k2, q);
    assert_eq!(w, 0, "the band-edge witness k2 = 3q/4 must have signal 0");
    assert_eq!(
        ref_mod2((k2 + quarter) % q, w, q),
        ref_mod2(k2, w, q),
        "+q/4 at the edge still agrees"
    );
    assert_ne!(
        ref_mod2((k2 + quarter + 2) % q, w, q),
        ref_mod2(k2, w, q),
        "+q/4+2 at the edge must disagree"
    );
}

#[test]
fn ding_exchange_noise_stays_strictly_within_the_reconciliation_tolerance() {
    // WHY the exchange always agrees, proven rather than observed: with ternary secrets/errors the
    // two parties' key polynomials differ by 2(e₂s₁ − e₁s₂), and EVERY coefficient's centered
    // distance on the ring stays strictly below δ = q/4 − 2 — so, by the tolerance theorem above,
    // reconciliation is guaranteed to agree.
    let n = 512;
    let delta = Q / 4 - 2;
    let mut rng = R(0x00B0_D6E7_0000_0001);
    let mut worst = 0i64;
    for _ in 0..25 {
        let a: Vec<i64> = (0..n).map(|_| rng.uniform_q()).collect();
        let s1: Vec<i64> = (0..n).map(|_| rng.ternary_encoded()).collect();
        let e1: Vec<i64> = (0..n).map(|_| rng.ternary_encoded()).collect();
        let s2: Vec<i64> = (0..n).map(|_| rng.ternary_encoded()).collect();
        let e2: Vec<i64> = (0..n).map(|_| rng.ternary_encoded()).collect();
        let p1 = ref_add_scaled(&ref_negacyclic(&a, &s1, Q), &e1, 2, Q);
        let p2 = ref_add_scaled(&ref_negacyclic(&a, &s2, Q), &e2, 2, Q);
        let k2 = ref_negacyclic(&p1, &s2, Q);
        let k1 = ref_negacyclic(&p2, &s1, Q);
        for i in 0..n {
            let d = (k1[i] - k2[i]).rem_euclid(Q);
            let centered = d.min(Q - d);
            worst = worst.max(centered);
            assert!(
                centered <= delta,
                "exchange noise {centered} exceeded δ={delta} — agreement not guaranteed"
            );
        }
    }
    assert!(
        worst > 0 && worst < delta,
        "sanity: real noise exists ({worst}) and stays well under δ={delta}"
    );
}

#[test]
fn logos_ding_signal_and_mod2_match_the_reference_over_the_entire_ring_across_tiers() {
    // The Ding contribution itself — the signal function and the Mod2 reconciliation — proven in
    // LOGOS over the ENTIRE ring Z_q, for BOTH boundary bits and BOTH signal values, via a polynomial
    // hash equal to the reference's. Any single residue where LOGOS disagrees changes the hash.
    const MOD: i64 = 1_000_000_007;
    let q = Q;
    let (mut hs0, mut hs1, mut hm0, mut hm1) = (0i64, 0i64, 0i64, 0i64);
    for v in 0..q {
        let s0 = if v < q / 4 || v > 3 * q / 4 { 1 } else { 0 };
        let s1 = if v < q / 4 + 1 || v > 3 * q / 4 + 1 {
            1
        } else {
            0
        };
        hs0 = (hs0 * 257 + s0) % MOD;
        hs1 = (hs1 * 257 + s1) % MOD;
        hm0 = (hm0 * 257 + ref_mod2(v, 0, q)) % MOD;
        hm1 = (hm1 * 257 + ref_mod2(v, 1, q)) % MOD;
    }
    let prog = format!(
        "## Main\nLet q be {q}.\n\
         Let mutable hs0 be 0.\nLet mutable hs1 be 0.\nLet mutable hm0 be 0.\nLet mutable hm1 be 0.\n\
         Repeat for v from 0 to {qm1}:\n\
         \u{20}   Set hs0 to ((hs0 * 257 + dingSignal(v, q, 0)) % 1000000007).\n\
         \u{20}   Set hs1 to ((hs1 * 257 + dingSignal(v, q, 1)) % 1000000007).\n\
         \u{20}   Set hm0 to ((hm0 * 257 + dingMod2(v, 0, q)) % 1000000007).\n\
         \u{20}   Set hm1 to ((hm1 * 257 + dingMod2(v, 1, q)) % 1000000007).\n\
         Show hs0.\nShow hs1.\nShow hm0.\nShow hm1.\n",
        q = q,
        qm1 = q - 1,
    );
    assert_eq!(
        tw_vm_ints(&prog),
        vec![hs0, hs1, hm0, hm1],
        "LOGOS dingSignal/dingMod2 must match the reference over ALL of Z_q (both boundaries, both signals)"
    );
}

// ── RED-1: signal + reconciliation match the reference on every tier ───────────────────

#[test]
fn ding_signal_and_mod2_match_the_reference_across_tiers() {
    let lb = Q / 4;
    let ub = 3 * Q / 4;
    let hq = Q / 2;
    let vs: Vec<i64> = vec![
        0,
        1,
        2,
        lb - 1,
        lb,
        lb + 1,
        hq - 1,
        hq,
        hq + 1,
        ub - 1,
        ub,
        ub + 1,
        Q - 2,
        Q - 1,
    ];
    let prog = format!(
        "## Main\n\
         Let q be {q}.\n\
         Let vs be [{vs}].\n\
         Repeat for i from 1 to length of vs:\n    Show dingSignal(item i of vs, q, 0).\n\
         Repeat for i from 1 to length of vs:\n    Show dingMod2(item i of vs, 0, q).\n\
         Repeat for i from 1 to length of vs:\n    Show dingMod2(item i of vs, 1, q).\n",
        q = Q,
        vs = lit(&vs),
    );

    let got = tw_vm_ints(&prog);
    let mut want: Vec<i64> = Vec::new();
    want.extend(vs.iter().map(|&v| ref_signal(v, Q)));
    want.extend(vs.iter().map(|&v| ref_mod2(v, 0, Q)));
    want.extend(vs.iter().map(|&v| ref_mod2(v, 1, Q)));
    assert_eq!(
        got, want,
        "LOGOS dingSignal/dingMod2 must equal the reference (tw == vm)"
    );
}

// ── RED-1 (cont.): the ring/orchestration helpers match the reference on every tier ────

#[test]
fn ding_ring_and_orchestration_helpers_match_the_reference_across_tiers() {
    // Small ring so the tree-walker and VM run fast; exercises dingNegacyclic, dingAddScaled,
    // dingSignalBits, and dingSharedKey directly.
    let n = 24;
    let (a, s1, _e1, s2, _e2) = kex_inputs(0x0D16_5EED, n);
    let x = a.clone();
    let e = s1.clone(); // reuse as a small {0,1,q−1} vector

    let prog = format!(
        "## Main\n\
         Let q be {q}.\n\
         Let a be [{a}].\n\
         Let s be [{s}].\n\
         Let prod be dingNegacyclic(a, s, q).\n\
         Repeat for i from 1 to length of prod:\n    Show item i of prod.\n\
         Let x be [{x}].\n\
         Let e be [{e}].\n\
         Let scaled be dingAddScaled(x, e, 2, q).\n\
         Repeat for i from 1 to length of scaled:\n    Show item i of scaled.\n\
         Let signal be dingSignalBits(prod, q).\n\
         Repeat for i from 1 to length of signal:\n    Show item i of signal.\n\
         Let sk be dingSharedKey(prod, signal, q).\n\
         Repeat for i from 1 to length of sk:\n    Show item i of sk.\n",
        q = Q,
        a = lit(&a),
        s = lit(&s2),
        x = lit(&x),
        e = lit(&e),
    );

    let prod = ref_negacyclic(&a, &s2, Q);
    let signal: Vec<i64> = prod.iter().map(|&v| ref_signal(v, Q)).collect();
    let mut want: Vec<i64> = Vec::new();
    want.extend(prod.iter().copied());
    want.extend(ref_add_scaled(&x, &e, 2, Q));
    want.extend(signal.iter().copied());
    want.extend(prod.iter().zip(&signal).map(|(&v, &w)| ref_mod2(v, w, Q)));

    let got = tw_vm_ints(&prog);
    assert_eq!(
        got, want,
        "LOGOS ring + orchestration helpers must equal the reference (tw == vm)"
    );
}

// ── RED-2: the full exchange reconciles on the tree-walker AND the VM (fast, default) ──

#[test]
fn ding_key_exchange_reconciles_on_tree_walker_and_vm() {
    // A small ring keeps the interpreted O(n²) product fast; the reconciliation margin is huge, so
    // agreement is guaranteed. Runs the FULL exchange — including the ring product — in LOGOS.
    let n = 64;
    for seed in [0x0D16_2001u64, 0x0D16_2002] {
        let (a, s1, e1, s2, e2) = kex_inputs(seed, n);
        let (want_a, want_b) = expected_shared_keys(&a, &s1, &e1, &s2, &e2);
        assert_eq!(want_a, want_b, "reference must agree (seed {seed:#x})");
        let got = tw_vm_ints(&kex_program(&a, &s1, &e1, &s2, &e2));
        check_exchange_output(&got, &want_a, &want_b, "tw+vm");
    }
}

// ── RED-2 (full dimension): the DING512 exchange reconciles on EVERY tier ──────────────

#[test]
#[ignore = "full-dimension exchange on tw + vm + AOT (the AOT tier compiles a cargo project) — slow"]
fn ding_key_exchange_full_dimension_reconciles_on_every_tier() {
    let n = 512;
    let (a, s1, e1, s2, e2) = kex_inputs(0x0D16_0512, n);
    let (want_a, want_b) = expected_shared_keys(&a, &s1, &e1, &s2, &e2);
    assert_eq!(want_a, want_b, "reference must agree at n=512");
    let prog = kex_program(&a, &s1, &e1, &s2, &e2);

    // Tree-walker + VM (in-process differential).
    check_exchange_output(&tw_vm_ints(&prog), &want_a, &want_b, "tw+vm (n=512)");
    // Native AOT.
    check_exchange_output(&aot_ints(&prog), &want_a, &want_b, "AOT (n=512)");
}
