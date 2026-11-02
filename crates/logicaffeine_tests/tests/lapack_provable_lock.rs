//! Provable linear algebra, end to end: the LOGOS stdlib *runs* a solve in floating point; the proof
//! engine *proves* the same solve exactly, as a KERNEL-CHECKED theorem.
//!
//! `gesv` (the stdlib LU solve, `assets/std/lapack.lg`) returns a float `x` on every tier — it RUNS.
//! `logicaffeine_proof::linalg_certify::certify_system` returns the EXACT `x` over ℚ together with a
//! kernel proof of `Aᵢ·x = bᵢ` for every row and a kernel proof that `det A ≠ 0` — it PROVES. This
//! lock ties the two: the float answer agrees with the exact one, and the exact one is verified by
//! the kernel (the same judge as the Gödel/Chaitin theorems). On the ill-conditioned Hilbert system,
//! floating point can only approximate; the exact certificate is identically zero residual, proven.

mod common;
use common::run_all_tiers;
use logicaffeine_base::Rational;
use logicaffeine_proof::linalg;
use logicaffeine_proof::linalg_certify::{certify_system, ring_context};

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
/// The LOGOS stdlib float solve, cross-tier (interp == VM).
fn logos_gesv(n: usize, a: &[f64], b: &[f64]) -> Vec<f64> {
    let src = format!(
        "## Main\nShow gesv({n}, {}, {}).\n",
        seq_literal(a),
        seq_literal(b)
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "gesv errored: {:?}", out.error);
    parse_seq_floats(&out.output)
}

fn ri(x: i64) -> Rational {
    Rational::from_i64(x)
}
fn rr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}
fn to_floats(v: &[Rational]) -> Vec<f64> {
    v.iter().map(|r| r.to_f64()).collect()
}
fn flat(m: &[Vec<Rational>]) -> Vec<f64> {
    m.iter().flatten().map(|r| r.to_f64()).collect()
}

#[test]
fn logos_runs_the_solve_and_the_engine_proves_it() {
    // A tridiagonal-ish SPD-flavoured 3×3 with an exact integer solution.
    let a = vec![
        vec![ri(2), ri(1), ri(0)],
        vec![ri(1), ri(3), ri(1)],
        vec![ri(0), ri(1), ri(2)],
    ];
    let x_true = vec![ri(1), ri(2), ri(3)];
    let b = linalg::matvec(&a, &x_true);

    // RUN: the LOGOS stdlib gesv, in floating point, on every tier.
    let x_float = logos_gesv(3, &flat(&a), &to_floats(&b));

    // PROVE: the proof engine, exactly, kernel-checked.
    let ctx = ring_context();
    let cert = certify_system(&ctx, &a, &b).expect("kernel-certified unique solution");

    // the exact answer is the true one, and float agrees with it (runs) — but only exact is proven.
    assert_eq!(cert.x, x_true, "exact solve is the true solution");
    let x_exact_f = to_floats(&cert.x);
    for (f, e) in x_float.iter().zip(&x_exact_f) {
        assert!((f - e).abs() < 1e-9, "float gesv {f} agrees with exact {e}");
    }
    assert_eq!(
        cert.solve_proofs.len(),
        3,
        "one kernel proof per row of A·x=b"
    );
    assert!(!cert.det.is_zero(), "det≠0 kernel proof accompanies it");
}

#[test]
fn hilbert_float_approximates_but_the_exact_solve_is_proven() {
    // The Hilbert system is the textbook ill-conditioned case: float gesv gets close but carries a
    // nonzero residual; the exact certificate has identically-zero residual and is kernel-verified.
    let n = 4;
    let h: Vec<Vec<Rational>> = (0..n)
        .map(|i| (0..n).map(|j| rr(1, (i + j + 1) as i64)).collect())
        .collect();
    let x_true = vec![ri(1), ri(1), ri(1), ri(1)];
    let b = linalg::matvec(&h, &x_true);

    // RUN — float, cross-tier.
    let x_float = logos_gesv(n, &flat(&h), &to_floats(&b));
    // float residual r = H·x_float − b, measured in f64 — generally NOT identically zero.
    let hf = flat(&h);
    let bf = to_floats(&b);
    let mut max_resid = 0.0f64;
    for i in 0..n {
        let dot: f64 = (0..n).map(|j| hf[i * n + j] * x_float[j]).sum();
        max_resid = max_resid.max((dot - bf[i]).abs());
    }

    // PROVE — exact, kernel-verified, identically zero residual.
    let ctx = ring_context();
    let cert =
        certify_system(&ctx, &h, &b).expect("Hilbert has a kernel-certified unique solution");
    assert_eq!(cert.x, x_true, "exact Hilbert solve is [1,1,1,1]");
    assert!(
        linalg::residual_is_zero(&h, &cert.x, &b),
        "exact residual is IDENTICALLY zero"
    );
    assert_eq!(cert.solve_proofs.len(), n);
    // float agrees to a few ULPs but is only approximate — that's the whole point of proving.
    for (f, e) in x_float.iter().zip(to_floats(&cert.x)) {
        assert!(
            (f - e).abs() < 1e-4,
            "float in the neighborhood of the exact answer"
        );
    }
    // the demonstration: float carries a residual (proof would be vacuous if it were exact).
    // (We don't assert max_resid > 0 — some tiers may round to 0 — but surface it for the record.)
    eprintln!("Hilbert-4 float max |H·x−b| = {max_resid:e}; exact residual = 0 (kernel-proven)");
}
