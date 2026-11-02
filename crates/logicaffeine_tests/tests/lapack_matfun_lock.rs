//! LOGOS-LAPACK matrix functions — `trace`, `matpow` (A^k), `kron` (Kronecker product), and the
//! flagship `expm` (matrix exponential via scaling-and-squaring + Taylor series). `expm` is pure
//! native LOGOS: it reproduces even a rotation matrix's cos/sin ENTIRELY from the matrix series,
//! with no transcendental builtins. Exact cases use `assert_all_tiers`; `expm` uses a tolerance
//! harness against known closed forms.

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
fn expm(n: usize, a: &[f64]) -> Vec<f64> {
    let src = format!("## Main\nShow expm({n}, {}).\n", seq_literal(a));
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "expm errored: {:?}", out.error);
    parse_seq_floats(&out.output)
}

#[test]
fn trace_sums_the_diagonal() {
    assert_all_tiers("## Main\nShow matTrace(2, [1.0, 2.0, 3.0, 4.0]).\n", "5");
}

#[test]
fn matpow_repeated_multiplication() {
    // [[1,1],[0,1]]^3 = [[1,3],[0,1]].
    assert_all_tiers(
        "## Main\nShow matpow(2, [1.0, 1.0, 0.0, 1.0], 3).\n",
        "[1, 3, 0, 1]",
    );
}

#[test]
fn matpow_zero_is_identity() {
    assert_all_tiers(
        "## Main\nShow matpow(2, [5.0, 7.0, 2.0, 3.0], 0).\n",
        "[1, 0, 0, 1]",
    );
}

#[test]
fn kron_kronecker_product() {
    // [2] ⊗ [[1,2],[3,4]] = [[2,4],[6,8]].
    assert_all_tiers(
        "## Main\nShow kron(1, 1, 2, 2, [2.0], [1.0, 2.0, 3.0, 4.0]).\n",
        "[2, 4, 6, 8]",
    );
}

// ---- expm: matrix exponential ----

#[test]
fn expm_of_zero_is_identity() {
    assert_close(
        &expm(2, &[0.0, 0.0, 0.0, 0.0]),
        &[1.0, 0.0, 0.0, 1.0],
        1e-12,
    );
}

#[test]
fn expm_of_nilpotent_is_i_plus_a() {
    // [[0,1],[0,0]] is nilpotent → e^A = I + A = [[1,1],[0,1]].
    assert_close(
        &expm(2, &[0.0, 1.0, 0.0, 0.0]),
        &[1.0, 1.0, 0.0, 1.0],
        1e-10,
    );
}

#[test]
fn expm_of_identity_is_e_times_i() {
    // e^I = e·I.
    let e = std::f64::consts::E;
    assert_close(&expm(2, &[1.0, 0.0, 0.0, 1.0]), &[e, 0.0, 0.0, e], 1e-10);
}

#[test]
fn expm_of_rotation_generator_gives_cos_sin() {
    // e^[[0,-1],[1,0]] = [[cos1,-sin1],[sin1,cos1]] — trig recovered from the matrix series alone.
    let (c, s) = (1.0_f64.cos(), 1.0_f64.sin());
    assert_close(&expm(2, &[0.0, -1.0, 1.0, 0.0]), &[c, -s, s, c], 1e-9);
}
