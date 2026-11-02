//! The CUDA backend's optimization **knob lattice**.
//!
//! Design principle (the discipline this whole tier is held to):
//!
//! > **Every optimization is a trade-off.** If a transform is *strictly better* — Pareto-dominant, i.e.
//! > faster (or smaller) with **no** downside for any valid program — it is baked in UNCONDITIONALLY and
//! > gets **no knob** (see [`STRICTLY_BETTER`]). A knob exists *only* where the transform **wins** in some
//! > regimes and **loses** in others. Each knob then **defaults to the best general case**, and records its
//! > trade-off ([`CudaKnobs::tradeoffs`]) so an autotuner — or a human — can search the lattice with eyes
//! > open, never blindly.
//!
//! This mirrors the machinery the codebase already uses: [`crate::codegen_sva::fabric_target::FabricTarget`]
//! (a capability lattice), `spatial::passes::tile::best_tile_config` (a cost-model search over a `TileConfig`
//! lattice), and the published Pareto frontier. The knobs here are the same idea aimed at the CUDA emitters:
//! the GEMV/GEMM kernels *and* (as they land) the transformer orchestration (KV-cache, CUDA-graph capture,
//! quant fusion). The set of legal knob assignments *is* a lattice; [`CudaKnobs::default`] is its
//! best-general point and every other point is reachable by flipping a documented trade-off.

use crate::codegen_cuda::chip::CudaChip;
use crate::codegen_cuda::qwen_emit::Q8MatKind;
use crate::codegen_sva::fabric_target::Precision;

/// GEMV/GEMM weight-load width. Wider loads expose more memory-level parallelism (the Q8_0 forward is
/// weight-**bandwidth**-bound), but cost registers per thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadWidth {
    /// 128-byte `char4` loads. Ties `Int4` on tiny N; lighter register pressure.
    Char4,
    /// 512-byte `int4` (16-byte) loads — measured winner on the large-N projections (92% of the 3060's peak
    /// BW vs 87% for `char4`); heavier register pressure on small tiles.
    Int4,
}

/// Contraction precision for a lowered matmul.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatmulPrecision {
    /// Best **general** case: pick the always-correct path unless the value analysis *proves* a faster one is
    /// safe. Today resolves to [`ResolvedPrecision::Int64`] (we do not yet carry a proof that operands fit
    /// int8 through the recognizer), so `Auto` is exactly "the correct-for-any-program default".
    Auto,
    /// Signed-int8 `__dp4a` accumulation — faster, but VALID ONLY when operands provably fit int8 and the
    /// contraction fits int32. Not strictly better (restricted domain) → a knob, off by default.
    Int8Dp4a,
    /// 64-bit integer accumulation — always correct (matches the VM's i64 semantics for any operands), slower.
    Int64,
    /// **bf16 tensor-core** GEMM (HMMA 16×16×16, bf16 operands, f32 accumulate) — the half-precision fast path
    /// for a matmul over `Real` tensors on a 16-aligned shape (sm_80+). When the block tile *divides* the shape
    /// and its footprint fits static shared memory, the lowering auto-selects the cp.async double-buffered
    /// kernel (the measured 1.43× lever, `select_hmma_kernel`); otherwise the basic one-warp WMMA kernel. On an
    /// incapable shape/arch it falls back to the always-correct int64 kernel. A knob (restricted domain), off by default.
    Bf16Hmma,
    /// **fp16 tensor-core** GEMM (HMMA 16×16×16, f16 operands, f32 accumulate) — the fp16 counterpart (sm_70+).
    Fp16Hmma,
}

/// What a [`MatmulPrecision`] actually lowers to after `Auto` is resolved against what we can prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedPrecision {
    Int8Dp4a,
    Int64,
    Bf16Hmma,
    Fp16Hmma,
}

impl MatmulPrecision {
    /// Resolve the knob to a concrete lowering. `Auto` conservatively picks the always-correct `Int64` — we
    /// only emit a fast path (`Int8Dp4a`/`Bf16Hmma`/`Fp16Hmma`) when the caller *explicitly* requests it,
    /// which is precisely "default to best general; opt into the trade-off".
    pub fn resolved(self) -> ResolvedPrecision {
        match self {
            MatmulPrecision::Int8Dp4a => ResolvedPrecision::Int8Dp4a,
            MatmulPrecision::Bf16Hmma => ResolvedPrecision::Bf16Hmma,
            MatmulPrecision::Fp16Hmma => ResolvedPrecision::Fp16Hmma,
            MatmulPrecision::Auto | MatmulPrecision::Int64 => ResolvedPrecision::Int64,
        }
    }
}

/// The CUDA backend's optimization knobs — one point in the lattice. Construct with [`CudaKnobs::default`]
/// for the best-general point and flip individual knobs to trade off. Every field is a trade-off; there are
/// deliberately **no** fields for strictly-better transforms (those are always applied, see [`STRICTLY_BETTER`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CudaKnobs {
    /// Weight-load width for GEMV/GEMM. Default [`LoadWidth::Int4`] (measured best-general on the projections).
    pub gemv_load: LoadWidth,
    /// Contraction precision. Default [`MatmulPrecision::Auto`] (⇒ always-correct int64).
    pub precision: MatmulPrecision,
    /// KV-cache for autoregressive generation — skip recomputing past K/V. A large win for generation, but
    /// costs VRAM and only applies to attention over a growing sequence. Default `true`.
    pub kv_cache: bool,
    /// Capture the steady-state (T=1) step as a CUDA graph and replay it — collapses ~540 launches/token to
    /// one. Wins when many tiny kernels make launch latency dominate; neutral-to-negative for one big kernel.
    /// Default `true`.
    pub cuda_graphs: bool,
    /// Fuse activation-quantization into the preceding norm/elementwise kernel — saves a global round-trip at
    /// the cost of a slightly larger, less-reusable kernel. Default `true`.
    pub fuse_quant: bool,
    /// The activation precision of a projection by Q8_0 weights in the compiled generation lane: `f32` activations
    /// with `f64` sums (`Dequant`), a high and a low int8 word per value (`TwoPassInt8`, ~14 bits), or one int8 word
    /// (`Int8Act`, llama.cpp's precision). Default [`Q8MatKind::TwoPassInt8`].
    pub activations: Q8MatKind,
    /// The GPU part this kernel targets — its SM arch, shared-memory budget, and realized precisions. Default
    /// [`CudaChip::rtx3060`] (the local dev GPU). Flipping it to [`CudaChip::b200`] retargets the SAME
    /// recognized op to Blackwell. A trade-off (portability vs hardware exploitation), hence a knob.
    pub chip: CudaChip,
    /// The precision the compiled generation lane stores a gated-delta layer's recurrent memory in between steps:
    /// `f32`, or bf16 — computed in `f32`, rounded to bf16 after every row the delta rule walks. Default
    /// [`StatePrecision::F32`].
    pub recurrent_state: StatePrecision,
    /// The precision the compiled generation lane stores an attention's key and value cache in: `f32`, or binary16 —
    /// each row computed in `f32` and rounded to the nearest as it is written, every read of the cache taking the
    /// rounded row. Default [`CachePrecision::F32`].
    pub cache_precision: CachePrecision,
}

/// The precision an attention's cache is stored in (`CudaKnobs::cache_precision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CachePrecision {
    /// Four bytes a value, as the step computes it.
    #[default]
    F32,
    /// Two bytes a value: binary16, rounded to the nearest, ties to even, as llama.cpp's default cache keeps it.
    F16,
}

/// The precision a recurrent memory is stored in between steps (`CudaKnobs::recurrent_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatePrecision {
    /// Four bytes a value, as the step computes it.
    #[default]
    F32,
    /// Two bytes a value: bfloat16, rounded to the nearest, ties to even.
    Bf16,
}

impl Default for CudaKnobs {
    /// The best **general** point of the lattice: wide loads, always-correct precision, and the
    /// generation-time wins (KV-cache, graph replay, quant fusion) enabled — each the right call for the
    /// common case, each overridable.
    fn default() -> Self {
        CudaKnobs {
            gemv_load: LoadWidth::Int4,
            precision: MatmulPrecision::Auto,
            kv_cache: true,
            cuda_graphs: true,
            fuse_quant: true,
            activations: Q8MatKind::TwoPassInt8,
            chip: CudaChip::rtx3060(),
            recurrent_state: StatePrecision::F32,
            cache_precision: CachePrecision::F32,
        }
    }
}

impl CudaKnobs {
    /// Map a [`MatmulPrecision`] to the fabric-level [`Precision`] descriptor (for `FabricTarget` gating).
    pub fn fabric_precision(&self) -> Precision {
        match self.precision.resolved() {
            ResolvedPrecision::Int8Dp4a => Precision::Int8,
            ResolvedPrecision::Int64 => Precision::Int64,
            ResolvedPrecision::Bf16Hmma => Precision::Bf16,
            ResolvedPrecision::Fp16Hmma => Precision::Fp16,
        }
    }

    /// The trade-off table — one row per knob, each naming the regime where the *aggressive* setting **wins**
    /// and the regime where it **loses** (the reason it can't be baked in unconditionally). This is the
    /// lattice's map; the knob-lattice lock asserts it stays complete and honest.
    pub fn tradeoffs() -> &'static [KnobTradeoff] {
        &[
            KnobTradeoff {
                knob: "gemv_load",
                default: "Int4",
                wins: "large-N projections — 512-byte loads hit 92% of peak bandwidth vs 87% for char4",
                loses: "tiny N / small tiles — the wider load ties char4 but spends more registers",
            },
            KnobTradeoff {
                knob: "precision",
                default: "Auto (⇒ int64, always correct, scalar — no tensor cores)",
                wins: "tensor-core precisions on a 16-aligned shape: int8 IMMA is fastest (density — 1-byte \
                       operands, ~2× the bf16 rate; MEASURED 2.10× on the 3060 at 1024³, cuda_precision_tradeoff_lock); \
                       bf16/f16 HMMA give an fp32-accumulate wide-exponent range for Real tensors. A tile-dividing \
                       shape auto-selects the cp.async double-buffered kernel (measured 1.43× over basic WMMA)",
                loses: "int8 needs operands that provably fit [-128,127] and a contraction that fits int32 (else wrong \
                        answer); bf16/f16 halve throughput vs int8 for the range; a ragged/small-arch shape falls back \
                        to the always-correct scalar int64 kernel — no tensor cores at all",
            },
            KnobTradeoff {
                knob: "kv_cache",
                default: "on",
                wins: "autoregressive generation — past K/V are reused instead of recomputed each token",
                loses: "prefill / one-shot with no growing sequence — pure VRAM cost, no reuse",
            },
            KnobTradeoff {
                knob: "cuda_graphs",
                default: "on",
                wins: "many tiny per-token kernels — one graph launch replaces ~540 launches",
                loses: "a single large kernel — capture/replay overhead with nothing to amortize it over",
            },
            KnobTradeoff {
                knob: "fuse_quant",
                default: "on",
                wins: "quant-then-consume chains — one global round-trip saved",
                loses: "when the un-fused quantized activation is reused by several consumers",
            },
            KnobTradeoff {
                knob: "activations",
                default: "TwoPassInt8 (a high and a low int8 word per activation, ~14 bits)",
                wins: "batch-1 decode is bound on the weight bytes, so the second word's dp4a is free: Qwen3.5-0.8B and \
                       Llama-3.2-1B follow llama.cpp's f32 run on every step of their 32-prompt corpora (llama.cpp's \
                       shipped run: 93% and 91%), Qwen3-8B decodes at 38.5 tok/s against 38.8 for one word",
                loses: "a compute-bound prefill runs two tensor-core mma per block — Int8Act halves that work at \
                        llama.cpp's precision; Dequant (f32 activations, f64 sums) is the exact reference at a tenth \
                        the speed",
            },
            KnobTradeoff {
                knob: "recurrent_state",
                default: "F32",
                wins: "a batch of gated-delta sequences moves every memory each step: bf16 halves those bytes — \
                       Qwen3.5-0.8B's delta rule at 64 sequences, 412 µs a layer to 245 on the 3060",
                loses: "the memory keeps eight bits of mantissa: every step rounds it, so the logits drift from the f32 \
                        memory's by up to about 1e-2 of their scale on the tiny models, and long generations part sooner",
            },
            KnobTradeoff {
                knob: "cache_precision",
                default: "F32",
                wins: "a batch's attention reads every sequence's whole cache each step: binary16 halves those bytes",
                loses: "the cache keeps eleven bits of significand: every key and value the attention reads is rounded, \
                        so the logits move from the f32 cache's by the rounding's share of each score",
            },
            KnobTradeoff {
                knob: "chip",
                default: "rtx3060 (the local Ampere sm_86 dev GPU)",
                wins: "targeting the actual deployment silicon — a B200 kernel uses sm_100 5th-gen tensor cores, fp8/nvfp4, and a larger shared-memory budget",
                loses: "portability — a kernel built for a newer arch will not run on an older host, and one built for an older arch leaves the newer part's features unused",
            },
        ]
    }
}

/// A single knob's trade-off — why it is a knob and not a baked-in transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnobTradeoff {
    /// The `CudaKnobs` field this row describes.
    pub knob: &'static str,
    /// Its default value, in words (the best-general choice).
    pub default: &'static str,
    /// The regime where the aggressive setting wins.
    pub wins: &'static str,
    /// The regime where it loses — the reason it cannot be baked in unconditionally.
    pub loses: &'static str,
}

/// Transforms that are **strictly better** — Pareto-dominant, never worse for any valid program — and are
/// therefore applied UNCONDITIONALLY with **no knob**. If any entry here ever acquires a downside, it must
/// grow a `CudaKnobs` field and move out of this list (the knob-lattice lock enforces the disjointness).
pub const STRICTLY_BETTER: &[&str] = &[
    "coalesced global loads — contiguous per-warp addressing (never worse than strided)",
    "scale folded into the accumulator — no separate dequant pass over the weights",
    "no host-side recompute of device-resident results — read back once",
    "native-arch codegen (-O2, compiled for the target's own SM arch) — no reason to deoptimize",
];
