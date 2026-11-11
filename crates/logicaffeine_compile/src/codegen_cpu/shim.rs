//! What the CPU tier runs whole: a CPU's answer to the shared optimizer, by its instruction set.
//!
//! The CPU program calls its kernels in order on the host's cores, each over every row of its op, its threads splitting
//! a product's outputs; nothing is launched, captured or staged across a bus. Every condition here is a fact about a
//! kernel of the library the CPU link emits:
//!
//!   • a norm, a rotation, an attention over its own cache rows, a soft cap and a product reading checkpoint rows in
//!     their block format in place are each one kernel;
//!   • a part of an operand read at a stride, a row written into a table, two operands joined in place — the link
//!     reads and writes through a view, so no copy is made;
//!   • a fused op is one run of the phases its parts' units make (`link::fuse`), each part computed by the function it
//!     runs alone;
//!   • a product by checkpoint rows reads its activation quantized in one word a 32-value block, its dot products
//!     exact in 32-bit integers, as llama.cpp's CPU dots read theirs;
//!   • an attention's cache rows are `f32`, binary16 or int8 codes under a binary16 scale a head's row, turned by the
//!     Walsh–Hadamard transform before they are quantized where the format says (`CpuCache`): written as the format
//!     keeps them, a quantized row a whole head's row at a time, and read widened — a turned cache's attention taking
//!     its query turned and turning its output back;
//!   • a row attending more positions than a split is attended a split at a time and the splits merged by
//!     log-sum-exp, its splits spread over the threads where its rows are too few to fill them.

use crate::model::quant::GgmlType;
use crate::tensor_graph::fuse::{gemv, leaves, Weights};
use crate::tensor_graph::graph::{Op, QuantFormat, Sym, TensorGraph};
use crate::tensor_graph::numerics::{holds_accumulator, Bound, Contract, Keep, Scheme};
use crate::tensor_graph::shim::{Shim, Support};
use crate::tensor_graph::symbols::{ClassId, Kind};

/// An instruction set the CPU tier emits its kernels for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CpuIsa {
    /// x86-64 with AVX2, FMA and AVX-VNNI (VEX, 256-bit): Alder Lake and later client cores, the i9-14900K.
    X86Avx2Vnni,
    /// x86-64 with AVX-512 F, BW, VNNI and BF16: Sapphire Rapids, Zen 4.
    X86Avx512,
    /// AArch64 with NEON, the dot product and the int8 matrix multiply (`i8mm`).
    ArmNeonDot,
    /// RV64GC with the vector extension (RVV 1.0).
    Rv64Rvv,
}

impl CpuIsa {
    /// Every instruction set the tier answers for.
    pub const ALL: [CpuIsa; 4] = [CpuIsa::X86Avx2Vnni, CpuIsa::X86Avx512, CpuIsa::ArmNeonDot, CpuIsa::Rv64Rvv];

    /// The instruction set's name in a shim's name and a program's identity.
    pub fn name(self) -> &'static str {
        match self {
            CpuIsa::X86Avx2Vnni => "x86-avx2-vnni",
            CpuIsa::X86Avx512 => "x86-avx512",
            CpuIsa::ArmNeonDot => "arm-neon-dot",
            CpuIsa::Rv64Rvv => "rv64-rvv",
        }
    }
}

/// How a product by checkpoint rows reads its activation on the CPU: as its values (the precise numerics: each block's
/// codes widened to `f32` against them), or quantized in one word a 32-value block (its dots exact in 32-bit integers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CpuActivations {
    Values,
    OneWord,
}

/// How the CPU program keeps an attention's cache rows: as `f32` values; as binary16 (llama.cpp's default cache), each
/// written rounded to the nearest and read widened; or as int8 codes under one binary16 scale a head's row
/// (`Keep::Int8Rows`), the row turned first by the Walsh–Hadamard transform where `Int8RowsRotated` (the D5005 engine's
/// cache) — each row written as `Keep::keep_row` keeps it, codes and scale bit for bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CpuCache {
    F32,
    F16,
    Int8Rows,
    Int8RowsRotated,
}

impl CpuCache {
    /// Every format the CPU program keeps a cache in.
    pub const ALL: [CpuCache; 4] = [CpuCache::F32, CpuCache::F16, CpuCache::Int8Rows, CpuCache::Int8RowsRotated];

    /// How the format keeps a cache's values.
    pub fn keep(self) -> Keep {
        match self {
            CpuCache::F32 => Keep::F32,
            CpuCache::F16 => Keep::F16,
            CpuCache::Int8Rows => Keep::Int8Rows,
            CpuCache::Int8RowsRotated => Keep::Int8RowsRotated,
        }
    }
}

/// A CPU of an instruction set under its numerics, answering for the CPU tier's link: its activations, its caches'
/// format, and its fused classes linked whole (`fuse`) or every one of them as its parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuShim {
    pub isa: CpuIsa,
    pub activations: CpuActivations,
    pub cache: CpuCache,
    pub fuse: bool,
}

impl Shim for CpuShim {
    fn name(&self) -> String {
        let numerics = match self.activations {
            CpuActivations::OneWord => "",
            CpuActivations::Values => ":precise",
        };
        let cache = match self.cache {
            CpuCache::F32 => "",
            CpuCache::F16 => ":f16-cache",
            CpuCache::Int8Rows => ":int8-cache",
            CpuCache::Int8RowsRotated => ":int8-rotated-cache",
        };
        format!("cpu:{}{numerics}{cache}{}", self.isa.name(), if self.fuse { "" } else { ":apart" })
    }

    /// Every class by name: the ones the CPU link takes whole (`kernels::LINKED_WHOLE`) — a fused one as its parts where
    /// fusion is off — the ones it runs as its parts, and the ones it refuses, each with its reason.
    fn supports(&self, class: ClassId) -> Support {
        use ClassId::*;
        use Support::{Decompose, Native, Refuse};
        let whole = if self.fuse || class.class().kind != Kind::Fused { Native } else { Decompose };
        match class {
            NormWeight | AddNorm | NormThenAdd | NormScale | NormWeightOffset | NormQuant | AddNormQuant | DualNorm
            | GatedNorm | EmbedNorm | EmbedScale | GemvNormEpilogue | PrefillNormEpilogues | AttnResidualNorm
            | RecomputeFanout | QkNormRope | QkNormRopeKvAppend | RopeKvAppend | RopeQkOneLaunch | QkNormRopeGateSplit
            | QkvHeadEpilogue | HorizontalNormRopeInsert | CrossLayerLaunch | HostResidentKv | HostLayersOnCpu
            | KvCacheQuant | HadamardRotation | Mla | MlaPrologue | MlaUnabsorbInFold | FlashDecode | FlashPrefill
            | AttnGqaRowPacking | AttnLseMerge | AttentionVariantsInline | Softcap | AttnOutputQuant | AttnOutputGate
            | RowWriteQuant | GluActMul | GluActMulQuant | GemvGateUpGlu | ElementwiseChains | ElementwiseRegions
            | ReluSqr | MaskedExpertActivation | GemmBias | GemmScale | MergedQkv | MergedGateUp | GemmDequantInkernel
            | AffineInputFold | RouterTopk | GroupedTopk | RouterSharedGateMerge | MoeTokenSort | MoeDirectPairs
            | MoeGateUpAct | MoeGatherInOperand | MoeDownCombine | MoeDeferredFinalize | SharedExpertFusion | GdnStep
            | GdnInputEpilogue | GdnDecodeWhole | GdnPostConvPrep | ConvBiasSilu | ConvParts | GdnGateProjections
            | GdnGating | FusedSampling | CapArgmax | StridedRead | RowWrite | JoinInPlace | StridedValues
            | ConvSiluL2norm | GdnOutputQuant | GemvQuant | ElementwiseQuant | DeltaQkNorm => whole,
            LastLayerRows | CudaGraphs | PersistentLayerMegakernel | HostWeightStreaming => Decompose,
            CompensatedResidual => Refuse("the CPU program sums its residual in f32, uncompensated"),
            MixedBatchAttention => Refuse("no op of the IR states a step whose rows are one sequence's prompt beside another's next token"),
            SparseAttention => Refuse("no op of the IR states an indexer or the cached blocks an attention selects"),
            Sampling => Refuse("no op of the IR states a sampled token: a step picks its next token by arg-max"),
            AllreduceNorm | AllreduceSumsqNorm => Refuse("the CPU program runs on one host, until tensor parallelism spans CPUs"),
        }
    }

    /// A cache in int8 rows keeps a unit only whole, so no op lands a piece of one: a latent's row assembled from its
    /// parts is joined first and set whole, as the meaning keeps it.
    fn fits(&self, op: &Op, g: &TensorGraph) -> Result<(), String> {
        holds_accumulator(op, g, 32)?;
        if matches!(self.cache, CpuCache::Int8Rows | CpuCache::Int8RowsRotated) {
            let units = g.cache_units();
            for leaf in leaves(op.clone()) {
                let Some(widths) = units.get(&leaf.dst.buf) else { continue };
                if leaf.dst.unit.is_some_and(|u| widths.iter().any(|w| u.width % w != 0 || u.offset % w != 0 || u.pitch % w != 0)) {
                    return Err(format!("a cache in int8 rows (`{}`) takes a whole unit at a time, not a piece", leaf.dst.buf));
                }
            }
        }
        match &op.sym {
            Sym::Prim(_) | Sym::Fused(_) | Sym::View { .. } | Sym::Quantize(QuantFormat::OneWord) => Ok(()),
            Sym::Quantize(QuantFormat::TwoWords) => Err("the CPU program quantizes an activation in one word".to_string()),
        }
    }

    /// An operand a product by checkpoint rows in a block format takes as its activation — the consumer itself, or any
    /// product among a fused consumer's parts — is read in one word a block: the program quantizes it once, before the
    /// consumer or in its producer's epilogue, and every product reading it takes those codes.
    fn activation(&self, consumer: &Op, operand: usize, g: &TensorGraph) -> Option<QuantFormat> {
        let x = consumer.ins.get(operand)?;
        let reads = |p: &Op| p.ins.first() == Some(x) && gemv(p, g).is_some_and(|p| matches!(p.weights, Weights::Rows(ty) if !sixteen(ty)));
        (self.activations == CpuActivations::OneWord && leaves(consumer.clone()).iter().any(reads)).then_some(QuantFormat::OneWord)
    }

    fn numerics(&self) -> Option<Contract> {
        let (activations, words) = match self.activations {
            CpuActivations::OneWord => (Some(Scheme::of(QuantFormat::OneWord)), "one-word"),
            CpuActivations::Values => (None, "precise"),
        };
        Some(Contract {
            point: format!(
                "the CPU program on {}: {words} activations, {} cache, f32 state",
                self.isa.name(),
                match self.cache {
                    CpuCache::F32 => "f32",
                    CpuCache::F16 => "binary16",
                    CpuCache::Int8Rows => "int8-row",
                    CpuCache::Int8RowsRotated => "turned int8-row",
                }
            ),
            activations,
            sixteen: None,
            cache: self.cache.keep(),
            state: Keep::F32,
            attention: None,
            fused: Bound::EXACT,
            accumulator_bits: 32,
        })
    }
}

/// Whether checkpoint rows of `ty` are 16-bit floats, whose products read their activation as values.
fn sixteen(ty: GgmlType) -> bool {
    matches!(ty, GgmlType::F16 | GgmlType::Bf16)
}
