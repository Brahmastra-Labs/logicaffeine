//! LOGOS-LAPACK vector utilities + SVD least-squares — `gelsd` (minimum-norm least squares via the
//! pseudo-inverse, handles rank-deficient `A`), `normalize` (unit vector), `cross` (3D cross
//! product). Native LOGOS. Exact cases use `assert_all_tiers`; `gelsd` uses a tolerance harness.

mod common;
use common::{assert_all_tiers, run_all_tiers};

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

#[test]
fn gelsd_overdetermined_least_squares() {
    // Fit y = 1 + x through (0,1),(1,2),(2,3): A=[[1,0],[1,1],[1,2]], b=[1,2,3] → x=[1,1].
    let src = format!(
        "## Main\nShow gelsd(3, 2, {}, {}).\n",
        seq_literal(&[1.0, 0.0, 1.0, 1.0, 1.0, 2.0]),
        seq_literal(&[1.0, 2.0, 3.0])
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "gelsd errored: {:?}", out.error);
    assert_close(&parse_seq_floats(&out.output), &[1.0, 1.0], 1e-8);
}

#[test]
fn normalize_makes_a_unit_vector() {
    // [3,4] / 5 = [0.6, 0.8].
    assert_all_tiers("## Main\nShow normalize(2, [3.0, 4.0]).\n", "[0.6, 0.8]");
}

#[test]
fn cross_of_basis_vectors() {
    // x̂ × ŷ = ẑ.
    assert_all_tiers(
        "## Main\nShow cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).\n",
        "[0, 0, 1]",
    );
}

#[test]
fn cross_is_antisymmetric() {
    // ŷ × x̂ = -ẑ.
    assert_all_tiers(
        "## Main\nShow cross([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]).\n",
        "[0, 0, -1]",
    );
}
