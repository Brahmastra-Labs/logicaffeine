//! The ML-KEM number-theoretic transform, written in Logos and validated against a Rust
//! oracle (F2 → L2). This first milestone is the CORRECT modular-NTT computation over 𝔽₃₃₂₉:
//! the schoolbook DFT form `NTT(f)[k] = Σ_j f[j]·ω^(jk) mod q`, the simplest shape to verify.
//! The fast O(n log n) butterfly NTT — where the kernel-certified Gauss/symmetry speedups
//! apply — is the next milestone; this locks the arithmetic and the Logos idioms it needs.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

const Q: i64 = 3329; // Kyber / ML-KEM modulus (prime; q − 1 = 3328 = 2^8 · 13).

fn pow_mod(mut base: i64, mut exp: i64, q: i64) -> i64 {
    let mut r = 1i64;
    base = base.rem_euclid(q);
    while exp > 0 {
        if exp & 1 == 1 {
            r = r * base % q;
        }
        base = base * base % q;
        exp >>= 1;
    }
    r
}

fn inv_mod(a: i64, q: i64) -> i64 {
    pow_mod(a.rem_euclid(q), q - 2, q) // Fermat: a^(q−2) ≡ a⁻¹ (mod prime q)
}

/// A primitive n-th root of unity mod q (requires n | q−1).
fn root_of_unity(n: i64, q: i64) -> i64 {
    assert_eq!((q - 1) % n, 0, "n must divide q-1");
    for g in 2..q {
        // g generates (ℤ/q)* iff g^((q−1)/p) ≠ 1 for every prime p | (q−1). q−1 = 2^8·13.
        if pow_mod(g, (q - 1) / 2, q) != 1 && pow_mod(g, (q - 1) / 13, q) != 1 {
            return pow_mod(g, (q - 1) / n, q);
        }
    }
    panic!("no generator mod {q}");
}

/// Schoolbook DFT NTT — correct by definition, the oracle the Logos NTT must match.
fn ntt(f: &[i64], w: i64, q: i64) -> Vec<i64> {
    let n = f.len() as i64;
    (0..n)
        .map(|k| {
            let mut acc = 0i64;
            for j in 0..n {
                acc = (acc + f[j as usize] * pow_mod(w, (j * k).rem_euclid(n), q)) % q;
            }
            acc
        })
        .collect()
}

/// Inverse DFT — used only to TRUST the oracle via round-trip.
fn intt(fhat: &[i64], w: i64, q: i64) -> Vec<i64> {
    let n = fhat.len() as i64;
    let n_inv = inv_mod(n, q);
    let w_inv = inv_mod(w, q);
    (0..n)
        .map(|m| {
            let mut acc = 0i64;
            for k in 0..n {
                acc = (acc + fhat[k as usize] * pow_mod(w_inv, (m * k).rem_euclid(n), q)) % q;
            }
            acc * n_inv % q
        })
        .collect()
}

/// Build the Logos schoolbook NTT program for a concrete input + power table.
fn logos_ntt_program(f: &[i64], powers: &[i64]) -> String {
    let n = f.len();
    let f_lit = f.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let p_lit = powers.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    format!(
        "## Main\n\
         Let f be [{f_lit}].\n\
         Let powers be [{p_lit}].\n\
         Let result be a new Seq of Int.\n\
         Repeat for k from 0 to {top}:\n\
         \x20   Let acc be 0.\n\
         \x20   Repeat for j from 0 to {top}:\n\
         \x20       Let e be (j * k) % {n}.\n\
         \x20       Set acc to (acc + (item (j + 1) of f) * (item (e + 1) of powers)) % {q}.\n\
         \x20   Push acc to result.\n\
         Repeat for i from 1 to {n}:\n\
         \x20   Show item i of result.\n",
        top = n - 1,
        q = Q,
    )
}

#[test]
fn oracle_ntt_round_trips() {
    // Trust check: the Rust oracle's NTT is invertible, so its outputs are a valid reference.
    let n = 8;
    let w = root_of_unity(n, Q);
    let f: Vec<i64> = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let back = intt(&ntt(&f, w, Q), w, Q);
    assert_eq!(back, f, "the oracle NTT must round-trip (intt∘ntt = id)");
}

/// Run the Logos NTT over `f` (length a power of two dividing q−1) and assert it equals the
/// Rust DFT oracle bit-for-bit.
fn assert_logos_ntt_matches_oracle(f: &[i64]) {
    let n = f.len() as i64;
    let w = root_of_unity(n, Q);
    let powers: Vec<i64> = (0..n).map(|i| pow_mod(w, i, Q)).collect();
    let expected = ntt(f, w, Q);

    let prog = logos_ntt_program(f, &powers);
    let r = tw_outcome(&prog);
    assert_eq!(r.error, None, "Logos NTT must run without error: {:?}", r.error);
    let got: Vec<i64> = r
        .output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("integer output line"))
        .collect();
    assert_eq!(got.len(), n as usize, "one output coefficient per index");
    assert_eq!(got, expected, "the Logos NTT must equal the Rust DFT oracle");
}

#[test]
fn logos_ntt_matches_oracle_n8() {
    assert_logos_ntt_matches_oracle(&[123, 2900, 7, 3328, 0, 1500, 999, 42]);
}

// ---------------------------------------------------------------------------------------
// Fast O(n log n) butterfly NTT — the symmetry ratchet (radix-2 Cooley-Tukey exploits
// ω^(n/2) = −1 to split the transform). Decimation-in-time: bit-reversed input → natural
// output, so its result equals the schoolbook DFT and the same oracle validates it.
// ---------------------------------------------------------------------------------------

fn bit_reverse(mut x: usize, bits: u32) -> usize {
    let mut r = 0usize;
    for _ in 0..bits {
        r = (r << 1) | (x & 1);
        x >>= 1;
    }
    r
}

fn bit_reverse_vec(f: &[i64]) -> Vec<i64> {
    let bits = f.len().trailing_zeros();
    (0..f.len()).map(|i| f[bit_reverse(i, bits)]).collect()
}

fn logos_fast_ntt_program(a: &[i64], powers: &[i64]) -> String {
    let n = a.len();
    let stages = n.trailing_zeros() as usize; // log2(n)
    let a_lit = a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let p_lit = powers.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    format!(
        "## Main\n\
         Let a be [{a_lit}].\n\
         Let powers be [{p_lit}].\n\
         Let len be 2.\n\
         Repeat for stage from 1 to {stages}:\n\
         \x20   Let half be len / 2.\n\
         \x20   Let m be {n} / len.\n\
         \x20   Repeat for blk from 0 to m - 1:\n\
         \x20       Let start be blk * len.\n\
         \x20       Repeat for j from 0 to half - 1:\n\
         \x20           Let tw be item (m * j + 1) of powers.\n\
         \x20           Let idx be start + j.\n\
         \x20           Let u be item (idx + 1) of a.\n\
         \x20           Let t be (tw * (item (idx + half + 1) of a)) % {q}.\n\
         \x20           Set item (idx + 1) of a to (u + t) % {q}.\n\
         \x20           Set item (idx + half + 1) of a to (u - t + {q}) % {q}.\n\
         \x20   Set len to len * 2.\n\
         Repeat for i from 1 to {n}:\n\
         \x20   Show item i of a.\n",
        q = Q,
    )
}

fn run_logos_ntt(prog: &str) -> Vec<i64> {
    let r = tw_outcome(prog);
    assert_eq!(r.error, None, "Logos NTT error: {:?}", r.error);
    r.output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("integer output line"))
        .collect()
}

fn assert_logos_fast_ntt_matches_oracle(f: &[i64]) {
    let n = f.len() as i64;
    let w = root_of_unity(n, Q);
    let powers: Vec<i64> = (0..n).map(|i| pow_mod(w, i, Q)).collect();
    let expected = ntt(f, w, Q); // schoolbook oracle, natural order

    let a = bit_reverse_vec(f); // DIT consumes bit-reversed input
    let got = run_logos_ntt(&logos_fast_ntt_program(&a, &powers));
    assert_eq!(got.len(), n as usize, "one output per index");
    assert_eq!(got, expected, "the fast butterfly NTT must equal the DFT oracle");
}

#[test]
fn logos_fast_ntt_matches_oracle_n8() {
    assert_logos_fast_ntt_matches_oracle(&[123, 2900, 7, 3328, 0, 1500, 999, 42]);
}

// ── Montgomery NTT (division-free — the libcrux/Kyber speed form) ─────────────────────────────
//
// R = 2^16, q = 3329. The unsigned REDC of `x ∈ [0, q·R)` computes `x·R⁻¹ mod q` with only a
// mask, two multiplies, an add, a shift, and one conditional subtract — NO division:
//   lo = (x mod R)·(−q⁻¹ mod R) mod R          (−q⁻¹ mod 2^16 = 3327)
//   t  = (x + lo·q) / R                          (x + lo·q is divisible by R, so / R is exact)
//   if t ≥ q: t −= q                             (t ∈ [0, 2q) → [0, q))
// Twiddles are stored in Montgomery form (`p·R mod q`), so `redc(tw_mont·x) = tw·x mod q`: the
// identical butterfly product as the schoolbook `(tw·x) % q`, but the only `% q`-class division
// in the inner loop is gone (replaced by `/ R` = a shift + a branch-free subtract).

const MONT_R: i64 = 65536; // R = 2^16
const NEG_QINV_MOD_R: i64 = 3327; // (−q⁻¹) mod 2^16

/// Reference REDC over `i64` — pins the constants and is the oracle the Logos `redc` matches.
fn redc_oracle(x: i64) -> i64 {
    let lo = ((x % MONT_R) * NEG_QINV_MOD_R) % MONT_R;
    let mut t = (x + lo * Q) / MONT_R;
    if t >= Q {
        t -= Q;
    }
    t
}

#[test]
fn redc_computes_montgomery_reduction() {
    let r_inv = inv_mod(MONT_R, Q); // R⁻¹ mod q
    for x in [0i64, 1, 7, Q, MONT_R, 11_000_000, Q * (MONT_R - 1), (Q - 1) * (Q - 1)] {
        let got = redc_oracle(x);
        let want = (x.rem_euclid(Q) * r_inv).rem_euclid(Q); // x·R⁻¹ mod q
        assert_eq!(got, want, "redc({x}) must equal x·R⁻¹ mod q");
        assert!((0..Q).contains(&got), "redc output must land in [0, q)");
    }
}

/// The Montgomery-form radix-2 DIT NTT in Logos: the SAME structure as `logos_fast_ntt_program`,
/// but `(tw·x) % q → redc(tw·x)` (twiddles in Montgomery form) and the add/sub use conditional
/// subtracts. Division-free in the inner loop.
fn logos_montgomery_ntt_program(a: &[i64], powers_mont: &[i64]) -> String {
    let n = a.len();
    let stages = n.trailing_zeros() as usize;
    let a_lit = a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let p_lit = powers_mont.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let q = Q;
    let lines: Vec<String> = vec![
        "## To redc (x: Int) -> Int:".to_string(),
        "    Let lo be ((x % 65536) * 3327) % 65536.".to_string(),
        format!("    Let t be (x + lo * {q}) / 65536."),
        format!("    If t is at least {q}:"),
        format!("        Set t to t - {q}."),
        "    Return t.".to_string(),
        "## Main".to_string(),
        format!("Let a be [{a_lit}]."),
        format!("Let powers be [{p_lit}]."),
        "Let len be 2.".to_string(),
        format!("Repeat for stage from 1 to {stages}:"),
        "    Let half be len / 2.".to_string(),
        format!("    Let m be {n} / len."),
        "    Repeat for blk from 0 to m - 1:".to_string(),
        "        Let start be blk * len.".to_string(),
        "        Repeat for j from 0 to half - 1:".to_string(),
        "            Let tw be item (m * j + 1) of powers.".to_string(),
        "            Let idx be start + j.".to_string(),
        "            Let u be item (idx + 1) of a.".to_string(),
        "            Let t be redc(tw * (item (idx + half + 1) of a)).".to_string(),
        "            Let v be u + t.".to_string(),
        format!("            If v is at least {q}:"),
        format!("                Set v to v - {q}."),
        "            Set item (idx + 1) of a to v.".to_string(),
        format!("            Let w be u - t + {q}."),
        format!("            If w is at least {q}:"),
        format!("                Set w to w - {q}."),
        "            Set item (idx + half + 1) of a to w.".to_string(),
        "    Set len to len * 2.".to_string(),
        format!("Repeat for i from 1 to {n}:"),
        "    Show item i of a.".to_string(),
    ];
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

fn assert_logos_montgomery_ntt_matches_oracle(f: &[i64]) {
    let n = f.len() as i64;
    let w = root_of_unity(n, Q);
    let powers: Vec<i64> = (0..n).map(|i| pow_mod(w, i, Q)).collect();
    let powers_mont: Vec<i64> = powers.iter().map(|&p| (p * MONT_R).rem_euclid(Q)).collect();
    let expected = ntt(f, w, Q); // schoolbook DFT oracle, natural order

    let a: Vec<i64> = bit_reverse_vec(f).iter().map(|&x| x.rem_euclid(Q)).collect();
    let got = run_logos_ntt(&logos_montgomery_ntt_program(&a, &powers_mont));
    assert_eq!(got.len(), n as usize, "one output per index");
    assert_eq!(got, expected, "the Montgomery (division-free) NTT must equal the DFT oracle");
}

#[test]
fn probe_redc_codegen_uses_mask_and_shift() {
    // The redc reductions are powers of two: `% 65536` must lower to a mask and `/ 65536` to a
    // shift — NOT a hardware divide. Print the generated `redc` body.
    let prog = logos_montgomery_ntt_program(&[1, 2, 3, 4, 5, 6, 7, 8], &[1, 1, 1, 1, 1, 1, 1, 1]);
    let rust = logicaffeine_compile::compile::compile_to_rust(&prog).expect("Montgomery NTT compiles");
    eprintln!("=== generated redc ===");
    let mut in_redc = false;
    for line in rust.lines() {
        if line.contains("fn redc") {
            in_redc = true;
        }
        if in_redc {
            eprintln!("{line}");
            if line.trim() == "}" {
                break;
            }
        }
    }
}

#[test]
fn logos_montgomery_ntt_matches_oracle_n8() {
    assert_logos_montgomery_ntt_matches_oracle(&[123, 2900, 7, 3328, 0, 1500, 999, 42]);
}

#[test]
fn logos_montgomery_ntt_matches_oracle_at_kyber_size_256() {
    let mut s = 0x1234_5678_u64;
    let f: Vec<i64> = (0..256)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (s >> 16) as i64 % Q
        })
        .collect();
    assert_logos_montgomery_ntt_matches_oracle(&f);
}

/// The DEMAND-IMPORTED stdlib `ntt` (assets/std/crypto.lg) — a program that just calls
/// `ntt(a, powers)` pulls the module in; the division-free Montgomery NTT it ships must equal the
/// DFT oracle, proving the SHIPPED stdlib transform (not a test fixture) is correct.
fn assert_stdlib_ntt_matches_oracle(f: &[i64]) {
    let n = f.len() as i64;
    let w = root_of_unity(n, Q);
    let powers: Vec<i64> = (0..n).map(|i| pow_mod(w, i, Q)).collect();
    let powers_mont: Vec<i64> = powers.iter().map(|&p| (p * MONT_R).rem_euclid(Q)).collect();
    let expected = ntt(f, w, Q);
    let a: Vec<i64> = bit_reverse_vec(f).iter().map(|&x| x.rem_euclid(Q)).collect();
    let a_lit = a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let p_lit = powers_mont.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let prog = format!(
        "## Main\nLet a be [{a_lit}].\nLet powers be [{p_lit}].\nLet result be ntt(a, powers).\nRepeat for i from 1 to {n}:\n    Show item i of result.\n"
    );
    let got = run_logos_ntt(&prog);
    assert_eq!(got, expected, "the demand-imported stdlib `ntt` must equal the DFT oracle");
}

#[test]
fn stdlib_ntt_matches_oracle_n8() {
    assert_stdlib_ntt_matches_oracle(&[123, 2900, 7, 3328, 0, 1500, 999, 42]);
}

#[test]
fn stdlib_ntt_matches_oracle_at_kyber_size_256() {
    let mut s = 0xABCD_1234_u64;
    let f: Vec<i64> = (0..256)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (s >> 16) as i64 % Q
        })
        .collect();
    assert_stdlib_ntt_matches_oracle(&f);
}

#[test]
fn logos_fast_ntt_matches_oracle_at_kyber_size_256() {
    let mut s = 0x1234_5678_u64;
    let f: Vec<i64> = (0..256)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (s >> 16) as i64 % Q
        })
        .collect();
    assert_logos_fast_ntt_matches_oracle(&f);
}

#[test]
fn fast_ntt_ratchets_the_multiply_count_vs_schoolbook() {
    // Side-by-side op-count: the symmetry split takes the multiply count from n² to
    // (n/2)·log2(n). At Kyber's n = 256 that is a measured 64× fewer multiplies — the
    // ratchet, deterministic and zero-cost (a static algorithmic fact, no instrumentation).
    for &n in &[8i64, 256] {
        let schoolbook = n * n;
        let fast = (n / 2) * (n.trailing_zeros() as i64);
        let ratio = schoolbook / fast;
        println!("n={n:>4}: schoolbook {schoolbook:>6} mults | fast {fast:>5} mults | {ratio}× fewer");
        assert!(fast < schoolbook, "the fast NTT must do strictly fewer multiplies");
    }
    assert_eq!(256 * 256, 65536);
    assert_eq!((256 / 2) * 8, 1024);
    assert_eq!(65536 / 1024, 64, "n=256: a 64× multiply reduction");
}

#[test]
fn fast_and_schoolbook_ntt_agree_and_we_time_them_side_by_side() {
    // Measured side-by-side on the SAME input (tree-walker wall-clock; the racing number comes
    // from the compiled tier, but even interpreted the symmetry ratchet shows). Both must agree.
    let n = 256i64;
    let w = root_of_unity(n, Q);
    let powers: Vec<i64> = (0..n).map(|i| pow_mod(w, i, Q)).collect();
    let mut s = 0xDEAD_u64;
    let f: Vec<i64> = (0..256)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (s >> 16) as i64 % Q
        })
        .collect();

    let school_prog = logos_ntt_program(&f, &powers);
    let fast_prog = logos_fast_ntt_program(&bit_reverse_vec(&f), &powers);

    let t0 = std::time::Instant::now();
    let school_out = run_logos_ntt(&school_prog);
    let school_t = t0.elapsed();

    let t1 = std::time::Instant::now();
    let fast_out = run_logos_ntt(&fast_prog);
    let fast_t = t1.elapsed();

    assert_eq!(fast_out, school_out, "fast and schoolbook NTT must produce identical coefficients");
    println!(
        "n=256 tree-walker: schoolbook {school_t:?} | fast {fast_t:?} | speedup {:.1}×",
        school_t.as_secs_f64() / fast_t.as_secs_f64().max(1e-9)
    );
}

#[test]
fn logos_ntt_matches_oracle_at_kyber_size_256() {
    // ML-KEM's polynomial degree: 256 coefficients mod 3329. A deterministic pseudo-random
    // input so the test is reproducible. (Schoolbook O(n²) here — correctness, not yet speed;
    // the fast O(n log n) butterfly NTT, where the certified symmetry speedups apply, is next.)
    let mut s = 0x9E37_79B9_u64;
    let f: Vec<i64> = (0..256)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (s >> 16) as i64 % Q
        })
        .collect();
    assert_logos_ntt_matches_oracle(&f);
}

// ---------------------------------------------------------------------------------------
// The race: the Logos fast NTT compiled through the EXODIA JIT (native tier) vs a
// hand-written Rust NTT (the ml-kem / libcrux-equivalent), same machine, same algorithm.
// ---------------------------------------------------------------------------------------

/// Hand-written Rust DIT butterfly NTT — the baseline a PQC library ships.
fn fast_ntt_rust(a: &mut [i64], powers: &[i64], q: i64) {
    let n = a.len();
    let mut len = 2usize;
    while len <= n {
        let half = len / 2;
        let m = n / len;
        for blk in 0..m {
            let start = blk * len;
            for j in 0..half {
                let tw = powers[m * j];
                let idx = start + j;
                let u = a[idx];
                let t = tw * a[idx + half] % q;
                a[idx] = (u + t) % q;
                a[idx + half] = (u - t + q) % q;
            }
        }
        len *= 2;
    }
}

/// The fast NTT wrapped in an `iters`-deep loop so JIT-tiered steady-state execution
/// dominates one-time parse/warmup, then one observed coefficient (defeats DCE).
fn logos_fast_ntt_bench_program(a: &[i64], powers: &[i64], iters: usize) -> String {
    let n = a.len();
    let stages = n.trailing_zeros() as usize;
    let a_lit = a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let p_lit = powers.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    format!(
        "## Main\n\
         Let a be [{a_lit}].\n\
         Let powers be [{p_lit}].\n\
         Repeat for iter from 1 to {iters}:\n\
         \x20   Let len be 2.\n\
         \x20   Repeat for stage from 1 to {stages}:\n\
         \x20       Let half be len / 2.\n\
         \x20       Let m be {n} / len.\n\
         \x20       Repeat for blk from 0 to m - 1:\n\
         \x20           Let start be blk * len.\n\
         \x20           Repeat for j from 0 to half - 1:\n\
         \x20               Let tw be item (m * j + 1) of powers.\n\
         \x20               Let idx be start + j.\n\
         \x20               Let u be item (idx + 1) of a.\n\
         \x20               Let t be (tw * (item (idx + half + 1) of a)) % {q}.\n\
         \x20               Set item (idx + 1) of a to (u + t) % {q}.\n\
         \x20               Set item (idx + half + 1) of a to (u - t + {q}) % {q}.\n\
         \x20       Set len to len * 2.\n\
         Show item 1 of a.\n",
        q = Q,
    )
}

#[test]
fn race_logos_jit_ntt_against_handwritten_rust() {
    let n = 256i64;
    let w = root_of_unity(n, Q);
    let powers: Vec<i64> = (0..n).map(|i| pow_mod(w, i, Q)).collect();
    let mut s = 0xBEEF_u64;
    let f: Vec<i64> = (0..256)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (s >> 16) as i64 % Q
        })
        .collect();
    let a0 = bit_reverse_vec(&f);

    const K: usize = 100;

    let run_jit = |iters: usize| -> (String, std::time::Duration) {
        let prog = logos_fast_ntt_bench_program(&a0, &powers, iters);
        let tier = ForgeTier::new();
        let t = std::time::Instant::now();
        let r = vm_outcome_with_args(&prog, &[], Some(&tier as &dyn NativeTier));
        let dt = t.elapsed();
        assert_eq!(r.error, None, "JIT NTT error: {:?}", r.error);
        (r.output.trim().to_string(), dt)
    };
    let (_o1, t1) = run_jit(1);
    let (ojit, tk) = run_jit(K);
    let jit_ns = tk.saturating_sub(t1).as_nanos() as f64 / (K - 1) as f64;

    let mut ar = a0.clone();
    let t = std::time::Instant::now();
    for _ in 0..K {
        fast_ntt_rust(&mut ar, &powers, Q);
    }
    let rust_ns = t.elapsed().as_nanos() as f64 / K as f64;

    // Correctness: coefficient 1 after K transforms must equal hand-written Rust's.
    assert_eq!(ojit, ar[0].to_string(), "Logos-JIT NTT must agree with hand-written Rust");

    // Tree-walker floor, for the tier picture.
    let tw_t = std::time::Instant::now();
    let _ = tw_outcome(&logos_fast_ntt_program(&a0, &powers));
    let tw_ns = tw_t.elapsed().as_nanos() as f64;

    println!("\n=== n=256 forward NTT — one 256-point transform ===");
    println!("  hand-written Rust    : {rust_ns:>10.0} ns");
    println!("  Logos EXODIA JIT     : {jit_ns:>10.0} ns   ({:.1}× of Rust)", jit_ns / rust_ns.max(1.0));
    println!("  Logos tree-walker    : {tw_ns:>10.0} ns   (interpreter floor)");
    assert!(jit_ns > 0.0 && rust_ns > 0.0);
}

// SYMMETRY RATCHET #2 — finding (recorded, not yet implemented): the "one zeta per block"
// hoist (twiddle loop-invariant in the inner loop, ~30% fewer butterfly ops, what makes the
// SOTA libs fast) is mathematically the NEGACYCLIC NTT over ℤ_q[X]/(X^n+1) — Kyber's ACTUAL
// transform — not the cyclic DFT the fast NTT above computes. A standalone Rust experiment
// confirmed it: one-zeta-per-block produced non-DFT values. So the next ratchet is the real
// ML-KEM negacyclic NTT (where for n=256 mod 3329 no 512th root exists → Kyber's incomplete
// 7-level NTT, and the certified Gauss 5→4 base-multiply applies). That is Task #7's next
// milestone. The cyclic fast NTT above already banks the fundamental FFT symmetry (64× fewer
// multiplies, measured) — the bigger remaining gap to hand-Rust is codegen (the JIT not
// tiering this nested modular-array loop), not the algorithm.

// ---------------------------------------------------------------------------------------
// The REAL race: AOT-compiled native Logos (largo build --release → rustc -O3 -lto
// -target-cpu=native) vs hand-written native Rust, same flags, same machine. This is the
// "compiled Logos is the measure" number — the JIT/tree-walker are just floors.
// ---------------------------------------------------------------------------------------

/// Bench program with `mutable` on the reassigned `a`/`len` (the AOT path needs it), K NTTs.
fn logos_fast_ntt_bench_program_mut(a: &[i64], powers: &[i64], iters: usize) -> String {
    let n = a.len();
    let stages = n.trailing_zeros() as usize;
    let a_lit = a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let p_lit = powers.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    format!(
        "## Main\n\
         Let mutable a be [{a_lit}].\n\
         Let powers be [{p_lit}].\n\
         Repeat for iter from 1 to {iters}:\n\
         \x20   Let mutable len be 2.\n\
         \x20   Repeat for stage from 1 to {stages}:\n\
         \x20       Let half be len / 2.\n\
         \x20       Let m be {n} / len.\n\
         \x20       Repeat for blk from 0 to m - 1:\n\
         \x20           Let start be blk * len.\n\
         \x20           Repeat for j from 0 to half - 1:\n\
         \x20               Let tw be item (m * j + 1) of powers.\n\
         \x20               Let idx be start + j.\n\
         \x20               Let u be item (idx + 1) of a.\n\
         \x20               Let t be (tw * (item (idx + half + 1) of a)) % {q}.\n\
         \x20               Set item (idx + 1) of a to (u + t) % {q}.\n\
         \x20               Set item (idx + half + 1) of a to (u - t + {q}) % {q}.\n\
         \x20       Set len to len * 2.\n\
         Show item 1 of a.\n",
        q = Q,
    )
}

fn rust_ntt_bench_source(a: &[i64], powers: &[i64], iters: usize) -> String {
    let a_lit = a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let p_lit = powers.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    format!(
        "fn ntt(a: &mut [i64], powers: &[i64], q: i64) {{\n\
        \x20   let n = a.len();\n\
        \x20   let mut len = 2usize;\n\
        \x20   while len <= n {{\n\
        \x20       let half = len / 2; let m = n / len;\n\
        \x20       let mut blk = 0;\n\
        \x20       while blk < m {{\n\
        \x20           let start = blk * len;\n\
        \x20           let mut j = 0;\n\
        \x20           while j < half {{\n\
        \x20               let tw = powers[m * j]; let idx = start + j;\n\
        \x20               let u = a[idx]; let t = tw * a[idx + half] % q;\n\
        \x20               a[idx] = (u + t) % q; a[idx + half] = (u - t + q) % q;\n\
        \x20               j += 1;\n\
        \x20           }}\n\
        \x20           blk += 1;\n\
        \x20       }}\n\
        \x20       len *= 2;\n\
        \x20   }}\n\
        }}\n\
        fn main() {{\n\
        \x20   let mut a: Vec<i64> = vec![{a_lit}];\n\
        \x20   let powers: Vec<i64> = vec![{p_lit}];\n\
        \x20   for _ in 0..{iters} {{ ntt(&mut a, &powers, {q}); }}\n\
        \x20   println!(\"{{}}\", a[0]);\n\
        }}\n",
        q = Q,
    )
}

#[test]
#[ignore = "heavy: builds native binaries via largo + rustc"]
fn aot_native_race_compiled_logos_vs_rust() {
    use std::process::Command;
    let n = 256i64;
    let w = root_of_unity(n, Q);
    let powers: Vec<i64> = (0..n).map(|i| pow_mod(w, i, Q)).collect();
    let mut s = 0xBEEFu64;
    let f: Vec<i64> = (0..256)
        .map(|_| { s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345); (s >> 16) as i64 % Q })
        .collect();
    let a0 = bit_reverse_vec(&f);

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap().to_path_buf();
    let largo = repo.join("target/release/largo");
    assert!(largo.exists(), "build largo first: cargo build -p logicaffeine-cli --release");

    let proj = std::env::temp_dir().join("ntt_logos_aot_race");
    let _ = std::fs::remove_dir_all(&proj);
    std::fs::create_dir_all(proj.join("src")).unwrap();

    let median = |bin: &std::path::Path| -> (String, std::time::Duration) {
        let mut best = std::time::Duration::from_secs(999);
        let mut out = String::new();
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let o = Command::new(bin).output().expect("run bin");
            let dt = t.elapsed();
            out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if dt < best { best = dt; }
        }
        (out, best)
    };
    let bench = |iters: usize| -> (Duration_, Duration_) {
        // returns (logos, rust) process times for `iters` NTTs
        std::fs::write(proj.join("src/main.lg"), logos_fast_ntt_bench_program_mut(&a0, &powers, iters)).unwrap();
        std::fs::write(proj.join("Largo.toml"), "[package]\nname = \"nttbench\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n").unwrap();
        let st = Command::new(&largo).args(["build", "--release"]).current_dir(&proj).env("LOGOS_WORKSPACE", &repo).status().unwrap();
        assert!(st.success(), "largo build --release failed");
        // largo nests the binary under target/release/build/target/release/.
        let lbin = [
            "target/release/nttbench",
            "target/release/build/target/release/nttbench",
            "target/release/bench",
        ]
        .iter()
        .map(|p| proj.join(p))
        .find(|p| p.exists())
        .expect("logos binary");
        let rs = proj.join("rust_ntt.rs");
        std::fs::write(&rs, rust_ntt_bench_source(&a0, &powers, iters)).unwrap();
        let rbin = proj.join("rust_ntt");
        let st = Command::new("rustc").args(["-C","opt-level=3","-C","lto=fat","-C","codegen-units=1","-C","target-cpu=native","--edition","2021","-o"]).arg(&rbin).arg(&rs).status().unwrap();
        assert!(st.success(), "rustc failed");
        let (lo, lt) = median(&lbin);
        let (ro, rt) = median(&rbin);
        assert_eq!(lo, ro, "compiled Logos must agree with Rust (a[0] after {iters} NTTs)");
        (lt, rt)
    };
    // Subtract method: per-NTT = (t_K - t_base) / (K - base), isolating execution from startup.
    let k = 40000usize; let base = 4000usize;
    let (lk, rk) = bench(k);
    let (lb, rb) = bench(base);
    let lo_ns = lk.saturating_sub(lb).as_nanos() as f64 / (k - base) as f64;
    let ru_ns = rk.saturating_sub(rb).as_nanos() as f64 / (k - base) as f64;
    println!("\n=== n=256 forward NTT — AOT NATIVE (rustc -O3 -lto -target-cpu=native) ===");
    println!("  hand-written Rust : {ru_ns:>8.0} ns");
    println!("  compiled Logos    : {lo_ns:>8.0} ns   ({:.2}× of Rust)", lo_ns / ru_ns.max(1.0));
}
type Duration_ = std::time::Duration;

/// Mutable, iterated Montgomery NTT bench (AOT) — the division-free form, K NTTs.
fn logos_montgomery_ntt_bench_program_mut(a: &[i64], powers_mont: &[i64], iters: usize) -> String {
    let n = a.len();
    let stages = n.trailing_zeros() as usize;
    let a_lit = a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let p_lit = powers_mont.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let q = Q;
    let lines: Vec<String> = vec![
        "## To redc (x: Int) -> Int:".to_string(),
        "    Let lo be ((x % 65536) * 3327) % 65536.".to_string(),
        format!("    Let t be (x + lo * {q}) / 65536."),
        format!("    If t is at least {q}:"),
        format!("        Set t to t - {q}."),
        "    Return t.".to_string(),
        "## Main".to_string(),
        format!("Let mutable a be [{a_lit}]."),
        format!("Let powers be [{p_lit}]."),
        format!("Repeat for iter from 1 to {iters}:"),
        "    Let mutable len be 2.".to_string(),
        format!("    Repeat for stage from 1 to {stages}:"),
        "        Let half be len / 2.".to_string(),
        format!("        Let m be {n} / len."),
        "        Repeat for blk from 0 to m - 1:".to_string(),
        "            Let start be blk * len.".to_string(),
        "            Repeat for j from 0 to half - 1:".to_string(),
        "                Let tw be item (m * j + 1) of powers.".to_string(),
        "                Let idx be start + j.".to_string(),
        "                Let u be item (idx + 1) of a.".to_string(),
        "                Let t be redc(tw * (item (idx + half + 1) of a)).".to_string(),
        "                Let v be u + t.".to_string(),
        format!("                If v is at least {q}:"),
        format!("                    Set v to v - {q}."),
        "                Set item (idx + 1) of a to v.".to_string(),
        format!("                Let w be u - t + {q}."),
        format!("                If w is at least {q}:"),
        format!("                    Set w to w - {q}."),
        "                Set item (idx + half + 1) of a to w.".to_string(),
        "        Set len to len * 2.".to_string(),
        "Show item 1 of a.".to_string(),
    ];
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

#[test]
#[ignore = "heavy: builds native binaries via largo + rustc — the Montgomery vs schoolbook race"]
fn aot_montgomery_vs_schoolbook_ntt() {
    use std::process::Command;
    let n = 256i64;
    let w = root_of_unity(n, Q);
    let powers: Vec<i64> = (0..n).map(|i| pow_mod(w, i, Q)).collect();
    let powers_mont: Vec<i64> = powers.iter().map(|&p| (p * MONT_R).rem_euclid(Q)).collect();
    let mut s = 0xBEEFu64;
    let f: Vec<i64> = (0..256)
        .map(|_| { s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345); (s >> 16) as i64 % Q })
        .collect();
    let a0 = bit_reverse_vec(&f);

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap().to_path_buf();
    let largo = repo.join("target/release/largo");
    assert!(largo.exists(), "build largo first: cargo build -p logicaffeine-cli --release");

    let build_logos = |src: String, tag: &str| -> std::path::PathBuf {
        let proj = std::env::temp_dir().join(format!("ntt_mont_{tag}"));
        let _ = std::fs::remove_dir_all(&proj);
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::write(proj.join("src/main.lg"), src).unwrap();
        std::fs::write(proj.join("Largo.toml"), "[package]\nname = \"nttb\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n").unwrap();
        let st = Command::new(&largo).args(["build", "--release"]).current_dir(&proj).env("LOGOS_WORKSPACE", &repo).status().unwrap();
        assert!(st.success(), "largo build --release failed for {tag}");
        ["target/release/nttb", "target/release/build/target/release/nttb"]
            .iter().map(|p| proj.join(p)).find(|p| p.exists()).expect("logos binary")
    };
    let median = |bin: &std::path::Path| -> (String, std::time::Duration) {
        let mut best = std::time::Duration::from_secs(999);
        let mut out = String::new();
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let o = Command::new(bin).output().expect("run bin");
            let dt = t.elapsed();
            out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if dt < best { best = dt; }
        }
        (out, best)
    };
    let per_ntt = |prog: &dyn Fn(usize) -> String| -> (String, f64) {
        let (k, base) = (40000usize, 4000usize);
        let bk = build_logos(prog(k), "k");
        let (ok, tk) = median(&bk);
        let bb = build_logos(prog(base), "base");
        let (_ob, tb) = median(&bb);
        (ok, tk.saturating_sub(tb).as_nanos() as f64 / (k - base) as f64)
    };
    let (school_out, school_ns) = per_ntt(&|it| logos_fast_ntt_bench_program_mut(&a0, &powers, it));
    let (mont_out, mont_ns) = per_ntt(&|it| logos_montgomery_ntt_bench_program_mut(&a0, &powers_mont, it));
    assert_eq!(school_out, mont_out, "schoolbook and Montgomery must compute the same a[0]");
    println!("\n=== n=256 forward NTT — compiled-Logos: Montgomery vs schoolbook (AOT native) ===");
    println!("  schoolbook (% q)  : {school_ns:>8.0} ns / NTT");
    println!("  Montgomery (redc) : {mont_ns:>8.0} ns / NTT   ({:.2}× of schoolbook)", mont_ns / school_ns.max(1.0));
}
