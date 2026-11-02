//! LOGOS-LAPACK generalized symmetric eigenproblem — `sygv` (`A·x = λ·B·x`, A symmetric, B SPD).
//!
//! Reduced to a standard eigenproblem via Cholesky: `B = L·Lᵀ`, `C = L⁻¹·A·L⁻ᵀ`, `syev(C)`, then
//! back-transform eigenvectors `x = L⁻ᵀ·y`. All sqrt-only. Locked by the defining generalized
//! eigenpair identity `A·V − B·V·diag(λ) == 0`, computed in-LOGOS, plus a known eigenvalue set.

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

// A·V − B·V·diag(λ), which must be ~0 for every generalized eigenpair.
const GEN_RESIDUAL: &str = "\
## To genRes (n: Int, a: Seq of Float, b: Seq of Float, v: Seq of Float, lam: Seq of Float) -> Seq of Float:\n\
\x20   Let mutable out be a new Seq of Float.\n\
\x20   Repeat for i from 1 to n:\n\
\x20       Repeat for k from 1 to n:\n\
\x20           Let mutable av be 0.0.\n\
\x20           Let mutable bv be 0.0.\n\
\x20           Repeat for j from 1 to n:\n\
\x20               Set av to av + (item ((i - 1) * n + j) of a) * (item ((j - 1) * n + k) of v).\n\
\x20               Set bv to bv + (item ((i - 1) * n + j) of b) * (item ((j - 1) * n + k) of v).\n\
\x20           Push av - (item k of lam) * bv to out.\n\
\x20   Return out.\n";

fn check_sygv_residual(n: usize, a: &[f64], b: &[f64]) {
    let src = format!(
        "{GEN_RESIDUAL}\n## Main\nLet e be sygv({n}, {a}, {b}).\nLet v be e's vecs.\nLet lam be e's vals.\n\
         Show genRes({n}, {a}, {b}, v, lam).\n",
        a = seq_literal(a),
        b = seq_literal(b),
    );
    let out = run_all_tiers(&src);
    assert!(
        out.error.is_none(),
        "sygv residual errored: {:?}",
        out.error
    );
    let res = parse_seq_floats(&out.output);
    assert_eq!(res.len(), n * n, "residual size");
    for (idx, v) in res.iter().enumerate() {
        assert!(v.abs() <= 1e-7, "residual[{idx}] = {v} should be ~0");
    }
}

#[test]
fn sygv_standard_when_b_is_identity() {
    // B = I → generalized reduces to standard: A=[[2,1],[1,2]] has eigenvalues {1,3}.
    check_sygv_residual(2, &[2.0, 1.0, 1.0, 2.0], &[1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn sygv_diagonal_metric() {
    // A=[[2,0],[0,2]], B=[[2,0],[0,1]] → eigenvalues {1,2}.
    check_sygv_residual(2, &[2.0, 0.0, 0.0, 2.0], &[2.0, 0.0, 0.0, 1.0]);
}

#[test]
fn sygv_3x3_spd_metric() {
    // A symmetric, B SPD (=[[4,2,2],[2,5,3],[2,3,6]]).
    check_sygv_residual(
        3,
        &[2.0, 1.0, 0.0, 1.0, 3.0, 1.0, 0.0, 1.0, 2.0],
        &[4.0, 2.0, 2.0, 2.0, 5.0, 3.0, 2.0, 3.0, 6.0],
    );
}

#[test]
fn sygv_eigenvalues_of_diagonal_metric() {
    // A=[[2,0],[0,2]], B=[[2,0],[0,1]] → eigenvalues {2, 1} (sorted).
    let src = format!(
        "## Main\nLet e be sygv(2, {}, {}).\nShow e's vals.\n",
        seq_literal(&[2.0, 0.0, 0.0, 2.0]),
        seq_literal(&[2.0, 0.0, 0.0, 1.0])
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "sygv errored: {:?}", out.error);
    let mut vals = parse_seq_floats(&out.output);
    vals.sort_by(|a, b| b.partial_cmp(a).unwrap());
    for (g, e) in vals.iter().zip([2.0, 1.0].iter()) {
        assert!((g - e).abs() <= 1e-8, "eigenvalue {g} vs {e}");
    }
}
