//! tensor_probes — the ONE table of tensor-sugar probe programs every tensor-op ratchet consumes.
//!
//! A probe is a self-contained LOGOS program that exercises exactly one tensor sugar and `Show`s its result,
//! so the VM oracle computes it and every backend / stage lock can compare against that stdout. The table
//! lives here — included per lock via `#[path = "common/tensor_probes.rs"]` (D4) — so the backend coverage
//! matrix (`tensor_op_coverage_lock`) and the spatial stage ledger (`spatial_op_gap_ledger_lock`) can never
//! drift apart on WHICH programs they mean by "the ops".
//!
//! Three tables over one set of `const` probes:
//!   • `OP_PROBES`             — every probe, the universe the spatial ledger ratchets over;
//!   • `IMPLEMENTED_PROBES`    — the probes whose sugar the parser accepts today (the backend matrix);
//!   • `PENDING_SUGAR_PROBES`  — sugars a plan schedules before the parser accepts them: RED at the parser
//!                               until then, listed in the ledger under `PENDING_SUGAR_REASON` (empty since
//!                               `row … of` / `argmax of` / `Set row … to` landed).
//! `the_probe_tables_partition_the_universe` (in the ledger lock) keeps the three consistent.
//!
//! `needs` names the algebra primitives an op's spatial composition requires (drawn from `PRIMITIVES`), so a
//! later phase can filter the ledger by the piece it is landing. `SUGAR_TO_PROBE` maps every parser
//! `fn desugar_tensor_<name>` to the probe that exercises it; `desugar_sugars_in` is the scanner the ledger
//! runs over the parser source to prove the map is complete and has no stale rows.
//!
//! The oracle-projection helpers at the bottom (`shows_of`, `inputs_from_fills`, `shown_from_outputs`,
//! `agrees_with_oracle`) turn a spatial `eval_choreography` result into the integers the program's `Show`
//! lines would print, under the SAME rounding rule `spatial_front_end_lock` uses (the target's float datapath
//! width, within one ulp of the ×1e6 rounding), so "≡ VM" means one thing everywhere.
#![allow(dead_code)]

use std::collections::BTreeSet;

use logicaffeine_compile::spatial::eval::{round_to, Inputs, Outputs, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorDecl, TensorId, TensorRole};

/// A tensor-sugar op, a self-contained program that exercises it, how many values it `Show`s, the algebra
/// primitives its spatial composition needs, and (for the exact ops) the known output — an independent
/// correctness anchor on the VM oracle. Float ops leave `expect` `None` (their correctness is locked by
/// property in `tensor_reduction_ops_lock`); the VM is still their cross-tier oracle.
#[derive(Clone, Copy, Debug)]
pub struct OpProbe {
    pub op: &'static str,
    /// The prefix of the spatial program name the op must lower AS — one of the program's sections is named so
    /// (`matmul_2x2x2`, `elementwise_+_4`, `rmsnorm_1x4`, an attention's `attn_scores_2x2`, …). Recognition means
    /// being lifted as ONESELF: a `gelu` program lowered as `rmsnorm_1x4` — a fragment of its desugar matched as the
    /// whole program — is not recognized, it is miscompiled, and the ledger's `MISRECOGNIZED` table is where that goes.
    pub family: &'static str,
    /// The kind of stage the whole-program walk must lift the probe AS (`TensorOpSpec::kind`): one stage of this
    /// kind, and none of another — a fragment of the desugar lifted as some other op is not recognition.
    pub kind: &'static str,
    pub program: &'static str,
    pub shows: usize,
    pub needs: &'static [&'static str],
    pub expect: Option<&'static [&'static str]>,
}

/// The closed vocabulary `needs` draws from — the primitives B-P3/B-P5/B-P6 add to the spatial algebra.
/// A typo in a probe's `needs` fails `every_need_is_a_named_primitive` instead of silently filtering nothing.
pub const PRIMITIVES: &[&str] = &[
    // flat-affine, signed element access (transpose, head slicing, reshape, rope pair reads)
    "flat_affine",
    // axis-general reduction (softmax over positions, conv over taps, per-row norms)
    "row_reduce",
    // `ReducePool::ArgMax` (next token, MoE top-k)
    "argmax",
    // `TensorBody::Select` — piecewise along an axis (rope windows, conv ring shift, position mask)
    "select",
    // `SfpuKind::Ln` (`decay = exp(R·ln(1+e^u))`)
    "ln",
    // value-dependent element/tile address (embedding row, top-k scatter, KV-cache row)
    "gather",
    // stored-and-loaded in one program / persisting across steps (GDN memory, conv ring, KV cache)
    "state",
];

/// The reason the ledger records for a probe whose sugar the parser does not accept yet.
pub const PENDING_SUGAR_REASON: &str = "parser sugar not yet implemented (B-P5/B-P6)";

/// Build a 2×2 int probe: declare A,B (filled 1..4 / 5..8), bind `C` via `<binder>`, Show C's four entries.
macro_rules! int_probe {
    ($binder:expr) => {
        concat!(
            "## Main\n",
            "Let A be a 2 by 2 tensor of Int.\n",
            "Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n",
            "Let B be a 2 by 2 tensor of Int.\n",
            "Set item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\n",
            $binder, "\n",
            "Show item 1 of C.\nShow item 2 of C.\nShow item 3 of C.\nShow item 4 of C.\n",
        )
    };
}

/// Build a unary float probe: declare a Real 4-vector `X`, bind `Y` via `<binder>`, Show `round(Y·1e6)`.
/// The rounding makes the output cross-tier-deterministic despite `powf`'s per-libm rounding.
macro_rules! real_probe {
    ($binder:expr) => {
        concat!(
            "## Main\n",
            "Let X be a 1 by 4 tensor of Real.\n",
            "Set item 1 of X to 2.0.\nSet item 2 of X to 4.0.\nSet item 3 of X to 6.0.\nSet item 4 of X to 8.0.\n",
            $binder, "\n",
            "Show round(item 1 of Y * 1000000).\nShow round(item 2 of Y * 1000000).\n",
            "Show round(item 3 of Y * 1000000).\nShow round(item 4 of Y * 1000000).\n",
        )
    };
}

// ---- the backend-matrix probes (every backend runs these today) ------------------------------------

// C = A·B = [[19,22],[43,50]]
pub const MATMUL: OpProbe = OpProbe {
    op: "matmul",
    family: "matmul",
    kind: "Matmul",
    program: int_probe!("Let C be A matmul B."),
    shows: 4,
    needs: &[],
    expect: Some(&["19", "22", "43", "50"]),
};
// C = A + B (elementwise) = [[6,8],[10,12]]
pub const PLUS: OpProbe = OpProbe {
    op: "plus",
    family: "elementwise_+",
    kind: "Elementwise",
    program: int_probe!("Let C be A plus B."),
    shows: 4,
    needs: &[],
    expect: Some(&["6", "8", "10", "12"]),
};
// C = A ∘ B (elementwise / Hadamard) = [[5,12],[21,32]]
pub const HADAMARD: OpProbe = OpProbe {
    op: "hadamard",
    family: "elementwise_*",
    kind: "Elementwise",
    program: int_probe!("Let C be A hadamard B."),
    shows: 4,
    needs: &[],
    expect: Some(&["5", "12", "21", "32"]),
};
// Y = rmsnorm(X) — unit-RMS rescale of [2,4,6,8].
pub const RMSNORM: OpProbe = OpProbe {
    op: "rmsnorm",
    family: "rmsnorm",
    kind: "FusedRow",
    program: real_probe!("Let Y be rmsnorm of X."),
    shows: 4,
    needs: &["row_reduce"],
    expect: None,
};
// Y = softmax(X) — probability distribution over [2,4,6,8].
pub const SOFTMAX: OpProbe = OpProbe {
    op: "softmax",
    family: "softmax",
    kind: "FusedRow",
    program: real_probe!("Let Y be softmax of X."),
    shows: 4,
    needs: &["row_reduce"],
    expect: None,
};
// Y = rmsnorm(X) with a per-column weight — the HF/transformer RMSNorm (reduce+scale core of fused-add).
pub const RMSNORM_WEIGHTED: OpProbe = OpProbe {
    op: "rmsnorm_weighted", family: "rmsnorm", kind: "FusedRow",
    program: real_probe!(concat!(
        "Let W be a 1 by 4 tensor of Real.\n",
        "Set item 1 of W to 0.5.\nSet item 2 of W to 1.0.\nSet item 3 of W to 1.5.\nSet item 4 of W to 2.0.\n",
        "Let Y be rmsnorm of X with weight W."
    )),
    shows: 4,
    needs: &["row_reduce"],
    expect: None,
};
// Y = fused_add_rmsnorm(X, R, W) — `Y = rmsnorm(X + R) * W`, the FIB `001_fused_add_rmsnorm` shape.
pub const FUSED_ADD_RMSNORM: OpProbe = OpProbe {
    op: "fused_add_rmsnorm", family: "rmsnorm", kind: "FusedRow",
    program: real_probe!(concat!(
        "Let R be a 1 by 4 tensor of Real.\n",
        "Set item 1 of R to 0.5.\nSet item 2 of R to 0.5.\nSet item 3 of R to 0.5.\nSet item 4 of R to 0.5.\n",
        "Let W be a 1 by 4 tensor of Real.\n",
        "Set item 1 of W to 0.5.\nSet item 2 of W to 1.0.\nSet item 3 of W to 1.5.\nSet item 4 of W to 2.0.\n",
        "Let Y be rmsnorm of X plus R with weight W."
    )),
    shows: 4,
    needs: &["row_reduce"],
    expect: None,
};
// Y = layernorm(X, W, B) — `Y = (X-mean)/sqrt(var+eps)*W + B`, per row.
pub const LAYERNORM: OpProbe = OpProbe {
    op: "layernorm", family: "layernorm", kind: "FusedRow",
    program: real_probe!(concat!(
        "Let W be a 1 by 4 tensor of Real.\n",
        "Set item 1 of W to 0.5.\nSet item 2 of W to 1.0.\nSet item 3 of W to 1.5.\nSet item 4 of W to 2.0.\n",
        "Let B be a 1 by 4 tensor of Real.\n",
        "Set item 1 of B to 0.1.\nSet item 2 of B to 0.2.\nSet item 3 of B to 0.3.\nSet item 4 of B to 0.4.\n",
        "Let Y be layernorm of X with weight W and bias B."
    )),
    shows: 4,
    needs: &["row_reduce"],
    expect: None,
};
// Y = swiglu(Gate, Up) — `Y[i] = silu(Gate[i]) * Up[i]`, the gated MLP activation.
pub const SWIGLU: OpProbe = OpProbe {
    op: "swiglu", family: "swiglu", kind: "SwiGlu",
    program: real_probe!(concat!(
        "Let G be a 1 by 4 tensor of Real.\n",
        "Set item 1 of G to -1.0.\nSet item 2 of G to 0.0.\nSet item 3 of G to 1.0.\nSet item 4 of G to 2.0.\n",
        "Let U be a 1 by 4 tensor of Real.\n",
        "Set item 1 of U to 1.0.\nSet item 2 of U to 2.0.\nSet item 3 of U to 3.0.\nSet item 4 of U to 4.0.\n",
        "Let Y be swiglu of G and U."
    )),
    shows: 4,
    needs: &[],
    expect: None,
};
// ---- Qwen3.8-27B ops -------------------------------------------------------------------------
// Y = sigmoid(X) — `1/(1+e^-x)`. The gated-attention output gate and GDN's beta.
pub const SIGMOID: OpProbe = OpProbe {
    op: "sigmoid",
    family: "sigmoid",
    kind: "Sigmoid",
    program: real_probe!("Let Y be sigmoid of X."),
    shows: 4,
    needs: &[],
    expect: None,
};
// Y = gelu(X) — the tanh approximation ggml uses, so the ViT matches llama.cpp's mmproj.
pub const GELU: OpProbe = OpProbe {
    op: "gelu",
    family: "gelu",
    kind: "Gelu",
    program: real_probe!("Let Y be gelu of X."),
    shows: 4,
    needs: &[],
    expect: None,
};
// Y = X·1.5 — a tensor scaled by a constant (Gemma's embedding scale): [3, 6, 9, 12], exact.
pub const SCALE: OpProbe = OpProbe {
    op: "scale",
    family: "scale",
    kind: "Scale",
    program: real_probe!("Let Y be X scaled by 1.5."),
    shows: 4,
    needs: &[],
    expect: Some(&["3000000", "6000000", "9000000", "12000000"]),
};
// Y = X capped at 5 — `5·tanh(x/5)`, every element (Gemma 2 caps its logits so; `capped at C`).
pub const SOFTCAP: OpProbe = OpProbe {
    op: "softcap",
    family: "softcap",
    kind: "Softcap",
    program: real_probe!("Let Y be X capped at 5.0."),
    shows: 4,
    needs: &[],
    expect: None,
};
// ---- DeepSeek's latent-attention pieces ---------------------------------------------------------------
// P = the last 2 of every 3 of Q — each group's part from offset 1: [2, 3, 5, 6, 8, 9, 11, 12], exact.
pub const HEAD_SLICE: OpProbe = OpProbe {
    op: "head_slice", family: "head_slice", kind: "HeadSlice",
    program: concat!(
        "## Main\n",
        "Let Q be a 2 by 6 tensor of Real.\n",
        "Set item 1 of Q to 1.0.\nSet item 2 of Q to 2.0.\nSet item 3 of Q to 3.0.\nSet item 4 of Q to 4.0.\n",
        "Set item 5 of Q to 5.0.\nSet item 6 of Q to 6.0.\nSet item 7 of Q to 7.0.\nSet item 8 of Q to 8.0.\n",
        "Set item 9 of Q to 9.0.\nSet item 10 of Q to 10.0.\nSet item 11 of Q to 11.0.\nSet item 12 of Q to 12.0.\n",
        "Let P be the last 2 of every 3 of Q.\n",
        "Show round(item 1 of P * 1000000).\nShow round(item 2 of P * 1000000).\nShow round(item 3 of P * 1000000).\n",
        "Show round(item 4 of P * 1000000).\nShow round(item 5 of P * 1000000).\nShow round(item 6 of P * 1000000).\n",
        "Show round(item 7 of P * 1000000).\nShow round(item 8 of P * 1000000).\n",
    ),
    shows: 8,
    needs: &["flat_affine"],
    expect: Some(&["2000000", "3000000", "5000000", "6000000", "8000000", "9000000", "11000000", "12000000"]),
};
// J = A joined with B across 2 heads — each row's heads as [A part ‖ B part]: [1, 2, 10, 3, 4, 11, 5, 6, 12, 7, 8, 13].
pub const JOIN: OpProbe = OpProbe {
    op: "join", family: "join", kind: "Join",
    program: concat!(
        "## Main\n",
        "Let A be a 2 by 4 tensor of Real.\n",
        "Set item 1 of A to 1.0.\nSet item 2 of A to 2.0.\nSet item 3 of A to 3.0.\nSet item 4 of A to 4.0.\n",
        "Set item 5 of A to 5.0.\nSet item 6 of A to 6.0.\nSet item 7 of A to 7.0.\nSet item 8 of A to 8.0.\n",
        "Let B be a 2 by 2 tensor of Real.\n",
        "Set item 1 of B to 10.0.\nSet item 2 of B to 11.0.\nSet item 3 of B to 12.0.\nSet item 4 of B to 13.0.\n",
        "Let J be A joined with B across 2 heads.\n",
        "Show round(item 1 of J * 1000000).\nShow round(item 2 of J * 1000000).\nShow round(item 3 of J * 1000000).\n",
        "Show round(item 4 of J * 1000000).\nShow round(item 5 of J * 1000000).\nShow round(item 6 of J * 1000000).\n",
        "Show round(item 7 of J * 1000000).\nShow round(item 8 of J * 1000000).\nShow round(item 9 of J * 1000000).\n",
        "Show round(item 10 of J * 1000000).\nShow round(item 11 of J * 1000000).\nShow round(item 12 of J * 1000000).\n",
    ),
    shows: 12,
    needs: &["flat_affine"],
    expect: Some(&[
        "1000000", "2000000", "10000000", "3000000", "4000000", "11000000", "5000000", "6000000", "12000000", "7000000",
        "8000000", "13000000",
    ]),
};
// Y = X matmul W across 2 heads — each row's head h times W's rows 2h, 2h+1: [1, 2, 10, 11, 5, 6, 22, 23], exact.
pub const HEAD_MATMUL: OpProbe = OpProbe {
    op: "head_matmul", family: "head_matmul", kind: "HeadMatmul",
    program: concat!(
        "## Main\n",
        "Let X be a 2 by 4 tensor of Real.\n",
        "Set item 1 of X to 1.0.\nSet item 2 of X to 2.0.\nSet item 3 of X to 3.0.\nSet item 4 of X to 4.0.\n",
        "Set item 5 of X to 5.0.\nSet item 6 of X to 6.0.\nSet item 7 of X to 7.0.\nSet item 8 of X to 8.0.\n",
        "Let W be a 4 by 2 tensor of Real.\n",
        "Set item 1 of W to 1.0.\nSet item 2 of W to 0.0.\nSet item 3 of W to 0.0.\nSet item 4 of W to 1.0.\n",
        "Set item 5 of W to 2.0.\nSet item 6 of W to 1.0.\nSet item 7 of W to 1.0.\nSet item 8 of W to 2.0.\n",
        "Let Y be X matmul W across 2 heads.\n",
        "Show round(item 1 of Y * 1000000).\nShow round(item 2 of Y * 1000000).\nShow round(item 3 of Y * 1000000).\n",
        "Show round(item 4 of Y * 1000000).\nShow round(item 5 of Y * 1000000).\nShow round(item 6 of Y * 1000000).\n",
        "Show round(item 7 of Y * 1000000).\nShow round(item 8 of Y * 1000000).\n",
    ),
    shows: 8,
    needs: &["flat_affine", "row_reduce"],
    expect: Some(&["1000000", "2000000", "10000000", "11000000", "5000000", "6000000", "22000000", "23000000"]),
};
// Y = l2norm(X) — `X/‖X‖`, per row. GDN L2-normalizes q and k before the delta update.
pub const L2NORM: OpProbe = OpProbe {
    op: "l2norm",
    family: "l2norm",
    kind: "L2Norm",
    program: real_probe!("Let Y be l2norm of X."),
    shows: 4,
    needs: &["row_reduce"],
    expect: None,
};
// Y = rope(Q, cos, sin) over a PARTIAL rotary slice: Qwen3.8 rotates 64 of each head's 256 dims and
// passes the rest through. `across H heads` pins head_dim = |Q|/H independently of the table width.
pub const ROPE_PARTIAL: OpProbe = OpProbe {
    op: "rope_partial", family: "rope", kind: "Rope",
    program: concat!(
        "## Main\n",
        "Let Q be a 1 by 8 tensor of Real.\n",
        "Set item 1 of Q to 1.0.\nSet item 2 of Q to 2.0.\nSet item 3 of Q to 3.0.\nSet item 4 of Q to 4.0.\n",
        "Set item 5 of Q to 5.0.\nSet item 6 of Q to 6.0.\nSet item 7 of Q to 7.0.\nSet item 8 of Q to 8.0.\n",
        "Let RC be a 1 by 1 tensor of Real.\nSet item 1 of RC to 0.6.\n",
        "Let RS be a 1 by 1 tensor of Real.\nSet item 1 of RS to 0.8.\n",
        "Let Y be rope of Q with cos RC and sin RS across 2 heads.\n",
        "Show round(item 1 of Y * 1000000).\nShow round(item 2 of Y * 1000000).\n",
        "Show round(item 3 of Y * 1000000).\nShow round(item 4 of Y * 1000000).\n",
        "Show round(item 5 of Y * 1000000).\nShow round(item 6 of Y * 1000000).\n",
        "Show round(item 7 of Y * 1000000).\nShow round(item 8 of Y * 1000000).\n",
    ),
    shows: 8,
    needs: &["flat_affine", "select"],
    expect: None,
};
// Y = causal_conv(X, WC, CS) — the depthwise causal conv1d (kernel 4 in the real model) that GDN runs
// over q|k|v. `CS` is the decode ring of the last `taps` inputs, shifted and appended in place.
pub const CAUSAL_CONV: OpProbe = OpProbe {
    op: "causal_conv",
    family: "conv_",
    kind: "CausalConv",
    program: concat!(
        "## Main\n",
        "Let X be a 1 by 3 tensor of Real.\n",
        "Set item 1 of X to 1.0.\nSet item 2 of X to 2.0.\nSet item 3 of X to 3.0.\n",
        "Let WC be a 2 by 3 tensor of Real.\n",
        "Set item 1 of WC to 0.5.\nSet item 2 of WC to 0.25.\nSet item 3 of WC to 0.125.\n",
        "Set item 4 of WC to 1.5.\nSet item 5 of WC to 1.25.\nSet item 6 of WC to 1.125.\n",
        "Let CS be a 2 by 3 tensor of Real.\n",
        "Set item 1 of CS to 0.0.\nSet item 2 of CS to 0.0.\nSet item 3 of CS to 0.0.\n",
        "Set item 4 of CS to 0.0.\nSet item 5 of CS to 0.0.\nSet item 6 of CS to 0.0.\n",
        "Let Y be causal conv of X with weight WC state CS over 2 taps.\n",
        "Show round(item 1 of Y * 1000000).\nShow round(item 2 of Y * 1000000).\n",
        "Show round(item 3 of Y * 1000000).\n",
    ),
    shows: 3,
    needs: &["select", "row_reduce", "state"],
    expect: None,
};
// C = full_attention(Q, K, V) — BIDIRECTIONAL T×T attention (no causal mask), the vision tower's
// self-attention. Distinct from `attention`, which is single-query decode over a growing KV cache.
pub const FULL_ATTENTION: OpProbe = OpProbe {
    op: "full_attention", family: "full_attn_", kind: "FullAttention",
    program: concat!(
        "## Main\n",
        "Let Q be a 2 by 2 tensor of Real.\n",
        "Set item 1 of Q to 1.0.\nSet item 2 of Q to 0.0.\nSet item 3 of Q to 0.0.\nSet item 4 of Q to 1.0.\n",
        "Let K be a 2 by 2 tensor of Real.\n",
        "Set item 1 of K to 1.0.\nSet item 2 of K to 1.0.\nSet item 3 of K to 0.0.\nSet item 4 of K to 2.0.\n",
        "Let V be a 2 by 2 tensor of Real.\n",
        "Set item 1 of V to 3.0.\nSet item 2 of V to 4.0.\nSet item 3 of V to 5.0.\nSet item 4 of V to 6.0.\n",
        "Let C be full attention of Q over K and V with 1 heads.\n",
        "Show round(item 1 of C * 1000000).\nShow round(item 2 of C * 1000000).\n",
        "Show round(item 3 of C * 1000000).\nShow round(item 4 of C * 1000000).\n",
    ),
    shows: 4,
    needs: &["flat_affine", "row_reduce"],
    expect: None,
};
// O = gated_delta(...) — the Gated DeltaNet recurrent step, 48 of Qwen3.8-27B's 64 layers. Per value
// head: `u = S·k`, `S ← α·S + β·(v − α·u)·kᵀ`, `o = S·q`, then the head is RMSNorm'd and gated by
// `silu(z)`. `S` is a persistent D×D state per head, read AND written in place.
pub const GATED_DELTA: OpProbe = OpProbe {
    op: "gated_delta", family: "gd_", kind: "GatedDelta",
    program: concat!(
        "## Main\n",
        "Let Q be a 1 by 2 tensor of Real.\nSet item 1 of Q to 0.6.\nSet item 2 of Q to 0.8.\n",
        "Let K be a 1 by 2 tensor of Real.\nSet item 1 of K to 0.8.\nSet item 2 of K to 0.6.\n",
        "Let V be a 1 by 4 tensor of Real.\n",
        "Set item 1 of V to 1.0.\nSet item 2 of V to 2.0.\nSet item 3 of V to 3.0.\nSet item 4 of V to 4.0.\n",
        "Let Z be a 1 by 4 tensor of Real.\n",
        "Set item 1 of Z to 0.5.\nSet item 2 of Z to 1.0.\nSet item 3 of Z to 1.5.\nSet item 4 of Z to 2.0.\n",
        "Let B be a 1 by 2 tensor of Real.\nSet item 1 of B to 0.7.\nSet item 2 of B to 0.3.\n",
        "Let A be a 1 by 2 tensor of Real.\nSet item 1 of A to 0.9.\nSet item 2 of A to 0.5.\n",
        "Let S be a 4 by 2 tensor of Real.\n",
        "Set item 1 of S to 0.1.\nSet item 2 of S to 0.2.\nSet item 3 of S to 0.3.\nSet item 4 of S to 0.4.\n",
        "Set item 5 of S to 0.5.\nSet item 6 of S to 0.6.\nSet item 7 of S to 0.7.\nSet item 8 of S to 0.8.\n",
        "Let GN be a 1 by 2 tensor of Real.\nSet item 1 of GN to 1.0.\nSet item 2 of GN to 2.0.\n",
        "Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with 2 heads 1 groups.\n",
        "Show round(item 1 of O * 1000000).\nShow round(item 2 of O * 1000000).\n",
        "Show round(item 3 of O * 1000000).\nShow round(item 4 of O * 1000000).\n",
    ),
    shows: 4,
    needs: &["flat_affine", "row_reduce", "state"],
    expect: None,
};
// ---- ops that were already in the language but MISSING from this matrix ----------------------
// C = attention(Q, K, V) — single-query GQA decode over a KV cache. 2 query heads, 1 KV head, hd 2.
pub const ATTENTION: OpProbe = OpProbe {
    op: "attention", family: "attn_", kind: "Attention",
    program: concat!(
        "## Main\n",
        "Let Q be a 1 by 4 tensor of Real.\n",
        "Set item 1 of Q to 1.0.\nSet item 2 of Q to 0.0.\nSet item 3 of Q to 0.0.\nSet item 4 of Q to 1.0.\n",
        "Let K be a 2 by 2 tensor of Real.\n",
        "Set item 1 of K to 1.0.\nSet item 2 of K to 1.0.\nSet item 3 of K to 0.0.\nSet item 4 of K to 2.0.\n",
        "Let V be a 2 by 2 tensor of Real.\n",
        "Set item 1 of V to 3.0.\nSet item 2 of V to 4.0.\nSet item 3 of V to 5.0.\nSet item 4 of V to 6.0.\n",
        "Let C be attention of Q over K and V with 2 heads 1 groups.\n",
        "Show round(item 1 of C * 1000000).\nShow round(item 2 of C * 1000000).\n",
        "Show round(item 3 of C * 1000000).\nShow round(item 4 of C * 1000000).\n",
    ),
    shows: 4,
    needs: &["flat_affine", "select", "row_reduce", "state"],
    expect: None,
};
// H = moe(X, Wr, Wg, Wu, Wd) — top-1-of-2 Mixture-of-Experts FFN, d=2, dff=2.
pub const MOE: OpProbe = OpProbe {
    op: "moe", family: "moe", kind: "Moe",
    program: concat!(
        "## Main\n",
        "Let X be a 1 by 2 tensor of Real.\nSet item 1 of X to 0.5.\nSet item 2 of X to 1.5.\n",
        "Let Wr be a 2 by 2 tensor of Real.\n",
        "Set item 1 of Wr to 1.0.\nSet item 2 of Wr to 0.0.\nSet item 3 of Wr to 0.0.\nSet item 4 of Wr to 1.0.\n",
        "Let Wg be a 4 by 2 tensor of Real.\n",
        "Set item 1 of Wg to 0.5.\nSet item 2 of Wg to 0.25.\nSet item 3 of Wg to 0.75.\nSet item 4 of Wg to 1.0.\n",
        "Set item 5 of Wg to 0.1.\nSet item 6 of Wg to 0.2.\nSet item 7 of Wg to 0.3.\nSet item 8 of Wg to 0.4.\n",
        "Let Wu be a 4 by 2 tensor of Real.\n",
        "Set item 1 of Wu to 1.0.\nSet item 2 of Wu to 0.5.\nSet item 3 of Wu to 0.25.\nSet item 4 of Wu to 0.75.\n",
        "Set item 5 of Wu to 0.6.\nSet item 6 of Wu to 0.7.\nSet item 7 of Wu to 0.8.\nSet item 8 of Wu to 0.9.\n",
        "Let Wd be a 4 by 2 tensor of Real.\n",
        "Set item 1 of Wd to 1.0.\nSet item 2 of Wd to 0.0.\nSet item 3 of Wd to 0.0.\nSet item 4 of Wd to 1.0.\n",
        "Set item 5 of Wd to 0.5.\nSet item 6 of Wd to 0.5.\nSet item 7 of Wd to 0.5.\nSet item 8 of Wd to 0.5.\n",
        "Let H be moe of X with router Wr gate Wg up Wu down Wd choosing 1 of 2.\n",
        "Show round(item 1 of H * 1000000).\nShow round(item 2 of H * 1000000).\n",
    ),
    shows: 2,
    needs: &["argmax", "gather", "row_reduce"],
    expect: None,
};

// ---- sugars the language has that the matrix never named ------------------------------------
// Y = softmax(X) PER ROW — a 2-D tensor takes the per-row form (`rows > 1`); row 2 is uniform → 1/3 each.
pub const SOFTMAX_ROWS: OpProbe = OpProbe {
    op: "softmax_rows", family: "softmax", kind: "FusedRow",
    program: concat!(
        "## Main\n",
        "Let X be a 2 by 3 tensor of Real.\n",
        "Set item 1 of X to 1.0.\nSet item 2 of X to 2.0.\nSet item 3 of X to 3.0.\n",
        "Set item 4 of X to 1.0.\nSet item 5 of X to 1.0.\nSet item 6 of X to 1.0.\n",
        "Let Y be softmax of X.\n",
        "Show round(item 1 of Y * 1000000).\nShow round(item 2 of Y * 1000000).\nShow round(item 3 of Y * 1000000).\n",
        "Show round(item 4 of Y * 1000000).\nShow round(item 5 of Y * 1000000).\nShow round(item 6 of Y * 1000000).\n",
    ),
    shows: 6,
    needs: &["row_reduce"],
    expect: None,
};
// Kt = transpose(K) — an `R by C` tensor becomes `C by R`, `Kt[j][i] = K[i][j]`; attention's `Q · Kᵀ`.
// The sugar builds a `Seq of Real`, so the probe is a Real tensor shown through the ×1e6 rounding.
pub const TRANSPOSE: OpProbe = OpProbe {
    op: "transpose", family: "transpose", kind: "Transpose",
    program: concat!(
        "## Main\n",
        "Let K be a 2 by 3 tensor of Real.\n",
        "Set item 1 of K to 1.0.\nSet item 2 of K to 2.0.\nSet item 3 of K to 3.0.\n",
        "Set item 4 of K to 4.0.\nSet item 5 of K to 5.0.\nSet item 6 of K to 6.0.\n",
        "Let Kt be transpose of K.\n",
        "Show round(item 1 of Kt * 1000000).\nShow round(item 2 of Kt * 1000000).\nShow round(item 3 of Kt * 1000000).\n",
        "Show round(item 4 of Kt * 1000000).\nShow round(item 5 of Kt * 1000000).\nShow round(item 6 of Kt * 1000000).\n",
    ),
    shows: 6,
    needs: &["flat_affine"],
    expect: Some(&["1000000", "4000000", "2000000", "5000000", "3000000", "6000000"]),
};
// A = decay(Ar, Dt, Alog) — Gated DeltaNet's per-head forget gate `(1 + e^(Ar+Dt))^Alog`, one value per head.
pub const DECAY: OpProbe = OpProbe {
    op: "decay", family: "decay", kind: "Decay",
    program: concat!(
        "## Main\n",
        "Let Ar be a 1 by 2 tensor of Real.\nSet item 1 of Ar to 0.5.\nSet item 2 of Ar to -1.0.\n",
        "Let Dt be a 1 by 2 tensor of Real.\nSet item 1 of Dt to 0.1.\nSet item 2 of Dt to 0.2.\n",
        "Let Al be a 1 by 2 tensor of Real.\nSet item 1 of Al to -0.5.\nSet item 2 of Al to -1.0.\n",
        "Let A be decay of Ar with bias Dt rate Al.\n",
        "Show round(item 1 of A * 1000000).\nShow round(item 2 of A * 1000000).\n",
    ),
    shows: 2,
    needs: &["ln"],
    expect: None,
};
// Y = silu(X) — `x·σ(x)`, elementwise. The SwiGLU gate activation on its own.
pub const SILU: OpProbe = OpProbe {
    op: "silu",
    family: "silu",
    kind: "Silu",
    program: real_probe!("Let Y be silu of X."),
    shows: 4,
    needs: &[],
    expect: None,
};
// Y = l2norm(X) ACROSS HEADS — a flat `1 by (H·D)` row normalized per head: [3,4 | 6,8] → [.6,.8 | .6,.8].
pub const L2NORM_HEADS: OpProbe = OpProbe {
    op: "l2norm_heads", family: "l2norm", kind: "L2Norm",
    program: concat!(
        "## Main\n",
        "Let X be a 1 by 4 tensor of Real.\n",
        "Set item 1 of X to 3.0.\nSet item 2 of X to 4.0.\nSet item 3 of X to 6.0.\nSet item 4 of X to 8.0.\n",
        "Let Y be l2norm of X across 2 heads.\n",
        "Show round(item 1 of Y * 1000000).\nShow round(item 2 of Y * 1000000).\n",
        "Show round(item 3 of Y * 1000000).\nShow round(item 4 of Y * 1000000).\n",
    ),
    shows: 4,
    needs: &["flat_affine", "row_reduce"],
    expect: None,
};

// ---- sugars the plan schedules (B-P5 gather, B-P6 state) — RED at the parser until they land ----------
// x = row tok of Emb — the embedding lookup: a data-indexed row gather out of the token table.
pub const ROW_OF: OpProbe = OpProbe {
    op: "row_of",
    family: "row_gather",
    kind: "RowGather",
    program: concat!(
        "## Main\n",
        "Let Emb be a 3 by 2 tensor of Real.\n",
        "Set item 1 of Emb to 1.0.\nSet item 2 of Emb to 2.0.\nSet item 3 of Emb to 3.0.\n",
        "Set item 4 of Emb to 4.0.\nSet item 5 of Emb to 5.0.\nSet item 6 of Emb to 6.0.\n",
        "Let tok be 2.\n",
        "Let x be row tok of Emb.\n",
        "Show round(item 1 of x * 1000000).\nShow round(item 2 of x * 1000000).\n",
    ),
    shows: 2,
    needs: &["gather"],
    expect: None,
};
// X = rows toks of Emb — the block of lookups: every token of a chunk's row of the table at once.
pub const ROWS_OF: OpProbe = OpProbe {
    op: "rows_of",
    family: "rows_gather",
    kind: "RowsGather",
    program: concat!(
        "## Main\n",
        "Let Emb be a 3 by 2 tensor of Real.\n",
        "Set item 1 of Emb to 1.0.\nSet item 2 of Emb to 2.0.\nSet item 3 of Emb to 3.0.\n",
        "Set item 4 of Emb to 4.0.\nSet item 5 of Emb to 5.0.\nSet item 6 of Emb to 6.0.\n",
        "Let toks be a 1 by 2 tensor of Int.\n",
        "Set item 1 of toks to 3.\nSet item 2 of toks to 1.\n",
        "Let X be rows toks of Emb.\n",
        "Show round(item 1 of X * 1000000).\nShow round(item 2 of X * 1000000).\n",
        "Show round(item 3 of X * 1000000).\nShow round(item 4 of X * 1000000).\n",
    ),
    shows: 4,
    needs: &["gather"],
    expect: None,
};
// t = argmax of X — the index of the first maximal entry (`next` token, MoE top-k).
pub const ARGMAX: OpProbe = OpProbe {
    op: "argmax", family: "argmax", kind: "ArgMax",
    program: concat!(
        "## Main\n",
        "Let X be a 1 by 4 tensor of Real.\n",
        "Set item 1 of X to 0.5.\nSet item 2 of X to 2.5.\nSet item 3 of X to 2.5.\nSet item 4 of X to 1.0.\n",
        "Let t be argmax of X.\n",
        "Show t.\n",
    ),
    shows: 1,
    needs: &["argmax"],
    expect: None,
};
// Set row p of Kc to kp — the KV-cache append: a data-indexed row scatter into a persistent table.
pub const SET_ROW: OpProbe = OpProbe {
    op: "set_row", family: "set_row", kind: "SetRow",
    program: concat!(
        "## Main\n",
        "Let Kc be a 3 by 2 tensor of Real.\n",
        "Let kp be a 1 by 2 tensor of Real.\nSet item 1 of kp to 7.0.\nSet item 2 of kp to 8.0.\n",
        "Let p be 1.\n",
        "Set row p of Kc to kp.\n",
        "Show round(item 1 of Kc * 1000000).\nShow round(item 2 of Kc * 1000000).\nShow round(item 3 of Kc * 1000000).\n",
        "Show round(item 4 of Kc * 1000000).\nShow round(item 5 of Kc * 1000000).\nShow round(item 6 of Kc * 1000000).\n",
    ),
    shows: 6,
    needs: &["gather", "state"],
    expect: None,
};

// C = (A · 104593) >> 16 — an int8 requant's multiply and arithmetic shift over [1,2,3,4]: [1,3,4,6].
pub const INT_REQUANT: OpProbe = OpProbe {
    op: "int_requant", family: "int_requant", kind: "IntRequant",
    program: concat!(
        "## Main\n",
        "Let A be a 2 by 2 tensor of Int.\n",
        "Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n",
        "Let C be A times 104593 shifted right by 16.\n",
        "Show item 1 of C.\nShow item 2 of C.\nShow item 3 of C.\nShow item 4 of C.\n",
    ),
    shows: 4,
    needs: &[],
    expect: Some(&["1", "3", "4", "6"]),
};
// C = A clamped to [2, 3] over [1,2,3,4]: [2,2,3,3].
pub const INT_CLAMP: OpProbe = OpProbe {
    op: "int_clamp", family: "int_clamp", kind: "IntClamp",
    program: concat!(
        "## Main\n",
        "Let A be a 2 by 2 tensor of Int.\n",
        "Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n",
        "Let C be A clamped between 2 and 3.\n",
        "Show item 1 of C.\nShow item 2 of C.\nShow item 3 of C.\nShow item 4 of C.\n",
    ),
    shows: 4,
    needs: &[],
    expect: Some(&["2", "2", "3", "3"]),
};
// C = relu of A over [-2,3,-4,5]: [0,3,0,5].
pub const RELU: OpProbe = OpProbe {
    op: "relu", family: "relu", kind: "Relu",
    program: concat!(
        "## Main\n",
        "Let A be a 2 by 2 tensor of Int.\n",
        "Set item 1 of A to -2.\nSet item 2 of A to 3.\nSet item 3 of A to -4.\nSet item 4 of A to 5.\n",
        "Let C be relu of A.\n",
        "Show item 1 of C.\nShow item 2 of C.\nShow item 3 of C.\nShow item 4 of C.\n",
    ),
    shows: 4,
    needs: &[],
    expect: Some(&["0", "3", "0", "5"]),
};
// C = the int8 tiny transformer's attention over a cache of 3 rows up to position 2: integer scores, a weight read
// from the exp table `E` at the requantized distance from the row max, and the floor-divided weighted sum.
pub const LUT_ATTENTION: OpProbe = OpProbe {
    op: "lut_attention", family: "lut_attn_", kind: "LutAttention",
    program: concat!(
        "## Main\n",
        "Let Q be a 1 by 2 tensor of Int.\nSet item 1 of Q to 100.\nSet item 2 of Q to -2.\n",
        "Let K be a 3 by 2 tensor of Int.\nSet item 1 of K to 3.\nSet item 4 of K to -100.\n",
        "Let V be a 3 by 2 tensor of Int.\nSet item 1 of V to -9.\nSet item 4 of V to 11.\nSet item 6 of V to -128.\n",
        "Let E be a 1 by 2 tensor of Int.\nSet item 1 of E to 32768.\nSet item 2 of E to 1.\n",
        "Let at be 2.\n",
        "Let C be lut attention of Q over K and V with 1 heads 1 groups up to at with table E and scale 21076 shifted right by 6.\n",
        "Show item 1 of C.\nShow item 2 of C.\n",
    ),
    shows: 2,
    needs: &["gather", "select", "row_reduce"],
    expect: None,
};

/// Every probe — the universe the spatial ledger ratchets over.
pub const OP_PROBES: &[OpProbe] = &[
    MATMUL,
    PLUS,
    HADAMARD,
    RMSNORM,
    SOFTMAX,
    RMSNORM_WEIGHTED,
    FUSED_ADD_RMSNORM,
    LAYERNORM,
    SWIGLU,
    SIGMOID,
    GELU,
    L2NORM,
    ROPE_PARTIAL,
    CAUSAL_CONV,
    FULL_ATTENTION,
    GATED_DELTA,
    ATTENTION,
    MOE,
    SOFTMAX_ROWS,
    TRANSPOSE,
    DECAY,
    SILU,
    L2NORM_HEADS,
    ROW_OF,
    ROWS_OF,
    ARGMAX,
    SET_ROW,
    SCALE,
    SOFTCAP,
    HEAD_SLICE,
    JOIN,
    HEAD_MATMUL,
    INT_REQUANT,
    INT_CLAMP,
    RELU,
    LUT_ATTENTION,
];

/// The probes whose sugar the parser accepts today — what every executing tier can be asked to run.
pub const IMPLEMENTED_PROBES: &[OpProbe] = &[
    MATMUL,
    PLUS,
    HADAMARD,
    RMSNORM,
    SOFTMAX,
    RMSNORM_WEIGHTED,
    FUSED_ADD_RMSNORM,
    LAYERNORM,
    SWIGLU,
    SIGMOID,
    GELU,
    L2NORM,
    ROPE_PARTIAL,
    CAUSAL_CONV,
    FULL_ATTENTION,
    GATED_DELTA,
    ATTENTION,
    MOE,
    SOFTMAX_ROWS,
    TRANSPOSE,
    DECAY,
    SILU,
    L2NORM_HEADS,
    ROW_OF,
    ROWS_OF,
    ARGMAX,
    SET_ROW,
    SCALE,
    SOFTCAP,
    HEAD_SLICE,
    JOIN,
    HEAD_MATMUL,
    INT_REQUANT,
    INT_CLAMP,
    RELU,
    LUT_ATTENTION,
];

/// The probes the seven-backend coverage matrix runs: every implemented sugar. A sugar may be held out of
/// the matrix only while the spatial ledger's `MISRECOGNIZED` table lists it as a fragment miscompile — the
/// matrix would otherwise enshrine the fragment's kernel in `FUSED_KERNEL_OPS` as the op's own;
/// `the_probe_tables_partition_the_universe` insists on exactly that correspondence.
pub const BACKEND_MATRIX_PROBES: &[OpProbe] = &[
    MATMUL,
    PLUS,
    HADAMARD,
    RMSNORM,
    SOFTMAX,
    RMSNORM_WEIGHTED,
    FUSED_ADD_RMSNORM,
    LAYERNORM,
    SWIGLU,
    SIGMOID,
    GELU,
    L2NORM,
    ROPE_PARTIAL,
    CAUSAL_CONV,
    FULL_ATTENTION,
    GATED_DELTA,
    ATTENTION,
    MOE,
    SOFTMAX_ROWS,
    TRANSPOSE,
    DECAY,
    SILU,
    L2NORM_HEADS,
    ROW_OF,
    ROWS_OF,
    ARGMAX,
    SET_ROW,
    SCALE,
    SOFTCAP,
    HEAD_SLICE,
    JOIN,
    HEAD_MATMUL,
    INT_REQUANT,
    INT_CLAMP,
    RELU,
    LUT_ATTENTION,
];

/// The probes whose sugar the plan schedules (B-P5/B-P6): the parser refuses them until then.
pub const PENDING_SUGAR_PROBES: &[OpProbe] = &[];

/// Every parser `fn desugar_tensor_<name>` → a probe that exercises it. A sugar may have several rows
/// (`elementwise` is `plus` AND `hadamard`); every row's sugar must exist in the parser and every row's probe
/// in `OP_PROBES`. `new` is the `Let A be a R by C tensor of T` declaration every probe opens with.
pub const SUGAR_TO_PROBE: &[(&str, &str)] = &[
    ("new", "matmul"),
    ("matmul", "matmul"),
    ("elementwise", "plus"),
    ("elementwise", "hadamard"),
    ("rmsnorm", "rmsnorm"),
    ("rmsnorm_weighted", "rmsnorm_weighted"),
    ("rmsnorm_weighted", "fused_add_rmsnorm"),
    ("layernorm", "layernorm"),
    ("softmax", "softmax"),
    ("softmax_rows", "softmax_rows"),
    ("rope", "rope_partial"),
    ("silu", "silu"),
    ("sigmoid", "sigmoid"),
    ("gelu", "gelu"),
    ("decay", "decay"),
    ("l2norm", "l2norm"),
    ("l2norm", "l2norm_heads"),
    ("swiglu", "swiglu"),
    ("causal_conv", "causal_conv"),
    ("full_attention", "full_attention"),
    ("gated_delta", "gated_delta"),
    ("moe", "moe"),
    ("attention", "attention"),
    ("attention_block", "attention"),
    ("transpose", "transpose"),
    ("row", "row_of"),
    ("rows", "rows_of"),
    ("argmax", "argmax"),
    ("set_row", "set_row"),
    ("scale", "scale"),
    ("cap", "softcap"),
    ("head_slice", "head_slice"),
    ("join", "join"),
    ("head_matmul", "head_matmul"),
    ("map", "int_requant"),
    ("map", "int_clamp"),
    ("map", "relu"),
    ("lut_attention", "lut_attention"),
];

/// The probe named `op`, or a panic naming the miss (a ledger row can never point at nothing).
pub fn probe(op: &str) -> &'static OpProbe {
    OP_PROBES
        .iter()
        .find(|p| p.op == op)
        .unwrap_or_else(|| panic!("no probe named `{op}`"))
}

/// The parser source every sugar lives in, read from the sibling crate.
pub fn parser_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_language/src/parser/mod.rs");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Rust source with every `//` line comment and `/* */` block comment removed — so a sugar mentioned in a
/// comment is not counted as one that exists.
pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else {
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Every `<name>` for which `fn desugar_tensor_<name>(` is DEFINED in `src` (comments stripped first).
pub fn desugar_sugars_in(src: &str) -> BTreeSet<String> {
    const NEEDLE: &str = "fn desugar_tensor_";
    let clean = strip_comments(src);
    let mut found = BTreeSet::new();
    for (at, _) in clean.match_indices(NEEDLE) {
        let rest = &clean[at + NEEDLE.len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && rest[name.len()..].starts_with('(') {
            found.insert(name);
        }
    }
    found
}

// ── Oracle projection: what a program's `Show` lines print, read off a spatial evaluation ─────────────

/// One `Show` of a probe program: which tensor, which flat (0-based) element, and the `round(· × scale)`
/// the source applies before printing (`None` for a raw integer `Show`).
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub tensor: String,
    pub index: usize,
    pub scale: Option<f64>,
}

/// The `Show` lines of a probe program, in order. Accepts the three spellings the probes use:
/// `Show item N of T.`, `Show round(item N of T * S).`, and a bare `Show T.` (a scalar result).
pub fn shows_of(program: &str) -> Vec<Shown> {
    let mut out = Vec::new();
    for line in program.lines() {
        let Some(rest) = line.trim().strip_prefix("Show ") else {
            continue;
        };
        let rest = rest.trim_end_matches('.').trim();
        let (inner, scale) = match rest
            .strip_prefix("round(")
            .and_then(|r| r.strip_suffix(')'))
        {
            Some(r) => {
                let (item, mul) = r
                    .rsplit_once('*')
                    .unwrap_or_else(|| panic!("`{line}`: a rounded Show multiplies by a scale"));
                (
                    item.trim(),
                    Some(
                        mul.trim()
                            .parse::<f64>()
                            .unwrap_or_else(|_| panic!("`{line}`: scale is a number")),
                    ),
                )
            }
            None => (rest, None),
        };
        match inner.strip_prefix("item ") {
            Some(r) => {
                let (n, tensor) = r
                    .split_once(" of ")
                    .unwrap_or_else(|| panic!("`{line}`: `item N of T`"));
                let n: usize = n
                    .trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("`{line}`: item index is an integer"));
                assert!(n >= 1, "`{line}`: LOGOS items are 1-based");
                out.push(Shown {
                    tensor: tensor.trim().to_string(),
                    index: n - 1,
                    scale,
                });
            }
            None => out.push(Shown {
                tensor: inner.to_string(),
                index: 0,
                scale,
            }),
        }
    }
    out
}

/// Inputs for a program whose tensors carry deterministic fills: every non-Output tensor is materialized
/// from its recorded init (the same construction the tests-crate spatial locks use). A tensor whose init
/// is not deterministic (`External`, a random fill) is an `Err` naming it — a verdict must be able to say
/// "no inputs" rather than abort, because a mis-recognized program can carry such a tensor.
pub fn inputs_from_fills(p: &SpatialProgram) -> Result<Inputs, String> {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let n = t.elems.rows * t.elems.cols;
        if t.init.is_float() {
            let vals = (0..n)
                .map(|i| t.init.element_f64(i))
                .collect::<Option<Vec<f64>>>();
            inputs.tensors.insert(
                t.id,
                Value::Float(vals.ok_or_else(|| {
                    format!(
                        "`{}` has no deterministic float fill ({:?})",
                        t.name, t.init
                    )
                })?),
            );
        } else {
            let vals = (0..n)
                .map(|i| t.init.element(i))
                .collect::<Option<Vec<i64>>>();
            inputs.tensors.insert(
                t.id,
                Value::Int(vals.ok_or_else(|| {
                    format!("`{}` has no deterministic fill ({:?})", t.name, t.init)
                })?),
            );
        }
    }
    Ok(inputs)
}

/// The program's single Output tensor.
pub fn output_id(p: &SpatialProgram) -> TensorId {
    p.tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .expect("an output tensor")
        .id
}

/// The tensor a `Show` names: by name when the program declares it, else the program's single Output (a
/// lowering may rename the result while the source keeps the sugar's binder).
pub fn shown_tensor<'p>(p: &'p SpatialProgram, name: &str) -> Result<&'p TensorDecl, String> {
    if let Some(t) = p.tensors.iter().find(|t| t.name == name) {
        return Ok(t);
    }
    let outs: Vec<&TensorDecl> = p
        .tensors
        .iter()
        .filter(|t| t.role == TensorRole::Output)
        .collect();
    match outs.as_slice() {
        [only] => Ok(only),
        _ => Err(format!("`Show` names `{name}`, which the spatial program does not declare, and it has {} outputs", outs.len())),
    }
}

/// The integers the program's `Show` lines print when its result tensor holds `outputs` — the spatial answer
/// as the target's float datapath stores it: `round(round_to(dtype, x) × scale)`.
pub fn shown_from_outputs(
    program: &str,
    p: &SpatialProgram,
    outputs: &Outputs,
) -> Result<Vec<i64>, String> {
    let mut got = Vec::new();
    for s in shows_of(program) {
        let t = shown_tensor(p, &s.tensor)?;
        let v = outputs
            .tensors
            .get(&t.id)
            .ok_or_else(|| format!("the evaluation produced no value for `{}`", t.name))?;
        let x = match (v, s.scale) {
            (Value::Int(v), None) => *v
                .get(s.index)
                .ok_or_else(|| format!("`{}` has no element {}", t.name, s.index))?,
            (Value::Int(v), Some(scale)) => (*v
                .get(s.index)
                .ok_or_else(|| format!("`{}` has no element {}", t.name, s.index))?
                as f64
                * scale)
                .round() as i64,
            (Value::Float(v), Some(scale)) => (round_to(
                t.dtype,
                *v.get(s.index)
                    .ok_or_else(|| format!("`{}` has no element {}", t.name, s.index))?,
            ) * scale)
                .round() as i64,
            (Value::Float(v), None) => *v
                .get(s.index)
                .ok_or_else(|| format!("`{}` has no element {}", t.name, s.index))?
                as i64,
        };
        got.push(x);
    }
    Ok(got)
}

/// The integers the VM printed.
pub fn oracle_ints(output: &str) -> Result<Vec<i64>, String> {
    output
        .split_whitespace()
        .map(|t| {
            t.parse::<i64>()
                .map_err(|_| format!("the oracle printed a non-integer `{t}`"))
        })
        .collect()
}

/// The comparison rule shared with `spatial_front_end_lock`: a raw integer `Show` must agree exactly; a
/// rounded float `Show` must agree with the VM's value as the target's datapath stores it, within one ulp of
/// the ×scale rounding. Lengths must match — a program that shows fewer values than the oracle is not ≡ it.
pub fn agrees_with_oracle(
    program: &str,
    p: &SpatialProgram,
    outputs: &Outputs,
    oracle_stdout: &str,
) -> Result<(), String> {
    let got = shown_from_outputs(program, p, outputs)?;
    let vm = oracle_ints(oracle_stdout)?;
    let shows = shows_of(program);
    if got.len() != vm.len() || vm.len() != shows.len() {
        return Err(format!(
            "spatial showed {} values, the VM {}, the program has {} Shows",
            got.len(),
            vm.len(),
            shows.len()
        ));
    }
    for (i, ((g, v), s)) in got.iter().zip(vm.iter()).zip(shows.iter()).enumerate() {
        match s.scale {
            None => {
                if g != v {
                    return Err(format!(
                        "Show #{}: spatial {g} vs VM {v} (exact integer)",
                        i + 1
                    ));
                }
            }
            Some(scale) => {
                let dtype = shown_tensor(p, &s.tensor)?.dtype;
                let want = (round_to(dtype, *v as f64 / scale) * scale).round() as i64;
                if (g - want).abs() > 1 {
                    return Err(format!("Show #{}: spatial {g} vs VM {v} at {dtype:?} = {want} (one ulp of the ×{scale} rounding allowed)", i + 1));
                }
            }
        }
    }
    Ok(())
}
