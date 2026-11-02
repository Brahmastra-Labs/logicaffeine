//! LOGOS-LAPACK linear-solve lock — `gesv` (LU with partial pivoting → solve `A·x = b`).
//!
//! Pivoted LU produces non-dyadic multipliers (e.g. 2/3), so the recovered `x` is correct only
//! to a few ULP, not bit-exact to a clean integer vector — this harness compares within a tight
//! tolerance rather than by string equality. Two independent checks per case: the solution
//! matches the known `x`, and the residual `A·x` reproduces `b` (computed by reusing the BLAS
//! `gemv` kernel — LAPACK verified against BLAS). `run_all_tiers` additionally enforces interp==VM.

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

/// Solve `A·x = b` via `gesv` and confirm both the solution and the residual `A·x == b`.
fn check_solve(n: usize, a: &[f64], b: &[f64], expect_x: &[f64]) {
    let a_lit = seq_literal(a);
    let b_lit = seq_literal(b);

    // 1. Solution matches the known x.
    let solve_src = format!("## Main\nShow gesv({n}, {a_lit}, {b_lit}).\n");
    let solved = run_all_tiers(&solve_src);
    assert!(solved.error.is_none(), "gesv errored: {:?}", solved.error);
    let x = parse_seq_floats(&solved.output);
    assert_close(&x, expect_x, 1e-10);

    // 2. Residual A·x reproduces b, via the BLAS gemv kernel (LAPACK ⟂ BLAS cross-check).
    let zeros = seq_literal(&vec![0.0; n]);
    let residual_src = format!(
        "## Main\nLet x be gesv({n}, {a_lit}, {b_lit}).\nShow gemv({n}, {n}, 1.0, {a_lit}, x, 0.0, {zeros}).\n"
    );
    let residual = run_all_tiers(&residual_src);
    assert!(
        residual.error.is_none(),
        "residual errored: {:?}",
        residual.error
    );
    let ax = parse_seq_floats(&residual.output);
    assert_close(&ax, b, 1e-10);
}

#[test]
fn gesv_2x2_requires_a_row_pivot() {
    // A=[[1,2],[3,4]] — |3| > |1| forces a row swap at step 1. b chosen so x=[1,1].
    check_solve(2, &[1.0, 2.0, 3.0, 4.0], &[3.0, 7.0], &[1.0, 1.0]);
}

#[test]
fn gesv_3x3_pivots_twice() {
    // A=[[2,1,1],[4,3,3],[8,7,9]] — pivots at both steps. b = A·[1,2,3].
    check_solve(
        3,
        &[2.0, 1.0, 1.0, 4.0, 3.0, 3.0, 8.0, 7.0, 9.0],
        &[7.0, 19.0, 49.0],
        &[1.0, 2.0, 3.0],
    );
}

#[test]
fn gesv_solves_a_fractional_system() {
    // A well-conditioned fractional system; x=[2, -1, 0.5]. b = A·x.
    // A = [[4,1,2],[1,3,1],[2,1,5]]  (SPD, but solved via general LU here).
    // b1 = 4·2 + 1·(-1) + 2·0.5 = 8 - 1 + 1 = 8
    // b2 = 1·2 + 3·(-1) + 1·0.5 = 2 - 3 + 0.5 = -0.5
    // b3 = 2·2 + 1·(-1) + 5·0.5 = 4 - 1 + 2.5 = 5.5
    check_solve(
        3,
        &[4.0, 1.0, 2.0, 1.0, 3.0, 1.0, 2.0, 1.0, 5.0],
        &[8.0, -0.5, 5.5],
        &[2.0, -1.0, 0.5],
    );
}
