//! LOGOS-LAPACK QR lock — `geqrf` (Householder QR: `A = Q·R`, `Q` orthogonal, `R` upper-triangular).
//!
//! Householder reflectors are the numerically stable route (the reference avoids Givens/`atan2`,
//! which LOGOS lacks until the transcendentals land). Two defining properties are checked, both
//! within a tight tolerance and both computed by reusing the BLAS `gemm` kernel: `Q·R == A`
//! (reconstruction) and `R` upper-triangular (below-diagonal entries vanish). `run_all_tiers`
//! also enforces interp==VM.

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
    let inner: Vec<String> = xs.iter().map(|x| float_lit(*x)).collect();
    format!("[{}]", inner.join(", "))
}

fn parse_seq_floats(out: &str) -> Vec<f64> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or_else(|| panic!("expected a bracketed Seq, got {out:?}"));
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float element"))
        .collect()
}

fn assert_close(got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "length: got {got:?} want {want:?}");
    for (i, (g, e)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            (g - e).abs() <= tol * (1.0 + e.abs()),
            "element {i}: got={g} want={e} diff={:.3e}",
            (g - e).abs()
        );
    }
}

/// Factor `A` (square, `n×n`) with `geqrf`, then reconstruct `Q·R` via `gemm` and confirm it
/// reproduces `A`; also confirm `R`'s strictly-lower triangle is ~0.
fn check_qr_square(n: usize, a: &[f64]) {
    let a_lit = seq_literal(a);
    let zeros = seq_literal(&vec![0.0; n * n]);

    // Q·R == A.
    let recon_src = format!(
        "## Main\nLet f be geqrf({n}, {n}, {a_lit}).\nLet q be f's q.\nLet r be f's r.\n\
         Show gemm({n}, {n}, {n}, 1.0, q, r, 0.0, {zeros}).\n"
    );
    let recon = run_all_tiers(&recon_src);
    assert!(
        recon.error.is_none(),
        "QR reconstruction errored: {:?}",
        recon.error
    );
    let qr = parse_seq_floats(&recon.output);
    assert_close(&qr, a, 1e-9);

    // R upper-triangular: every entry strictly below the diagonal is ~0.
    let r_src = format!("## Main\nLet f be geqrf({n}, {n}, {a_lit}).\nShow f's r.\n");
    let rout = run_all_tiers(&r_src);
    assert!(rout.error.is_none(), "R read errored: {:?}", rout.error);
    let r = parse_seq_floats(&rout.output);
    for i in 1..=n {
        for j in 1..i {
            let v = r[(i - 1) * n + (j - 1)];
            assert!(
                v.abs() <= 1e-9,
                "R[{i}][{j}] = {v} should be ~0 (upper-triangular)"
            );
        }
    }
}

#[test]
fn geqrf_reconstructs_2x2() {
    check_qr_square(2, &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn geqrf_reconstructs_golub_3x3() {
    // The classic Golub–Van Loan QR example.
    check_qr_square(3, &[12.0, -51.0, 4.0, 6.0, 167.0, -68.0, -4.0, 24.0, -41.0]);
}

#[test]
fn geqrf_reconstructs_asymmetric_3x3() {
    check_qr_square(3, &[2.0, -1.0, 0.0, 1.0, 4.0, 3.0, -2.0, 1.0, 5.0]);
}
