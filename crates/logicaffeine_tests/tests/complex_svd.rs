//! Complex Hermitian eigenvectors (cheevv) + full complex SVD (cgesvd), verified by reconstruction
//! (`V·Λ·Vᴴ = H`, `U·Σ·Vᴴ = A`).
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted, run_all_tiers};

fn parse_complex(t: &str) -> (f64, f64) {
    let t = t.trim();
    if !t.ends_with('i') {
        return (t.parse().unwrap(), 0.0);
    }
    let b = &t[..t.len() - 1];
    let by = b.as_bytes();
    let mut sp = None;
    for k in (1..by.len()).rev() {
        let c = by[k] as char;
        if (c == '+' || c == '-') && by[k - 1] as char != 'e' && by[k - 1] as char != 'E' {
            sp = Some(k);
            break;
        }
    }
    match sp {
        Some(p) => {
            let re: f64 = b[..p].parse().unwrap();
            let im = match &b[p..] {
                "+" => 1.0,
                "-" => -1.0,
                s => s.parse().unwrap(),
            };
            (re, im)
        }
        None => {
            let im = match b {
                "" => 1.0,
                "-" => -1.0,
                s => s.parse().unwrap(),
            };
            (0.0, im)
        }
    }
}
fn parse_cseq(out: &str) -> Vec<(f64, f64)> {
    let inner = out
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap();
    if inner.trim().is_empty() {
        return vec![];
    }
    inner.split(',').map(|s| parse_complex(s.trim())).collect()
}
fn recon_close(src: &str, want: &[(f64, f64)], tol: f64) {
    let out = run_all_tiers(src);
    assert!(out.error.is_none(), "{:?}\n{src}", out.error);
    let g = parse_cseq(&out.output);
    assert_eq!(g.len(), want.len(), "{g:?} vs {want:?}");
    for (a, b) in g.iter().zip(want.iter()) {
        assert!(
            (a.0 - b.0).abs() <= tol && (a.1 - b.1).abs() <= tol,
            "{a:?} vs {b:?} in {g:?}"
        );
    }
}

const Z4: &str = "[complex(0.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0)]";

#[test]
fn cheevv_diagonal_vecs_are_identity() {
    let src = "## Main\nLet e be cheevv(2, [complex(3.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(5.0, 0.0)]).\nShow e's vals.\nShow e's vecs.\n";
    assert_all_tiers(src, "[3, 5]\n[1, 0, 0, 1]");
    assert_compiled_equals_interpreted(src);
}

#[test]
fn cheevv_reconstructs_hermitian() {
    // V·diag(λ)·Vᴴ == H for H=[[2,i],[-i,2]].
    let src = format!(
        "\
## Main
Let e be cheevv(2, [complex(2.0, 0.0), complex(0.0, 1.0), complex(0.0, -1.0), complex(2.0, 0.0)]).
Let vv be e's vecs.
Let vals be e's vals.
Let mutable lam be {Z4}.
Set item 1 of lam to complex(item 1 of vals, 0.0).
Set item 4 of lam to complex(item 2 of vals, 0.0).
Let vl be cgemm(2, 2, 2, complex(1.0, 0.0), vv, lam, complex(0.0, 0.0), {Z4}).
Let vh be cconjtranspose(2, 2, vv).
Show cgemm(2, 2, 2, complex(1.0, 0.0), vl, vh, complex(0.0, 0.0), {Z4}).
"
    );
    recon_close(
        &src,
        &[(2.0, 0.0), (0.0, 1.0), (0.0, -1.0), (2.0, 0.0)],
        1e-6,
    );
}

#[test]
fn cgesvd_reconstructs_matrix() {
    // U·Σ·Vᴴ == A for A=[[2, i],[-i, 2]] (Hermitian, but treated as a general 2×2).
    let src = format!("\
## Main
Let sv be cgesvd(2, 2, [complex(2.0, 0.0), complex(0.0, 1.0), complex(0.0, -1.0), complex(2.0, 0.0)]).
Let u be sv's u.
Let s be sv's s.
Let vv be sv's v.
Let mutable sig be {Z4}.
Set item 1 of sig to complex(item 1 of s, 0.0).
Set item 4 of sig to complex(item 2 of s, 0.0).
Let us be cgemm(2, 2, 2, complex(1.0, 0.0), u, sig, complex(0.0, 0.0), {Z4}).
Let vh be cconjtranspose(2, 2, vv).
Show cgemm(2, 2, 2, complex(1.0, 0.0), us, vh, complex(0.0, 0.0), {Z4}).
");
    recon_close(
        &src,
        &[(2.0, 0.0), (0.0, 1.0), (0.0, -1.0), (2.0, 0.0)],
        1e-6,
    );
}
