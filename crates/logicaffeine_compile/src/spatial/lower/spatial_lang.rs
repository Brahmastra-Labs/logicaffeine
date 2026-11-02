//! **Spatial source emission** for a VERIFIED `SpatialProgram` — the RDU's own front-door language (Spatial,
//! Koeplinger et al. PLDI 2018; the Plasticine/RDU lineage; the public Spatial → PIR → plastiroute → Tungsten
//! chain built by `scripts/plasticine/`). The `SpatialProgram` twin of `codegen_sva::spatial_emit`,
//! which prints a `LoopFabric`.
//!
//! Every construct printed is the image of one IR object:
//!   * a tensor is a `DRAM[T]`, filled on the host from its recorded fill;
//!   * a buffer is an `SRAM[T]` sized by the page its producer fills (Spatial N-buffers a memory written in
//!     one controller and read in the next, so the IR's depth is recorded beside the declaration rather
//!     than spelled), or a `FIFO[T]` when its edge is a `Fifo` handshake;
//!   * a stage is one controller: a Load is a `load` (DRAM → SRAM), a Store a `store`, a `MatmulTiles`
//!     the row/column `Foreach` nest around a lane-parallel `Reduce` dot product — inside a `MemReduce`
//!     over the K blocks when the IR spills partials through an `Interm` buffer — and a `Generic` an
//!     element-wise `Foreach` whose reduce axis is a `Reduce`, its operands read through their access maps
//!     with an absent axis reading coordinate 0 (the broadcast), exactly as `spatial::eval` reads them;
//!   * the host checks the output's CHECKSUM against the gold: the spatial oracle's answer
//!     (`eval_choreography` over the recorded fills) unless the caller hands over the VM's.
//! The handshake ops (`Reserve`/`Push`/`Wait`/`Pop`, the accumulator acquire/release, the barriers) have no
//! spelling of their own: Spatial's compiler derives the buffering and the pipeline from the controller
//! nest, which is why a depth is a comment and not a construct.
//!
//! What is not spelled is refused BY NAME (`SpatialLowerError::Unsupported`): multicast groups and `Recv`
//! stages, `Scalar` bodies, the fabric's fused epilogues, external or vendor-random fills, weight-stationary
//! dataflow, cross-site handshakes, non-rectangular tile blocks, aliased buffers, sites carrying several
//! work items. A multi-site program prints as a `Parallel` of per-site `Pipe`s — spelled, not yet gated
//! through the chain (the gate runs the single-site RDU GEMM).

use std::collections::{BTreeMap, BTreeSet};

use super::{require_verified, SpatialLowerError};
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::eval::{eval_choreography, Inputs, Value};
use crate::spatial::ir::*;
use crate::spatial::passes::{LV_ITEM, LV_ITEM_X, LV_ITEM_Y, LV_KBLOCK, LV_SUB_H, LV_SUB_W};
use crate::spatial::target::{DataflowPolicy, SpatialTarget, TileShape};

/// The Spatial scalar the whole app computes in. `Int` when every value the program touches provably fits
/// 32 bits (what the chain's GEMM regression apps use), `Long` otherwise, `Float` for any float precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scalar {
    Int,
    Long,
    Float,
}

impl Scalar {
    fn spatial(self) -> &'static str {
        match self {
            Scalar::Int => "Int",
            Scalar::Long => "Long",
            Scalar::Float => "Float",
        }
    }
    fn is_float(self) -> bool {
        self == Scalar::Float
    }
}

/// Which half of the public chain an app is printed for — the two differ only in how a fold is spelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpatialFlavor {
    /// PIR and the fabric: every fold a `Reduce`, which PIR maps onto a PCU's reduction tree.
    Fabric,
    /// Spatial's Scala simulator: every fold an explicit register scan — seeded by the first point, combined in
    /// order, as the oracle folds. This Spatial's Scala backend drops the first-iteration flags of an unrolled
    /// `Reduce` when it splits the node's kernel out (the kernel reads flags it is never passed).
    Simulator,
}

/// The Spatial type a memory is declared with: `T`, the app's scalar, or `I` — the integer scalar of a program that
/// holds floats and integers both (its indices, positions and tokens). A program of one class declares everything `T`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    T,
    I,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::T => "T",
            Ty::I => "I",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Gold {
    Int(i64),
    Float(f64),
}

/// A stage's shape, read ONCE from the IR (this is the one place the compute enum is spelled): every
/// printer below matches on this view, so the case analysis over compute shapes has exactly one home.
enum Shape<'a> {
    Load,
    Store,
    Recv,
    Matmul {
        block: &'a BlockShape,
        sub_blocks: (usize, usize),
        num_blocks: usize,
        operand: Precision,
        acc: AccPolicy,
        dataflow: DataflowPolicy,
        interm: Option<BufferId>,
    },
    Generic {
        iters: &'a [IterSpec],
        ins: &'a [ElemAccess],
        out: &'a ElemAccess,
        body: &'a TensorBody,
    },
    /// The fabric's fused epilogues (requant, bias, relu).
    Epilogue,
    Scalar,
    /// A cross-chip reduction of the partials of a tensor-parallel shard.
    AllReduce,
}

fn shape(s: &Stage) -> Shape<'_> {
    match &s.kind {
        StageKind::Load { .. } => Shape::Load,
        StageKind::Store { .. } => Shape::Store,
        StageKind::Recv { .. } => Shape::Recv,
        StageKind::AllReduce { .. } => Shape::AllReduce,
        StageKind::Scalar(_) => Shape::Scalar,
        StageKind::Compute(ComputeOp::MatmulTiles {
            block,
            sub_blocks,
            num_blocks,
            operand,
            acc,
            dataflow,
            interm,
            ..
        }) => Shape::Matmul {
            block,
            sub_blocks: *sub_blocks,
            num_blocks: *num_blocks,
            operand: *operand,
            acc: *acc,
            dataflow: *dataflow,
            interm: *interm,
        },
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins,
            out,
            body,
        }) => Shape::Generic {
            iters,
            ins,
            out,
            body,
        },
        StageKind::Compute(_) => Shape::Epilogue,
    }
}

fn ident(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn dram_name(t: &TensorDecl) -> String {
    format!("d_{}", ident(&t.name))
}

/// Every buffer's Scala identifier: its name, and its id too where two buffers share a name — a section whose
/// work items share a site clones its buffers under one name, and one Scala scope holds them all.
fn buffer_names(p: &SpatialProgram) -> BTreeMap<BufferId, String> {
    let mut uses: BTreeMap<String, usize> = BTreeMap::new();
    for b in &p.buffers {
        *uses.entry(ident(&b.name)).or_default() += 1;
    }
    p.buffers
        .iter()
        .map(|b| {
            (
                b.id,
                if uses[&ident(&b.name)] > 1 {
                    format!("{}_b{}", ident(&b.name), b.id.idx())
                } else {
                    ident(&b.name)
                },
            )
        })
        .collect()
}

fn is_float_precision(p: Precision) -> bool {
    matches!(
        p,
        Precision::Fp8 | Precision::Bf16 | Precision::Fp16 | Precision::Fp32
    )
}

/// Whether any constant of `body` satisfies `pred`.
fn body_has_const(body: &TensorBody, pred: &dyn Fn(u64) -> bool) -> bool {
    match body {
        TensorBody::Const(bits) => pred(*bits),
        TensorBody::Operand(_) => false,
        TensorBody::Add(x, y)
        | TensorBody::Sub(x, y)
        | TensorBody::Mul(x, y)
        | TensorBody::Min(x, y)
        | TensorBody::Max(x, y)
        | TensorBody::FloorDiv(x, y) => body_has_const(x, pred) || body_has_const(y, pred),
        TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => body_has_const(arg, pred),
        TensorBody::Select {
            then, otherwise, ..
        } => body_has_const(then, pred) || body_has_const(otherwise, pred),
    }
}

/// A literal of type `ty` — a float Scala parses as a `Double` when `float`, else an integer — lifted to it.
fn lit_of(v: f64, ty: Ty, float: bool) -> String {
    let s = if float {
        format!("{v:?}")
    } else {
        (v as i64).to_string()
    };
    if v < 0.0 {
        format!("({s}).to[{}]", ty.name())
    } else {
        format!("{s}.to[{}]", ty.name())
    }
}

/// `expr`, of type `from`, as a `to`.
fn cast(expr: String, from: Ty, to: Ty) -> String {
    if from == to {
        expr
    } else {
        format!("{expr}.to[{}]", to.name())
    }
}

/// A float literal Scala parses as a `Double`, lifted to `T`.
fn float_lit(v: f64) -> String {
    let s = format!("{v:?}");
    if v < 0.0 {
        format!("({s}).to[T]")
    } else {
        format!("{s}.to[T]")
    }
}

/// A 64-bit host literal for the checksum comparison: `(vL: Long)` is legal for every `i64` and lifted by
/// Spatial's `FixFromLong` (a bare `.to[Long]` on a wide literal is "integer number too large").
fn long_lit(v: i64) -> String {
    format!("({v}L: Long)")
}

/// The largest lane count that divides `n`, at most the PCU's 16 SIMD lanes — the `par` of the dot product.
fn lanes(n: usize) -> usize {
    (1..=16.min(n.max(1)))
        .rev()
        .find(|l| n % l == 0)
        .unwrap_or(1)
}

// ── Refusals ──────────────────────────────────────────────────────────────────────────────────────

/// Every construct the emitter does not spell, refused by name before anything is printed.
fn refuse_unspelled(p: &SpatialProgram) -> Result<(), SpatialLowerError> {
    let pl = p.placement.as_ref().expect("a verified program is placed");
    let non_finite = |bits: u64| !f64::from_bits(bits).is_finite();
    let fills_non_finite = |t: &TensorDecl| match &t.init {
        TensorInit::ConstF(b) => non_finite(*b),
        TensorInit::FloatValues(v) => v
            .iter()
            .take(t.elems.rows * t.elems.cols)
            .any(|b| non_finite(*b)),
        _ => false,
    };
    let body_non_finite = |s: &Stage| match shape(s) {
        Shape::Generic { body, .. } => body_has_const(body, &non_finite),
        _ => false,
    };
    if p.tensors.iter().any(fills_non_finite) || p.stages.iter().any(body_non_finite) {
        return Err(SpatialLowerError::Unsupported("a non-finite float constant (±∞ or NaN, in a body or a fill) is not spelled — Spatial has no verified FltPt literal for one"));
    }
    let multicast = !pl.groups.is_empty()
        || p.stages.iter().any(|s| {
            matches!(s.kind, StageKind::Recv { .. })
                || matches!(
                    s.kind,
                    StageKind::Load {
                        multicast: Some(_),
                        ..
                    }
                )
        });
    if multicast {
        return Err(SpatialLowerError::Unsupported("multicast groups (a Load that broadcasts a block to a rectangle of sites, and the Recv stages that receive it) are not spelled — Spatial has no group broadcast; place the program without reader fusion"));
    }
    for s in &p.stages {
        match shape(s) {
            Shape::Scalar => {
                return Err(SpatialLowerError::Unsupported("a Scalar stage (the fabric's LoopCell datapath) is not spelled — print it through codegen_sva::spatial_emit's counted-loop form"));
            }
            Shape::Matmul {
                dataflow,
                num_blocks,
                interm,
                ..
            } => {
                if dataflow == DataflowPolicy::WeightStationary {
                    return Err(SpatialLowerError::Unsupported("a weight-stationary MatmulTiles dataflow (the weight block resident across output blocks) is not spelled — only the output-stationary nest is"));
                }
                if num_blocks > 1 && interm.is_none() {
                    return Err(SpatialLowerError::Unsupported("a MatmulTiles reduction over several K blocks with no Interm buffer (the accumulator held in registers across blocks) is not spelled — the MemReduce needs the spill buffer"));
                }
            }
            Shape::Generic {
                iters, ins, out, ..
            } => {
                if iters.len() != 2 {
                    return Err(SpatialLowerError::Unsupported("a Generic compute stage with a contraction axis (LV_GEN_K, a third folded iteration variable) — no Spatial controller spelled for it"));
                }
                if ins.len() != s.ins.len() {
                    return Err(SpatialLowerError::Unsupported(
                        "a Generic compute whose access maps do not match its operand buffers",
                    ));
                }
                if out.indirect.is_some() {
                    return Err(SpatialLowerError::Unsupported("an indirect (value-dependent) OUTPUT map — an element scatter — is not spelled; the oracle gives it no semantics either"));
                }
            }
            Shape::Epilogue => {
                return Err(SpatialLowerError::Unsupported("the fabric's fused epilogues (requant, bias, relu) are not spelled — Spatial's compute unit has no packer epilogue slot; legalize them into a Generic"));
            }
            Shape::AllReduce => {
                return Err(SpatialLowerError::Unsupported("a cross-chip AllReduce (the sum of a tensor-parallel shard's partials) is not spelled — one Spatial app describes ONE RDU; emit the per-chip program and let the host collective combine them"));
            }
            Shape::Load | Shape::Store => {
                let (tensor, pattern) = match &s.kind {
                    StageKind::Load {
                        tensor, pattern, ..
                    }
                    | StageKind::Store {
                        tensor, pattern, ..
                    } => (p.tensor(*tensor), pattern),
                    _ => unreachable!(),
                };
                if pattern.stride_w != 1 || pattern.stride_h != tensor.tiles.cols_t {
                    return Err(SpatialLowerError::Unsupported("a Load or Store whose tile block is not a contiguous rectangle (stride_w other than 1, stride_h other than the tensor's tile row) is not spelled as one DRAM range"));
                }
                if pattern.origin.indirect.is_some() {
                    return Err(SpatialLowerError::Unsupported("an indirect (value-dependent) tile address — a gathered Load or scattered Store — is not spelled; a DRAM range needs an affine origin"));
                }
            }
            Shape::Recv => unreachable!("refused above"),
        }
    }
    if !p.trips.is_empty() {
        return Err(SpatialLowerError::Unsupported("a loop whose count the program reads from its data (a trip) is not spelled — the emitter prints every K-block loop with a fixed count"));
    }
    for e in &p.edges {
        if !matches!(e.handshake, Handshake::Fifo | Handshake::ValidReady) {
            return Err(SpatialLowerError::Unsupported("a cross-site handshake edge (Token, Semaphore or Barrier) is not spelled — Spatial has no inter-controller semaphore; every edge must be a same-site Fifo or ValidReady hand-off"));
        }
    }
    for b in &p.buffers {
        if b.alias_of.is_some() {
            return Err(SpatialLowerError::Unsupported("an aliased buffer (a spill sharing its memory region with the output) is not spelled — Spatial memories do not alias"));
        }
    }
    for t in &p.tensors {
        match t.init {
            TensorInit::External => {
                return Err(SpatialLowerError::Unsupported("a tensor whose fill is External has no host data to fill its DRAM with — the regression harness needs a deterministic fill"));
            }
            TensorInit::RandomBf16 { .. } => {
                return Err(SpatialLowerError::Unsupported("a tensor with a RandomBf16 fill — the vendor's generator is not reproduced on the host"));
            }
            _ => {}
        }
    }
    // ★ SEVERAL ITEMS ON ONE SITE ONLY MATTERS IF SOMETHING INDEXES BY THE ITEM.
    //
    // Every stage carries its OWN work item (`Stage::item`), and `resolve_origin` already reads that
    // item's coordinate for `LV_ITEM_Y` / `LV_ITEM_X`. So a site holding one stage per section resolves
    // each of them against the right item and needs no loop printed at all. The single thing this
    // emitter cannot spell is `LV_ITEM` — the item's LINEAR index — which `resolve_origin` pins to zero
    // because it had been guaranteed one item per site.
    //
    // Refusing every multi-item site to protect that one case refused the whole composed Qwen3.8
    // forward, whose 117 sections stack onto shared sites. Measured on that program: the schedule
    // contains 39 counted loops and every one of them is `LV_KBLOCK`; `LV_ITEM` does not appear. So the
    // guard was turning away a program it could spell correctly. It now refuses exactly what it cannot
    // resolve — an address that reads the linear item index on a site carrying more than one, or an
    // `LV_ITEM` loop anywhere in the schedule, whose body it does not print.
    let crowded: std::collections::BTreeSet<Site> = pl
        .items_of_site
        .iter()
        .filter(|(_, v)| v.len() != 1)
        .map(|(s, _)| *s)
        .collect();
    if !crowded.is_empty() {
        let reads_item = |at: &TileAddr| at.steps.iter().any(|(v, s)| *v == LV_ITEM && *s != 0);
        for st in &p.stages {
            let on_crowded = st.site.map(|s| crowded.contains(&s)).unwrap_or(false);
            if !on_crowded {
                continue;
            }
            let at = match &st.kind {
                StageKind::Load { pattern, .. } | StageKind::Store { pattern, .. } => {
                    Some(&pattern.origin)
                }
                _ => None,
            };
            if at.map(reads_item).unwrap_or(false) {
                return Err(SpatialLowerError::Unsupported("a tile address that reads the linear work-item index on a site carrying several items — the emitter resolves an item's coordinate but has no counted loop to give that index a value"));
            }
        }
        if let Some(sched) = p.schedule.as_ref() {
            let mut loops_item = false;
            for prog in &sched.programs {
                walk_tagged_ops(&prog.ops, 1, &mut |op, _| {
                    if let ChoreoOp::Loop { var, .. } = op {
                        loops_item |= *var == LV_ITEM;
                    }
                });
            }
            if loops_item {
                return Err(SpatialLowerError::Unsupported("a counted loop over work items — the emitter prints the K-block loop only, so an item loop's body would be spelled once instead of once per item"));
            }
        }
    }
    Ok(())
}

// ── The oracle: fills in, checksum out ────────────────────────────────────────────────────────────

/// Every Input/Param tensor materialized from its recorded fill.
fn fill_inputs(p: &SpatialProgram) -> Result<Inputs, SpatialLowerError> {
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
                Value::Float(vals.ok_or(SpatialLowerError::Unsupported(
                    "a tensor whose value fill is shorter than the tensor",
                ))?),
            );
        } else {
            let vals = (0..n)
                .map(|i| t.init.element(i))
                .collect::<Option<Vec<i64>>>();
            inputs.tensors.insert(
                t.id,
                Value::Int(vals.ok_or(SpatialLowerError::Unsupported(
                    "a tensor whose value fill is shorter than the tensor",
                ))?),
            );
        }
    }
    Ok(inputs)
}

/// The checksum the host compares: the wrapping sum of an integer output, the plain sum of a float one.
fn checksum_gold(out: &Value) -> Gold {
    match out {
        Value::Int(v) => Gold::Int(v.iter().fold(0i64, |s, x| s.wrapping_add(*x))),
        Value::Float(v) => Gold::Float(v.iter().sum()),
    }
}

fn parse_gold(s: &str, scalar: Scalar) -> Result<Gold, SpatialLowerError> {
    let s = s.trim();
    if scalar.is_float() {
        s.parse::<f64>().map(Gold::Float).map_err(|_| {
            SpatialLowerError::Unsupported(
                "a gold checksum that is not a number of the program's float output precision",
            )
        })
    } else {
        s.parse::<i64>().map(Gold::Int).map_err(|_| {
            SpatialLowerError::Unsupported(
                "a gold checksum that is not an integer of the program's output precision",
            )
        })
    }
}

/// `Int` only when every value the app will hold — the fills, the output, and every partial sum of every
/// matmul — provably fits 32 bits; `Long` otherwise.
fn scalar_type(p: &SpatialProgram, ix: &Index, inputs: &Inputs, out: &Value) -> Scalar {
    let any_float = p.tensors.iter().any(|t| is_float_precision(t.dtype))
        || p.buffers.iter().any(|b| is_float_precision(b.dtype))
        || matches!(out, Value::Float(_));
    if any_float {
        return Scalar::Float;
    }
    let limit = i32::MAX as i64;
    let mut bound: i64 = 0;
    for v in inputs.tensors.values() {
        if let Value::Int(xs) = v {
            bound = bound.max(xs.iter().map(|x| x.abs()).max().unwrap_or(0));
        }
    }
    if let Value::Int(xs) = out {
        bound = bound.max(xs.iter().map(|x| x.abs()).max().unwrap_or(0));
    }
    for s in &p.stages {
        if let Shape::Matmul { .. } = shape(s) {
            let operand_bound = |buf: BufferId| -> Option<i64> {
                ix.producer
                    .get(&buf)
                    .map(|id| p.stage(*id))
                    .and_then(|st| match &st.kind {
                        StageKind::Load { tensor, .. } => p.tensor(*tensor).abs_bound(),
                        _ => None,
                    })
            };
            let (a, b) = (
                s.ins.first().and_then(|b| operand_bound(*b)),
                s.ins.get(1).and_then(|b| operand_bound(*b)),
            );
            match (a, b) {
                (Some(a), Some(b)) => {
                    let k = (p.work.k_tiles * p.tile.cols) as i64;
                    bound = bound.max(k.saturating_mul(a).saturating_mul(b));
                }
                _ => return Scalar::Long,
            }
        }
    }
    if bound <= limit {
        Scalar::Int
    } else {
        Scalar::Long
    }
}

// ── Geometry ──────────────────────────────────────────────────────────────────────────────────────

/// A buffer's memory as (rows, cols) of elements: the page its producer fills.
fn buffer_geom(ctx: &Ctx, b: &Buffer) -> (usize, usize) {
    let p = ctx.p;
    let (tr, tc) = (p.tile.rows, p.tile.cols);
    let by_matmul = |s: &Stage| match shape(s) {
        Shape::Matmul {
            block, sub_blocks, ..
        } => Some((sub_blocks.0 * block.osh * tr, sub_blocks.1 * block.osw * tc)),
        _ => None,
    };
    if let Some(s) = ctx.ix.producer.get(&b.id).map(|id| p.stage(*id)) {
        if let StageKind::Load { pattern, .. } = &s.kind {
            return (pattern.block_h * tr, pattern.block_w * tc);
        }
        if let Some(g) = by_matmul(s) {
            return g;
        }
    }
    // A matmul's spill buffer is read and written by the same stage, so it is not in `outs`.
    if let Some(s) = ctx.ix.spill.get(&b.id).map(|id| p.stage(*id)) {
        return by_matmul(s).unwrap();
    }
    (tr, b.kind.page_tiles() * tc)
}

/// A tile address with the work item's coordinates folded in: the remaining runtime step is the K block's.
struct Origin {
    row_t: usize,
    col_t: usize,
    /// Tile rows / tile columns the block advances per K block.
    k_row_step: usize,
    k_col_step: usize,
}

fn resolve_origin(
    p: &SpatialProgram,
    at: &TileAddr,
    tensor: &TensorDecl,
    item: &WorkItem,
    k_blocks: usize,
    sub: Option<(usize, usize)>,
) -> Result<Origin, SpatialLowerError> {
    let cols_t = tensor.tiles.cols_t.max(1);
    let mut base = at.base;
    let (mut k_row_step, mut k_col_step) = (0, 0);
    for (var, stride) in &at.steps {
        if *var == LV_ITEM_Y {
            base += item.coord.0 * stride;
        } else if *var == LV_ITEM_X {
            base += item.coord.1 * stride;
        } else if *var == LV_ITEM {
            // One item per site (refused otherwise): the item index is 0.
        } else if *var == LV_KBLOCK {
            if k_blocks > 1 {
                if *stride < cols_t {
                    k_col_step = *stride;
                } else if stride % cols_t == 0 {
                    k_row_step = stride / cols_t;
                } else {
                    return Err(SpatialLowerError::Unsupported("a K-block tile step that is neither a whole tile row nor within one (the block would wrap the tensor's tile grid)"));
                }
            }
        } else if *var == LV_SUB_H || *var == LV_SUB_W {
            // A store's sub-block steps are folded into one whole-block range by the caller.
            let (block_h, block_w) = sub.ok_or(SpatialLowerError::Unsupported(
                "a Load tile address stepping on a sub-block loop variable",
            ))?;
            let expected = if *var == LV_SUB_H {
                block_h * cols_t
            } else {
                block_w
            };
            if *stride != expected {
                return Err(SpatialLowerError::Unsupported("a Store whose sub-blocks do not tile a contiguous rectangle of the output block"));
            }
        } else {
            let _ = p;
            return Err(SpatialLowerError::Unsupported("a tile address stepping on a loop variable the emitter does not spell (only the work item, the sub-block and the K block)"));
        }
    }
    Ok(Origin {
        row_t: base / cols_t,
        col_t: base % cols_t,
        k_row_step,
        k_col_step,
    })
}

/// A DRAM range `r0::r1, c0::c1` in elements — with the K-block iterator `kb` folded in when the block
/// advances per K block.
fn dram_range(
    o: &Origin,
    block_h_t: usize,
    block_w_t: usize,
    tile: TileShape,
    kb: Option<&str>,
) -> String {
    let (r0, c0) = (o.row_t * tile.rows, o.col_t * tile.cols);
    let (h, w) = (block_h_t * tile.rows, block_w_t * tile.cols);
    let axis = |base: usize, len: usize, step_t: usize, elems: usize| -> String {
        match kb {
            Some(kb) if step_t > 0 => {
                let step = step_t * elems;
                format!(
                    "(({kb} * {step}) + {base})::(({kb} * {step}) + {})",
                    base + len
                )
            }
            _ => format!("{base}::{}", base + len),
        }
    };
    format!(
        "{}, {}",
        axis(r0, h, o.k_row_step, tile.rows),
        axis(c0, w, o.k_col_step, tile.cols)
    )
}

// ── The program, read for structure ───────────────────────────────────────────────────────────────

/// What the printer asks of the program per section, site, stage and buffer, each derived in one pass. A composed
/// program is printed section by section and site by site, so every question is answered here by a lookup, and the
/// print stays linear in the program.
struct Index {
    /// Each site's stages of each section, in dataflow order — a producer before every consumer on the site,
    /// declaration order breaking ties — keyed by `(site, section)`; `None` is the whole site.
    order: BTreeMap<(Site, Option<usize>), Vec<StageId>>,
    /// Each site's buffers, in id order.
    site_buffers: BTreeMap<Site, Vec<BufferId>>,
    /// The first stage whose outputs hold each buffer: its producer.
    producer: BTreeMap<BufferId, StageId>,
    /// The first matmul spilling into each buffer.
    spill: BTreeMap<BufferId, StageId>,
    /// How many K blocks the schedule loops each stage over, for a stage some op of which sits inside a K-block loop
    /// that runs more than once (the last such op read, in program then op order).
    k_blocks: BTreeMap<StageId, usize>,
    /// The section of the first Store of each tensor.
    stored_in: BTreeMap<TensorId, Option<usize>>,
    /// The sites each section runs on.
    section_sites: Vec<BTreeSet<Site>>,
}

impl Index {
    fn of(p: &SpatialProgram) -> Index {
        let (mut producer, mut spill, mut stored_in) =
            (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let mut section_sites = vec![BTreeSet::new(); p.sections.len()];
        let mut on_site: BTreeMap<Site, Vec<StageId>> = BTreeMap::new();
        for st in &p.stages {
            for b in &st.outs {
                producer.entry(*b).or_insert(st.id);
            }
            if let Shape::Matmul {
                interm: Some(i), ..
            } = shape(st)
            {
                spill.entry(i).or_insert(st.id);
            }
            if let StageKind::Store { tensor, .. } = &st.kind {
                stored_in
                    .entry(*tensor)
                    .or_insert_with(|| p.section_of_stage(st.id));
            }
            if let Some(site) = st.site {
                on_site.entry(site).or_default().push(st.id);
                if let Some(set) = p
                    .section_of_stage(st.id)
                    .and_then(|k| section_sites.get_mut(k))
                {
                    set.insert(site);
                }
            }
        }
        // The dataflow among each site's own stages.
        let mut succ: BTreeMap<StageId, Vec<StageId>> = BTreeMap::new();
        let mut indeg: BTreeMap<StageId, usize> = BTreeMap::new();
        for e in &p.edges {
            let from_site = p.stage(e.from).site;
            if from_site.is_none() {
                continue;
            }
            for to in &e.to {
                if p.stage(*to).site == from_site && *to != e.from {
                    succ.entry(e.from).or_default().push(*to);
                    *indeg.entry(*to).or_default() += 1;
                }
            }
        }
        let mut order: BTreeMap<(Site, Option<usize>), Vec<StageId>> = BTreeMap::new();
        for (site, stages) in &on_site {
            let mut ready: BTreeSet<StageId> = stages
                .iter()
                .copied()
                .filter(|s| indeg.get(s).copied().unwrap_or(0) == 0)
                .collect();
            let mut whole = Vec::with_capacity(stages.len());
            while let Some(next) = ready.pop_first() {
                whole.push(next);
                for s in succ.get(&next).into_iter().flatten() {
                    let d = indeg
                        .get_mut(s)
                        .expect("a successor on the site has an in-degree");
                    *d -= 1;
                    if *d == 0 {
                        ready.insert(*s);
                    }
                }
            }
            for id in &whole {
                if let Some(k) = p.section_of_stage(*id) {
                    order.entry((*site, Some(k))).or_default().push(*id);
                }
            }
            order.insert((*site, None), whole);
        }
        let mut site_buffers: BTreeMap<Site, Vec<BufferId>> = BTreeMap::new();
        for b in &p.buffers {
            if let Some(site) = b.site {
                site_buffers.entry(site).or_default().push(b.id);
            }
        }
        let mut k_blocks = BTreeMap::new();
        fn walk(ops: &[TaggedOp], under: Option<usize>, found: &mut BTreeMap<StageId, usize>) {
            for t in ops {
                if let ChoreoOp::Loop { var, count, body } = &t.op {
                    let inner = if *var == LV_KBLOCK {
                        Some(count.bound())
                    } else {
                        under
                    };
                    walk(body, inner, found);
                } else if let Some(n) = under.filter(|n| *n > 1) {
                    found.insert(t.stage, n);
                }
            }
        }
        for prog in p.schedule.iter().flat_map(|sch| sch.programs.iter()) {
            walk(&prog.ops, None, &mut k_blocks);
        }
        Index {
            order,
            site_buffers,
            producer,
            spill,
            k_blocks,
            stored_in,
            section_sites,
        }
    }

    fn k_blocks_of(&self, stage: StageId) -> Option<usize> {
        self.k_blocks.get(&stage).copied()
    }

    /// The section whose Store writes `t`, if any.
    fn storing_section(&self, t: TensorId) -> Option<usize> {
        self.stored_in.get(&t).copied().flatten()
    }
}

// ── Printing ──────────────────────────────────────────────────────────────────────────────────────

struct Emit {
    s: String,
    ind: usize,
}

impl Emit {
    fn l(&mut self, line: impl AsRef<str>) {
        for _ in 0..self.ind {
            self.s.push_str("  ");
        }
        self.s.push_str(line.as_ref());
        self.s.push('\n');
    }
    fn open(&mut self, line: impl AsRef<str>) {
        self.l(line);
        self.ind += 1;
    }
    fn close(&mut self, line: &str) {
        self.ind -= 1;
        self.l(line);
    }
}

struct Ctx<'a> {
    p: &'a SpatialProgram,
    scalar: Scalar,
    /// The integer scalar `I` of a program holding floats and integers both; `None` when every memory is `T`.
    int_scalar: Option<Scalar>,
    /// Buffers whose edge is a `Fifo` handshake: spelled `FIFO[T]`, read and written in stream order.
    fifo: BTreeSet<BufferId>,
    names: BTreeMap<BufferId, String>,
    flavor: SpatialFlavor,
    ix: Index,
}

impl<'a> Ctx<'a> {
    fn ty(&self, dtype: Precision) -> Ty {
        if self.int_scalar.is_some() && !is_float_precision(dtype) {
            Ty::I
        } else {
            Ty::T
        }
    }
    fn name(&self, b: &Buffer) -> String {
        self.names[&b.id].clone()
    }
    fn buf_ty(&self, b: &Buffer) -> Ty {
        self.ty(b.dtype)
    }
    fn is_float(&self, ty: Ty) -> bool {
        ty == Ty::T && self.scalar.is_float()
    }
    fn buf(&self, b: BufferId) -> &'a Buffer {
        self.p.buffer(b)
    }
    fn tensor_of_load(&self, s: &Stage) -> &'a TensorDecl {
        match &s.kind {
            StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. } => {
                self.p.tensor(*tensor)
            }
            _ => unreachable!(),
        }
    }
    fn item_of(&self, s: &Stage) -> &'a WorkItem {
        let id = s.item.expect("a placed stage carries its work item");
        &self.p.work.items[id.idx()]
    }
}

/// The iteration variables a body or a predicate may name: the Generic's (row, col) axes.
struct Axes {
    row: LoopVar,
    col: LoopVar,
}

impl Axes {
    fn name(&self, v: LoopVar) -> Result<&'static str, SpatialLowerError> {
        if v == self.row {
            Ok("r")
        } else if v == self.col {
            Ok("c")
        } else {
            Err(SpatialLowerError::Unsupported(
                "a body predicate on a loop variable outside the (rows, cols) iteration space",
            ))
        }
    }
}

/// Whether `b` is positive at every point by its structure alone — a positive constant, an exponential, and their
/// sums and products.
fn positive(b: &TensorBody) -> bool {
    match b {
        TensorBody::Const(bits) => f64::from_bits(*bits) > 0.0,
        TensorBody::Unary {
            kind: SfpuKind::Exp,
            ..
        } => true,
        TensorBody::Add(x, y) | TensorBody::Mul(x, y) => positive(x) && positive(y),
        _ => false,
    }
}

/// `(a, b)` when `body` is `e^(a·ln b)` with `b` positive by structure.
fn as_power(body: &TensorBody) -> Option<(&TensorBody, &TensorBody)> {
    let TensorBody::Unary {
        kind: SfpuKind::Exp,
        arg,
    } = body
    else {
        return None;
    };
    let TensorBody::Mul(x, y) = &**arg else {
        return None;
    };
    let (a, b) = match (&**x, &**y) {
        (
            a,
            TensorBody::Unary {
                kind: SfpuKind::Ln,
                arg: b,
            },
        )
        | (
            TensorBody::Unary {
                kind: SfpuKind::Ln,
                arg: b,
            },
            a,
        ) => (a, &**b),
        _ => return None,
    };
    positive(b).then_some((a, b))
}

/// A body, computed in `ty` (every operand already read as a `ty`).
fn body_expr(
    ctx: &Ctx,
    ty: Ty,
    body: &TensorBody,
    axes: &Axes,
    operand: &dyn Fn(usize) -> String,
) -> Result<String, SpatialLowerError> {
    // ★ `e^(a·ln b)` IS `b^a` FOR b > 0, AND IS SPELLED SO. The fabric chain's Tungsten backend prints `ln` as a bare
    // `log(…)` that its contexts shadow — the generated C++ does not compile — while `pow` reaches the library.
    // Gated DeltaNet's decay `e^(A·ln(1 + e^u))` is the case: `1 + e^u` is positive by structure.
    if let Some((a, b)) = as_power(body) {
        return Ok(format!(
            "pow({}, {})",
            body_expr(ctx, ty, b, axes, operand)?,
            body_expr(ctx, ty, a, axes, operand)?
        ));
    }
    Ok(match body {
        TensorBody::Operand(i) => operand(*i),
        TensorBody::Const(bits) => lit_of(f64::from_bits(*bits), ty, ctx.is_float(ty)),
        TensorBody::Add(x, y) => format!(
            "({} + {})",
            body_expr(ctx, ty, x, axes, operand)?,
            body_expr(ctx, ty, y, axes, operand)?
        ),
        TensorBody::Sub(x, y) => format!(
            "({} - {})",
            body_expr(ctx, ty, x, axes, operand)?,
            body_expr(ctx, ty, y, axes, operand)?
        ),
        TensorBody::Mul(x, y) => format!(
            "({} * {})",
            body_expr(ctx, ty, x, axes, operand)?,
            body_expr(ctx, ty, y, axes, operand)?
        ),
        TensorBody::Min(..) | TensorBody::Max(..) | TensorBody::FloorDiv(..) | TensorBody::Shr { .. } => {
            return Err(SpatialLowerError::Unsupported(
                "an integer model's min, max, floor division or arithmetic shift — the Spatial-language printer has no lock holding them to the VM yet",
            ))
        }
        TensorBody::Unary { kind, arg } => {
            let x = body_expr(ctx, ty, arg, axes, operand)?;
            let t = ty.name();
            match kind {
                SfpuKind::Exp => format!("exp({x})"),
                SfpuKind::Recip => format!("(1.to[{t}] / {x})"),
                SfpuKind::Rsqrt { eps_nano, over } => {
                    let eps = *eps_nano as f64 / 1e9;
                    format!(
                        "(1.to[{t}] / sqrt(({x} / {}) + {}))",
                        lit_of((*over).max(1) as f64, ty, true),
                        lit_of(eps, ty, true)
                    )
                }
                SfpuKind::Silu => format!("({x} / (1.to[{t}] + exp(0.to[{t}] - {x})))"),
                SfpuKind::Relu => format!("max({x}, 0.to[{t}])"),
                SfpuKind::Ln if ctx.flavor == SpatialFlavor::Fabric => {
                    return Err(SpatialLowerError::Unsupported("a natural log outside `e^(a·ln b)` with b positive, on the fabric flavor — Tungsten prints `ln` as a bare `log(…)` its contexts shadow"));
                }
                SfpuKind::Ln => format!("ln({x})"),
            }
        }
        // Decided by POSITION: a `mux` on the axis, both branches computed (branch-free), as the oracle does.
        TensorBody::Select {
            cond,
            then,
            otherwise,
        } => {
            let axis = axes.name(cond.var())?;
            let op = match cond {
                Pred::AxisLt(..) => "<",
                Pred::AxisGe(..) => ">=",
            };
            let test = match cond.rhs() {
                PredRhs::Const(k) => format!("{axis} {op} {k}"),
                PredRhs::Operand(i) => format!("{axis}.to[{}] {op} {}", ty.name(), operand(i)),
            };
            format!(
                "mux({test}, {}, {})",
                body_expr(ctx, ty, then, axes, operand)?,
                body_expr(ctx, ty, otherwise, axes, operand)?
            )
        }
    })
}

/// A map's strides along the Generic's `(row, col)` axes; a step on any other loop variable is refused.
fn map_strides(m: &ElemAccess, axes: &Axes) -> Result<(i64, i64), SpatialLowerError> {
    let (mut sr, mut sc) = (0i64, 0i64);
    for (v, st) in &m.terms {
        if *v == axes.row {
            sr += st;
        } else if *v == axes.col {
            sc += st;
        } else if *st != 0 {
            return Err(SpatialLowerError::Unsupported("an access map stepping on a loop variable outside the (rows, cols) iteration space"));
        }
    }
    Ok((sr, sc))
}

/// The affine `(row, col)` of a map in a page `width` elements wide and `plane_rows` high — the plane the oracle
/// reads: `off = base + r·sr + c·sc`, `(off / width, off % width)`, with the division folded away. `Some` exactly
/// when every offset of the space is on the page and its column part never carries into the next page row;
/// [`flat_read`] reads every other map through its offset.
fn affine_rc(
    base: i64,
    sr: i64,
    sc: i64,
    width: usize,
    plane_rows: usize,
    r_ext: usize,
    c_ext: usize,
) -> Option<(String, String)> {
    if base < 0 || sr < 0 || sc < 0 {
        return None;
    }
    let w = width.max(1) as i64;
    let (rm, cm) = ((r_ext as i64 - 1).max(0), (c_ext as i64 - 1).max(0));
    if base % w + rm * (sr % w) + cm * (sc % w) >= w
        || base / w + rm * (sr / w) + cm * (sc / w) >= plane_rows as i64
    {
        return None;
    }
    let term = |coef: i64, var: &str| -> Option<String> {
        match coef {
            0 => None,
            1 => Some(var.to_string()),
            k => Some(format!("{var} * {k}")),
        }
    };
    let join = |konst: i64, a: Option<String>, b: Option<String>| -> String {
        let mut parts: Vec<String> = Vec::new();
        if konst != 0 {
            parts.push(konst.to_string());
        }
        parts.extend(a);
        parts.extend(b);
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.remove(0),
            _ => format!("({})", parts.join(" + ")),
        }
    };
    Some((
        join(base / w, term(sr / w, "r"), term(sc / w, "c")),
        join(base % w, term(sr % w, "r"), term(sc % w, "c")),
    ))
}

/// Operand `i`'s read at the point `(r, c)`, in its own memory's type, and the `val`s it needs first.
///
/// A map the affine split covers reads `buf(row, col)`. Any other — a signed window that starts before the page
/// (rope's partner, the conv ring's shift), a flat offset that carries into the next page row, a GATHER whose
/// `IndirectTerm` adds `index · stride` — reads through its flat offset `o`, split at runtime. An offset the space
/// cannot drive off the page reads directly; one it can (or one that carries data) is clamped onto the page for the
/// address and the read muxed to 0 off it — the oracle's reading of an offset outside the plane, exactly.
fn flat_read(
    ctx: &Ctx,
    b: &Buffer,
    m: &ElemAccess,
    axes: &Axes,
    r_ext: usize,
    c_ext: usize,
    gather: Option<(&str, i64)>,
    tag: usize,
) -> Result<(Vec<String>, String), SpatialLowerError> {
    let ty = ctx.buf_ty(b);
    let (rows, width) = plane_of(ctx, b)?;
    let (sr, sc) = map_strides(m, axes)?;
    let name = ctx.name(b);
    if gather.is_none() {
        if let Some((r, c)) = affine_rc(m.base, sr, sc, width, rows, r_ext, c_ext) {
            return Ok((Vec::new(), format!("{name}({r}, {c})")));
        }
    }
    let n = (rows * width) as i64;
    let (rm, cm) = ((r_ext as i64 - 1).max(0), (c_ext as i64 - 1).max(0));
    let lo = m.base + (rm * sr).min(0) + (cm * sc).min(0);
    let hi = m.base + (rm * sr).max(0) + (cm * sc).max(0);
    let mut parts: Vec<String> = Vec::new();
    if m.base != 0 {
        parts.push(m.base.to_string());
    }
    for (coef, var) in [(sr, "r"), (sc, "c")] {
        match coef {
            0 => {}
            1 => parts.push(var.to_string()),
            k => parts.push(format!("{var} * {k}")),
        }
    }
    if let Some((index, stride)) = gather {
        parts.push(if stride == 1 {
            index.to_string()
        } else {
            format!("{index} * {stride}")
        });
    }
    let off = if parts.is_empty() {
        "0.to[Int]".to_string()
    } else {
        parts.join(" + ")
    };
    let o = format!("o{tag}");
    let mut pre = vec![format!("val {o}: Int = {off}")];
    if gather.is_none() && lo >= 0 && hi < n {
        return Ok((pre, format!("{name}({o} / {width}, {o} % {width})")));
    }
    let a = format!("a{tag}");
    pre.push(format!(
        "val {a} = mux({o} >= 0, mux({o} < {n}, {o}, 0.to[Int]), 0.to[Int])"
    ));
    let zero = lit_of(0.0, ty, ctx.is_float(ty));
    Ok((
        pre,
        format!(
            "mux({o} >= 0, mux({o} < {n}, {name}({a} / {width}, {a} % {width}), {zero}), {zero})"
        ),
    ))
}

/// Every operand's read at the point `(r, c)` — each in its own memory's type — and the `val`s they need first.
/// A gather's index is its index operand read through that operand's AFFINE map (the oracle's `offset_at`), as
/// an `Int`; a stride-0 gather is a bounds check that moves nothing, and reads its affine offset.
fn operand_reads(
    ctx: &Ctx,
    s: &Stage,
    maps: &[ElemAccess],
    axes: &Axes,
    r_ext: usize,
    c_ext: usize,
    folds: bool,
) -> Result<(Vec<String>, Vec<String>), SpatialLowerError> {
    let mut prelude: Vec<String> = Vec::new();
    let mut indices: BTreeMap<usize, String> = BTreeMap::new();
    for m in maps {
        if let Some(ind) = m.indirect.filter(|ind| ind.stride != 0) {
            if ind.operand >= maps.len() {
                return Err(SpatialLowerError::Unsupported(
                    "a gather whose index names an absent operand",
                ));
            }
            indices
                .entry(ind.operand)
                .or_insert_with(|| format!("ix{}", ind.operand));
        }
    }
    for (&k, index) in &indices {
        let b = ctx.buf(s.ins[k]);
        if ctx.fifo.contains(&b.id) {
            return Err(SpatialLowerError::Unsupported("a gather whose index is behind a Fifo handshake — the index would be dequeued once for the address and again as an operand"));
        }
        let affine = ElemAccess {
            indirect: None,
            ..maps[k].clone()
        };
        let (pre, read) = flat_read(ctx, b, &affine, axes, r_ext, c_ext, None, maps.len() + k)?;
        prelude.extend(pre);
        prelude.push(format!("val {index} = {read}.to[Int]"));
    }
    let mut reads: Vec<String> = Vec::new();
    for (i, m) in maps.iter().enumerate() {
        let b = ctx.buf(s.ins[i]);
        if ctx.fifo.contains(&b.id) {
            let (rows, width) = plane_of(ctx, b)?;
            let (sr, sc) = map_strides(m, axes)?;
            let identity = !folds
                && m.indirect.is_none()
                && affine_rc(m.base, sr, sc, width, rows, r_ext, c_ext)
                    == Some(("r".to_string(), "c".to_string()));
            if !identity {
                return Err(SpatialLowerError::Unsupported("a Fifo-handshake buffer read out of stream order (a broadcast, window, gather or reduce map) — Spatial's FIFO dequeues in order; place the program on a target with ValidReady handshakes"));
            }
            reads.push(format!("{}.deq()", ctx.name(b)));
            continue;
        }
        let gather = m
            .indirect
            .filter(|ind| ind.stride != 0)
            .map(|ind| (indices[&ind.operand].as_str(), ind.stride));
        let (pre, read) = flat_read(ctx, b, m, axes, r_ext, c_ext, gather, i)?;
        prelude.extend(pre);
        reads.push(read);
    }
    Ok((prelude, reads))
}

/// The plane a Generic operand's page is read as: `(rows, width)` in elements — one tile high, `page_tiles`
/// tiles wide, which is the SRAM a one-row Load or a Generic fills.
fn plane_of(ctx: &Ctx, b: &Buffer) -> Result<(usize, usize), SpatialLowerError> {
    let (rows, cols) = buffer_geom(ctx, b);
    if rows != ctx.p.tile.rows {
        return Err(SpatialLowerError::Unsupported("a Generic operand or output whose page is a multi-row tile block — the plane a flat map addresses is one tile row of pages"));
    }
    Ok((rows, cols))
}

fn emit_generic(
    ctx: &Ctx,
    e: &mut Emit,
    s: &Stage,
    iters: &[IterSpec],
    maps: &[ElemAccess],
    out: &ElemAccess,
    body: &TensorBody,
) -> Result<(), SpatialLowerError> {
    let (row_ax, col_ax) = (iters[0], iters[1]);
    let axes = Axes {
        row: row_ax.var,
        col: col_ax.var,
    };
    let out_buf = ctx.buf(*s.outs.first().ok_or(SpatialLowerError::Unsupported(
        "a Generic compute stage without an output buffer",
    ))?);
    let pool_of = |k: IterKind| match k {
        IterKind::Reduce { pool } => Some(pool),
        IterKind::Parallel => None,
    };
    let (row_pool, col_pool) = (pool_of(row_ax.kind), pool_of(col_ax.kind));
    let fold_of = |pool: ReducePool| -> Result<&'static str, SpatialLowerError> {
        match pool {
            ReducePool::Sum => Ok("{ _ + _ }"),
            ReducePool::Max => Ok("{ (a, b) => max(a, b) }"),
            ReducePool::ArgMax => Err(SpatialLowerError::Unsupported("an ArgMax reduce pool over the row axis, or over both — the index scan is spelled along the columns")),
        }
    };
    let (r_ext, c_ext) = (row_ax.extent, col_ax.extent);
    let out_ty = ctx.buf_ty(out_buf);
    let in_ty: Vec<Ty> = s.ins.iter().map(|b| ctx.buf_ty(ctx.buf(*b))).collect();
    // The body computes in its output's type — except an index scan's, which compares the VALUES it reads.
    let index_scan = col_pool == Some(ReducePool::ArgMax) && row_pool.is_none();
    let body_ty = if index_scan {
        in_ty
            .iter()
            .copied()
            .find(|t| ctx.is_float(*t))
            .or(in_ty.first().copied())
            .unwrap_or(out_ty)
    } else {
        out_ty
    };
    let (prelude, reads) = operand_reads(
        ctx,
        s,
        maps,
        &axes,
        r_ext,
        c_ext,
        row_pool.is_some() || col_pool.is_some(),
    )?;
    let operand = |i: usize| -> String { cast(reads[i].clone(), in_ty[i], body_ty) };
    let expr = body_expr(ctx, body_ty, body, &axes, &operand)?;
    let (out_rows, out_width) = plane_of(ctx, out_buf)?;
    let out_name = ctx.name(out_buf);
    let out_fifo = ctx.fifo.contains(&out_buf.id);
    if out_fifo && (row_pool.is_some() || col_pool.is_some()) {
        return Err(SpatialLowerError::Unsupported("a Fifo-handshake buffer written by a reduced Generic (one scalar broadcast over a tile) — Spatial's FIFO enqueues one value per point"));
    }
    for it in [row_ax, col_ax] {
        if matches!(it.kind, IterKind::Reduce { .. }) && out.varies_along(it.var) {
            return Err(SpatialLowerError::Unsupported(
                "a Generic compute whose output map varies along a reduced axis",
            ));
        }
    }
    let (bt, ot) = (body_ty.name(), out_ty.name());
    // `val fold` over `space` (bound to `binders`, whose axes `axes_folded` are the ones folded): a `Reduce` for the
    // fabric, or a register scan seeded at the first point for the simulator.
    let fold_line = |e: &mut Emit,
                     space: String,
                     binders: &str,
                     axes_folded: &[&str],
                     pool: ReducePool|
     -> Result<(), SpatialLowerError> {
        match ctx.flavor {
            SpatialFlavor::Fabric => {
                let head = format!("val fold = Reduce(Reg[{bt}])({space}) {{ {binders} =>");
                let tail = fold_of(pool)?;
                if prelude.is_empty() {
                    e.l(format!("{head} {expr} }} {tail}"));
                } else {
                    e.open(head);
                    for l in &prelude {
                        e.l(l);
                    }
                    e.l(&expr);
                    e.close(&format!("}} {tail}"));
                }
            }
            SpatialFlavor::Simulator => {
                let combined = match pool {
                    ReducePool::Sum => "fold.value + v",
                    ReducePool::Max => "max(fold.value, v)",
                    ReducePool::ArgMax => return Err(SpatialLowerError::Unsupported("an ArgMax reduce on a Generic's row axis in the simulator flavor — the index scan is spelled along the columns")),
                };
                let seeded = axes_folded.iter().rev().fold("v".to_string(), |inner, ax| {
                    format!("mux({ax} < 1, {inner}, {combined})")
                });
                e.l(format!("val fold = Reg[{bt}]"));
                e.open(format!("Sequential.Foreach({space}) {{ {binders} =>"));
                for l in &prelude {
                    e.l(l);
                }
                e.l(format!("val v = {expr}"));
                e.l(format!("fold := {seeded}"));
                e.close("}");
            }
        }
        Ok(())
    };
    // ★ THE ORACLE'S PAGE STARTS AT ZERO. A stage writes the points of its space, and a space narrower than the page
    // (a 1-row space over a 32-row tile, a row of 8 over 32 columns) leaves the rest as the oracle leaves it: 0. An
    // SRAM starts undefined, and a later section that folds over a whole tile — a matmul's K — reads that rest.
    let covered = match (row_pool, col_pool) {
        (None, None) => r_ext >= out_rows && c_ext >= out_width,
        (None, Some(_)) => r_ext >= out_rows,
        (Some(_), None) => c_ext >= out_width,
        (Some(_), Some(_)) => true,
    };
    if !covered && !out_fifo {
        e.l(format!("// zero page: {} writes {r_ext}x{c_ext} of its {out_rows}x{out_width} page, and the rest is 0 as the oracle's page starts", s.name));
        e.l(format!("Foreach(0 until {out_rows}, 0 until {out_width}) {{ (r, c) => {out_name}(r, c) = 0.to[{ot}] }}"));
    }
    match (row_pool, col_pool) {
        // Every point of the space is written at (r, c) — the oracle writes a parallel space that way whatever
        // the output map says, so a per-row value lands across its whole row and a per-row reader finds it at
        // column 0.
        (None, None) => {
            if c_ext > out_width || r_ext > out_rows {
                return Err(SpatialLowerError::Unsupported("a Generic compute whose (rows, cols) space is larger than its output page plane"));
            }
            e.l(format!(
                "// stage {} (Generic: {r_ext} rows x {c_ext} cols, body {body:?})",
                s.name
            ));
            e.open(format!(
                "Foreach(0 until {r_ext}, 0 until {c_ext}) {{ (r, c) =>"
            ));
            for l in &prelude {
                e.l(l);
            }
            if out_fifo {
                e.l(format!(
                    "{out_name}.enq({})",
                    cast(expr.clone(), body_ty, out_ty)
                ));
            } else {
                e.l(format!(
                    "{out_name}(r, c) = {}",
                    cast(expr.clone(), body_ty, out_ty)
                ));
            }
            e.close("}");
        }
        // The first maximum of each row, written ACROSS the output tile row: the VM's `>` scan, exactly as the
        // `ArgMax` fold is — column 0 seeds the running best, and a later column replaces it only when strictly
        // greater. A scan carries its best from column to column, so it runs `Sequential`.
        // ★ ON THE FABRIC THE SAME SCAN IS TWO PLAIN FOLDS. PIR maps a register only as an accumulation (`acc = f(acc,
        // x)`), and refuses the scan's pair of mux-updated registers. With `v0` the seed and `w_c = v_c`, or `v0` where
        // `v_c` is NaN: `m = max w`, `first = min { c : w_c = m }`, and the answer is `first` — or 0 when `v0` is NaN.
        // A NaN replaced by `v0` can create no new maximum and ties go to the lowest index, so this IS the scan.
        (None, Some(ReducePool::ArgMax)) if ctx.flavor == SpatialFlavor::Fabric => {
            e.l(format!("// stage {} (Generic: {r_ext} rows, the first maximum of {c_ext} cols — the scan as a max fold and a min-index fold; body {body:?})", s.name));
            e.open(format!("Foreach(0 until {r_ext}) {{ r =>"));
            e.open("val v0 = {");
            e.l("val c = 0.to[I32]");
            for l in &prelude {
                e.l(l);
            }
            e.l(&expr);
            e.close("}");
            e.open(format!(
                "val m = Reduce(Reg[{bt}])(0 until {c_ext}) {{ c =>"
            ));
            for l in &prelude {
                e.l(l);
            }
            e.l(format!("val v = {expr}"));
            e.l("mux(v === v, v, v0)");
            e.close("} { (a, b) => max(a, b) }");
            e.open(format!(
                "val first = Reduce(Reg[I32])(0 until {c_ext}) {{ c =>"
            ));
            for l in &prelude {
                e.l(l);
            }
            e.l(format!("val v = {expr}"));
            e.l(format!(
                "mux(mux(v === v, v, v0) === m.value, c, {c_ext}.to[I32])"
            ));
            e.close("} { (a, b) => min(a, b) }");
            e.open(format!("Foreach(0 until {out_width}) {{ cc =>"));
            e.l(format!(
                "{out_name}(r, cc) = mux(v0 === v0, first.value, 0.to[I32]).to[{ot}]"
            ));
            e.close("}");
            e.close("}");
        }
        (None, Some(ReducePool::ArgMax)) => {
            e.l(format!("// stage {} (Generic: {r_ext} rows, the first maximum of {c_ext} cols — seeded by col 0, replaced only on `>`; body {body:?})", s.name));
            e.open(format!("Foreach(0 until {r_ext}) {{ r =>"));
            e.l(format!("val best = Reg[{bt}]"));
            e.l(format!("val at = Reg[{ot}]"));
            e.open(format!("Sequential.Foreach(0 until {c_ext}) {{ c =>"));
            for l in &prelude {
                e.l(l);
            }
            e.l(format!("val v = {expr}"));
            e.l("val ahead = v > best.value");
            e.l("best := mux(c < 1, v, mux(ahead, v, best.value))");
            e.l(format!(
                "at := mux(c < 1, c.to[{ot}], mux(ahead, c.to[{ot}], at.value))"
            ));
            e.close("}");
            e.open(format!("Foreach(0 until {out_width}) {{ cc =>"));
            e.l(format!("{out_name}(r, cc) = at.value"));
            e.close("}");
            e.close("}");
        }
        // One scalar per row, written ACROSS the output tile row (a per-row broadcast reads column 0).
        (None, Some(pool)) => {
            e.l(format!(
                "// stage {} (Generic: {r_ext} rows, {c_ext} cols reduced {pool:?}, body {body:?})",
                s.name
            ));
            e.open(format!("Foreach(0 until {r_ext}) {{ r =>"));
            fold_line(e, format!("0 until {c_ext}"), "c", &["c"], pool)?;
            e.open(format!("Foreach(0 until {out_width}) {{ cc =>"));
            e.l(format!(
                "{out_name}(r, cc) = {}",
                cast("fold".into(), body_ty, out_ty)
            ));
            e.close("}");
            e.close("}");
        }
        // One scalar per column, written DOWN the output tile (a per-column broadcast reads row 0).
        (Some(pool), None) => {
            e.l(format!(
                "// stage {} (Generic: {c_ext} cols, {r_ext} rows reduced {pool:?}, body {body:?})",
                s.name
            ));
            e.open(format!("Foreach(0 until {c_ext}) {{ c =>"));
            fold_line(e, format!("0 until {r_ext}"), "r", &["r"], pool)?;
            e.open(format!("Foreach(0 until {out_rows}) {{ rr =>"));
            e.l(format!(
                "{out_name}(rr, c) = {}",
                cast("fold".into(), body_ty, out_ty)
            ));
            e.close("}");
            e.close("}");
        }
        // One scalar, on the whole output tile.
        (Some(rp), Some(cp)) => {
            if rp != cp {
                return Err(SpatialLowerError::Unsupported(
                    "a Generic compute folding its two axes with two different reduce pools",
                ));
            }
            fold_of(rp)?;
            e.l(format!("// stage {} (Generic: {r_ext} rows x {c_ext} cols both reduced {rp:?}, body {body:?})", s.name));
            fold_line(
                e,
                format!("0 until {r_ext}, 0 until {c_ext}"),
                "(r, c)",
                &["r", "c"],
                rp,
            )?;
            e.open(format!(
                "Foreach(0 until {out_rows}, 0 until {out_width}) {{ (rr, cc) =>"
            ));
            e.l(format!(
                "{out_name}(rr, cc) = {}",
                cast("fold".into(), body_ty, out_ty)
            ));
            e.close("}");
        }
    }
    Ok(())
}

fn emit_load(
    ctx: &Ctx,
    e: &mut Emit,
    s: &Stage,
    kb: Option<&str>,
) -> Result<(), SpatialLowerError> {
    let StageKind::Load {
        pattern,
        num_blocks,
        ..
    } = &s.kind
    else {
        unreachable!()
    };
    let t = ctx.tensor_of_load(s);
    let item = ctx.item_of(s);
    let k_blocks = if kb.is_some() { *num_blocks } else { 1 };
    let o = resolve_origin(ctx.p, &pattern.origin, t, item, k_blocks, None)?;
    if o.row_t + (k_blocks - 1) * o.k_row_step + pattern.block_h > t.tiles.rows_t
        || o.col_t + (k_blocks - 1) * o.k_col_step + pattern.block_w > t.tiles.cols_t
    {
        return Err(SpatialLowerError::Unsupported(
            "a Load whose K blocks run past the tensor's tile grid",
        ));
    }
    let buf = ctx.buf(*s.outs.first().ok_or(SpatialLowerError::Unsupported(
        "a Load stage without an output buffer",
    ))?);
    let step = if k_blocks > 1 {
        format!(
            ", stepping {} tile rows / {} tile cols per K block over {} blocks",
            o.k_row_step, o.k_col_step, k_blocks
        )
    } else {
        String::new()
    };
    e.l(format!(
        "// stage {} (Load {}: {}x{} tiles at tile ({},{}){step})",
        s.name, t.name, pattern.block_h, pattern.block_w, o.row_t, o.col_t
    ));
    if ctx.fifo.contains(&buf.id) {
        // A FIFO fills in stream order: one DRAM row of the block per enqueue burst.
        let (r0, c0) = (o.row_t * ctx.p.tile.rows, o.col_t * ctx.p.tile.cols);
        let (h, w) = (
            pattern.block_h * ctx.p.tile.rows,
            pattern.block_w * ctx.p.tile.cols,
        );
        e.open(format!("Foreach(0 until {h}) {{ row =>"));
        e.l(format!(
            "{} load {}({r0} + row, {c0}::{})",
            ctx.name(buf),
            dram_name(t),
            c0 + w
        ));
        e.close("}");
    } else {
        e.l(format!(
            "{} load {}({})",
            ctx.name(buf),
            dram_name(t),
            dram_range(&o, pattern.block_h, pattern.block_w, ctx.p.tile, kb)
        ));
    }
    Ok(())
}

fn emit_store(ctx: &Ctx, e: &mut Emit, s: &Stage) -> Result<(), SpatialLowerError> {
    let StageKind::Store {
        pattern,
        sub_blocks,
        ..
    } = &s.kind
    else {
        unreachable!()
    };
    let t = ctx.tensor_of_load(s);
    let item = ctx.item_of(s);
    let o = resolve_origin(
        ctx.p,
        &pattern.origin,
        t,
        item,
        1,
        Some((pattern.block_h, pattern.block_w)),
    )?;
    let (h_t, w_t) = (
        sub_blocks.0 * pattern.block_h,
        sub_blocks.1 * pattern.block_w,
    );
    if o.row_t + h_t > t.tiles.rows_t || o.col_t + w_t > t.tiles.cols_t {
        return Err(SpatialLowerError::Unsupported(
            "a Store whose block runs past the tensor's tile grid",
        ));
    }
    let buf = ctx.buf(*s.ins.first().ok_or(SpatialLowerError::Unsupported(
        "a Store stage without an input buffer",
    ))?);
    let (rows, cols) = buffer_geom(ctx, buf);
    if (rows, cols) != (h_t * ctx.p.tile.rows, w_t * ctx.p.tile.cols) {
        return Err(SpatialLowerError::Unsupported(
            "a Store whose block does not match the buffer it drains (the compute's output block)",
        ));
    }
    e.l(format!(
        "// stage {} (Store {}: {}x{} sub-blocks of {}x{} tiles at tile ({},{}))",
        s.name,
        t.name,
        sub_blocks.0,
        sub_blocks.1,
        pattern.block_h,
        pattern.block_w,
        o.row_t,
        o.col_t
    ));
    if ctx.fifo.contains(&buf.id) {
        let (r0, c0) = (o.row_t * ctx.p.tile.rows, o.col_t * ctx.p.tile.cols);
        e.open(format!("Foreach(0 until {rows}) {{ row =>"));
        e.l(format!(
            "{}({r0} + row, {c0}::{}) store {}",
            dram_name(t),
            c0 + cols,
            ctx.name(buf)
        ));
        e.close("}");
    } else {
        e.l(format!(
            "{}({}) store {}",
            dram_name(t),
            dram_range(&o, h_t, w_t, ctx.p.tile, None),
            ctx.name(buf)
        ));
    }
    Ok(())
}

/// The dot-product nest of one `MatmulTiles` stage, writing into `dst` (its output buffer, or its spill
/// buffer inside a `MemReduce`).
fn emit_matmul_nest(
    ctx: &Ctx,
    e: &mut Emit,
    s: &Stage,
    dst: &Buffer,
) -> Result<(), SpatialLowerError> {
    let Shape::Matmul { block, .. } = shape(s) else {
        unreachable!()
    };
    if s.ins.len() != 2 {
        return Err(SpatialLowerError::Unsupported(
            "a MatmulTiles stage without exactly two operand buffers (the A block and the B block)",
        ));
    }
    let (a, b) = (ctx.buf(s.ins[0]), ctx.buf(s.ins[1]));
    if ctx.fifo.contains(&a.id) || ctx.fifo.contains(&b.id) {
        return Err(SpatialLowerError::Unsupported("a MatmulTiles operand behind a Fifo handshake — the dot product needs random access to the block"));
    }
    let (rows, cols) = buffer_geom(ctx, dst);
    let kb = block.ibw * ctx.p.tile.cols;
    if buffer_geom(ctx, a) != (rows, kb) || buffer_geom(ctx, b) != (kb, cols) {
        return Err(SpatialLowerError::Unsupported("MatmulTiles operand blocks that do not match the output block (A must be rows x K block, B must be K block x cols)"));
    }
    let d = ctx.buf_ty(dst);
    let (ra, rb) = (
        cast(format!("{}(i, k)", ctx.name(a)), ctx.buf_ty(a), d),
        cast(format!("{}(k, j)", ctx.name(b)), ctx.buf_ty(b), d),
    );
    e.open(format!("Foreach(0 until {rows}) {{ i =>"));
    e.open(format!("Foreach(0 until {cols}) {{ j =>"));
    match ctx.flavor {
        SpatialFlavor::Fabric => e.l(format!(
            "val dot = Reduce(Reg[{}])(0 until {kb} par {}) {{ k => {ra} * {rb} }} {{ _ + _ }}",
            d.name(),
            lanes(kb)
        )),
        SpatialFlavor::Simulator => {
            e.l(format!("val dot = Reg[{}]", d.name()));
            e.l(format!("Sequential.Foreach(0 until {kb}) {{ k => dot := mux(k < 1, {ra} * {rb}, dot.value + {ra} * {rb}) }}"));
        }
    }
    e.l(format!("Pipe {{ {}(i, j) = dot }}", ctx.name(dst)));
    e.close("}");
    e.close("}");
    Ok(())
}

fn emit_matmul(
    ctx: &Ctx,
    e: &mut Emit,
    s: &Stage,
    k_loads: &[&Stage],
) -> Result<(), SpatialLowerError> {
    let Shape::Matmul {
        block,
        sub_blocks,
        num_blocks,
        operand,
        acc,
        interm,
        ..
    } = shape(s)
    else {
        unreachable!()
    };
    let out = ctx.buf(*s.outs.first().ok_or(SpatialLowerError::Unsupported(
        "a MatmulTiles stage without an output buffer",
    ))?);
    let k_blocks = ctx.ix.k_blocks_of(s.id).unwrap_or(1);
    if k_blocks != num_blocks && k_blocks > 1 {
        return Err(SpatialLowerError::Unsupported("a MatmulTiles stage whose schedule loops over a different number of K blocks than its plan"));
    }
    e.l(format!(
        "// stage {} (MatmulTiles: {}x{} sub-blocks of {}x{} tiles, K block {} tiles, {:?} operands accumulated {:?}, output-stationary, {} K block(s))",
        s.name, sub_blocks.0, sub_blocks.1, block.osh, block.osw, block.ibw, operand, acc, num_blocks
    ));
    if k_blocks > 1 {
        if ctx.flavor == SpatialFlavor::Simulator {
            return Err(SpatialLowerError::Unsupported("a MatmulTiles reduction over several K blocks (a MemReduce) in the simulator flavor — its K-block fold is a Reduce the simulator cannot split"));
        }
        let spill = ctx.buf(interm.expect("refused above when absent"));
        e.open(format!(
            "MemReduce({})(0 until {k_blocks}) {{ kb =>",
            ctx.name(out)
        ));
        for l in k_loads {
            emit_load(ctx, e, l, Some("kb"))?;
        }
        emit_matmul_nest(ctx, e, s, spill)?;
        e.l(ctx.name(spill));
        e.close("} { _ + _ }");
    } else {
        emit_matmul_nest(ctx, e, s, out)?;
    }
    Ok(())
}

/// One site's memories and controllers — of section `section` alone in a composed program, whose sections run one
/// after another.
fn emit_site(
    ctx: &Ctx,
    e: &mut Emit,
    site: Site,
    section: Option<usize>,
) -> Result<(), SpatialLowerError> {
    let p = ctx.p;
    let pl = p.placement.as_ref().unwrap();
    let order: &[StageId] = ctx
        .ix
        .order
        .get(&(site, section))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let item = match section {
        None => &p.work.items[pl.items_at(site)[0].idx()],
        Some(_) => ctx.item_of(p.stage(*order.first().ok_or(SpatialLowerError::Unsupported(
            "a section with no stage on a site it is placed on",
        ))?)),
    };
    e.l(format!(
        "// site ({},{}): work item {} -> output block tiles ({},{}) {}x{}",
        site.x,
        site.y,
        item.id.idx(),
        item.out.row0_t,
        item.out.col0_t,
        item.out.h_t,
        item.out.w_t
    ));
    let used: BTreeSet<BufferId> = order
        .iter()
        .flat_map(|id| {
            let st = p.stage(*id);
            let interm = match shape(st) {
                Shape::Matmul { interm, .. } => interm,
                _ => None,
            };
            st.ins
                .iter()
                .chain(st.outs.iter())
                .copied()
                .chain(interm)
                .collect::<Vec<_>>()
        })
        .collect();
    let buffers: Vec<&Buffer> = match section {
        None => ctx
            .ix
            .site_buffers
            .get(&site)
            .into_iter()
            .flatten()
            .map(|b| p.buffer(*b))
            .collect(),
        Some(_) => used
            .iter()
            .map(|b| p.buffer(*b))
            .filter(|b| b.site == Some(site))
            .collect(),
    };
    for b in buffers {
        let (rows, cols) = buffer_geom(ctx, b);
        let tier = format!("{:?}", b.space);
        let depth = match b.kind {
            BufferKind::Fifo {
                depth_pages,
                page_tiles,
            } => format!("Fifo depth {depth_pages} pages x {page_tiles} tiles"),
            BufferKind::Interm {
                depth_pages,
                page_tiles,
            } => format!("Interm depth {depth_pages} pages x {page_tiles} tiles"),
            BufferKind::Accumulator { tiles } => format!("Accumulator {tiles} tiles"),
        };
        if ctx.fifo.contains(&b.id) {
            let cap = b.kind.tiles() * p.tile.rows * p.tile.cols;
            e.l(format!(
                "// buffer {} {tier} {depth} ({:?}) — Fifo handshake: a {cap}-element FIFO",
                b.name, b.dtype
            ));
            e.l(format!(
                "val {} = FIFO[{}]({cap})",
                ctx.name(b),
                ctx.buf_ty(b).name()
            ));
        } else {
            e.l(format!("// buffer {} {tier} {depth} ({:?}) — one page is {rows}x{cols}; Spatial N-buffers it across the controllers", b.name, b.dtype));
            e.l(format!(
                "val {} = SRAM[{}]({rows}, {cols})",
                ctx.name(b),
                ctx.buf_ty(b).name()
            ));
        }
    }
    // Loads the schedule runs once per K block belong inside the matmul's MemReduce.
    let k_loads: Vec<&Stage> = order
        .iter()
        .map(|id| p.stage(*id))
        .filter(|s| matches!(s.kind, StageKind::Load { .. }) && ctx.ix.k_blocks_of(s.id).is_some())
        .collect();
    for id in order {
        let s = p.stage(*id);
        match shape(s) {
            Shape::Load => {
                if ctx.ix.k_blocks_of(s.id).is_none() {
                    emit_load(ctx, e, s, None)?;
                }
            }
            Shape::Store => emit_store(ctx, e, s)?,
            Shape::Matmul { .. } => emit_matmul(ctx, e, s, &k_loads)?,
            Shape::Generic {
                iters,
                ins,
                out,
                body,
            } => {
                if ctx.ix.k_blocks_of(s.id).is_some() {
                    return Err(SpatialLowerError::Unsupported("a Generic compute stage inside the K-block loop — only the matmul's operand loads run per K block"));
                }
                emit_generic(ctx, e, s, iters, ins, out, body)?;
            }
            Shape::Recv | Shape::Epilogue | Shape::Scalar | Shape::AllReduce => {
                unreachable!("refused by refuse_unspelled")
            }
        }
    }
    Ok(())
}

/// The host's element checks: each slice read back from its DRAM and compared with its gold — exactly for an integer
/// tensor, within its tolerance for a float one — every result printed, and `cksum` their conjunction.
fn emit_element_checks(
    e: &mut Emit,
    ctx: &Ctx,
    checks: &[SpatialCheck],
) -> Result<(), SpatialLowerError> {
    let mut oks: Vec<String> = Vec::new();
    for (k, c) in checks.iter().enumerate() {
        let t = ctx.p.tensors.iter().find(|t| t.name == c.tensor).ok_or(
            SpatialLowerError::Unsupported("a check naming a tensor the program does not have"),
        )?;
        let n = c.gold.len();
        if n == 0 || c.offset + n > t.elems.rows * t.elems.cols {
            return Err(SpatialLowerError::Unsupported(
                "a check whose slice is empty or runs past its tensor",
            ));
        }
        let (ty, float) = (ctx.ty(t.dtype), ctx.is_float(ctx.ty(t.dtype)));
        let (name, cols, off) = (dram_name(t), t.elems.cols, c.offset);
        e.l(format!(
            "// check {k}: {} elements {off}..{} of {} against the caller's gold",
            n,
            off + n,
            t.name
        ));
        // Each check is its own method: a gold table costs JVM code per literal, and `main` holds one method's worth.
        e.open(format!("def check{k}(): Bit = {{"));
        e.l(format!("val {name}_check{k} = getMatrix({name})"));
        let lits: Vec<String> = c.gold.iter().map(|v| lit_of(*v, ty, float)).collect();
        let table = format!("gold{k}");
        emit_table(e, &table, ty.name(), &lits);
        let got = format!("{name}_check{k}(({off} + q) / {cols}, ({off} + q) % {cols})");
        if float {
            let tol = c.rel_tol * c.gold.iter().fold(0.0f64, |m, v| m.max(v.abs())) + 1e-6;
            e.l(format!(
                "val err{k} = Array.tabulate({n}) {{ q => abs({got} - {table}_vals(q)) }}.max"
            ));
            e.l(format!("val ok{k} = err{k} <= {}", lit_of(tol, ty, true)));
            // The Scala backend's harness reads any output line holding "error" as a failure, so the deviation is
            // printed as a deviation.
            e.l(format!(
                "println(r\"check {k} {}: max deviation $err{k} tolerance {tol:e} ok $ok{k}\")",
                t.name
            ));
        } else {
            e.l(format!("val ok{k} = Array.tabulate({n}) {{ q => {got} == {table}_vals(q) }}.reduce {{ _ && _ }}"));
            e.l(format!(
                "println(r\"check {k} {}: exact ok $ok{k}\")",
                t.name
            ));
        }
        e.l(format!("ok{k}"));
        e.close("}");
        e.l(format!("val ok{k} = check{k}()"));
        oks.push(format!("ok{k}"));
    }
    e.l(format!("val cksum = {}", oks.join(" && ")));
    Ok(())
}

/// The host-side fill of one tensor from its recorded init.
/// The literals of a table fill, as `val <name>_vals: Array[ty]`. A JVM method holds at most 64 KB of code and every
/// staged literal costs dozens of bytes of it, so a table past [`TABLE_CHUNK`] literals is built from chunks, each
/// its own `def`, joined with `++`.
fn emit_table(e: &mut Emit, name: &str, tn: &str, lits: &[String]) {
    if lits.len() <= TABLE_CHUNK {
        e.l(format!(
            "val {name}_vals = Array[{tn}]({})",
            lits.join(", ")
        ));
        return;
    }
    let chunks: Vec<&[String]> = lits.chunks(TABLE_CHUNK).collect();
    for (k, chunk) in chunks.iter().enumerate() {
        e.l(format!(
            "def {name}_part{k}(): Array[{tn}] = Array[{tn}]({})",
            chunk.join(", ")
        ));
    }
    let parts: Vec<String> = (0..chunks.len())
        .map(|k| format!("{name}_part{k}()"))
        .collect();
    e.l(format!("val {name}_vals = {}", parts.join(" ++ ")));
}

/// Literals per table chunk: well inside a JVM method's code limit at every literal width.
const TABLE_CHUNK: usize = 256;

/// With [`HostFill::OwnMethod`], the data is built inside its own `def` — a composed program's fills are many, and one
/// JVM method holding them all is past what the Scala backend can analyze; with [`HostFill::MethodOnly`] the DRAM is
/// a registry's ([`emit_dram_registry`]) and the caller calls the `def`.
fn emit_host_fill(e: &mut Emit, ctx: &Ctx, t: &TensorDecl, mode: HostFill) {
    let (rows, cols) = (t.elems.rows, t.elems.cols);
    let (pr, pc) = padded_dims(ctx.p, t);
    let name = dram_name(t);
    let (ty, float) = (ctx.ty(t.dtype), ctx.is_float(ctx.ty(t.dtype)));
    let tn = ty.name();
    e.l(format!(
        "// tensor {} {:?} {}x{} {:?} {:?}",
        t.name, t.role, rows, cols, t.dtype, t.space
    ));
    if mode == HostFill::OwnMethod {
        e.l(format!("val {name} = DRAM[{tn}]({pr}, {pc})"));
    }
    if mode != HostFill::Inline {
        e.open(format!("def fill_{name}(): Unit = {{"));
    }
    let lit = |v: f64| lit_of(v, ty, float);
    // A tensor is tiled, and a tile past its elements holds zeros — the oracle's padding, which a tile-block load
    // reads. `inside` keeps a value on the elements and zeroes the padding; a tile-aligned tensor has none.
    let padded = (pr, pc) != (rows, cols);
    let zero = format!("0.to[{tn}]");
    let inside = |v: String| {
        if padded {
            format!("mux(i < {rows}, mux(j < {cols}, {v}, {zero}), {zero})")
        } else {
            v
        }
    };
    let index = if padded {
        format!("mux(i < {rows}, mux(j < {cols}, i * {cols} + j, 0), 0)")
    } else {
        format!("i * {cols} + j")
    };
    match &t.init {
        TensorInit::Zero => e.l(format!(
            "val {name}_data = (0::{pr}, 0::{pc}) {{ (i, j) => 0.to[{tn}] }}"
        )),
        TensorInit::ConstF(b) => e.l(format!(
            "val {name}_data = (0::{pr}, 0::{pc}) {{ (i, j) => {} }}",
            inside(format!("{}.to[{tn}]", lit(f64::from_bits(*b))))
        )),
        TensorInit::Pattern { modulus, shift } => {
            let shift = if *shift < 0 {
                format!("- {}", -shift)
            } else {
                format!("+ {shift}")
            };
            e.l(format!(
                "val {name}_data = (0::{pr}, 0::{pc}) {{ (i, j) => {} }}",
                inside(format!("(((i * {cols} + j) % {modulus}) {shift}).to[{tn}]"))
            ));
        }
        TensorInit::Values(_) | TensorInit::FloatValues(_) => {
            let vals: Vec<f64> = match &t.init {
                TensorInit::Values(v) => v.iter().take(rows * cols).map(|x| *x as f64).collect(),
                TensorInit::FloatValues(v) => v
                    .iter()
                    .take(rows * cols)
                    .map(|b| f64::from_bits(*b))
                    .collect(),
                _ => unreachable!(),
            };
            if vals.len() == rows * cols
                && vals.windows(2).all(|w| w[0].to_bits() == w[1].to_bits())
                && !vals.is_empty()
            {
                // One value everywhere (a scaler table): a formula, not a table.
                e.l(format!(
                    "val {name}_data = (0::{pr}, 0::{pc}) {{ (i, j) => {} }}",
                    inside(lit(vals[0]))
                ));
            } else {
                let lits: Vec<String> = vals.iter().map(|v| lit(*v)).collect();
                emit_table(e, &name, tn, &lits);
                e.l(format!(
                    "val {name}_data = (0::{pr}, 0::{pc}) {{ (i, j) => {} }}",
                    inside(format!("{name}_vals({index})"))
                ));
            }
        }
        TensorInit::External | TensorInit::RandomBf16 { .. } => {
            unreachable!("refused by refuse_unspelled")
        }
    }
    match mode {
        HostFill::Inline => {
            e.l(format!("val {name} = DRAM[{tn}]({pr}, {pc})"));
            e.l(format!("setMem({name}, {name}_data)"));
        }
        HostFill::OwnMethod => {
            e.l(format!("setMem({name}, {name}_data)"));
            e.close("}");
            e.l(format!("fill_{name}()"));
        }
        HostFill::MethodOnly => {
            e.l(format!("setMem({name}, {name}_data)"));
            e.close("}");
        }
    }
}

/// How the host declares and fills one tensor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HostFill {
    /// `val d = DRAM…`, its data, `setMem` — all in `main` (a one-section program).
    Inline,
    /// `val d = DRAM…` in `main`, the data and `setMem` in `def fill_d()`, called at once.
    OwnMethod,
    /// Only `def fill_d()`: the DRAM is a registry's, and the fills are called in chunks.
    MethodOnly,
}

/// ★ THE JVM BOUNDS ONE METHOD'S CODE AT 64 KB, AND THE HOST IS ONE METHOD. Every staged `DRAM[T](r, c)` carries
/// the macro-generated source context Spatial threads through it — a few hundred bytes of `main` each — so a
/// composed program's host holds [`HOST_INLINE`] tensors at most (measured: 201 compile, 398 do not). Past that, the
/// DRAMs are made in [`DRAM_CHUNK`]s by methods of their own into registries (a Scala `Vector` — `Array` is Spatial's
/// staged host array), each tensor's name an accessor into one (`def d_x: DRAM2[T] = dram_T_0_all(5)` — a method, so
/// `main` holds none of it), and the fills are called in chunks the same way.
const HOST_INLINE: usize = 128;
const DRAM_CHUNK: usize = 64;

/// The DRAM registries of a host past [`HOST_INLINE`] tensors: per element type, [`DRAM_CHUNK`] allocations to a
/// method, the method's `Seq` held once in `main`, and one accessor per tensor.
fn emit_dram_registry(e: &mut Emit, ctx: &Ctx, host: &[TensorDecl]) {
    let mut by_type: BTreeMap<&'static str, Vec<&TensorDecl>> = BTreeMap::new();
    for t in host {
        by_type.entry(ctx.ty(t.dtype).name()).or_default().push(t);
    }
    for (tn, tensors) in &by_type {
        for (c, chunk) in tensors.chunks(DRAM_CHUNK).enumerate() {
            e.open(format!("def dram_{tn}_{c}(): scala.collection.immutable.Vector[DRAM2[{tn}]] = scala.collection.immutable.Vector("));
            for (i, t) in chunk.iter().enumerate() {
                let (pr, pc) = padded_dims(ctx.p, t);
                let sep = if i + 1 < chunk.len() { "," } else { "" };
                e.l(format!("DRAM[{tn}]({pr}, {pc}){sep} // {}", t.name));
            }
            e.close(")");
            e.l(format!("val dram_{tn}_{c}_all = dram_{tn}_{c}()"));
        }
    }
    for (tn, tensors) in &by_type {
        for (c, chunk) in tensors.chunks(DRAM_CHUNK).enumerate() {
            for (i, t) in chunk.iter().enumerate() {
                e.l(format!(
                    "def {}: DRAM2[{tn}] = dram_{tn}_{c}_all({i})",
                    dram_name(t)
                ));
            }
        }
    }
}

/// ★ SPATIAL WRITES A CONTROLLER'S BLOCK AS ONE PARAMETER LIST, AND THE JVM BOUNDS ONE AT 254. A sequence past
/// [`SEQUENCE_CHUNK`] calls is nested — each chunk its own `def` running its calls in a `Sequential` — until the
/// outermost holds at most that many; the order is the calls' order at every level. Returns the outermost calls.
const SEQUENCE_CHUNK: usize = 128;

fn chunk_sequence(
    e: &mut Emit,
    what: &str,
    mut calls: Vec<String>,
    controller: Option<&str>,
) -> Vec<String> {
    let mut level = 0;
    while calls.len() > SEQUENCE_CHUNK {
        let mut next = Vec::new();
        for (g, chunk) in calls.chunks(SEQUENCE_CHUNK).enumerate() {
            let name = format!("{what}_{level}_{g}");
            e.open(format!("def {name}(): Unit = {{"));
            if let Some(c) = controller {
                e.open(format!("{c} {{"));
            }
            for call in chunk {
                e.l(call);
            }
            if controller.is_some() {
                e.close("}");
            }
            e.close("}");
            next.push(format!("{name}()"));
        }
        calls = next;
        level += 1;
    }
    calls
}

/// A tensor's DRAM extent: its tile grid, in elements — every tile a Load or Store moves lies inside it.
fn padded_dims(p: &SpatialProgram, t: &TensorDecl) -> (usize, usize) {
    (t.tiles.rows_t * p.tile.rows, t.tiles.cols_t * p.tile.cols)
}

/// Emit the Spatial regression app (`@spatial class Logos_<name> extends SpatialTest`) for a verified spatial
/// program: every tensor a `DRAM[T]` filled from its recorded fill, every buffer an `SRAM[T]`/`FIFO[T]`, every
/// stage one controller, and the host checking the output tensor's checksum against `gold` — the VM's answer
/// when the caller has it (`Show checksum` of the output), the spatial oracle's (`eval_choreography`)
/// otherwise. `target` is the descriptor the program was compiled for (its name is recorded in the header).
/// The harness's `PASS: true (Logos_<name>)` line is what the toolchain ledger parses.
pub fn emit_spatial_program(
    p: &SpatialProgram,
    target: &SpatialTarget,
    gold: Option<&str>,
) -> Result<String, SpatialLowerError> {
    emit_app(p, target, AppCheck::Checksum(gold), SpatialFlavor::Fabric)
}

/// A slice of a tensor the app checks, element by element, after the accelerator has run: `gold` holds the values
/// the caller vouches for (the VM's) for the row-major elements `offset..offset + gold.len()`. An integer tensor
/// must match exactly; a float one within `rel_tol · max|gold|`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpatialCheck {
    pub tensor: String,
    pub offset: usize,
    pub gold: Vec<f64>,
    pub rel_tol: f64,
}

/// [`emit_spatial_program`] with the host checking every slice of `checks` — the output, and any activation or
/// carried state the caller holds a gold for — rather than one checksum of the output.
pub fn emit_spatial_program_checked(
    p: &SpatialProgram,
    target: &SpatialTarget,
    checks: &[SpatialCheck],
    flavor: SpatialFlavor,
) -> Result<String, SpatialLowerError> {
    if checks.is_empty() {
        return Err(SpatialLowerError::Unsupported(
            "a checked program with no tensor to check — its host would pass against nothing",
        ));
    }
    emit_app(p, target, AppCheck::Elements(checks), flavor)
}

enum AppCheck<'a> {
    Checksum(Option<&'a str>),
    Elements(&'a [SpatialCheck]),
    /// A run of sections, entered with the oracle's values and checked against them.
    Segment {
        sections: std::ops::Range<usize>,
        values: &'a BTreeMap<TensorId, Value>,
        rel_tol: f64,
    },
}

/// A run of consecutive sections of a composed program, printed as ITS OWN app — the unit PIR can map, since PIR
/// writes an accelerator as one JVM class and a whole composed program is past what the JVM allows one. The app
/// holds the tensors the segment's stages touch, each filled with its value at the segment's ENTRY; runs the
/// sections in order; and checks every tensor they store against its value at the segment's EXIT — exactly for an
/// integer tensor, within `rel_tol · max|value|` for a float one.
///
/// `values` is every tensor at the program's end ([`crate::spatial::eval::eval_choreography_every_tensor`]). A
/// composed program stores each tensor in one section at most, so a tensor stored before the segment enters with
/// that value, and any other enters with its fill.
pub fn emit_spatial_segment(
    p: &SpatialProgram,
    target: &SpatialTarget,
    sections: std::ops::Range<usize>,
    values: &BTreeMap<TensorId, Value>,
    rel_tol: f64,
    flavor: SpatialFlavor,
) -> Result<String, SpatialLowerError> {
    if sections.is_empty() || sections.end > p.sections.len() {
        return Err(SpatialLowerError::Unsupported(
            "a segment that is empty or runs past the program's sections",
        ));
    }
    emit_app(
        p,
        target,
        AppCheck::Segment {
            sections,
            values,
            rel_tol,
        },
        flavor,
    )
}

/// A tensor's elements as a fill: the oracle's value, carried into a segment's app.
fn fill_of_value(v: &Value) -> TensorInit {
    match v {
        Value::Float(f) => TensorInit::FloatValues(f.iter().map(|x| x.to_bits()).collect()),
        Value::Int(i) => TensorInit::Values(i.clone()),
    }
}

fn emit_app(
    p: &SpatialProgram,
    target: &SpatialTarget,
    check: AppCheck,
    flavor: SpatialFlavor,
) -> Result<String, SpatialLowerError> {
    let gold = match check {
        AppCheck::Checksum(g) => g,
        AppCheck::Elements(_) | AppCheck::Segment { .. } => None,
    };
    // The sections this app runs, and the tensors it holds — every one, or a segment's.
    let run: std::ops::Range<usize> = match &check {
        AppCheck::Segment { sections, .. } => sections.clone(),
        _ => 0..p.sections.len(),
    };
    let touched: BTreeSet<TensorId> = p
        .stages
        .iter()
        .filter(|st| {
            p.sections.is_empty() || p.section_of_stage(st.id).is_some_and(|k| run.contains(&k))
        })
        .filter_map(|st| match &st.kind {
            StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. } => Some(*tensor),
            _ => None,
        })
        .collect();
    require_verified(p)?;
    refuse_unspelled(p)?;
    let inputs = fill_inputs(p)?;
    let outputs = eval_choreography(p, &inputs).map_err(SpatialLowerError::Uncertified)?;
    let out_t = p.output_tensor().ok_or(SpatialLowerError::Unsupported(
        "a program without an Output tensor has nothing for the host to check",
    ))?;
    let out_v = outputs
        .tensors
        .get(&out_t.id)
        .ok_or(SpatialLowerError::Unsupported(
            "the oracle produced no value for the Output tensor",
        ))?;
    let ix = Index::of(p);
    let scalar = scalar_type(p, &ix, &inputs, out_v);
    // A program holding floats and integers both computes its floats in `T` and declares its integer memories —
    // indices, positions, tokens — `I`.
    let dtypes: Vec<Precision> = p
        .tensors
        .iter()
        .map(|t| t.dtype)
        .chain(p.buffers.iter().map(|b| b.dtype))
        .collect();
    let int_scalar = (dtypes.iter().any(|d| is_float_precision(*d))
        && dtypes.iter().any(|d| !is_float_precision(*d)))
    .then(|| {
        if dtypes.contains(&Precision::Int64) {
            Scalar::Long
        } else {
            Scalar::Int
        }
    });
    let gold_source = if gold.is_some() {
        "the VM's checksum, handed over by the caller"
    } else {
        "the spatial oracle's checksum (eval_choreography over the recorded fills)"
    };
    let gold_note = format!(" // {gold_source}");
    // The gold is of the output's class: a program of floats and integers may emit an integer (a token).
    let out_scalar = if int_scalar.is_some() && !is_float_precision(out_t.dtype) {
        int_scalar.unwrap()
    } else {
        scalar
    };
    let gold = match gold {
        Some(s) => parse_gold(s, out_scalar)?,
        None => checksum_gold(out_v),
    };
    if let Gold::Float(v) = gold {
        if !v.is_finite() {
            return Err(SpatialLowerError::Unsupported("a float gold checksum that is not finite — the output tensor overflowed its precision"));
        }
    }
    let fifo: BTreeSet<BufferId> = p
        .edges
        .iter()
        .filter(|e| matches!(e.handshake, Handshake::Fifo))
        .map(|e| e.buffer)
        .collect();
    let ctx = Ctx {
        p,
        scalar,
        int_scalar,
        fifo,
        names: buffer_names(p),
        flavor,
        ix,
    };
    let class = match &check {
        AppCheck::Segment { sections, .. } => format!(
            "Logos_{}_s{}_{}",
            ident(&p.name),
            sections.start,
            sections.end
        ),
        _ => format!("Logos_{}", ident(&p.name)),
    };
    let sites: Vec<Site> = p.sites().into_iter().collect();

    let mut e = Emit {
        s: String::new(),
        ind: 0,
    };
    e.l(format!(
        "// Generated by LOGOS (logicaffeine_compile::spatial::lower::spatial_lang) — Spatial regression app for the verified\n// spatial program `{}` on {} ({} site(s), tile {}x{}, numerics {:?}). Every controller and memory below is the\n// image of one IR object (stage / buffer / tensor); the host fills every tensor from its recorded fill and checks the\n// output checksum against the gold (the VM's checksum when the caller hands it over, else the spatial oracle's).\n// Values are Spatial `{}`.",
        p.name, target.name, sites.len(), p.tile.rows, p.tile.cols, p.claim, scalar.spatial()
    ));
    e.l("import spatial.dsl._");
    e.l("");
    e.open(format!("@spatial class {class} extends SpatialTest {{"));
    e.l(format!("type T = {}", scalar.spatial()));
    if let Some(i) = int_scalar {
        e.l(format!("type I = {}", i.spatial()));
    }
    e.open("def main(args: Array[String]): Unit = {");
    // A segment enters with the oracle's value of every tensor an earlier section stored.
    let mut host: Vec<TensorDecl> = Vec::new();
    for t in p
        .tensors
        .iter()
        .filter(|t| touched.contains(&t.id) || !matches!(check, AppCheck::Segment { .. }))
    {
        host.push(match &check {
            AppCheck::Segment {
                sections, values, ..
            } if ctx
                .ix
                .storing_section(t.id)
                .is_some_and(|k| k < sections.start) =>
            {
                let v = values.get(&t.id).ok_or(SpatialLowerError::Unsupported(
                    "a segment entering with a tensor the oracle's values do not hold",
                ))?;
                TensorDecl {
                    init: fill_of_value(v),
                    role: TensorRole::Activation,
                    ..t.clone()
                }
            }
            _ => t.clone(),
        });
    }
    if !p.sections.is_empty() && host.len() > HOST_INLINE {
        emit_dram_registry(&mut e, &ctx, &host);
        for t in host.iter().filter(|t| t.role != TensorRole::Output) {
            emit_host_fill(&mut e, &ctx, t, HostFill::MethodOnly);
        }
        let fills: Vec<String> = host
            .iter()
            .filter(|t| t.role != TensorRole::Output)
            .map(|t| format!("fill_{}()", dram_name(t)))
            .collect();
        let mut chunked = Vec::new();
        for (g, chunk) in fills.chunks(DRAM_CHUNK).enumerate() {
            e.open(format!("def fill_all_{g}(): Unit = {{"));
            for call in chunk {
                e.l(call);
            }
            e.close("}");
            chunked.push(format!("fill_all_{g}()"));
        }
        for call in chunked {
            e.l(call);
        }
    } else {
        for t in &host {
            if t.role == TensorRole::Output {
                e.l(format!(
                    "// tensor {} {:?} {}x{} {:?} {:?}",
                    t.name, t.role, t.elems.rows, t.elems.cols, t.dtype, t.space
                ));
                let (pr, pc) = padded_dims(p, t);
                e.l(format!(
                    "val {} = DRAM[{}]({pr}, {pc})",
                    dram_name(t),
                    ctx.ty(t.dtype).name()
                ));
            } else {
                emit_host_fill(
                    &mut e,
                    &ctx,
                    t,
                    if p.sections.is_empty() {
                        HostFill::Inline
                    } else {
                        HostFill::OwnMethod
                    },
                );
            }
        }
    }
    // A composed program's sections are each their own method, for the same reason as its fills.
    for (k, section) in p
        .sections
        .iter()
        .enumerate()
        .filter(|(k, _)| run.contains(k))
    {
        let on = &ctx.ix.section_sites[k];
        e.l(format!(
            "// section {k}: {} ({} site(s))",
            section.name,
            on.len()
        ));
        e.open(format!("def section_{k}(): Unit = {{"));
        e.open(if on.len() == 1 {
            "Sequential {"
        } else {
            "Parallel {"
        });
        for site in on {
            if on.len() > 1 {
                e.open("Pipe {");
            }
            emit_site(&ctx, &mut e, *site, Some(k))?;
            if on.len() > 1 {
                e.close("}");
            }
        }
        e.close("}");
        e.close("}");
    }
    let sequence = chunk_sequence(
        &mut e,
        "run",
        run.clone().map(|k| format!("section_{k}()")).collect(),
        Some("Sequential"),
    );
    e.open("Accel {");
    if p.sections.is_empty() {
        if sites.len() == 1 {
            emit_site(&ctx, &mut e, sites[0], None)?;
        } else {
            e.open("Parallel {");
            for site in &sites {
                e.open("Pipe {");
                emit_site(&ctx, &mut e, *site, None)?;
                e.close("}");
            }
            e.close("}");
        }
    } else {
        // ★ THE RENDEZVOUS IS THE SEQUENCE. A composed program's sections hand over through DRAM, and its gather /
        // release barrier holds every site of section k+1 until every site of section k is done. Spatial orders no
        // DRAM store before a DRAM load in a parallel controller, so the barrier is spelled as what it means: a
        // `Sequential` over the sections, each a `Parallel` of its sites.
        e.open("Sequential {");
        for call in &sequence {
            e.l(call);
        }
        e.close("}");
    }
    e.close("}");
    let segment_checks: Vec<SpatialCheck>;
    let element_checks = match &check {
        AppCheck::Elements(checks) => Some(*checks),
        AppCheck::Segment {
            sections,
            values,
            rel_tol,
        } => {
            segment_checks = p
                .tensors
                .iter()
                .filter(|t| {
                    ctx.ix
                        .storing_section(t.id)
                        .is_some_and(|k| sections.contains(&k))
                })
                .map(|t| {
                    let v = values.get(&t.id).ok_or(SpatialLowerError::Unsupported(
                        "a segment storing a tensor the oracle's values do not hold",
                    ))?;
                    let gold: Vec<f64> = match v {
                        Value::Float(f) => f.clone(),
                        Value::Int(i) => i.iter().map(|x| *x as f64).collect(),
                    };
                    Ok(SpatialCheck {
                        tensor: t.name.clone(),
                        offset: 0,
                        gold,
                        rel_tol: *rel_tol,
                    })
                })
                .collect::<Result<_, SpatialLowerError>>()?;
            if segment_checks.is_empty() {
                return Err(SpatialLowerError::Unsupported(
                    "a segment that stores nothing — it would pass against nothing",
                ));
            }
            Some(&segment_checks[..])
        }
        AppCheck::Checksum(_) => None,
    };
    if let Some(checks) = element_checks {
        emit_element_checks(&mut e, &ctx, checks)?;
        e.l(format!("println(\"PASS: \" + cksum + \" ({class})\")"));
        e.l("assert(cksum)");
        e.close("}");
        e.close("}");
        return Ok(e.s);
    }
    let out_name = dram_name(out_t);
    let (rows, cols) = (out_t.elems.rows, out_t.elems.cols);
    e.l(format!("val {out_name}_result = getMatrix({out_name})"));
    match gold {
        Gold::Int(v) => {
            e.l(format!("val checksum = Array.tabulate({}) {{ q => {out_name}_result(q / {cols}, q % {cols}).to[Long] }}.reduce {{ _ + _ }}", rows * cols));
            e.l(format!("val gold = {}{gold_note}", long_lit(v)));
            e.l("val cksum = checksum == gold");
        }
        Gold::Float(v) => {
            let tol = v.abs() * 1e-2 + 1e-3;
            e.l(format!("val checksum = Array.tabulate({}) {{ q => {out_name}_result(q / {cols}, q % {cols}) }}.reduce {{ _ + _ }}", rows * cols));
            e.l(format!("val gold = {}{gold_note}", float_lit(v)));
            e.l(format!("val tol = {}", float_lit(tol)));
            e.l("val cksum = abs(checksum - gold) <= tol");
        }
    }
    e.l("println(r\"checksum $checksum gold $gold\")");
    e.l(format!("println(\"PASS: \" + cksum + \" ({class})\")"));
    e.l("assert(cksum)");
    e.close("}");
    e.close("}");
    Ok(e.s)
}
