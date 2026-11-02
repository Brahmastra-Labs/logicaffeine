//! Wave 3 complex LAPACK: cdet, ctrace, ceye, cdiagOf, cpotrs, cpotri, cgels + real stev.
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted, run_all_tiers};

fn both(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
}
fn parse_floats(out: &str) -> Vec<f64> {
    let inner = out
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .expect("[]");
    if inner.trim().is_empty() {
        return vec![];
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().unwrap())
        .collect()
}
fn close(src: &str, want: &[f64], tol: f64) {
    let out = run_all_tiers(src);
    assert!(out.error.is_none(), "{:?}", out.error);
    let mut g = parse_floats(&out.output);
    g.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut w = want.to_vec();
    w.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(g.len(), w.len(), "{g:?} vs {w:?}");
    for (a, b) in g.iter().zip(w.iter()) {
        assert!((a - b).abs() <= tol * (1.0 + b.abs()), "{a} vs {b}");
    }
}

#[test]
fn cdet_diag_and_swap() {
    both("## Main\nShow cdet(2, [complex(1.0, 1.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(1.0, -1.0)]).\n", "2");
    both("## Main\nShow cdet(2, [complex(0.0, 0.0), complex(1.0, 0.0), complex(1.0, 0.0), complex(0.0, 0.0)]).\n", "-1");
}
#[test]
fn ctrace_diagonal_sum() {
    both("## Main\nShow ctrace(2, [complex(1.0, 2.0), complex(9.0, 9.0), complex(9.0, 9.0), complex(3.0, 4.0)]).\n", "4+6i");
}
#[test]
fn ceye_identity() {
    both("## Main\nShow ceye(3).\n", "[1, 0, 0, 0, 1, 0, 0, 0, 1]");
}
#[test]
fn cdiag_of_matrix() {
    both("## Main\nShow cdiagOf(2, [complex(1.0, 1.0), complex(2.0, 0.0), complex(3.0, 0.0), complex(4.0, -1.0)]).\n", "[1+i, 4-i]");
}
#[test]
fn cpotrs_solve_with_factor() {
    // L = [[2,0],[-i,2]] (Cholesky of [[4,2i],[-2i,5]]), b=[4+2i, 5-2i] ⇒ x=[1,1].
    both("## Main\nShow cpotrs(2, [complex(2.0, 0.0), complex(0.0, 0.0), complex(0.0, -1.0), complex(2.0, 0.0)], [complex(4.0, 2.0), complex(5.0, -2.0)]).\n", "[1, 1]");
}
#[test]
fn cpotri_hpd_inverse() {
    both("## Main\nShow cpotri(2, [complex(4.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(16.0, 0.0)]).\n", "[0.25, 0, 0, 0.0625]");
}
#[test]
fn cgels_least_squares() {
    // A=[[1,0],[0,1],[0,0]] (3×2), b=[3,4,0] ⇒ x=[3,4].
    both("## Main\nShow cgels(3, 2, [complex(1.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(1.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0)], [complex(3.0, 0.0), complex(4.0, 0.0), complex(0.0, 0.0)]).\n", "[3, 4]");
}
#[test]
fn stev_tridiagonal_eigenvalues() {
    // d=[2,2], e=[1] ⇒ [[2,1],[1,2]] ⇒ {1,3}.
    close(
        "## Main\nShow stev(2, [2.0, 2.0], [1.0]).\n",
        &[1.0, 3.0],
        1e-9,
    );
}
