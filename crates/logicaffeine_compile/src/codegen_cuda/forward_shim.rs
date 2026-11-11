//! What the older compiled forward runs whole (`compile_to_cuda_forward*`): its answer to the shared optimizer.
//!
//! A lifted op is the forward's exactly when its kernels compute it ([`kernel_refusal`] says why they do not
//! otherwise). Its fused kernels are the ones its emitter writes:
//!
//!   • a sum and the RMS norm of it that is the next stage, as one row kernel writing the sum on (`add_norm`), the norm
//!     without a residual of its own over the whole of the sum;
//!   • a weighted RMS norm as one row kernel;
//!   • a run of elementwise ops, each carrying the last one's result, as one register-carried kernel — the runs taken
//!     before a norm takes a sum, so a sum a run carries stays in its run;
//!   • under the single-pass int8 numerics, the activation a Q8_0 projection reads quantized in its producer's epilogue
//!     — a norm's, a sum-then-norm's, an attention's;
//!   • a sparse mixture as a top-k router, a gate-up kernel and a down-and-combine kernel, its Q8_0 matrices (the
//!     forward's mixture manifest) dequantized inside the kernels that read them;
//!   • the whole forward captured as one CUDA graph.
//!
//! It reads no part of a tensor in place and writes none into rows of another: every stage reads and writes whole
//! buffers.

use crate::tensor_graph::front::TensorOpSpec;
use crate::model::quant::GgmlType;
use crate::tensor_graph::fuse::{binary, leaves, moe, norm, weights, ChipRule, Weights, RULES};
use crate::tensor_graph::graph::{Access, Op, QuantFormat, Sym, TensorGraph};
use crate::tensor_graph::numerics::{holds_accumulator, Bound, Contract, Keep, Scheme};
use crate::tensor_graph::shim::{Position, Shim, Support};
use crate::tensor_graph::symbols::ClassId;

use super::qwen_emit::Q8MatKind;
use crate::tensor_recognize::qwen::{GateAct, MoeAct};
use crate::tensor_recognize::RowReduceKind;

/// The static shared memory one attention block may declare (`sc[L]` scores plus the 32-float reduction row) — the
/// 48 KiB every CUDA architecture grants without an opt-in.
const ATTN_STATIC_SHARED_BYTES: usize = 48 * 1024;

/// Why the forward's kernels do not compute a lifted op, or `None` when they do. The spec is the op's meaning: a field
/// of it the kernel does not implement is a refusal, never a field the kernel ignores.
pub fn kernel_refusal(op: &TensorOpSpec) -> Option<&'static str> {
    use TensorOpSpec as T;
    match op {
        T::Matmul(_) | T::Silu(_) | T::Transpose(_) => None,
        T::Elementwise(e) if e.row.is_some() => {
            Some("an elementwise op over a row combined with every row of a block — its kernel reads both at one index")
        }
        T::Elementwise(_) => None,
        T::FusedRow(r) if r.weight_rows > 1 => {
            Some("a norm in lanes (`in L lanes`), each row by its own row of the weight — its kernels read one weight row")
        }
        T::FusedRow(r) => match r.reduce {
            RowReduceKind::RmsNorm if r.bias.is_some() => Some("a bias on an RMSNorm — its kernel scales and never shifts"),
            RowReduceKind::Softmax if r.weight.is_some() || r.residual.is_some() || r.bias.is_some() => {
                Some("a weight, residual or bias on a softmax")
            }
            RowReduceKind::LayerNorm if r.residual.is_some() => Some("a residual added before a layernorm"),
            _ => None,
        },
        T::Rope(r) if r.from.is_some() => Some("a rope reading its angles from a position in a cache of angles"),
        T::Rope(r) if r.rows != 1 => Some("a rope with a table row per row of the query — its kernel reads one row of angles"),
        T::Rope(r) if r.hd != 2 * r.half => Some("a partial rotary — its kernel turns every dimension of the head"),
        T::Rope(_) => None,
        T::Attention(a) if a.softcap.is_some() => Some("a soft-capped attention (`capped at C`) — its kernel caps no score"),
        T::GatedDelta(g) if g.gate == GateAct::Sigmoid => Some("a gated delta gated by a sigmoid (`with sigmoid gate Z`) — its kernel gates by SiLU"),
        T::Attention(a) if a.pos.is_some() => Some("a masked attention (`up to P`) — its kernel attends every cached position"),
        T::Softcap(_) => Some("a soft-capped tensor (`capped at C`) — no kernel caps one"),
        T::Attention(a) if a.sinks.is_some() => Some("an attention with sinks (`with sinks S`) — its kernel's softmax has no sink"),
        T::Attention(a) if a.window.is_some() => {
            Some("a windowed attention (`within a window of W`) — its kernel attends every cached position")
        }
        T::Attention(a) if a.vd != a.hd => {
            Some("an attention whose values are narrower than its keys — its kernel reads values as wide as keys")
        }
        T::Attention(a) if a.scale.is_some() => {
            Some("an attention with a given scale (`scaled by S`) — its kernel scales the scores by 1/√HD")
        }
        T::Attention(a) if a.rows != 1 => Some("a causal block of query rows — its kernel attends for one query"),
        T::Attention(a) if a.kv == 0 || a.h % a.kv != 0 => Some("query heads that do not group evenly over the key/value heads"),
        T::Attention(a) if a.l * 4 + 32 * 4 > ATTN_STATIC_SHARED_BYTES => {
            Some("a cache longer than the score row its kernel holds in static shared memory")
        }
        T::Attention(_) => None,
        T::Moe(m) if m.router_bias.is_some() || m.biases.iter().any(Option::is_some) => Some("an expert mixture with biases — its kernels add none"),
        T::Moe(m) if !m.renormalize => {
            Some("an expert mixture without renormalizing — its router renormalizes the kept experts' weights")
        }
        T::Moe(m) if m.act != MoeAct::Swiglu => Some("an expert mixture with a clamped SwiGLU — its kernels gate with SwiGLU"),
        T::Moe(m) if !m.weighs_outputs_by_softmax() => {
            Some("an expert mixture whose router is not a softmax weighting its experts' outputs — scored by sigmoids, biased in its choice, limited to groups, scaled, or weighting its inputs")
        }
        T::Moe(_) => None,
        T::LaneSums(_) | T::SignedRoot(_) => {
            Some("a lane sum or a signed root (`the lane sums of X over L lanes`, `the signed root of X`) — no kernel computes one")
        }
        T::NgramIds(_) => Some("an n-gram hash (`the ngram ids of T after R by M modulo V from O ending at E`) — no kernel hashes one"),
        T::LaneMean(_) | T::LaneSpread(_) => {
            Some("a hyper-connected residual's lanes (`the mean of X over L lanes`, `O spread over L lanes`) — no kernel mixes or spreads them")
        }
        T::SwiGlu(_)
        | T::Sigmoid(_)
        | T::Gelu(_)
        | T::Scale(_)
        | T::Decay(_)
        | T::L2Norm(_)
        | T::HeadSlice(_)
        | T::Join(_)
        | T::HeadMatmul(_)
        | T::CausalConv(_)
        | T::GatedDelta(_)
        | T::FullAttention(_)
        | T::RowGather(_)
        | T::RowsGather(_)
        | T::ArgMax(_)
        | T::SetRow(_)
        | T::Scatter(_)
        | T::Map(_)
        | T::IntRequant(_)
        | T::IntClamp(_)
        | T::Relu(_)
        | T::LutAttention(_) => Some("this lane has no kernel for the op"),
    }
}

/// The older forward under its numerics.
#[derive(Clone, Copy, Debug)]
pub struct ForwardShim {
    pub kind: Q8MatKind,
}

impl ForwardShim {
    /// Whether a product reads Q8_0 rows: the forward's int8 projection.
    fn q8_product(op: &Op, g: &TensorGraph) -> bool {
        matches!(op.prim(), Some(TensorOpSpec::Matmul(_))) && matches!(weights(g, &op.ins[1]), Some(Weights::Rows(GgmlType::Q8_0)))
    }
}

impl Shim for ForwardShim {
    fn name(&self) -> String {
        format!("cuda-forward:{:?}", self.kind)
    }

    fn supports(&self, class: ClassId) -> Support {
        use ClassId::*;
        use Support::{Decompose, Native, Refuse};
        let producer_quant = if self.kind == Q8MatKind::Int8Act { Native } else { Decompose };
        match class {
            NormWeight | AddNorm | ElementwiseChains | ElementwiseRegions | GemmDequantInkernel | CudaGraphs => Native,
            RouterTopk | MoeGateUpAct | MoeDownCombine => Native,
            NormQuant | AddNormQuant | AttnOutputQuant => producer_quant,
            AttnOutputGate | RowWriteQuant => Decompose,
            NormThenAdd | NormScale | NormWeightOffset | DualNorm | GatedNorm | EmbedNorm | EmbedScale => Decompose,
            GemvNormEpilogue | PrefillNormEpilogues | AttnResidualNorm | RecomputeFanout => Decompose,
            QkNormRope | QkNormRopeKvAppend | RopeKvAppend | RopeQkOneLaunch | QkNormRopeGateSplit | QkvHeadEpilogue => Decompose,
            HorizontalNormRopeInsert | CrossLayerLaunch | Mla | MlaPrologue | MlaUnabsorbInFold => Decompose,
            GluActMul | GluActMulQuant | GemvGateUpGlu | MaskedExpertActivation => Decompose,
            GemmBias | GemmScale | MergedQkv | MergedGateUp | AffineInputFold | RouterSharedGateMerge => Decompose,
            MoeTokenSort | MoeGatherInOperand | MoeDeferredFinalize | SharedExpertFusion => Decompose,
            GdnInputEpilogue | GdnDecodeWhole | GdnPostConvPrep | ConvBiasSilu | ConvParts | ConvSiluL2norm => Decompose,
            GdnGateProjections | GdnGating | FusedSampling | LastLayerRows | AllreduceNorm => Decompose,
            ElementwiseQuant | GdnOutputQuant | GemvQuant => Decompose,
            ReluSqr => Refuse("the forward links no ReLU"),
            FlashDecode | FlashPrefill => Refuse("the forward attends one query over a cache held in one block's shared scores"),
            AttentionVariantsInline | Softcap | MixedBatchAttention | SparseAttention => {
                Refuse("the forward's attention takes no mask, window, sink, soft cap or given scale")
            }
            AttnGqaRowPacking | AttnLseMerge => Refuse("the forward attends a cache in one block, never in chunks"),
            GroupedTopk => Refuse("the forward routes by one top-k over every expert"),
            Sampling => Refuse("the forward computes logits and picks no token"),
            GdnStep => Refuse("the forward links no delta rule"),
            KvCacheQuant | HadamardRotation => Refuse("the forward holds its caches in f32"),
            CompensatedResidual => Refuse("the forward sums its residual in f32, uncompensated"),
            HostResidentKv => Refuse("the forward keeps every buffer on the device"),
            MoeDirectPairs => Refuse("the forward runs one sequence's row through its kept experts"),
            PersistentLayerMegakernel => Refuse("the forward replays a graph of a kernel a stage"),
            StridedRead | RowWrite | JoinInPlace | StridedValues => Refuse("the forward's stages read and write whole buffers"),
        }
    }

    fn fits(&self, op: &Op, g: &TensorGraph) -> Result<(), String> {
        holds_accumulator(op, g, 32)?;
        if op.access.iter().any(|a| *a != Access::Whole) || op.dst.own().is_none() {
            return Err("the forward's stages read and write whole buffers".to_string());
        }
        match &op.sym {
            Sym::Prim(spec @ TensorOpSpec::Moe(_)) if op.ins[1..5].iter().any(|w| !matches!(weights(g, w), Some(Weights::Rows(GgmlType::Q8_0)))) => {
                kernel_refusal(spec).map_or_else(|| Err(format!("moe stage `{}` has no entry in the MoE weight manifest", op.out)), |why| Err(why.to_string()))
            }
            Sym::Prim(spec) => kernel_refusal(spec).map_or(Ok(()), |why| Err(why.to_string())),
            Sym::View { .. } => Err("the forward reads no view into another buffer".to_string()),
            Sym::Quantize(_) if self.kind == Q8MatKind::Dequant => Err("the forward's precise numerics read no activation quantized".to_string()),
            Sym::Quantize(_) => Ok(()),
            Sym::Fused(f) => {
                for leaf in leaves(op.clone()) {
                    self.fits(&leaf, g)?;
                }
                match f.class {
                    ClassId::AddNorm => {
                        let next = match (f.parts.first(), f.parts.last()) {
                            (Some(add), Some(n)) => add.covers.last().zip(n.covers.first()).is_some_and(|(a, b)| a + 1 == *b),
                            _ => false,
                        };
                        match norm(op) {
                            Some(n) if n.part.is_none() && !n.pre && next => Ok(()),
                            Some(n) if n.part.is_none() && !n.pre => Err("the forward adds into a norm only where the norm is the next stage".to_string()),
                            _ => Err("the forward adds into the norm that follows, over the whole of the sum".to_string()),
                        }
                    }
                    ClassId::ElementwiseChains => {
                        let carried = f.parts.windows(2).all(|w| {
                            binary(&w[1]).is_some_and(|b| (b.a == w[0].out) ^ (b.b == w[0].out))
                                || matches!(w[1].prim(), Some(TensorOpSpec::Silu(_)))
                        });
                        if carried {
                            Ok(())
                        } else {
                            Err("the forward carries a chain's result through each step, as exactly one operand".to_string())
                        }
                    }
                    ClassId::AttnOutputQuant => match f.parts.first().and_then(|host| host.prim()) {
                        Some(TensorOpSpec::Attention(a)) if a.hd % 32 == 0 => Ok(()),
                        _ => Err("the forward's attention quantizes its result only where each head's values are whole 32-value blocks".to_string()),
                    },
                    _ => Ok(()),
                }
            }
        }
    }

    /// The forward takes its regions of elementwise ops, then its runs of them, before a norm takes a sum: the shared
    /// rules, run first — a region of several joined ops is one kernel where a run would take only one of its arms.
    fn rules(&self) -> Vec<(Position, ChipRule<'_>)> {
        let rule = |name: &str| RULES.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("the shared table runs {name}")).apply;
        vec![
            (Position::Before("add_norm"), ChipRule { name: "elementwise_regions", builds: &[ClassId::ElementwiseRegions], apply: Box::new(rule("elementwise_regions")) }),
            (Position::Before("add_norm"), ChipRule { name: "elementwise_chains", builds: &[ClassId::ElementwiseChains], apply: Box::new(rule("elementwise_chains")) }),
        ]
    }

    fn activation(&self, consumer: &Op, operand: usize, g: &TensorGraph) -> Option<QuantFormat> {
        let format = match self.kind {
            Q8MatKind::Dequant => return None,
            Q8MatKind::Int8Act => QuantFormat::OneWord,
            Q8MatKind::TwoPassInt8 => QuantFormat::TwoWords,
        };
        (operand == 0 && (Self::q8_product(consumer, g) || moe(consumer).is_some())).then_some(format)
    }

    fn numerics(&self) -> Option<Contract> {
        let activations = match self.kind {
            Q8MatKind::Dequant => None,
            Q8MatKind::Int8Act => Some(Scheme::of(QuantFormat::OneWord)),
            Q8MatKind::TwoPassInt8 => Some(Scheme::of(QuantFormat::TwoWords)),
        };
        let point = format!("the older forward: {:?} activations", self.kind);
        Some(Contract { point, activations, cache: Keep::F32, state: Keep::F32, attention: None, fused: Bound::EXACT, accumulator_bits: 32 })
    }
}
