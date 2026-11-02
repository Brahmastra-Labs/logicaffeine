//! ★ ANALOG ISING LOCK ★ — the attempted lowering: soft-spin relaxation returns a CANDIDATE,
//! certified for exactly what it is — its exact energy and its exact 1-flip stability —
//! and the claim type cannot say more.
//!
//! What this file locks:
//!   1. CANDIDACY, CERTIFIED — on pinned small instances the candidate's claim passes the
//!      independent exact checker: recomputed energy matches, 1-flip stability holds.
//!   2. THE GAP, REPORTED HONESTLY — where brute force can compute the true ground energy
//!      (n ≤ 10 ⇒ ≤ 1024 states, exact), the lock states the candidate-vs-ground gap. On
//!      the easy ferromagnet the attractor IS the ground state; frustrated instances may
//!      land higher, and the assertion is CANDIDACY — never optimality.
//!   3. TAMPER — a wrong energy, a false stability flag, asymmetric J, or a fake spin value
//!      is rejected by the checker.
//!   4. DETERMINISM — same seed, same candidate, forever.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a better anneal lands a lower attractor, the
//! pinned energies may go DOWN — never up.

use logicaffeine_base::numeric::Rational;
use logicaffeine_compile::codegen_ana::dynamics::{ising_by_soft_spin, SettleOpts};
use logicaffeine_proof::relax_replay::check_ising_claim;

fn opts() -> SettleOpts {
    SettleOpts {
        steps: 4000,
        rate: 0.2,
        eps_required: f64::INFINITY,
        noise_amp: 0.3,
        seed: 42,
    }
}

/// Exact brute-force ground energy over all 2ⁿ states (f64 J/h entries are small integers
/// here, so f64 accumulation is exact).
fn ground_energy(j: &[Vec<f64>], h: &[f64]) -> f64 {
    let n = h.len();
    let mut best = f64::INFINITY;
    for mask in 0u32..(1 << n) {
        let s = |i: usize| if mask & (1 << i) != 0 { 1.0 } else { -1.0 };
        let mut e = 0.0;
        for i in 0..n {
            for k in (i + 1)..n {
                e -= j[i][k] * s(i) * s(k);
            }
            e -= h[i] * s(i);
        }
        best = best.min(e);
    }
    best
}

/// A 6-spin ring ferromagnet with a symmetry-breaking field — the easy instance.
fn ferromagnet() -> (Vec<Vec<f64>>, Vec<f64>) {
    let n = 6;
    let mut j = vec![vec![0.0; n]; n];
    for i in 0..n {
        let k = (i + 1) % n;
        j[i][k] = 1.0;
        j[k][i] = 1.0;
    }
    let h = vec![0.25; n];
    (j, h)
}

/// A frustrated 5-spin instance (odd antiferromagnetic ring): no state satisfies every bond.
fn frustrated() -> (Vec<Vec<f64>>, Vec<f64>) {
    let n = 5;
    let mut j = vec![vec![0.0; n]; n];
    for i in 0..n {
        let k = (i + 1) % n;
        j[i][k] = -1.0;
        j[k][i] = -1.0;
    }
    (j, vec![0.0; n])
}

#[test]
fn the_easy_ferromagnet_lands_the_ground_state_and_certifies() {
    let (j, h) = ferromagnet();
    let out = ising_by_soft_spin(&j, &h, &opts(), 4.0).expect("settles");
    assert!(
        check_ising_claim(&out.claim),
        "the exact checker accepts the honest candidate"
    );
    assert!(out.flip_stable);
    assert_eq!(
        out.spins,
        vec![1; 6],
        "the field breaks the symmetry toward all-up"
    );
    let ground = ground_energy(&j, &h);
    assert_eq!(
        out.energy, ground,
        "on the easy instance the attractor IS the ground state"
    );
}

#[test]
fn the_frustrated_ring_returns_a_certified_candidate_and_the_gap_is_reported() {
    let (j, h) = frustrated();
    let out = ising_by_soft_spin(&j, &h, &opts(), 4.0).expect("settles");
    assert!(
        check_ising_claim(&out.claim),
        "candidacy certifies even under frustration"
    );
    assert!(out.flip_stable, "greedy polish guarantees 1-flip stability");
    let ground = ground_energy(&j, &h);
    // HONESTY: the assertion is candidacy + the measured gap, never optimality. An odd
    // antiferromagnetic ring's ground energy is −3 (one frustrated bond); every 1-flip-stable
    // state sits at −3 or −1.
    println!(
        "frustrated ring: candidate {} vs ground {ground}",
        out.energy
    );
    assert!(
        out.energy >= ground,
        "no candidate beats the exact ground energy"
    );
    assert!(
        out.energy <= ground + 2.0,
        "a 1-flip-stable state of this ring is within one frustration step: {} vs {ground}",
        out.energy
    );

    // Determinism: the same seed lands the same attractor, forever.
    let again = ising_by_soft_spin(&j, &h, &opts(), 4.0).expect("replay");
    assert_eq!(again.spins, out.spins);
    assert_eq!(again.claim, out.claim);
}

#[test]
fn every_tamper_is_rejected_by_the_exact_checker() {
    let (j, h) = ferromagnet();
    let out = ising_by_soft_spin(&j, &h, &opts(), 4.0).expect("settles");

    let mut cooked = out.claim.clone();
    cooked.energy = cooked.energy.sub(&Rational::one());
    assert!(
        !check_ising_claim(&cooked),
        "a cooked energy must be caught by recomputation"
    );

    let mut unstable = out.claim.clone();
    unstable.spins[0] = -unstable.spins[0];
    assert!(
        !check_ising_claim(&unstable),
        "flipping a spin breaks BOTH the energy and (here) the stability claim"
    );

    let mut asym = out.claim.clone();
    asym.j[0][1] = asym.j[0][1].add(&Rational::one());
    assert!(
        !check_ising_claim(&asym),
        "asymmetric J is not an Ising instance"
    );

    let mut fake = out.claim.clone();
    fake.spins[2] = 0;
    assert!(!check_ising_claim(&fake), "a spin is ±1, nothing else");
}
