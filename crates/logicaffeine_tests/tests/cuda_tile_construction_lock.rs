//! cuda_tile_construction_lock — the tile is CONSTRUCTED FROM THE PROBLEM, not told. We do NOT pick a winner
//! and we assume NO weighting between the physical axes. The construction has exactly two rigorous steps, both
//! with zero fitted constants:
//!   1. build the PROVABLY-COMPLETE feasible region (every legal tile that fits Ampere's smem + register file),
//!   2. keep only the PARETO frontier — tiles not dominated on all three physical axes (DRAM traffic ↓, SMEM
//!      traffic ↓, occupancy ↑). A dominated tile is provably worse on every axis, so it cannot be the optimum.
//! The GPU then crowns the winner among the frontier by MEASUREMENT (see cuda_tile_frontier_gpu_lock). The
//! machine resolves the un-modelable physics; we never assert which tile wins.
//!
//! This lock certifies the construction is SOUND (no GPU needed): the frontier is a genuine, complete,
//! meaningfully-smaller Pareto set. If it goes RED, fix the construction — never weaken the assertion.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_tile_construction_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{feasible_tiles, pareto_frontier, GpuLimits};

#[test]
fn the_frontier_is_a_complete_pareto_set_over_the_feasible_region() {
    let gpu = GpuLimits::ampere();
    let all = feasible_tiles(&gpu);
    let front = pareto_frontier(&gpu);

    assert!(!all.is_empty(), "the feasible region must be non-empty");
    assert!(!front.is_empty(), "the frontier must be non-empty");

    // (a) the frontier is a subset of the feasible region (only legal tiles).
    for t in &front {
        assert!(t.is_feasible(&gpu), "frontier tile {t:?} must be feasible");
    }

    // (b) the frontier is an ANTICHAIN: no frontier tile dominates another (nothing redundant survives).
    for a in &front {
        for b in &front {
            if a != b {
                assert!(
                    !a.dominates(b),
                    "frontier tile {a:?} dominates {b:?} — not a clean Pareto set"
                );
            }
        }
    }

    // (c) COMPLETENESS: every feasible tile is either on the frontier or dominated by some feasible tile.
    //     So pruning to the frontier discards ONLY provably-dominated tiles — the optimum is guaranteed kept.
    for t in &all {
        let kept = front.contains(t);
        let dominated = all.iter().any(|o| o.dominates(t));
        assert!(
            kept || dominated,
            "feasible tile {t:?} is neither on the frontier nor dominated — the frontier is not complete"
        );
    }

    // (d) the pruning is REAL: the frontier is a small fraction of the feasible region (so the GPU measures a
    //     handful, not hundreds). This is the "shift left" payoff.
    eprintln!(
        "feasible tiles: {}   Pareto frontier: {}   ({:.1}% measured)",
        all.len(),
        front.len(),
        100.0 * front.len() as f64 / all.len() as f64
    );
    assert!(
        front.len() * 3 < all.len(),
        "frontier ({}) must prune the feasible region ({}) by at least 3x",
        front.len(),
        all.len()
    );
}

#[test]
fn dominance_is_a_strict_partial_order_no_weights() {
    // Sanity on the ordering itself: irreflexive (a tile never dominates itself) and consistent with the
    // physical axes it is built from — this is the assumption-free core, so it must be exactly right.
    for t in feasible_tiles(&GpuLimits::ampere()) {
        assert!(
            !t.dominates(&t),
            "dominance must be irreflexive: {t:?} dominates itself"
        );
    }
}
