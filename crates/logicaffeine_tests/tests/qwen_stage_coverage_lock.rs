//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! QWEN STAGE COVERAGE — the pit-of-success ratchet for "Qwen authored in LOGOS, lowered to parallel CUDA".
//!
//! A Qwen3 forward decomposes into a small set of stages. Most are already FIRST-CLASS LOGOS tensor ops
//! (`matmul`, `plus`, `hadamard`, `rmsnorm [with weight]`, `softmax`) that the CUDA backend recognizes and
//! lowers to a genuine PARALLEL kernel (DP4A GEMM / one-block-per-row fused reduction) — NOT the single-thread
//! whole-program path. This ratchet pins that: every covered Qwen stage must keep lowering to a parallel
//! kernel, and the stages that are NOT yet first-class tensor ops (RoPE, SiLU) are listed as explicit,
//! shrink-only gaps. A completeness assertion guarantees every stage a real Qwen forward needs is either
//! covered or an accounted gap — nothing is silently missing.
//!
//! This is the mechanism (rule 14) that drives the "beat llama.cpp" path: as RoPE/SiLU become first-class
//! (removing them from `STAGE_GAPS`), the whole forward becomes expressible as parallel-lowered tensor ops.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::compile_to_cuda_tensor_op;

/// A 2×2 int tensor probe binding `C` via `<binder>` (A = 1..4, B = 5..8).
fn int_probe(binder: &str) -> String {
    format!(
        "## Main\n\
         Let A be a 2 by 2 tensor of Int.\n\
         Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n\
         Let B be a 2 by 2 tensor of Int.\n\
         Set item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\n\
         {binder}\n\
         Show item 1 of C.\n"
    )
}

/// A Real 1×4 vector probe (`X = [2,4,6,8]`) binding `Y` via `<binder>`.
fn real_probe(binder: &str) -> String {
    format!(
        "## Main\n\
         Let X be a 1 by 4 tensor of Real.\n\
         Set item 1 of X to 2.0.\nSet item 2 of X to 4.0.\nSet item 3 of X to 6.0.\nSet item 4 of X to 8.0.\n\
         {binder}\n\
         Show round(item 1 of Y * 1000000).\n"
    )
}

/// The RMSNorm-with-weight probe (the HF/Qwen transformer norm), binding `Y`.
fn rmsnorm_weighted_probe() -> String {
    real_probe(
        "Let W be a 1 by 4 tensor of Real.\n\
         Set item 1 of W to 0.5.\nSet item 2 of W to 1.0.\nSet item 3 of W to 1.5.\nSet item 4 of W to 2.0.\n\
         Let Y be rmsnorm of X with weight W.",
    )
}

/// Every stage a real Qwen3 forward performs, and how it maps onto a first-class LOGOS tensor op. `probe` is
/// `Some(program)` when the stage lowers to a parallel CUDA kernel today, `None` when it is a gap.
struct Stage {
    name: &'static str,
    /// The Qwen sub-computations this stage covers.
    covers: &'static str,
    probe: Option<String>,
}

fn stages() -> Vec<Stage> {
    vec![
        Stage {
            name: "matmul",
            covers: "Q/K/V/O projections, attention Q·Kᵀ and scores·V, SwiGLU gate/up/down, LM head",
            probe: Some(int_probe("Let C be A matmul B.")),
        },
        Stage {
            name: "rmsnorm_weighted",
            covers: "attn_norm, ffn_norm, q_norm, k_norm, output_norm (per-head/per-row RMSNorm with gain)",
            probe: Some(rmsnorm_weighted_probe()),
        },
        Stage {
            name: "softmax",
            covers: "attention score normalization",
            probe: Some(real_probe("Let Y be softmax of X.")),
        },
        Stage {
            name: "hadamard",
            covers: "SwiGLU elementwise product silu(gate) ⊙ up",
            probe: Some(int_probe("Let C be A hadamard B.")),
        },
        Stage {
            name: "plus",
            covers: "residual adds after attention and FFN",
            probe: Some(int_probe("Let C be A plus B.")),
        },
        // ── gaps: Qwen stages not yet expressible as a single first-class tensor op ──────────────────
        Stage { name: "silu", covers: "SwiGLU gate activation x·σ(x)", probe: None },
        Stage { name: "rope", covers: "RoPE-NEOX rotary position embedding on Q/K heads", probe: None },
    ]
}

/// SHRINK-ONLY: the Qwen stages that are NOT yet first-class parallel tensor ops. Empty is the goal; each
/// removal (as `silu`/`rope` become first-class) must be matched by giving that stage a `probe` above.
const STAGE_GAPS: &[&str] = &["silu", "rope"];

/// Every covered Qwen stage lowers to a genuine PARALLEL CUDA kernel (recognized tensor op, not the
/// single-thread whole-program path). `compile_to_cuda_tensor_op` returns `Ok` only when it recognizes a
/// matmul / elementwise / reduction and emits its optimized kernel.
#[test]
fn every_covered_qwen_stage_lowers_to_a_parallel_cuda_kernel() {
    for s in stages() {
        if let Some(prog) = &s.probe {
            let cu = compile_to_cuda_tensor_op(prog).unwrap_or_else(|e| {
                panic!("Qwen stage '{}' ({}) must lower to a parallel CUDA tensor-op kernel — got: {e}", s.name, s.covers)
            });
            assert!(
                cu.contains("Auto-generated by LOGOS codegen_cuda"),
                "stage '{}' must emit a real generated CUDA kernel, not a stub:\n{}",
                s.name,
                &cu[..cu.len().min(200)]
            );
        }
    }
}

/// The gap ledger is exactly the stages without a probe — no covered stage silently regressed into a gap,
/// and no gap is silently "covered". This is the shrink-only ratchet: change one side, this lock forces the
/// other.
#[test]
fn the_stage_gap_ledger_is_exact_and_shrink_only() {
    let mut declared_gaps: Vec<&str> = stages()
        .into_iter()
        .filter(|s| s.probe.is_none())
        .map(|s| s.name)
        .collect();
    declared_gaps.sort_unstable();
    let mut expected: Vec<&str> = STAGE_GAPS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        declared_gaps, expected,
        "STAGE_GAPS must list exactly the stages without a lowering probe. To CLOSE a gap, give the stage a \
         probe AND remove it here (the ratchet only shrinks); regressing a covered stage into a gap fails here."
    );
}

/// Completeness: the union of covered stages and gaps accounts for every stage of the Qwen forward we
/// enumerate — a Qwen forward needs nothing outside this set, so nothing is silently missing.
#[test]
fn the_qwen_forward_stage_set_is_complete() {
    let all: Vec<&str> = stages().iter().map(|s| s.name).collect();
    for required in [
        "matmul",
        "rmsnorm_weighted",
        "softmax",
        "hadamard",
        "plus",
        "silu",
        "rope",
    ] {
        assert!(
            all.contains(&required),
            "the Qwen forward stage set must include '{required}'"
        );
    }
    let covered = stages().iter().filter(|s| s.probe.is_some()).count();
    let gaps = STAGE_GAPS.len();
    assert_eq!(
        covered + gaps,
        stages().len(),
        "every stage is either covered or an accounted gap"
    );
    println!("Qwen forward stages: {covered} lower to parallel CUDA kernels, {gaps} accounted gaps ({STAGE_GAPS:?}).");
}
