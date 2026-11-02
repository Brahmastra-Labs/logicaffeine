//! What the decode engine runs as one unit: its answer to the shared optimizer.
//!
//! The engine reads a step block by block ([`super::lower_graph`]), and the merges it makes are the shared table's
//! classes:
//!   • the projections of one activation whose matrices are one checkpoint tensor — a Gated-DeltaNet block's query, key
//!     and value as row ranges following one another from the tensor's first row, an attention block's queries and
//!     output gate as the halves of each head — as one matvec (`merged_qkv`);
//!   • a convolution and the SiLU after it, the conv unit's one pass over a Gated-DeltaNet block's channels
//!     (`conv_bias_silu`);
//!   • a Gated-DeltaNet block's write strength and forget gate, the sigmoid and the decay its delta rule reads, as the
//!     gates unit (`gdn_gating`);
//!   • an attention's output gate, its result times the sigmoid of the gate, as the sigmoid-gate unit (`glu_act_mul`).
//! A norm, a rotation, a cache append and an add each run as a command of their own. An op the engine has no unit for is
//! refused as having none, for what [`super::capability`] names when it refuses a whole checkpoint for it.

use crate::model::bind::{self, Source, View};
use crate::model::spec::ModelSpec;
use crate::tensor_graph::front::TensorOpSpec;
use crate::tensor_graph::fuse::{activation, leaves};
use crate::tensor_graph::graph::{Access, Op, QuantFormat, Sym, TensorGraph};
use crate::spatial::serving::{AttentionMath, VALUE_BLOCK};
use crate::tensor_graph::numerics::{AttentionNumerics, Block, Bound, Contract, Keep, Scale, Scheme, Taken};
use crate::tensor_graph::shim::{Shim, Support};
use crate::tensor_graph::symbols::ClassId;
use crate::tensor_recognize::qwen::Activation;

use super::capability::Refusal;
use super::model::{CacheFormat, EngineNumerics, StateFormat};
use super::numerics::{ActScaling, Words, ACT_BLOCK, WIDE_BITS};

/// How the shim begins every refusal of an op the engine has no unit for.
pub const NO_UNIT: &str = "the engine has no unit for ";

/// The engine's shim for one model at one knob point: its spec says which checkpoint tensor and view each constant
/// reads, its numerics the contract its build keeps — which change the contract, never what the engine runs whole.
pub struct EngineShim<'a> {
    pub spec: &'a ModelSpec,
    pub numerics: EngineNumerics,
}

impl EngineShim<'_> {
    /// The checkpoint tensor and view `constant` reads, where the checkpoint binds it to a weight.
    fn view(&self, constant: &str) -> Option<(String, View)> {
        match bind::bind(self.spec, constant) {
            Ok(Source::Weight(b)) => Some((b.tensor, b.view)),
            _ => None,
        }
    }

    /// Whether `parts`, products of one activation, multiply by one checkpoint matrix the engine streams as one matvec —
    /// row ranges of one tensor following one another from its first row, or the two halves of each head of one tensor —
    /// or by the start of one: a merge grows a product at a time.
    fn one_matrix(&self, parts: &[Op]) -> bool {
        let Some(views) = parts.iter().map(|p| p.ins.get(1).and_then(|w| self.view(w))).collect::<Option<Vec<_>>>() else {
            return false;
        };
        if views.iter().any(|(t, _)| *t != views[0].0) {
            return false;
        }
        let ranges = views.iter().try_fold(0usize, |next, (_, v)| match v {
            View::RowRange { first, count, transpose: true } if *first == next => Some(first + count),
            _ => None,
        });
        let halves = match views.as_slice() {
            [(_, View::HeadPart { stride: s1, offset: 0, width: w1, transpose: true }), rest @ ..] if *s1 == 2 * w1 => match rest {
                [] => true,
                [(_, View::HeadPart { stride: s2, offset: o2, width: w2, transpose: true })] => s2 == s1 && o2 == w1 && w2 == w1,
                _ => false,
            },
            _ => false,
        };
        ranges.is_some() || halves
    }
}

impl Shim for EngineShim<'_> {
    fn name(&self) -> String {
        format!("d5005-engine:{}", self.spec.arch)
    }

    fn supports(&self, class: ClassId) -> Support {
        use ClassId::*;
        use Support::{Decompose, Native, Refuse};
        match class {
            MergedQkv | ConvBiasSilu | GdnGating | GluActMul => Native,
            NormWeight | AddNorm | NormThenAdd | NormScale | NormWeightOffset | DualNorm | GatedNorm | EmbedNorm => Decompose,
            GemvNormEpilogue | PrefillNormEpilogues | AttnResidualNorm | RecomputeFanout | ElementwiseChains | ElementwiseRegions => Decompose,
            QkNormRope | QkNormRopeKvAppend | RopeKvAppend | RopeQkOneLaunch | QkNormRopeGateSplit | QkvHeadEpilogue => Decompose,
            HorizontalNormRopeInsert | CrossLayerLaunch | ConvParts | ConvSiluL2norm | GdnGateProjections | GdnInputEpilogue => Decompose,
            GdnPostConvPrep | GdnDecodeWhole | GemvGateUpGlu | MergedGateUp | GemmDequantInkernel | FusedSampling | LastLayerRows => Decompose,
            NormQuant | AddNormQuant | GluActMulQuant | ElementwiseQuant | GdnOutputQuant | GemvQuant | AttnOutputQuant | GdnStep => Decompose,
            AttnOutputGate | RowWriteQuant => Decompose,
            GemmBias | GemmScale | AffineInputFold => Refuse("the engine's matvec unit adds no bias and scales by no factor"),
            EmbedScale => Refuse("the engine reads its embedding unscaled"),
            Mla | MlaPrologue | MlaUnabsorbInFold => Refuse("the engine has no unit for multi-head latent attention"),
            RouterTopk | MoeTokenSort | MoeGateUpAct | MoeDownCombine | SharedExpertFusion | MoeGatherInOperand => {
                Refuse("the engine has no unit for experts")
            }
            MoeDeferredFinalize | MoeDirectPairs | GroupedTopk | RouterSharedGateMerge | MaskedExpertActivation => {
                Refuse("the engine has no unit for experts")
            }
            FlashDecode | FlashPrefill | AttnLseMerge | AttnGqaRowPacking => Refuse("the engine attends one query over its cache in its attention unit"),
            AttentionVariantsInline | Softcap | MixedBatchAttention | SparseAttention => {
                Refuse("the engine's attention takes no mask, window, sink or soft cap")
            }
            StridedRead | RowWrite | JoinInPlace | StridedValues => Refuse("the engine's units read and write whole slots"),
            ReluSqr => Refuse("the engine has no ReLU unit"),
            Sampling => Refuse("the engine picks the next token by arg-max"),
            KvCacheQuant | HadamardRotation => Refuse("the engine's cache format is its numerics', not a fusion"),
            CompensatedResidual => Refuse("the engine sums its residual uncompensated"),
            HostResidentKv => Refuse("the engine keeps its caches in its card's memory"),
            PersistentLayerMegakernel | CudaGraphs => Refuse("the engine runs a program of commands, not kernel launches"),
            AllreduceNorm => Refuse("the engine runs on one card"),
        }
    }

    fn fits(&self, op: &Op, _: &TensorGraph) -> Result<(), String> {
        for leaf in leaves(op.clone()) {
            if leaf.access.iter().any(|a| *a != Access::Whole) || leaf.dst.row.is_some() || leaf.dst.unit.is_some() {
                return Err("the engine's units read and write whole slots".into());
            }
            let refused = match leaf.prim() {
                Some(TensorOpSpec::Moe(_)) => Some(Refusal::Ffn { layer: 0, what: "experts" }.gap()),
                Some(TensorOpSpec::Gelu(_)) => Some(Refusal::Ffn { layer: 0, what: "GELU" }.gap()),
                Some(TensorOpSpec::Attention(a)) if a.window.is_some() => Some(Refusal::Attention { layer: 0, what: "a sliding window" }.gap()),
                Some(TensorOpSpec::Attention(a)) if a.sinks.is_some() => Some(Refusal::Attention { layer: 0, what: "attention sinks" }.gap()),
                Some(TensorOpSpec::Attention(a)) if a.softcap.is_some() => {
                    Some(Refusal::Attention { layer: 0, what: "soft-capped scores" }.gap())
                }
                Some(TensorOpSpec::Softcap(_)) => Some(Refusal::LogitSoftcap.gap()),
                Some(TensorOpSpec::HeadMatmul(_)) => Some(Refusal::Mixer { layer: 0, what: "multi-head latent attention" }.gap()),
                Some(TensorOpSpec::Scale(_)) => Some(Refusal::EmbeddingScale.gap()),
                Some(spec @ (TensorOpSpec::Relu(_)
                | TensorOpSpec::IntRequant(_)
                | TensorOpSpec::IntClamp(_)
                | TensorOpSpec::LutAttention(_)
                | TensorOpSpec::FullAttention(_)
                | TensorOpSpec::Transpose(_)
                | TensorOpSpec::HeadSlice(_)
                | TensorOpSpec::Join(_)
                | TensorOpSpec::RowsGather(_))) => Some(format!("a {}", spec.kind())),
                _ => None,
            };
            if let Some(what) = refused {
                return Err(format!("{NO_UNIT}{what}"));
            }
        }
        let Sym::Fused(f) = &op.sym else { return Ok(()) };
        match f.class {
            ClassId::MergedQkv if !self.one_matrix(&f.parts) => {
                Err("the engine streams one checkpoint matrix a matvec: row ranges of one tensor from its first row, or each head's two halves".into())
            }
            ClassId::GluActMul if f.parts.iter().find_map(activation) != Some(Activation::Sigmoid) => {
                Err("the engine gates an attention's result by a sigmoid only".into())
            }
            _ => Ok(()),
        }
    }

    fn activation(&self, _: &Op, _: usize, _: &TensorGraph) -> Option<QuantFormat> {
        None
    }

    fn numerics(&self) -> Option<Contract> {
        let n = self.numerics;
        let (block, significand_bits) = match n.scaling {
            ActScaling::Sub32 => (Block::Values(ACT_BLOCK), 4),
            ActScaling::PerBlock => (Block::OfTheWeights, WIDE_BITS),
        };
        let words = match n.activations {
            Words::One => 1,
            Words::Two => 2,
        };
        let state = match n.state {
            StateFormat::F32 => Keep::F32,
            StateFormat::F16 => Keep::F16,
        };
        let cache = match n.cache {
            CacheFormat::F16 => Keep::F16,
            CacheFormat::Int8 => Keep::Int8Rows,
            CacheFormat::Int8Hadamard => Keep::Int8RowsRotated,
        };
        let activations = Scheme { block, scale: Scale::LeastPowerOfTwo { significand_bits }, words };
        // On the array an attention takes its head's query as an activation, a whole head a block where the products'
        // blocks are the weights' (`quantize_head`), the cached rows as the cache keeps them, and its weights, each
        // times its value row's scale, as unsigned codes under a power of two a block of positions (`weighted_int8`).
        let attention = match n.attention {
            AttentionMath::Vector => None,
            AttentionMath::Array => Some(AttentionNumerics {
                queries: Taken::Quantized(Scheme { block: if block == Block::OfTheWeights { Block::Row } else { block }, ..activations }),
                keys_values: Taken::Kept(cache),
                weights: Taken::Quantized(Scheme { block: Block::Values(VALUE_BLOCK as usize), scale: Scale::LeastPowerOfTwoOver255, words: 1 }),
            }),
        };
        // Integer dots are exact in i64 (36 bits used); a block's or a group's sum is exact in a 64-place window over
        // i128 and rounded to f32 once.
        Some(Contract { point: format!("the engine: {n:?}"), activations: Some(activations), cache, state, attention, fused: Bound::EXACT, accumulator_bits: 64 })
    }
}
