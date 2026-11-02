//! ★ THERMAL RESIDUAL LOCK ★ — the independent thermal-map judge, proven on hand-solvable
//! networks and tamper probes (the proof-crate side).
//!
//! The 2-node network here is solvable on paper, so the lock knows the TRUE answer without
//! trusting any producer: node 0 (1 W injected) — g=2 W/K — node 1 — ground 4 W/K.
//! Balance: 2·(ΔT₀−ΔT₁) = 1 and 2·(ΔT₁−ΔT₀) + 4·ΔT₁ = 0 ⇒ ΔT₁ = 1/4, ΔT₀ = 3/4.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The checker never learns to trust a producer; the
//! equality is exact, not ε-loose.

use logicaffeine_base::numeric::Rational;
use logicaffeine_proof::thermal_replay::{check_thermal_claim, ThermalClaim};

fn r(n: i64, d: i64) -> Rational {
    Rational::from_i64(n).div(&Rational::from_i64(d)).unwrap()
}

fn true_claim() -> ThermalClaim {
    ThermalClaim {
        n: 2,
        edges: vec![(0, 1, r(2, 1))],
        grounds: vec![(1, r(4, 1))],
        injections: vec![(0, r(1, 1))],
        delta_t: vec![r(3, 4), r(1, 4)],
    }
}

#[test]
fn the_hand_solved_network_certifies_and_names_its_hotspot() {
    let v = check_thermal_claim(&true_claim()).expect("the paper solution certifies");
    assert_eq!(v.hottest_node, 0, "the injected node is the hotspot");
    assert_eq!(v.hottest_delta_t, r(3, 4));
}

#[test]
fn every_tamper_is_rejected() {
    // A nudged temperature.
    let mut warm = true_claim();
    warm.delta_t[0] = r(7, 8);
    assert!(
        check_thermal_claim(&warm).is_none(),
        "a false map must not certify"
    );

    // A phantom injection the temperatures don't account for.
    let mut phantom = true_claim();
    phantom.injections.push((1, r(1, 2)));
    assert!(check_thermal_claim(&phantom).is_none());

    // A deleted edge (the balance breaks at both endpoints).
    let mut cut = true_claim();
    cut.edges.clear();
    assert!(check_thermal_claim(&cut).is_none());

    // A dropped ground: no steady state exists at all.
    let mut adrift = true_claim();
    adrift.grounds.clear();
    assert!(check_thermal_claim(&adrift).is_none());
}

#[test]
fn malformed_and_unphysical_claims_fail_closed() {
    // Non-positive conductance.
    let mut bad_g = true_claim();
    bad_g.edges[0].2 = Rational::zero();
    assert!(check_thermal_claim(&bad_g).is_none());

    // Negative injection (a heat sink is a boundary, not a negative source).
    let mut sink = true_claim();
    sink.injections[0].1 = r(-1, 1);
    assert!(check_thermal_claim(&sink).is_none());

    // A node below ambient under non-negative injection.
    let mut cold = true_claim();
    cold.delta_t[1] = r(-1, 4);
    assert!(check_thermal_claim(&cold).is_none());

    // A disconnected island (node 2 floats — its heat has nowhere to go).
    let island = ThermalClaim {
        n: 3,
        edges: vec![(0, 1, r(2, 1))],
        grounds: vec![(1, r(4, 1))],
        injections: vec![(0, r(1, 1)), (2, r(1, 1))],
        delta_t: vec![r(3, 4), r(1, 4), Rational::zero()],
    };
    assert!(check_thermal_claim(&island).is_none());

    // Shape garbage.
    let mut ragged = true_claim();
    ragged.delta_t.pop();
    assert!(check_thermal_claim(&ragged).is_none());
    let mut oob = true_claim();
    oob.edges[0].1 = 9;
    assert!(check_thermal_claim(&oob).is_none());
}
