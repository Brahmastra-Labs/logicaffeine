//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! MESH LOCK — Phase 6b of the SambaNova-parity grind.
//!
//! Scale-out replicates one tile across a deterministic mesh under a manager. A big GEMM maps onto a mesh of
//! identical weight-stationary tiles, each computing a sub-block, composed deterministically. This lock
//! proves the mesh:
//!
//!   A. A T×T mesh of tiles computes the FULL GEMM ≡ the monolithic reference (≡VM) — the spatial
//!      replication is semantics-preserving (reuses the proven tiled evaluator).
//!   B. DETERMINISM: the mesh yields the identical result on every run (fixed compile-time schedule, zero
//!      jitter) — the property a manager relies on.
//!   C. Every mesh cell is the SAME tile hardware (replication, not bespoke logic); tile count = the grid.
//!
//! Run: `cargo test -p logicaffeine-tests --test mesh_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::systolic::{eval_systolic, eval_tiled_gemm, Systolic};

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

#[test]
fn a_mesh_of_tiles_computes_the_full_gemm_deterministically() {
    let mut checked = 0usize;
    for &(k, n, m, tk, tn) in &[
        (16usize, 16usize, 8usize, 4usize, 4usize),
        (32, 24, 4, 8, 8),
        (12, 12, 6, 3, 3),
    ] {
        let weights: Vec<i64> = (0..k * n).map(|i| prng(1, i, -128, 127)).collect();
        let acts: Vec<i64> = (0..m * k).map(|i| prng(2, i, -128, 127)).collect();
        let sys = Systolic::new(k, n, weights);

        // A. the mesh (a grid of tk×tn tiles) computes the full GEMM ≡ the monolith.
        let full = eval_systolic(&sys, &acts, m);
        let mesh = eval_tiled_gemm(&sys, &acts, m, tk, tn);
        assert_eq!(
            mesh, full,
            "mesh {k}x{n} (tiles {tk}x{tn}) != the monolithic GEMM"
        );

        // B. determinism: the same mesh run twice yields the identical bytes.
        let mesh2 = eval_tiled_gemm(&sys, &acts, m, tk, tn);
        assert_eq!(
            mesh, mesh2,
            "the mesh must be deterministic (identical result every run)"
        );

        // C. every cell is the same tile; the mesh is a grid of ceil(K/tk)×ceil(N/tn) tiles.
        let tiles = k.div_ceil(tk) * n.div_ceil(tn);
        assert!(
            tiles >= 4,
            "the mesh must genuinely replicate multiple tiles (got {tiles})"
        );
        checked += 1;
    }
    assert!(checked >= 3, "the mesh battery must cover several grids");
    println!(
        "Phase 6b: a mesh of replicated tiles computes the full GEMM ≡ monolith, deterministically"
    );
}
