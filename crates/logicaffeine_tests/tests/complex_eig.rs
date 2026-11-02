//! Complex eigenvalues — `cgeev` (`clapack.lg`, LAPACK `ZGEEV`, values only). Wilkinson-shifted
//! complex QR iteration (complex modified-Gram–Schmidt QR each step) drives a general complex
//! matrix to upper-triangular Schur form; the eigenvalues are the diagonal. Being iterative, the
//! values are floating-point accurate (not exact), so — exactly like the real `geev` lock — this
//! parses the returned `Seq of ComplexF` and compares the spectrum sorted, within tolerance.
//! `run_all_tiers` enforces interpreter == VM.
mod common;
use common::run_all_tiers;

/// Parse one `ComplexF` display token (`"3"`, `"-i"`, `"1+2i"`, `"3-4i"`, `"2.5-0.5i"`) → `(re, im)`.
fn parse_complex(tok: &str) -> (f64, f64) {
    let t = tok.trim();
    if !t.ends_with('i') {
        return (
            t.parse::<f64>()
                .unwrap_or_else(|_| panic!("real parse: {t:?}")),
            0.0,
        );
    }
    let body = &t[..t.len() - 1]; // strip trailing 'i'
    let bytes = body.as_bytes();
    // The imaginary part's sign = the last '+'/'-' past index 0 that isn't an exponent sign.
    let mut split = None;
    for k in (1..bytes.len()).rev() {
        let c = bytes[k] as char;
        if (c == '+' || c == '-') && bytes[k - 1] as char != 'e' && bytes[k - 1] as char != 'E' {
            split = Some(k);
            break;
        }
    }
    match split {
        Some(p) => {
            let re: f64 = body[..p]
                .parse()
                .unwrap_or_else(|_| panic!("re parse: {body:?}"));
            let im = match &body[p..] {
                "+" => 1.0,
                "-" => -1.0,
                s => s
                    .parse::<f64>()
                    .unwrap_or_else(|_| panic!("im parse: {s:?}")),
            };
            (re, im)
        }
        None => {
            // imaginary-only: `""`(=i), `"-"`(=−i), `"2"`, `"-3"`.
            let im = match body {
                "" => 1.0,
                "-" => -1.0,
                s => s
                    .parse::<f64>()
                    .unwrap_or_else(|_| panic!("im-only parse: {s:?}")),
            };
            (0.0, im)
        }
    }
}

fn parse_complex_seq(out: &str) -> Vec<(f64, f64)> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .expect("bracketed Seq");
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner.split(',').map(|s| parse_complex(s.trim())).collect()
}

fn sorted(mut v: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap()
            .then(a.1.partial_cmp(&b.1).unwrap())
    });
    v
}

fn run_eigs(n: usize, a: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let elems = a
        .iter()
        .map(|(re, im)| format!("complex({re:?}, {im:?})"))
        .collect::<Vec<_>>()
        .join(", ");
    let src = format!("## Main\nLet e be cgeev({n}, [{elems}]).\nShow e.\n");
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "cgeev errored: {:?}\n{src}", out.error);
    parse_complex_seq(&out.output)
}

fn assert_eigs_close(got: Vec<(f64, f64)>, want: &[(f64, f64)], tol: f64) {
    let g = sorted(got);
    let w = sorted(want.to_vec());
    assert_eq!(g.len(), w.len(), "spectrum size {g:?} vs {w:?}");
    for (a, b) in g.iter().zip(w.iter()) {
        let ok = (a.0 - b.0).abs() <= tol * (1.0 + b.0.abs())
            && (a.1 - b.1).abs() <= tol * (1.0 + b.1.abs());
        assert!(ok, "eigenvalue {a:?} vs {b:?} in {g:?} / {w:?}");
    }
}

#[test]
fn cgeev_diagonal_matrix_returns_the_diagonal() {
    // diag(2, 3) — a fixed point of QR.
    let a = [(2.0, 0.0), (0.0, 0.0), (0.0, 0.0), (3.0, 0.0)];
    assert_eigs_close(run_eigs(2, &a), &[(2.0, 0.0), (3.0, 0.0)], 1e-7);
}

#[test]
fn cgeev_upper_triangular_complex_spectrum() {
    // [[1+i, 2], [0, 3-i]] — eigenvalues are the diagonal {1+i, 3-i}.
    let a = [(1.0, 1.0), (2.0, 0.0), (0.0, 0.0), (3.0, -1.0)];
    assert_eigs_close(run_eigs(2, &a), &[(1.0, 1.0), (3.0, -1.0)], 1e-6);
}

#[test]
fn cgeev_real_matrix_with_a_pure_imaginary_pair() {
    // [[0, -1], [1, 0]] has eigenvalues ±i — the equal-magnitude case a plain Rayleigh shift can't
    // split; the Wilkinson shift (= ±i) breaks the symmetry and converges.
    let a = [(0.0, 0.0), (-1.0, 0.0), (1.0, 0.0), (0.0, 0.0)];
    assert_eigs_close(run_eigs(2, &a), &[(0.0, 1.0), (0.0, -1.0)], 1e-6);
}

#[test]
fn cgeev_hermitian_matrix_has_real_eigenvalues() {
    // [[2, i], [-i, 2]] is Hermitian ⇒ real spectrum {1, 3}.
    let a = [(2.0, 0.0), (0.0, 1.0), (0.0, -1.0), (2.0, 0.0)];
    assert_eigs_close(run_eigs(2, &a), &[(1.0, 0.0), (3.0, 0.0)], 1e-6);
}

#[test]
fn cgeev_3x3_upper_triangular_complex_spectrum() {
    // [[1+i, 5, 2], [0, 2, 3], [0, 0, 3-i]] ⇒ {1+i, 2, 3-i}.
    let a = [
        (1.0, 1.0),
        (5.0, 0.0),
        (2.0, 0.0),
        (0.0, 0.0),
        (2.0, 0.0),
        (3.0, 0.0),
        (0.0, 0.0),
        (0.0, 0.0),
        (3.0, -1.0),
    ];
    assert_eigs_close(
        run_eigs(3, &a),
        &[(1.0, 1.0), (2.0, 0.0), (3.0, -1.0)],
        1e-6,
    );
}
