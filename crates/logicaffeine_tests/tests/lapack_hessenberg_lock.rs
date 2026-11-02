//! LOGOS-LAPACK Hessenberg reduction — `hessenberg` (`H = Qᵀ·A·Q` upper-Hessenberg, reference
//! LAPACK `DGEHRD`). Householder reflectors applied on BOTH sides (a similarity transform), so the
//! eigenvalues are preserved. Locked by the defining structure (entries below the sub-diagonal
//! vanish — a symmetric `A` becomes tridiagonal) and by trace invariance (`trace(H) == trace(A)`).

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
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float"))
        .collect()
}
fn hessenberg_of(n: usize, a: &[f64]) -> Vec<f64> {
    let src = format!("## Main\nShow hessenberg({n}, {}).\n", seq_literal(a));
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "hessenberg errored: {:?}", out.error);
    parse_seq_floats(&out.output)
}

#[test]
fn hessenberg_reduces_symmetric_to_tridiagonal() {
    // A symmetric 4×4 → H is tridiagonal (|i-j|>1 entries vanish); trace = 4+5+6+7 = 22 preserved.
    let a = [
        4.0, 1.0, 2.0, 3.0, //
        1.0, 5.0, 1.0, 2.0, //
        2.0, 1.0, 6.0, 1.0, //
        3.0, 2.0, 1.0, 7.0,
    ];
    let h = hessenberg_of(4, &a);
    assert_eq!(h.len(), 16);
    for i in 1..=4usize {
        for j in 1..=4usize {
            if (i as i32 - j as i32).abs() > 1 {
                let v = h[(i - 1) * 4 + (j - 1)];
                assert!(
                    v.abs() <= 1e-9,
                    "H[{i}][{j}] = {v} should vanish (tridiagonal)"
                );
            }
        }
    }
    let tr: f64 = (0..4).map(|k| h[k * 4 + k]).sum();
    assert!((tr - 22.0).abs() <= 1e-9, "trace = {tr}, expected 22");
}

#[test]
fn hessenberg_nonsymmetric_zeros_below_subdiagonal() {
    // General 4×4 → H[i][j] = 0 for i > j+1 (H[3][1], H[4][1], H[4][2]); trace preserved.
    let a = [
        1.0, 2.0, 3.0, 4.0, //
        5.0, 6.0, 7.0, 8.0, //
        9.0, 1.0, 2.0, 3.0, //
        4.0, 5.0, 6.0, 7.0,
    ];
    let h = hessenberg_of(4, &a);
    for &(i, j) in &[(3usize, 1usize), (4, 1), (4, 2)] {
        let v = h[(i - 1) * 4 + (j - 1)];
        assert!(
            v.abs() <= 1e-9,
            "H[{i}][{j}] = {v} should vanish (below sub-diagonal)"
        );
    }
    let tr: f64 = (0..4).map(|k| h[k * 4 + k]).sum();
    assert!((tr - 16.0).abs() <= 1e-9, "trace = {tr}, expected 16"); // 1+6+2+7
}
