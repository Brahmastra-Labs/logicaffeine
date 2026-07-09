//! De-risk probe for the Word16-representation rewrite: a compiled LOGOS program that builds a
//! `Seq of Word16`, calls the Word16-carrier native NTT (`mlkemNttW16`), and `Show`s the result.
//! Confirms the AOT codegen handles `Seq of Word16` native params (zero-copy borrow) + a
//! `LogosSeq<Word16>` return + `Show` of `Word16` — the path the full Word16 ML-KEM rides on.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — Word16-native NTT codegen gate"]
fn word16_ntt_native_matches_kernel() {
    use logicaffeine_base::Word16;
    let input: Vec<Word16> = (0..256).map(|i| Word16(i as u16)).collect();
    let want: Vec<i64> =
        logicaffeine_system::ntt::mlkem_ntt_w16(&input).iter().map(|w| w.0 as i64).collect();

    let prog = "## Main\nLet a be a new Seq of Word16.\nRepeat for i from 0 to 255:\n    Push word16(i) to a.\nLet b be mlkemNttW16(a).\nRepeat for i from 1 to 256:\n    Show item i of b.\n";
    let aot = run_logos_with_args(prog, &[]);
    assert!(aot.success, "AOT failed:\n--- stderr ---\n{}\n--- rust ---\n{}", aot.stderr, aot.rust_code);
    let got: Vec<i64> = aot
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("integer output line"))
        .collect();
    assert_eq!(got.len(), 256, "256 NTT coefficients");
    assert_eq!(got, want, "Word16-native NTT in compiled Logos must match the runtime kernel");
}
