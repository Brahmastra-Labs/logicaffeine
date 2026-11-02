//! LOGOS complex-LAPACK differential lock against the reference ZGESV / ZPOTRF / ZGEEV.
//!
//! `scripts/test/gen-lapack-goldens.sh` links a Fortran driver against the system reference LAPACK,
//! applies each routine to deterministic complex inputs, and writes fixtures holding BOTH the
//! inputs AND the reference outputs (`tests/data/lapack/{zgesv,zpotrf,zgeev}.txt`, each complex
//! value as two full-precision f64 lines, row-major). This test feeds those exact inputs — read
//! from the fixture, so the LOGOS side can never drift from a re-derived formula — into the LOGOS
//! `cgesv` / `cpotrf` / `cgeev`, runs across every tier (`run_all_tiers` also enforces interp==VM),
//! and compares to the reference within tolerance (the external LOGOS-vs-Fortran boundary, where
//! pivot order / iteration path may differ by a few ULP; the eigenvalue spectrum is order-free so
//! it is compared sorted).
mod common;
use common::run_all_tiers;
use std::path::Path;

/// Read a fixture: skip `#` comments, first data line is `n`, the rest are f64s (re,im interleaved).
fn fixture(name: &str) -> (usize, Vec<f64>) {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/lapack")
        .join(name);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
    let mut it = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let n: usize = it.next().expect("n line").parse().expect("n");
    let vals: Vec<f64> = it.map(|s| s.parse::<f64>().expect("f64 value")).collect();
    (n, vals)
}

/// `[re0, im0, re1, im1, …]` → a LOGOS `[complex(re, im), …]` literal (full-precision round-trip).
fn complex_lits(pairs: &[f64]) -> String {
    let inner: Vec<String> = pairs
        .chunks(2)
        .map(|c| format!("complex({:?}, {:?})", c[0], c[1]))
        .collect();
    format!("[{}]", inner.join(", "))
}

/// Parse one `ComplexF` display token (`"3"`, `"-i"`, `"1+2i"`, `"3-4i"`) → `(re, im)`.
fn parse_complex(tok: &str) -> (f64, f64) {
    let t = tok.trim();
    if !t.ends_with('i') {
        return (
            t.parse::<f64>()
                .unwrap_or_else(|_| panic!("real parse: {t:?}")),
            0.0,
        );
    }
    let body = &t[..t.len() - 1];
    let bytes = body.as_bytes();
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

fn run_complex_seq(src: &str) -> Vec<(f64, f64)> {
    let out = run_all_tiers(src);
    assert!(out.error.is_none(), "errored: {:?}\n{src}", out.error);
    parse_complex_seq(&out.output)
}

fn pairs(v: &[f64]) -> Vec<(f64, f64)> {
    v.chunks(2).map(|c| (c[0], c[1])).collect()
}

fn assert_close(got: &[(f64, f64)], want: &[(f64, f64)], tol: f64) {
    assert_eq!(got.len(), want.len(), "length {got:?} vs {want:?}");
    for (a, b) in got.iter().zip(want.iter()) {
        let ok = (a.0 - b.0).abs() <= tol * (1.0 + b.0.abs())
            && (a.1 - b.1).abs() <= tol * (1.0 + b.1.abs());
        assert!(ok, "{a:?} vs {b:?}");
    }
}

fn sorted(mut v: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap()
            .then(a.1.partial_cmp(&b.1).unwrap())
    });
    v
}

#[test]
fn cgesv_matches_reference_zgesv() {
    let (n, vals) = fixture("zgesv.txt");
    let nn = 2 * n * n;
    let a = &vals[0..nn];
    let b = &vals[nn..nn + 2 * n];
    let x_ref = pairs(&vals[nn + 2 * n..nn + 4 * n]);
    let src = format!(
        "## Main\nShow cgesv({n}, {}, {}).\n",
        complex_lits(a),
        complex_lits(b)
    );
    assert_close(&run_complex_seq(&src), &x_ref, 1e-9);
}

#[test]
fn cpotrf_matches_reference_zpotrf() {
    let (n, vals) = fixture("zpotrf.txt");
    let nn = 2 * n * n;
    let a = &vals[0..nn];
    let l_ref = pairs(&vals[nn..2 * nn]);
    let src = format!("## Main\nShow cpotrf({n}, {}).\n", complex_lits(a));
    assert_close(&run_complex_seq(&src), &l_ref, 1e-9);
}

#[test]
fn cgeev_matches_reference_zgeev() {
    let (n, vals) = fixture("zgeev.txt");
    let nn = 2 * n * n;
    let a = &vals[0..nn];
    let w_ref = pairs(&vals[nn..nn + 2 * n]);
    let src = format!("## Main\nShow cgeev({n}, {}).\n", complex_lits(a));
    // The spectrum is order-free: sort both sides before comparing.
    assert_close(&sorted(run_complex_seq(&src)), &sorted(w_ref), 1e-6);
}
