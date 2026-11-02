//! LOGOS-LAPACK specialized solvers — `posv` (SPD via Cholesky) and `gtsv` (tridiagonal, Thomas).
//!
//! These exploit matrix structure the general LU solve ignores: `posv` factors an SPD system with
//! Cholesky (half the work, no pivoting), `gtsv` runs the O(n) tridiagonal sweep. Each is locked by
//! solving a system with a known solution and confirming the residual within tolerance.

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
fn assert_close(got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "len: {got:?} vs {want:?}");
    for (i, (g, e)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            (g - e).abs() <= tol * (1.0 + e.abs()),
            "elem {i}: {g} vs {e}"
        );
    }
}
fn run_seq(src: &str) -> Vec<f64> {
    let out = run_all_tiers(src);
    assert!(out.error.is_none(), "errored: {:?}\n{src}", out.error);
    parse_seq_floats(&out.output)
}

// ---- posv: SPD solve via Cholesky ----

#[test]
fn posv_solves_2x2_spd() {
    // A=[[4,2],[2,5]] SPD, x=[1,1] → b=[6,7].
    let src = format!(
        "## Main\nShow posv(2, {}, {}).\n",
        seq_literal(&[4.0, 2.0, 2.0, 5.0]),
        seq_literal(&[6.0, 7.0])
    );
    assert_close(&run_seq(&src), &[1.0, 1.0], 1e-10);
}

#[test]
fn posv_solves_3x3_spd() {
    // A=[[4,2,2],[2,5,3],[2,3,6]] SPD, x=[1,2,3] → b=[14,21,26].
    let src = format!(
        "## Main\nShow posv(3, {}, {}).\n",
        seq_literal(&[4.0, 2.0, 2.0, 2.0, 5.0, 3.0, 2.0, 3.0, 6.0]),
        seq_literal(&[14.0, 21.0, 26.0])
    );
    assert_close(&run_seq(&src), &[1.0, 2.0, 3.0], 1e-10);
}

// ---- gtsv: tridiagonal solve (Thomas algorithm) ----

#[test]
fn gtsv_solves_symmetric_tridiagonal() {
    // Tridiag(1;2;1) size 3, x=[1,1,1] → b=[3,4,3]. dl=[1,1], d=[2,2,2], du=[1,1].
    let src = format!(
        "## Main\nShow gtsv(3, {}, {}, {}, {}).\n",
        seq_literal(&[1.0, 1.0]),
        seq_literal(&[2.0, 2.0, 2.0]),
        seq_literal(&[1.0, 1.0]),
        seq_literal(&[3.0, 4.0, 3.0])
    );
    assert_close(&run_seq(&src), &[1.0, 1.0, 1.0], 1e-10);
}

#[test]
fn gtsv_solves_fractional_tridiagonal() {
    // Tridiag(1;3;1) size 3, x=[1,-1,2] → b=[2,0,5].
    let src = format!(
        "## Main\nShow gtsv(3, {}, {}, {}, {}).\n",
        seq_literal(&[1.0, 1.0]),
        seq_literal(&[3.0, 3.0, 3.0]),
        seq_literal(&[1.0, 1.0]),
        seq_literal(&[2.0, 0.0, 5.0])
    );
    assert_close(&run_seq(&src), &[1.0, -1.0, 2.0], 1e-10);
}
