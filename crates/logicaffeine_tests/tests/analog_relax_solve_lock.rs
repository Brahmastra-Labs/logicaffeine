//! ★ RELAXATION SOLVE LOCK ★ — `Solve A·x = b` by physical settling, with the exact residual
//! certificate.
//!
//! THE TRUST COVENANT this file pins: the physics (here, its behavioral model) computes
//! UNTRUSTED — we never certify the trajectory at all — and the digital side verifies the
//! SETTLED ANSWER exactly: `r = A·x̂ − b` is re-multiplied in exact ℚ by an independent
//! proof-crate checker ([`check_relax_claim`]), `‖r‖∞ ≤ ε` declared and checked.
//!
//! What this file locks:
//!   1. ADMISSION — only systems with a cited convergence theorem settle: symmetric AND
//!      (strictly diagonally dominant with positive diagonal, OR all leading principal minors
//!      positive — Sylvester), decided over EXACT ℚ. Anything else refuses TYPED
//!      (`ConvergenceUnprovable`), never a wrong answer.
//!   2. THE FORCE NETWORK — `ẋ = b − A·x` is built from the EXISTING Sum-tile vocabulary:
//!      the tiles stop computing results and start generating the forces that move state.
//!   3. FAIL-CLOSED SETTLING — settling time is a performance knob, not a correctness one:
//!      an under-settled run has a too-large exact residual and REFUSES
//!      (`ResidualExceedsBudget`); it can never emit a certificate it does not meet.
//!   4. INDEPENDENT VERIFICATION — a tampered x̂, a shrunk ε, or a malformed claim is
//!      rejected by the proof-crate checker, which re-multiplies with its OWN arithmetic.
//!   5. HONEST NOISE — a noise-injected settle (fixed seed, deterministic forever) still
//!      certifies, with an honestly larger residual.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The admission gate only ever admits MORE provable
//! classes; the checker never learns to trust the producer.

use logicaffeine_base::numeric::{rational_from_f64_exact, Rational};
use logicaffeine_compile::codegen_ana::dynamics::{
    gradient_flow_system, solve_by_settling, SettleOpts,
};
use logicaffeine_compile::codegen_ana::{AnaLowerError, AnalogCell, AnalogTarget};
use logicaffeine_proof::relax_replay::check_relax_claim;

/// SPD and strictly diagonally dominant — the admission gate's easy yes.
const A3: [[f64; 3]; 3] = [[4.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 2.0]];
const B3: [f64; 3] = [1.0, 2.0, 3.0];

fn a3() -> Vec<Vec<f64>> {
    A3.iter().map(|r| r.to_vec()).collect()
}

fn opts_settled() -> SettleOpts {
    SettleOpts {
        steps: 20_000,
        rate: 0.1,
        eps_required: 0.01,
        noise_amp: 0.0,
        seed: 7,
    }
}

#[test]
fn a_spd_system_settles_and_the_exact_residual_certificate_checks() {
    let target = AnalogTarget::ideal();
    let out = solve_by_settling(&a3(), &B3, &target, &opts_settled()).expect("SPD + settled");
    assert_eq!(out.x_hat.len(), 3);
    assert!(
        check_relax_claim(&out.claim),
        "the independent checker must accept the honest claim"
    );
    // The true solution is (2/9, 1/9, 13/9); the readout is ADC-quantized, so compare at the
    // certificate's own resolution, not bit-exactly.
    let truth = [2.0 / 9.0, 1.0 / 9.0, 13.0 / 9.0];
    for (i, t) in truth.iter().enumerate() {
        assert!(
            (out.x_hat[i] - t).abs() < 0.01,
            "x̂[{i}] = {} vs true {t} — settling must actually solve",
            out.x_hat[i]
        );
    }
    // The declared ε IS the exact ∞-norm of the recomputed residual — nothing looser.
    let eps = out.claim.eps.to_f64();
    assert!(
        eps <= 0.01,
        "the certificate meets the demanded budget: {eps}"
    );
    assert!(
        eps > 0.0,
        "ADC quantization makes a zero residual essentially impossible: {eps}"
    );
}

#[test]
fn the_force_network_is_built_from_the_existing_tile_vocabulary() {
    let sys = gradient_flow_system(&a3(), &B3);
    assert_eq!(sys.n, 3);
    assert_eq!(sys.force.len(), 3, "one force tile per state");
    for (i, cell) in sys.force.iter().enumerate() {
        let AnalogCell::Sum { inputs, weights } = cell else {
            panic!("force cell {i} must be the EXISTING Sum tile, got {cell:?}");
        };
        assert_eq!(inputs.len(), 4, "n state taps + the b-constant leg");
        assert_eq!(weights.len(), 4);
        // ẋ_i = b_i − Σ_j a_ij·x_j: state legs carry −a_ij, the constant leg carries b_i.
        for (j, w) in weights.iter().take(3).enumerate() {
            assert_eq!(*w, -A3[i][j], "force weight [{i}][{j}]");
        }
        assert_eq!(weights[3], B3[i], "the b leg");
    }
}

#[test]
fn inadmissible_systems_refuse_typed_never_wrong() {
    let target = AnalogTarget::ideal();
    // Symmetric but indefinite (second leading minor is 1·1 − 2·2 = −3).
    let indefinite = vec![vec![1.0, 2.0], vec![2.0, 1.0]];
    match solve_by_settling(&indefinite, &[1.0, 1.0], &target, &opts_settled()) {
        Err(AnaLowerError::ConvergenceUnprovable { .. }) => {}
        other => panic!("indefinite must refuse ConvergenceUnprovable, got {other:?}"),
    }
    // Asymmetric — the gradient-flow theorem does not apply, however dominant.
    let asym = vec![vec![4.0, 1.0], vec![0.0, 4.0]];
    match solve_by_settling(&asym, &[1.0, 1.0], &target, &opts_settled()) {
        Err(AnaLowerError::ConvergenceUnprovable { .. }) => {}
        other => panic!("asymmetric must refuse, got {other:?}"),
    }
    // Malformed shapes refuse rather than panic.
    let ragged = vec![vec![1.0, 2.0], vec![3.0]];
    assert!(solve_by_settling(&ragged, &[1.0, 1.0], &target, &opts_settled()).is_err());
    assert!(solve_by_settling(&a3(), &[1.0], &target, &opts_settled()).is_err());
}

#[test]
fn an_under_settled_run_fails_closed() {
    let target = AnalogTarget::ideal();
    let hasty = SettleOpts {
        steps: 3,
        ..opts_settled()
    };
    match solve_by_settling(&a3(), &B3, &target, &hasty) {
        Err(AnaLowerError::ResidualExceedsBudget { linf, required }) => {
            assert!(
                linf > required,
                "the refusal carries the honest numbers: {linf} vs {required}"
            );
        }
        other => panic!("3 Euler steps cannot meet 1e-2 — must refuse typed, got {other:?}"),
    }
}

#[test]
fn a_tampered_claim_is_rejected_by_the_independent_checker() {
    let target = AnalogTarget::ideal();
    let out = solve_by_settling(&a3(), &B3, &target, &opts_settled()).expect("baseline");
    assert!(check_relax_claim(&out.claim));

    // A nudged solution component.
    let mut tampered = out.claim.clone();
    tampered.x_hat[0] = tampered.x_hat[0].add(&Rational::from_i64(1));
    assert!(
        !check_relax_claim(&tampered),
        "a moved x̂ must be caught by re-multiplication"
    );

    // A dishonestly shrunk ε.
    let mut shrunk = out.claim.clone();
    shrunk.eps = shrunk.eps.mul(&rational_from_f64_exact(1e-6).unwrap());
    assert!(
        !check_relax_claim(&shrunk),
        "an ε below the true residual must be caught"
    );

    // Malformed shapes fail closed.
    let mut ragged = out.claim.clone();
    ragged.b.pop();
    assert!(!check_relax_claim(&ragged));
    let mut negative = out.claim.clone();
    negative.eps = negative.eps.negated();
    assert!(!check_relax_claim(&negative));
}

#[test]
fn noise_injected_settling_still_certifies_with_an_honestly_larger_residual() {
    let target = AnalogTarget::ideal();
    let clean = solve_by_settling(&a3(), &B3, &target, &opts_settled()).expect("clean");
    let noisy_opts = SettleOpts {
        noise_amp: 5.0e-3,
        eps_required: 0.1,
        ..opts_settled()
    };
    let noisy = solve_by_settling(&a3(), &B3, &target, &noisy_opts).expect("noisy but certified");
    assert!(
        check_relax_claim(&noisy.claim),
        "an honest noisy claim still checks"
    );
    assert!(
        noisy.claim.eps > clean.claim.eps,
        "noise must show up in the residual, not vanish: noisy {} vs clean {}",
        noisy.claim.eps.to_f64(),
        clean.claim.eps.to_f64()
    );
    // Determinism: the same seed settles to the same certificate, forever.
    let again = solve_by_settling(&a3(), &B3, &target, &noisy_opts).expect("replay");
    assert_eq!(again.claim.eps, noisy.claim.eps);
    assert_eq!(again.x_hat, noisy.x_hat);
}
