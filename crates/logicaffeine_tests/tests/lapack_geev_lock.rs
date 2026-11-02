//! LOGOS-LAPACK nonsymmetric eigenvalues — `geev` (`A·x = λ·x` for a general real `A`).
//!
//! Shifted QR iteration (reusing the `geqrf` kernel) drives `A` toward real Schur form; the
//! eigenvalues are then read off the quasi-triangular result — `1×1` blocks give real eigenvalues,
//! `2×2` blocks give complex-conjugate pairs (returned as real parts `wr` and imaginary parts `wi`).
//! Locked against known real and complex spectra, and cross-checked against `syev` on a symmetric
//! matrix (which exercises real QR convergence). `run_all_tiers` enforces interp==VM.

mod common;
use common::run_all_tiers;

fn float_lit(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}
fn seq_literal(xs: &[f64]) -> String {
    format!(
        "[{}]",
        xs.iter()
            .map(|x| float_lit(*x))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn parse_seq_floats(out: &str) -> Vec<f64> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .expect("bracketed Seq");
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float"))
        .collect()
}
fn run_field(n: usize, a: &[f64], field: &str) -> Vec<f64> {
    let src = format!(
        "## Main\nLet e be geev({n}, {}).\nShow e's {field}.\n",
        seq_literal(a)
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "geev errored: {:?}\n{src}", out.error);
    parse_seq_floats(&out.output)
}
fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}
fn assert_sorted_close(got: Vec<f64>, want: &[f64], tol: f64) {
    let g = sorted(got);
    let w = sorted(want.to_vec());
    assert_eq!(g.len(), w.len(), "len {g:?} vs {w:?}");
    for (a, b) in g.iter().zip(w.iter()) {
        assert!(
            (a - b).abs() <= tol * (1.0 + b.abs()),
            "{a} vs {b} in {g:?}/{w:?}"
        );
    }
}

#[test]
fn geev_upper_triangular_real_spectrum() {
    // Eigenvalues of an upper-triangular matrix are its diagonal: {2, 3}.
    let a = [2.0, 1.0, 0.0, 3.0];
    assert_sorted_close(run_field(2, &a, "wr"), &[2.0, 3.0], 1e-7);
    assert_sorted_close(run_field(2, &a, "wi"), &[0.0, 0.0], 1e-7);
}

#[test]
fn geev_pure_imaginary_pair() {
    // [[0,-1],[1,0]] has eigenvalues ±i: real parts 0, imaginary parts ±1.
    let a = [0.0, -1.0, 1.0, 0.0];
    assert_sorted_close(run_field(2, &a, "wr"), &[0.0, 0.0], 1e-7);
    assert_sorted_close(run_field(2, &a, "wi"), &[-1.0, 1.0], 1e-7);
}

#[test]
fn geev_complex_pair() {
    // [[3,-2],[4,-1]]: trace 2, det 5 → eigenvalues 1 ± 2i.
    let a = [3.0, -2.0, 4.0, -1.0];
    assert_sorted_close(run_field(2, &a, "wr"), &[1.0, 1.0], 1e-7);
    assert_sorted_close(run_field(2, &a, "wi"), &[-2.0, 2.0], 1e-7);
}

#[test]
fn geev_3x3_upper_triangular() {
    // Diagonal {2,3,4}.
    let a = [2.0, 1.0, 5.0, 0.0, 3.0, 1.0, 0.0, 0.0, 4.0];
    assert_sorted_close(run_field(3, &a, "wr"), &[2.0, 3.0, 4.0], 1e-7);
    assert_sorted_close(run_field(3, &a, "wi"), &[0.0, 0.0, 0.0], 1e-7);
}

#[test]
fn geev_block_diagonal_real_plus_complex() {
    // block-diag(2, [[0,-1],[1,0]]) → eigenvalues {2, ±i}.
    let a = [2.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0];
    assert_sorted_close(run_field(3, &a, "wr"), &[0.0, 0.0, 2.0], 1e-7);
    assert_sorted_close(run_field(3, &a, "wi"), &[-1.0, 0.0, 1.0], 1e-7);
}

#[test]
fn geev_matches_syev_on_symmetric() {
    // On a symmetric matrix geev's real spectrum must equal syev's (real QR must converge).
    let a = [4.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 1.0, 2.0];
    let geev_wr = sorted(run_field(3, &a, "wr"));
    // syev on the same matrix.
    let src = format!(
        "## Main\nLet e be syev(3, {}).\nShow e's vals.\n",
        seq_literal(&a)
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "syev errored: {:?}", out.error);
    let syev_vals = sorted(parse_seq_floats(&out.output));
    for (g, s) in geev_wr.iter().zip(syev_vals.iter()) {
        assert!((g - s).abs() <= 1e-6, "geev {g} vs syev {s}");
    }
    // imaginary parts must vanish for a symmetric matrix.
    assert_sorted_close(run_field(3, &a, "wi"), &[0.0, 0.0, 0.0], 1e-6);
}
