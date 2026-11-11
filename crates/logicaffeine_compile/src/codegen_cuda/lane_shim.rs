//! What the compiled generation lane runs whole: the CUDA lane's answer to the shared optimizer.
//!
//! The shared rules decide a fusion by dataflow; the lane's kernels decide whether a fused op has a launch. Every
//! condition here is a fact about a kernel of the library the lane links:
//!
//!   • a per-head product reads a part of every unit of its input in place only at whole 32-value blocks, the quantized
//!     form's;
//!   • an attention reads its values in place only from an even place at an even pitch, a pair at a time;
//!   • a convolution's epilogue takes a per-head L2 norm over heads of at most 1024 values, whole warps of them, that
//!     divide its channels;
//!   • a gated pair of products is one launch as GEMVs (a decode step's rows, a batch's, or the precise numerics), or on
//!     the tensor cores as one GEMM of the gate's and up's tiles side by side where their formats agree and the hidden
//!     width is whole pair tiles;
//!   • a mixture folds its shared expert into a block of at most 32 warps of kept experts, and never writes its result at
//!     a row a parameter names;
//!   • a product by checkpoint rows reads its activation quantized under the lane's quantized numerics — one int8 word a
//!     value, or two — and a producer quantizes in its epilogue under the `fuse_quant` knob where its kernel can: a norm,
//!     an elementwise op, a SwiGLU, a delta rule, a gated pair whose hidden width is whole 32-value blocks, a product of
//!     four rows a warp whose outputs are whole blocks;
//!   • several products of one activation are one tensor-core GEMM in parts — at most four, one format, each a whole
//!     number of its GEMM's tiles (128 outputs on the int8 tensor cores, `gemm_h16_tile_n` on the 16-bit ones) — above
//!     the GEMV's rows.
//!
//! The lane's own rules link into the table: a wide chunk's latent attention taken whole before the shared rules (the
//! lane runs it expanded), and the decode step's products of one activation as one GEMV in parts before the products
//! on the tensor cores are merged.

use std::collections::BTreeSet;

use crate::codegen_cuda::knobs::{CachePrecision, CudaKnobs, StatePrecision};
use crate::codegen_cuda::qwen_emit::Q8MatKind;
use crate::tensor_graph::facts;
use crate::tensor_graph::fuse::{
    allreduce_partials, class, conv, epilogued_product, gemv, leaves, moe, norm, parts_norm, rope, scaled_norm, slice, unit_output, ChipRule, ConvForm,
    GemvForm, Rewriter, Weights,
};
use crate::tensor_graph::front::TensorOpSpec;
use crate::tensor_graph::graph::{Access, Fused, Op, QuantFormat, RowSel, Storage, Sym, TensorGraph};
use crate::tensor_graph::numerics::{holds_accumulator, AttentionNumerics, Block, Bound, Contract, Keep, Scale, Scheme, SixteenBit, Taken};
use crate::tensor_graph::shim::{Position, Shim, Support};
use crate::tensor_graph::symbols::ClassId;
use crate::tensor_recognize::qwen::{GateAct, MoeAct, RowIndex};
use crate::tensor_recognize::ElementwiseOp;

use super::lane::kernels;
use super::lane::plan::{attended_span, Dst as LaneDst, Launch, Part as LanePart, Split, GEMV_ROWS, QBLOCK};

/// The convolutions one launch takes as parts.
const CONV_PARTS: usize = 4;

/// The rotations one launch takes: a layer's query and its key.
const ROPE_SIDES: usize = 2;

/// The lane under its knobs, compiling a form whose products are a batch's (each row its own sequence) or not, with the
/// lane rules `held` back.
#[derive(Clone, Debug)]
pub struct LaneShim {
    pub knobs: CudaKnobs,
    pub batch: bool,
    pub held: Vec<&'static str>,
}

impl LaneShim {
    /// The output rows a warp of a product's kernel takes: four for a wide product in a block format met one row at a
    /// time (`rows_a_warp`), one otherwise.
    pub fn per_warp(&self, p: &GemvForm) -> usize {
        match p.weights {
            Weights::Rows(format) if kernels::sixteen(format) => kernels::sixteen_rows_a_warp(self.knobs.activations, p.n),
            Weights::Rows(format) if !self.held.contains(&"rows_a_warp") => kernels::rows_a_warp(format, p.n, p.k, p.m),
            _ => 1,
        }
    }

    /// A decode mixture's down projection launch (`kernels::moe_down_launch`): its experts' intermediate width from the
    /// mixture's own spec, its shared expert's from that expert's down projection where it is folded in.
    fn moe_down_launch(op: &Op, g: &TensorGraph) -> Option<(usize, usize)> {
        let m = moe(op)?;
        let dff = leaves(op.clone()).iter().find_map(|p| match p.prim()? {
            TensorOpSpec::Moe(s) => Some(s.dff),
            _ => None,
        })?;
        let sdff = match &op.sym {
            Sym::Fused(f) if f.class == ClassId::SharedExpertFusion => gemv(f.parts.get(2)?, g)?.k,
            _ => 0,
        };
        Some(kernels::moe_down_launch(m.d, dff, m.k, sdff))
    }

    /// Whether a product multiplies on the int8 tensor cores: past the GEMV's rows, not a batch's, under the quantized
    /// numerics, by quantized checkpoint rows (a 16-bit float matrix's take the 16-bit tensor cores, as
    /// `Launch::on_tensor_cores` decides).
    pub fn on_tensor_cores(&self, p: &GemvForm) -> bool {
        p.m > GEMV_ROWS && !self.batch && self.knobs.activations != Q8MatKind::Dequant && matches!(p.weights, Weights::Rows(ty) if !kernels::sixteen(ty))
    }

    /// Whether a product multiplies on the 16-bit tensor cores' GEMM (`lk_gemm_h16`): a 16-bit float matrix's, past the
    /// GEMV's rows, not a batch's (whose parts the GEMV's rule merges), under the quantized numerics, whole 32s of `k`.
    fn sixteen_on_tensor_cores(&self, p: &GemvForm) -> bool {
        p.m > GEMV_ROWS
            && !self.batch
            && self.knobs.activations != Q8MatKind::Dequant
            && matches!(p.weights, Weights::Rows(ty) if kernels::sixteen(ty))
            && kernels::sixteen_fits(p.k)
    }

    /// Whether a product lands where a GEMM in parts can write it: a buffer of its own, or a table's rows its width wide.
    fn lands_whole(p: &GemvForm) -> bool {
        p.dst.unit.is_none() && p.dst.row.as_ref().map_or(true, |(_, pitch)| *pitch == p.n)
    }

    /// A product's form, where it is a plain product by checkpoint rows (not one that quantizes its output).
    fn rows_product(op: &Op, g: &TensorGraph) -> Option<GemvForm> {
        gemv(op, g).filter(|p| matches!(p.weights, Weights::Rows(_)))
    }

    /// The product a member of a GEMV in parts computes: a plain product by checkpoint rows, or one that turns its
    /// heads in its epilogue.
    fn part_product(op: &Op, g: &TensorGraph) -> Option<GemvForm> {
        match &op.sym {
            Sym::Fused(f) if f.class == ClassId::QkvHeadEpilogue => Self::rows_product(&f.parts[0], g),
            _ => Self::rows_product(op, g),
        }
    }

    /// Whether the members of a GEMV in parts that turn their heads in its epilogue turn them as one launch of
    /// rotations would: at most a query's and a key's, by one table at one position, one head width, both normed or
    /// neither.
    fn turned_alike(members: &[Op]) -> bool {
        let sides: Vec<_> = members
            .iter()
            .filter_map(|m| match &m.sym {
                Sym::Fused(f) if f.class == ClassId::QkvHeadEpilogue => rope(&f.parts[1]),
                _ => None,
            })
            .collect();
        sides.len() <= ROPE_SIDES
            && sides.iter().all(|s| {
                (&s.cos, &s.sin, &s.pos, s.hd, s.half, s.norm.is_some()) == (&sides[0].cos, &sides[0].sin, &sides[0].pos, sides[0].hd, sides[0].half, sides[0].norm.is_some())
            })
    }

    /// Whether a product is one the GEMV takes in parts: within the GEMV's rows (or a batch's), landing whole, its
    /// outputs whole warps of rows.
    fn gemv_mergeable(&self, p: &GemvForm) -> bool {
        (p.m <= GEMV_ROWS || self.batch) && Self::lands_whole(p) && p.n % self.per_warp(p) == 0
    }

    /// Whether every member of a GEMV in parts is one the GEMV takes together: at most four, one activation, one
    /// format, one number of rows a warp.
    fn gemv_in_parts(&self, members: &[Op], g: &TensorGraph) -> bool {
        let products: Option<Vec<GemvForm>> = members.iter().map(|m| Self::part_product(m, g)).collect();
        let Some(products) = products else { return false };
        let first = &products[0];
        let sixteen = matches!(first.weights, Weights::Rows(format) if kernels::sixteen(format));
        products.len() <= kernels::GEMM_PARTS
            && Self::turned_alike(members)
            && products.iter().all(|p| {
                self.gemv_mergeable(p)
                    && p.weights == first.weights
                    && (p.x.as_str(), p.k, p.m) == (first.x.as_str(), first.k, first.m)
                    && (sixteen || self.per_warp(p) == self.per_warp(first))
            })
    }

    /// The rows a warp of a GEMV in parts takes: a 16-bit float matrix's the fewest any part takes — each part's a power
    /// of two dividing its outputs and its upload's group (`lk_h16_group`), so the fewest divides every part's and lies
    /// in every part's groups — and a quantized matrix's the rows every part takes alike.
    pub fn parts_per_warp(&self, products: &[GemvForm]) -> usize {
        match products.first().map(|p| &p.weights) {
            Some(Weights::Rows(format)) if kernels::sixteen(*format) => products.iter().map(|p| self.per_warp(p)).min().unwrap_or(1),
            Some(_) => self.per_warp(&products[0]),
            None => 1,
        }
    }

    /// Whether every member of a GEMM in parts is one the tensor cores take together: whole tiles of the int8 GEMM's
    /// outputs, or of the 16-bit GEMM's (`gemm_h16_tile_n`), so the parts are one launch.
    fn gemm_parts(&self, members: &[Op], g: &TensorGraph) -> bool {
        let products: Option<Vec<GemvForm>> = members.iter().map(|m| Self::rows_product(m, g)).collect();
        let Some(products) = products else { return false };
        let first = &products[0];
        let tile = |p: &GemvForm| {
            if self.on_tensor_cores(p) {
                Some(kernels::GEMM_TILE_N)
            } else if self.sixteen_on_tensor_cores(p) {
                Some(kernels::gemm_h16_tile_n(self.knobs.activations == Q8MatKind::TwoPassInt8))
            } else {
                None
            }
        };
        products.len() <= kernels::GEMM_PARTS
            && products.iter().all(|p| {
                tile(p).is_some_and(|t| p.n % t == 0)
                    && Self::lands_whole(p)
                    && p.weights == first.weights
                    && (p.x.as_str(), p.k, p.m) == (first.x.as_str(), first.k, first.m)
            })
    }

    /// A gated delta layer's inputs as one launch (`gdn_input_epilogue`, `lk_gemv_gdn`): its products of the activation's
    /// one row, which the GEMV takes in parts; at most three convolutions of a step's one row through the SiLU, of one
    /// length of taps, each reading one product's whole output, the normed ones over heads of one width of at most 1024
    /// values, whole warps, that divide their channels; and its gates — from two of the products, their write strength
    /// and forget gate a head each, or projected from the activation by `f32` matrices beside the GEMV.
    pub fn gdn_inputs<'a>(&self, f: &'a Fused, g: &TensorGraph) -> Result<GdnInputs<'a>, String> {
        let refuse = |why: &str| Err(why.to_string());
        let (mut products, mut convs, mut gating): (Vec<&Op>, Vec<(&Op, ConvForm)>, Option<&Op>) = (Vec::new(), Vec::new(), None);
        for p in &f.parts {
            if Self::rows_product(p, g).is_some() {
                products.push(p);
            } else if let Some(c) = conv(p) {
                convs.push((p, c));
            } else if let (Sym::Fused(cp), Some(ClassId::ConvParts)) = (&p.sym, class(p)) {
                for member in &cp.parts {
                    let Some(c) = conv(member) else { return refuse("a launch of convolutions takes convolutions") };
                    convs.push((member, c));
                }
            } else if matches!(class(p), Some(ClassId::GdnGating | ClassId::GdnGateProjections)) && gating.is_none() {
                gating = Some(p);
            } else {
                return refuse("a gated delta layer's inputs are its products, its convolutions and its gates");
            }
        }
        let Some(gating) = gating else { return refuse("a gated delta layer's inputs make its gates") };
        let members: Vec<Op> = products.iter().map(|p| (*p).clone()).collect();
        let forms: Vec<GemvForm> = products.iter().filter_map(|p| Self::rows_product(p, g)).collect();
        if members.is_empty() || !self.gemv_in_parts(&members, g) || self.batch || forms.iter().any(|p| p.m != 1 || p.dst.own().is_none()) {
            return refuse("a gated delta layer's products are one GEMV in parts of the step's one row");
        }
        let whole = |x: &str, chan: usize| forms.iter().any(|p| p.dst.own() == Some(x) && p.n == chan);
        let normed: BTreeSet<_> = convs.iter().filter_map(|(_, c)| c.l2.map(|(cols, eps)| (cols, eps.get().to_bits()))).collect();
        let heads_fit = convs.iter().all(|(_, c)| c.l2.is_none_or(|(cols, _)| cols <= 1024 && cols % 32 == 0 && c.chan % cols == 0));
        let one = !convs.is_empty()
            && convs.len() <= GDN_CONVS
            && convs.iter().all(|(op, c)| {
                c.rows == 1 && c.silu && c.taps == convs[0].1.taps && c.taps <= kernels::GDN_TAPS && op.access[0] == Access::Whole && whole(&c.x, c.chan)
            })
            && normed.len() <= 1
            && heads_fit;
        if !one {
            return refuse("a gated delta layer convolves in its products' launch at most three of their whole outputs, a step's one row through the SiLU, of one length of at most four taps, the normed ones over heads of one width of at most 1024 values, whole warps, that divide their channels");
        }
        let Sym::Fused(gf) = &gating.sym else { return refuse("a gated delta layer's gates") };
        let gates_fit = match gf.class {
            ClassId::GdnGating => gf.parts.iter().all(|p| match p.prim() {
                Some(TensorOpSpec::Sigmoid(s)) => whole(&p.ins[0], s.n),
                Some(TensorOpSpec::Decay(d)) => d.rows == 1 && whole(&p.ins[0], d.n),
                _ => false,
            }),
            _ => gf.parts.iter().filter_map(|p| gemv(p, g)).all(|p| p.weights == Weights::Values && p.m == 1 && (p.x.as_str(), p.k) == (forms[0].x.as_str(), forms[0].k)),
        };
        if !gates_fit {
            return refuse("a gated delta layer's gates read two of its products whole, a head a row, or project its activation in `f32`");
        }
        Ok(GdnInputs { products, convs: convs.into_iter().map(|(_, c)| c).collect(), gating })
    }
}

/// The convolutions one launch of a gated delta layer's inputs takes: its query's, its key's and its values'.
const GDN_CONVS: usize = 3;

/// A gated delta layer's inputs as the lane runs them in one launch ([`LaneShim::gdn_inputs`]): its products, its
/// convolutions, and the op making its gates.
pub struct GdnInputs<'a> {
    pub products: Vec<&'a Op>,
    pub convs: Vec<ConvForm>,
    pub gating: &'a Op,
}

/// The norm a fold of the embedding's gather and its first norm closes with.
fn embedded_norm(op: &Op) -> Option<crate::tensor_graph::fuse::NormForm> {
    match &op.sym {
        Sym::Fused(f) if f.class == ClassId::EmbedNorm => norm(f.parts.last()?),
        _ => None,
    }
}

/// Whether `op` is a plain norm of a whole row a join of rows makes in `g` (`allreduce_sumsq_norm`'s row).
fn norms_a_joined_row(op: &Op, g: &TensorGraph) -> bool {
    let Some(n) = norm(op).filter(|n| n.add.is_none() && n.part.is_none()) else { return false };
    g.producer(&n.x).and_then(|i| g.ops[i].prim()).is_some_and(|p| matches!(p, TensorOpSpec::Join(j) if j.heads == 1 && j.rows == n.rows && j.wa + j.wb == n.cols))
}

impl Shim for LaneShim {
    fn name(&self) -> String {
        format!("cuda-lane:{}:{:?}", self.knobs.chip.name, self.knobs.activations)
    }

    fn supports(&self, class: ClassId) -> Support {
        use ClassId::*;
        use Support::{Decompose, Native, Refuse};
        let epilogue = if self.knobs.fuse_quant { Native } else { Decompose };
        match class {
            NormWeight | AddNorm | NormThenAdd | NormScale | GatedNorm | EmbedScale | AllreduceNorm | AllreduceSumsqNorm => Native,
            QkNormRope | QkNormRopeKvAppend | RopeKvAppend | RopeQkOneLaunch | Mla => Native,
            FlashDecode | FlashPrefill | AttentionVariantsInline | AttnLseMerge | Softcap | KvCacheQuant => Native,
            GluActMul | GemvGateUpGlu | GemmBias | GemmScale | MergedQkv | MergedGateUp | GemmDequantInkernel | ReluSqr => Native,
            RouterTopk | MoeTokenSort | MoeGateUpAct | MoeDownCombine | SharedExpertFusion => Native,
            GdnStep | ConvBiasSilu | ConvParts | ConvSiluL2norm | GdnGateProjections | GdnGating => Native,
            FusedSampling | CapArgmax | StridedRead | RowWrite | JoinInPlace | StridedValues | CudaGraphs => Native,
            // A matrix in host memory streams to the device a launch at a time (`residence::Plan::streamed`).
            HostWeightStreaming => Native,
            NormQuant | AddNormQuant | GluActMulQuant | ElementwiseQuant | GdnOutputQuant | GemvQuant => epilogue,
            GemvNormEpilogue => Native,
            EmbedNorm => Native,
            NormWeightOffset | DualNorm | PrefillNormEpilogues | AttnResidualNorm => Decompose,
            QkvHeadEpilogue => Native,
            RecomputeFanout | QkNormRopeGateSplit | HorizontalNormRopeInsert | CrossLayerLaunch => Decompose,
            AttnOutputQuant => epilogue,
            AttnOutputGate | RowWriteQuant => Native,
            MlaPrologue | MlaUnabsorbInFold | ElementwiseChains | ElementwiseRegions => Decompose,
            MaskedExpertActivation | AffineInputFold | RouterSharedGateMerge | MoeGatherInOperand => Decompose,
            MoeDeferredFinalize | GdnInputEpilogue | DeltaQkNorm => Native,
            GdnDecodeWhole | GdnPostConvPrep | LastLayerRows => Decompose,
            MixedBatchAttention => Refuse("the lane attends a batch's sequences each at its own position, none mid-prefill"),
            SparseAttention => Refuse("the lane's attention reads every cached row of its span"),
            GroupedTopk => Refuse("the lane routes by one top-k over every expert"),
            Sampling => Refuse("the lane picks the next token by arg-max"),
            HadamardRotation => Refuse("the lane rotates no cache"),
            CompensatedResidual => Refuse("the lane sums its residual in f32, uncompensated"),
            HostResidentKv => Refuse("the lane keeps its caches on the device; only matrices move to host memory"),
            HostLayersOnCpu => Refuse("the lane computes every layer of its step on the device"),
            AttnGqaRowPacking => Refuse("the lane's decode attention runs a head's dots on the CUDA cores"),
            MoeDirectPairs => Refuse("the lane sorts a chunk's rows by expert"),
            PersistentLayerMegakernel => Refuse("the lane replays a graph of a launch an op"),
        }
    }

    fn fits(&self, op: &Op, g: &TensorGraph) -> Result<(), String> {
        holds_accumulator(op, g, 32)?;
        let refuse = |why: &str| Err(why.to_string());
        for leaf in leaves(op.clone()) {
            match leaf.prim() {
                Some(TensorOpSpec::GatedDelta(g)) if g.gate == GateAct::Sigmoid => {
                    return refuse("the lane's gated delta gates by SiLU, not a sigmoid")
                }
                Some(TensorOpSpec::LaneSums(_) | TensorOpSpec::SignedRoot(_)) => {
                    return refuse("the lane computes no lane sum and no signed root")
                }
                Some(TensorOpSpec::NgramIds(_)) => return refuse("the lane hashes no n-gram ids"),
                Some(TensorOpSpec::LaneMean(_) | TensorOpSpec::LaneSpread(_)) => {
                    return refuse("the lane mixes and spreads no hyper-connected residual's lanes")
                }
                Some(TensorOpSpec::FusedRow(r)) if r.weight_rows > 1 => {
                    return refuse("the lane's norms read one weight row, not one for each of their lanes (`in L lanes`)")
                }
                _ => {}
            }
            if let Some(u) = &leaf.dst.unit {
                if !unit_output(&leaf).is_some_and(|(_, width)| width == u.width) {
                    return refuse("the lane writes a part of every unit of a wider tensor only from a norm, a rotation or a per-head product");
                }
            }
            for access in &leaf.access {
                let Access::Strided(s) = access else { continue };
                match leaf.prim() {
                    Some(TensorOpSpec::HeadMatmul(_)) if s.pitch % QBLOCK != 0 || s.offset % QBLOCK != 0 => {
                        return refuse("a per-head product reads a part in place only at whole 32-value blocks");
                    }
                    Some(TensorOpSpec::Attention(_)) if s.pitch % 2 != 0 || s.offset % 2 != 0 => {
                        return refuse("an attention reads its values in place a pair at a time, from an even place");
                    }
                    _ => {}
                }
            }
        }
        if moe(op).is_some() && matches!(op.dst.row, Some((RowSel::Param(_), _))) {
            return refuse("a mixture writes its result at a fixed row, never at one a parameter names");
        }
        let Sym::Fused(f) = &op.sym else { return Ok(()) };
        // A norm of a row joined from parts runs as its own launch, summing the row by its parts.
        if f.class != ClassId::AllreduceSumsqNorm && leaves(op.clone()).iter().any(|l| norms_a_joined_row(l, g)) {
            return refuse("a norm of a row joined from parts runs as its own launch, by its parts");
        }
        match f.class {
            ClassId::NormThenAdd if norm(op).is_none() => Err("the lane's norm-then-add kernel norms the sum it writes".to_string()),
            ClassId::AllreduceSumsqNorm => match parts_norm(op) {
                Some((parts, _, slices)) if parts.len() <= kernels::NORM_PARTS_MAX && slices.len() <= kernels::NORM_PARTS_MAX => Ok(()),
                _ => refuse("the lane norms a row by at most eight parts, writing at most eight of them"),
            },
            ClassId::NormScale if scaled_norm(op).is_none() => refuse("the lane's norm scales the result of its own launch"),
            ClassId::AllreduceNorm => match allreduce_partials(op) {
                Some((partials, _)) if partials.len() <= kernels::FOLD_PARTIALS => Ok(()),
                _ => refuse("the lane's norm folds an all-reduce of at most eight partials"),
            },
            ClassId::GemmScale if epilogued_product(op, g).is_none() => refuse("the lane's product scales the result of its own launch"),
            // relu² folds into its producer's epilogue where nothing reads it quantized, or where the product quantizes
            // what it writes too — four rows a warp, whole blocks; otherwise it keeps its own launch, which quantizes.
            ClassId::ReluSqr if f.parts.len() == 3 => {
                let Some(p) = epilogued_product(op, g) else { return refuse("relu² folds into the epilogue of a product") };
                let quantized = g.ops.iter().any(|o| o.ins.iter().enumerate().any(|(k, t)| *t == op.out && self.activation(o, k, g).is_some()));
                let quantizes = !self.batch && self.per_warp(&p) == 4 && p.n % QBLOCK == 0;
                if !quantized || quantizes {
                    Ok(())
                } else {
                    refuse("relu² folds into its producer only where the product quantizes what a product reads of it, four rows a warp, whole blocks")
                }
            }
            ClassId::GemvNormEpilogue => {
                let (Some(p), Some(n)) = (gemv(&f.parts[0], g), norm(&f.parts[1])) else {
                    return refuse("a projection's epilogue norms the add of its result");
                };
                let rows = matches!(p.weights, Weights::Rows(ty) if kernels::reads_in_place(ty));
                let one_gemv = rows && p.m == 1 && !self.batch && p.dst.own().is_some();
                let whole_row = n.part.is_none() && n.rows == p.m && n.cols == p.n && f.parts[1].dst.own().is_some();
                if !(one_gemv && whole_row) {
                    return refuse("a projection norms in its last block the one row it writes whole, a GEMV of a step's one row by checkpoint rows");
                }
                // A Q8_0 projection's block is the norm's own; a block format's is eight warps or four.
                let block = match p.weights {
                    Weights::Rows(crate::model::quant::GgmlType::Q8_0) => kernels::norm_threads(n.cols),
                    _ => kernels::gemv_blk_norm_threads(self.per_warp(&p)),
                };
                if kernels::block_holds_its_norm(block, n.cols) {
                    Ok(())
                } else {
                    refuse("a projection norms in its last block only where its block is at least the norm's width")
                }
            }
            ClassId::MoeDeferredFinalize => {
                let (Some(m), Some(n)) = (moe(&f.parts[0]), f.parts.last().and_then(norm)) else {
                    return refuse("a mixture's last step norms the add of its result");
                };
                let summed = f.parts.len() == 2 || f.parts[1].prim().is_some_and(|p| matches!(p, TensorOpSpec::Elementwise(e) if e.op == ElementwiseOp::Add));
                let whole_rows = n.part.is_none() && n.rows == m.rows && n.cols == m.d && n.own && m.dst.own().is_some();
                if !(summed && whole_rows) {
                    return refuse("a mixture, alone or summed with another array, adds and norms in its last step the whole rows it writes");
                }
                // The decode step's last down block norms the row; a chunk's or a batch's combine takes a block of the
                // norm's threads a row.
                if m.rows == 1 && !self.batch {
                    let Some((_, threads)) = Self::moe_down_launch(&f.parts[0], g) else {
                        return refuse("a decode mixture's down projection reads its experts' widths from its parts");
                    };
                    if !kernels::block_holds_its_norm(threads, m.d) {
                        return refuse("a decode mixture norms in its last down block only where the block is at least the norm's width");
                    }
                }
                Ok(())
            }
            ClassId::EmbedNorm => {
                let gathered = |o: &Op| match o.prim() {
                    Some(TensorOpSpec::RowsGather(_) | TensorOpSpec::RowGather(_)) => true,
                    _ => class(o) == Some(ClassId::EmbedScale),
                };
                match embedded_norm(op) {
                    Some(n) if gathered(&f.parts[0]) && n.own => Ok(()),
                    _ => refuse("a gather norms in its own blocks the whole rows it gathers"),
                }
            }
            ClassId::QkvHeadEpilogue => {
                let (Some(p), Some(ro)) = (Self::rows_product(&f.parts[0], g), rope(&f.parts[1])) else {
                    return refuse("a projection's epilogue turns the heads of its result");
                };
                let one_row = p.m == 1 && !self.batch && self.gemv_mergeable(&p) && p.dst.own().is_some();
                let its_heads = ro.part.is_none() && ro.heads == ro.per_row && p.n == ro.heads * ro.hd && ro.hd <= kernels::ROPE_HEAD;
                if one_row && its_heads {
                    Ok(())
                } else {
                    refuse("a projection turns in its epilogue the heads of the one row it writes whole, a GEMV of a step's one row")
                }
            }
            ClassId::ConvSiluL2norm => {
                let found = leaves(op.clone());
                let chan = found.iter().find_map(|p| match p.prim()? {
                    TensorOpSpec::CausalConv(c) => Some(c.chan),
                    _ => None,
                });
                let cols = found.iter().find_map(|p| match p.prim()? {
                    TensorOpSpec::L2Norm(l) => Some(l.cols),
                    _ => None,
                });
                match (chan, cols) {
                    (Some(chan), Some(cols)) if cols <= 1024 && cols % 32 == 0 && chan % cols == 0 => Ok(()),
                    _ => refuse("a convolution's epilogue norms heads of at most 1024 values, whole warps, that divide its channels"),
                }
            }
            ClassId::GemvGateUpGlu => {
                let products: Vec<GemvForm> = f.parts.iter().filter_map(|p| gemv(p, g)).collect();
                let (Some(gate), Some(up)) = (products.first(), products.get(1)) else {
                    return refuse("a gated pair is two products");
                };
                if !matches!((gate.weights, up.weights), (Weights::Rows(_), Weights::Rows(_))) {
                    return refuse("the lane pairs a gate and an up product only over checkpoint rows");
                }
                let gemv_rows = gate.m <= GEMV_ROWS || self.knobs.activations == Q8MatKind::Dequant || self.batch;
                let sixteen = matches!(gate.weights, Weights::Rows(ty) if kernels::sixteen(ty));
                // The 16-bit GEMM pairs any hidden width; the int8 GEMM whole pair tiles.
                let paired = !gemv_rows && gate.weights == up.weights && (sixteen || gate.n % (kernels::GEMM_TILE_N / 2) == 0);
                if gemv_rows || paired {
                    Ok(())
                } else {
                    refuse("a gated pair on the tensor cores pairs tiles of one format, the hidden width whole pair tiles in a quantized one")
                }
            }
            ClassId::ConvParts => {
                let members: Option<Vec<ConvForm>> = f.parts.iter().map(conv).collect();
                let Some(members) = members else { return refuse("a launch of convolutions takes convolutions") };
                let normed: BTreeSet<_> = members.iter().filter_map(|c| c.l2.map(|(cols, eps)| (cols, eps.get().to_bits()))).collect();
                let one = members.len() <= CONV_PARTS
                    && members.iter().all(|c| c.rows == 1 && c.silu && c.taps == members[0].taps)
                    && normed.len() <= 1;
                if one {
                    Ok(())
                } else {
                    refuse("one launch convolves a step's one row through the SiLU in at most four parts of one length of taps, the normed ones over heads of one width")
                }
            }
            ClassId::GdnGateProjections => {
                let by_values = f.parts.iter().filter_map(|p| gemv(p, g)).all(|p| p.weights == Weights::Values);
                if by_values {
                    Ok(())
                } else {
                    refuse("the lane takes a delta rule's gates from its activation only by matrices it holds as values")
                }
            }
            ClassId::SharedExpertFusion => {
                let found = leaves(op.clone());
                let swiglu = found.iter().any(|p| matches!(p.prim(), Some(TensorOpSpec::SwiGlu(_))));
                let biased = found.iter().any(|p| matches!(p.prim(), Some(TensorOpSpec::Elementwise(e)) if e.op == ElementwiseOp::Add && e.row.is_some()));
                let rows = found.iter().filter_map(|p| gemv(p, g)).all(|p| matches!(p.weights, Weights::Rows(_)) && p.m == 1);
                match moe(op) {
                    Some(m) if m.k < 32 && m.rows == 1 && m.act == MoeAct::Swiglu && swiglu && !biased && rows => Ok(()),
                    _ => refuse("a mixture folds its unbiased SwiGLU shared expert, a step's one row of checkpoint rows, into at most 32 warps of kept experts"),
                }
            }
            ClassId::RopeQkOneLaunch => {
                let sides: Option<Vec<_>> = f.parts.iter().map(rope).collect();
                let Some(sides) = sides else { return refuse("a launch of rotations takes rotations") };
                let one = sides.len() <= ROPE_SIDES
                    && sides.iter().all(|s| s.heads == s.per_row && s.norm.is_some() == sides[0].norm.is_some());
                if one {
                    Ok(())
                } else {
                    refuse("one launch rotates at most a query and a key of a step's one row, both normed or neither")
                }
            }
            // The gate rides the decode step's, a short chunk's and a batch's sequences' attention stores, a pair a thread;
            // a chunk in tiles gates apart.
            ClassId::AttnOutputGate => {
                let attentions: Vec<_> = f.parts.iter().filter_map(|p| match p.prim() {
                    Some(TensorOpSpec::Attention(a)) => Some(a),
                    _ => None,
                }).collect();
                if !attentions.is_empty() && attentions.iter().all(|a| a.rows <= GEMV_ROWS && a.vd % 2 == 0) {
                    Ok(())
                } else {
                    refuse("an attention gates its output only in the decode step's, a short chunk's and a batch's sequences' kernels, its heads whole pairs")
                }
            }
            // A row set carries its row's quantized form a block at a time: the rows whole 32-value blocks.
            ClassId::RowWriteQuant => match f.parts[0].prim() {
                Some(TensorOpSpec::SetRow(r)) if r.cols % QBLOCK == 0 => Ok(()),
                _ => refuse("a row set carries its rows' quantized form only where they are whole 32-value blocks"),
            },
            // A batch's sequences each quantize their gated row in their store, set with its quantized form into the
            // product's (PARITY-BATCH-ROW-QUANT).
            ClassId::GluActMulQuant if class(&f.parts[0]) == Some(ClassId::AttnOutputGate) => match &f.parts[0].sym {
                Sym::Fused(gated) => {
                    let whole = gated.parts.iter().all(|p| match p.prim() {
                        Some(TensorOpSpec::Attention(a)) => a.vd % 32 == 0,
                        Some(TensorOpSpec::SetRow(r)) => r.cols % QBLOCK == 0,
                        _ => true,
                    });
                    if whole {
                        Ok(())
                    } else {
                        refuse("a gated attention quantizes its output in its store only where its heads, and a batch's rows, are whole 32-value groups")
                    }
                }
                _ => refuse("a gated attention is a fold of its attention and its gate"),
            },
            ClassId::AttnOutputQuant => match f.parts[0].prim() {
                // The decode step's and a short chunk's attention kernels store their output a pair a thread, each 32-value
                // group in one half-warp's sixteen lanes; a chunk the attention takes in tiles quantizes apart.
                Some(TensorOpSpec::Attention(a)) if a.rows <= GEMV_ROWS && a.vd % 32 == 0 => Ok(()),
                _ => refuse("an attention quantizes its output only in the decode step's and a short chunk's kernels, its heads whole 32-value groups"),
            },
            ClassId::NormQuant if norm(&f.parts[0]).or_else(|| embedded_norm(&f.parts[0])).or_else(|| scaled_norm(&f.parts[0]).map(|(n, _)| n)).is_none() => {
                refuse("the lane's norm quantizes its result only from an RMS norm")
            }
            ClassId::GemvQuant => match gemv(&f.parts[0], g).or_else(|| epilogued_product(&f.parts[0], g)) {
                Some(p) if self.per_warp(&p) == 4 && p.n % QBLOCK == 0 && p.dst.own().is_some() => Ok(()),
                _ => refuse("a product quantizes its outputs only four rows a warp, whole blocks of them"),
            },
            ClassId::GluActMulQuant if class(&f.parts[0]) == Some(ClassId::GemvGateUpGlu) => {
                let hidden = match &f.parts[0].sym {
                    Sym::Fused(glu) => glu.parts.iter().find_map(|p| gemv(p, g)).map(|p| p.n),
                    _ => None,
                };
                match hidden {
                    Some(n) if n % QBLOCK == 0 => Ok(()),
                    _ => refuse("a gated pair quantizes its hidden values in whole 32-value blocks"),
                }
            }
            ClassId::GdnInputEpilogue => self.gdn_inputs(f, g).map(|_| ()),
            ClassId::DeltaQkNorm => {
                let Some((delta, norms)) = f.parts.split_last() else { return refuse("a step's delta rule norms its query and key heads") };
                let Some(TensorOpSpec::GatedDelta(d)) = delta.prim() else { return refuse("a step's delta rule norms its query and key heads") };
                let heads = |n: &Op| match n.prim() {
                    Some(TensorOpSpec::L2Norm(l)) if l.cols == d.d && l.rows * l.cols == d.hk * d.d => Some(l.eps.get().to_bits()),
                    _ => None,
                };
                let eps: Option<BTreeSet<u64>> = norms.iter().map(heads).collect();
                let reads: BTreeSet<&str> = norms.iter().map(|n| n.out.as_str()).collect();
                let fits = d.rows == 1 && !self.batch && eps.is_some_and(|e| e.len() == 1) && reads == BTreeSet::from([delta.ins[0].as_str(), delta.ins[1].as_str()]);
                if fits {
                    Ok(())
                } else {
                    refuse("a step's delta rule norms its one row's query and key heads itself, each head of its width by one epsilon")
                }
            }
            ClassId::MergedQkv => {
                // The GEMV in parts is the lane's own rule, held with it.
                if self.gemm_parts(&f.parts, g) || (!self.held.contains(&"gemv_parts") && self.gemv_in_parts(&f.parts, g)) {
                    Ok(())
                } else {
                    refuse("products of one activation are one launch in parts: at most four, one format — on the tensor cores whole tiles of their GEMM's outputs, in the GEMV one number of rows a warp")
                }
            }
            _ => Ok(()),
        }
    }

    fn activation(&self, consumer: &Op, operand: usize, g: &TensorGraph) -> Option<QuantFormat> {
        // A producer that quantizes its own result in its epilogue reads its operands as it reads them alone.
        if let Sym::Fused(f) = &consumer.sym {
            use ClassId::*;
            if matches!(f.class, NormQuant | AddNormQuant | GluActMulQuant | ElementwiseQuant | GdnOutputQuant | GemvQuant | AttnOutputQuant) {
                return self.activation(&f.parts[0], operand, g);
            }
            // A projection with its epilogue folded in reads its operands first, as the projection alone reads them.
            if matches!(f.class, GemvNormEpilogue | QkvHeadEpilogue | MoeDeferredFinalize | GemmScale | ReluSqr) {
                return self.activation(&f.parts[0], operand, g).filter(|_| operand < f.parts[0].ins.len());
            }
            // A gated delta layer's inputs read its activation as their GEMV in parts reads it; gates projected in
            // `f32` read it `f32` beside (`Link::only_projections`).
            if f.class == GdnInputEpilogue {
                let x = f.parts.iter().find_map(|p| Self::rows_product(p, g)).map(|p| p.x);
                return x.filter(|x| consumer.ins.get(operand) == Some(x)).and_then(|_| self.activation(f.parts.iter().find(|p| Self::rows_product(p, g).is_some())?, 0, g));
            }
        }
        let format = match self.knobs.activations {
            Q8MatKind::Dequant => return None,
            Q8MatKind::Int8Act => QuantFormat::OneWord,
            Q8MatKind::TwoPassInt8 => QuantFormat::TwoWords,
        };
        // A product by a 16-bit float matrix reads its activation `f32` (`kernels::sixteen`).
        let quantized = |p: GemvForm| matches!(p.weights, Weights::Rows(ty) if !kernels::sixteen(ty));
        let projected = operand == 0
            && (Self::rows_product(consumer, g).is_some_and(quantized)
                || moe(consumer).is_some()
                || matches!(class(consumer), Some(ClassId::GemvGateUpGlu | ClassId::MergedQkv))
                    && leaves(consumer.clone()).iter().filter_map(|l| gemv(l, g)).all(quantized));
        projected.then_some(format)
    }

    fn rules(&self) -> Vec<(Position, ChipRule<'_>)> {
        vec![
            (Position::Before("add_norm"), ChipRule { name: "latent_expanded", builds: &[ClassId::Mla], apply: Box::new(move |r| self.latent_expanded(r)) }),
            (Position::After("gdn_input_epilogue"), ChipRule { name: "delta_qk_norm", builds: &[ClassId::DeltaQkNorm], apply: Box::new(move |r| self.delta_qk_norm(r)) }),
            (Position::Before("merged_projections"), ChipRule { name: "gemv_parts", builds: &[ClassId::MergedQkv], apply: Box::new(move |r| self.gemv_parts(r)) }),
        ]
    }

    fn numerics(&self) -> Option<Contract> {
        let activations = match self.knobs.activations {
            Q8MatKind::Dequant => None,
            Q8MatKind::Int8Act => Some(Scheme::of(QuantFormat::OneWord)),
            Q8MatKind::TwoPassInt8 => Some(Scheme::of(QuantFormat::TwoWords)),
        };
        // A product by a 16-bit float matrix multiplies on the 16-bit tensor cores under the quantized numerics, its
        // activation in one word of the matrix's format or two (`lk_h16_words`), and in `f32` under the precise ones.
        let sixteen = match self.knobs.activations {
            Q8MatKind::Dequant => None,
            Q8MatKind::Int8Act => Some(SixteenBit { words: 1 }),
            Q8MatKind::TwoPassInt8 => Some(SixteenBit { words: 2 }),
        };
        let cache = match self.knobs.cache_precision {
            CachePrecision::F32 => Keep::F32,
            CachePrecision::F16 => Keep::F16,
            CachePrecision::E4m3 => Keep::E4m3,
        };
        let state = match self.knobs.recurrent_state {
            StatePrecision::F32 => Keep::F32,
            StatePrecision::Bf16 => Keep::Bf16,
        };
        let point = format!("the lane: {:?} activations, {:?} cache, {:?} state", self.knobs.activations, self.knobs.cache_precision, self.knobs.recurrent_state);
        Some(Contract { point, activations, sixteen, cache, state, attention: Self::attention(self.knobs.activations), fused: Bound::EXACT, accumulator_bits: 32 })
    }
}

impl LaneShim {
    /// How a wide chunk's attention takes its operands at `activations`: on the f16 tensor cores under one word (its
    /// queries, keys, values and weights rounded to f16, `lk_attention_mma`); on the int8 tensor cores under two
    /// (`lk_attention_i8`: each query, key and value row in two words under its largest over 127, each weight times its
    /// value row's scale in three under the largest of its row in the tile, every product of the words taken); in
    /// `f32` under the dequantized numerics.
    pub fn attention(activations: Q8MatKind) -> Option<AttentionNumerics> {
        let row = |words| Taken::Quantized(Scheme { block: Block::Row, scale: Scale::MaxOver127, words });
        match activations {
            Q8MatKind::Dequant => None,
            Q8MatKind::Int8Act => Some(AttentionNumerics { queries: Taken::Kept(Keep::F16), keys_values: Taken::Kept(Keep::F16), weights: Taken::Kept(Keep::F16) }),
            Q8MatKind::TwoPassInt8 => Some(AttentionNumerics { queries: row(2), keys_values: row(2), weights: row(3) }),
        }
    }

    /// A step's products of one activation's rows within one GEMV's — in one format, at one number of rows a warp, each
    /// into a buffer of its own or a table's rows its width wide, none quantizing its result, those turning their heads
    /// in the epilogue turning them alike — as one GEMV over their outputs side by side, at the first one's place: the
    /// short ones ride with the wide. Nothing between them may write the activation or what a later member reads, or
    /// touch what it writes.
    fn gemv_parts(&self, r: &mut Rewriter) {
        for i in 0..r.s.len() {
            let Some(first) = r.s.slot(i).and_then(|o| Self::part_product(o, r.g)).filter(|p| self.gemv_mergeable(p)) else { continue };
            let mut group = vec![i];
            for j in i + 1..r.s.len() {
                let Some(op) = r.s.slot(j) else { continue };
                if group.len() == kernels::GEMM_PARTS || facts::writes(op).contains(&first.x.as_str()) {
                    break;
                }
                let members: Vec<Op> = group.iter().map(|&k| r.s.get(k).clone()).chain([op.clone()]).collect();
                let fits = Self::part_product(op, r.g).is_some() && self.gemv_in_parts(&members, r.g);
                if !fits {
                    continue;
                }
                let (written, read) = (facts::writes(op), facts::reads(op));
                let crossed = (i + 1..j).filter(|b| !group.contains(b)).filter_map(|b| r.s.slot(b)).any(|between| {
                    let (br, bw) = (facts::reads(between), facts::writes(between));
                    written.iter().any(|t| br.contains(t) || bw.contains(t)) || read.iter().any(|t| bw.contains(t))
                });
                if !crossed {
                    group.push(j);
                }
            }
            if group.len() < 2 {
                continue;
            }
            let members: Vec<Op> = group.iter().map(|&k| r.s.get(k).clone()).collect();
            let mut ins: Vec<(String, Access)> = vec![(first.x.clone(), members[0].access[0])];
            ins.extend(members.iter().flat_map(|m| m.ins[1..].iter().map(|t| (t.clone(), Access::Whole))));
            let (out, dst) = (members[0].out.clone(), members[0].dst.clone());
            r.fuse("gemv_parts", ClassId::MergedQkv, i, &group[1..], ins, out, dst);
        }
    }

    /// A step's delta rule norms its query and key heads itself where its inputs' launch could not norm them in the blocks
    /// that compute them: a one-row delta rule whose query and key the convolutions of one inputs' launch
    /// (`gdn_input_epilogue`) norm over heads of the delta's width, read by nothing else, where a block of that launch —
    /// eight warps, or the head's width of threads where that is more, `per_warp` rows a warp — holds less than a head.
    /// The launch hands the norms to the step (`delta_qk_norm`) and writes the heads unnormed; the step norms each head as
    /// the convolution's norm sums it (`lk_head_den`), so every value it reads is the one the convolution wrote, and the
    /// launch holds no head's count and its query and key blocks no fence. A launch whose blocks each hold whole heads
    /// keeps its norms (Qwen3.5-0.8B in BF16, sixteen rows a warp, a block one head of 128, decoded 192.3 tokens a second
    /// so against 191.4 with every warp of the step norming its head again).
    fn delta_qk_norm(&self, r: &mut Rewriter) {
        for e in 0..r.s.len() {
            let Some(Sym::Fused(f)) = r.s.slot(e).map(|o| &o.sym) else { continue };
            if f.class != ClassId::GdnInputEpilogue {
                continue;
            }
            let Ok(inputs) = self.gdn_inputs(f, r.g) else { continue };
            let Some((cols, _)) = inputs.convs.iter().find_map(|c| c.l2) else { continue };
            let forms: Vec<GemvForm> = inputs.products.iter().filter_map(|p| Self::rows_product(p, r.g)).collect();
            if cols.max(256) / 32 * self.parts_per_warp(&forms) >= cols {
                continue;
            }
            let normed: Vec<String> = inputs.convs.iter().filter(|c| c.l2.is_some()).map(|c| c.y.clone()).collect();
            let alone = |r: &Rewriter, t: &str| -> Option<usize> {
                match r.s.readers(t)[..] {
                    [j] if !r.observed(t) => Some(j),
                    _ => None,
                }
            };
            let step = normed.iter().find_map(|q| {
                let j = alone(r, q)?;
                let op = r.s.get(j);
                let Some(TensorOpSpec::GatedDelta(d)) = op.prim() else { return None };
                let (q, k) = (op.ins[0].clone(), op.ins[1].clone());
                let both = [&q, &k].iter().all(|t| normed.contains(t) && alone(r, t) == Some(j));
                (both && d.rows == 1 && d.d == cols).then_some((j, q, k))
            });
            if let Some((j, q, k)) = step {
                r.hand_over("delta_qk_norm", ClassId::DeltaQkNorm, e, &[q, k], j);
            }
        }
    }

    /// A wide chunk's latent attention whole (`mla`), where the lane runs it expanded: under the quantized numerics, a
    /// chunk of more rows than the GEMV takes attending the cache absorbed — each head's unrotated query sliced from the
    /// query projection and absorbed by the key half of the latent's expansion, joined with its rotated part, attending
    /// every cached row with the rows' latents as its values, and the attended latent expanded by the value half. The
    /// op's meaning is those parts; the lane runs each cached latent row through both halves instead, and each head's
    /// query attends its own expanded keys and values, multiplying each cached row once rather than every query a
    /// head's whole latent wide. Taken where both halves and the attention run in tiles.
    fn latent_expanded(&self, r: &mut Rewriter) {
        let numerics = self.knobs.activations;
        if numerics == Q8MatKind::Dequant || self.batch {
            return;
        }
        let whole = |op: &Op| op.access.iter().all(|a| *a == Access::Whole);
        // A latent's halves in a 16-bit float format run apart: the per-head kernels decode quantized sub-blocks.
        let rows_of = |t: &str| match r.g.tensors.get(t).map(|t| &t.storage) {
            Some(Storage::Rows { format, transposed }) if !kernels::sixteen(*format) => Some((*format, *transposed)),
            _ => None,
        };
        let mut found = Vec::new();
        for i in 0..r.s.len() {
            let Some(op) = r.s.slot(i) else { continue };
            let Some(TensorOpSpec::Attention(a)) = op.prim() else { continue };
            if a.sinks.is_some() || a.window.is_some() || a.kv != 1 || a.rows <= GEMV_ROWS || a.vd >= a.hd || !whole(op) {
                continue;
            }
            let (q, kc, vc, ol) = (op.ins[0].clone(), op.ins[1].clone(), op.ins[2].clone(), op.out.clone());
            let pos = match &a.pos {
                Some(RowIndex::Named(_)) => Some(op.ins[3].clone()),
                Some(RowIndex::Literal(_)) => continue,
                None => None,
            };
            let (heads, kw, lora, len, rows, scale) = (a.h, a.hd, a.vd, a.l, a.rows, a.scale.map_or(1.0 / (a.hd as f64).sqrt(), |s| s.get()));
            let cap = a.softcap.map(|c| c.get());
            let rope = kw - lora;
            // The values: the latent part of every cache row.
            let Some(vi) = r.s.producer(&vc) else { continue };
            if !slice(r.s.get(vi)).is_some_and(|v| v.x == kc && v.every == kw && v.from == 0 && v.width == lora) || r.s.readers(&vc) != [i] {
                continue;
            }
            // The query: each head's absorbed part, then its rotated part.
            let Some(ji) = r.s.producer(&q) else { continue };
            let join = r.s.get(ji);
            let Some(TensorOpSpec::Join(j)) = join.prim() else { continue };
            if join.dst.own() != Some(q.as_str()) || !whole(join) || (j.rows, j.heads, j.wa / j.heads, j.wb / j.heads) != (rows, heads, lora, rope) || r.s.readers(&q) != [i] {
                continue;
            }
            let (ql, qpp) = (join.ins[0].clone(), join.ins[1].clone());
            let Some(ai) = r.s.producer(&ql) else { continue };
            let absorb = r.s.get(ai);
            let Some(TensorOpSpec::HeadMatmul(m)) = absorb.prim() else { continue };
            if absorb.dst.own() != Some(ql.as_str()) || !whole(absorb) || (m.m, m.heads, m.b) != (rows, heads, lora) || r.s.readers(&ql) != [ji] {
                continue;
            }
            let (qn, wkb, nope) = (absorb.ins[0].clone(), absorb.ins[1].clone(), m.a);
            let Some((fk, kro)) = rows_of(&wkb) else { continue };
            let qk = nope + rope;
            let Some(qi) = r.s.producer(&qn) else { continue };
            let Some(qs) = slice(r.s.get(qi)).filter(|s| (s.groups, s.every, s.from, s.width) == (rows * heads, qk, 0, nope)) else { continue };
            if r.s.readers(&qn) != [ai] {
                continue;
            }
            let qr = qs.x;
            let Some(ri) = r.s.producer(&qpp) else { continue };
            let rot = r.s.get(ri);
            if !matches!(rot.prim(), Some(TensorOpSpec::Rope(_))) || rot.dst.own() != Some(qpp.as_str()) || !whole(rot) || r.s.readers(&qpp) != [ji] {
                continue;
            }
            let rotated_from_query = r.s.producer(&rot.ins[0]).and_then(|e| slice(r.s.get(e))).is_some_and(|e| e.x == qr && (e.every, e.from, e.width) == (qk, nope, rope));
            if !rotated_from_query {
                continue;
            }
            // The attended latent through the value half.
            let [ci] = r.s.readers(&ol)[..] else { continue };
            let expand = r.s.get(ci);
            let Some(TensorOpSpec::HeadMatmul(c)) = expand.prim() else { continue };
            let Some((fv, true)) = rows_of(&expand.ins[1]) else { continue };
            let Some(ct) = expand.dst.own().map(str::to_string) else { continue };
            if !whole(expand) || (c.m, c.heads, c.a) != (rows, heads, lora) {
                continue;
            }
            let latent = Latent {
                qr,
                qpp,
                kc,
                pos,
                heads,
                kw,
                lora,
                nope,
                len,
                rows,
                scale,
                cap,
                wkb,
                fk,
                kro,
                wvb: expand.ins[1].clone(),
                fv,
                vd: c.b,
                ct,
            };
            if self.latent_launches(&latent).is_some_and(|[kh, vh, attend]| attend.attends_in_tiles(numerics) && kh.heads_in_tiles(numerics) && vh.heads_in_tiles(numerics)) {
                found.push((i, [qi, ai, ji, vi, ci], expand.dst.clone()));
            }
        }
        for (i, gone, dst) in found {
            let members: Vec<Op> = std::iter::once(i).chain(gone).map(|k| r.s.get(k).clone()).collect();
            let made: Vec<&str> = members.iter().map(|m| m.out.as_str()).collect();
            let mut ins: Vec<(String, Access)> = Vec::new();
            let mut parts = members.clone();
            parts.sort_by_key(|p| p.covers.first().copied().unwrap_or(0));
            for p in &parts {
                for (t, a) in p.ins.iter().zip(&p.access) {
                    if !made.contains(&t.as_str()) && !ins.iter().any(|(n, _)| n == t) {
                        ins.push((t.clone(), *a));
                    }
                }
            }
            r.fuse("latent_expanded", ClassId::Mla, i, &gone, ins, dst.buf.clone(), dst);
        }
    }

    /// A latent attention's expanded launches: the key half and the value half over every cached latent row, then the
    /// attention of each head's split query over its own keys and values — `None` where one block's shared memory holds
    /// no head.
    pub fn latent_launches(&self, l: &Latent) -> Option<[Launch; 3]> {
        let numerics = self.knobs.activations;
        let acc = kernels::accumulator(kernels::mode(numerics)).0;
        let sms = self.knobs.chip.sms;
        let qk = l.nope + (l.kw - l.lora);
        let upto = l.pos.clone().map(|p| (p, l.rows - 1));
        let half = |w: &str, format, b, rows_out, y: &str| Launch::HeadGemv {
            x: l.kc.clone(),
            part: Some(LanePart { pitch: l.kw, offset: 0 }),
            w: w.to_string(),
            format,
            y: LaneDst::to(y),
            m: l.len,
            heads: l.heads,
            a: l.lora,
            b,
            rows_out,
            shared: true,
            upto: upto.clone(),
            batch: false,
        };
        let (keys, values) = (format!("{}.keys", l.ct), format!("{}.values", l.ct));
        let kh = half(&l.wkb, l.fk, l.nope, !l.kro, &keys);
        let vh = half(&l.wvb, l.fv, l.vd, true, &values);
        let span = attended_span(l.len, None);
        let per_block = kernels::attn_heads_per_block(1, qk, l.vd, span, sms, self.knobs.chip.smem_kib_per_block * 1024, acc)?;
        let splits = kernels::attn_splits(l.heads / per_block, span, sms);
        let attend = Launch::Attention {
            q: l.qr.clone(),
            kc: keys,
            vc: values,
            pos: l.pos.clone(),
            sinks: None,
            out: l.ct.clone(),
            heads: l.heads,
            group: 1,
            hd: qk,
            vd: l.vd,
            len: l.len,
            rows: l.rows,
            window: None,
            chunked: false,
            vstride: l.heads * l.vd,
            voff: 0,
            scale: l.scale,
            cap: l.cap,
            per_block,
            splits,
            parallel: chunks_apart(splits, l.rows, l.heads / per_block, sms),
            split: Some(Split {
                own: l.nope,
                q_pitch: qk,
                q_tail: (l.qpp.clone(), LanePart { pitch: l.kw - l.lora, offset: 0 }),
                k_tail: (l.kc.clone(), LanePart { pitch: l.kw, offset: l.lora }),
            }),
            shared: None,
            seq: None,
            f32: true,
            quant: false,
            gate: None,
            goff: 0,
            stream: 0,
        };
        Some([kh, vh, attend])
    }
}

/// A latent attention's shape and names, as its expanded launches read them: the query projection `qr` (each head's
/// unrotated part its first `nope` values) and the rotated part `qpp`; the cache `kc`, `kw` wide a row — the latent
/// `lora` values then the shared rotated key — `len` rows; the key and value halves of the latent's expansion and their
/// formats (`kro`: the key half's checkpoint rows are its outputs); the result `ct`.
pub struct Latent {
    pub qr: String,
    pub qpp: String,
    pub kc: String,
    pub pos: Option<String>,
    pub heads: usize,
    pub kw: usize,
    pub lora: usize,
    pub nope: usize,
    pub len: usize,
    pub rows: usize,
    pub scale: f64,
    pub cap: Option<f64>,
    pub wkb: String,
    pub fk: crate::model::quant::GgmlType,
    pub kro: bool,
    pub wvb: String,
    pub fv: crate::model::quant::GgmlType,
    pub vd: usize,
    pub ct: String,
}

/// Whether an attention cut into `splits` chunks runs a block a chunk: where its rows' head blocks alone leave the chip
/// idle.
pub fn chunks_apart(splits: usize, rows: usize, head_blocks: usize, sms: usize) -> bool {
    splits > 1 && rows * head_blocks < 2 * sms
}
