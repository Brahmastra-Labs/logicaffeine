//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! DOMINANCE / TOGGLE LOCK — the cross-cutting change policy for the whole grind.
//!
//! Every new lowering ships ALONGSIDE the old, both selectable via the `FabricTarget` dial. A new path
//! becomes the DEFAULT only if it STRICTLY DOMINATES the old — correct AND no-worse on every axis (energy,
//! area, cycles, Fmax) with at least one strictly better. Otherwise BOTH stay live toggles, characterized by
//! which axis each wins. "Strictly better ⇒ default always; else ⇒ a proven toggle." This lock pins that
//! policy and self-tests it (a fake regression must NOT flip the default).
//!
//! Run: `cargo test -p logicaffeine-tests --test dominance_toggle_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[derive(Clone, Copy, Debug)]
struct Metrics {
    correct: bool, // ≡VM — non-negotiable
    energy_per_mac_pj: f64,
    area_mm2: f64,
    cycles: f64,
    fmax_mhz: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    /// The new path strictly dominates — make it the default (always).
    DefaultNew,
    /// Neither dominates — keep both as characterized toggles.
    KeepBothToggle,
}

/// The change policy: DefaultNew iff `new` is correct AND ≤ `old` on every cost axis (energy, area, cycles)
/// AND ≥ on Fmax, with at least one axis strictly better, and no regression on any. Else KeepBothToggle.
fn policy(new: Metrics, old: Metrics) -> Choice {
    if !new.correct {
        return Choice::KeepBothToggle; // an incorrect path can never be the default
    }
    let no_worse = new.energy_per_mac_pj <= old.energy_per_mac_pj
        && new.area_mm2 <= old.area_mm2
        && new.cycles <= old.cycles
        && new.fmax_mhz >= old.fmax_mhz;
    let strictly_better = new.energy_per_mac_pj < old.energy_per_mac_pj
        || new.area_mm2 < old.area_mm2
        || new.cycles < old.cycles
        || new.fmax_mhz > old.fmax_mhz;
    if no_worse && strictly_better {
        Choice::DefaultNew
    } else {
        Choice::KeepBothToggle
    }
}

#[test]
fn strictly_better_flips_the_default() {
    let old = Metrics {
        correct: true,
        energy_per_mac_pj: 8.0,
        area_mm2: 0.4,
        cycles: 82.0,
        fmax_mhz: 100.0,
    };
    let new = Metrics {
        correct: true,
        energy_per_mac_pj: 4.0,
        area_mm2: 0.3,
        cycles: 1.0,
        fmax_mhz: 230.0,
    };
    assert_eq!(
        policy(new, old),
        Choice::DefaultNew,
        "a strictly-dominant new path must become the default"
    );
}

#[test]
fn a_mixed_tradeoff_keeps_both_as_toggles() {
    // weight-stationary on a TINY array: wins cycles, but costs area — neither dominates → both stay toggles.
    let reduction = Metrics {
        correct: true,
        energy_per_mac_pj: 8.0,
        area_mm2: 0.02,
        cycles: 82.0,
        fmax_mhz: 100.0,
    };
    let weight_stationary = Metrics {
        correct: true,
        energy_per_mac_pj: 8.0,
        area_mm2: 0.30,
        cycles: 1.0,
        fmax_mhz: 100.0,
    };
    assert_eq!(
        policy(weight_stationary, reduction),
        Choice::KeepBothToggle,
        "a path that wins one axis but loses another must NOT auto-replace — keep both toggles"
    );
}

#[test]
fn a_regression_never_flips_the_default() {
    // A "new" path that is worse on energy (a regression) must NEVER become the default, even if correct.
    let old = Metrics {
        correct: true,
        energy_per_mac_pj: 8.0,
        area_mm2: 0.3,
        cycles: 1.0,
        fmax_mhz: 230.0,
    };
    let regressed = Metrics {
        correct: true,
        energy_per_mac_pj: 9.0,
        ..old
    };
    assert_eq!(
        policy(regressed, old),
        Choice::KeepBothToggle,
        "a regression must not flip the default"
    );
}

#[test]
fn an_incorrect_path_is_never_the_default() {
    // Correctness is non-negotiable: a faster/smaller but INCORRECT path can never be the default.
    let old = Metrics {
        correct: true,
        energy_per_mac_pj: 8.0,
        area_mm2: 0.3,
        cycles: 82.0,
        fmax_mhz: 100.0,
    };
    let fast_but_wrong = Metrics {
        correct: false,
        energy_per_mac_pj: 1.0,
        area_mm2: 0.1,
        cycles: 1.0,
        fmax_mhz: 300.0,
    };
    assert_eq!(
        policy(fast_but_wrong, old),
        Choice::KeepBothToggle,
        "an incorrect path can never default"
    );
    println!("dominance/toggle policy: strictly-better⇒default, mixed⇒toggle, regression/incorrect⇒never default");
}
