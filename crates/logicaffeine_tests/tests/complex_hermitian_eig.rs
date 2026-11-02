//! `re`/`im` builtins + Hermitian eigenvalues (cheev) + complex singular values (csvdvals).
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted, run_all_tiers};

fn parse_floats(out: &str) -> Vec<f64> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .expect("bracketed");
    if inner.trim().is_empty() {
        return vec![];
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("f64"))
        .collect()
}
fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}
fn assert_close(src: &str, want: &[f64], tol: f64) {
    let out = run_all_tiers(src);
    assert!(out.error.is_none(), "errored: {:?}\n{src}", out.error);
    let g = sorted(parse_floats(&out.output));
    let w = sorted(want.to_vec());
    assert_eq!(g.len(), w.len(), "{g:?} vs {w:?}");
    for (a, b) in g.iter().zip(w.iter()) {
        assert!(
            (a - b).abs() <= tol * (1.0 + b.abs()),
            "{a} vs {b} in {g:?}/{w:?}"
        );
    }
}

#[test]
fn re_im_extract_parts() {
    let src = "## Main\nShow re(complex(3.0, 4.0)).\nShow im(complex(3.0, 4.0)).\nShow re(5.0).\nShow im(5.0).\n";
    assert_all_tiers(src, "3\n4\n5\n0");
    assert_compiled_equals_interpreted(src);
}

#[test]
fn cheev_hermitian_2x2_spectrum() {
    // [[2, i], [-i, 2]] ⇒ {1, 3}.
    assert_close("## Main\nShow cheev(2, [complex(2.0, 0.0), complex(0.0, 1.0), complex(0.0, -1.0), complex(2.0, 0.0)]).\n", &[1.0, 3.0], 1e-6);
}
#[test]
fn cheev_diagonal_is_exact() {
    let src = "## Main\nShow cheev(2, [complex(4.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(9.0, 0.0)]).\n";
    assert_all_tiers(src, "[4, 9]");
    assert_compiled_equals_interpreted(src);
}
#[test]
fn cheev_3x3_hermitian() {
    // [[2, 1+i, 0],[1-i, 3, 0],[0,0,5]] ⇒ eigenvalues of the 2×2 block {(5±√5... )} + 5.
    // 2×2 block [[2,1+i],[1-i,3]]: trace 5, det 6-2=4 ⇒ λ=(5±3)/2={1,4}; plus 5 ⇒ {1,4,5}.
    assert_close("## Main\nShow cheev(3, [complex(2.0,0.0),complex(1.0,1.0),complex(0.0,0.0),complex(1.0,-1.0),complex(3.0,0.0),complex(0.0,0.0),complex(0.0,0.0),complex(0.0,0.0),complex(5.0,0.0)]).\n", &[1.0, 4.0, 5.0], 1e-6);
}
#[test]
fn csvdvals_diagonal() {
    // A=diag(3,4) ⇒ singular values {3,4}.
    assert_close("## Main\nShow csvdvals(2, 2, [complex(3.0,0.0),complex(0.0,0.0),complex(0.0,0.0),complex(4.0,0.0)]).\n", &[3.0, 4.0], 1e-6);
}
#[test]
fn csvdvals_rank_deficient() {
    // A=[[3,4],[0,0]] ⇒ AᴴA=[[9,12],[12,16]], eig {0,25} ⇒ σ {0,5}.
    assert_close("## Main\nShow csvdvals(2, 2, [complex(3.0,0.0),complex(4.0,0.0),complex(0.0,0.0),complex(0.0,0.0)]).\n", &[0.0, 5.0], 1e-6);
}
