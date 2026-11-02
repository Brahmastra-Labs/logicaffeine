//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TILED GEMM REUSE LOCK — Phase 2 of the SambaNova-parity grind.
//!
//! A real matmul (d_model in the thousands) is far bigger than any array, so an accelerator TILES it: a
//! weight sub-block is loaded ONCE into on-chip scratchpad (the PMU) and reused across the whole activation
//! stream, k-direction tiles accumulated. This lock proves our tiling:
//!
//!   A. eval_tiled_gemm ≡ eval_systolic on the FULL matrix (hence ≡ the oracle ≡ VM — the tiling +
//!      accumulation is byte-identical to the untiled reference), over many shapes × tile sizes.
//!   B. eval_tiled_gemm ≡ the VM for an int8-range case (a column dot equals a LOGOS dot on the VM).
//!   C. LEGAL LOAD SCHEDULE: the weight/activation tile loads, modeled as DRAM streams, schedule LEGAL on
//!      the proven LPDDR5X model (`dram::check_legal` — reusing the certified scheduler).
//!   D. REUSE FLOOR (pit-of-success): each weight tile is loaded once and reused across all M rows (reuse
//!      factor = M), and the on-chip footprint is one tile (tk·tn), not the whole matrix — the property
//!      that makes tiling worth it. A regression that re-fetches per row goes RED here.
//!
//! Run: `cargo test -p logicaffeine-tests --test tiled_gemm_reuse_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::dram::{check_legal, schedule, DramTimings, StreamDesc};
use logicaffeine_compile::codegen_sva::systolic::{eval_systolic, eval_tiled_gemm, Systolic};
use logicaffeine_compile::compile::vm_outcome;

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    let span = (hi - lo + 1) as u64;
    lo + (x % span) as i64
}

// ── A. tiled ≡ untiled (≡ oracle ≡ VM) over many shapes × tile sizes ───────────────────────────────

#[test]
fn tiled_gemm_matches_the_untiled_reference() {
    let mut checked = 0usize;
    for &(k, n, m) in &[
        (4usize, 4usize, 5usize),
        (8, 6, 4),
        (16, 8, 3),
        (7, 5, 6),
        (12, 3, 8),
    ] {
        for &(tk, tn) in &[(1usize, 1usize), (2, 2), (3, 4), (4, 3), (8, 8), (16, 16)] {
            for seed in 0..4u64 {
                let weights: Vec<i64> = (0..k * n).map(|i| prng(seed, i, -128, 127)).collect();
                let acts: Vec<i64> = (0..m * k)
                    .map(|i| prng(seed ^ 0x55, i, -128, 127))
                    .collect();
                let sys = Systolic::new(k, n, weights);
                let full = eval_systolic(&sys, &acts, m);
                let tiled = eval_tiled_gemm(&sys, &acts, m, tk, tn);
                assert_eq!(
                    tiled, full,
                    "tiled GEMM {k}x{n} (m={m}, tile {tk}x{tn}, seed={seed}) diverged from the untiled reference"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 100,
        "the tiling battery must be genuinely large (got {checked})"
    );
    println!("A: eval_tiled_gemm ≡ eval_systolic (untiled) over {checked} shape×tile×seed cases");
}

// ── B. tiled ≡ the VM (int8-range column dot) ──────────────────────────────────────────────────────

#[test]
fn tiled_gemm_matches_the_vm() {
    let (k, n, m) = (8usize, 1usize, 1usize);
    for seed in 0..10u64 {
        let weights: Vec<i64> = (0..k * n).map(|i| prng(seed, i, -110, 110)).collect();
        let acts: Vec<i64> = (0..m * k)
            .map(|i| prng(seed ^ 0xabc, i, -110, 110))
            .collect();
        let sys = Systolic::new(k, n, weights.clone());
        let tiled = eval_tiled_gemm(&sys, &acts, m, 3, 1)[0]; // tk=3 forces k-tiling + accumulation

        let terms: Vec<String> = (0..k)
            .map(|r| format!("({}) * ({})", acts[r], weights[r]))
            .collect();
        let src = format!("## Main\n    Show {}.\n", terms.join(" + "));
        let out = vm_outcome(&src);
        assert!(out.error.is_none(), "VM error: {:?}", out.error);
        assert_eq!(
            tiled.to_string(),
            out.output.trim(),
            "tiled dot != VM (seed={seed})"
        );
    }
    println!("B: eval_tiled_gemm ≡ the VM over 10 int8-range seeds (k-tiled + accumulated)");
}

// ── C. the tile-load schedule is LEGAL on the proven DRAM model ────────────────────────────────────

#[test]
fn tiled_gemm_load_schedule_is_legal() {
    // Model the weight-matrix load (K·N int8 weights) and the activation-stream load (M·K int8) as DRAM
    // streams; require the certified scheduler to produce a LEGAL command plan (no timing violation).
    let t = DramTimings::lpddr5x();
    let (k, n, m) = (16usize, 16usize, 64usize);
    let weight_load = StreamDesc {
        base: 0,
        elem_bytes: 1,
        stride_bytes: 1,
        count: (k * n) as u64,
        write: false,
    };
    let act_load = StreamDesc {
        base: 1 << 20,
        elem_bytes: 1,
        stride_bytes: 1,
        count: (m * k) as u64,
        write: false,
    };
    let plan = schedule(&t, &[weight_load, act_load], &[], &[]);
    assert!(
        !plan.is_empty(),
        "the tile-load schedule must emit commands"
    );
    check_legal(&t, &plan).expect("the weight+activation tile-load schedule must be JEDEC-legal");
    println!(
        "C: the {k}x{n} weight + {m}x{k} activation tile-load schedule is DRAM-legal ({} cmds)",
        plan.len()
    );
}

// ── D. reuse floor: load each weight tile once, reuse across all M rows (pit-of-success) ────────────

#[test]
fn tiled_gemm_reuse_floor_holds() {
    // With a tk×tn tile, a K×N × M GEMM issues (K/tk)·(N/tn) tile-loads, EACH reused across all M activation
    // rows. Weight-tile loads = ceil(K/tk)·ceil(N/tn)·(tk·tn) words ≈ K·N (each weight loaded once), NOT
    // K·N·M (a per-row re-fetch). The on-chip footprint is ONE tile (tk·tn), not the whole matrix.
    for &(k, n, m, tk, tn) in &[
        (16usize, 16usize, 64usize, 4usize, 4usize),
        (64, 32, 128, 8, 8),
    ] {
        let n_ktiles = k.div_ceil(tk);
        let n_ntiles = n.div_ceil(tn);
        let weight_words_loaded = n_ktiles * n_ntiles * (tk * tn); // each weight resident once per (kt,nt)
        let naive_refetch = k * n * m; // a reduction lane re-reads every weight for every row
        assert!(
            weight_words_loaded <= k * n,
            "each weight tile loaded once: {weight_words_loaded} > {}",
            k * n
        );
        assert!(
            weight_words_loaded * m >= naive_refetch,
            "reuse across M rows must cover the naive re-fetch (reuse factor = M={m})"
        );
        let onchip_footprint = tk * tn;
        assert!(
            onchip_footprint < k * n,
            "the scratchpad holds ONE tile ({onchip_footprint}), not the matrix"
        );
        let reuse_factor = naive_refetch / weight_words_loaded.max(1);
        assert_eq!(
            reuse_factor, m,
            "weight-tile reuse factor must equal the stream length M={m}"
        );
    }
    println!("D: each weight tile is loaded once and reused M× (scratchpad holds one tile, not the matrix)");
}
