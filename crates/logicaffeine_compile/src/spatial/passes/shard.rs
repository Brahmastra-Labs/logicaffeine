//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TENSOR PARALLELISM — one program over many RDUs, and the reduction that makes it mean something.
//!
//! An SN50 reaches its token throughput by splitting one model across chips; the published 16-chip
//! tensor-parallel run is the shape this pass exists to express. Before it, our tensor parallelism was
//! arithmetic in `spatial/fit.rs` — weight bytes DIVIDED by a chip count and reported to fit. That says
//! a sharded model would fit. It does not say what a sharded model COMPUTES.
//!
//! ★ WHAT A ROW-PARALLEL SHARD ACTUALLY IS. For `C(m×n) = A(m×k)·B(k×n)`, split the CONTRACTION axis
//! `k` into `chips` slices. Chip `c` holds `A[:, kc]` and `B[kc, :]` and computes
//!
//! ```text
//! C_c = A[:, kc] · B[kc, :]
//! ```
//!
//! which has the FULL `m×n` shape and is a PARTIAL SUM — it is missing every term outside its slice.
//! The answer is `C = Σ_c C_c`, and that sum is the all-reduce. This is the whole hazard: `C_c` is
//! correctly shaped, fully written, and wrong. A shard that never reduces returns `Ok` with `1/chips`
//! of the answer, so the reduction is not an optimization to be scheduled later — it is part of the
//! meaning of the program, and [`crate::spatial::certificate`] carries an obligation that says so.
//!
//! ★ THE CHIP IS A COORDINATE, NOT A COMPILATION. Sharding does not produce `chips` programs. It
//! produces ONE program whose sites carry a chip index, because the all-reduce is an edge that spans
//! chips and there is no single-chip program in which that edge can be written down. Everything
//! downstream — placement, buffer sizing, handshake synthesis, the certificate — then sees the
//! collective the way it sees every other hand-off.
//!
//! ★ THE COLLECTIVE'S RECEIVE SIDE IS STILL MISSING, and the IR is right to refuse it. `validate_ports`
//! records the rule: "inter-site edges name the SOURCE buffer at the sender; the receivers own their own
//! copies." A cross-site edge is therefore NOT a producer of the sender's buffer — so a reduce that reads
//! the per-chip partial buffers directly leaves each of them with zero producers
//! (`FifoProducerCount { buffer, count: 0 }`). That is not a technicality: it is the IR insisting that
//! data crossing a chip boundary LANDS SOMEWHERE, in a buffer the receiving site owns. `StageKind::Recv`
//! is already exactly this shape for multicast, and the collective must follow it — one receive per chip
//! at the reducing site, the fold reading those local copies. Until that lands, `shard` builds a program
//! the validator refuses, which is the honest state: it never returns a shard it cannot stand behind.
//!
//! ★ WHY THIS RUNS ON A PLACED PROGRAM. `place` and `tile` are no-ops once their results exist, so
//! sharding a verified program and re-running the pinned passes rebuilds exactly the parts that must
//! change (buffers, handshakes, schedule, certificate) and preserves the tiling decision the cost model
//! already made. The chip assignment this pass writes therefore survives, which it would not if the
//! program were re-placed from scratch.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::passes::from_tensor_op::ident_map;
use crate::spatial::passes::from_tensor_op::MatmulNames;
use crate::spatial::passes::SpatialReject;
use crate::spatial::passes::{LV_GEN_COL, LV_GEN_ROW};
use crate::spatial::pipeline::run_passes;
use crate::spatial::pipeline::template_from_spec;
use crate::spatial::target::HandshakeKind;
use crate::spatial::target::SpatialTarget;
use crate::tensor_recognize::{Fill, MatmulSpec};

/// How a program is split across chips.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardPlan {
    /// How many RDUs the program is spread over. `1` is the identity.
    pub chips: usize,
}

impl ShardPlan {
    /// Split every matmul along its CONTRACTION axis: each chip computes a full-shape partial, and the
    /// partials are summed. This is the half of Megatron tensor parallelism that needs a collective.
    pub fn row_parallel(chips: usize) -> ShardPlan {
        ShardPlan { chips }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShardError {
    /// There is nothing to shard: no matmul, so no contraction axis to split.
    NothingToShard,
    /// The contraction axis does not divide evenly. Refused rather than rounded — a rounded shard drops
    /// or double-counts a slice of the contraction, and the sum of the partials is then quietly wrong.
    AxisNotDivisible {
        axis: &'static str,
        extent: usize,
        chips: usize,
    },
    /// The slice would not be a whole number of native tiles, so a chip's operand could not be loaded by
    /// the tile-addressed movement stages.
    SliceNotTileAligned {
        extent: usize,
        chips: usize,
        tile: usize,
    },
    /// A fill that cannot be sliced faithfully: the shard's elements would not be the original's.
    UnsliceableFill { tensor: String, why: &'static str },
    /// The target offers no hand-off that crosses a site, so a collective cannot be expressed on it at
    /// all — every edge it provides means "these two stages share a site".
    NoCrossChipHandshake { target: &'static str },
    /// The re-run of the pinned passes rejected the sharded program.
    Passes(SpatialReject),
}

/// The front door speaks `Fill`; a lowered program carries `TensorInit`. A slice keeps the ORIGINAL
/// element range (a slice of `(p % 7) - 3` still lies in `[-3, 3]`), so the recovered fill is what decides
/// the operand precision for the per-chip shape exactly as it did for the whole.
fn fill_of(init: &TensorInit) -> Fill {
    match init {
        TensorInit::Zero => Fill::Const(0),
        TensorInit::Pattern { modulus, shift } => Fill::Pattern {
            modulus: *modulus,
            shift: *shift,
        },
        TensorInit::ConstF(b) => Fill::ConstF(*b),
        TensorInit::Values(v) => Fill::Overrides {
            base: Box::new(Fill::Const(0)),
            sets: v.iter().copied().enumerate().collect(),
        },
        TensorInit::FloatValues(v) => Fill::FloatOverrides {
            base: 0,
            sets: v.iter().copied().enumerate().collect(),
        },
        TensorInit::RandomBf16 { .. } | TensorInit::External => Fill::Unknown,
    }
}

/// A tensor's flat element slice for chip `c` of `chips`, along `axis`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Rows,
    Cols,
}

/// Slice a deterministic fill so chip `c`'s elements are the ORIGINAL elements at their original
/// positions. A fill that is a function of the flat index cannot simply be carried over: chip 1's
/// element 0 is the whole tensor's element `offset`, not element 0.
fn slice_init(
    t: &TensorDecl,
    axis: Axis,
    c: usize,
    chips: usize,
) -> Result<TensorInit, ShardError> {
    let (rows, cols) = (t.elems.rows, t.elems.cols);
    let unsliceable = |why: &'static str| {
        Err(ShardError::UnsliceableFill {
            tensor: t.name.clone(),
            why,
        })
    };
    match &t.init {
        // Zero slices to zero, and External stays external: the host supplies each chip its own shard,
        // which is exactly what a real checkpoint load does.
        TensorInit::Zero => Ok(TensorInit::Zero),
        // ★ A CONSTANT IS CONSTANT ON EVERY SLICE. The shard of a tensor whose every element is `b` is a
        // tensor whose every element is `b` — no elements need looking at, which is the whole point of
        // holding it symbolically.
        TensorInit::ConstF(b) => Ok(TensorInit::ConstF(*b)),
        TensorInit::External => Ok(TensorInit::External),
        TensorInit::Pattern { .. } | TensorInit::Values(_) => {
            let mut vals = Vec::new();
            match axis {
                Axis::Rows => {
                    let per = rows / chips;
                    for r in c * per..(c + 1) * per {
                        for j in 0..cols {
                            vals.push(t.init.element(r * cols + j).ok_or_else(|| {
                                ShardError::UnsliceableFill {
                                    tensor: t.name.clone(),
                                    why: "a fill shorter than the tensor",
                                }
                            })?);
                        }
                    }
                }
                Axis::Cols => {
                    let per = cols / chips;
                    for r in 0..rows {
                        for j in c * per..(c + 1) * per {
                            vals.push(t.init.element(r * cols + j).ok_or_else(|| {
                                ShardError::UnsliceableFill {
                                    tensor: t.name.clone(),
                                    why: "a fill shorter than the tensor",
                                }
                            })?);
                        }
                    }
                }
            }
            Ok(TensorInit::Values(vals))
        }
        TensorInit::FloatValues(v) => {
            let mut vals = Vec::new();
            match axis {
                Axis::Rows => {
                    let per = rows / chips;
                    for r in c * per..(c + 1) * per {
                        for j in 0..cols {
                            vals.push(*v.get(r * cols + j).ok_or_else(|| {
                                ShardError::UnsliceableFill {
                                    tensor: t.name.clone(),
                                    why: "a float fill shorter than the tensor",
                                }
                            })?);
                        }
                    }
                }
                Axis::Cols => {
                    let per = cols / chips;
                    for r in 0..rows {
                        for j in c * per..(c + 1) * per {
                            vals.push(*v.get(r * cols + j).ok_or_else(|| {
                                ShardError::UnsliceableFill {
                                    tensor: t.name.clone(),
                                    why: "a float fill shorter than the tensor",
                                }
                            })?);
                        }
                    }
                }
            }
            Ok(TensorInit::FloatValues(vals))
        }
        // The vendor's generator is a function of the whole tensor's shape; a slice of it is not the
        // slice of the values it would have produced, so there is no faithful shard.
        TensorInit::RandomBf16 { .. } => {
            unsliceable("a vendor bf16 random fill is not reproducible slice-wise")
        }
    }
}

fn sliced_decl(
    t: &TensorDecl,
    axis: Axis,
    c: usize,
    chips: usize,
    id: TensorId,
    tile: usize,
) -> Result<TensorDecl, ShardError> {
    let init = slice_init(t, axis, c, chips)?;
    let (rows, cols) = match axis {
        Axis::Rows => (t.elems.rows / chips, t.elems.cols),
        Axis::Cols => (t.elems.rows, t.elems.cols / chips),
    };
    let tiles = TileGrid {
        rows_t: rows.div_ceil(tile),
        cols_t: cols.div_ceil(tile),
    };
    Ok(TensorDecl {
        id,
        name: format!("{}_c{c}", t.name),
        role: t.role,
        elems: ElemShape { rows, cols },
        tiles,
        dtype: t.dtype,
        layout: t.layout,
        space: t.space,
        init,
        range: t.range,
        chip: Some(ChipId(c as u16)),
    })
}

/// The hand-off a partial crosses a chip boundary on: the first of `Token`, `Semaphore`, `Barrier` the target provides.
/// A local hand-off (`Fifo`, `ValidReady`) means "these two stages share a site", which a chip boundary is not.
pub(crate) fn crossing_kind(target: &SpatialTarget) -> Result<HandshakeKind, ShardError> {
    [
        HandshakeKind::Token,
        HandshakeKind::Semaphore,
        HandshakeKind::Barrier,
    ]
    .into_iter()
    .find(|k| target.provides_handshake(*k))
    .ok_or(ShardError::NoCrossChipHandshake {
        target: target.name,
    })
}

/// The edge a crossing of kind `kind` is spelled as.
pub(crate) fn crossing_handshake(kind: HandshakeKind) -> Handshake {
    match kind {
        HandshakeKind::Token => Handshake::Token,
        _ => Handshake::Semaphore {
            ready: SemId::new(0),
            valid: SemId::new(1),
            ready_count: 1,
        },
    }
}

/// The tensor a stage loads, if it is a Load.
fn loaded_tensor(s: &Stage) -> Option<TensorId> {
    match &s.kind {
        StageKind::Load { tensor, .. } => Some(*tensor),
        _ => None,
    }
}

/// Split `p` over `plan.chips` RDUs along the contraction axis, insert the all-reduce, and re-run the
/// pinned passes so the collective is scheduled and certified like any other hand-off.
pub fn shard(
    p: &SpatialProgram,
    plan: &ShardPlan,
    target: &SpatialTarget,
) -> Result<SpatialProgram, ShardError> {
    let chips = plan.chips;
    if chips <= 1 {
        return Ok(p.clone());
    }

    // ── The matmul, and the two operands whose contraction axis is being split.
    let mm = p
        .stages
        .iter()
        .find(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })))
        .ok_or(ShardError::NothingToShard)?;
    let producer_of = |buf: BufferId| {
        p.edges
            .iter()
            .find(|e| e.buffer == buf)
            .map(|e| p.stage(e.from))
    };
    let lhs_stage = mm
        .ins
        .first()
        .and_then(|b| producer_of(*b))
        .ok_or(ShardError::NothingToShard)?;
    let rhs_stage = mm
        .ins
        .get(1)
        .and_then(|b| producer_of(*b))
        .ok_or(ShardError::NothingToShard)?;
    let lhs_t = loaded_tensor(lhs_stage).ok_or(ShardError::NothingToShard)?;
    let rhs_t = loaded_tensor(rhs_stage).ok_or(ShardError::NothingToShard)?;
    let lhs = p
        .tensors
        .iter()
        .find(|t| t.id == lhs_t)
        .ok_or(ShardError::NothingToShard)?;
    let rhs = p
        .tensors
        .iter()
        .find(|t| t.id == rhs_t)
        .ok_or(ShardError::NothingToShard)?;

    // The contraction extent, agreed by both operands.
    let extent = lhs.elems.cols;
    if extent != rhs.elems.rows {
        return Err(ShardError::NothingToShard);
    }
    if extent % chips != 0 {
        return Err(ShardError::AxisNotDivisible {
            axis: "the contraction axis",
            extent,
            chips,
        });
    }
    let tile = p.tile.cols.max(1);
    if (extent / chips) % tile != 0 {
        return Err(ShardError::SliceNotTileAligned {
            extent,
            chips,
            tile,
        });
    }

    // ── ONE CHIP'S PROGRAM, COMPILED FROM A SPEC FOR THE SLICE IT ACTUALLY HOLDS.
    //
    // ★ WHY THE SLICE IS RE-LOWERED FROM A SPEC RATHER THAN EDITED INTO THE LOWERED PROGRAM. Lowering
    // BAKES THE TILING IN. A matmul over a 512-deep contraction lowers to Load stages carrying
    // `num_blocks: 16` and a `MatmulTiles` carrying the same — numbers that live in the STAGES, not only
    // in `work.plan`. Slicing the tensor declarations and clearing the plan therefore produces a program
    // whose tensors say 256 deep and whose stages still say sixteen K blocks: the matmul waits on pages
    // the loads never push, and the choreography fails its counting balance. (Measured, not supposed:
    // `CountingImbalance { buffer: BufferId(0), reserved: 0, pushed: 0, popped: 1 }` with tensors
    // correctly sliced to `a_c0 32x256` and stages still reading `nb=16`.)
    //
    // So the shard goes back to the FRONT DOOR with the dimensions it actually has, and the pinned passes
    // choose the block plan that belongs to a contraction of `k/chips`. Every chip has the same shape and
    // differs only in data, so this is compiled once and replicated.
    let out = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .ok_or(ShardError::NothingToShard)?;
    let names = MatmulNames {
        a: lhs.name.clone(),
        b: rhs.name.clone(),
        c: out.name.clone(),
    };
    let spec = MatmulSpec {
        m: lhs.elems.rows,
        n: rhs.elems.cols,
        k: extent / chips,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let (fa, fb) = (fill_of(&lhs.init), fill_of(&rhs.init));
    let proto = template_from_spec(
        &spec,
        &names,
        (&fa, &fb),
        (lhs.range, rhs.range),
        p.numerics,
        target,
    )
    .and_then(|t| run_passes(t, target))
    .map_err(ShardError::Passes)?;

    // ── Per-chip tensors. Chip 0's slices are already in `proto`; every chip gets its own ids so the
    // shards are distinct tensors with distinct data, which is what "chip 0 holds 1/chips of the
    // weights" has to mean if it means anything.
    let mut tensors: Vec<TensorDecl> = proto
        .tensors
        .iter()
        .filter(|t| t.id != lhs_t && t.id != rhs_t)
        .cloned()
        .collect();
    let mut next_tensor = proto.tensors.iter().map(|t| t.id.0).max().unwrap_or(0) + 1;
    let mut lhs_of_chip = Vec::with_capacity(chips);
    let mut rhs_of_chip = Vec::with_capacity(chips);
    for c in 0..chips {
        let a = TensorId(next_tensor);
        next_tensor += 1;
        tensors.push(sliced_decl(lhs, Axis::Cols, c, chips, a, tile)?);
        lhs_of_chip.push(a);
        let b = TensorId(next_tensor);
        next_tensor += 1;
        tensors.push(sliced_decl(rhs, Axis::Rows, c, chips, b, tile)?);
        rhs_of_chip.push(b);
    }

    // ── Replicate the per-chip pipeline, then ONE all-reduce, then the store it feeds.
    let store = proto
        .stages
        .iter()
        .find(|s| matches!(s.kind, StageKind::Store { .. }))
        .ok_or(ShardError::NothingToShard)?
        .clone();
    let per_chip: Vec<&Stage> = proto.stages.iter().filter(|s| s.id != store.id).collect();

    let mut stages: Vec<Stage> = Vec::new();
    let mut buffers: Vec<Buffer> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut next_stage = 0u32;
    let mut next_buffer = 0u32;
    let mut partial_of_chip = Vec::with_capacity(chips);
    // The proto stage each emitted stage was cloned from, so the ENGINE assignment is inherited rather
    // than invented. Which engine a movement stage runs on is a scheduling decision the placer already
    // made — two operand loads share a site but not an engine, and collapsing them onto one serializes a
    // hand-off the choreography counts on.
    let mut from_proto: Vec<StageId> = Vec::new();

    for c in 0..chips {
        let chip = ChipId(c as u16);
        let on_chip = |s: Option<Site>| s.map(|s| Site::on_chip(chip, s.x as usize, s.y as usize));
        let mut buf_map: Vec<(BufferId, BufferId)> = Vec::new();
        for b in &proto.buffers {
            let id = BufferId(next_buffer);
            next_buffer += 1;
            buf_map.push((b.id, id));
            // ★ THE BUFFER THAT LEAVES THE CHIP HOLDS A PARTIAL SUM, and its only consumer is the
            // collective on another chip. A `Fifo` is the IR's LOCAL hand-off and is held to exactly one
            // producing and one consuming edge — a count that inter-site edges deliberately do not
            // contribute to, since "the receivers own their own copies". `Interm` is the kind the IR
            // already has for a partial-sum buffer, and it carries no local-pairing obligation, so it is
            // both the legal and the honest spelling for a shard's outgoing partial.
            // Chip 0 reduces in place, so ITS partial never leaves; only the other chips' do.
            let leaves_chip = c != 0
                && proto.stages.iter().any(|s| {
                    s.outs.contains(&b.id)
                        && proto
                            .edges
                            .iter()
                            .any(|e| e.buffer == b.id && e.to.contains(&store.id))
                });
            let kind = match (leaves_chip, b.kind) {
                (
                    true,
                    BufferKind::Fifo {
                        depth_pages,
                        page_tiles,
                    },
                ) => BufferKind::Interm {
                    depth_pages,
                    page_tiles,
                },
                (_, k) => k,
            };
            buffers.push(Buffer {
                id,
                name: format!("{}_c{c}", b.name),
                kind,
                dtype: b.dtype,
                space: b.space,
                site: on_chip(b.site),
                origin: None,
                alias_of: None,
            });
        }
        let mb = |old: BufferId| {
            buf_map
                .iter()
                .find(|(o, _)| *o == old)
                .map(|(_, n)| *n)
                .expect("every buffer mapped")
        };
        let mut stage_map: Vec<(StageId, StageId)> = Vec::new();
        for s in &per_chip {
            let id = StageId(next_stage);
            next_stage += 1;
            stage_map.push((s.id, id));
            from_proto.push(s.id);
            let kind = match &s.kind {
                StageKind::Load {
                    tensor,
                    pattern,
                    num_blocks,
                    multicast,
                } => {
                    let t = if *tensor == lhs_t {
                        lhs_of_chip[c]
                    } else if *tensor == rhs_t {
                        rhs_of_chip[c]
                    } else {
                        *tensor
                    };
                    StageKind::Load {
                        tensor: t,
                        pattern: pattern.clone(),
                        num_blocks: *num_blocks,
                        multicast: *multicast,
                    }
                }
                other => other.clone(),
            };
            stages.push(Stage {
                id,
                name: format!("{}_c{c}", s.name),
                kind,
                role: s.role,
                ins: s.ins.iter().map(|b| mb(*b)).collect(),
                outs: s.outs.iter().map(|b| mb(*b)).collect(),
                site: on_chip(s.site),
                item: s.item,
                origin: None,
            });
        }
        let ms = |old: StageId| {
            stage_map
                .iter()
                .find(|(o, _)| *o == old)
                .map(|(_, n)| *n)
                .expect("every stage mapped")
        };
        for e in &proto.edges {
            if e.to.contains(&store.id) {
                partial_of_chip.push(mb(e.buffer));
                continue;
            }
            edges.push(Edge {
                id: EdgeId(edges.len() as u32),
                from: ms(e.from),
                buffer: mb(e.buffer),
                to: e.to.iter().map(|t| ms(*t)).collect(),
                handshake: e.handshake.clone(),
            });
        }
    }

    if partial_of_chip.len() != chips {
        return Err(ShardError::NothingToShard);
    }

    // ── The collective, on chip 0 beside the store that consumes its result.
    let out_tensor = match &store.kind {
        StageKind::Store { tensor, .. } => *tensor,
        _ => return Err(ShardError::NothingToShard),
    };
    let seed = store
        .ins
        .first()
        .and_then(|b| proto.buffers.iter().find(|x| x.id == *b));
    let reduced = BufferId(next_buffer);
    let home = store
        .site
        .map(|s| Site::on_chip(ChipId(0), s.x as usize, s.y as usize));
    buffers.push(Buffer {
        id: reduced,
        name: "allreduce_out".into(),
        kind: seed.map(|b| b.kind).unwrap_or(BufferKind::Fifo {
            depth_pages: 2,
            page_tiles: 1,
        }),
        dtype: seed.map(|b| b.dtype).unwrap_or(Precision::Int64),
        space: seed.map(|b| b.space).unwrap_or(MemSpace::Local),
        site: home,
        origin: None,
        alias_of: None,
    });
    // ★ THE COMBINE IS AN N-ARY ELEMENTWISE FOLD, nothing more. `Sum` over `chips` operands is
    // `Add(Add(op0, op1), op2)…` on identity maps — the closed algebra's general form, so the collective
    // needs no print arm, no evaluator arm and no cost arm of its own.
    let width = target.tile.cols;
    let combine = ComputeOp::Generic {
        iters: vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: target.tile.rows,
                kind: IterKind::Parallel,
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: width.max(1),
                kind: IterKind::Parallel,
            },
        ],
        ins: (0..chips).map(|_| ident_map(width)).collect(),
        out: ident_map(width),
        body: (1..chips).fold(TensorBody::Operand(0), |acc, i| {
            TensorBody::Add(Box::new(acc), Box::new(TensorBody::Operand(i)))
        }),
    };
    let reduce_id = StageId(next_stage);
    next_stage += 1;
    stages.push(Stage {
        id: reduce_id,
        name: format!("allreduce_{chips}_chips"),
        kind: StageKind::AllReduce {
            pool: ReducePool::Sum,
            tensor: out_tensor,
            combine: Box::new(combine),
        },
        role: SiteRole::Compute,
        ins: partial_of_chip.clone(),
        outs: vec![reduced],
        site: home,
        item: store.item,
        origin: None,
    });
    from_proto.push(store.id);
    let store_id = StageId(next_stage);
    stages.push(Stage {
        id: store_id,
        name: store.name.clone(),
        kind: store.kind.clone(),
        role: store.role,
        ins: vec![reduced],
        outs: Vec::new(),
        site: home,
        item: store.item,
        origin: None,
    });
    from_proto.push(store.id);
    // ★ A PARTIAL CROSSES A CHIP BOUNDARY, so its edge may not carry a LOCAL hand-off. A Fifo or
    // ValidReady edge means "these two stages share a site"; the IR refuses one whose endpoints do not
    // (`CrossSiteLocalEdge`), which is the invariant catching the difference between a collective and an
    // ordinary producer/consumer pair. The handshake is taken from the target rather than hardcoded so
    // the lane is not SN50-shaped by accident.
    let local = crate::spatial::passes::from_tensor_op::local_handshake(target)
        .map_err(ShardError::Passes)?;
    let crossing = crossing_kind(target)?;
    for (c, b) in partial_of_chip.iter().enumerate() {
        let from = stages
            .iter()
            .find(|s| s.outs.contains(b))
            .map(|s| s.id)
            .expect("a partial has a producer");
        // The collective reduces ON chip 0, so chip 0's own partial is a SAME-SITE hand-off and must use
        // the local one; only the other chips' partials cross. An inter-site handshake between two stages
        // that share a site is refused just as firmly as the reverse (`LocalCrossSiteEdge`), and rightly:
        // each spelling is a claim about where the data has to travel.
        let handshake = if c == 0 {
            // The target's own local hand-off, not a bare `Fifo`: `Fifo` is the TEMPLATE's abstract
            // spelling and SN50 does not provide it (its local kind is ValidReady).
            local.clone()
        } else {
            crossing_handshake(crossing)
        };
        edges.push(Edge {
            id: EdgeId(edges.len() as u32),
            from,
            buffer: *b,
            to: vec![reduce_id],
            handshake,
        });
    }
    edges.push(Edge {
        id: EdgeId(edges.len() as u32),
        from: reduce_id,
        buffer: reduced,
        to: vec![store_id],
        handshake: local.clone(),
    });

    // ── The placement, rewritten with the chip coordinate so `place` leaves it alone.
    let placement = proto.placement.as_ref().map(|pl| Placement {
        mesh: pl.mesh,
        site_of: stages
            .iter()
            .map(|s| s.site.unwrap_or(Site::new(0, 0)))
            .collect(),
        engine_of: stages
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let inherited = from_proto.get(i).and_then(|o| pl.engine_of.get(o.idx()));
                // Inherit the placer's engine choice, EXCEPT where the role disagrees: the collective is
                // the one stage with no counterpart in the per-chip program, and it folds, so it belongs
                // on a compute engine rather than on the movement engine its neighbouring store uses.
                let engine = match inherited.map(|e| e.engine) {
                    Some(e) if (e == Engine::Compute) == (s.role == SiteRole::Compute) => e,
                    _ if s.role == SiteRole::Compute => Engine::Compute,
                    _ => Engine::Movement(0),
                };
                EngineSlot {
                    site: s.site.unwrap_or(Site::new(0, 0)),
                    engine,
                    noc: inherited.and_then(|e| e.noc),
                }
            })
            .collect(),
        groups: pl.groups.clone(),
        items_of_site: {
            let mut v: Vec<(Site, Vec<WorkItemId>)> = Vec::new();
            for (site, items) in &pl.items_of_site {
                for c in 0..chips {
                    v.push((
                        Site::on_chip(ChipId(c as u16), site.x as usize, site.y as usize),
                        items.clone(),
                    ));
                }
            }
            v
        },
    });

    // ── Dense tensor ids. Removing the two whole operands and appending `2*chips` slices leaves the
    // surviving tensors at their OLD ids and the list out of step with them; every index-addressed lookup
    // downstream then reads the wrong declaration. Renumber by position and rewrite every stage that
    // names a tensor.
    let remap: Vec<(TensorId, TensorId)> = tensors
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id, TensorId(i as u32)))
        .collect();
    let rt = |old: TensorId| {
        remap
            .iter()
            .find(|(o, _)| *o == old)
            .map(|(_, n)| *n)
            .unwrap_or(old)
    };
    for (i, t) in tensors.iter_mut().enumerate() {
        t.id = TensorId(i as u32);
    }
    for st in &mut stages {
        match &mut st.kind {
            StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. } => {
                *tensor = rt(*tensor)
            }
            StageKind::AllReduce { tensor, .. } => *tensor = rt(*tensor),
            _ => {}
        }
    }

    let sharded = SpatialProgram {
        name: format!("{}_tp{chips}", p.name),
        tile: proto.tile,
        tensors,
        stages,
        buffers,
        edges,
        semaphores: Vec::new(),
        sections: Vec::new(),
        trips: proto
            .trips
            .iter()
            .map(|t| Trip {
                tensor: rt(t.tensor),
                ..t.clone()
            })
            .collect(),
        links: Vec::new(),
        work: proto.work.clone(),
        numerics: proto.numerics,
        claim: proto.claim.clone(),
        acc_tiles_used: proto.acc_tiles_used,
        mem_banks: proto.mem_banks,
        requires: {
            // The shard introduces a hand-off the single-chip program never needed; `requires` is the
            // program's own declaration of what its target must provide, so it has to name it.
            let mut r = proto.requires.clone();
            r.handshakes.insert(crossing);
            r
        },
        placement,
        memory: None,
        schedule: None,
        certificate: None,
    };
    run_passes(sharded, target).map_err(ShardError::Passes)
}
