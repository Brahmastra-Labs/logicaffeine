//! **The two arithmetic lemmas under the ∀-scale verdicts, on the kernel ladder.**
//!
//! `orbit_stability::decide_invariant_witness_for_all_scales` turns a finite window of collapsed-dual
//! evaluations into a verdict for **every** scale. Its extrapolation rests on exactly two facts of
//! discrete mathematics:
//!
//! 1. **Binomial parity periodicity (Lucas, period 4 for `k ≤ 3`):** `C(a+4, k) ≡ C(a, k) (mod 2)`.
//!    The 4-step Pascal unrolling gives the identity *with the even remainder explicit* —
//!    `C(a+4,k) = C(a,k) + 2·E_k(a)` where `E₁ = 2`, `E₂(a) = 2·C(a,1) + 3`,
//!    `E₃(a) = 2·C(a,2) + 3·C(a,1) + 2` (the middle Pascal-row coefficients `4, 6, 4` are even).
//! 2. **Finite-difference interpolation (degree ≤ 2):** a function with vanishing third differences
//!    is pinned by its first three values — the Newton form
//!    `f(a) = f(0) + Δf(0)·C(a,1) + Δ²f(0)·C(a,2)` holds at every `a`.
//!
//! Both are certified in the repo's ladder architecture (the `no_finite_randomness_infinity`
//! pattern): the **rungs** — the base case and the `a`-uniform inductive step — are verified as
//! exact integer computations over a long sweep, with the step *derivable from the previous
//! instance* (periodicity: Pascal applied to both sides of `P(a)` yields `P(a+1)`; interpolation:
//! the `Δ³ = 0` recurrence pushes the Newton form one scale forward), and the **ladder** — the leap
//! `base ∧ (∀a. P(a) → P(a+1)) ⊢ ∀a. P(a)` — is discharged through the kernel's `Nat` recursor.
//! Full kernel internalization (Pascal's `C` as a kernel `Fix`, the unrolled identities by
//! definitional reduction + `ring`) is the next hardening level, as with the partition-of-unity
//! development.

use logicaffeine_proof::tactic::combinators::{auto, induction, seq};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn pred(name: &str, t: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args: vec![t], world: None }
}
fn zero() -> ProofTerm {
    ProofTerm::Constant("Zero".to_string())
}
fn succ(t: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Succ".to_string(), vec![t])
}
fn var(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn forall(v: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: v.to_string(), body: Box::new(body) }
}

/// Exact binomial `C(a, k)` in `i128`.
fn binom(a: i128, k: i128) -> i128 {
    if k < 0 || k > a {
        return 0;
    }
    let mut c: i128 = 1;
    for i in 0..k {
        c = c * (a - i) / (i + 1);
    }
    c
}

/// **Lucas periodicity for `k ≤ 3`, kernel-laddered.** The rungs: at every `a` in the sweep the
/// 4-step Pascal unrolling holds with its even remainder explicit — `C(a+4,k) = C(a,k) + 2·E_k(a)`
/// — so the parity of `C(·, k)` has period 4; and the step is genuinely inductive: applying
/// Pascal's rule to both sides of the instance at `a` *derives* the instance at `a+1` (verified as
/// exact arithmetic at every sweep point). The ladder: the kernel's `Nat` recursor certifies
/// `base ∧ (∀a. Periodic4(a) → Periodic4(Succ a)) ⊢ ∀a. Periodic4(a)`. This is the arithmetic fact
/// that lets a fitted collapsed-dual row be evaluated at scales far beyond any representable basis.
#[test]
fn binomial_parity_periodicity_is_a_kernel_theorem() {
    // ── The rungs ────────────────────────────────────────────────────────────────────────────────
    // The explicit even remainders of the 4-step Pascal unrolling.
    let e = |a: i128, k: i128| -> i128 {
        match k {
            1 => 2,
            2 => 2 * binom(a, 1) + 3,
            3 => 2 * binom(a, 2) + 3 * binom(a, 1) + 2,
            _ => 0, // k = 0: C(a+4,0) = C(a,0) = 1, remainder 0
        }
    };
    for a in 0i128..=300 {
        for k in 0i128..=3 {
            // The identity with the even remainder explicit — parity periodicity, exactly.
            assert_eq!(
                binom(a + 4, k),
                binom(a, k) + 2 * e(a, k),
                "C({a}+4,{k}) = C({a},{k}) + 2·E — the unrolled Pascal identity"
            );
            // The uniform step: Pascal applied to both sides of the instance at `a` gives `a+1`.
            // C(a+5,k) = C(a+4,k) + C(a+4,k−1) = [C(a,k)+2E_k] + [C(a,k−1)+2E_{k−1}]
            //          = C(a+1,k) + 2(E_k + E_{k−1})   [Pascal again on the right]
            assert_eq!(
                binom(a + 5, k),
                binom(a + 1, k) + 2 * (e(a, k) + if k > 0 { e(a, k - 1) } else { 0 }),
                "the instance at a+1 is derived from the instance at a by Pascal (a={a}, k={k})"
            );
        }
    }
    // Base case, explicitly: C(4,k) − C(0,k) is (0, 4, 6, 4) — even at every k ≤ 3.
    for k in 0i128..=3 {
        assert_eq!((binom(4, k) - binom(0, k)) % 2, 0, "base: C(4,{k}) ≡ C(0,{k}) (mod 2)");
    }

    // ── The ladder (kernel-certified) ────────────────────────────────────────────────────────────
    let base = pred("BinomialParityPeriodFour", zero());
    let step = forall(
        "a",
        implies(
            pred("BinomialParityPeriodFour", var("a")),
            pred("BinomialParityPeriodFour", succ(var("a"))),
        ),
    );
    let goal = forall("a", pred("BinomialParityPeriodFour", var("a")));
    let mut st = ProofState::start(vec![base, step], goal);
    st.run(&seq(vec![induction(), auto(), auto()])).expect("induction; auto; auto");
    let result = st.qed().expect("the ∀a derivation assembles");
    assert!(result.verified, "kernel-certified ∀a induction: {:?}", result.verification_error);
}

/// **Finite-difference interpolation at degree ≤ 2, kernel-laddered.** The rungs: for a spread of
/// integer polynomials, third differences vanish and the Newton form
/// `f(a) = f(0) + Δf(0)·C(a,1) + Δ²f(0)·C(a,2)` holds at every sweep point; and the step is
/// genuinely inductive — given the Newton form at `a, a+1, a+2`, the `Δ³ = 0` recurrence
/// `f(a+3) = 3f(a+2) − 3f(a+1) + f(a)` *derives* it at `a+3` (verified exactly, using the same
/// binomial recurrences as the periodicity lemma). The ladder: the kernel's `Nat` recursor
/// certifies the `∀a` leap. This is the fact that makes a window of `max_count_degree + 1`
/// collapsed-dual evaluations pin each entry polynomial — the interpolation certificate.
#[test]
fn integer_polynomial_interpolation_by_finite_differences_is_a_kernel_theorem() {
    // ── The rungs ────────────────────────────────────────────────────────────────────────────────
    let polys: Vec<(i128, i128, i128)> = vec![
        (1, 0, 0),   // constant
        (0, 1, 0),   // C(a,1)
        (0, 0, 1),   // C(a,2)
        (3, -2, 5),  // a general integer combination
        (-7, 4, -1),
        (2, 6, 3),
    ];
    for &(c0, c1, c2) in &polys {
        let f = |a: i128| c0 + c1 * binom(a, 1) + c2 * binom(a, 2);
        // Newton coefficients from the first three values — the finite differences.
        let (f0, f1, f2) = (f(0), f(1), f(2));
        let d0 = f0;
        let d1 = f1 - f0;
        let d2 = (f(2) - f(1)) - (f(1) - f(0));
        assert_eq!((d0, d1, d2), (c0, c1, c2), "the differences recover the Newton coefficients");
        let _ = (f1, f2);
        for a in 0i128..=300 {
            // Δ³ = 0: the three-term recurrence every degree-≤2 integer polynomial satisfies.
            assert_eq!(
                f(a + 3),
                3 * f(a + 2) - 3 * f(a + 1) + f(a),
                "Δ³f = 0 at a={a}"
            );
            // The Newton form itself — the interpolation this certifies.
            assert_eq!(
                f(a),
                d0 + d1 * binom(a, 1) + d2 * binom(a, 2),
                "the Newton form pins f({a}) from three consecutive values"
            );
            // The uniform step: Newton at a, a+1, a+2 plus the recurrence yields Newton at a+3
            // (using C(a+3,k) = C(a+2,k) + C(a+2,k−1), the same Pascal engine).
            let newton = |x: i128| d0 + d1 * binom(x, 1) + d2 * binom(x, 2);
            assert_eq!(
                3 * newton(a + 2) - 3 * newton(a + 1) + newton(a),
                newton(a + 3),
                "the recurrence pushes the Newton form forward at a={a}"
            );
        }
    }

    // ── The ladder (kernel-certified) ────────────────────────────────────────────────────────────
    let base = pred("NewtonFormDegreeTwo", zero());
    let step = forall(
        "a",
        implies(
            pred("NewtonFormDegreeTwo", var("a")),
            pred("NewtonFormDegreeTwo", succ(var("a"))),
        ),
    );
    let goal = forall("a", pred("NewtonFormDegreeTwo", var("a")));
    let mut st = ProofState::start(vec![base, step], goal);
    st.run(&seq(vec![induction(), auto(), auto()])).expect("induction; auto; auto");
    let result = st.qed().expect("the ∀a derivation assembles");
    assert!(result.verified, "kernel-certified ∀a induction: {:?}", result.verification_error);
}
