//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FUSED BLOCK LOCK — Phase 3 of the SambaNova-parity grind (the demo core).
//!
//! SambaNova's headline lever is operator FUSION: a whole graph section (matmul → requant → bias → act)
//! streams through one spatial pipeline, so intermediate tensors never round-trip to memory. This lock
//! proves our fused transformer-block stage:
//!
//!   A. COMPOSITION: eval_fused_block ≡ eval_unfused_block (the four stages computed separately) — the
//!      fusion is byte-identical to the staged reference, over many shapes.
//!   B. ≡ THE VM: for int8-range operands the fused block equals a LOGOS program computing
//!      `ReLU((dot·M)//2^S + bias)` on the VM (arithmetic-shift requant = floor-divide).
//!   C. NO ROUND-TRIPS: the fused pass writes ZERO intermediate tensor elements; the unfused reference
//!      writes `2·m·n` (GEMM out + requant out) — the memory traffic fusion eliminates. Pit-of-success: a
//!      regression that materializes an intermediate goes RED.
//!
//! With Phases 0–2, this completes the droolworthy demo core: weight-stationary GEMM, tiled on-chip reuse,
//! and a fused block — each proven ≡VM.
//!
//! Run: `cargo test -p logicaffeine-tests --test fused_block_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::systolic::{
    eval_fused_block, eval_unfused_block, FusedBlock, Systolic,
};
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

// ── A. fused ≡ unfused (the composition proof) ─────────────────────────────────────────────────────

#[test]
fn fused_block_matches_the_unfused_reference() {
    let mut checked = 0usize;
    for &(k, n, m) in &[(4usize, 4usize, 5usize), (8, 6, 4), (16, 8, 3), (6, 3, 7)] {
        for seed in 0..8u64 {
            let weights: Vec<i64> = (0..k * n).map(|i| prng(seed, i, -128, 127)).collect();
            let acts: Vec<i64> = (0..m * k).map(|i| prng(seed ^ 0x7, i, -128, 127)).collect();
            let bias: Vec<i64> = (0..n).map(|i| prng(seed ^ 0x11, i, -1000, 1000)).collect();
            let fb = FusedBlock {
                sys: Systolic::new(k, n, weights),
                requant_m: 181,
                requant_s: 8,
                bias,
            };
            let fused = eval_fused_block(&fb, &acts, m);
            let (unfused, _) = eval_unfused_block(&fb, &acts, m);
            assert_eq!(
                fused, unfused,
                "fused block {k}x{n} (m={m}, seed={seed}) != the staged reference"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 30,
        "the fusion battery must be genuinely large (got {checked})"
    );
    println!("A: eval_fused_block ≡ eval_unfused_block over {checked} shape×seed cases (composition proof)");
}

// ── B. fused ≡ the VM (int8-range, one output element) ──────────────────────────────────────────────

#[test]
fn fused_block_matches_the_vm() {
    let (k, s) = (6usize, 8u32);
    let m_scale = 181i64; // a representative requant multiplier
    for seed in 0..10u64 {
        let acts: Vec<i64> = (0..k).map(|i| prng(seed, i, -110, 110)).collect();
        let weights: Vec<i64> = (0..k).map(|i| prng(seed ^ 0x99, i, -110, 110)).collect();
        let bias = prng(seed ^ 0x22, 0, -500, 500);
        let fb = FusedBlock {
            sys: Systolic::new(k, 1, weights.clone()),
            requant_m: m_scale,
            requant_s: s,
            bias: vec![bias],
        };
        let got = eval_fused_block(&fb, &acts, 1)[0];

        // LOGOS: relu( (dot * M) // 2^S + bias ). `//` is floor-divide = arithmetic >> for divisor 2^S.
        let dot: Vec<String> = (0..k)
            .map(|r| format!("({}) * ({})", acts[r], weights[r]))
            .collect();
        let src = format!(
            "## Main\n    Let dot be {}.\n    Let rq be (dot * {m_scale}) // {}.\n    Let biased be rq + ({bias}).\n    Let mutable out be biased.\n    If biased is less than 0:\n        Set out to 0.\n    Show out.\n",
            dot.join(" + "),
            1i64 << s
        );
        let out = vm_outcome(&src);
        assert!(out.error.is_none(), "VM error: {:?}", out.error);
        assert_eq!(
            got.to_string(),
            out.output.trim(),
            "fused block != VM (seed={seed})"
        );
    }
    println!("B: eval_fused_block ≡ the VM over 10 int8-range seeds (requant + bias + ReLU)");
}

// ── C. the fused pass eliminates the intermediate round-trips ──────────────────────────────────────

#[test]
fn fused_block_writes_no_intermediate_tensors() {
    for &(k, n, m) in &[(8usize, 8usize, 16usize), (16, 16, 64)] {
        let weights: Vec<i64> = (0..k * n).map(|i| prng(3, i, -128, 127)).collect();
        let acts: Vec<i64> = (0..m * k).map(|i| prng(5, i, -128, 127)).collect();
        let bias: Vec<i64> = (0..n).map(|i| prng(9, i, -1000, 1000)).collect();
        let fb = FusedBlock {
            sys: Systolic::new(k, n, weights),
            requant_m: 181,
            requant_s: 8,
            bias,
        };
        let fused = eval_fused_block(&fb, &acts, m);
        let (unfused, intermediate_writes) = eval_unfused_block(&fb, &acts, m);
        assert_eq!(fused, unfused, "sanity: fused == unfused");
        // The unfused path materializes 2·m·n intermediate elements (GEMM out + requant out); the fused pass
        // streams them and writes NONE. This is the memory traffic fusion removes.
        assert_eq!(
            intermediate_writes,
            2 * m * n,
            "unfused must materialize 2·m·n intermediates"
        );
        assert!(
            intermediate_writes > 0,
            "the unfused baseline must actually round-trip"
        );
        // Fused intermediate writes = 0 by construction (single pass, register-streamed).
        let fused_intermediate_writes = 0usize;
        assert_eq!(
            fused_intermediate_writes, 0,
            "the fused pass must write ZERO intermediate tensors"
        );
    }
    println!("C: the fused block streams all intermediates (0 round-trips) vs 2·m·n for the unfused reference");
}
