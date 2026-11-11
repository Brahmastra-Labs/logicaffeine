//! The class registry: every fused symbol, primitive, edge layout and schedule property the book names, registered
//! before its rule exists, so a fusion mined from another stack has a place to land.
//!
//! A class's id is its ledger id where `benchmarks/fusion_parity.json` tracks it, and an "ours" id where no other stack
//! has it. Its kind says what it is:
//!
//!   • **fused** — built by a rule; its parts are its definition;
//!   • **primitive** — an op the front end lifts whole, which a chip links whole or runs as its parts;
//!   • **placement** — a layout on an edge: a part read in place, a result written into rows or units of another
//!     tensor;
//!   • **numerics** — a value's representation: a quantized or rotated form, a compensated sum;
//!   • **schedule** — how a chip runs something, never a fusion of meaning.
//!
//! Each class with a sugar today carries an exemplar, a minimal LOGOS program holding its pattern — the op kinds of its
//! definition, in program order. A class nothing in the language writes yet carries the reason instead.

/// What a class is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Fused,
    Primitive,
    Placement,
    Numerics,
    Schedule,
}

/// Where a class's id comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// A class `benchmarks/fusion_parity.json` tracks, mined from the stacks the book's catalog names.
    Ledger,
    /// A class the book names that the ledger does not track yet.
    Book,
    /// A class only this compiler has.
    Ours,
}

/// A class's exemplar: a minimal program holding its pattern, or why there is none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exemplar {
    Program { source: &'static str, pattern: &'static [&'static str] },
    None(&'static str),
}

/// One registered class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Class {
    pub class: ClassId,
    pub id: &'static str,
    pub kind: Kind,
    pub origin: Origin,
    pub exemplar: Exemplar,
}

const fn class(class: ClassId, id: &'static str, kind: Kind, origin: Origin, exemplar: Exemplar) -> Class {
    Class { class, id, kind, origin, exemplar }
}

const fn ex(source: &'static str, pattern: &'static [&'static str]) -> Exemplar {
    Exemplar::Program { source, pattern }
}

const NO_SUGAR: &str = "no sugar writes it yet";
const SCHEDULE: &str = "a schedule property: how a chip runs a program, which no program states";
const NUMERICS: &str = "a value's representation, which a chip's numerics choose and no program states";

use Kind::{Fused, Numerics, Placement, Primitive, Schedule};
use Origin::{Book, Ledger, Ours};

/// Every class, in the order the book's section 3.4 names them, then the "ours" folds of section 5.3: the [`ClassId`]
/// each is named by in code and its row of [`CLASSES`], from one table.
macro_rules! registry {
    ($($name:ident = $id:literal, $kind:expr, $origin:expr, $exemplar:expr;)*) => {
        /// A registered class, by name: a fused op's class, a rule's product, a shim's answer. A class added to the
        /// registry is a variant every exhaustive match over it must answer.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum ClassId {
            $($name),*
        }

        impl ClassId {
            /// Every class, in the registry's order.
            pub const ALL: &'static [ClassId] = &[$(ClassId::$name),*];

            /// The class's id: its ledger id, or its "ours" id.
            pub const fn id(self) -> &'static str {
                match self {
                    $(ClassId::$name => $id),*
                }
            }
        }

        /// Every class's row.
        pub const CLASSES: &[Class] = &[$(class(ClassId::$name, $id, $kind, $origin, $exemplar)),*];
    };
}

registry! {
    NormWeight = "norm_weight", Primitive, Ledger, ex("## Main\nLet X be a 2 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet Y be rmsnorm of X with weight W.\n", &["FusedRow"]);
    AddNorm = "add_norm", Fused, Ledger, ex(
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet Y be rmsnorm of S with weight W.\n",
        &["Elementwise", "FusedRow"],
    );
    NormThenAdd = "norm_then_add", Fused, Ledger, ex(
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet X be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet V be a 1 by 8 tensor of Real.\nLet N be rmsnorm of X with weight W.\nLet S be A plus N.\nLet Y be rmsnorm of S with weight V.\n",
        &["FusedRow", "Elementwise", "FusedRow"],
    );
    NormScale = "norm_scale", Fused, Ledger, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet N be rmsnorm of X with weight W.\nLet Y be N scaled by 0.5.\n",
        &["FusedRow", "Scale"],
    );
    NormWeightOffset = "norm_weight_offset", Fused, Book, Exemplar::None("the norm's (1 + w) is the checkpoint's binding, not a statement");
    NormQuant = "norm_quant", Fused, Ledger, Exemplar::None(NUMERICS);
    AddNormQuant = "add_norm_quant", Fused, Ledger, Exemplar::None(NUMERICS);
    DualNorm = "dual_norm", Fused, Book, ex(
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet V be a 1 by 8 tensor of Real.\nLet Y be rmsnorm of A with weight W.\nLet Z be rmsnorm of B with weight V.\n",
        &["FusedRow", "FusedRow"],
    );
    GatedNorm = "gated_norm", Fused, Book, ex(
        "## Main\nLet Q be a 1 by 64 tensor of Real.\nLet K be a 1 by 64 tensor of Real.\nLet V be a 1 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet B be a 1 by 4 tensor of Real.\nLet A be a 1 by 4 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet O be gated delta of Q over K and V with gate Z beta B decay A state S norm G with 4 heads 2 groups.\n",
        &["GatedDelta"],
    );
    EmbedNorm = "embed_norm", Fused, Book, ex(
        "## Main\nLet E be a 16 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet tok be 3.\nLet X be row tok of E.\nLet Y be rmsnorm of X with weight W.\n",
        &["RowGather", "FusedRow"],
    );
    EmbedScale = "embed_scale", Fused, Ours, ex(
        "## Main\nLet E be a 16 by 8 tensor of Real.\nLet tok be 3.\nLet X be row tok of E.\nLet Y be X scaled by 2.0.\n",
        &["RowGather", "Scale"],
    );
    GemvNormEpilogue = "gemv_norm_epilogue", Fused, Book, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet P be X matmul M.\nLet S be R plus P.\nLet Y be rmsnorm of S with weight W.\n",
        &["Matmul", "Elementwise", "FusedRow"],
    );
    PrefillNormEpilogues = "prefill_norm_epilogues", Fused, Book, ex(
        "## Main\nLet X be a 4 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet R be a 4 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet P be X matmul M.\nLet S be R plus P.\nLet Y be rmsnorm of S with weight W.\n",
        &["Matmul", "Elementwise", "FusedRow"],
    );
    AttnResidualNorm = "attn_residual_norm", Fused, Book, Exemplar::None(NO_SUGAR);
    CompensatedResidual = "compensated_residual", Numerics, Book, Exemplar::None(NUMERICS);
    RecomputeFanout = "recompute_fanout", Fused, Book, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet S be silu of X.\nLet A be S plus X.\nLet B be S hadamard X.\n",
        &["Silu", "Elementwise", "Elementwise"],
    );
    QkNormRope = "qk_norm_rope", Fused, Ledger, ex(
        "## Main\nLet Q be a 1 by 16 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet M be rmsnorm of Q with weight N across 2 heads.\nLet P be rope of M with cos RC and sin RS.\n",
        &["FusedRow", "Rope"],
    );
    QkNormRopeKvAppend = "qk_norm_rope_kv_append", Fused, Ledger, ex(
        "## Main\nLet K be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet RC be a 4 by 4 tensor of Real.\nLet RS be a 4 by 4 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet pos be 2.\nLet M be rmsnorm of K with weight N across 1 heads.\nLet P be rope of M with cos RC and sin RS from position pos across 1 heads.\nSet row pos of C to P.\n",
        &["FusedRow", "Rope", "SetRow"],
    );
    RopeKvAppend = "rope_kv_append", Fused, Ledger, ex(
        "## Main\nLet K be a 1 by 8 tensor of Real.\nLet RC be a 4 by 4 tensor of Real.\nLet RS be a 4 by 4 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet pos be 2.\nLet P be rope of K with cos RC and sin RS from position pos across 1 heads.\nSet row pos of C to P.\n",
        &["Rope", "SetRow"],
    );
    RopeQkOneLaunch = "rope_qk_one_launch", Fused, Book, ex(
        "## Main\nLet Q be a 1 by 16 tensor of Real.\nLet K be a 1 by 8 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet P be rope of Q with cos RC and sin RS.\nLet R be rope of K with cos RC and sin RS.\n",
        &["Rope", "Rope"],
    );
    QkNormRopeGateSplit = "qk_norm_rope_gate_split", Fused, Book, Exemplar::None("its pattern is a gated attention's query and gate, held by the tiny Qwen3.5 cuts");
    QkvHeadEpilogue = "qkv_head_epilogue", Fused, Book, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet K be X matmul M.\nLet H be rmsnorm of K with weight N across 1 heads.\nLet P be rope of H with cos RC and sin RS.\n",
        &["Matmul", "FusedRow", "Rope"],
    );
    HorizontalNormRopeInsert = "horizontal_norm_rope_insert", Fused, Book, Exemplar::None("its pattern is a layer's query and key branches, held by the tiny Qwen3 cuts");
    KvCacheQuant = "kv_cache_quant", Numerics, Ledger, Exemplar::None(NUMERICS);
    HadamardRotation = "hadamard_rotation", Numerics, Book, Exemplar::None(NUMERICS);
    CrossLayerLaunch = "cross_layer_launch", Fused, Book, ex(
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet D be a 4 by 8 tensor of Real.\nLet pos be 2.\nSet row pos of C to A.\nSet row pos of D to B.\n",
        &["SetRow", "SetRow"],
    );
    HostResidentKv = "host_resident_kv", Schedule, Book, Exemplar::None(SCHEDULE);
    HostWeightStreaming = "host_weight_streaming", Schedule, Book, Exemplar::None(SCHEDULE);
    HostLayersOnCpu = "host_layers_on_cpu", Schedule, Book, Exemplar::None(SCHEDULE);
    Mla = "mla", Fused, Ledger, Exemplar::None("its pattern is a whole latent attention block, held by the tiny DeepSeek-V2 cuts");
    MlaPrologue = "mla_prologue", Fused, Book, Exemplar::None("its pattern is a whole latent attention block, held by the tiny DeepSeek-V2 cuts");
    FlashDecode = "flash_decode", Primitive, Ledger, ex(
        "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet pos be 3.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos.\n",
        &["Attention"],
    );
    FlashPrefill = "flash_prefill", Primitive, Ledger, ex(
        "## Main\nLet Q be a 2 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet pos be 2.\nLet C be causal attention of Q over K and V with 2 heads 1 groups from pos.\n",
        &["Attention"],
    );
    AttentionVariantsInline = "attention_variants_inline", Primitive, Ledger, ex(
        "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet pos be 4.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos within a window of 2.\n",
        &["Attention"],
    );
    Softcap = "softcap", Primitive, Ledger, ex("## Main\nLet X be a 2 by 8 tensor of Real.\nLet Y be X capped at 30.0.\n", &["Softcap"]);
    AttnOutputQuant = "attn_output_quant", Fused, Ledger, Exemplar::None(NUMERICS);
    AttnOutputGate = "attn_output_gate", Fused, Ours, ex(
        "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet G be a 1 by 8 tensor of Real.\nLet pos be 3.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos.\nLet S be sigmoid of G.\nLet Y be C hadamard S.\n",
        &["Attention", "Sigmoid", "Elementwise"],
    );
    RowWriteQuant = "row_write_quant", Fused, Ours, Exemplar::None(NUMERICS);
    AttnGqaRowPacking = "attn_gqa_row_packing", Schedule, Book, Exemplar::None(SCHEDULE);
    AttnLseMerge = "attn_lse_merge", Schedule, Book, Exemplar::None(SCHEDULE);
    MlaUnabsorbInFold = "mla_unabsorb_in_fold", Fused, Book, Exemplar::None("its pattern is a whole latent attention block, held by the tiny DeepSeek-V2 cuts");
    MixedBatchAttention = "mixed_batch_attention", Primitive, Book, Exemplar::None(NO_SUGAR);
    SparseAttention = "sparse_attention", Primitive, Book, Exemplar::None(NO_SUGAR);
    GluActMul = "glu_act_mul", Fused, Ledger, ex(
        "## Main\nLet G be a 1 by 8 tensor of Real.\nLet U be a 1 by 8 tensor of Real.\nLet S be sigmoid of G.\nLet Y be U hadamard S.\n",
        &["Sigmoid", "Elementwise"],
    );
    GluActMulQuant = "glu_act_mul_quant", Fused, Ledger, Exemplar::None(NUMERICS);
    GemvGateUpGlu = "gemv_gate_up_glu", Fused, Ledger, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet G be a 8 by 16 tensor of Real.\nLet U be a 8 by 16 tensor of Real.\nLet A be X matmul G.\nLet B be X matmul U.\nLet H be swiglu of A and B.\n",
        &["Matmul", "Matmul", "SwiGlu"],
    );
    ElementwiseChains = "elementwise_chains", Fused, Ledger, ex(
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet C be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet Y be S hadamard C.\n",
        &["Elementwise", "Elementwise"],
    );
    ElementwiseRegions = "elementwise_regions", Fused, Book, ex(
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet P be A hadamard B.\nLet Y be S plus P.\n",
        &["Elementwise", "Elementwise", "Elementwise"],
    );
    ReluSqr = "relu_sqr", Fused, Ledger, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be relu of X.\nLet Y be R hadamard R.\n",
        &["Relu", "Elementwise"],
    );
    MaskedExpertActivation = "masked_expert_activation", Fused, Book, Exemplar::None(NO_SUGAR);
    GemmBias = "gemm_bias", Fused, Ledger, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet P be X matmul M.\nLet Y be P plus B.\n",
        &["Matmul", "Elementwise"],
    );
    GemmScale = "gemm_scale", Fused, Ledger, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet P be X matmul M.\nLet Y be P scaled by 0.25.\n",
        &["Matmul", "Scale"],
    );
    MergedQkv = "merged_qkv", Fused, Ledger, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet Q be a 8 by 8 tensor of Real.\nLet K be a 8 by 4 tensor of Real.\nLet V be a 8 by 4 tensor of Real.\nLet A be X matmul Q.\nLet B be X matmul K.\nLet C be X matmul V.\n",
        &["Matmul", "Matmul", "Matmul"],
    );
    MergedGateUp = "merged_gate_up", Fused, Ledger, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet G be a 8 by 16 tensor of Real.\nLet U be a 8 by 16 tensor of Real.\nLet A be X matmul G.\nLet B be X matmul U.\n",
        &["Matmul", "Matmul"],
    );
    GemmDequantInkernel = "gemm_dequant_inkernel", Primitive, Ledger, ex("## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet Y be X matmul M.\n", &["Matmul"]);
    AffineInputFold = "affine_input_fold", Fused, Book, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet S be X scaled by 0.5.\nLet Y be S matmul M.\n",
        &["Scale", "Matmul"],
    );
    RouterTopk = "router_topk", Primitive, Ledger, Exemplar::None("its pattern is a whole mixture, held by the tiny Qwen3-MoE cuts");
    GroupedTopk = "grouped_topk", Primitive, Book, Exemplar::None(NO_SUGAR);
    RouterSharedGateMerge = "router_shared_gate_merge", Fused, Book, Exemplar::None("its pattern is a whole mixture with a shared expert, held by the tiny DeepSeek-V2 cuts");
    MoeTokenSort = "moe_token_sort", Fused, Ledger, Exemplar::None("its pattern is a block's mixtures, held by the tiny Qwen3-MoE prefill cuts");
    MoeDirectPairs = "moe_direct_pairs", Schedule, Book, Exemplar::None(SCHEDULE);
    MoeGateUpAct = "moe_gate_up_act", Primitive, Ledger, Exemplar::None("its pattern is a whole mixture, held by the tiny Qwen3-MoE cuts");
    MoeGatherInOperand = "moe_gather_in_operand", Fused, Book, Exemplar::None("its pattern is a whole mixture, held by the tiny Qwen3-MoE cuts");
    MoeDownCombine = "moe_down_combine", Primitive, Ledger, Exemplar::None("its pattern is a whole mixture, held by the tiny Qwen3-MoE cuts");
    MoeDeferredFinalize = "moe_deferred_finalize", Fused, Book, Exemplar::None("its pattern is a whole mixture before its residual, held by the tiny Qwen3-MoE cuts");
    SharedExpertFusion = "shared_expert_fusion", Fused, Ledger, Exemplar::None("its pattern is a whole mixture with a shared expert, held by the tiny DeepSeek-V2 cuts");
    GdnStep = "gdn_step", Primitive, Ledger, ex(
        "## Main\nLet Q be a 1 by 64 tensor of Real.\nLet K be a 1 by 64 tensor of Real.\nLet V be a 1 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet B be a 1 by 4 tensor of Real.\nLet A be a 1 by 4 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet O be gated delta of Q over K and V with gate Z beta B decay A state S norm G with 4 heads 2 groups.\n",
        &["GatedDelta"],
    );
    GdnInputEpilogue = "gdn_input_epilogue", Fused, Book, Exemplar::None("its pattern is a whole gated-delta input block, held by the tiny Qwen3.5 cuts");
    GdnDecodeWhole = "gdn_decode_whole", Fused, Book, ex(
        "## Main\nLet XQ be a 1 by 64 tensor of Real.\nLet XK be a 1 by 64 tensor of Real.\nLet XV be a 1 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 1 by 4 tensor of Real.\nLet AR be a 1 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 2 heads.\nLet K be l2norm of SK across 2 heads.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
        &["CausalConv", "CausalConv", "CausalConv", "Silu", "Silu", "Silu", "L2Norm", "L2Norm", "Sigmoid", "Decay", "GatedDelta"],
    );
    GdnPostConvPrep = "gdn_post_conv_prep", Fused, Book, Exemplar::None("its pattern is a whole gated-delta input block, held by the tiny Qwen3.5 cuts");
    ConvBiasSilu = "conv_bias_silu", Fused, Ledger, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet W be a 4 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet Y be causal conv of X with weight W state C over 4 taps.\nLet S be silu of Y.\n",
        &["CausalConv", "Silu"],
    );
    ConvParts = "conv_parts", Fused, Ours, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet Y be a 1 by 8 tensor of Real.\nLet W be a 4 by 8 tensor of Real.\nLet V be a 4 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet D be a 4 by 8 tensor of Real.\nLet P be causal conv of X with weight W state C over 4 taps.\nLet R be causal conv of Y with weight V state D over 4 taps.\n",
        &["CausalConv", "CausalConv"],
    );
    GdnGateProjections = "gdn_gate_projections", Fused, Ours, ex(
        "## Main\nLet Q be a 1 by 64 tensor of Real.\nLet K be a 1 by 64 tensor of Real.\nLet V be a 1 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet X be a 1 by 8 tensor of Real.\nLet WB be a 8 by 4 tensor of Real.\nLet WA be a 8 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet BR be X matmul WB.\nLet BT be sigmoid of BR.\nLet AR be X matmul WA.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
        &["Matmul", "Sigmoid", "Matmul", "Decay", "GatedDelta"],
    );
    GdnGating = "gdn_gating", Fused, Ours, ex(
        "## Main\nLet Q be a 1 by 64 tensor of Real.\nLet K be a 1 by 64 tensor of Real.\nLet V be a 1 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 1 by 4 tensor of Real.\nLet AR be a 1 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
        &["Sigmoid", "Decay", "GatedDelta"],
    );
    FusedSampling = "fused_sampling", Fused, Ledger, ex("## Main\nLet X be a 1 by 8 tensor of Real.\nLet next be argmax of X.\n", &["ArgMax"]);
    CapArgmax = "cap_argmax", Fused, Ours, ex("## Main\nLet X be a 1 by 8 tensor of Real.\nLet Y be X capped at 30.0.\nLet next be argmax of Y.\n", &["Softcap", "ArgMax"]);
    Sampling = "sampling", Primitive, Book, Exemplar::None(NO_SUGAR);
    LastLayerRows = "last_layer_rows", Fused, Book, Exemplar::None("its pattern is a prefill chunk's last layer, held by the tiny prefill cuts");
    StridedRead = "strided_read", Placement, Ours, ex(
        "## Main\nLet X be a 1 by 64 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet P be the last 8 of every 16 of X.\nLet Y be rope of P with cos RC and sin RS.\n",
        &["HeadSlice", "Rope"],
    );
    RowWrite = "row_write", Placement, Book, ex(
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet T be a 4 by 8 tensor of Real.\nLet pos be 2.\nLet K be X matmul M.\nSet row pos of T to K.\n",
        &["Matmul", "SetRow"],
    );
    JoinInPlace = "join_in_place", Placement, Ours, ex(
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 4 tensor of Real.\nLet SA be A scaled by 2.0.\nLet SB be B scaled by 3.0.\nLet J be SA joined with SB across 4 heads.\n",
        &["Scale", "Scale", "Join"],
    );
    StridedValues = "strided_values", Placement, Ours, ex(
        "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 8 tensor of Real.\nLet pos be 3.\nLet V be the first 4 of every 8 of K.\nLet C be attention of Q over K and V with 1 heads 1 groups up to pos.\n",
        &["HeadSlice", "Attention"],
    );
    CudaGraphs = "cuda_graphs", Schedule, Ledger, Exemplar::None(SCHEDULE);
    PersistentLayerMegakernel = "persistent_layer_megakernel", Schedule, Book, Exemplar::None(SCHEDULE);
    AllreduceNorm = "allreduce_norm", Fused, Ledger, ex(
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet N be a 8 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet P be A matmul M.\nLet Q be B matmul N.\nLet S be P plus Q.\nLet T be R plus S.\nLet Y be rmsnorm of T with weight W.\n",
        &["Matmul", "Matmul", "Elementwise", "Elementwise", "FusedRow"],
    );
    AllreduceSumsqNorm = "allreduce_sumsq_norm", Fused, Book, ex(
        "## Main\nLet A be a 1 by 4 tensor of Real.\nLet B be a 1 by 4 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet J be A joined with B.\nLet Y be rmsnorm of J with weight W.\nLet P be the first 4 of every 8 of Y.\nLet Q be the last 4 of every 8 of Y.\n",
        &["Join", "FusedRow", "HeadSlice", "HeadSlice"],
    );
    ConvSiluL2norm = "conv_silu_l2norm", Fused, Ours, ex(
        "## Main\nLet X be a 1 by 64 tensor of Real.\nLet W be a 4 by 64 tensor of Real.\nLet C be a 4 by 64 tensor of Real.\nLet Y be causal conv of X with weight W state C over 4 taps.\nLet S be silu of Y.\nLet N be l2norm of S across 2 heads.\n",
        &["CausalConv", "Silu", "L2Norm"],
    );
    GdnOutputQuant = "gdn_output_quant", Fused, Ours, Exemplar::None(NUMERICS);
    GemvQuant = "gemv_quant", Fused, Ours, Exemplar::None(NUMERICS);
    ElementwiseQuant = "elementwise_quant", Fused, Ours, Exemplar::None(NUMERICS);
    DeltaQkNorm = "delta_qk_norm", Fused, Ours, Exemplar::None("a chip's own fold of a gated delta layer's step and the norms its inputs' launch hands it, held by the tiny Qwen3.5 cuts");
}

impl ClassId {
    /// The class's row.
    pub fn class(self) -> &'static Class {
        &CLASSES[self as usize]
    }

    /// The class whose id is `id`.
    pub fn named(id: &str) -> Option<ClassId> {
        ClassId::ALL.iter().copied().find(|c| c.id() == id)
    }
}

impl std::fmt::Display for ClassId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// The class registered as `id`.
pub fn class_of(id: &str) -> Option<&'static Class> {
    ClassId::named(id).map(ClassId::class)
}
