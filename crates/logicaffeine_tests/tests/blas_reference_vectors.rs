//! LOGOS-BLAS differential lock against the netlib Fortran reference.
//!
//! `scripts/test/gen-lapack-goldens.sh` compiles the *actual* reference DGEMM (BLAS 3.12.0) with
//! `gfortran -O0 -fno-fast-math` and writes a fixture holding deterministic fractional inputs
//! AND the reference product (`tests/data/lapack/gemm_6.txt`). This test feeds those exact
//! inputs — read from the fixture, so the LOGOS side can never drift from a re-derived formula
//! — into the LOGOS `gemm`, runs it across every tier (`run_all_tiers` also enforces interp==VM),
//! and compares to netlib element-wise within a tight tolerance. LOGOS's internal tiers are
//! bit-identical to each other; the tolerance is the *external* LOGOS-vs-Fortran boundary, where
//! accumulation order may differ by a few ULP (here it happens to coincide, since the reference
//! `TEMP=B(l,j)` inner-`i` loop accumulates the same terms in the same `l`-order).

mod common;
use common::run_all_tiers;
use std::path::Path;

struct GemmFixture {
    m: usize,
    n: usize,
    k: usize,
    a: Vec<f64>,
    b: Vec<f64>,
    c_expected: Vec<f64>,
}

/// Parse the golden: a `# …`-commented header line `m n k alpha beta`, then the A, B and
/// C matrices as one full-precision f64 per line, all in logical row-major order.
fn parse_gemm_fixture(text: &str) -> GemmFixture {
    let mut rows = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let header = rows.next().expect("fixture header line");
    let mut hp = header.split_whitespace();
    let m: usize = hp.next().unwrap().parse().unwrap();
    let n: usize = hp.next().unwrap().parse().unwrap();
    let k: usize = hp.next().unwrap().parse().unwrap();
    // Fortran emits exponents as `E-001`; Rust's f64 parser accepts them verbatim.
    let vals: Vec<f64> = rows.map(|s| s.parse::<f64>().expect("f64 value")).collect();
    assert_eq!(vals.len(), m * k + k * n + m * n, "fixture value count");
    let a = vals[0..m * k].to_vec();
    let b = vals[m * k..m * k + k * n].to_vec();
    let c_expected = vals[m * k + k * n..].to_vec();
    GemmFixture {
        m,
        n,
        k,
        a,
        b,
        c_expected,
    }
}

/// Format an f64 as a LOGOS float literal: shortest round-trip, always carrying a decimal
/// point so a value like `0` is typed `Float`, not `Int`, inside a `Seq of Float` literal.
fn float_lit(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn seq_literal(xs: &[f64]) -> String {
    let inner: Vec<String> = xs.iter().map(|x| float_lit(*x)).collect();
    format!("[{}]", inner.join(", "))
}

/// Parse LOGOS's `Show`n `Seq of Float` (`[v1, v2, …]`) back into f64s.
fn parse_seq_floats(out: &str) -> Vec<f64> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or_else(|| panic!("expected a bracketed Seq, got {out:?}"));
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float element"))
        .collect()
}

#[test]
fn gemm_matches_netlib_dgemm_reference() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/lapack/gemm_6.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("missing golden {path:?} — regenerate with scripts/test/gen-lapack-goldens.sh")
    });
    let fx = parse_gemm_fixture(&text);

    let zeros = vec![0.0f64; fx.m * fx.n];
    let src = format!(
        "## Main\nShow gemm({m}, {n}, {k}, 1.0, {a}, {b}, 0.0, {c}).\n",
        m = fx.m,
        n = fx.n,
        k = fx.k,
        a = seq_literal(&fx.a),
        b = seq_literal(&fx.b),
        c = seq_literal(&zeros),
    );

    // run_all_tiers also asserts the tree-walker and VM agree byte-for-byte.
    let agreed = run_all_tiers(&src);
    assert!(agreed.error.is_none(), "gemm errored: {:?}", agreed.error);
    let got = parse_seq_floats(&agreed.output);
    assert_eq!(got.len(), fx.c_expected.len(), "result length");

    for (idx, (g, e)) in got.iter().zip(fx.c_expected.iter()).enumerate() {
        let tol = 1e-12 * (1.0 + e.abs());
        assert!(
            (g - e).abs() <= tol,
            "gemm[{idx}]: logos={g:.17e} netlib={e:.17e} diff={:.3e} tol={tol:.3e}",
            (g - e).abs()
        );
    }
}
