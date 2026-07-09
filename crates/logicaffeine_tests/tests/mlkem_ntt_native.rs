//! The stdlib `mlkemNtt` native function lowers a LOGOS `mlkemNtt(a)` call to the verified
//! scalar+AVX2 i16 ML-KEM NTT kernel in `logicaffeine_system` (≈49 ns/NTT). A demand-imported
//! program that calls it must, compiled the full AOT way, produce exactly what the runtime
//! kernel produces — proving the fast native NTT is reachable from compiled LOGOS.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;

fn program(input: &[i64]) -> String {
    let a_lit = input.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    format!(
        "## Main\nLet a be [{a_lit}].\nLet result be mlkemNtt(a).\nRepeat for i from 1 to 256:\n    Show item i of result.\n"
    )
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT mlkemNtt native-kernel gate"]
fn mlkem_ntt_stdlib_compiles_to_the_runtime_kernel() {
    let input: Vec<i64> = (0..256).map(|i| (i * 37 % 5000) as i64 - 1000).collect();
    // The ground truth: the verified runtime kernel itself.
    let want: Vec<i64> =
        logicaffeine_system::mlkem_ntt(&input).to_vec();

    let aot = run_logos_with_args(&program(&input), &[]);
    assert!(
        aot.success,
        "AOT compile+run of mlkemNtt failed:\n--- stderr ---\n{}\n--- generated rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    let got: Vec<i64> = aot
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("integer output line"))
        .collect();
    assert_eq!(got.len(), 256, "256 NTT coefficients");
    assert_eq!(got, want, "compiled mlkemNtt must equal the verified runtime AVX2 kernel");
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT NTT round-trip gate"]
fn mlkem_ntt_invntt_round_trips_in_compiled_logos() {
    const MONT: i64 = 2285; // R mod q — invntt is `tomont`, so invntt(ntt(a)) = a·MONT mod q
    const Q: i64 = 3329;
    let input: Vec<i64> = (0..256).map(|i| (i * 91 % 6000) as i64 - 1500).collect();

    // Ground truth via the verified runtime kernels, AND the round-trip property a·MONT mod q.
    let h = logicaffeine_system::mlkem_ntt(&input).to_vec();
    let want = logicaffeine_system::mlkem_inv_ntt(&h).to_vec();
    for i in 0..256 {
        assert_eq!(
            want[i],
            (input[i].rem_euclid(Q) * MONT).rem_euclid(Q),
            "round-trip property invntt(ntt(a)) = a·MONT mod q at {i}"
        );
    }

    let a_lit = input.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let prog = format!(
        "## Main\nLet a be [{a_lit}].\nLet h be mlkemNtt(a).\nLet r be mlkemInvNtt(h).\nRepeat for i from 1 to 256:\n    Show item i of r.\n"
    );
    let aot = run_logos_with_args(&prog, &[]);
    assert!(aot.success, "AOT failed:\n{}\n{}", aot.stderr, aot.rust_code);
    let got: Vec<i64> = aot
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("integer output line"))
        .collect();
    assert_eq!(got, want, "compiled mlkemInvNtt(mlkemNtt(a)) must equal the runtime round-trip");
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT full polynomial-multiply gate"]
fn full_poly_multiply_in_compiled_logos() {
    let a: Vec<i64> = (0..256).map(|i| (i * 13 % 3000) as i64).collect();
    let b: Vec<i64> = (0..256).map(|i| (i * 29 % 3000) as i64).collect();

    // Ground truth: the verified runtime kernels — invntt(basemul(ntt(a), ntt(b))).
    let ah = logicaffeine_system::mlkem_ntt(&a).to_vec();
    let bh = logicaffeine_system::mlkem_ntt(&b).to_vec();
    let prod = logicaffeine_system::mlkem_base_mul(&ah, &bh).to_vec();
    let want = logicaffeine_system::mlkem_inv_ntt(&prod).to_vec();

    let a_lit = a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let b_lit = b.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let prog = format!(
        "## Main\nLet a be [{a_lit}].\nLet b be [{b_lit}].\nLet ah be mlkemNtt(a).\nLet bh be mlkemNtt(b).\nLet prod be mlkemBaseMul(ah, bh).\nLet c be mlkemInvNtt(prod).\nRepeat for i from 1 to 256:\n    Show item i of c.\n"
    );
    let aot = run_logos_with_args(&prog, &[]);
    assert!(aot.success, "AOT failed:\n{}\n{}", aot.stderr, aot.rust_code);
    let got: Vec<i64> = aot
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("integer output line"))
        .collect();
    assert_eq!(
        got, want,
        "compiled Logos polynomial multiply (invntt∘basemul∘ntt) must equal the runtime"
    );
}
