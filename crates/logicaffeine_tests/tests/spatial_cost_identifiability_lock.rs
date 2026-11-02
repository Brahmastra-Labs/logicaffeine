//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! IDENTIFIABILITY LOCK — a constant the measurements cannot see may not be published as fitted, and
//! no two constants may span one column of the fit.
//!
//! `spatial_cost_model_lock` pins the cycle model's constants and requires a re-fit to reproduce them.
//! That obligation is only meaningful if the fit has a unique answer, and for a long time it did not:
//!
//!   * `c_sem_rtt` and `c_mcast_setup` appeared in the model ONLY as `(c_sem_rtt + c_mcast_setup)`.
//!     Two constants, one column — no measurement anywhere could ever separate them. They are now the
//!     single `c_mcast_block`.
//!   * `c_dispatch` was a free intercept beside `c_kernel * kernel_classes`, and `c_sem_setup` was a
//!     third setup term whose `n_sems` is a deterministic function of `kernel_classes` on every ledger
//!     row (3 classes with 0 semaphores, or 6 with 4). Three parameters spanning a two-dimensional
//!     space; two of them are gone and `c_kernel` carries what they described.
//!   * `steady` is a `max` over four branches, so a constant in a branch that never binds is INVISIBLE
//!     to the objective. `f_hifi3`/`f_hifi2`/`f_lofi` cannot move because every measured row is HiFi4,
//!     and the synchronization constants cannot move because no measured row is sync-bound — yet they
//!     were published in the fitted model as though measured.
//!
//! The observable consequence was a flat valley: re-fitting moved 17 of 19 constants (`c_dispatch`
//! 0.0004 → 2798, `c_kernel` 783 → 256) while the objective barely changed.
//!
//! ★ WHAT THIS FILE DELIBERATELY DOES NOT ASSERT, and why — a wrong turn worth recording.
//!
//! A loss-based redundancy gate was built here first: freeze a constant at half its value, refit
//! everything else, and call it undetermined if the loss recovers. Six of sixteen constants failed it.
//! Returning those six to their priors and re-pinning barely moved predicted CYCLES — and changed TEN
//! OF TWENTY of the ledger's compiler-chosen plan DECISIONS, the calibrated 512³ GEMM among them. The
//! valley is flat in loss and is not flat in the decisions taken inside it, so loss-redundancy is not
//! evidence that a constant is idle. The operational question belongs to
//! `spatial_cost_model_lock::the_synchronization_terms_are_load_bearing`: does it change a DECISION?
//!
//! So this file asserts only what is provable — visibility and structural collinearity — and reports
//! the rest. A lock that over-claims costs more than one that says less.
//!
//! NON-VACUITY PROVEN: `the_detector_catches_a_deliberately_collinear_pair` builds a model and a point
//! set in which `c_mcast_block` and `c_noc_tile` are exactly proportional, and requires the probe to
//! flag one of them — so a probe that had stopped discriminating cannot pass this file silently.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_cost_identifiability_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::cost::{quantities_for_point, CycleModel, PlanQuantities};
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::*;

/// The rows the Wormhole model is fitted against — the same set `spatial_cost_model_lock` uses.
fn fit_set() -> Vec<(PlanQuantities, u64)> {
    let wh = SpatialTarget::wormhole_ttsim();
    ledger_points()
        .iter()
        .filter(|p| p.chip == "wormhole_b0" && p.stack != "ttnn_production")
        .filter_map(|p| quantities_for_point(p, &wh).map(|q| (q, p.cycles)))
        .collect()
}

/// Can the rest of the model absorb constant `i`? Freeze it at a deliberately wrong value, let every
/// OTHER constant refit from the same starting point, and report `(baseline, best without it)`.
///
/// Start-matched on purpose: both searches begin at the SAME perturbed point, one free to move `i` back
/// and one not. Comparing a frozen search against a baseline found elsewhere reports which basin each
/// search happened to land in rather than whether the constant carries information.
fn absorbable(m: &CycleModel, pts: &[(PlanQuantities, u64)], i: usize) -> (f64, f64) {
    let vi = m.constants()[i].1;
    let held = m.with_constant(i, if vi == 0.0 { 1.0 } else { vi * 0.5 });
    let free = CycleModel::refit_from(pts, &held, &[]).loss(pts);
    (
        m.loss(pts).min(free),
        CycleModel::refit_from(pts, &held, &[i]).loss(pts),
    )
}

/// Rung 1 — THE SPECIFICATION. A constant no measured row can see is a structural guess, and the
/// published model must leave it at exactly its prior rather than dressing it as a measurement.
///
/// This is provable rather than empirical: `unexercised` changes a constant and checks whether ANY
/// point's predicted cycles respond. If none do, no amount of fitting can have determined it.
#[test]
fn every_constant_the_model_publishes_as_fitted_is_visible_to_the_measurements() {
    let pts = fit_set();
    assert!(
        pts.len() >= 40,
        "the fit set must be real: {} rows",
        pts.len()
    );
    let pinned = CycleModel::wormhole_ttsim();
    let prior = CycleModel::uncalibrated();
    let blind = pinned.unexercised(&pts);

    for (name, v) in pinned.constants() {
        if !blind.contains(&name) {
            continue;
        }
        let p = prior
            .constants()
            .into_iter()
            .find(|(n, _)| *n == name)
            .expect("same vector")
            .1;
        assert_eq!(
            v, p,
            "no measured row can see `{name}` — changing it moves no prediction — so its value cannot \
             have been measured. It must stay at the structural prior {p}, not be published as {v}. \
             Measure a row whose pace it sets and it will become fittable on its own."
        );
    }
    assert!(
        !blind.is_empty(),
        "the visibility probe found nothing at all — it has stopped discriminating"
    );
    assert!(
        blind.len() < pinned.constants().len(),
        "no constant is visible — the fit set cannot see the model"
    );
}

/// Rung 2 — the fit must be REPRODUCIBLE, from the same rows and from the same rows in another order.
/// Pinned literals are a promise that anyone re-running the fit on this ledger gets these numbers back;
/// that promise was empty while the search was a geometric ladder that stopped short of the optimum and
/// landed wherever it started.
#[test]
fn the_fit_is_reproducible_from_the_ledger() {
    let pts = fit_set();
    let a = CycleModel::fit(&pts);
    for ((n, x), (_, y)) in a
        .constants()
        .into_iter()
        .zip(CycleModel::fit(&pts).constants())
    {
        assert_eq!(
            x, y,
            "`{n}` is not deterministic: two fits of the same ledger gave {x} and {y}"
        );
    }
    let mut shuffled = pts.clone();
    shuffled.reverse();
    for ((n, x), (_, z)) in a
        .constants()
        .into_iter()
        .zip(CycleModel::fit(&shuffled).constants())
    {
        let rel = (x - z).abs() / x.abs().max(z.abs()).max(1e-12);
        assert!(
            rel < 0.01,
            "`{n}` moved {:.2}% when the SAME rows were presented in another order ({x} vs {z})",
            rel * 100.0
        );
    }
}

/// NON-VACUITY. Build a point set and a model in which `c_mcast_block` and `c_noc_tile` are exactly
/// proportional and the synchronization branch sets the pace, and require the probe to call one of them
/// absorbable. A probe that had stopped discriminating would let a genuinely collinear pair through.
#[test]
fn the_detector_catches_a_deliberately_collinear_pair() {
    let pts = fit_set();
    let (mut q, _) = pts[0];
    q.mcast_blocks = 64;
    q.mcast_tiles = 128;
    let mut m = CycleModel::uncalibrated();
    for z in [
        &mut m.c_mac,
        &mut m.c_pack,
        &mut m.c_acq,
        &mut m.c_reload,
        &mut m.c_drain,
        &mut m.c_dram_tile,
        &mut m.c_dram_shared,
        &mut m.c_cb,
    ] {
        *z = 0.0;
    }
    let rigged: Vec<(PlanQuantities, u64)> = (1..=6)
        .map(|s| {
            let mut qq = q;
            qq.mcast_blocks = 64 * s;
            qq.mcast_tiles = 128 * s;
            (qq, m.cycles(&qq) as u64)
        })
        .collect();
    let i = m
        .constants()
        .iter()
        .position(|(n, _)| *n == "c_mcast_block")
        .expect("the merged sync constant");
    let (base, recovered) = absorbable(&m, &rigged, i);
    assert!(
        recovered <= base * (1.0 + 1e-3) + 1e-9,
        "the probe FAILED to see an exactly collinear pair: baseline {base:.12}, and after halving \
         `c_mcast_block` the rest of the model still reaches only {recovered:.12}."
    );
}

/// The calibration's own edges, printed. Not assertions — a report, so a reader can see at a glance
/// which constants this ledger can see, which the rest of the model could absorb, and which a single
/// fold of the ledger would move. All three are worth knowing and none of them is a defect on its own.
#[test]
fn report_the_edges_of_this_calibration() {
    let pts = fit_set();
    let m = CycleModel::wormhole_ttsim();
    let blind = m.unexercised(&pts);
    let shaky = m.unstable(&pts);
    let soft: Vec<&'static str> = (0..m.constants().len())
        .filter(|&i| {
            let (b, r) = absorbable(&m, &pts, i);
            r <= b * (1.0 + CycleModel::DETERMINED_MARGIN) + 1e-9
        })
        .map(|i| m.constants()[i].0)
        .collect();
    println!("\n{} rows, {} constants", pts.len(), m.constants().len());
    println!("  invisible to every row      : {blind:?}");
    println!("  absorbable by the rest      : {soft:?}");
    println!(
        "  moved by a {:.0}% jackknife   : {shaky:?}",
        CycleModel::STABLE_MARGIN * 100.0
    );
    println!("  loss at the pinned model    : {:.9}", m.loss(&pts));
}
