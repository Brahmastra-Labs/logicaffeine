//! `from_tensor_op` — a recognized tensor op becomes a TEMPLATE spatial program (phase `Template`): the
//! tensors, the four stages, three double-buffered FIFOs, and one work item per output tile. The numerics are
//! chosen HERE from the target's matrix engine and the recognized fills, and the exactness claim is derived,
//! never assumed: an `Int` source on a float-primary engine rides as bf16 operands with an fp32 accumulator and
//! is claimed exact only while every partial sum fits fp32's mantissa.

use super::{SpatialReject, LV_GEN_COL, LV_GEN_ROW, LV_ITEM_X, LV_ITEM_Y, LV_KBLOCK};
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::target::{precision_int_bits, SpatialTarget};
use crate::tensor_recognize::{ElementwiseOp, Fill, MatmulSpec, RowReduceKind};

/// The source-level names of a matmul's tensors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatmulNames {
    pub a: String,
    pub b: String,
    pub c: String,
}

pub fn init_of(fill: &Fill, n: usize) -> TensorInit {
    match fill {
        Fill::Const(0) => TensorInit::Zero,
        Fill::Const(c) => TensorInit::Pattern {
            modulus: 1,
            shift: *c,
        },
        Fill::Pattern { modulus, shift } => TensorInit::Pattern {
            modulus: *modulus,
            shift: *shift,
        },
        Fill::Overrides { base, sets } => {
            let base = init_of(base, n);
            let mut vals: Vec<i64> = (0..n).map(|i| base.element(i).unwrap_or(0)).collect();
            for (i, v) in sets {
                if *i < n {
                    vals[*i] = *v;
                }
            }
            TensorInit::Values(vals)
        }
        Fill::ConstF(b) => TensorInit::ConstF(*b),
        Fill::FloatOverrides { base, sets } => {
            let mut vals = vec![*base; n];
            for (i, v) in sets {
                if *i < n {
                    vals[*i] = *v;
                }
            }
            TensorInit::FloatValues(vals)
        }
        // A real pattern is supplied at run time here, as data the compiler cannot see is: the device inits hold
        // a float fill as one constant or as every value, and a pattern's every value is the materialization the
        // `ConstF` init exists to avoid.
        Fill::RealPattern { .. } | Fill::Unknown => TensorInit::External,
    }
}

// ── The other tensor ops: one template shape, one builder ─────────────────────────────────────────

/// Names of an elementwise op's tensors (from the source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementwiseNames {
    pub a: String,
    pub b: String,
    pub c: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwiGluNames {
    pub gate: String,
    pub up: String,
    pub y: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FusedRowNames {
    pub x: String,
    pub y: String,
    pub weight: Option<String>,
    pub residual: Option<String>,
    pub bias: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FusedRowFills {
    pub x: Fill,
    pub weight: Option<Fill>,
    pub residual: Option<Fill>,
    pub bias: Option<Fill>,
}

/// A template under construction: tensors, stages, buffers and local hand-off edges, in id order.
///
/// ★ THE ONE BUILDER EVERY TEMPLATE COMES FROM — the recognized workloads below, the B-P4 Qwen
/// compositions, and the hand-built probes of `spatial_algebra_expansion_lock`. Building a stage any
/// other way is how two programs end up disagreeing about what a stage IS.
pub struct Template {
    tile: crate::spatial::target::TileShape,
    local: Handshake,
    tensors: Vec<TensorDecl>,
    stages: Vec<Stage>,
    buffers: Vec<Buffer>,
    edges: Vec<Edge>,
    trips: Vec<Trip>,
}

impl Template {
    pub fn new(target: &SpatialTarget) -> Result<Template, SpatialReject> {
        Ok(Template {
            tile: target.tile,
            local: Handshake::Fifo,
            tensors: Vec::new(),
            stages: Vec::new(),
            buffers: Vec::new(),
            edges: Vec::new(),
            trips: Vec::new(),
        })
    }
    /// Declare a loop count the program reads from its data ([`Trip`]).
    pub fn trip(&mut self, trip: Trip) -> TripId {
        self.trips.push(trip);
        TripId::new(self.trips.len() - 1)
    }
    pub fn tensor(
        &mut self,
        name: &str,
        role: TensorRole,
        rows: usize,
        cols: usize,
        dtype: Precision,
        init: TensorInit,
    ) -> TensorId {
        let id = TensorId::new(self.tensors.len());
        self.tensors.push(TensorDecl {
            id,
            name: name.to_string(),
            role,
            elems: ElemShape { rows, cols },
            tiles: TileGrid {
                rows_t: rows.div_ceil(self.tile.rows),
                cols_t: cols.div_ceil(self.tile.cols),
            },
            dtype,
            layout: Layout::RowMajorTiles,
            space: TensorSpace::Dram,
            init,
            range: None,
            chip: None,
        });
        id
    }
    pub fn buffer(&mut self, name: &str, page_tiles: usize, dtype: Precision) -> BufferId {
        let id = BufferId::new(self.buffers.len());
        self.buffers.push(Buffer {
            id,
            name: name.into(),
            kind: BufferKind::Fifo {
                depth_pages: 2,
                page_tiles,
            },
            dtype,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        });
        id
    }
    /// A buffer that lives only between two compute stages. It is neither filled by a reader nor
    /// drained by a writer, so it takes the intermediate circular-buffer class rather than an
    /// operand or output slot.
    /// A hand-off of an explicit DEPTH. Double buffering is the right default — it lets a producer fill
    /// page `n+1` while the consumer drains page `n` — but it costs twice the local memory, and a stage
    /// whose page is consumed by the very next stage in its own item never overlaps anything. A section
    /// that fans into many sub-blocks pays that doubling once per operand per sub-block, which is how a
    /// 426 KB site budget disappears.
    pub fn buffer_deep(
        &mut self,
        name: &str,
        depth_pages: usize,
        page_tiles: usize,
        dtype: Precision,
    ) -> BufferId {
        let id = BufferId::new(self.buffers.len());
        self.buffers.push(Buffer {
            id,
            name: name.into(),
            kind: BufferKind::Fifo {
                depth_pages,
                page_tiles,
            },
            dtype,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        });
        id
    }
    /// [`Template::scratch`] with an explicit depth.
    pub fn scratch_deep(
        &mut self,
        name: &str,
        depth_pages: usize,
        page_tiles: usize,
        dtype: Precision,
    ) -> BufferId {
        let id = BufferId::new(self.buffers.len());
        self.buffers.push(Buffer {
            id,
            name: name.into(),
            kind: BufferKind::Interm {
                depth_pages,
                page_tiles,
            },
            dtype,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        });
        id
    }
    pub fn scratch(&mut self, name: &str, page_tiles: usize, dtype: Precision) -> BufferId {
        let id = BufferId::new(self.buffers.len());
        self.buffers.push(Buffer {
            id,
            name: name.into(),
            kind: BufferKind::Interm {
                depth_pages: 2,
                page_tiles,
            },
            dtype,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        });
        id
    }
    pub fn stage(
        &mut self,
        name: &str,
        kind: StageKind,
        ins: Vec<BufferId>,
        outs: Vec<BufferId>,
    ) -> StageId {
        let id = StageId::new(self.stages.len());
        let role = kind.role();
        self.stages.push(Stage {
            id,
            name: name.into(),
            kind,
            role,
            ins,
            outs,
            site: None,
            item: None,
            origin: None,
        });
        id
    }
    /// A load of `block_w` tiles of one tile-row of `tensor`, addressed by the item's coordinates.
    pub fn load(
        &mut self,
        name: &str,
        tensor: TensorId,
        steps: Vec<(LoopVar, usize)>,
        block_w: usize,
        out: BufferId,
    ) -> StageId {
        let cols_t = self.tensors[tensor.idx()].tiles.cols_t;
        self.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps,
                        indirect: None,
                    },
                    block_h: 1,
                    block_w,
                    stride_w: 1,
                    stride_h: cols_t,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![],
            vec![out],
        )
    }
    /// A load of a `block_h x block_w` TILE BLOCK whose rows are `stride_h` tiles apart — the shape a
    /// strided view needs when the rows it wants are not adjacent in the tensor. A causal conv's ring is
    /// `taps x chan` flattened, so one channel block's taps sit `chan/tile` tiles apart.
    pub fn load_block(
        &mut self,
        name: &str,
        tensor: TensorId,
        steps: Vec<(LoopVar, usize)>,
        block_h: usize,
        block_w: usize,
        stride_h: usize,
        out: BufferId,
    ) -> StageId {
        self.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps,
                        indirect: None,
                    },
                    block_h,
                    block_w,
                    stride_w: 1,
                    stride_h,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![],
            vec![out],
        )
    }
    /// The store twin of [`Template::load_block`], with an explicit tile `base` — the constant part of
    /// an address whose variable part is the item's coordinate.
    pub fn store_block(
        &mut self,
        name: &str,
        tensor: TensorId,
        base: usize,
        steps: Vec<(LoopVar, usize)>,
        block_h: usize,
        block_w: usize,
        stride_h: usize,
        input: BufferId,
    ) -> StageId {
        self.stage(
            name,
            StageKind::Store {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base,
                        steps,
                        indirect: None,
                    },
                    block_h,
                    block_w,
                    stride_w: 1,
                    stride_h,
                },
                sub_blocks: (1, 1),
            },
            vec![input],
            vec![],
        )
    }
    /// [`Template::store_block`] over the item's `sub_blocks` — the extent the pattern's `LV_SUB_H` / `LV_SUB_W` steps
    /// walk. ★ A TEMPLATE STATES WHAT ITS PLAN SAYS: placement reshapes only the store of the section's result to a
    /// site's block, so every other store's sub-blocks are the template's, and the coverage check reads them.
    #[allow(clippy::too_many_arguments)]
    pub fn store_block_over(
        &mut self,
        name: &str,
        tensor: TensorId,
        base: usize,
        steps: Vec<(LoopVar, usize)>,
        block_h: usize,
        block_w: usize,
        stride_h: usize,
        sub_blocks: (usize, usize),
        input: BufferId,
    ) -> StageId {
        self.stage(
            name,
            StageKind::Store {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base,
                        steps,
                        indirect: None,
                    },
                    block_h,
                    block_w,
                    stride_w: 1,
                    stride_h,
                },
                sub_blocks,
            },
            vec![input],
            vec![],
        )
    }
    /// [`Template::load_block`] with an explicit tile `base`.
    pub fn load_block_at(
        &mut self,
        name: &str,
        tensor: TensorId,
        base: usize,
        steps: Vec<(LoopVar, usize)>,
        block_h: usize,
        block_w: usize,
        stride_h: usize,
        out: BufferId,
    ) -> StageId {
        self.stage(
            name,
            StageKind::Load {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base,
                        steps,
                        indirect: None,
                    },
                    block_h,
                    block_w,
                    stride_w: 1,
                    stride_h,
                },
                num_blocks: 1,
                multicast: None,
            },
            vec![],
            vec![out],
        )
    }
    pub fn store(
        &mut self,
        name: &str,
        tensor: TensorId,
        steps: Vec<(LoopVar, usize)>,
        block_w: usize,
        input: BufferId,
    ) -> StageId {
        let cols_t = self.tensors[tensor.idx()].tiles.cols_t;
        self.stage(
            name,
            StageKind::Store {
                tensor,
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps,
                        indirect: None,
                    },
                    block_h: 1,
                    block_w,
                    stride_w: 1,
                    stride_h: cols_t,
                },
                sub_blocks: (1, 1),
            },
            vec![input],
            vec![],
        )
    }
    /// Whether a stage names a buffer among its operands (a chain wires by this, not by position).
    pub fn stage_reads(&self, s: StageId, b: BufferId) -> bool {
        self.stages[s.idx()].ins.contains(&b)
    }
    /// The buffer a chain stage writes.
    pub fn stage_output(&self, s: StageId) -> BufferId {
        self.stages[s.idx()].outs[0]
    }
    /// The element precision buffer `b` holds.
    pub fn dtype(&self, b: BufferId) -> Precision {
        self.buffers[b.idx()].dtype
    }
    /// Tiles per page of a buffer — the page WIDTH every flat access map is taken over.
    pub fn page_tiles(&self, b: BufferId) -> usize {
        self.buffers[b.idx()].kind.page_tiles()
    }
    /// One hand-off read by SEVERAL stages. A buffer has exactly one producing EDGE, not one per
    /// consumer — a fan-out is a single edge with a list of readers, which is also what the choreography
    /// needs in order to pop the page once rather than once per reader.
    pub fn edge_many(&mut self, from: StageId, buffer: BufferId, to: Vec<StageId>) {
        let id = EdgeId::new(self.edges.len());
        self.edges.push(Edge {
            id,
            from,
            buffer,
            to,
            handshake: self.local.clone(),
        });
    }
    pub fn edge(&mut self, from: StageId, buffer: BufferId, to: StageId) {
        let id = EdgeId::new(self.edges.len());
        self.edges.push(Edge {
            id,
            from,
            buffer,
            to: vec![to],
            handshake: self.local.clone(),
        });
    }
    pub fn finish(
        self,
        name: String,
        items: Vec<WorkItem>,
        acc: AccPolicy,
        claim: NumericsClaim,
        target: &SpatialTarget,
    ) -> Result<SpatialProgram, SpatialReject> {
        let mut requires = Requirements::default();
        requires.handshakes.insert(self.local.kind());
        let p = SpatialProgram {
            name,
            tile: self.tile,
            tensors: self.tensors,
            stages: self.stages,
            buffers: self.buffers,
            edges: self.edges,
            semaphores: Vec::new(),
            sections: Vec::new(),
            trips: self.trips,
            links: Vec::new(),
            work: WorkList {
                items,
                k_tiles: 1,
                plan: None,
                k_trip: None,
            },
            numerics: NumericsPolicy::Exact,
            claim,
            acc_tiles_used: target.effective_acc_tiles(acc, true),
            mem_banks: target.site.mem_banks,
            requires,
            placement: None,
            memory: None,
            schedule: None,
            certificate: None,
        };
        p.validate()?;
        Ok(p)
    }
}

fn elementwise_precisions(
    target: &SpatialTarget,
) -> Option<&std::collections::BTreeSet<Precision>> {
    target.compute.iter().find_map(|c| match c {
        crate::spatial::target::ComputeCap::Elementwise { precisions } => Some(precisions),
        _ => None,
    })
}

fn row_reduce_precisions(target: &SpatialTarget) -> Option<&std::collections::BTreeSet<Precision>> {
    target.compute.iter().find_map(|c| match c {
        crate::spatial::target::ComputeCap::RowReduce { precisions } => Some(precisions),
        _ => None,
    })
}

/// The float precision the target's ELEMENTWISE datapath offers, widest first — the one a composition
/// carries its activations in (bf16 on the RDU's witnessed envelope).
pub fn elementwise_float(target: &SpatialTarget) -> Option<Precision> {
    elementwise_precisions(target).and_then(float_precision)
}

/// The integer precision the target's ELEMENTWISE datapath offers, widest first — the one an integer model's
/// tensors are carried in: an `Int` is the VM's 64-bit integer, so the widest the part has is the closest to it.
pub fn elementwise_int(target: &SpatialTarget) -> Option<Precision> {
    let ps = elementwise_precisions(target)?;
    [Precision::Int64, Precision::Int32, Precision::Int16, Precision::Int8]
        .into_iter()
        .find(|p| ps.contains(p))
}

/// The widest float precision a capability offers (fp32 first: exact for integer data below 2²⁴).
fn float_precision(ps: &std::collections::BTreeSet<Precision>) -> Option<Precision> {
    [Precision::Fp32, Precision::Bf16, Precision::Fp16]
        .into_iter()
        .find(|p| ps.contains(p))
}

/// One work item per output tile along a single tile-row (elementwise / SwiGLU: 1×n tensors).
pub(crate) fn items_per_tile(cols_t: usize) -> Vec<WorkItem> {
    items_per_tile_grid(1, cols_t)
}

/// One work item per tile of a `rows_t × cols_t` tile grid, row-major — the tiling of a pointwise map over a
/// matrix. Item `(ry, cx)` owns tile `(ry, cx)`.
pub(crate) fn items_per_tile_grid(rows_t: usize, cols_t: usize) -> Vec<WorkItem> {
    (0..rows_t)
        .flat_map(|ry| (0..cols_t).map(move |cx| (ry, cx)))
        .enumerate()
        .map(|(id, (ry, cx))| WorkItem {
            id: WorkItemId::new(id),
            coord: (ry, cx),
            out: TileRect {
                row0_t: ry,
                col0_t: cx,
                h_t: 1,
                w_t: 1,
            },
        })
        .collect()
}

/// The tile address of an item of [`items_per_tile_grid`]: tile `ry · cols_t + cx`. A single tile row needs no
/// row term, and a `1 × n` row keeps exactly the address it has always had.
pub(crate) fn tile_grid_steps(rows_t: usize, cols_t: usize) -> Vec<(LoopVar, usize)> {
    if rows_t > 1 {
        vec![(LV_ITEM_Y, cols_t), (LV_ITEM_X, 1)]
    } else {
        vec![(LV_ITEM_X, 1)]
    }
}

/// ★ A RECOGNIZED PRODUCER/CONSUMER PIPE, AS SPATIAL IR — the template that retires
/// `codegen_tt::concurrency_mesh`'s hand-written two-core program.
///
/// **The shape was already in the IR's vocabulary.** A task sending N values into a pipe while main
/// receives and folds them is a LOAD on one engine feeding a FIFO edge, and a REDUCTION on the other
/// draining it — which is precisely what the retired emitter spelled out by hand as two baby-RISC-V
/// kernels around a circular buffer. Nothing new was needed: `ChoreoOp::{Reserve,Push,Wait,Pop}` are
/// the circular-buffer discipline, and a fold is not an operator but an `IterKind::Reduce` axis.
///
/// The sends are constants OF THE RECOGNIZED PROGRAM, so they ride as the input tensor's `Values`
/// init rather than as a `const uint32_t vals[N]` baked into a kernel string. That is the whole
/// difference between a template and a lowering: the same numbers, but reachable by every pass,
/// every census and the evaluator instead of being sealed inside C++ text.
///
/// **Numerics — why fp32 and not an integer datapath.** A pipe of integers has an integer sum, so
/// the fold must be exact. Asking for `Precision::Int64` would make `trivial_plan` choose
/// `AccPolicy::I64`, which the Tensix printer refuses: the matrix engine is float-primary, the same
/// hardware fact that retired the int8 GEMM path (measured 0/4096 correct on Wormhole AND Blackhole).
/// Fp32 represents integers exactly below 2^23, so a bounded pipe keeps `NumericsClaim::Exact` on the
/// validated datapath. A pipe whose fold could leave that range is REFUSED rather than silently
/// rounded — the claim has to be true, not convenient.
pub fn from_pipe(sends: &[i64], target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let n = sends.len();
    if n == 0 {
        return Err(SpatialReject::NotLowered("a pipe with no sends"));
    }
    // The fold's worst case, not the individual values: summing many small integers is what leaves
    // the range, and it leaves it silently.
    const EXACT_FP32: i64 = 1 << 23;
    let bound: i64 = sends
        .iter()
        .map(|v| v.saturating_abs())
        .try_fold(0i64, |a: i64, v| a.checked_add(v))
        .unwrap_or(i64::MAX);
    if bound >= EXACT_FP32 {
        return Err(SpatialReject::NotLowered(
            "a pipe whose fold leaves the exactly-representable fp32 range",
        ));
    }
    let caps = elementwise_precisions(target).ok_or(SpatialReject::MissingCompute {
        needed: "elementwise",
        target: target.name,
    })?;
    let precision = float_precision(caps).ok_or(SpatialReject::MissingCompute {
        needed: "a float elementwise datapath",
        target: target.name,
    })?;

    let mut t = Template::new(target)?;
    let ch = t.tensor(
        "ch",
        TensorRole::Input,
        1,
        n,
        precision,
        TensorInit::Values(sends.to_vec()),
    );
    let total = t.tensor(
        "total",
        TensorRole::Output,
        1,
        1,
        precision,
        TensorInit::Zero,
    );
    let cols_t = t.tensors[ch.idx()].tiles.cols_t;
    let pipe = t.buffer("pipe", cols_t, precision);
    let out = t.buffer("out", 1, precision);
    // ★ THE REDUCE ENGINE'S SCALER OPERAND, AT 1.0. Tensix's `reduce_tile` multiplies every term by a
    // constant scaler tile and REQUIRES one — a fold with a single operand is not expressible on the
    // part. At 1.0 it is the identity, so the sum is the sum; the operand exists because the hardware
    // has it, not because the specification wanted a coefficient.
    let scaler_elems = target.tile.rows * target.tile.cols;
    let ones = t.tensor(
        "pipe_scaler",
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        precision,
        TensorInit::FloatValues(vec![1.0f64.to_bits(); scaler_elems]),
    );
    let ones_in = t.buffer("in_scaler", 1, precision);
    let l_ones = t.load("load_scaler", ones, vec![], 1, ones_in);
    // The producer: one load of the whole pipe, the send side of the circular buffer.
    let send = t.load("send", ch, vec![(LV_ITEM_X, 1)], cols_t, pipe);
    // The consumer: receive and fold. The column axis is REDUCED away, so `out` is the scalar total.
    let fold = t.stage(
        "receive_fold",
        StageKind::Compute(ComputeOp::Generic {
            iters: space(target, target.tile.cols, Some(ReducePool::Sum)),
            ins: vec![ident_map(cols_t * target.tile.cols), const_map()],
            out: const_map(),
            body: TensorBody::Operand(0),
        }),
        vec![pipe, ones_in],
        vec![out],
    );
    let show = t.store("show", total, vec![(LV_ITEM_X, 1)], 1, out);
    t.edge(send, pipe, fold);
    t.edge(l_ones, ones_in, fold);
    t.edge(fold, out, show);
    t.finish(
        format!("pipe_{n}"),
        items_per_tile(1),
        AccPolicy::F32,
        NumericsClaim::Exact,
        target,
    )
}

/// `C[p] = A[p] ⊕ B[p]` over a flat length: integer datapath where the target has one, fp32 otherwise.
pub fn from_elementwise(
    spec: &crate::tensor_recognize::ElementwiseSpec,
    names: &ElementwiseNames,
    fills: (&Fill, &Fill),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    from_elementwise_with(spec, names, fills, false, (1, spec.n), target)
}

/// The same, told whether the operands are float even when neither carries a recognized fill — which is
/// every activation inside a composition. Without it a target that offers an integer elementwise
/// datapath would take it for a pair of tensors that hold bf16 model activations.
pub fn from_elementwise_with(
    spec: &crate::tensor_recognize::ElementwiseSpec,
    names: &ElementwiseNames,
    fills: (&Fill, &Fill),
    float_hint: bool,
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let caps = elementwise_precisions(target).ok_or(SpatialReject::MissingCompute {
        needed: "elementwise",
        target: target.name,
    })?;
    let (rows, cols) = if shape.0 * shape.1 == spec.n {
        shape
    } else {
        (1, spec.n)
    };
    let init_a = init_of(fills.0, spec.n);
    let init_b = init_of(fills.1, spec.n);
    let float_data = float_hint || init_a.is_float() || init_b.is_float();
    let precision = if !float_data && caps.contains(&Precision::Int64) {
        Precision::Int64
    } else {
        float_precision(caps).ok_or(SpatialReject::MissingCompute {
            needed: "a float elementwise datapath",
            target: target.name,
        })?
    };
    let bound = init_a
        .abs_bound()
        .unwrap_or(i64::MAX)
        .max(init_b.abs_bound().unwrap_or(i64::MAX));
    let claim = match precision {
        Precision::Int64 => NumericsClaim::Exact,
        Precision::Fp32 if !float_data && bound < (1 << 23) => NumericsClaim::Exact,
        _ => NumericsClaim::Approx {
            reason: format!("elementwise in {precision:?}"),
        },
    };
    let body = match spec.op {
        ElementwiseOp::Add => TensorBody::Add(Box::new(TensorBody::Operand(0)), Box::new(TensorBody::Operand(1))),
        ElementwiseOp::Mul => TensorBody::Mul(Box::new(TensorBody::Operand(0)), Box::new(TensorBody::Operand(1))),
    };
    // ★ ONE ROW COMBINED WITH EVERY ROW (a bias over a block of rows): `C[r, c] = A[r, c] ⊕ B[c]`.
    if let Some(w) = spec.row {
        if w == 0 || spec.n % w != 0 {
            return Err(SpatialReject::NotLowered("an elementwise op whose row does not divide its operand"));
        }
        return row_broadcast(spec, names, fills, (rows, cols), w, precision, claim, body, target);
    }
    // ★ A ROW IS ONE ITEM WHILE A SITE HOLDS IT — the rule a row map follows (`from_qwen_ops::unary_map`), for two
    // operands: an item a tile reads, adds and writes the row an element at a time where the tile is an element. A row
    // of one tile is one item already, over its tile. Past the page, or past a site's memory before the page, the row
    // is the tile-per-item form below.
    if rows == 1 && cols.div_ceil(target.tile.cols) > 1 && !super::from_qwen_ops::row_needs_blocking(cols, target) {
        let whole = super::from_qwen_ops::row_binary_map(
            format!("elementwise_{}_{}", spec.op.c_op(), spec.n),
            "elementwise",
            body.clone(),
            (&names.a, fills.0),
            (&names.b, fills.1),
            &names.c,
            cols,
            precision,
            claim.clone(),
            target,
        )?;
        if super::from_qwen_ops::fits_a_site(&whole, target) {
            return Ok(whole);
        }
    }
    let mut t = Template::new(target)?;
    let a = t.tensor(&names.a, TensorRole::Input, rows, cols, precision, init_a);
    let b = t.tensor(&names.b, TensorRole::Input, rows, cols, precision, init_b);
    let c = t.tensor(
        &names.c,
        TensorRole::Output,
        rows,
        cols,
        precision,
        TensorInit::Zero,
    );
    let (rows_t, cols_t) = (
        t.tensors[c.idx()].tiles.rows_t,
        t.tensors[c.idx()].tiles.cols_t,
    );
    let in0 = t.buffer("in0", 1, precision);
    let in1 = t.buffer("in1", 1, precision);
    let out = t.buffer("out", 1, precision);
    let la = t.load("load_a", a, tile_grid_steps(rows_t, cols_t), 1, in0);
    let lb = t.load("load_b", b, tile_grid_steps(rows_t, cols_t), 1, in1);
    let ew = t.stage(
        "elementwise",
        StageKind::Compute(ComputeOp::Generic {
            iters: space(target, target.tile.cols, None),
            ins: vec![ident_map(target.tile.cols), ident_map(target.tile.cols)],
            out: ident_map(target.tile.cols),
            // ★ AN OPERATOR BECAME A COMPOSITION. `A plus B` is no longer a named op the printer
            // recognizes — it is the general form with two identity maps and a two-operand body, and the
            // Tensix arm derives `add_tiles` from exactly that. A new pointwise operation is a new
            // `TensorBody`, and costs no print arm anywhere.
            body,
        }),
        vec![in0, in1],
        vec![out],
    );
    let st = t.store("store_c", c, tile_grid_steps(rows_t, cols_t), 1, out);
    t.edge(la, in0, ew);
    t.edge(lb, in1, ew);
    t.edge(ew, out, st);
    let acc = if precision_int_bits(precision).is_some() {
        AccPolicy::I64
    } else {
        AccPolicy::F32
    };
    let name = if rows > 1 {
        format!("elementwise_{}_{rows}x{cols}", spec.op.c_op())
    } else {
        format!("elementwise_{}_{}", spec.op.c_op(), spec.n)
    };
    t.finish(
        name,
        items_per_tile_grid(rows_t, cols_t),
        acc,
        claim,
        target,
    )
}

/// `C = A ⊕ B` where `B` is one row of `w` combined with every row of `A`. A dense block of rows is a tile grid, each
/// output tile reading its tile of `A` and the tile of `B` in its column, `B` constant down the tile's rows; a flat
/// block is one row a band per row, `B` read at the band's row start ([`super::from_qwen_ops::flat_row_broadcast`]).
#[allow(clippy::too_many_arguments)]
fn row_broadcast(
    spec: &crate::tensor_recognize::ElementwiseSpec,
    names: &ElementwiseNames,
    fills: (&Fill, &Fill),
    (rows, cols): (usize, usize),
    w: usize,
    precision: Precision,
    claim: NumericsClaim,
    body: TensorBody,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if rows == 1 || cols != w {
        return super::from_qwen_ops::flat_row_broadcast(spec.op, names, fills, spec.n, w, precision, claim, target);
    }
    let mut t = Template::new(target)?;
    let a = t.tensor(&names.a, TensorRole::Input, rows, cols, precision, init_of(fills.0, spec.n));
    let b = t.tensor(&names.b, TensorRole::Input, 1, w, precision, init_of(fills.1, w));
    let c = t.tensor(&names.c, TensorRole::Output, rows, cols, precision, TensorInit::Zero);
    let (rows_t, cols_t) = (t.tensors[c.idx()].tiles.rows_t, t.tensors[c.idx()].tiles.cols_t);
    let in0 = t.buffer("in0", 1, precision);
    let in1 = t.buffer("in1", 1, precision);
    let out = t.buffer("out", 1, precision);
    let la = t.load("load_a", a, tile_grid_steps(rows_t, cols_t), 1, in0);
    let lb = t.load("load_row", b, vec![(LV_ITEM_X, 1)], 1, in1);
    let tw = target.tile.cols;
    let ew = t.stage(
        "elementwise_row",
        StageKind::Compute(ComputeOp::Generic {
            iters: space(target, tw, None),
            ins: vec![ident_map(tw), per_col_map()],
            out: ident_map(tw),
            body,
        }),
        vec![in0, in1],
        vec![out],
    );
    let st = t.store("store_c", c, tile_grid_steps(rows_t, cols_t), 1, out);
    t.edge(la, in0, ew);
    t.edge(lb, in1, ew);
    t.edge(ew, out, st);
    let acc = if precision_int_bits(precision).is_some() { AccPolicy::I64 } else { AccPolicy::F32 };
    t.finish(format!("elementwise_{}_row_{rows}x{cols}", spec.op.c_op()), items_per_tile_grid(rows_t, cols_t), acc, claim, target)
}

/// The page WIDTH in elements a stage's operand is read through — the row stride of every flat map
/// over that buffer. A `Generic` page is a strip of `page_tiles` tiles, so the width is fixed the
/// moment the buffer is; the passes that re-page (`apply_plan`) touch matmul chains only.
pub fn page_width(page_tiles: usize, target: &SpatialTarget) -> usize {
    page_tiles * target.tile.cols
}

/// The identity map over a page `width` elements wide: the operand is walked along both axes,
/// `[(row, width), (col, 1)]` — a row step is a page width because the offset is FLAT.
pub fn ident_map(width: usize) -> ElemAccess {
    ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_ROW, width as i64), (LV_GEN_COL, 1)],
        indirect: None,
    }
}

/// A map that OMITS the column axis — one scalar per row, constant along the row. This is a
/// broadcast, and it is spelled as a missing term rather than as an operator.
pub fn per_row_map(width: usize) -> ElemAccess {
    ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_ROW, width as i64)],
        indirect: None,
    }
}

/// A map that OMITS the row axis — one value per column, constant down the rows (a weight, a bias).
pub fn per_col_map() -> ElemAccess {
    ElemAccess {
        base: 0,
        terms: vec![(LV_GEN_COL, 1)],
        indirect: None,
    }
}

/// A map constant in both axes — a scalar tile, which is what Tensix's `reduce_tile` wants as its
/// scaler operand.
pub fn const_map() -> ElemAccess {
    ElemAccess {
        base: 0,
        terms: Vec::new(),
        indirect: None,
    }
}

/// The `(rows, cols)` iteration space of a compute stage covering `cols` ELEMENTS.
///
/// ★ THE TRUE COLUMN COUNT, NEVER THE PADDED WIDTH. A tensor is stored in whole 32-wide tiles, so a
/// 1×4 row occupies a tile whose other 28 columns are zero. Walking the padded width is harmless for
/// a parallel axis — those results are cropped on the way out — and WRONG for a reduced one: a `max`
/// then folds in 28 zeros (which wins outright when every real element is negative) and a `sum` picks
/// up 28 spurious terms. `spatial_algebra_semantics_lock` caught exactly this, comparing against the
/// mathematics rather than against the compiler.
pub(crate) fn space(
    target: &SpatialTarget,
    cols: usize,
    reduce: Option<ReducePool>,
) -> Vec<IterSpec> {
    vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: target.tile.rows,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: cols.max(1),
            kind: match reduce {
                Some(pool) => IterKind::Reduce { pool },
                None => IterKind::Parallel,
            },
        },
    ]
}

pub fn from_swiglu(
    spec: &crate::tensor_recognize::SwiGluSpec,
    names: &SwiGluNames,
    fills: (&Fill, &Fill),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    from_swiglu_shaped(spec, names, fills, (1, spec.total), target)
}

/// [`from_swiglu`] over the program's `(rows, cols)` — SwiGLU is pointwise, so a matrix is mapped tile by tile in
/// the tiling the matmuls on either side of it use.
pub fn from_swiglu_shaped(
    spec: &crate::tensor_recognize::SwiGluSpec,
    names: &SwiGluNames,
    fills: (&Fill, &Fill),
    shape: (usize, usize),
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let caps = elementwise_precisions(target).ok_or(SpatialReject::MissingCompute {
        needed: "elementwise",
        target: target.name,
    })?;
    let precision = float_precision(caps).ok_or(SpatialReject::MissingCompute {
        needed: "a float elementwise datapath with exp (swiglu)",
        target: target.name,
    })?;
    let (rows, cols) = if shape.0 * shape.1 == spec.total {
        shape
    } else {
        (1, spec.total)
    };
    let mut t = Template::new(target)?;
    let g = t.tensor(
        &names.gate,
        TensorRole::Input,
        rows,
        cols,
        precision,
        init_of(fills.0, spec.total),
    );
    let u = t.tensor(
        &names.up,
        TensorRole::Input,
        rows,
        cols,
        precision,
        init_of(fills.1, spec.total),
    );
    let y = t.tensor(
        &names.y,
        TensorRole::Output,
        rows,
        cols,
        precision,
        TensorInit::Zero,
    );
    let (rows_t, cols_t) = (
        t.tensors[y.idx()].tiles.rows_t,
        t.tensors[y.idx()].tiles.cols_t,
    );
    let in0 = t.buffer("in0", 1, precision);
    let in1 = t.buffer("in1", 1, precision);
    let out = t.buffer("out", 1, precision);
    let lg = t.load("load_gate", g, tile_grid_steps(rows_t, cols_t), 1, in0);
    let lu = t.load("load_up", u, tile_grid_steps(rows_t, cols_t), 1, in1);
    // silu(gate) · up, as two compositions. S4's fusion pass folds them back into one register
    // window; expressing them apart first is what lets the fusion be CHECKED rather than assumed.
    let act = t.scratch("silu_g", 1, AccPolicy::F32.interm_precision());
    let s1 = t.stage(
        "silu",
        StageKind::Compute(ComputeOp::Generic {
            iters: space(target, target.tile.cols, None),
            ins: vec![ident_map(target.tile.cols)],
            out: ident_map(target.tile.cols),
            body: TensorBody::Unary {
                kind: SfpuKind::Silu,
                arg: Box::new(TensorBody::Operand(0)),
            },
        }),
        vec![in0],
        vec![act],
    );
    let sw = t.stage(
        "swiglu",
        StageKind::Compute(ComputeOp::Generic {
            iters: space(target, target.tile.cols, None),
            ins: vec![ident_map(target.tile.cols), ident_map(target.tile.cols)],
            out: ident_map(target.tile.cols),
            body: TensorBody::Mul(
                Box::new(TensorBody::Operand(0)),
                Box::new(TensorBody::Operand(1)),
            ),
        }),
        vec![act, in1],
        vec![out],
    );
    let st = t.store("store_y", y, tile_grid_steps(rows_t, cols_t), 1, out);
    t.edge(lg, in0, s1);
    t.edge(s1, act, sw);
    t.edge(lu, in1, sw);
    t.edge(sw, out, st);
    let name = if rows > 1 {
        format!("swiglu_{rows}x{cols}")
    } else {
        format!("swiglu_{}", spec.total)
    };
    t.finish(
        name,
        items_per_tile_grid(rows_t, cols_t),
        AccPolicy::F32,
        NumericsClaim::Approx {
            reason: format!("sigmoid in {precision:?}"),
        },
        target,
    )
}

/// A fused row op (RMSNorm / softmax / LayerNorm with optional residual, weight and bias): one work item per
/// tile-row, the whole row of tiles in one page, reduced in the register file.
/// A norm's epsilon in the `Rsqrt` transform's nano units, refused by name when it is not a whole number of them.
pub(crate) fn eps_nano(eps: crate::tensor_recognize::Epsilon) -> Result<u64, SpatialReject> {
    let nano = eps.get() * 1e9;
    let whole = nano.round();
    if whole < 1.0 || (nano - whole).abs() > 1e-6 * whole {
        return Err(SpatialReject::NotLowered("a norm epsilon that is not a whole number of nano"));
    }
    Ok(whole as u64)
}

pub fn from_fused_row(
    spec: &crate::tensor_recognize::FusedRowSpec,
    names: &FusedRowNames,
    fills: &FusedRowFills,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let caps = row_reduce_precisions(target).ok_or(SpatialReject::MissingCompute {
        needed: "a row-reduce engine (rmsnorm / softmax / layernorm)",
        target: target.name,
    })?;
    let precision = float_precision(caps).ok_or(SpatialReject::MissingCompute {
        needed: "a float row-reduce datapath",
        target: target.name,
    })?;
    if spec.weight_rows > 1 {
        return Err(SpatialReject::NotLowered(
            "a norm in lanes in its tile-row form — it broadcasts one weight row down every row (`weight_rows` 1)",
        ));
    }
    let (rows, cols) = (spec.rows, spec.cols);
    let kind = spec.reduce;
    let mut t = Template::new(target)?;
    let x = t.tensor(
        &names.x,
        TensorRole::Input,
        rows,
        cols,
        precision,
        init_of(&fills.x, rows * cols),
    );
    let residual = match (&names.residual, &fills.residual) {
        (Some(n), Some(f)) => Some(t.tensor(
            n,
            TensorRole::Input,
            rows,
            cols,
            precision,
            init_of(f, rows * cols),
        )),
        _ => None,
    };
    let weight = match (&names.weight, &fills.weight) {
        (Some(n), Some(f)) => {
            Some(t.tensor(n, TensorRole::Param, 1, cols, precision, init_of(f, cols)))
        }
        _ => None,
    };
    let bias = match (&names.bias, &fills.bias) {
        (Some(n), Some(f)) => {
            Some(t.tensor(n, TensorRole::Param, 1, cols, precision, init_of(f, cols)))
        }
        _ => None,
    };
    let y = t.tensor(
        &names.y,
        TensorRole::Output,
        rows,
        cols,
        precision,
        TensorInit::Zero,
    );
    let (rows_t, cols_t) = (
        t.tensors[y.idx()].tiles.rows_t,
        t.tensors[y.idx()].tiles.cols_t,
    );
    if cols_t > target.effective_acc_tiles(AccPolicy::F32, true) {
        return Err(SpatialReject::AccTilesExceeded {
            needed: cols_t,
            available: target.effective_acc_tiles(AccPolicy::F32, true),
            acc: AccPolicy::F32,
        });
    }
    let row_steps = vec![(LV_ITEM_Y, cols_t)];
    let xin = t.buffer("in_x", cols_t, precision);
    let lx = t.load("load_x", x, row_steps.clone(), cols_t, xin);
    let mut loads = vec![(lx, xin)];
    let res_in = residual.map(|r| {
        let b = t.buffer("in_res", cols_t, precision);
        let l = t.load("load_res", r, row_steps.clone(), cols_t, b);
        loads.push((l, b));
        b
    });
    let w_in = weight.map(|w| {
        let b = t.buffer("in_w", cols_t, precision);
        let l = t.load("load_w", w, vec![], cols_t, b);
        loads.push((l, b));
        b
    });
    let b_in = bias.map(|bt| {
        let b = t.buffer("in_b", cols_t, precision);
        let l = t.load("load_b", bt, vec![], cols_t, b);
        loads.push((l, b));
        b
    });
    let out = t.buffer("out", cols_t, precision);
    let wide = page_width(cols_t, target);
    let one = page_width(1, target);
    // ── The normalization, as the pipeline it actually is ──
    //
    // ★ NOT ONE OP. A reduce engine reads circular buffers, never the destination registers, so the
    // row's sum has to be PACKED OUT before it can be broadcast back across the row — and a pack
    // closes the register window. A single fused op would therefore have to smuggle three
    // acquire/pack cycles inside one, fighting the brackets the choreography puts around it. Split
    // into stages, each one fits its window honestly and the choreography synthesizes the handshakes
    // it already knows how to synthesize. The square and the residual add are not new primitives
    // either: they are the elementwise op that already exists.
    // ★ SOFTMAX AND LAYERNORM ARE COMPOSITIONS, NOT OPERATORS — AND ADDING THEM TOUCHED NO BACKEND.
    //
    // This is the property the whole campaign exists to demonstrate, and
    // `spatial_algebra_lock::a_new_workload_touches_no_backend` is what holds it: both workloads
    // below are new sequences of `ComputeOp::Generic`, and not one file under `spatial/lower/` gained
    // an arm for either of them. A reduction is an ITERATOR KIND, a broadcast is an ABSENT TERM in an
    // access map, and a transform is an `SfpuKind` — so the printers already knew how to emit every
    // instruction these need, without knowing the words "softmax" or "layernorm".
    if kind != RowReduceKind::RmsNorm {
        let acc_path = AccPolicy::F32.interm_precision();
        let mut chain: Vec<StageId> = Vec::new();
        let scaler_tile = target.tile.rows * target.tile.cols;
        // The reduce engine's scaler operand, at 1.0 — every divide in these chains happens ONCE,
        // explicitly, rather than being smeared across the reduction's terms. See
        // `SfpuKind::Rsqrt { over }` for what that mistake cost the first time.
        let ones = t.tensor(
            "row_scaler",
            TensorRole::Param,
            target.tile.rows,
            target.tile.cols,
            precision,
            TensorInit::FloatValues(vec![1.0f64.to_bits(); scaler_tile]),
        );
        let ones_in = t.buffer("in_scaler", 1, precision);
        let l_ones = t.load("load_scaler", ones, vec![], 1, ones_in);
        loads.push((l_ones, ones_in));

        let mk = |t: &mut Template,
                      name: &str,
                      iters: Vec<IterSpec>,
                      ins: Vec<BufferId>,
                      maps: Vec<ElemAccess>,
                      omap: ElemAccess,
                      body: TensorBody,
                      dst: BufferId|
         -> StageId {
            t.stage(
                name,
                StageKind::Compute(ComputeOp::Generic {
                    iters,
                    ins: maps,
                    out: omap,
                    body,
                }),
                ins,
                vec![dst],
            )
        };
        let out_stage;
        if kind == RowReduceKind::Softmax {
            // exp(x - max) / Σ exp(x - max)
            let rmax = t.scratch("row_max", 1, acc_path);
            chain.push(mk(
                &mut t,
                "row_max",
                space(target, cols, Some(ReducePool::Max)),
                vec![xin, ones_in],
                vec![ident_map(wide), const_map()],
                per_row_map(one),
                TensorBody::Operand(0),
                rmax,
            ));
            let cx = t.scratch("centred", cols_t, acc_path);
            chain.push(mk(
                &mut t,
                "centre",
                space(target, cols, None),
                vec![xin, rmax],
                vec![ident_map(wide), per_row_map(one)],
                ident_map(wide),
                TensorBody::Sub(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Operand(1)),
                ),
                cx,
            ));
            let ex = t.scratch("expx", cols_t, acc_path);
            chain.push(mk(
                &mut t,
                "exp",
                space(target, cols, None),
                vec![cx],
                vec![ident_map(wide)],
                ident_map(wide),
                TensorBody::Unary {
                    kind: SfpuKind::Exp,
                    arg: Box::new(TensorBody::Operand(0)),
                },
                ex,
            ));
            let rsum = t.scratch("row_sum", 1, acc_path);
            chain.push(mk(
                &mut t,
                "row_sum",
                space(target, cols, Some(ReducePool::Sum)),
                vec![ex, ones_in],
                vec![ident_map(wide), const_map()],
                per_row_map(one),
                TensorBody::Operand(0),
                rsum,
            ));
            let inv = t.scratch("row_inv", 1, acc_path);
            chain.push(mk(
                &mut t,
                "recip",
                space(target, target.tile.cols, None),
                vec![rsum],
                vec![per_row_map(one)],
                per_row_map(one),
                TensorBody::Unary {
                    kind: SfpuKind::Recip,
                    arg: Box::new(TensorBody::Operand(0)),
                },
                inv,
            ));
            out_stage = mk(
                &mut t,
                "scale",
                space(target, cols, None),
                vec![ex, inv],
                vec![ident_map(wide), per_row_map(one)],
                ident_map(wide),
                TensorBody::Mul(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Operand(1)),
                ),
                out,
            );
            chain.push(out_stage);
        } else {
            // (x - mean)·rsqrt(var + eps), then the per-column weight and bias.
            let inv_cols = (1.0f64 / cols as f64).to_bits();
            let sum = t.scratch("row_sum", 1, acc_path);
            chain.push(mk(
                &mut t,
                "row_sum",
                space(target, cols, Some(ReducePool::Sum)),
                vec![xin, ones_in],
                vec![ident_map(wide), const_map()],
                per_row_map(one),
                TensorBody::Operand(0),
                sum,
            ));
            let mean = t.scratch("row_mean", 1, acc_path);
            chain.push(mk(
                &mut t,
                "mean",
                space(target, target.tile.cols, None),
                vec![sum],
                vec![per_row_map(one)],
                per_row_map(one),
                TensorBody::Mul(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Const(inv_cols)),
                ),
                mean,
            ));
            let dev = t.scratch("dev", cols_t, acc_path);
            chain.push(mk(
                &mut t,
                "centre",
                space(target, cols, None),
                vec![xin, mean],
                vec![ident_map(wide), per_row_map(one)],
                ident_map(wide),
                TensorBody::Sub(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Operand(1)),
                ),
                dev,
            ));
            let dsq = t.scratch("dev_sq", cols_t, acc_path);
            chain.push(mk(
                &mut t,
                "square",
                space(target, cols, None),
                vec![dev, dev],
                vec![ident_map(wide), ident_map(wide)],
                ident_map(wide),
                TensorBody::Mul(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Operand(1)),
                ),
                dsq,
            ));
            let vsum = t.scratch("var_sum", 1, acc_path);
            chain.push(mk(
                &mut t,
                "var",
                space(target, cols, Some(ReducePool::Sum)),
                vec![dsq, ones_in],
                vec![ident_map(wide), const_map()],
                per_row_map(one),
                TensorBody::Operand(0),
                vsum,
            ));
            let inv = t.scratch("row_inv", 1, acc_path);
            chain.push(mk(
                &mut t,
                "rsqrt",
                space(target, target.tile.cols, None),
                vec![vsum],
                vec![per_row_map(one)],
                per_row_map(one),
                TensorBody::Unary {
                    kind: SfpuKind::Rsqrt {
                        eps_nano: eps_nano(spec.eps)?,
                        over: cols,
                    },
                    arg: Box::new(TensorBody::Operand(0)),
                },
                inv,
            ));
            // Normalize, then the per-column weight and bias — each a row-broadcast, each an access
            // map with the ROW axis omitted.
            let mut prev = dev;
            let mut steps: Vec<(&str, BufferId, ElemAccess, TensorBody)> = vec![(
                "normalize",
                inv,
                per_row_map(one),
                TensorBody::Mul(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Operand(1)),
                ),
            )];
            if let Some(w) = w_in {
                steps.push((
                    "weight",
                    w,
                    per_col_map(),
                    TensorBody::Mul(
                        Box::new(TensorBody::Operand(0)),
                        Box::new(TensorBody::Operand(1)),
                    ),
                ));
            }
            if let Some(b) = b_in {
                steps.push((
                    "bias",
                    b,
                    per_col_map(),
                    TensorBody::Add(
                        Box::new(TensorBody::Operand(0)),
                        Box::new(TensorBody::Operand(1)),
                    ),
                ));
            }
            let last = steps.len() - 1;
            let mut tail = None;
            for (i, (name, second, m, body)) in steps.into_iter().enumerate() {
                let dst = if i == last {
                    out
                } else {
                    t.scratch(&format!("{name}_out"), cols_t, acc_path)
                };
                let sid = mk(
                    &mut t,
                    name,
                    space(target, cols, None),
                    vec![prev, second],
                    vec![ident_map(wide), m],
                    ident_map(wide),
                    body,
                    dst,
                );
                chain.push(sid);
                prev = dst;
                tail = Some(sid);
            }
            out_stage = tail.expect("the layernorm chain ends in the stage that writes the output");
        }
        let st = t.store("store_y", y, row_steps, cols_t, out);
        for (l, b) in loads {
            let consumer = *chain
                .iter()
                .find(|c| t.stages[c.idx()].ins.contains(&b))
                .expect("every load feeds a chain stage");
            t.edge(l, b, consumer);
        }
        for pair in chain.windows(2) {
            let b = t.stages[pair[0].idx()].outs[0];
            if t.stages[pair[1].idx()].ins.contains(&b) {
                t.edge(pair[0], b, pair[1]);
            } else {
                let later = *chain
                    .iter()
                    .find(|c| t.stages[c.idx()].ins.contains(&b))
                    .expect("every chain output is read");
                t.edge(pair[0], b, later);
            }
        }
        t.edge(out_stage, out, st);
        let items: Vec<WorkItem> = (0..rows_t)
            .map(|oy| WorkItem {
                id: WorkItemId::new(oy),
                coord: (oy, 0),
                out: TileRect {
                    row0_t: oy,
                    col0_t: 0,
                    h_t: 1,
                    w_t: cols_t,
                },
            })
            .collect();
        return t.finish(
            format!("{}_{}x{}", kind.name(), rows, cols),
            items,
            AccPolicy::F32,
            NumericsClaim::Approx {
                reason: format!("row reduction in {precision:?}"),
            },
            target,
        );
    }
    // ★ THE INTERMEDIATES CARRY THE ACCUMULATOR'S PRECISION, NOT THE OPERAND'S.
    //
    // A fused op rounds once, at its output. The same arithmetic split into a chain rounds at EVERY
    // stage boundary, because each stage packs its result into a buffer and the next unpacks it. Left
    // at the operand precision that is a real numeric change: on the RDU, whose witnessed datapath is
    // bf16, the legalized rmsnorm drifted a full bf16 ulp from the VM and
    // `spatial_front_end_lock::the_row_reduce_and_swiglu_sugars_compile_where_provided…` caught it
    // (230469 vs 231445). Legalization must not be able to change what a program computes — that is
    // the whole premise the campaign rests on — so the chain keeps fp32 between stages and rounds
    // only where the fused form would have, at the output.
    //
    // This is the same rule `validate_compute` already enforces for a matmul's spill buffer
    // (`IrViolation::IntermNarrowerThanAcc`): an intermediate is part of the accumulation, not part
    // of the operand set.
    // ★ WIDTH ON THE ACCUMULATION PATH ONLY — AND BOTH HALVES ARE LOAD-BEARING, TESTED SEPARATELY.
    //
    // rmsnorm drifted a full bf16 ulp from the VM on the RDU once it became a chain. Two things were
    // wrong and NEITHER fix works alone — removing either brings the failure straight back:
    //   1. scale-per-term vs scale-once (now carried by `SfpuKind::Rsqrt { over }`), and
    //   2. the reduction's own path narrowing at each stage boundary.
    //
    // But (2) is only needed where the ACCUMULATION lives: the squares that feed the reduce, its
    // result, and the transform of that result. The elementwise-shaped links — a fused residual add,
    // the output of a broadcast rescale — round exactly where the fused form would have rounded
    // anyway, so widening them buys nothing and doubles the bytes crossing every one of those
    // boundaries. This is the same rule `validate_compute` already applies to a matmul's spill
    // (`IrViolation::IntermNarrowerThanAcc`): an intermediate is part of the accumulation, or it is
    // not, and only the former earns the accumulator's width.
    let acc_path = AccPolicy::F32.interm_precision();
    let interm = precision;
    let mut chain: Vec<StageId> = Vec::new();
    let mut src = xin;
    if let Some(r) = res_in {
        let xr = t.scratch("xr", cols_t, interm);
        chain.push(t.stage(
            "add_residual",
            StageKind::Compute(ComputeOp::Generic {
                iters: space(target, cols, None),
                ins: vec![ident_map(wide), ident_map(wide)],
                out: ident_map(wide),
                body: TensorBody::Add(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Operand(1)),
                ),
            }),
            vec![xin, r],
            vec![xr],
        ));
        src = xr;
    }
    let xsq = t.scratch("xsq", cols_t, acc_path);
    chain.push(t.stage(
        "square",
        StageKind::Compute(ComputeOp::Generic {
            iters: space(target, cols, None),
            ins: vec![ident_map(wide), ident_map(wide)],
            out: ident_map(wide),
            body: TensorBody::Mul(
                Box::new(TensorBody::Operand(0)),
                Box::new(TensorBody::Operand(1)),
            ),
        }),
        vec![src, src],
        vec![xsq],
    ));
    // The reduction's scaler carries `1/cols`, so the reduce YIELDS the mean and no later divide can
    // fall out of step with the column count. It is one whole tile of the same constant, which is
    // both what the engine broadcasts and what the evaluator reads at (0,0).
    let tile_elems = target.tile.rows * target.tile.cols;
    // 1.0, not 1/cols: the reduce engine multiplies EVERY term by its scaler, and the specification
    // scales once at the end. `RowXformKind::Rsqrt` now carries the divide. See its doc comment.
    let recip = 1.0f64;
    let ones = t.tensor(
        "rms_scaler",
        TensorRole::Param,
        target.tile.rows,
        target.tile.cols,
        precision,
        TensorInit::FloatValues(vec![recip.to_bits(); tile_elems]),
    );
    let ones_in = t.buffer("in_scaler", 1, precision);
    let l_ones = t.load("load_scaler", ones, vec![], 1, ones_in);
    loads.push((l_ones, ones_in));
    let ssum = t.scratch("row_mean", 1, acc_path);
    // The scaler is named twice on purpose: the op carries it so the printer knows WHICH operand is
    // the scaler, and the stage lists it so it is a real port with a real edge. Carrying it only
    // inside the op leaves the load's edge pointing at a buffer the stage does not declare.
    let reduce = t.stage(
        "reduce_rows",
        StageKind::Compute(ComputeOp::Generic {
            // The column axis is FOLDED — that is the whole of "this is a reduction". The scaler rides
            // as the last operand because Tensix's `reduce_tile` takes one; its map is constant in both
            // axes, which is how the printer recognizes it as the scaler rather than as data.
            iters: space(target, cols, Some(ReducePool::Sum)),
            ins: vec![ident_map(wide), const_map()],
            out: per_row_map(one),
            body: TensorBody::Operand(0),
        }),
        vec![xsq, ones_in],
        vec![ssum],
    );
    chain.push(reduce);
    let scale = t.scratch("row_scale", 1, acc_path);
    chain.push(t.stage(
        "rsqrt",
        StageKind::Compute(ComputeOp::Generic {
            iters: space(target, target.tile.cols, None),
            ins: vec![per_row_map(one)],
            out: per_row_map(one),
            body: TensorBody::Unary {
                kind: SfpuKind::Rsqrt {
                    eps_nano: eps_nano(spec.eps)?,
                    over: cols,
                },
                arg: Box::new(TensorBody::Operand(0)),
            },
        }),
        vec![ssum],
        vec![scale],
    ));
    // Normalize, then the per-column weight, then the per-column bias. Each is its own stage for the
    // same reason the reduce is: its second operand is read from a circular buffer, so the previous
    // result has to be packed out before it can be used. Only the last one writes the output buffer.
    let mut steps: Vec<(&str, ComputeOp, BufferId)> = vec![(
        "rescale",
        ComputeOp::Generic {
            // The scale is one value per ROW: its map omits the column axis. That absence IS the
            // broadcast, and it is what the Tensix arm turns into `mul_tiles_bcast_cols`.
            iters: space(target, cols, None),
            ins: vec![ident_map(wide), per_row_map(one)],
            out: ident_map(wide),
            body: TensorBody::Mul(
                Box::new(TensorBody::Operand(0)),
                Box::new(TensorBody::Operand(1)),
            ),
        },
        scale,
    )];
    if let Some(w) = w_in {
        steps.push((
            "weight",
            ComputeOp::Generic {
                // A per-COLUMN weight: its map omits the ROW axis — `mul_tiles_bcast_rows`.
                iters: space(target, cols, None),
                ins: vec![ident_map(wide), per_col_map()],
                out: ident_map(wide),
                body: TensorBody::Mul(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Operand(1)),
                ),
            },
            w,
        ));
    }
    if let Some(b) = b_in {
        steps.push((
            "bias",
            ComputeOp::Generic {
                iters: space(target, cols, None),
                ins: vec![ident_map(wide), per_col_map()],
                out: ident_map(wide),
                body: TensorBody::Add(
                    Box::new(TensorBody::Operand(0)),
                    Box::new(TensorBody::Operand(1)),
                ),
            },
            b,
        ));
    }
    let last = steps.len() - 1;
    let mut prev = src;
    for (i, (name, op, second)) in steps.into_iter().enumerate() {
        let dst = if i == last {
            out
        } else {
            t.scratch(&format!("{name}_out"), cols_t, interm)
        };
        chain.push(t.stage(name, StageKind::Compute(op), vec![prev, second], vec![dst]));
        prev = dst;
    }
    let rescale = *chain
        .last()
        .expect("the chain ends in the stage that writes the output");
    let st = t.store("store_y", y, row_steps, cols_t, out);
    // Every load feeds the first stage that reads it; the chain's own edges carry the rest.
    for (l, b) in loads {
        let consumer = *chain
            .iter()
            .find(|c| t.stages[c.idx()].ins.contains(&b))
            .expect("every load feeds a chain stage");
        t.edge(l, b, consumer);
    }
    t.edge(chain[0], t.stages[chain[0].idx()].outs[0], chain[1]);
    for pair in chain.windows(2).skip(1) {
        let b = t.stages[pair[0].idx()].outs[0];
        t.edge(pair[0], b, pair[1]);
    }
    t.edge(rescale, out, st);
    let items: Vec<WorkItem> = (0..rows_t)
        .map(|oy| WorkItem {
            id: WorkItemId::new(oy),
            coord: (oy, 0),
            out: TileRect {
                row0_t: oy,
                col0_t: 0,
                h_t: 1,
                w_t: cols_t,
            },
        })
        .collect();
    t.finish(
        format!("{}_{}x{}", kind.name(), rows, cols),
        items,
        AccPolicy::F32,
        NumericsClaim::Approx {
            reason: format!("row reduction in {precision:?}"),
        },
        target,
    )
}

/// The largest integer magnitude a precision represents exactly.
fn exact_int_bound(p: Precision) -> i64 {
    match p {
        Precision::Int8 => 127,
        Precision::Int16 => 32767,
        Precision::Int32 => i32::MAX as i64,
        Precision::Int64 => i64::MAX,
        Precision::Fp8 => 15,
        Precision::Bf16 => 256,
        Precision::Fp16 => 2048,
        Precision::Fp32 => 1 << 24,
    }
}

/// The chosen numerics of a matmul on a target, from each operand's largest magnitude: `(operand, accumulator, output,
/// claim)`. An operand bounded by `b` lies in `[−b, b]` ([`choose_matmul_numerics_ranged`]).
pub fn choose_matmul_numerics(
    target: &SpatialTarget,
    bound_a: Option<i64>,
    bound_b: Option<i64>,
    k: usize,
) -> Result<(Precision, AccPolicy, Precision, NumericsClaim), SpatialReject> {
    let symmetric = |b: Option<i64>| b.map(|b| (-b, b));
    choose_matmul_numerics_ranged(target, symmetric(bound_a), symmetric(bound_b), k)
}

/// The narrowest integer precision whose two's-complement range holds `[lo, hi]`.
fn int_holds(p: Precision, (lo, hi): (i64, i64)) -> bool {
    match crate::spatial::target::precision_int_bits(p) {
        Some(bits) if bits >= 64 => true,
        Some(bits) => lo >= -(1i64 << (bits - 1)) && hi < (1i64 << (bits - 1)),
        None => false,
    }
}

/// The chosen numerics of a matmul on a target, from each operand's range `[lo, hi]`: `(operand, accumulator, output,
/// claim)`. An integer operand is the narrowest two's-complement width that holds both ranges — an int8 activation
/// clamped to `[−128, 127]` is int8 — and the claim is exact while `k` products of the operands' extremes fit the
/// accumulator.
pub fn choose_matmul_numerics_ranged(
    target: &SpatialTarget,
    range_a: Option<(i64, i64)>,
    range_b: Option<(i64, i64)>,
    k: usize,
) -> Result<(Precision, AccPolicy, Precision, NumericsClaim), SpatialReject> {
    let magnitude = |(lo, hi): (i64, i64)| (lo as i128).abs().max((hi as i128).abs());
    // The largest magnitude of a partial sum, from the fact every chip's accumulator is judged by; a sum past i128 is no
    // bound.
    let product_bound: Option<i128> = match (range_a, range_b) {
        (Some(a), Some(b)) => {
            let (lo, hi) = crate::tensor_graph::facts::product_sums(a, b, k);
            (lo > i128::MIN && hi < i128::MAX).then(|| lo.unsigned_abs().max(hi.unsigned_abs()) as i128)
        }
        _ => None,
    };
    // Integer path first: the narrowest integer operand that holds the ranges exactly.
    for p in [
        Precision::Int8,
        Precision::Int16,
        Precision::Int32,
        Precision::Int64,
    ] {
        if !target.matmul_supports(p) {
            continue;
        }
        let fits = match (range_a, range_b) {
            (Some(a), Some(b)) => int_holds(p, a) && int_holds(p, b),
            _ => p == Precision::Int64,
        };
        if !fits {
            continue;
        }
        let acc = if target.matmul_acc_int_bits() >= 64 {
            AccPolicy::I64
        } else {
            AccPolicy::I32
        };
        let limit: i128 = 1i128 << (acc.bits() - 1);
        let claim = match product_bound {
            Some(pb) if pb < limit => NumericsClaim::Exact,
            Some(_) => NumericsClaim::Approx {
                reason: format!("partial sums may exceed the {}-bit accumulator", acc.bits()),
            },
            None => NumericsClaim::Approx {
                reason: "operand range unknown (external data)".into(),
            },
        };
        // The result is what the accumulator holds: an INT32 accumulator packs INT32, never a widened copy the
        // part has no type for.
        let out =
            crate::spatial::target::int_precision_of_bits(acc.bits()).unwrap_or(Precision::Int64);
        return Ok((p, acc, out, claim));
    }
    // Float engine: the narrowest float operand the datapath takes — bf16, then fp16, then fp32 on a datapath with
    // nothing narrower — with an fp32 accumulator; exact only inside the mantissa.
    for p in [Precision::Bf16, Precision::Fp16, Precision::Fp32] {
        if !target.matmul_supports(p) {
            continue;
        }
        if !target.matmul_fp32_acc() {
            return Err(SpatialReject::MissingCompute {
                needed: "fp32 accumulate",
                target: target.name,
            });
        }
        let exact = exact_int_bound(p) as i128;
        let claim = match (range_a.map(magnitude), range_b.map(magnitude), product_bound) {
            (Some(a), Some(b), Some(pb)) if a <= exact && b <= exact && pb < (1i128 << 24) => NumericsClaim::Exact,
            (Some(a), Some(b), _) if a > exact || b > exact => NumericsClaim::Approx {
                reason: format!("operands exceed the exact integer range of {p:?}"),
            },
            (Some(_), Some(_), _) => NumericsClaim::Approx {
                reason: "partial sums exceed fp32's 24-bit mantissa".into(),
            },
            _ => NumericsClaim::Approx {
                reason: "operand range unknown (external data)".into(),
            },
        };
        return Ok((p, AccPolicy::F32, Precision::Fp32, claim));
    }
    Err(SpatialReject::MissingCompute {
        needed: "tile matmul",
        target: target.name,
    })
}

/// Lift a recognized matmul to a template spatial program on `target`.
pub fn from_matmul(
    spec: &MatmulSpec,
    names: &MatmulNames,
    fills: (&Fill, &Fill),
    ranges: (Option<(i64, i64)>, Option<(i64, i64)>),
    numerics: NumericsPolicy,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let tile = target.tile;
    let init_a = init_of(fills.0, spec.m * spec.k);
    let init_b = init_of(fills.1, spec.k * spec.n);
    let (operand, acc, mut out_dtype, mut claim) = choose_matmul_numerics_ranged(
        target,
        ranges.0.or(init_a.range()),
        ranges.1.or(init_b.range()),
        spec.k,
    )?;
    // A program that accepts spill rounding packs its partials and its output in the operand's width.
    if numerics == NumericsPolicy::AllowSpillRounding && acc == AccPolicy::F32 {
        out_dtype = operand;
        claim = NumericsClaim::Approx {
            reason: "spill and output rounding to the operand width accepted".into(),
        };
    }
    let dataflow =
        target
            .matmul_dataflows()
            .first()
            .copied()
            .ok_or(SpatialReject::MissingCompute {
                needed: "tile matmul",
                target: target.name,
            })?;
    let local = Handshake::Fifo;

    let grid = |rows: usize, cols: usize| TileGrid {
        rows_t: rows.div_ceil(tile.rows),
        cols_t: cols.div_ceil(tile.cols),
    };
    let tensors = vec![
        TensorDecl {
            id: TensorId(0),
            name: names.a.clone(),
            role: TensorRole::Input,
            elems: ElemShape {
                rows: spec.m,
                cols: spec.k,
            },
            tiles: grid(spec.m, spec.k),
            dtype: operand,
            layout: Layout::RowMajorTiles,
            space: TensorSpace::Dram,
            init: init_a,
            range: ranges.0,
            chip: None,
        },
        TensorDecl {
            id: TensorId(1),
            name: names.b.clone(),
            role: TensorRole::Param,
            elems: ElemShape {
                rows: spec.k,
                cols: spec.n,
            },
            tiles: grid(spec.k, spec.n),
            dtype: operand,
            layout: Layout::RowMajorTiles,
            space: TensorSpace::Dram,
            init: init_b,
            range: ranges.1,
            chip: None,
        },
        TensorDecl {
            id: TensorId(2),
            name: names.c.clone(),
            role: TensorRole::Output,
            elems: ElemShape {
                rows: spec.m,
                cols: spec.n,
            },
            tiles: grid(spec.m, spec.n),
            dtype: out_dtype,
            layout: Layout::RowMajorTiles,
            space: TensorSpace::Dram,
            init: TensorInit::Zero,
            range: None,
            chip: None,
        },
    ];
    let (mt, nt, kt) = (
        tensors[2].tiles.rows_t,
        tensors[2].tiles.cols_t,
        tensors[0].tiles.cols_t,
    );

    let buffers = vec![
        Buffer {
            id: BufferId(0),
            name: "in0".into(),
            kind: BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: 1,
            },
            dtype: operand,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        },
        Buffer {
            id: BufferId(1),
            name: "in1".into(),
            kind: BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: 1,
            },
            dtype: operand,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        },
        Buffer {
            id: BufferId(2),
            name: "out".into(),
            kind: BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: 1,
            },
            dtype: out_dtype,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        },
    ];

    let stages = vec![
        Stage {
            id: StageId(0),
            name: "load_a".into(),
            kind: StageKind::Load {
                tensor: TensorId(0),
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![(LV_ITEM_Y, kt), (LV_KBLOCK, 1)],
                        indirect: None,
                    },
                    block_h: 1,
                    block_w: 1,
                    stride_w: 1,
                    stride_h: kt,
                },
                num_blocks: kt,
                multicast: None,
            },
            role: SiteRole::Movement,
            ins: vec![],
            outs: vec![BufferId(0)],
            site: None,
            item: None,
            origin: None,
        },
        Stage {
            id: StageId(1),
            name: "load_b".into(),
            kind: StageKind::Load {
                tensor: TensorId(1),
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![(LV_ITEM_X, 1), (LV_KBLOCK, nt)],
                        indirect: None,
                    },
                    block_h: 1,
                    block_w: 1,
                    stride_w: 1,
                    stride_h: nt,
                },
                num_blocks: kt,
                multicast: None,
            },
            role: SiteRole::Movement,
            ins: vec![],
            outs: vec![BufferId(1)],
            site: None,
            item: None,
            origin: None,
        },
        Stage {
            id: StageId(2),
            name: "mm".into(),
            kind: StageKind::Compute(ComputeOp::MatmulTiles {
                block: BlockShape {
                    osh: 1,
                    osw: 1,
                    ibw: 1,
                },
                sub_blocks: (1, 1),
                num_blocks: kt,
                operand,
                acc,
                dataflow,
                interm: None,
                granularity: Granularity::PerTile,
            }),
            role: SiteRole::Compute,
            ins: vec![BufferId(0), BufferId(1)],
            outs: vec![BufferId(2)],
            site: None,
            item: None,
            origin: None,
        },
        Stage {
            id: StageId(3),
            name: "store_c".into(),
            kind: StageKind::Store {
                tensor: TensorId(2),
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![(LV_ITEM_Y, nt), (LV_ITEM_X, 1)],
                        indirect: None,
                    },
                    block_h: 1,
                    block_w: 1,
                    stride_w: 1,
                    stride_h: nt,
                },
                sub_blocks: (1, 1),
            },
            role: SiteRole::Movement,
            ins: vec![BufferId(2)],
            outs: vec![],
            site: None,
            item: None,
            origin: None,
        },
    ];
    let edges = vec![
        Edge {
            id: EdgeId(0),
            from: StageId(0),
            buffer: BufferId(0),
            to: vec![StageId(2)],
            handshake: local.clone(),
        },
        Edge {
            id: EdgeId(1),
            from: StageId(1),
            buffer: BufferId(1),
            to: vec![StageId(2)],
            handshake: local.clone(),
        },
        Edge {
            id: EdgeId(2),
            from: StageId(2),
            buffer: BufferId(2),
            to: vec![StageId(3)],
            handshake: local.clone(),
        },
    ];
    let mut items = Vec::with_capacity(mt * nt);
    for oy in 0..mt {
        for ox in 0..nt {
            items.push(WorkItem {
                id: WorkItemId::new(items.len()),
                coord: (oy, ox),
                out: TileRect {
                    row0_t: oy,
                    col0_t: ox,
                    h_t: 1,
                    w_t: 1,
                },
            });
        }
    }
    let mut requires = Requirements::default();
    requires.handshakes.insert(local.kind());
    let p = SpatialProgram {
        name: format!("matmul_{}x{}x{}", spec.m, spec.n, spec.k),
        tile,
        tensors,
        stages,
        buffers,
        edges,
        semaphores: Vec::new(),
        sections: Vec::new(),
        trips: Vec::new(),
        links: Vec::new(),
        work: WorkList {
            items,
            k_tiles: kt,
            plan: None,
            k_trip: None,
        },
        numerics,
        claim,
        acc_tiles_used: target.effective_acc_tiles(acc, numerics == NumericsPolicy::Exact),
        mem_banks: target.site.mem_banks,
        requires,
        placement: None,
        memory: None,
        schedule: None,
        certificate: None,
    };
    p.validate()?;
    Ok(p)
}

/// Operand bit width for the integer reference (`None` for float operands).
pub fn operand_bits(p: Precision) -> Option<u32> {
    precision_int_bits(p)
}

/// The same-site hand-off primitive a target offers: a FIFO where it has one, else valid/ready.
/// The handshake a target provides for a same-site bounded hand-off. A template always says `Fifo` (the
/// abstract bounded buffer between a producer and its consumer); `synthesize_handshakes` retypes those edges
/// to this kind, so the template is the same value on every target and the target enters only through passes.
pub fn local_handshake(target: &SpatialTarget) -> Result<Handshake, SpatialReject> {
    use crate::spatial::target::HandshakeKind;
    if target.provides_handshake(HandshakeKind::Fifo) {
        Ok(Handshake::Fifo)
    } else if target.provides_handshake(HandshakeKind::ValidReady) {
        Ok(Handshake::ValidReady)
    } else if target.provides_handshake(HandshakeKind::Token) {
        Ok(Handshake::Token)
    } else {
        Err(SpatialReject::MissingHandshake {
            needed: HandshakeKind::Fifo,
            target: target.name,
        })
    }
}

/// A weight-stationary systolic tile as a spatial program: `O(m×n) = A(m×k)·W(k×n)`, the weights baked into
/// a `Param` tensor, the compute stage carrying the weight-stationary policy.
pub fn from_systolic(
    sys: &crate::codegen_sva::systolic::Systolic,
    m: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let spec = MatmulSpec {
        m,
        n: sys.n,
        k: sys.k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let names = MatmulNames {
        a: "a".into(),
        b: "w".into(),
        c: "o".into(),
    };
    let weights = sys.weights.iter().fold(None, |h: Option<(i64, i64)>, w| Some(h.map_or((*w, *w), |(lo, hi)| (lo.min(*w), hi.max(*w)))));
    let mut p = from_matmul(
        &spec,
        &names,
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-128, 127)), Some(weights.unwrap_or((0, 0)))),
        NumericsPolicy::Exact,
        target,
    )?;
    p.tensors[1].init = TensorInit::Values(sys.weights.clone());
    use crate::spatial::target::DataflowPolicy;
    let offered = target.matmul_dataflows();
    let policy = if offered.contains(&DataflowPolicy::WeightStationary) {
        DataflowPolicy::WeightStationary
    } else if offered.contains(&DataflowPolicy::OutputStationary) {
        DataflowPolicy::OutputStationary
    } else {
        return Err(SpatialReject::MissingCompute {
            needed: "a systolic dataflow (weight- or output-stationary)",
            target: target.name,
        });
    };
    for s in p.stages.iter_mut() {
        if let StageKind::Compute(ComputeOp::MatmulTiles { dataflow, .. }) = &mut s.kind {
            *dataflow = policy;
        }
    }
    p.name = format!("systolic_{}x{}_m{m}", sys.k, sys.n);
    p.validate()?;
    Ok(p)
}

/// A fused transformer block (matmul → requant → bias → relu) as ONE fused compute chain inside the
/// scalar datapath's fusion window, intermediates streaming through local hand-offs.
pub fn from_fused_block(
    fb: &crate::codegen_sva::systolic::FusedBlock,
    m: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let window = target.scalar_target().map(|t| t.fusion_window).unwrap_or(1);
    if window < 4 {
        return Err(SpatialReject::MissingCompute {
            needed: "fusion window of 4 (matmul→requant→bias→relu)",
            target: target.name,
        });
    }
    let mut p = from_systolic(&fb.sys, m, target)?;
    let local = Handshake::Fifo;
    let out_dtype = p.tensors[2].dtype;
    // Bias tensor: one row of n, a Param.
    let bias_id = TensorId::new(p.tensors.len());
    let n = fb.sys.n;
    p.tensors.push(TensorDecl {
        id: bias_id,
        name: "bias".into(),
        role: TensorRole::Param,
        elems: ElemShape { rows: 1, cols: n },
        tiles: TileGrid {
            rows_t: 1,
            cols_t: n.div_ceil(p.tile.cols),
        },
        dtype: out_dtype,
        layout: Layout::RowMajorTiles,
        space: TensorSpace::Dram,
        init: TensorInit::Values(fb.bias.clone()),
        range: fb.bias.iter().fold(None, |h: Option<(i64, i64)>, b| Some(h.map_or((*b, *b), |(lo, hi)| (lo.min(*b), hi.max(*b))))),
        chip: None,
    });
    // Rewire: mm → rq → bias(+load) → relu → store.
    let store = p
        .stages
        .iter()
        .position(|s| matches!(s.kind, StageKind::Store { .. }))
        .unwrap();
    let mm_out = p
        .stages
        .iter()
        .find(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })))
        .unwrap()
        .outs[0];
    let new_buf = |p: &mut SpatialProgram, name: &str| {
        let id = BufferId::new(p.buffers.len());
        p.buffers.push(Buffer {
            id,
            name: name.into(),
            kind: BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: 1,
            },
            dtype: out_dtype,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        });
        id
    };
    let rq_out = new_buf(&mut p, "rq");
    let bias_in = new_buf(&mut p, "bias_in");
    let bias_out = new_buf(&mut p, "biased");
    let relu_out = new_buf(&mut p, "relu");
    let mm_id = p
        .stages
        .iter()
        .find(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })))
        .unwrap()
        .id;
    let nt = p.tensors[2].tiles.cols_t;
    let push_stage = |p: &mut SpatialProgram,
                      name: &str,
                      kind: StageKind,
                      role: SiteRole,
                      ins: Vec<BufferId>,
                      outs: Vec<BufferId>|
     -> StageId {
        let id = StageId::new(p.stages.len());
        p.stages.push(Stage {
            id,
            name: name.into(),
            kind,
            role,
            ins,
            outs,
            site: None,
            item: None,
            origin: None,
        });
        id
    };
    let rq = push_stage(
        &mut p,
        "requant",
        StageKind::Compute(ComputeOp::Requant {
            m: fb.requant_m,
            s: fb.requant_s,
        }),
        SiteRole::Compute,
        vec![mm_out],
        vec![rq_out],
    );
    let load_bias = push_stage(
        &mut p,
        "load_bias",
        StageKind::Load {
            tensor: bias_id,
            pattern: AccessPattern {
                origin: TileAddr {
                    base: 0,
                    steps: vec![(LV_ITEM_X, 1)],
                    indirect: None,
                },
                block_h: 1,
                block_w: 1,
                stride_w: 1,
                stride_h: nt,
            },
            num_blocks: 1,
            multicast: None,
        },
        SiteRole::Movement,
        vec![],
        vec![bias_in],
    );
    let bias = push_stage(
        &mut p,
        "bias",
        StageKind::Compute(ComputeOp::Bias { bias: bias_in }),
        SiteRole::Compute,
        vec![rq_out, bias_in],
        vec![bias_out],
    );
    let relu = push_stage(
        &mut p,
        "relu",
        StageKind::Compute(ComputeOp::Relu),
        SiteRole::Compute,
        vec![bias_out],
        vec![relu_out],
    );
    // The store now consumes the relu output.
    let store_id = p.stages[store].id;
    p.stages[store].ins = vec![relu_out];
    for e in p.edges.iter_mut() {
        if e.from == mm_id {
            e.to = vec![rq];
        }
    }
    let add_edge = |p: &mut SpatialProgram, from: StageId, buffer: BufferId, to: StageId| {
        let id = EdgeId::new(p.edges.len());
        p.edges.push(Edge {
            id,
            from,
            buffer,
            to: vec![to],
            handshake: local.clone(),
        });
    };
    add_edge(&mut p, rq, rq_out, bias);
    add_edge(&mut p, load_bias, bias_in, bias);
    add_edge(&mut p, bias, bias_out, relu);
    add_edge(&mut p, relu, relu_out, store_id);
    p.name = format!("fused_block_{}x{}_m{m}", fb.sys.k, fb.sys.n);
    p.validate()?;
    Ok(p)
}

/// A decision fabric (an if-converted scalar body) as a `Scalar` stage applied element-wise over an
/// `n`-element input vector: `y[i] = f(x[i])`.
pub fn from_scalar_body(
    df: &crate::codegen_sva::fabric_rtl::DecisionFabric,
    n: usize,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if target.scalar_target().is_none() {
        return Err(SpatialReject::MissingCompute {
            needed: "scalar datapath",
            target: target.name,
        });
    }
    if df.n_in != 1 {
        return Err(SpatialReject::NotLowered("multi-input scalar bodies"));
    }
    let local = Handshake::Fifo;
    let tile = target.tile;
    let grid = |rows: usize, cols: usize| TileGrid {
        rows_t: rows.div_ceil(tile.rows),
        cols_t: cols.div_ceil(tile.cols),
    };
    let tensors = vec![
        TensorDecl {
            id: TensorId(0),
            name: "x".into(),
            role: TensorRole::Input,
            elems: ElemShape { rows: 1, cols: n },
            tiles: grid(1, n),
            dtype: Precision::Int64,
            layout: Layout::RowMajorTiles,
            space: TensorSpace::Dram,
            init: TensorInit::External,
            range: None,
            chip: None,
        },
        TensorDecl {
            id: TensorId(1),
            name: "y".into(),
            role: TensorRole::Output,
            elems: ElemShape { rows: 1, cols: n },
            tiles: grid(1, n),
            dtype: Precision::Int64,
            layout: Layout::RowMajorTiles,
            space: TensorSpace::Dram,
            init: TensorInit::Zero,
            range: None,
            chip: None,
        },
    ];
    let nt = tensors[1].tiles.cols_t;
    let buffers = vec![
        Buffer {
            id: BufferId(0),
            name: "in0".into(),
            kind: BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: 1,
            },
            dtype: Precision::Int64,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        },
        Buffer {
            id: BufferId(1),
            name: "out".into(),
            kind: BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: 1,
            },
            dtype: Precision::Int64,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: None,
        },
    ];
    let stages = vec![
        Stage {
            id: StageId(0),
            name: "load_x".into(),
            kind: StageKind::Load {
                tensor: TensorId(0),
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![(LV_ITEM_Y, nt), (LV_ITEM_X, 1)],
                        indirect: None,
                    },
                    block_h: 1,
                    block_w: 1,
                    stride_w: 1,
                    stride_h: nt,
                },
                num_blocks: 1,
                multicast: None,
            },
            role: SiteRole::Movement,
            ins: vec![],
            outs: vec![BufferId(0)],
            site: None,
            item: None,
            origin: None,
        },
        Stage {
            id: StageId(1),
            name: "body".into(),
            kind: StageKind::Scalar(ScalarBody {
                n_in: df.n_in,
                cells: df.cells.clone(),
                result: df.result,
            }),
            role: SiteRole::Compute,
            ins: vec![BufferId(0)],
            outs: vec![BufferId(1)],
            site: None,
            item: None,
            origin: None,
        },
        Stage {
            id: StageId(2),
            name: "store_y".into(),
            kind: StageKind::Store {
                tensor: TensorId(1),
                pattern: AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![(LV_ITEM_Y, nt), (LV_ITEM_X, 1)],
                        indirect: None,
                    },
                    block_h: 1,
                    block_w: 1,
                    stride_w: 1,
                    stride_h: nt,
                },
                sub_blocks: (1, 1),
            },
            role: SiteRole::Movement,
            ins: vec![BufferId(1)],
            outs: vec![],
            site: None,
            item: None,
            origin: None,
        },
    ];
    let edges = vec![
        Edge {
            id: EdgeId(0),
            from: StageId(0),
            buffer: BufferId(0),
            to: vec![StageId(1)],
            handshake: local.clone(),
        },
        Edge {
            id: EdgeId(1),
            from: StageId(1),
            buffer: BufferId(1),
            to: vec![StageId(2)],
            handshake: local.clone(),
        },
    ];
    let mut items = Vec::new();
    for ox in 0..nt {
        items.push(WorkItem {
            id: WorkItemId::new(items.len()),
            coord: (0, ox),
            out: TileRect {
                row0_t: 0,
                col0_t: ox,
                h_t: 1,
                w_t: 1,
            },
        });
    }
    let mut requires = Requirements::default();
    requires.handshakes.insert(local.kind());
    let p = SpatialProgram {
        name: "scalar_body".into(),
        tile,
        tensors,
        stages,
        buffers,
        edges,
        semaphores: Vec::new(),
        sections: Vec::new(),
        trips: Vec::new(),
        links: Vec::new(),
        work: WorkList {
            items,
            k_tiles: 1,
            plan: None,
            k_trip: None,
        },
        numerics: NumericsPolicy::Exact,
        claim: NumericsClaim::Exact,
        acc_tiles_used: target.site.acc_tiles.max(1),
        mem_banks: target.site.mem_banks,
        requires,
        placement: None,
        memory: None,
        schedule: None,
        certificate: None,
    };
    p.validate()?;
    Ok(p)
}
