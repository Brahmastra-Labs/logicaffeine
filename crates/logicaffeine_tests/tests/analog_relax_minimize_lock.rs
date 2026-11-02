//! ★ RELAXATION MINIMIZE LOCK ★ — convex `Minimize` by settling, certified by THEOREM: a
//! gradient residual becomes a distance-to-optimum bound through strong convexity.
//!
//! What this file locks:
//!   1. THE BOUND IS TRUE — on a known-minimum quadratic (f* computed EXACTLY via the proof
//!      crate's own ℚ solve), the settled candidate's claimed bound really contains
//!      `f(x̂) − f*`, and the independent theorem-checker accepts the claim.
//!   2. THE CHECKER BITES — an inflated μ, a shrunken bound, or an asymmetric Q is
//!      rejected; a Q with no positive Gershgorin floor refuses at the PRODUCER (no
//!      spectral floor, no theorem, no certificate).
//!   3. VIRTUAL SPRINGS PULL — folding `λ(gᵀx − c)²` into the quadratic moves the settled
//!      answer monotonically toward the constraint surface as λ stiffens.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. `Candidate` semantics: the artifact carries a
//! bound, never an optimality flag — there is no field to lie with.

use logicaffeine_base::numeric::{rational_from_f64_exact, Rational};
use logicaffeine_compile::codegen_ana::dynamics::{minimize_by_settling, with_penalty, SettleOpts};
use logicaffeine_compile::codegen_ana::{AnaLowerError, AnalogTarget};
use logicaffeine_proof::linalg;
use logicaffeine_proof::relax_replay::check_minimize_claim;

const Q3: [[f64; 3]; 3] = [[4.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 2.0]];
const B3: [f64; 3] = [1.0, -2.0, 0.5];

fn q3() -> Vec<Vec<f64>> {
    Q3.iter().map(|r| r.to_vec()).collect()
}

fn opts() -> SettleOpts {
    SettleOpts {
        steps: 20_000,
        rate: 0.1,
        eps_required: 0.01,
        noise_amp: 0.0,
        seed: 11,
    }
}

fn f_exact(q: &[Vec<f64>], b: &[f64], x: &[Rational]) -> Rational {
    let n = b.len();
    let qr: Vec<Vec<Rational>> = q
        .iter()
        .map(|row| {
            row.iter()
                .map(|v| rational_from_f64_exact(*v).unwrap())
                .collect()
        })
        .collect();
    let br: Vec<Rational> = b
        .iter()
        .map(|v| rational_from_f64_exact(*v).unwrap())
        .collect();
    let mut acc = Rational::zero();
    for i in 0..n {
        for j in 0..n {
            acc = acc.add(&x[i].mul(&qr[i][j]).mul(&x[j]));
        }
    }
    let mut lin = Rational::zero();
    for i in 0..n {
        lin = lin.add(&br[i].mul(&x[i]));
    }
    acc.div(&Rational::from_i64(2)).unwrap().add(&lin)
}

#[test]
fn the_suboptimality_bound_really_contains_the_gap_to_the_exact_minimum() {
    let target = AnalogTarget::ideal();
    let out = minimize_by_settling(&q3(), &B3, &target, &opts()).expect("settles");
    assert!(
        check_minimize_claim(&out.claim),
        "the theorem-checker accepts the honest claim"
    );

    // The EXACT minimizer: Q·x* = −b over ℚ, solved by the proof crate itself.
    let qr: Vec<Vec<Rational>> = Q3
        .iter()
        .map(|row| {
            row.iter()
                .map(|v| rational_from_f64_exact(*v).unwrap())
                .collect()
        })
        .collect();
    let neg_b: Vec<Rational> = B3
        .iter()
        .map(|v| rational_from_f64_exact(-*v).unwrap())
        .collect();
    let x_star = linalg::solve(&qr, &neg_b).expect("SPD is nonsingular");

    let f_hat = f_exact(&q3(), &B3, &out.claim.x_hat);
    let f_star = f_exact(&q3(), &B3, &x_star);
    let gap = f_hat.sub(&f_star);
    assert!(!gap.is_negative(), "f(x̂) ≥ f* always");
    assert!(
        gap <= out.claim.subopt,
        "the claimed bound must contain the TRUE gap: gap {} vs bound {}",
        gap.to_f64(),
        out.claim.subopt.to_f64()
    );
    assert!(
        out.subopt <= 0.01,
        "and it met the demanded budget: {}",
        out.subopt
    );
}

#[test]
fn the_checker_bites_and_the_floorless_refuse() {
    let target = AnalogTarget::ideal();
    let out = minimize_by_settling(&q3(), &B3, &target, &opts()).expect("settles");

    let mut inflated = out.claim.clone();
    inflated.mu = inflated.mu.mul(&Rational::from_i64(100));
    assert!(
        !check_minimize_claim(&inflated),
        "an inflated μ must fail the Gershgorin recheck"
    );

    let mut shrunk = out.claim.clone();
    shrunk.subopt = shrunk.subopt.div(&Rational::from_i64(1_000_000)).unwrap();
    assert!(
        !check_minimize_claim(&shrunk),
        "a dishonest bound must fail the theorem inequality"
    );

    let mut warped = out.claim.clone();
    warped.q[0][1] = warped.q[0][1].add(&Rational::one());
    assert!(!check_minimize_claim(&warped), "asymmetry must be rejected");

    // Sylvester-admissible but Gershgorin-floorless: [[2,2],[2,3]] (minors 2, 2 > 0; row
    // bounds 0 and 1 — no positive uniform floor from row 0).
    let floorless = vec![vec![2.0, 2.0], vec![2.0, 3.0]];
    match minimize_by_settling(&floorless, &[1.0, 1.0], &target, &opts()) {
        Err(AnaLowerError::ConvergenceUnprovable { reason }) => {
            assert!(reason.contains("spectral floor"), "{reason}");
        }
        other => panic!("no floor ⇒ no theorem ⇒ refusal, got {other:?}"),
    }
}

#[test]
fn virtual_springs_pull_the_candidate_onto_the_constraint() {
    // A single-coordinate spring (`x₀ = c`): its rank-1 penalty lands on the diagonal, so
    // stiffening IMPROVES the Gershgorin floor and the certificate rides along. (A DENSE
    // constraint row destroys the floor and honestly refuses — the theorem has no μ; that
    // refusal is pinned in `the_checker_bites_and_the_floorless_refuse`.)
    let target = AnalogTarget::ideal();
    let g = [1.0, 0.0, 0.0];
    let c = 1.5;
    let unconstrained = minimize_by_settling(&q3(), &B3, &target, &opts()).expect("settles");
    let violation = |x: &[f64]| (x[0] - c).abs();
    let v0 = violation(&unconstrained.x_hat);

    let mut last = v0;
    for lambda in [1.0, 10.0, 100.0] {
        let (q2, b2) = with_penalty(&q3(), &B3, &g, c, lambda);
        // A stiffer spring raises λ_max ≈ q₀₀ = 4 + 2λ, and explicit Euler is stable only
        // under rate < 2/λ_max — the step size must follow the stiffness (physics, not a
        // tuning whim).
        let stiff = SettleOpts {
            rate: 0.5 / (4.0 + 2.0 * lambda),
            steps: 100_000,
            eps_required: 10.0,
            ..opts()
        };
        let out = minimize_by_settling(&q2, &b2, &target, &stiff)
            .expect("the penalized quadratic still settles");
        assert!(check_minimize_claim(&out.claim));
        let v = violation(&out.x_hat);
        assert!(
            v < last,
            "stiffer springs pull closer: λ={lambda} violation {v} vs previous {last}"
        );
        last = v;
    }
    assert!(
        last < 0.05,
        "at λ=100 the candidate sits essentially on the constraint: {last}"
    );
}
