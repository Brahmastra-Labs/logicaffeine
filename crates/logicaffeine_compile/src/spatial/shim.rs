//! What the spatial chips run whole (CUDA persistent, Tensix, the RDU, the fabric): their answer to the shared optimizer.
//!
//! A spatial chip lowers each op of the one IR to sections of the closed compute algebra — matrix tiles, row maps,
//! reductions, recurrent and attention sections — places them on its sites and schedules the whole step as one program.
//! It links one fused class whole: a region of pointwise ops, its members' sections joined into one, each value it makes
//! handed from the member that makes it to the one that reads it and never stored. Every other fused op runs as its
//! parts, each its own sections, and the spatial passes join sections after lowering. A class turns native here when
//! the spatial link builds it as one section. Placements are refused: a section reads and writes each tensor through
//! its own access map, never a part of another op's.

use super::target::SpatialTarget;
use crate::tensor_graph::graph::{Op, Sym, TensorGraph};
use crate::tensor_graph::numerics::{holds_accumulator, Bound, Contract, Keep};
use crate::tensor_graph::shim::{Shim, Support};
use crate::tensor_graph::symbols::ClassId;

/// A spatial chip, by the target its sections are lowered for.
pub struct SpatialShim<'a> {
    pub target: &'a SpatialTarget,
}

impl Shim for SpatialShim<'_> {
    fn name(&self) -> String {
        format!("spatial:{}", self.target.name)
    }

    fn supports(&self, class: ClassId) -> Support {
        use ClassId::*;
        use Support::{Decompose, Native, Refuse};
        match class {
            NormWeight | FlashDecode | FlashPrefill | GemmDequantInkernel | RouterTopk | MoeGateUpAct | MoeDownCombine | GdnStep => Native,
            PersistentLayerMegakernel => Native,
            AddNorm | NormThenAdd | NormScale | NormWeightOffset | NormQuant | AddNormQuant | DualNorm | GatedNorm | EmbedNorm => Decompose,
            EmbedScale | GemvNormEpilogue | PrefillNormEpilogues | AttnResidualNorm | RecomputeFanout | QkNormRope => Decompose,
            QkNormRopeKvAppend | RopeKvAppend | RopeQkOneLaunch | QkNormRopeGateSplit | QkvHeadEpilogue | HorizontalNormRopeInsert => Decompose,
            CrossLayerLaunch | Mla | MlaPrologue | AttnOutputQuant | AttnOutputGate | RowWriteQuant | MlaUnabsorbInFold | GluActMul | GluActMulQuant => Decompose,
            ElementwiseRegions => Native,
            GemvGateUpGlu | ElementwiseChains | ReluSqr | MaskedExpertActivation | GemmBias | GemmScale => Decompose,
            MergedQkv | MergedGateUp | AffineInputFold | RouterSharedGateMerge | MoeTokenSort | MoeGatherInOperand => Decompose,
            MoeDeferredFinalize | SharedExpertFusion | GdnInputEpilogue | GdnDecodeWhole | GdnPostConvPrep | ConvBiasSilu => Decompose,
            ConvParts | GdnGateProjections | GdnGating | FusedSampling | LastLayerRows | AllreduceNorm => Decompose,
            ConvSiluL2norm | GdnOutputQuant | GemvQuant | ElementwiseQuant => Decompose,
            StridedRead | RowWrite | JoinInPlace | StridedValues => Refuse("a section reads and writes each tensor through its own access map"),
            Softcap => Native,
            AttentionVariantsInline => Refuse("the attention sections attend a prefix of the cache, with no window or sink"),
            MixedBatchAttention | SparseAttention => Refuse("the attention sections attend every cached row of one sequence's prefix"),
            AttnGqaRowPacking | AttnLseMerge => Refuse("an attention section holds its scores whole, never in chunks"),
            GroupedTopk => Refuse("the expert sections route by one top-k over every expert"),
            Sampling => Refuse("a spatial step picks its token by arg-max"),
            KvCacheQuant | HadamardRotation | CompensatedResidual => Refuse("a spatial chip holds its values in the precision the target's datapath reads"),
            HostResidentKv | MoeDirectPairs => Refuse("a spatial program places every tensor on the chip's own memory"),
            CudaGraphs => Refuse("a spatial program is placed and scheduled whole, never captured as launches"),
        }
    }

    fn fits(&self, op: &Op, g: &TensorGraph) -> Result<(), String> {
        holds_accumulator(op, g, self.target.matmul_acc_int_bits())?;
        match &op.sym {
            Sym::Prim(_) => Ok(()),
            Sym::View { .. } => Err("a section reads a row of a table through its own access map, never a view of it".to_string()),
            Sym::Quantize(_) => Err("a spatial chip reads no activation quantized".to_string()),
            Sym::Fused(f) if f.class == ClassId::ElementwiseRegions => {
                // The members join one into the next through each value they make, which the section then never stores:
                // a value written out for a later reader, or read twice within, keeps its store.
                let read_once = f.parts.iter().filter(|m| m.out != op.out).all(|m| f.parts.iter().flat_map(|p| &p.ins).filter(|t| **t == m.out).count() == 1);
                if op.keeps.is_empty() && read_once {
                    Ok(())
                } else {
                    Err("a region is one section where each value it makes is read once within it and none is written out".to_string())
                }
            }
            Sym::Fused(_) => Err("a spatial chip runs every fused op as its parts".to_string()),
        }
    }

    fn numerics(&self) -> Option<Contract> {
        use crate::codegen_sva::fabric_target::Precision;
        // A section reads its floats in the narrowest float the matrix datapath takes.
        let keep = if self.target.matmul_supports(Precision::Bf16) {
            Keep::Bf16
        } else if self.target.matmul_supports(Precision::Fp16) {
            Keep::F16
        } else {
            Keep::F32
        };
        let point = format!("spatial:{}", self.target.name);
        // A joined section keeps a folded intermediate in its registers where the program apart packs it and reads it
        // back: an exact program's joins are exact, an approximate one's within the tolerance its rewrites are held to.
        let fused = Bound { of_block: 0.0, of_value: crate::spatial::equiv::APPROX_REL_TOL, floor: crate::spatial::equiv::APPROX_REL_TOL };
        Some(Contract { point, activations: None, cache: keep, state: keep, attention: None, fused, accumulator_bits: self.target.matmul_acc_int_bits() })
    }
}
