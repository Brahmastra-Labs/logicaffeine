//! Our fabric: the spatial IR as synthesizable Verilog.
//!
//! Every IR object is one RTL object, and every choreography op is a step of the engine that owns it: a site is a
//! module holding its tensor memories, its page FIFOs, its semaphores and its engines; an engine is a module printed
//! once per engine-program class and instantiated per site with the site's work as parameters; a multicast group is one
//! bus at the top; a tile matmul is the proven loadable weight-stationary PE array shaped to the block, and a matmul of
//! one row — a matrix-vector product — multiplies each operand element as it lands instead ([`Ctx::matvec`]). The
//! discrete-event twin (`fabric_cycle_model`) walks the same micro-programs with the same durations and stall rules, so
//! its count IS the RTL's count.
//!
//! A FIFO is its control — counts, reservations, the slot it reads — and its pages live in one of its site's pools: the
//! buffers of one phase take pools 0, 1, 2, …, so a site has as many page memories as its busiest phase has buffers
//! ([`site_pools`]).
//!
//! A site's memories interleave their elements over [`SpatialProgram::mem_banks`] banks, and a page starts on a word. A
//! run of `w` elements from a multiple of `w` — a tile row, a page's span, an accumulator row's columns — moves in one
//! cycle, each bank at the one row every bank of the port shares. A walk moves the widest run every row it starts is
//! aligned to in each memory it touches ([`Ctx::run`]), and each port carries only the run widths its ops move
//! ([`port_widths`]), so no port rotates a lane past its own word.
//!
//! The Generic unit is a vector unit as wide as those runs: an op takes a run of one axis's coordinates a point, a lane a
//! coordinate, where its slots read that axis as runs or as one element, and a slot that gathers reads, in each lane, the
//! element its own index names through a read port of its own ([`lanes_of`], [`gather_lanes`]). A reduced lane axis folds its
//! lanes through a tree into the op's own fold; a parallel one keeps a fold per lane and writes a run of columns. A site
//! of one bank prints the scalar unit, byte for byte.
//!
//! An op that carries data takes states of its own; an effect that only signals it — a push, a pop, a semaphore's
//! increment — commits in that op's last cycle, and the op's last cycle also takes the loop bound after it and checks the
//! guards it reaches, so the sequencer spends a cycle only on work or on a wait that is real. That last cycle is a
//! register the op's duration counts down to, so nothing decodes the state in front of the units it resets.

use std::collections::{BTreeMap, BTreeSet};

pub mod budget;
mod engine;

pub use engine::{floor_divider_latency, floor_divider_verilog};

use super::{require_verified, Census, SpatialArtifact, SpatialBackend, SpatialLowerError};
use crate::codegen_sva::fabric_grid::{emit_grid_verilog, route_loop_body};
use crate::codegen_sva::fabric_target::Precision;
use crate::codegen_sva::handshake_rtl::{emit_mcast_bus_verilog, emit_semaphore_verilog};
use crate::codegen_sva::systolic::{emit_systolic_pipelined_loadable_verilog_starting, pipelined_latency, WeightStart};
use crate::spatial::eval::{Inputs, Value};
use crate::spatial::ir::*;
use crate::spatial::passes::{LV_ITEM, LV_ITEM_X, LV_ITEM_Y};
use crate::spatial::target::{precision_int_bits, SpatialTarget};
use crate::spatial::value_range::{program_ranges, signed_bits, ProgramRanges};
use crate::tensor_recognize::ElementwiseOp;

#[derive(Clone, Debug, PartialEq)]
pub struct VerilogArtifact {
    pub top: String,
    pub rtl: String,
    pub testbench: String,
    pub hex: Vec<(String, String)>,
    pub census: Census,
    pub cycle_model: u64,
    pub cycle_floor: u64,
}

pub struct FabricBackend {
    pub target: SpatialTarget,
}

impl FabricBackend {
    pub fn new(target: SpatialTarget) -> Self {
        FabricBackend { target }
    }
}

// ── The micro-program: one entry per FSM state, shared by the printer and the twin ────────────────────────

/// A loop variable's role in an address: bound by an enclosing loop, or an item coordinate.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Micro {
    /// A one-cycle op whose precondition must hold (a bounded-buffer or semaphore wait).
    Guarded(Guard),
    /// An op of a fixed number of cycles, no precondition.
    Fixed(Fixed),
    /// An effect that takes no cycle of its own: it commits in the last cycle of the op before it — a push, a pop, a
    /// semaphore's increment or set, a valid flag — or is nothing at all on the fabric (a barrier, whose reads and
    /// writes the op before it has already landed; an accumulator's release). See [`fold_effects`].
    Then(Fixed),
    LoopStart {
        var: LoopVar,
        count: usize,
        end: usize,
    },
    LoopEnd {
        var: LoopVar,
        count: usize,
        start: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Guard {
    Reserve { buf: BufferId, pages: usize },
    Wait { buf: BufferId, pages: usize },
    SemWait { sem: SemId, until: SemCond },
    /// The halt at link `point` of a chip's program, the `ordinal`-th it makes: open once the host has continued the
    /// fabric past it, which it does by its `ordinal + 1`-th continue of the step.
    Link { point: usize, ordinal: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Fixed {
    Push {
        buf: BufferId,
        pages: usize,
    },
    Pop {
        buf: BufferId,
        pages: usize,
    },
    Barrier,
    ReadTiles {
        tensor: TensorId,
        at: TileAddr,
        block_h: usize,
        block_w: usize,
        stride_w: usize,
        stride_h: usize,
        into: BufferId,
        /// The section the op belongs to: its item coordinates are that section's items.
        section: Option<usize>,
    },
    WriteTiles {
        from: BufferId,
        tensor: TensorId,
        at: TileAddr,
        block_h: usize,
        block_w: usize,
        stride_w: usize,
        stride_h: usize,
        section: Option<usize>,
    },
    AcquireAcc {
        tiles: usize,
        /// The tiles a row of the region holds (set with the plan): a run never crosses a row's end.
        cols: usize,
    },
    ReleaseAcc,
    Matmul {
        block: BlockShape,
        per_site_n: usize,
        /// The sub-blocks down the site's rows: where there is more than one, A's page holds each one's rows in turn.
        sub_rows: usize,
        operand: Precision,
        ins: [BufferId; 2],
        reload: Option<BufferId>,
        tiles: usize,
        /// The stage this op runs: what its values are proven to hold.
        stage: StageId,
        /// The engine's PE array `(rows, cols)` — the largest of its matmuls, which every one of them runs on: this
        /// one loads its own rows and reads its own columns, and its latency is the physical array's.
        array: (usize, usize),
    },
    Unary {
        kind: UnaryKind,
        ins: Vec<BufferId>,
        tiles: usize,
        stage: StageId,
    },
    /// The general form ([`ComputeOp::Generic`]) on the integer datapath: a point of the iteration space every
    /// [`Issue::ii`] cycles, its slots read on their schedule and its body cycle entering the body's pipeline — and
    /// `depth` cycles more for the last point's value to leave it.
    Generic {
        unit: Box<GenUnit>,
        ins: Vec<BufferId>,
        tiles: usize,
        stage: StageId,
        /// The cycles from a point's body cycle to its value: the pipeline of its body class on its engine, set with the
        /// plan.
        depth: u64,
    },
    PackAcc {
        to: BufferId,
        tiles: usize,
        trunc32: bool,
        accumulate: bool,
        /// The tiles a row of the region holds (set with the plan).
        cols: usize,
    },
    SemSet {
        sem: SemId,
        value: SemValue,
    },
    SemInc {
        sem: SemId,
        at: Site,
        by: usize,
    },
    McastSetValid {
        sem: SemId,
        group: GroupId,
    },
    McastSend {
        buf: BufferId,
        pages: usize,
        group: GroupId,
    },
}

/// What a general Generic engine walks: its axes, one read per operand slot, the body, when each point's reads go out
/// ([`Issue`]), and the lanes a point takes along its lane axis ([`lanes_of`]).
#[derive(Clone, Debug, PartialEq, Eq)]
struct GenUnit {
    row: IterSpec,
    col: IterSpec,
    k: Option<IterSpec>,
    maps: Vec<ElemAccess>,
    body: TensorBody,
    issue: Issue,
    /// The consecutive coordinates of its lane axis a point takes: one, or a power of two no wider than the banks.
    lanes: usize,
    lane: LaneAxis,
}

/// The axis a Generic op's lanes run along: its contraction, the axis it walks inside the other (`Mid`), or the one it
/// walks outside it (`Out`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LaneAxis {
    K,
    Mid,
    Out,
}

/// A Generic op's modulo schedule: a point starts every `ii` cycles; slot `s` reads `at[s]` cycles after its point's
/// start, through the read port of its buffer's FIFO; the point's body cycle is `body` cycles after its start, once every
/// slot's value is latched.
///
/// Slots on one port read at distinct cycles modulo `ii`, so `ii` is the most slots any port serves: one point a cycle
/// where every slot has a port of its own. A gather's slot reads once its index slot's value is latched, and before the
/// next point's index value replaces it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Issue {
    ii: usize,
    at: Vec<usize>,
    body: usize,
    /// The pairs of slots that share a read: a point reads the one its select takes.
    shares: Vec<Share>,
}

/// Two slots of one port that a select the point's own axes decide keeps apart: `then` is read only where `pred` holds
/// and `otherwise` only where it does not, so a point reads one of them, in one cycle, and the value it takes is the
/// only one of the two its body uses.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Share {
    then: usize,
    otherwise: usize,
    pred: Pred,
}

/// Each select of `body` whose bound is a constant — decided by the point's own axes before a slot is read — with the
/// slots read only on its `then` side and those read only on its `otherwise` side.
fn exclusive_sides(body: &TensorBody) -> Vec<(Pred, BTreeSet<usize>, BTreeSet<usize>)> {
    fn uses(b: &TensorBody, out: &mut BTreeMap<usize, usize>) {
        match b {
            TensorBody::Operand(i) => *out.entry(*i).or_default() += 1,
            TensorBody::Const(_) => {}
            TensorBody::Add(x, y)
            | TensorBody::Sub(x, y)
            | TensorBody::Mul(x, y)
            | TensorBody::Min(x, y)
            | TensorBody::Max(x, y)
            | TensorBody::FloorDiv(x, y) => {
                uses(x, out);
                uses(y, out);
            }
            TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => uses(arg, out),
            TensorBody::Select { cond, then, otherwise } => {
                if let Some(i) = cond.operand() {
                    *out.entry(i).or_default() += 1;
                }
                uses(then, out);
                uses(otherwise, out);
            }
        }
    }
    fn walk(b: &TensorBody, total: &BTreeMap<usize, usize>, out: &mut Vec<(Pred, BTreeSet<usize>, BTreeSet<usize>)>) {
        match b {
            TensorBody::Operand(_) | TensorBody::Const(_) => {}
            TensorBody::Add(x, y)
            | TensorBody::Sub(x, y)
            | TensorBody::Mul(x, y)
            | TensorBody::Min(x, y)
            | TensorBody::Max(x, y)
            | TensorBody::FloorDiv(x, y) => {
                walk(x, total, out);
                walk(y, total, out);
            }
            TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => walk(arg, total, out),
            TensorBody::Select { cond, then, otherwise } => {
                if cond.operand().is_none() {
                    let only = |side: &TensorBody| -> BTreeSet<usize> {
                        let mut u = BTreeMap::new();
                        uses(side, &mut u);
                        u.into_iter().filter(|(i, n)| total.get(i) == Some(n)).map(|(i, _)| i).collect()
                    };
                    out.push((*cond, only(then), only(otherwise)));
                }
                walk(then, total, out);
                walk(otherwise, total, out);
            }
        }
    }
    let mut total = BTreeMap::new();
    uses(body, &mut total);
    let mut out = Vec::new();
    walk(body, &total, &mut out);
    out
}

impl Issue {
    /// The schedule of slots read through `ports`, in `order` — every gather's index slot before the slot it indexes —
    /// with `index_of` naming a gather's index slot, on memories that answer `lat` cycles after an address. Two slots of
    /// one port on opposite sides of a select of `sides` share a read; neither a gather nor an index it adds is shared.
    fn of(
        ports: &[usize],
        order: &[usize],
        index_of: &dyn Fn(usize) -> Option<usize>,
        lat: usize,
        sides: &[(Pred, BTreeSet<usize>, BTreeSet<usize>)],
    ) -> Issue {
        let n = ports.len();
        let indexes: BTreeSet<usize> = (0..n).filter_map(index_of).collect();
        let free = |s: usize| index_of(s).is_none() && !indexes.contains(&s);
        let mut partner: Vec<Option<usize>> = vec![None; n];
        let mut shares = Vec::new();
        for (pred, then, otherwise) in sides {
            for &t in then {
                for &o in otherwise {
                    if t < n && o < n && ports[t] == ports[o] && free(t) && free(o) && partner[t].is_none() && partner[o].is_none() {
                        partner[t] = Some(o);
                        partner[o] = Some(t);
                        shares.push(Share { then: t, otherwise: o, pred: *pred });
                    }
                }
            }
        }
        let mut per_port: BTreeMap<usize, usize> = BTreeMap::new();
        for s in 0..n {
            if partner[s].is_none_or(|p| p > s) {
                *per_port.entry(ports[s]).or_default() += 1;
            }
        }
        let ii = per_port.values().copied().max().unwrap_or(1).max(1);
        let mut at = vec![0usize; n];
        let mut done = vec![false; n];
        let mut used: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for &s in order {
            if let Some(p) = partner[s].filter(|p| done[*p]) {
                at[s] = at[p];
                done[s] = true;
                continue;
            }
            // A gather reads once its index has latched, `lat + 1` after the index's read, and its product has been
            // registered, one more.
            let earliest = index_of(s).map_or(0, |ix| at[ix] + lat + 2);
            let taken = used.entry(ports[s]).or_default();
            // `ii` is the most reads any port takes, so each of a port's reads finds a residue of its own within `ii`
            // cycles of its earliest.
            let o = (earliest..earliest + ii)
                .find(|o| !taken.contains(&(o % ii)))
                .expect("a port takes no more reads than the issue interval has cycles");
            taken.insert(o % ii);
            at[s] = o;
            done[s] = true;
        }
        let body = at.iter().map(|a| a + lat + 1).max().unwrap_or(1);
        Issue { ii, at, body, shares }
    }

    /// Cycles of an op of `points` points: the last point's start, its body cycle, then its pipeline.
    fn cycles(&self, points: u64, depth: u64) -> u64 {
        points.saturating_sub(1) * self.ii as u64 + self.body as u64 + depth + 1
    }
}

impl GenUnit {
    fn k_extent(&self) -> usize {
        self.k.map_or(1, |k| k.extent)
    }
    /// The points the op issues: each axis's extent, its lane axis's in runs of its lanes.
    fn points(&self) -> u64 {
        (self.points_along(self.row.var) * self.points_along(self.col.var) * self.k.map_or(1, |k| self.points_along(k.var))) as u64
    }
    /// The points the op issues along axis `v`: its extent, in runs of the lanes where it is the lane axis.
    fn points_along(&self, v: LoopVar) -> usize {
        let extent = if v == self.row.var {
            self.row.extent
        } else if v == self.col.var {
            self.col.extent
        } else {
            self.k_extent()
        };
        if v == self.lane_var() {
            extent.div_ceil(self.lanes)
        } else {
            extent
        }
    }
    /// The loop variable of the lane axis.
    fn lane_var(&self) -> LoopVar {
        lane_var(self.lane, &self.row, &self.col, self.k.as_ref())
    }
    /// The lane axis's extent, in coordinates.
    fn lane_extent(&self) -> usize {
        match self.lane {
            LaneAxis::K => self.k_extent(),
            LaneAxis::Mid if self.rows_inner() => self.row.extent,
            LaneAxis::Mid => self.col.extent,
            LaneAxis::Out if self.rows_inner() => self.col.extent,
            LaneAxis::Out => self.row.extent,
        }
    }
    /// Whether the op's lanes fold into one value a point — a contraction, or the axis its fold walks inside — through
    /// the lane tree, rather than each lane keeping its own.
    fn tree(&self) -> bool {
        self.lanes > 1 && (self.lane == LaneAxis::K || (self.lane == LaneAxis::Mid && self.lane_kind_reduces()))
    }
    /// Whether the lane tree sums — a contraction, or a sum's fold — rather than taking a maximum.
    fn tree_sums(&self) -> bool {
        self.tree_pool() == ReducePool::Sum
    }
    /// The pool the lane tree folds by: the contraction's sum, or the pool of the axis its lanes run along.
    fn tree_pool(&self) -> ReducePool {
        let v = self.lane_var();
        let kind = if self.lane == LaneAxis::K { None } else if v == self.row.var { Some(self.row.kind) } else { Some(self.col.kind) };
        match kind {
            Some(IterKind::Reduce { pool }) => pool,
            _ => ReducePool::Sum,
        }
    }
    fn lane_kind_reduces(&self) -> bool {
        let v = self.lane_var();
        let kind = if v == self.row.var { self.row.kind } else if v == self.col.var { self.col.kind } else { IterKind::Reduce { pool: ReducePool::Sum } };
        matches!(kind, IterKind::Reduce { .. })
    }
    /// The levels of the lane tree a point's value climbs: `log2` of its lanes where they fold, none where each lane keeps
    /// its own.
    fn tree_levels(&self) -> u64 {
        if self.tree() {
            self.lanes.trailing_zeros() as u64
        } else {
            0
        }
    }
    /// Whether slot `s` reads one element a point, the same in every lane: its map does not move along the lane axis, and
    /// it does not gather — a gather's lanes each read the element their own index names.
    fn broadcast(&self, s: usize) -> bool {
        if self.gathers(s) {
            return false;
        }
        let v = self.lane_var();
        self.maps[s].terms.iter().filter(|(t, _)| *t == v).map(|(_, st)| *st).sum::<i64>() == 0
    }
    fn slots(&self) -> usize {
        self.maps.len()
    }
    /// Whether the row axis is walked inside the column axis: when only the row folds, so its fold sees its values
    /// one after another. Otherwise the row is outermost.
    fn rows_inner(&self) -> bool {
        matches!(self.row.kind, IterKind::Reduce { .. }) && matches!(self.col.kind, IterKind::Parallel)
    }
    /// Whether slot `s`'s address adds the value another slot read — a gather. A slot indexing itself at stride 0 is a
    /// range guard on a plain read.
    fn gathers(&self, s: usize) -> bool {
        gathers(&self.maps, s)
    }
}

/// Whether slot `s` of `maps` gathers: its address adds the value another slot read.
fn gathers(maps: &[ElemAccess], s: usize) -> bool {
    index_slot(maps, s).is_some()
}

/// The slot whose value slot `s` of `maps` gathers by. A slot indexing itself at stride 0 is a range guard on a plain
/// read: the evaluator checks the element it reads against the extent and moves nowhere, so it reads like any slot.
fn index_slot(maps: &[ElemAccess], s: usize) -> Option<usize> {
    maps[s].indirect.filter(|ind| !(ind.operand == s && ind.stride == 0)).map(|ind| ind.operand)
}

/// The order a Generic's operand slots are read in: every gather's index slot before the slot it indexes. An index
/// slot that is itself gathered, or that indexes itself, has no such order.
fn read_order(maps: &[ElemAccess]) -> Result<Vec<usize>, SpatialLowerError> {
    let index_of = |s: usize| index_slot(maps, s);
    for s in 0..maps.len() {
        if let Some(ix) = index_of(s) {
            if ix == s || ix >= maps.len() || index_of(ix).is_some() {
                return Err(SpatialLowerError::Unsupported(
                    "a gather whose index slot is itself gathered, or names itself or no slot",
                ));
            }
        }
    }
    let mut order: Vec<usize> = (0..maps.len()).filter(|s| index_of(*s).is_none()).collect();
    order.extend((0..maps.len()).filter(|s| index_of(*s).is_some()));
    Ok(order)
}

/// The loop variable of lane axis `lane` over the space `row × col (× k)`: the contraction, or the axis walked inside the
/// other — the row where only the row folds, else the column — or the one walked outside it.
fn lane_var(lane: LaneAxis, row: &IterSpec, col: &IterSpec, k: Option<&IterSpec>) -> LoopVar {
    let rows_inner = matches!(row.kind, IterKind::Reduce { .. }) && matches!(col.kind, IterKind::Parallel);
    match lane {
        LaneAxis::K => k.map_or(col.var, |k| k.var),
        LaneAxis::Mid if rows_inner => row.var,
        LaneAxis::Mid => col.var,
        LaneAxis::Out if rows_inner => col.var,
        LaneAxis::Out => row.var,
    }
}

/// ★ A GENERIC POINT TAKES A RUN. A banked site moves a run of consecutive elements a cycle, so its Generic unit takes
/// `L` consecutive coordinates of one axis a point — `L` a power of two no wider than the target's vector lanes
/// (`widest`, itself no wider than the banks), each a lane of the body — where every slot's read of a point is one such
/// run or one element:
///
/// - each slot's map moves along the lane axis by one (a run, a lane an element) or not at all (one element, the same in
///   every lane); a slot that gathers moves along no lane axis, and each of its lanes reads the element its own lane of
///   the index slot names, through a read port of the table's FIFO of its own ([`gather_lanes`]);
/// - a run starts at a multiple of `L` in its page: the slot's base and its stride along every other axis are multiples of
///   `L`, and so are the run's own steps;
/// - the lane axis is the contraction or the axis walked inside the other, which a reduced axis folds through the lane
///   tree; or a parallel column, which each lane keeps for itself, whichever axis it is walked as;
/// - a select whose two sides share a read ([`Issue`]) keeps sharing only where its bound along the lane axis is a
///   multiple of `L`, so every lane of a point takes the same side; otherwise both sides read;
/// - no node of the body is a floor division: its divider already takes a pair a cycle, a stage per quotient bit, and a
///   copy of it per lane is the costliest hardware the unit could repeat, for the few points a decode step divides.
///
/// Of the lane axes and widths an op allows, it takes the one that issues its points in the fewest cycles, its lane
/// tree's levels counted; on a tie the fewest lanes, then the contraction before the inner axis before the outer.
/// Returns the lanes, the lane axis and the issue schedule of that choice.
#[allow(clippy::too_many_arguments)]
fn lanes_of(
    widest: usize,
    row: &IterSpec,
    col: &IterSpec,
    k: Option<&IterSpec>,
    maps: &[ElemAccess],
    body: &TensorBody,
    ports: &[usize],
    order: &[usize],
    lat: usize,
) -> (usize, LaneAxis, Issue) {
    let index_of = |s: usize| index_slot(maps, s);
    let sides = exclusive_sides(body);
    let scalar = Issue::of(ports, order, &index_of, lat, &sides);
    let rows_inner = matches!(row.kind, IterKind::Reduce { .. }) && matches!(col.kind, IterKind::Parallel);
    let extent = |v: LoopVar| -> usize {
        if v == row.var {
            row.extent
        } else if v == col.var {
            col.extent
        } else {
            k.map_or(1, |k| k.extent)
        }
    };
    let kind = |v: LoopVar| -> IterKind {
        if v == row.var {
            row.kind
        } else if v == col.var {
            col.kind
        } else {
            IterKind::Reduce { pool: ReducePool::Sum }
        }
    };
    let stride = |m: &ElemAccess, v: LoopVar| -> i64 { m.terms.iter().filter(|(t, _)| *t == v).map(|(_, s)| *s).sum() };
    let axes: Vec<LoopVar> = [Some(row.var), Some(col.var), k.map(|k| k.var)].into_iter().flatten().collect();
    let points = |v: LoopVar, l: usize| -> u64 { axes.iter().map(|a| if *a == v { extent(*a).div_ceil(l) } else { extent(*a) } as u64).product() };
    // Each choice as (cycles, lanes, axis, issue); the scalar one first.
    let mut choices = vec![(points(col.var, 1) * scalar.ii as u64, 1usize, LaneAxis::Mid, scalar)];
    let scalar_only = body.post_order().iter().any(|n| matches!(n, TensorBody::FloorDiv(..)));
    for lane in [LaneAxis::K, LaneAxis::Mid, LaneAxis::Out] {
        // An outer lane axis is a parallel column each lane keeps: the outer axis is the column only where the row folds
        // inside it.
        if scalar_only || (lane == LaneAxis::K && k.is_none()) || (lane == LaneAxis::Out && !rows_inner) {
            continue;
        }
        let v = lane_var(lane, row, col, k);
        if extent(v) < 2 {
            continue;
        }
        let levels = |l: usize| if matches!(kind(v), IterKind::Reduce { .. }) { l.trailing_zeros() as u64 } else { 0 };
        let mut l = 2;
        while l <= widest {
            let aligned = maps.iter().enumerate().all(|(s, m)| match (index_of(s).is_some(), stride(m, v)) {
                (true, moves) => moves == 0,
                (false, 0) => true,
                (false, 1) => m.base.rem_euclid(l as i64) == 0 && axes.iter().filter(|a| **a != v).all(|a| stride(m, *a).rem_euclid(l as i64) == 0),
                _ => false,
            });
            if !aligned {
                break;
            }
            let kept: Vec<(Pred, BTreeSet<usize>, BTreeSet<usize>)> = sides
                .iter()
                .filter(|(pred, _, _)| pred.var() != v || matches!(pred.rhs(), PredRhs::Const(b) if b % l == 0))
                .cloned()
                .collect();
            let issue = Issue::of(ports, order, &index_of, lat, &kept);
            choices.push((points(v, l) * issue.ii as u64 + levels(l), l, lane, issue));
            l *= 2;
        }
    }
    let (_, lanes, lane, issue) = choices.into_iter().min_by_key(|(cycles, lanes, lane, _)| (*cycles, *lanes, *lane)).expect("the scalar choice");
    (lanes, lane, issue)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum UnaryKind {
    Requant { m: i64, s: u32 },
    Bias,
    Relu,
    Scalar(ScalarBody),
    Elementwise(ElementwiseOp),
}

/// The per-engine facts the printer and the twin both need.
struct EnginePlan {
    site: Site,
    engine: Engine,
    class: usize,
    micro: Vec<Micro>,
    item_start: usize,
    item_count: usize,
}

struct Ctx<'a> {
    p: &'a SpatialProgram,
    te: usize,
    tr: usize,
    tc: usize,
    grid_nx: usize,
    plan_acc: AccPolicy,
    /// Cycles from a memory read's address to its data: every op that reads a tensor or a buffer issues each address
    /// at its phase and takes the data `lat` phases later.
    lat: u64,
    /// What every tensor, buffer and stage of the program holds: the width of each memory and of each datapath.
    ranges: ProgramRanges,
    /// The banks every memory of a site interleaves its elements over ([`SpatialProgram::mem_banks`]).
    banks: usize,
    /// The widest run the Generic unit takes a point: the target's vector lanes, no wider than the banks.
    lanes: usize,
    /// The tiles one page of each buffer holds ([`SpatialProgram::page_extents`]).
    extent: Vec<usize>,
    /// The constants the engine being printed compares its phase with, each read as the flag `ph_ge{k}`.
    phase_flags: std::cell::RefCell<BTreeSet<u64>>,
    /// The halts of a chip's program, in order: each the gather of the rendezvous a [`LinkPoint`] follows, and the link's
    /// index. The root engine, past that gather, waits there for the host. Empty for a program that halts nowhere.
    halts: Vec<(SemId, usize)>,
    /// The widest product one multiplier slice takes ([`SiteSpec::multiplier`](crate::spatial::target::SiteSpec::multiplier)).
    multiplier: Option<(u32, u32)>,
    /// The tiles a tile-row of each tensor takes in its memory ([`tensor_pitch`]).
    pitch: Vec<usize>,
    /// Each buffer template whose pages keep their rows at a pitch: the rows' width and the pitch ([`page_pitches`]).
    page_pitch: BTreeMap<BufferId, (usize, usize)>,
}

impl Ctx<'_> {
    /// `ph >= k` as the register that holds it, `ph_ge{k}`, loaded from the next phase: no comparison of the phase stands
    /// in front of what it guards. The engine being printed declares and loads every flag read.
    fn ph_ge(&self, k: u64) -> String {
        if k == 0 {
            return "1'b1".into();
        }
        self.phase_flags.borrow_mut().insert(k);
        format!("ph_ge{k}")
    }
    /// `ph < k`, as the flag of `ph >= k` negated.
    fn ph_lt(&self, k: u64) -> String {
        if k == 0 {
            return "1'b0".into();
        }
        format!("!{}", self.ph_ge(k))
    }
    /// The accumulator policy of the section `stage` belongs to — a composition's sections each run under their own
    /// plan — or the program's, for a program of one section.
    fn acc_of(&self, stage: StageId) -> AccPolicy {
        self.p
            .sections
            .iter()
            .find(|s| (s.stages.0..s.stages.1).contains(&stage.idx()))
            .and_then(|s| s.plan)
            .map(|pl| pl.acc)
            .unwrap_or(self.plan_acc)
    }
    fn page_elems(&self, b: BufferId) -> usize {
        self.extent[b.idx()] * self.te
    }
    /// The words tensor `t`'s memory holds, its rows at the tensor's pitch: what `print_site` declares and the hex
    /// image fills.
    fn tensor_words(&self, t: &TensorDecl) -> usize {
        tensor_words(t, self.pitch[t.id.idx()], self.te)
    }
    /// The words a page of `b` takes in its pool: its elements, its rows at their pitch where its template keeps one
    /// ([`page_pitches`]).
    fn page_words(&self, b: BufferId) -> usize {
        match self.page_pitch.get(&self.template(b)) {
            Some((w, pitch)) => self.page_elems(b) / w * pitch,
            None => self.page_elems(b),
        }
    }
    /// The words from one row of a page of `b` to the next, for rows of `width` elements: its template's pitch, or the
    /// width where its rows lie end to end.
    fn page_row(&self, b: BufferId, width: usize) -> usize {
        self.page_pitch.get(&self.template(b)).map_or(width, |(_, pitch)| *pitch)
    }
    /// Whether a tile walk moves runs: its run is wider than an element, or it fills a page whose rows keep a pitch, whose
    /// end of row the run walk steps over.
    fn walks_runs(&self, f: &Fixed) -> bool {
        self.run(f) > 1 || matches!(f, Fixed::ReadTiles { into, .. } if self.page_pitch.contains_key(&self.template(*into)))
    }
    fn depth(&self, b: BufferId) -> usize {
        match self.p.buffer(b).kind {
            BufferKind::Fifo { depth_pages, .. } | BufferKind::Interm { depth_pages, .. } => {
                depth_pages
            }
            BufferKind::Accumulator { .. } => 1,
        }
    }
    /// The width of a buffer's words: what every buffer of its template holds, since one FIFO module serves them all.
    fn buffer_width(&self, b: BufferId) -> u32 {
        let t = self.template(b);
        self.p
            .buffers
            .iter()
            .filter(|x| self.template(x.id) == t)
            .filter_map(|x| self.ranges.buffers[x.id.idx()])
            .map(signed_bits)
            .max()
            .unwrap_or(1)
    }
    fn template(&self, b: BufferId) -> BufferId {
        self.p.buffer(b).origin.unwrap_or(b)
    }
    /// Cycles of a fixed op. An op that reads memory takes `lat` more, for its last read to return; a tile read through
    /// a gather `lat` more again, for the index its addresses add.
    fn duration(&self, f: &Fixed) -> u64 {
        let te = self.te as u64;
        let lat = self.lat;
        match f {
            Fixed::Push { .. }
            | Fixed::Pop { .. }
            | Fixed::Barrier
            | Fixed::ReleaseAcc
            | Fixed::SemSet { .. }
            | Fixed::SemInc { .. }
            | Fixed::McastSetValid { .. } => 1,
            Fixed::ReadTiles {
                block_h, block_w, at, ..
            } => self.walk_cycles(f, *block_h, *block_w) + self.gather_skew(at) + lat,
            Fixed::WriteTiles {
                block_h, block_w, ..
            } => self.walk_cycles(f, *block_h, *block_w) + lat,
            Fixed::AcquireAcc { tiles, cols } => self.acc_walk_cycles(*tiles, *cols, self.acc_run(f)),
            Fixed::PackAcc { tiles, accumulate, cols, .. } => self.acc_walk_cycles(*tiles, *cols, self.acc_run(f)) + if *accumulate { lat } else { 0 },
            Fixed::Unary { tiles, .. } => *tiles as u64 * te + lat,
            Fixed::Generic { unit, depth, .. } => unit.issue.cycles(unit.points(), *depth),
            Fixed::Matmul { block, per_site_n, sub_rows, ins, reload, array, .. } => {
                let (osh, ibw) = (block.osh as u64, block.ibw as u64);
                let k = ibw * self.tc as u64;
                let l = pipelined_latency(array.0, array.1) as u64;
                let run = self.matmul_run(block, self.page_row(ins[1], *per_site_n), *per_site_n, *sub_rows);
                let (reload, a, b) = self.matmul_loads(block, reload.is_some(), run);
                if self.matvec(block) {
                    // Each B element is multiplied as it lands and summed a cycle later; the sums join the accumulator
                    // the cycle after the last one.
                    reload + a + b + lat + 2
                } else {
                    reload + a + b + k + osh * self.tr as u64 + l + 1 + lat
                }
            }
            Fixed::McastSend { buf, pages, .. } => (*pages * self.page_elems(*buf)) as u64 + 1 + lat,
        }
    }
    /// Whether the site moves runs at all: its tiles are single elements, and its memories have more than one bank.
    fn runs(&self) -> bool {
        self.te == 1 && self.banks > 1
    }
    /// ★ A RUN IS A WORD OF ITS OWN ALIGNMENT: `w` consecutive elements from a multiple of `w`, `w` a power of two no
    /// larger than the banks, each from its own bank at the one row every bank of the port shares. The run's `j`-th
    /// element is in bank `start | j`, so a write puts lane `b & (w − 1)` in bank `b` and no port rotates by more than a
    /// whole word of the run. A walk moves runs of the largest `w` every run it starts is aligned to, in every memory
    /// it touches — the twin and the printer both take it from here, so a walk is never priced at a width the
    /// hardware does not move.
    ///
    /// The largest such `w` for an address term `x`: every multiple of it keeps a run aligned.
    fn align(&self, x: usize) -> usize {
        if x == 0 {
            self.banks
        } else {
            (1usize << x.trailing_zeros()).min(self.banks)
        }
    }
    /// `rows` rows of `width` elements laid end to end from a word: the alignment every row's start keeps.
    fn rows_align(&self, rows: usize, width: usize) -> usize {
        if rows <= 1 {
            self.banks
        } else {
            self.align(width)
        }
    }
    /// A tile walk's tensor addresses, `block_h` rows `stride_h` apart from `at`: the alignment every row's start keeps —
    /// the least of the base's, every loop stride's, every item coordinate of the op's section times its stride, and a
    /// gather's tile stride, whatever index it reads.
    fn tensor_align(&self, at: &TileAddr, block_h: usize, stride_h: usize, section: Option<usize>) -> usize {
        let coords: Vec<(usize, usize)> = self
            .p
            .work
            .items
            .iter()
            .filter(|it| section.is_none() || self.p.section_of_item(it.id) == section)
            .map(|it| it.coord)
            .collect();
        let mut w = self.align(at.base).min(self.rows_align(block_h, stride_h));
        if let Some(ind) = at.indirect {
            w = w.min(self.align(ind.tile_stride));
        }
        for (v, stride) in &at.steps {
            if *v == LV_ITEM_Y {
                w = coords.iter().fold(w, |w, c| w.min(self.align(c.0 * stride)));
            } else if *v == LV_ITEM_X {
                w = coords.iter().fold(w, |w, c| w.min(self.align(c.1 * stride)));
            } else {
                w = w.min(self.align(*stride));
            }
        }
        w
    }
    /// The elements a tile read or write moves a cycle: where its tiles are single elements laid one after another along
    /// a row, a run as wide as both its tensor rows and its page rows are aligned to — a page starts on a word; one
    /// element otherwise.
    fn run(&self, f: &Fixed) -> usize {
        match f {
            Fixed::ReadTiles { at, block_h, block_w, stride_w, stride_h, section, into, .. } if self.runs() && *stride_w == 1 => {
                self.rows_align(*block_h, self.page_row(*into, *block_w)).min(self.tensor_align(at, *block_h, *stride_h, *section))
            }
            Fixed::WriteTiles { at, block_h, block_w, stride_w, stride_h, section, .. } if self.runs() && *stride_w == 1 => {
                self.rows_align(*block_h, *block_w).min(self.tensor_align(at, *block_h, *stride_h, *section))
            }
            _ => 1,
        }
    }
    /// The elements an accumulator walk moves a cycle: where its tiles are single elements, a run of the region's row as
    /// wide as the page it fills keeps its rows aligned — an acquire fills none; one element otherwise. A region's row is
    /// its `cols` banks, which a run walks from column 0.
    fn acc_run(&self, f: &Fixed) -> usize {
        match f {
            Fixed::AcquireAcc { .. } if self.runs() => self.banks,
            Fixed::PackAcc { tiles, cols, .. } if self.runs() => self.rows_align(tiles.div_ceil((*cols).max(1)), *cols),
            _ => 1,
        }
    }
    /// The elements a matmul's loads move a cycle: where its tiles are single elements, a run as wide as every row of
    /// every page it loads keeps — the partial's `osh` rows of `osw`, A's `osh` rows of `ibw` at its sub-block, B's
    /// `ibw` rows of `osw` at its sub-block's columns, `b_row` apart (`per_site_n`, or its page's pitch); one element
    /// otherwise.
    fn matmul_run(&self, block: &BlockShape, b_row: usize, per_site_n: usize, sub_rows: usize) -> usize {
        if !self.runs() {
            return 1;
        }
        let (osh, osw, ibw) = (block.osh, block.osw, block.ibw);
        let mut w = self.rows_align(osh, osw).min(self.rows_align(osh, ibw)).min(self.rows_align(ibw, b_row));
        if sub_rows > 1 {
            w = w.min(self.align(osh * ibw));
        }
        if per_site_n != osw {
            w = w.min(self.align(osw));
        }
        w
    }
    /// ★ A MATMUL OF ONE ROW NEVER LOADS THE ARRAY. Loading the weight-stationary array costs its `k` rows and its fill
    /// `k + n − 1` for one streamed row: a matrix-vector product — every matmul of a decode step — multiplies each B
    /// element by its A element as it lands instead, one multiplier per column, and sums the column as the rows arrive.
    fn matvec(&self, block: &BlockShape) -> bool {
        block.osh * self.tr == 1
    }
    /// Cycles a matmul's loads take at `run` elements a cycle — the partial it reloads, its A rows, its B rows — each a
    /// row's runs.
    fn matmul_loads(&self, block: &BlockShape, reload: bool, run: usize) -> (u64, u64, u64) {
        let (osh, osw, ibw, te) = (block.osh, block.osw, block.ibw, self.te);
        let rows = |n: usize, width: usize| -> u64 { (n * (width * te).div_ceil(run)) as u64 };
        (if reload { rows(osh, osw) } else { 0 }, rows(osh, ibw), rows(ibw, osw))
    }
    /// Cycles an accumulator walk over `tiles` tiles in rows of `cols` takes at `run` elements a cycle: each row's runs,
    /// the last row's as far as the tiles reach.
    fn acc_walk_cycles(&self, tiles: usize, cols: usize, run: usize) -> u64 {
        let elems = tiles * self.te;
        match run {
            1 => elems as u64,
            run => {
                let cols = cols.max(1);
                ((elems / cols) * cols.div_ceil(run) + (elems % cols).div_ceil(run)) as u64
            }
        }
    }
    /// Cycles a tile walk of `block_h` rows of `block_w` tiles takes: each row's runs, or each tile's elements.
    fn walk_cycles(&self, f: &Fixed, block_h: usize, block_w: usize) -> u64 {
        match self.run(f) {
            1 => (block_h * block_w) as u64 * self.te as u64,
            run => (block_h * block_w.div_ceil(run)) as u64,
        }
    }
    /// The phases a tile read waits before its first address: a gather's addresses add the index it reads first.
    fn gather_skew(&self, at: &TileAddr) -> u64 {
        if at.indirect.is_some() {
            self.lat
        } else {
            0
        }
    }
}

/// A tile walk's address on the fabric: at its tensor's pitch ([`tensor_pitch`]), and, on a site that moves runs, with
/// only the terms that move it — an item coordinate's, or a loop's around it of more than one trip. A term of a variable
/// no loop around the walk binds adds nothing to the address it issues ([`addr_v`]), and one of a loop that runs once
/// adds zero; either would only narrow the runs the walk moves ([`Ctx::tensor_align`]).
fn walk_addr(ctx: &Ctx, tensor: TensorId, at: &TileAddr, strides: (usize, usize), loops: &[(LoopVar, usize)]) -> (TileAddr, usize, usize) {
    let (cols, pitch) = (ctx.p.tensor(tensor).tiles.cols_t, ctx.pitch[tensor.idx()]);
    let mut at = at.clone();
    if ctx.runs() {
        at.steps.retain(|(v, _)| *v == LV_ITEM_Y || *v == LV_ITEM_X || loops.iter().any(|(l, count)| l == v && *count > 1));
    }
    (pitched_addr(&at, cols, pitch), pitched(strides.0, cols, pitch), pitched(strides.1, cols, pitch))
}

fn lower_ops(ctx: &Ctx, ops: &[TaggedOp], out: &mut Vec<Micro>) -> Result<(), SpatialLowerError> {
    lower_ops_within(ctx, ops, &mut Vec::new(), out)
}

/// [`lower_ops`] inside `loops`, each with its trip count.
fn lower_ops_within(ctx: &Ctx, ops: &[TaggedOp], loops: &mut Vec<(LoopVar, usize)>, out: &mut Vec<Micro>) -> Result<(), SpatialLowerError> {
    for t in ops {
        match &t.op {
            ChoreoOp::Reserve { buf, pages } => out.push(Micro::Guarded(Guard::Reserve { buf: *buf, pages: *pages })),
            ChoreoOp::Push { buf, pages } => out.push(Micro::Fixed(Fixed::Push { buf: *buf, pages: *pages })),
            ChoreoOp::Wait { buf, pages } => out.push(Micro::Guarded(Guard::Wait { buf: *buf, pages: *pages })),
            ChoreoOp::Pop { buf, pages } => out.push(Micro::Fixed(Fixed::Pop { buf: *buf, pages: *pages })),
            ChoreoOp::ReadTiles { tensor, at, block_h, block_w, stride_w, stride_h, into } => {
                let (at, stride_w, stride_h) = walk_addr(ctx, *tensor, at, (*stride_w, *stride_h), loops);
                out.push(Micro::Fixed(Fixed::ReadTiles { tensor: *tensor, at, block_h: *block_h, block_w: *block_w, stride_w, stride_h, into: *into, section: ctx.p.section_of_stage(t.stage) }))
            }
            ChoreoOp::ReadBarrier | ChoreoOp::WriteBarrier | ChoreoOp::Flush { .. } => out.push(Micro::Fixed(Fixed::Barrier)),
            ChoreoOp::WriteTiles { from, tensor, at, block_h, block_w, stride_w, stride_h } => {
                let (at, stride_w, stride_h) = walk_addr(ctx, *tensor, at, (*stride_w, *stride_h), loops);
                out.push(Micro::Fixed(Fixed::WriteTiles { from: *from, tensor: *tensor, at, block_h: *block_h, block_w: *block_w, stride_w, stride_h, section: ctx.p.section_of_stage(t.stage) }))
            }
            ChoreoOp::AcquireAcc { tiles } => out.push(Micro::Fixed(Fixed::AcquireAcc { tiles: *tiles, cols: 0 })),
            ChoreoOp::ReleaseAcc => out.push(Micro::Fixed(Fixed::ReleaseAcc)),
            ChoreoOp::Compute { op, ins, acc_reload } => {
                // A pointwise op covers the tiles its region acquired, as the evaluator's does.
                let region = out.iter().rev().find_map(|m| match m {
                    Micro::Fixed(Fixed::AcquireAcc { tiles, .. }) => Some(Some(*tiles)),
                    Micro::Fixed(Fixed::ReleaseAcc) => Some(None),
                    _ => None,
                });
                let tiles = region.flatten().unwrap_or(ctx.p.acc_tiles_used).max(1);
                match op {
                    ComputeOp::MatmulTiles { block, sub_blocks, operand, .. } => {
                        if ins.len() != 2 {
                            return Err(SpatialLowerError::Unsupported("a tile matmul with two operand buffers"));
                        }
                        if precision_int_bits(*operand).is_none() {
                            return Err(SpatialLowerError::Precision { needed: *operand });
                        }
                        out.push(Micro::Fixed(Fixed::Matmul { block: *block, per_site_n: sub_blocks.1 * block.osw, sub_rows: sub_blocks.0, operand: *operand, ins: [ins[0], ins[1]], reload: *acc_reload, tiles: block.osh * block.osw, stage: t.stage, array: (block.ibw * ctx.tc, block.osw * ctx.tc) }));
                    }
                    ComputeOp::Requant { m, s } => out.push(Micro::Fixed(Fixed::Unary { kind: UnaryKind::Requant { m: *m, s: *s }, ins: ins.clone(), tiles, stage: t.stage })),
                    ComputeOp::Bias { .. } => out.push(Micro::Fixed(Fixed::Unary { kind: UnaryKind::Bias, ins: ins.clone(), tiles, stage: t.stage })),
                    ComputeOp::Relu => out.push(Micro::Fixed(Fixed::Unary { kind: UnaryKind::Relu, ins: ins.clone(), tiles, stage: t.stage })),
                    ComputeOp::Scalar(body) => out.push(Micro::Fixed(Fixed::Unary { kind: UnaryKind::Scalar(body.clone()), ins: ins.clone(), tiles, stage: t.stage })),
                    // The general form, where the fabric can express it: identity maps and one
                    // binary operation over two operands is exactly its `UnaryKind::Elementwise`
                    // datapath. Anything else (a broadcast, a reduction, a transcendental) is refused
                    // by name below — the integer fabric genuinely has no unit for those.
                    ComputeOp::Generic { iters, ins: maps, out: omap, body: TensorBody::Add(a, b) }
                        if ctx.p.generic_is_plain_pointwise(iters, maps, omap, ins) && matches!(**a, TensorBody::Operand(0)) && matches!(**b, TensorBody::Operand(1)) =>
                    {
                        out.push(Micro::Fixed(Fixed::Unary { kind: UnaryKind::Elementwise(ElementwiseOp::Add), ins: ins.clone(), tiles, stage: t.stage }))
                    }
                    ComputeOp::Generic { iters, ins: maps, out: omap, body: TensorBody::Mul(a, b) }
                        if ctx.p.generic_is_plain_pointwise(iters, maps, omap, ins) && matches!(**a, TensorBody::Operand(0)) && matches!(**b, TensorBody::Operand(1)) =>
                    {
                        out.push(Micro::Fixed(Fixed::Unary { kind: UnaryKind::Elementwise(ElementwiseOp::Mul), ins: ins.clone(), tiles, stage: t.stage }))
                    }
                    // A general form the integer fabric cannot express (a broadcast, a reduction, a
                    // transcendental), or the fused row spec — it genuinely has no unit for these.
                    // ★ THE GENERAL FORM IS THE EVALUATOR'S INTEGER ARM IN HARDWARE: walk the space, read each slot
                    // through its map (a gather through the value its index slot read), apply the body, fold. A
                    // body with a transcendental, or an operand that is not an integer buffer, has no unit here.
                    ComputeOp::Generic { iters, ins: maps, body, .. } => {
                        let (row, col, k) = match iters.as_slice() {
                            [r, c] => (*r, *c, None),
                            [r, c, k] => (*r, *c, Some(*k)),
                            _ => return Err(SpatialLowerError::Unsupported("a Generic whose space is not (rows, cols) or (rows, cols, contraction)")),
                        };
                        if k.is_some_and(|k| !matches!(k.kind, IterKind::Reduce { pool: ReducePool::Sum })) {
                            return Err(SpatialLowerError::Unsupported("a Generic whose contraction axis reduces by a pool other than a sum"));
                        }
                        if !body.integral() || ins.iter().any(|b| precision_int_bits(ctx.p.buffer(*b).dtype).is_none()) {
                            return Err(SpatialLowerError::Unsupported("a float row op on the integer fabric"));
                        }
                        if maps.len() != ins.len() || ctx.tr != ctx.tc {
                            return Err(SpatialLowerError::Unsupported("a Generic whose maps do not match its operands, or a non-square tile"));
                        }
                        let tiles = if matches!(col.kind, IterKind::Reduce { .. }) { 1 } else { col.extent.div_ceil(ctx.tr) };
                        let order = read_order(maps)?;
                        let ports: Vec<usize> = ins.iter().map(|b| ctx.template(*b).idx()).collect();
                        let (lanes, lane, issue) = lanes_of(ctx.lanes, &row, &col, k.as_ref(), maps, body, &ports, &order, ctx.lat as usize);
                        out.push(Micro::Fixed(Fixed::Generic {
                            unit: Box::new(GenUnit { row, col, k, maps: maps.clone(), body: body.clone(), issue, lanes, lane }),
                            ins: ins.clone(),
                            tiles,
                            stage: t.stage,
                            depth: 0,
                        }))
                    }
                }
            }
            ChoreoOp::PackAcc { to, tiles, accumulate, .. } => out.push(Micro::Fixed(Fixed::PackAcc { to: *to, tiles: *tiles, trunc32: ctx.acc_of(t.stage) == AccPolicy::I32, accumulate: *accumulate, cols: 0 })),
            ChoreoOp::SemSet { sem, value } => out.push(Micro::Fixed(Fixed::SemSet { sem: *sem, value: *value })),
            ChoreoOp::SemInc { sem, at, by } => out.push(Micro::Fixed(Fixed::SemInc { sem: *sem, at: *at, by: *by })),
            ChoreoOp::SemWait { sem, until } => {
                out.push(Micro::Guarded(Guard::SemWait { sem: *sem, until: *until }));
                if let Some(ordinal) = ctx.halts.iter().position(|(g, _)| g == sem) {
                    out.push(Micro::Guarded(Guard::Link { point: ctx.halts[ordinal].1, ordinal }));
                }
            }
            ChoreoOp::McastSetValid { sem, group } => out.push(Micro::Fixed(Fixed::McastSetValid { sem: *sem, group: *group })),
            ChoreoOp::McastSend { buf, pages, group, .. } => out.push(Micro::Fixed(Fixed::McastSend { buf: *buf, pages: *pages, group: *group })),
            ChoreoOp::Barrier { .. } => return Err(SpatialLowerError::Unsupported("a group barrier on the fabric")),
            ChoreoOp::Loop { count: LoopCount::Trip { .. }, .. } => {
                return Err(SpatialLowerError::Unsupported("a loop whose count the program reads from its data (a trip) — the fabric's loop micro-ops count to a constant"))
            }
            ChoreoOp::Loop { var, count, body } => {
                let start = out.len();
                out.push(Micro::LoopStart { var: *var, count: count.bound(), end: 0 });
                loops.push((*var, count.bound()));
                lower_ops_within(ctx, body, loops, out)?;
                loops.pop();
                let end = out.len();
                out.push(Micro::LoopEnd { var: *var, count: count.bound(), start });
                if let Micro::LoopStart { end: e, .. } = &mut out[start] {
                    *e = end;
                }
            }
        }
    }
    Ok(())
}

/// The port signals an effect drives — a FIFO's push or pop, a semaphore's increment or set, a group's valid flag, or
/// none for a barrier or a release — or `None` for an op that is no effect.
fn effect_signals(ctx: &Ctx, f: &Fixed) -> Option<Vec<String>> {
    Some(match f {
        Fixed::Push { buf, .. } => vec![format!("f{}_push", ctx.template(*buf).idx())],
        Fixed::Pop { buf, .. } => vec![format!("f{}_pop", ctx.template(*buf).idx())],
        Fixed::SemInc { sem, .. } => vec![format!("s{}_inc", sem.idx())],
        Fixed::SemSet { sem, .. } => vec![format!("s{}_set", sem.idx())],
        Fixed::McastSetValid { sem, group } => vec![format!("mv{}_g{}", sem.idx(), group.idx())],
        Fixed::Barrier | Fixed::ReleaseAcc => Vec::new(),
        _ => return None,
    })
}

/// Fold every effect that follows a fixed op into that op's last cycle ([`Micro::Then`]). An op's work commits at the
/// clock edge that ends its last cycle; the effect that signals it — a push of the page it filled, a pop of the page it
/// has read, an increment telling another engine it is done — commits at that same edge, where it took a cycle of its
/// own the edge after. Two effects on one port signal never share a cycle, and an effect after a guard or a loop's
/// bound keeps its own: a jump only ever lands on the op after a loop's bound, so none lands on an effect.
fn fold_effects(ctx: &Ctx, micro: &mut [Micro]) {
    let mut used: Option<BTreeSet<String>> = None;
    for m in micro.iter_mut() {
        let Micro::Fixed(f) = m else {
            used = None;
            continue;
        };
        let f = f.clone();
        let signals = effect_signals(ctx, &f);
        if let (Some(sig), Some(u)) = (&signals, used.as_mut()) {
            if sig.iter().all(|s| !u.contains(s)) {
                u.extend(sig.iter().cloned());
                *m = Micro::Then(f);
                continue;
            }
        }
        used = Some(signals.unwrap_or_default().into_iter().collect());
    }
}

/// What an effect or a guard reads or moves: a buffer's FIFO, or a semaphore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Resource {
    Fifo(BufferId),
    Sem(SemId),
    /// The host's continues, which no op of the fabric moves.
    Link,
}

fn effect_resource(f: &Fixed) -> Option<Resource> {
    match f {
        Fixed::Push { buf, .. } | Fixed::Pop { buf, .. } => Some(Resource::Fifo(*buf)),
        Fixed::SemInc { sem, .. } | Fixed::SemSet { sem, .. } | Fixed::McastSetValid { sem, .. } => Some(Resource::Sem(*sem)),
        _ => None,
    }
}

fn guard_resource(g: &Guard) -> Resource {
    match g {
        Guard::Reserve { buf, .. } | Guard::Wait { buf, .. } => Resource::Fifo(*buf),
        Guard::SemWait { sem, .. } => Resource::Sem(*sem),
        Guard::Link { .. } => Resource::Link,
    }
}

/// What a state's own effects do to a FIFO or a semaphore in the cycle they commit: a FIFO's count and reservations,
/// a semaphore's count at this site — or that it is set outright, which no early check reasons past.
#[derive(Clone, Copy, Default)]
struct OwnDelta {
    count: i64,
    reserved: i64,
    sem: i64,
    set: bool,
}

/// The own effects of the fixed op at `pc` and those folded into it.
fn own_deltas(micro: &[Micro], pc: usize, site: Site) -> BTreeMap<Resource, OwnDelta> {
    let mut out: BTreeMap<Resource, OwnDelta> = BTreeMap::new();
    for m in &micro[pc..after_effects(micro, pc)] {
        let (Micro::Fixed(f) | Micro::Then(f)) = m else { continue };
        let Some(r) = effect_resource(f) else { continue };
        let d = out.entry(r).or_default();
        match f {
            Fixed::Push { pages, .. } => {
                d.count += *pages as i64;
                d.reserved -= *pages as i64;
            }
            Fixed::Pop { pages, .. } => d.count -= *pages as i64,
            Fixed::SemInc { at, by, .. } if *at == site => d.sem += *by as i64,
            Fixed::SemInc { .. } => {}
            _ => d.set = true,
        }
    }
    out
}

/// A guard checked a cycle early: the threshold its FIFO's or semaphore's registered state must reach once this
/// state's own effects — and the reservations of the guards checked before it — are counted, or `None` where no early
/// check is sound. Another engine only ever pushes what this one waits on, pops what it reserves, raises what it counts
/// up to, so a state that reaches the threshold a cycle early reaches it on time; an equality on a semaphore this state
/// moves, or one set outright, has no such order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Early {
    /// `count ≥ n` — an `n` of zero or less always holds.
    Count(BufferId, i64),
    /// `free ≥ n`.
    Free(BufferId, i64),
    /// `count ≥ n` of a semaphore.
    AtLeast(SemId, i64),
    /// `count == n` of a semaphore this state leaves alone.
    Equals(SemId, i64),
}

fn early_check(g: &Guard, own: &BTreeMap<Resource, OwnDelta>) -> Option<Early> {
    let d = own.get(&guard_resource(g)).copied().unwrap_or_default();
    match g {
        Guard::Wait { buf, pages } => Some(Early::Count(*buf, *pages as i64 - d.count)),
        Guard::Reserve { buf, pages } => Some(Early::Free(*buf, *pages as i64 + d.count + d.reserved)),
        Guard::SemWait { sem, until } if !d.set => match until {
            SemCond::AtLeast(n) => Some(Early::AtLeast(*sem, *n as i64 - d.sem)),
            SemCond::Equals(n) if d.sem == 0 => Some(Early::Equals(*sem, *n as i64)),
            SemCond::Equals(_) => None,
        },
        Guard::SemWait { .. } => None,
        // Only the host opens a halt: no state of the fabric's reaches it a cycle early.
        Guard::Link { .. } => None,
    }
}

/// A loop's bound a fixed op's last cycle takes itself: the end of a loop — its variable stepped, back to the body's
/// first op while it runs, past the loop when it is done — or the start of one, its variable cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bound {
    End { var: LoopVar, count: usize, start: usize },
    Start { var: LoopVar, count: usize, end: usize },
}

/// Where a fixed op's last cycle goes. Past its folded effects; through one loop bound that follows them; then, in each
/// branch the bound leaves (the loop running on, or done; for a start, its body or — at a count of zero — past it),
/// past the guards that branch lands on, each checked early ([`Early`]): all of a branch's open, the op takes their
/// reservations and goes past them; one not open, it lands on the first, whose own state waits as before.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Exit {
    bound: Option<Bound>,
    /// Each branch: the state it lands on, and the guards there it checks early with their checks.
    branches: Vec<(usize, Vec<(usize, Early)>)>,
}

fn exit_of(micro: &[Micro], pc: usize, site: Site) -> Exit {
    let own = own_deltas(micro, pc, site);
    let after = after_effects(micro, pc);
    let checked = |at: usize| -> Vec<(usize, Early)> {
        let mut own = own.clone();
        let mut out = Vec::new();
        while let Some(Micro::Guarded(g)) = micro.get(at + out.len()) {
            let Some(e) = early_check(g, &own) else { break };
            if let Guard::Reserve { buf, pages } = g {
                own.entry(Resource::Fifo(*buf)).or_default().reserved += *pages as i64;
            }
            out.push((at + out.len(), e));
        }
        out
    };
    match micro.get(after) {
        Some(Micro::LoopEnd { var, count, start }) => Exit {
            bound: Some(Bound::End { var: *var, count: *count, start: *start }),
            branches: vec![(start + 1, checked(start + 1)), (after + 1, checked(after + 1))],
        },
        Some(Micro::LoopStart { var, count, end }) => Exit {
            bound: Some(Bound::Start { var: *var, count: *count, end: *end }),
            branches: vec![(after + 1, checked(after + 1)), (end + 1, Vec::new())],
        },
        _ => Exit { bound: None, branches: vec![(after, checked(after))] },
    }
}

/// The state after the fixed op at `pc`: past the effects folded into it.
fn after_effects(micro: &[Micro], pc: usize) -> usize {
    let mut n = pc + 1;
    while matches!(micro.get(n), Some(Micro::Then(_))) {
        n += 1;
    }
    n
}

fn plans(ctx: &Ctx) -> Result<Vec<EnginePlan>, SpatialLowerError> {
    let p = ctx.p;
    let sched = p
        .schedule
        .as_ref()
        .ok_or(SpatialLowerError::Unsupported("a scheduled program"))?;
    let pl = p
        .placement
        .as_ref()
        .ok_or(SpatialLowerError::Unsupported("a placed program"))?;
    let mut classes: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for prog in &sched.programs {
        let sig = format!("{:?}:{}", prog.engine, p.engine_signature(prog));
        let class = match classes.iter().position(|c| *c == sig) {
            Some(i) => i,
            None => {
                classes.push(sig);
                classes.len() - 1
            }
        };
        let mut micro = Vec::new();
        lower_ops(ctx, &prog.ops, &mut micro)?;
        let physical = micro.iter().fold((0, 0), |acc, m| match m {
            Micro::Fixed(Fixed::Matmul { array, .. }) => (acc.0.max(array.0), acc.1.max(array.1)),
            _ => acc,
        });
        for m in &mut micro {
            if let Micro::Fixed(Fixed::Matmul { array, .. }) = m {
                *array = physical;
            }
        }
        let regions = engine::acc_regions(&micro);
        for (m, region) in micro.iter_mut().zip(regions) {
            if let (Micro::Fixed(Fixed::AcquireAcc { cols, .. } | Fixed::PackAcc { cols, .. }), Some(r)) = (m, region) {
                *cols = r;
            }
        }
        fold_effects(ctx, &mut micro);
        let items = pl.items_at(prog.site);
        out.push(EnginePlan {
            site: prog.site,
            engine: prog.engine,
            class,
            micro,
            item_start: items.first().map(|i| i.idx()).unwrap_or(0),
            item_count: items.len(),
        });
    }
    engine::set_generic_depths(ctx, &mut out)?;
    Ok(out)
}

// ── The discrete-event twin ───────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct FifoState {
    count: usize,
    reserved: usize,
}

struct EngineState {
    pc: usize,
    ph: u64,
    lv: BTreeMap<LoopVar, usize>,
    done: bool,
}

/// Predicted RTL cycles of `p`: every engine walks its micro-program in lock-step cycles against registered
/// buffer and semaphore state (an effect lands the cycle after it is issued), exactly as the FSMs do.
pub fn fabric_cycle_model(p: &SpatialProgram, target: &SpatialTarget) -> u64 {
    let Ok(ctx) = context(p, target) else { return 0 };
    let Ok(plans) = plans(&ctx) else { return 0 };
    twin(&ctx, &plans, &mut |_, _| {}, &mut Vec::new(), TWIN_LIMIT)
}

/// The cycles past which the twin stops: a program still running there is reported at the limit plus one.
const TWIN_LIMIT: u64 = 50_000_000;

/// The twin's cycles for `p` where it finishes within `limit` cycles; `None` where it does not, or where the fabric
/// cannot lower it. The twin stops at the limit, so asking costs at most `limit` cycles of simulation.
pub fn fabric_cycles_within(p: &SpatialProgram, target: &SpatialTarget, limit: u64) -> Option<u64> {
    let ctx = context(p, target).ok()?;
    let plans = plans(&ctx).ok()?;
    let cycles = twin(&ctx, &plans, &mut |_, _| {}, &mut Vec::new(), limit);
    (cycles <= limit).then_some(cycles)
}

/// The fabric's cycle counter at each halt of `p`'s program ([`LinkPoint`]), in order, then at its end: what its board
/// replies to each `X`. The counter pauses while the fabric is halted, so each count is the cycles the program has run.
/// One count — [`fabric_cycle_model`]'s — for a program that halts nowhere; `None` where the fabric does not lower `p`.
pub fn fabric_cycle_segments(p: &SpatialProgram, target: &SpatialTarget) -> Option<Vec<u64>> {
    let ctx = context(p, target).ok()?;
    let plans = plans(&ctx).ok()?;
    let mut halts = Vec::new();
    let total = twin(&ctx, &plans, &mut |_, _| {}, &mut halts, TWIN_LIMIT);
    halts.push(total);
    Some(halts)
}

/// What one engine did in one cycle of the twin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Activity {
    /// Running an op of this kind, or passing a guard of it.
    Busy(&'static str),
    /// Held at a guard of this kind: a page not yet free or filled, a semaphore not yet raised.
    Waiting(&'static str),
    /// Entering or closing a loop.
    Looping,
    /// Finished, while another engine still runs.
    Done,
}

/// One engine's cycles of a run, by what it did in each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineCycles {
    pub site: Site,
    pub engine: Engine,
    pub cycles: BTreeMap<Activity, u64>,
}

/// Where a run's cycles go: its total, the twin's, and each engine's cycles by activity — every engine accounting for
/// every cycle of the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleBreakdown {
    pub total: u64,
    pub engines: Vec<EngineCycles>,
}

/// [`fabric_cycle_model`] with each engine's cycles attributed: what it ran, what held it, when it was done. `None`
/// where the fabric does not lower `p`.
pub fn fabric_cycle_breakdown(p: &SpatialProgram, target: &SpatialTarget) -> Option<CycleBreakdown> {
    let ctx = context(p, target).ok()?;
    let plans = plans(&ctx).ok()?;
    let mut engines: Vec<EngineCycles> =
        plans.iter().map(|pl| EngineCycles { site: pl.site, engine: pl.engine, cycles: BTreeMap::new() }).collect();
    let total = twin(&ctx, &plans, &mut |ei, a| *engines[ei].cycles.entry(a).or_default() += 1, &mut Vec::new(), TWIN_LIMIT);
    Some(CycleBreakdown { total, engines })
}

/// Each engine's micro-program as the twin and the printer walk it: one line per state — the op, and a fixed op's
/// cycles; a tile walk's run, and an accumulator walk's region columns and run. `None` where the fabric does not lower
/// `p`.
pub fn fabric_micro_listing(p: &SpatialProgram, target: &SpatialTarget) -> Option<Vec<(Site, Engine, Vec<String>)>> {
    let ctx = context(p, target).ok()?;
    let plans = plans(&ctx).ok()?;
    Some(
        plans
            .iter()
            .map(|pl| {
                let regions = engine::acc_regions(&pl.micro);
                let lines = pl
                    .micro
                    .iter()
                    .enumerate()
                    .map(|(i, m)| match m {
                        Micro::Fixed(f @ (Fixed::ReadTiles { .. } | Fixed::WriteTiles { .. })) => {
                            format!("{} [{}] {f:?} run {}", fixed_kind(f), ctx.duration(f), ctx.run(f))
                        }
                        Micro::Fixed(f @ (Fixed::AcquireAcc { .. } | Fixed::PackAcc { .. })) => format!(
                            "{} [{}] {f:?} region {} run {}",
                            fixed_kind(f),
                            ctx.duration(f),
                            regions[i].unwrap_or(1),
                            ctx.acc_run(f)
                        ),
                        Micro::Fixed(f @ Fixed::Generic { unit, .. }) => format!("{} [{}] {f:?} points: {}", fixed_kind(f), ctx.duration(f), unit.points()),
                        Micro::Fixed(f) => format!("{} [{}] {f:?}", fixed_kind(f), ctx.duration(f)),
                        Micro::Then(f) => format!("  then {f:?}"),
                        Micro::Guarded(g) => format!("{} {g:?}", guard_kind(g)),
                        Micro::LoopStart { var, count, .. } => format!("loop {var:?} × {count}"),
                        Micro::LoopEnd { var, .. } => format!("end loop {var:?}"),
                    })
                    .collect();
                (pl.site, pl.engine, lines)
            })
            .collect(),
    )
}

/// The kind a fixed op's cycles count under.
fn fixed_kind(f: &Fixed) -> &'static str {
    match f {
        Fixed::ReadTiles { .. } => "read tiles",
        Fixed::WriteTiles { .. } => "write tiles",
        Fixed::Push { .. } => "push",
        Fixed::Pop { .. } => "pop",
        Fixed::Barrier => "barrier",
        Fixed::AcquireAcc { .. } => "acquire",
        Fixed::ReleaseAcc => "release",
        Fixed::Matmul { .. } => "matmul",
        Fixed::Unary { .. } => "unary",
        Fixed::Generic { .. } => "generic",
        Fixed::PackAcc { .. } => "pack",
        Fixed::SemSet { .. } | Fixed::SemInc { .. } => "semaphore",
        Fixed::McastSetValid { .. } | Fixed::McastSend { .. } => "multicast",
    }
}

fn guard_kind(g: &Guard) -> &'static str {
    match g {
        Guard::Reserve { .. } => "reserve",
        Guard::Wait { .. } => "wait",
        Guard::SemWait { .. } => "semaphore wait",
        Guard::Link { .. } => "link",
    }
}

/// The twin: every engine in lock-step, `observe` told what each did in each cycle. Returns the cycles from `start` to
/// `done`.
fn twin(ctx: &Ctx, plans: &[EnginePlan], observe: &mut dyn FnMut(usize, Activity), halts: &mut Vec<u64>, limit: u64) -> u64 {
    let p = ctx.p;
    let mut fifos: BTreeMap<BufferId, FifoState> = p
        .buffers
        .iter()
        .map(|b| (b.id, FifoState::default()))
        .collect();
    let mut sems: BTreeMap<(SemId, Site), usize> = BTreeMap::new();
    for s in &p.semaphores {
        for site in &s.sites {
            sems.insert((s.id, *site), s.init as usize);
        }
    }
    let mut states: Vec<EngineState> = plans
        .iter()
        .map(|_| EngineState {
            pc: 0,
            ph: 0,
            lv: BTreeMap::new(),
            done: false,
        })
        .collect();
    let receivers_of = |g: GroupId| -> Vec<Site> {
        let pl = p.placement.as_ref().unwrap();
        let gr = &pl.groups[g.idx()];
        gr.rect
            .sites()
            .into_iter()
            .filter(|s| *s != gr.sender)
            .collect()
    };
    // The top counts from the `start` pulse to `done`: one cycle for every engine to latch `running`, then
    // the FSM rounds, then the cycle in which the last engine raises `done`.
    let mut cycles: u64 = 2;
    loop {
        if states.iter().all(|s| s.done) || cycles > limit {
            break;
        }
        // Phase 1: decide on registered state.
        let mut fifo_delta: BTreeMap<BufferId, (i64, i64)> = BTreeMap::new(); // (count, reserved)
        let mut sem_inc: BTreeMap<(SemId, Site), usize> = BTreeMap::new();
        let mut sem_set: BTreeMap<(SemId, Site), usize> = BTreeMap::new();
        let mut next: Vec<Option<(usize, u64, Option<(LoopVar, usize)>)>> = Vec::new();
        for (ei, plan) in plans.iter().enumerate() {
            let st = &states[ei];
            if st.done {
                observe(ei, Activity::Done);
                next.push(None);
                continue;
            }
            let Some(m) = plan.micro.get(st.pc) else {
                observe(ei, Activity::Done);
                next.push(Some((st.pc, 0, None)));
                continue;
            };
            observe(
                ei,
                match m {
                    Micro::Fixed(f) | Micro::Then(f) => Activity::Busy(fixed_kind(f)),
                    Micro::LoopStart { .. } | Micro::LoopEnd { .. } => Activity::Looping,
                    Micro::Guarded(g) => {
                        let open = match g {
                            Guard::Reserve { buf, pages } => {
                                let f = &fifos[buf];
                                ctx.depth(*buf) >= f.count + f.reserved + *pages
                            }
                            Guard::Wait { buf, pages } => fifos[buf].count >= *pages,
                            Guard::SemWait { sem, until } => {
                                let v = *sems.get(&(*sem, plan.site)).unwrap_or(&0);
                                match until {
                                    SemCond::Equals(n) => v == *n,
                                    SemCond::AtLeast(n) => v >= *n,
                                }
                            }
                            Guard::Link { .. } => true,
                        };
                        if open {
                            Activity::Busy(guard_kind(g))
                        } else {
                            Activity::Waiting(guard_kind(g))
                        }
                    }
                },
            );
            let open = |g: &Guard| match g {
                Guard::Reserve { buf, pages } => {
                    let f = &fifos[buf];
                    ctx.depth(*buf) >= f.count + f.reserved + *pages
                }
                Guard::Wait { buf, pages } => fifos[buf].count >= *pages,
                Guard::SemWait { sem, until } => {
                    let v = *sems.get(&(*sem, plan.site)).unwrap_or(&0);
                    match until {
                        SemCond::Equals(n) => v == *n,
                        SemCond::AtLeast(n) => v >= *n,
                    }
                }
                // The host continues a halted fabric at once: the halt costs the cycle that leaves it, and the counter,
                // paused while it is halted, reads the cycles run before it.
                Guard::Link { .. } => true,
            };
            // The count starts at two, the start's latch and the cycle that raises `done`; while the engines run, the
            // counter has counted only the first, so in any running cycle it reads one less than the count.
            if let Micro::Guarded(Guard::Link { .. }) = m {
                halts.push(cycles - 1);
            }
            let step = match m {
                Micro::Guarded(g) => {
                    let ok = open(g);
                    if ok {
                        if let Guard::Reserve { buf, pages } = g {
                            fifo_delta.entry(*buf).or_default().1 += *pages as i64;
                        }
                        Some((st.pc + 1, 0, None))
                    } else {
                        Some((st.pc, 0, None))
                    }
                }
                Micro::Fixed(f) => {
                    let d = ctx.duration(f);
                    if st.ph + 1 >= d {
                        // The op's own effect and every effect folded into its last cycle.
                        let next = after_effects(&plan.micro, st.pc);
                        let folded = plan.micro[st.pc + 1..next].iter().filter_map(|m| match m {
                            Micro::Then(e) => Some(e),
                            _ => None,
                        });
                        for e in std::iter::once(f).chain(folded) {
                            match e {
                                Fixed::Push { buf, pages } => {
                                    let e = fifo_delta.entry(*buf).or_default();
                                    e.0 += *pages as i64;
                                    e.1 -= *pages as i64;
                                }
                                Fixed::Pop { buf, pages } => fifo_delta.entry(*buf).or_default().0 -= *pages as i64,
                                Fixed::SemSet { sem, value } => {
                                    let v = match value {
                                        SemValue::Invalid => 0,
                                        SemValue::Valid => 1,
                                        SemValue::Count(n) => *n,
                                    };
                                    sem_set.insert((*sem, plan.site), v);
                                }
                                Fixed::SemInc { sem, at, by } => *sem_inc.entry((*sem, *at)).or_default() += *by,
                                Fixed::McastSetValid { sem, group } => {
                                    for r in receivers_of(*group) {
                                        sem_set.insert((*sem, r), 1);
                                    }
                                }
                                _ => {}
                            }
                        }
                        // Through the loop bound it takes itself, then past the guards of its branch it checks early.
                        let exit = exit_of(&plan.micro, st.pc, plan.site);
                        let count = |var: LoopVar, count: usize| if var == LV_ITEM { plan.item_count } else { count };
                        let (branch, lv) = match exit.bound {
                            None => (0, None),
                            Some(Bound::End { var, count: c, .. }) => {
                                let v = st.lv.get(&var).copied().unwrap_or(0) + 1;
                                (if v < count(var, c) { 0 } else { 1 }, Some((var, v)))
                            }
                            Some(Bound::Start { var, count: c, .. }) => (if count(var, c) == 0 { 1 } else { 0 }, Some((var, 0))),
                        };
                        let (at, checks) = &exit.branches[branch];
                        let holds = |e: &Early| match *e {
                            Early::Count(b, n) => fifos[&b].count as i64 >= n,
                            Early::Free(b, n) => {
                                let f = &fifos[&b];
                                ctx.depth(b) as i64 - f.count as i64 - f.reserved as i64 >= n
                            }
                            Early::AtLeast(sem, n) => *sems.get(&(sem, plan.site)).unwrap_or(&0) as i64 >= n,
                            Early::Equals(sem, n) => *sems.get(&(sem, plan.site)).unwrap_or(&0) as i64 == n,
                        };
                        if !checks.is_empty() && checks.iter().all(|(_, e)| holds(e)) {
                            for (g, _) in checks {
                                if let Micro::Guarded(Guard::Reserve { buf, pages }) = &plan.micro[*g] {
                                    fifo_delta.entry(*buf).or_default().1 += *pages as i64;
                                }
                            }
                            Some((checks[checks.len() - 1].0 + 1, 0, lv))
                        } else {
                            Some((*at, 0, lv))
                        }
                    } else {
                        Some((st.pc, st.ph + 1, None))
                    }
                }
                // A folded effect commits with the op before it; the sequencer never rests on one.
                Micro::Then(_) => Some((st.pc + 1, 0, None)),
                Micro::LoopStart { var, count, end } => {
                    if *count == 0 {
                        Some((end + 1, 0, None))
                    } else {
                        Some((st.pc + 1, 0, Some((*var, 0))))
                    }
                }
                Micro::LoopEnd { var, count, start } => {
                    let v = st.lv.get(var).copied().unwrap_or(0) + 1;
                    let count = if *var == LV_ITEM {
                        plan.item_count
                    } else {
                        *count
                    };
                    if v < count {
                        Some((start + 1, 0, Some((*var, v))))
                    } else {
                        Some((st.pc + 1, 0, Some((*var, v))))
                    }
                }
            };
            next.push(step);
        }
        // Phase 2: commit.
        for (ei, step) in next.into_iter().enumerate() {
            if let Some((pc, ph, lv)) = step {
                let st = &mut states[ei];
                st.pc = pc;
                st.ph = ph;
                if let Some((v, x)) = lv {
                    st.lv.insert(v, x);
                }
                if pc >= plans[ei].micro.len() {
                    st.done = true;
                }
            }
        }
        for (b, (dc, dr)) in fifo_delta {
            let f = fifos.get_mut(&b).unwrap();
            f.count = (f.count as i64 + dc).max(0) as usize;
            f.reserved = (f.reserved as i64 + dr).max(0) as usize;
        }
        for (k, v) in sem_set {
            sems.insert(k, v);
        }
        for (k, n) in sem_inc {
            *sems.entry(k).or_insert(0) += n;
        }
        cycles += 1;
    }
    cycles
}

fn context<'a>(
    p: &'a SpatialProgram,
    target: &'a SpatialTarget,
) -> Result<Ctx<'a>, SpatialLowerError> {
    let plan = p
        .work
        .plan
        .ok_or(SpatialLowerError::Unsupported("a tiled program"))?;
    let lat = target
        .site
        .local_mem_read_latency
        .ok_or(SpatialLowerError::Unsupported("a fabric target that states its local memory's read latency"))?;
    if !p.mem_banks.is_power_of_two() {
        return Err(SpatialLowerError::Unsupported("memory banks that are not a power of two"));
    }
    if !target.site.vector_lanes.is_power_of_two() {
        return Err(SpatialLowerError::Unsupported("a site whose vector unit has lanes that are not a power of two"));
    }
    Ok(Ctx {
        p,
        te: p.tile.elems(),
        tr: p.tile.rows,
        tc: p.tile.cols,
        grid_nx: plan.grid.1,
        plan_acc: plan.acc,
        lat: lat as u64,
        ranges: program_ranges(p),
        banks: p.mem_banks,
        lanes: target.site.vector_lanes.min(p.mem_banks),
        extent: p.page_extents(),
        phase_flags: Default::default(),
        halts: halts_of(p)?,
        multiplier: target.site.multiplier,
        pitch: p.tensors.iter().map(|t| tensor_pitch(p, t)).collect(),
        page_pitch: page_pitches(p),
    })
}

/// The halts of `p`'s links: each link after a section another follows halts at that section's rendezvous, past its
/// gather. A link before the first section is the host's to serve before the start, one after the last at the end.
fn halts_of(p: &SpatialProgram) -> Result<Vec<(SemId, usize)>, SpatialLowerError> {
    let mut out = Vec::new();
    for (i, l) in p.links.iter().enumerate() {
        let Some(k) = l.after.filter(|k| k + 1 < p.sections.len()) else { continue };
        let r = p.sections[k]
            .barrier_after
            .ok_or(SpatialLowerError::Unsupported("a link's halt after a section with no barrier after it"))?;
        out.push((r.gather, i));
    }
    Ok(out)
}

/// The width of tensor `t`'s words. A tensor the host carries — an `Input` it supplies, a `State` it keeps between
/// steps — is held at its own precision, so any word the host writes reads back whole; every other tensor at the width
/// of what it holds.
fn tensor_width(t: &TensorDecl, ranges: &ProgramRanges) -> u32 {
    match t.role {
        TensorRole::Input | TensorRole::State => precision_int_bits(t.dtype).unwrap_or(64),
        TensorRole::Param | TensorRole::Activation | TensorRole::Output => ranges.tensors[t.id.idx()].map_or(1, signed_bits),
    }
}

/// A memory word's lanes, `(low bit, width)`: one lane up to 32 bits, else 32-bit lanes. A block RAM wider than 32 bits
/// maps to the 72-bit simple dual port, whose parity lanes the open 7-series flow wires wrong; no lane ever is.
fn lanes(width: u32) -> Vec<(u32, u32)> {
    (0..width.div_ceil(32)).map(|l| (l * 32, (width - l * 32).min(32))).collect()
}

/// Lane `l` of memory `mem` that has `n` lanes.
fn lane(mem: &str, l: usize, n: usize) -> String {
    if n == 1 {
        mem.to_string()
    } else {
        format!("{mem}_l{l}")
    }
}

/// Bank `b` of memory `mem` that has `banks` banks: the memory itself where it has one.
fn bank(mem: &str, b: usize, banks: usize) -> String {
    if banks == 1 {
        mem.to_string()
    } else {
        format!("{mem}_b{b}")
    }
}

/// `log2` of a bank count, a power of two.
fn log2_banks(banks: usize) -> u32 {
    banks.trailing_zeros()
}

/// The word holding element `addr` — a constant — of memory `mem` of `width` bits in `banks` banks: its lanes, high
/// first.
fn memory_word(mem: &str, addr: usize, width: u32, banks: usize) -> String {
    let m = bank(mem, addr % banks, banks);
    let row = addr / banks;
    let n = lanes(width).len();
    let parts: Vec<String> = (0..n).rev().map(|l| format!("{}[{row}]", lane(&m, l, n))).collect();
    format!("{{{}}}", parts.join(", "))
}

/// The run of up to `banks` elements of `width` bits from `lanes`, a concatenation of `banks` lanes of `from` bits
/// each — the lowest the first — every lane brought to `to` bits, sign-extended or cut: the values a run moves hold in
/// both widths, so neither changes one.
fn resize_lanes(src: &str, from: u32, to: u32, banks: usize) -> String {
    let parts: Vec<String> = (0..banks)
        .rev()
        .map(|j| {
            let lo = j as u32 * from;
            if to > from {
                format!("{{{{{}{{{src}[{}]}}}}, {src}[{}:{lo}]}}", to - from, lo + from - 1, lo + from - 1)
            } else {
                format!("{src}[{}:{lo}]", lo + to - 1)
            }
        })
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// The wires that write memory `mem` of `width` bits in `banks` banks, named by `tag`, and the statements that write it:
/// `n` elements from element `addr` in a run of `2^lw`, `data`'s low `width` bits the first and `hi` the rest, lane
/// after lane. Every bank writes the row `addr` sits at. A run starts at a multiple of its width ([`Ctx::run`]), so its
/// element in bank `b` is lane `b mod 2^lw`, in the banks of its word; a single element is a run of one. The port carries
/// the run widths `runs` ([`port_widths`]) and no others. With one bank, `data` at `addr`. Under simulation, a run that
/// does not start on its word, or of a width the port does not carry, stops the run loudly.
#[allow(clippy::too_many_arguments)]
fn memory_write(tag: &str, mem: &str, addr: &str, n: &str, lw: &str, data: &str, hi: &str, width: u32, banks: usize, runs: &BTreeSet<usize>) -> (String, String) {
    let ls = lanes(width);
    let put = |m: &str, a: &str, v: &str| -> String {
        ls.iter()
            .enumerate()
            .map(|(l, (lo, w))| format!("{}[{a}] <= {v}[{}:{lo}];", lane(m, l, ls.len()), lo + w - 1))
            .collect::<Vec<_>>()
            .join(" ")
    };
    if banks == 1 {
        return (String::new(), put(mem, addr, data));
    }
    let lg = log2_banks(banks);
    let mut wires = format!(
        "  wire [31:0] {tag}_a = {addr}; wire [31:0] {tag}_r = {tag}_a >> {lg}; wire [{}:0] {tag}_o = {tag}_a[{}:0]; wire [{}:0] {tag}_v = {data}; wire [{}:0] {tag}_h = {hi};",
        lg - 1,
        lg - 1,
        width - 1,
        (banks as u32 - 1) * width - 1
    );
    let lane_of = |j: usize| if j == 0 { format!("{tag}_v") } else { format!("{tag}_h[{}:{}]", j as u32 * width - 1, (j as u32 - 1) * width) };
    // Bank `b` under a run of `2^w`: its lane, and whether the run's word holds it.
    let arm = |b: usize, w: u32| -> (String, String) {
        let j = b & ((1usize << w) - 1);
        let word = if w == lg { "1'b1".to_string() } else { format!("({tag}_o[{}:{w}] == {}'d{})", lg - 1, lg - w, b >> w) };
        (lane_of(j), format!("{word} && ({}'d{j} < {n})", lg + 1))
    };
    let carried: Vec<u32> = runs.iter().map(|w| w.trailing_zeros()).collect();
    let mut stmts = Vec::new();
    for b in 0..banks {
        if let [w] = carried[..] {
            let (d, e) = arm(b, w);
            wires.push_str(&format!(" wire [{}:0] {tag}_d{b} = {d}; wire {tag}_e{b} = {e};", width - 1));
        } else {
            let arms: String = carried
                .iter()
                .map(|w| {
                    let (d, e) = arm(b, *w);
                    format!(" 4'd{w}: begin {tag}_d{b} = {d}; {tag}_e{b} = {e}; end")
                })
                .collect();
            wires.push_str(&format!(
                " reg [{}:0] {tag}_d{b}; reg {tag}_e{b}; always @* case ({lw}){arms} default: begin {tag}_d{b} = {tag}_v; {tag}_e{b} = 1'b0; end endcase",
                width - 1
            ));
        }
        stmts.push(format!("if ({tag}_e{b}) begin {} end", put(&bank(mem, b, banks), &format!("{tag}_r"), &format!("{tag}_d{b}"))));
    }
    let known = carried.iter().map(|w| format!("{lw} == 4'd{w}")).collect::<Vec<_>>().join(" || ");
    stmts.push(format!(
        "\n// synthesis translate_off\nif (!({known}) || ({tag}_o & ((5'd1 << {lw}) - 5'd1)) != 0) begin $display(\"FATAL: {tag}: a run of 2^%0d from element %0d is not on its word, or not a width the port carries\", {lw}, {tag}_a); $finish; end\n// synthesis translate_on\n"
    ));
    (wires, stmts.join(" "))
}
/// The image files of tensor `t`'s memory holding `words` in `banks` banks: one per bank and lane, each bank's rows in
/// order, each word at its lane's width.
fn tensor_hex(t: &TensorDecl, words: &[i64], width: u32, banks: usize) -> Vec<(String, String)> {
    let ls = lanes(width);
    let rows = words.len().max(1).div_ceil(banks);
    let mut out = Vec::new();
    for b in 0..banks {
        for (l, (lo, w)) in ls.iter().enumerate() {
            let stem = bank(&format!("t{}", t.id.idx()), b, banks);
            let name = if ls.len() == 1 { format!("{stem}.hex") } else { format!("{stem}_l{l}.hex") };
            let digits = (*w as usize).div_ceil(4);
            let mask = if *w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            let mut body = String::with_capacity(rows * (digits + 1));
            for r in 0..rows {
                let x = words.get(r * banks + b).copied().unwrap_or(0);
                body.push_str(&format!("{:0digits$x}\n", ((x as u64) >> lo) & mask));
            }
            out.push((name, body));
        }
    }
    out
}

/// The compute-bound floor: the busiest site's compute cycles alone (no fill, no stall, no movement).
fn cycle_floor(ctx: &Ctx, plans: &[EnginePlan]) -> u64 {
    let mut per_site: BTreeMap<Site, u64> = BTreeMap::new();
    for plan in plans {
        if plan.engine != Engine::Compute {
            continue;
        }
        let mut total = 0u64;
        let mut mult: Vec<u64> = vec![1];
        for m in &plan.micro {
            match m {
                Micro::LoopStart { var, count, .. } => {
                    let c = if *var == LV_ITEM {
                        plan.item_count
                    } else {
                        *count
                    };
                    let top = *mult.last().unwrap();
                    mult.push(top * c as u64);
                }
                Micro::LoopEnd { .. } => {
                    mult.pop();
                }
                Micro::Fixed(f @ (Fixed::Matmul { .. } | Fixed::Unary { .. })) => {
                    total += ctx.duration(f) * *mult.last().unwrap()
                }
                _ => {}
            }
        }
        *per_site.entry(plan.site).or_default() += total;
    }
    per_site.values().copied().max().unwrap_or(0)
}

// ── Element layout ────────────────────────────────────────────────────────────────────────────────────────

/// The word of a site's image of `t` that holds element `(i, j)` — tiles in row-major order, a row of tiles at the
/// tensor's pitch ([`tensor_pitch`]), each tile's elements row-major within it — and so the address a host writes or
/// reads that element at.
pub fn host_word(p: &SpatialProgram, t: &TensorDecl, i: usize, j: usize) -> usize {
    word_at(p, tensor_pitch(p, t), i, j)
}

/// [`host_word`] at a pitch of `pitch` tiles a tile-row.
fn word_at(p: &SpatialProgram, pitch: usize, i: usize, j: usize) -> usize {
    let (tr, tc) = (p.tile.rows, p.tile.cols);
    let tile = (i / tr) * pitch + j / tc;
    tile * tr * tc + (i % tr) * tc + j % tc
}

/// The words a memory of tensor `t` holds: its tile rows, each `pitch` tiles ([`tensor_pitch`]) of `te` elements.
fn tensor_words(t: &TensorDecl, pitch: usize, te: usize) -> usize {
    t.tiles.rows_t * pitch * te
}

/// Tile-major element order of a tensor's padded tile grid (the memory image every site holds), its rows at the tensor's
/// pitch.
fn tiled_words(
    p: &SpatialProgram,
    t: &TensorDecl,
    values: &dyn Fn(usize, usize) -> i64,
) -> Vec<i64> {
    let pitch = tensor_pitch(p, t);
    let mut out = vec![0i64; tensor_words(t, pitch, p.tile.elems())];
    for i in 0..t.elems.rows {
        for j in 0..t.elems.cols {
            out[word_at(p, pitch, i, j)] = values(i, j);
        }
    }
    out
}

// ── Row pitch ──────────────────────────────────────────────────────────────────────────────────────────────

/// The word a row of `width` elements moves in on a site of `banks` banks: the narrowest power of two that holds the row,
/// no wider than the banks.
fn row_word(width: usize, banks: usize) -> usize {
    width.next_power_of_two().min(banks).max(1)
}

/// ★ A ROW STARTS ON A WORD. A run is aligned to its own width ([`Ctx::align`]), so a row of `w` elements laid end to end
/// after another row of `w` starts wherever `w` leaves it and moves in runs no wider than `w`'s alignment: the decode
/// step's tied head, sixteen rows of fourteen, moves two elements a cycle. A memory that keeps each row at a pitch of
/// whole words — the row's width rounded up to the word it moves in ([`row_word`]) — moves every row in runs of that
/// word; the padding past a row's end is never read.
///
/// The tiles a tile-row of tensor `t` takes in its memory: its columns, or — on a site of one-element tiles and more than
/// one bank, for a tensor of more than one row whose rows are not whole words — its columns rounded up to the word, where
/// every walk the program makes of the tensor keeps within a row ([`walk_splits`]), so that each address it issues has a
/// row and a column the pitch can place. A tensor no walk reads or writes takes the pitch too: only the host addresses it,
/// through [`host_word`].
pub(crate) fn tensor_pitch(p: &SpatialProgram, t: &TensorDecl) -> usize {
    let cols = t.tiles.cols_t;
    let word = row_word(cols, p.mem_banks);
    if p.tile.elems() != 1 || p.mem_banks <= 1 || t.tiles.rows_t <= 1 || cols % word == 0 {
        return cols;
    }
    let splits = tensor_walks(p).into_iter().all(|(op, loops, section)| match op {
        ChoreoOp::ReadTiles { tensor, at, block_h, block_w, stride_w, stride_h, .. }
        | ChoreoOp::WriteTiles { tensor, at, block_h, block_w, stride_w, stride_h, .. } => {
            let coords: Vec<(usize, usize)> = p
                .work
                .items
                .iter()
                .filter(|it| section.is_none() || p.section_of_item(it.id) == section)
                .map(|it| it.coord)
                .collect();
            *tensor != t.id || walk_splits(at, (*block_h, *block_w, *stride_w, *stride_h), &loops, &coords, cols)
        }
        _ => true,
    });
    if splits {
        cols.next_multiple_of(word)
    } else {
        cols
    }
}

/// Every tile walk the program's engines make — each read and write of a tensor — with the counts of the loops that
/// enclose it and the section it belongs to.
fn tensor_walks(p: &SpatialProgram) -> Vec<(&ChoreoOp, BTreeMap<LoopVar, usize>, Option<usize>)> {
    fn visit<'a>(
        p: &SpatialProgram,
        ops: &'a [TaggedOp],
        loops: &mut Vec<(LoopVar, usize)>,
        out: &mut Vec<(&'a ChoreoOp, BTreeMap<LoopVar, usize>, Option<usize>)>,
    ) {
        for t in ops {
            match &t.op {
                ChoreoOp::ReadTiles { .. } | ChoreoOp::WriteTiles { .. } => {
                    out.push((&t.op, loops.iter().copied().collect(), p.section_of_stage(t.stage)))
                }
                ChoreoOp::Loop { var, count, body } => {
                    loops.push((*var, count.bound()));
                    visit(p, body, loops, out);
                    loops.pop();
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for prog in p.schedule.iter().flat_map(|s| &s.programs) {
        visit(p, &prog.ops, &mut Vec::new(), &mut out);
    }
    out
}

/// Whether a walk from `at` — `block_h` rows `stride_h` apart, of `block_w` tiles `stride_w` apart — of a tensor of `cols`
/// tile columns keeps every address it issues within one row: each term's stride is whole rows and a column offset, and
/// the column offsets its terms add up to stay below `cols` at the largest value each term takes — a loop's count less
/// one, the largest of the items' `coords` (the section's work items), a gather's bound. A term of a variable no loop in
/// `loops` binds adds nothing, as it adds nothing to the address the walk issues ([`addr_v`]).
fn walk_splits(
    at: &TileAddr,
    (block_h, block_w, stride_w, stride_h): (usize, usize, usize, usize),
    loops: &BTreeMap<LoopVar, usize>,
    coords: &[(usize, usize)],
    cols: usize,
) -> bool {
    let col = |s: usize| s % cols;
    let mut reach = col(at.base) + block_h.saturating_sub(1) * col(stride_h) + block_w.saturating_sub(1) * col(stride_w);
    for (v, s) in &at.steps {
        let top = if *v == LV_ITEM_Y {
            coords.iter().map(|c| c.0).max().unwrap_or(0)
        } else if *v == LV_ITEM_X {
            coords.iter().map(|c| c.1).max().unwrap_or(0)
        } else {
            loops.get(v).map_or(0, |c| c.saturating_sub(1))
        };
        reach += top * col(*s);
    }
    if let Some(ind) = at.indirect {
        reach += ind.bound.1 / ind.elems_per_tile.max(1) * col(ind.tile_stride);
    }
    reach < cols
}

/// Tile offset `s` of a tensor of `cols` tile columns, in a memory that keeps its rows `pitch` tiles apart: its whole rows
/// at the pitch, its column kept.
fn pitched(s: usize, cols: usize, pitch: usize) -> usize {
    s / cols * pitch + s % cols
}

/// A walk's address at a tensor's pitch: its base, every term's stride and a gather's tile stride each placed by
/// [`pitched`] — exact for a walk that keeps within a row ([`walk_splits`]).
fn pitched_addr(at: &TileAddr, cols: usize, pitch: usize) -> TileAddr {
    TileAddr {
        base: pitched(at.base, cols, pitch),
        steps: at.steps.iter().map(|(v, s)| (*v, pitched(*s, cols, pitch))).collect(),
        indirect: at.indirect.map(|ind| IndirectTile { tile_stride: pitched(ind.tile_stride, cols, pitch), ..ind }),
    }
}

/// ★ A PAGE'S ROWS START ON WORDS TOO. A matmul's B page holds its `ibw` rows of the site's columns end to end, so a row
/// that is not a whole word moves in its alignment's runs into the page and out of it. Each buffer template whose pages
/// a tile walk of contiguous rows fills and a matmul reads only as its B — every fill and every read rows of one width,
/// more than one row a page, the width not a whole word — keeps its pages' rows at that width rounded up to the word
/// ([`row_word`]); every other page keeps its rows end to end. The template's row width and pitch, by template.
fn page_pitches(p: &SpatialProgram) -> BTreeMap<BufferId, (usize, usize)> {
    #[derive(Default)]
    struct Use {
        widths: BTreeSet<usize>,
        rows: usize,
        other: bool,
    }
    fn visit(p: &SpatialProgram, ops: &[TaggedOp], uses: &mut BTreeMap<BufferId, Use>) {
        let tp = |b: BufferId| p.buffer(b).origin.unwrap_or(b);
        let other = |b: BufferId, uses: &mut BTreeMap<BufferId, Use>| uses.entry(tp(b)).or_default().other = true;
        for t in ops {
            match &t.op {
                ChoreoOp::Reserve { .. } | ChoreoOp::Push { .. } | ChoreoOp::Wait { .. } | ChoreoOp::Pop { .. } => {}
                ChoreoOp::ReadTiles { at, block_h, block_w, stride_w, into, .. } => {
                    let u = uses.entry(tp(*into)).or_default();
                    u.widths.insert(*block_w);
                    u.rows = u.rows.max(*block_h);
                    u.other |= *stride_w != 1;
                    if let Some(ind) = at.indirect {
                        other(ind.index, uses);
                    }
                }
                ChoreoOp::WriteTiles { from, at, .. } => {
                    other(*from, uses);
                    if let Some(ind) = at.indirect {
                        other(ind.index, uses);
                    }
                }
                ChoreoOp::Compute { op, ins, acc_reload } => match op.shape() {
                    Ok(ComputeShape::Tiles { block, sub_blocks, .. }) if ins.len() == 2 => {
                        other(ins[0], uses);
                        let u = uses.entry(tp(ins[1])).or_default();
                        u.widths.insert(sub_blocks.1 * block.osw);
                        u.rows = u.rows.max(block.ibw);
                        if let Some(r) = acc_reload {
                            other(*r, uses);
                        }
                    }
                    _ => {
                        for b in ins.iter().chain(acc_reload) {
                            other(*b, uses);
                        }
                    }
                },
                ChoreoOp::PackAcc { to, .. } => other(*to, uses),
                ChoreoOp::McastSend { buf, .. } => other(*buf, uses),
                ChoreoOp::Loop { body, .. } => visit(p, body, uses),
                ChoreoOp::ReadBarrier
                | ChoreoOp::WriteBarrier
                | ChoreoOp::McastSetValid { .. }
                | ChoreoOp::Flush { .. }
                | ChoreoOp::SemSet { .. }
                | ChoreoOp::SemInc { .. }
                | ChoreoOp::SemWait { .. }
                | ChoreoOp::AcquireAcc { .. }
                | ChoreoOp::ReleaseAcc
                | ChoreoOp::Barrier { .. } => {}
            }
        }
    }
    let mut uses: BTreeMap<BufferId, Use> = BTreeMap::new();
    if p.tile.elems() != 1 || p.mem_banks <= 1 {
        return BTreeMap::new();
    }
    for prog in p.schedule.iter().flat_map(|s| &s.programs) {
        visit(p, &prog.ops, &mut uses);
    }
    for st in p.stages.iter().filter(|st| matches!(st.kind, StageKind::Recv { .. })) {
        for b in &st.outs {
            uses.entry(p.buffer(*b).origin.unwrap_or(*b)).or_default().other = true;
        }
    }
    let extents = p.page_extents();
    uses.into_iter()
        .filter_map(|(tpl, u)| {
            let [w] = u.widths.iter().copied().collect::<Vec<_>>()[..] else { return None };
            let word = row_word(w, p.mem_banks);
            let whole_rows = p.buffers.iter().filter(|b| b.origin.unwrap_or(b.id) == tpl).all(|b| extents[b.id.idx()] % w == 0);
            (!u.other && u.rows > 1 && w % word != 0 && whole_rows).then(|| (tpl, (w, w.next_multiple_of(word))))
        })
        .collect()
}

/// `$readmemh` files for the program's input tensors, in the sites' tile-major element order.
pub fn input_hex_files(p: &SpatialProgram, inputs: &Inputs) -> Vec<(String, String)> {
    let ranges = program_ranges(p);
    let mut out = Vec::new();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let Some(v) = inputs.tensors.get(&t.id) else {
            continue;
        };
        let cols = t.elems.cols;
        let words = match v {
            Value::Int(x) => tiled_words(p, t, &|i, j| x.get(i * cols + j).copied().unwrap_or(0)),
            Value::Float(x) => tiled_words(p, t, &|i, j| {
                x.get(i * cols + j).copied().unwrap_or(0.0) as i64
            }),
        };
        out.extend(tensor_hex(t, &words, tensor_width(t, &ranges), p.mem_banks));
    }
    out
}

// ── The printer ───────────────────────────────────────────────────────────────────────────────────────────

fn bits_for(n: usize) -> usize {
    (usize::BITS - n.max(2).saturating_sub(1).leading_zeros()).max(1) as usize
}

fn var_name(v: LoopVar) -> String {
    format!("lv{}", v.0)
}

/// The phase `by` cycles behind the FSM's: `ph − by`, or `ph` itself.
fn ph_less(by: u64) -> String {
    if by == 0 {
        "ph".into()
    } else {
        format!("(ph - {by})")
    }
}

/// `body` from phase `from` on: guarded by the flag of `ph >= from`, or unguarded from phase 0.
fn from_phase(ctx: &Ctx, from: u64, body: &str) -> String {
    if from == 0 {
        body.to_string()
    } else {
        format!("if ({}) begin {body} end", ctx.ph_ge(from))
    }
}

/// The tile index of an address in the FSM's registers.
/// A tile address: its base, each loop or item coordinate times its stride, and — for a gather — the evaluator's
/// `indirect_offset`, `(v div elems_per_tile) · tile_stride` for the index `v` at element 0 of its buffer's front page.
fn addr_v(at: &TileAddr, bound: &BTreeSet<LoopVar>, (oy, ox): (&str, &str), ctx: &Ctx) -> String {
    let mut terms = vec![format!("{}", at.base)];
    if let Some(ind) = at.indirect {
        let ki = ctx.template(ind.index).idx();
        terms.push(format!(
            "(($unsigned(f{ki}_rd_data) / {}) * {})",
            ind.elems_per_tile.max(1),
            ind.tile_stride
        ));
    }
    for (v, stride) in &at.steps {
        if *stride == 0 {
            continue;
        }
        let name = if *v == LV_ITEM_Y {
            oy.to_string()
        } else if *v == LV_ITEM_X {
            ox.to_string()
        } else if bound.contains(v) {
            var_name(*v)
        } else {
            continue;
        };
        terms.push(format!("{name} * {stride}"));
    }
    terms.join(" + ")
}

struct Emit {
    s: String,
}

impl Emit {
    fn l(&mut self, line: impl AsRef<str>) {
        self.s.push_str(line.as_ref());
        self.s.push('\n');
    }
}

/// Every buffer an engine touches, with the roles it takes.
#[derive(Default)]
struct BufUse {
    /// Reserves and pushes: the buffer's producer drives its reserve / push wires.
    produces: BTreeSet<BufferId>,
    /// Waits and pops: the consumer drives its pop wires.
    consumes: BTreeSet<BufferId>,
    writes: BTreeSet<BufferId>,
    reads: BTreeSet<BufferId>,
}

impl BufUse {
    fn all(&self) -> BTreeSet<BufferId> {
        self.produces
            .iter()
            .chain(&self.consumes)
            .chain(&self.writes)
            .chain(&self.reads)
            .copied()
            .collect()
    }
}

fn buffer_uses(micro: &[Micro]) -> BufUse {
    let mut u = BufUse::default();
    for m in micro {
        match m {
            Micro::Guarded(Guard::Reserve { buf, .. }) => {
                u.produces.insert(*buf);
            }
            Micro::Guarded(Guard::Wait { buf, .. }) => {
                u.consumes.insert(*buf);
            }
            Micro::Fixed(f) | Micro::Then(f) => match f {
                Fixed::Push { buf, .. } => {
                    u.produces.insert(*buf);
                }
                Fixed::Pop { buf, .. } => {
                    u.consumes.insert(*buf);
                }
                Fixed::ReadTiles { into, at, .. } => {
                    u.writes.insert(*into);
                    if let Some(ind) = at.indirect {
                        u.reads.insert(ind.index);
                    }
                }
                Fixed::WriteTiles { from, at, .. } => {
                    u.reads.insert(*from);
                    if let Some(ind) = at.indirect {
                        u.reads.insert(ind.index);
                    }
                }
                Fixed::Matmul { ins, reload, .. } => {
                    u.reads.insert(ins[0]);
                    u.reads.insert(ins[1]);
                    if let Some(r) = reload {
                        u.reads.insert(*r);
                    }
                }
                Fixed::Unary { ins, .. } | Fixed::Generic { ins, .. } => {
                    for b in ins {
                        u.reads.insert(*b);
                    }
                }
                Fixed::PackAcc { to, accumulate, .. } => {
                    u.writes.insert(*to);
                    if *accumulate {
                        // Reading the page it adds into.
                        u.reads.insert(*to);
                    }
                }
                Fixed::McastSend { buf, .. } => {
                    u.reads.insert(*buf);
                }
                _ => {}
            },
            _ => {}
        }
    }
    u
}

/// A memory port an op moves data through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Port {
    FifoWr(BufferId),
    FifoRd(BufferId),
    TensorWr(TensorId),
    TensorRd(TensorId),
}

/// ★ THE RUN WIDTHS EACH PORT CARRIES: the width of every run an op of `micro` moves through it ([`Ctx::run`],
/// [`Ctx::acc_run`], [`Ctx::matmul_run`]), and one for every op that moves single elements. A port's lane logic covers
/// exactly these: a memory only element ops touch keeps no lanes, and one whose runs are whole words rotates nothing.
fn port_widths(ctx: &Ctx, micro: &[Micro]) -> BTreeMap<Port, BTreeSet<usize>> {
    fn put(out: &mut BTreeMap<Port, BTreeSet<usize>>, port: Port, w: usize) {
        out.entry(port).or_default().insert(w);
    }
    let mut out = BTreeMap::new();
    for m in micro {
        let (Micro::Fixed(f) | Micro::Then(f)) = m else { continue };
        match f {
            Fixed::ReadTiles { tensor, at, into, .. } => {
                let w = ctx.run(f);
                put(&mut out, Port::TensorRd(*tensor), w);
                put(&mut out, Port::FifoWr(*into), w);
                if let Some(ind) = at.indirect {
                    put(&mut out, Port::FifoRd(ind.index), 1);
                }
            }
            Fixed::WriteTiles { from, tensor, at, .. } => {
                let w = ctx.run(f);
                put(&mut out, Port::FifoRd(*from), w);
                put(&mut out, Port::TensorWr(*tensor), w);
                if let Some(ind) = at.indirect {
                    put(&mut out, Port::FifoRd(ind.index), 1);
                }
            }
            Fixed::PackAcc { to, accumulate, .. } => {
                let w = ctx.acc_run(f);
                put(&mut out, Port::FifoWr(*to), w);
                if *accumulate {
                    put(&mut out, Port::FifoRd(*to), w);
                }
            }
            Fixed::Matmul { block, per_site_n, sub_rows, ins, reload, .. } => {
                let w = ctx.matmul_run(block, ctx.page_row(ins[1], *per_site_n), *per_site_n, *sub_rows);
                for b in ins.iter().chain(reload) {
                    put(&mut out, Port::FifoRd(*b), w);
                }
            }
            Fixed::Unary { ins, .. } => {
                for b in ins {
                    put(&mut out, Port::FifoRd(*b), 1);
                }
            }
            // A Generic slot reads a run of its lanes a point where its map moves along the lane axis, else one element.
            Fixed::Generic { unit, ins, .. } => {
                for (s, b) in ins.iter().enumerate() {
                    put(&mut out, Port::FifoRd(*b), if unit.broadcast(s) || unit.gathers(s) { 1 } else { unit.lanes });
                }
            }
            Fixed::McastSend { buf, .. } => put(&mut out, Port::FifoRd(*buf), 1),
            Fixed::Push { .. }
            | Fixed::Pop { .. }
            | Fixed::Barrier
            | Fixed::AcquireAcc { .. }
            | Fixed::ReleaseAcc
            | Fixed::SemSet { .. }
            | Fixed::SemInc { .. }
            | Fixed::McastSetValid { .. } => {}
        }
    }
    out
}

/// Each reading engine's port on each buffer's FIFO at `site`, numbered in engine order per template, and the ports each
/// template has there. An engine whose vector Generic ops gather from a buffer takes a port for each lane past the first
/// beside its own, numbered after it ([`gather_lanes`]).
fn read_ports(ctx: &Ctx, plans: &[EnginePlan], site: Site) -> (BTreeMap<(usize, BufferId), usize>, BTreeMap<BufferId, usize>) {
    let mut readers: BTreeMap<BufferId, usize> = BTreeMap::new();
    let mut index: BTreeMap<(usize, BufferId), usize> = BTreeMap::new();
    for (ei, plan) in plans.iter().enumerate().filter(|(_, pl)| pl.site == site) {
        let lanes = gather_lanes(ctx, &plan.micro);
        for b in buffer_uses(&plan.micro).reads {
            let n = readers.entry(ctx.template(b)).or_default();
            index.insert((ei, b), *n);
            *n += lanes.get(&ctx.template(b)).copied().unwrap_or(1);
        }
    }
    (index, readers)
}

/// ★ A GATHER TAKES LANES THROUGH PORTS OF ITS OWN. Each buffer template a vector Generic op of `micro` gathers from, with
/// the most lanes any such op takes: lane 0 reads the page through the engine's port, and every lane past it through a
/// port of its own, each at the element its own lane of the index slot names.
pub(super) fn gather_lanes(ctx: &Ctx, micro: &[Micro]) -> BTreeMap<BufferId, usize> {
    let mut out: BTreeMap<BufferId, usize> = BTreeMap::new();
    for m in micro {
        let Micro::Fixed(Fixed::Generic { unit, ins, .. }) = m else { continue };
        if unit.lanes > 1 {
            for s in (0..unit.slots()).filter(|s| unit.gathers(*s)) {
                let lanes = out.entry(ctx.template(ins[s])).or_insert(1);
                *lanes = (*lanes).max(unit.lanes);
            }
        }
    }
    out
}

/// The widths of `port` over the engines `ei` of `widths`, or a single element where nothing moves runs through it.
fn widths_or_element(found: BTreeSet<usize>) -> BTreeSet<usize> {
    if found.is_empty() {
        BTreeSet::from([1])
    } else {
        found
    }
}

fn tensors_read(micro: &[Micro]) -> BTreeSet<TensorId> {
    micro
        .iter()
        .filter_map(|m| {
            if let Micro::Fixed(Fixed::ReadTiles { tensor, .. }) = m {
                Some(*tensor)
            } else {
                None
            }
        })
        .collect()
}
fn tensors_written(micro: &[Micro]) -> BTreeSet<TensorId> {
    micro
        .iter()
        .filter_map(|m| {
            if let Micro::Fixed(Fixed::WriteTiles { tensor, .. }) = m {
                Some(*tensor)
            } else {
                None
            }
        })
        .collect()
}
/// Whether an engine halts for the host: its program holds a link's halt.
fn halts_in(micro: &[Micro]) -> bool {
    micro.iter().any(|m| matches!(m, Micro::Guarded(Guard::Link { .. })))
}
fn sems_local(micro: &[Micro]) -> BTreeSet<SemId> {
    let mut s = BTreeSet::new();
    for m in micro {
        match m {
            Micro::Guarded(Guard::SemWait { sem, .. })
            | Micro::Fixed(Fixed::SemSet { sem, .. })
            | Micro::Then(Fixed::SemSet { sem, .. }) => {
                s.insert(*sem);
            }
            _ => {}
        }
    }
    s
}
fn sem_incs(micro: &[Micro]) -> BTreeSet<SemId> {
    micro
        .iter()
        .filter_map(|m| {
            if let Micro::Fixed(Fixed::SemInc { sem, .. }) | Micro::Then(Fixed::SemInc { sem, .. }) = m {
                Some(*sem)
            } else {
                None
            }
        })
        .collect()
}
fn mcast_valids(micro: &[Micro]) -> BTreeSet<(SemId, GroupId)> {
    micro
        .iter()
        .filter_map(|m| {
            if let Micro::Fixed(Fixed::McastSetValid { sem, group }) | Micro::Then(Fixed::McastSetValid { sem, group }) = m {
                Some((*sem, *group))
            } else {
                None
            }
        })
        .collect()
}
fn mcast_sends(micro: &[Micro]) -> BTreeSet<GroupId> {
    micro
        .iter()
        .filter_map(|m| {
            if let Micro::Fixed(Fixed::McastSend { group, .. }) = m {
                Some(*group)
            } else {
                None
            }
        })
        .collect()
}
fn loop_vars(micro: &[Micro]) -> BTreeSet<LoopVar> {
    micro
        .iter()
        .filter_map(|m| {
            if let Micro::LoopStart { var, .. } = m {
                Some(*var)
            } else {
                None
            }
        })
        .collect()
}

/// The width a PE column's partial sum needs: a product of two `bits`-wide signed operands fits `2·bits` bits — the
/// largest, `(−2^(bits−1))²`, is `2^(2·bits−2)` — and a pass sums at most `k` of them, the array's rows. A sum across
/// passes accumulates in the engine's own 64-bit accumulator, never in the array.
fn pe_acc_bits(bits: u32, k: usize) -> u32 {
    (2 * bits + (usize::BITS - k.max(1).saturating_sub(1).leading_zeros())).min(64)
}

/// The PE array an engine's matmuls run on — the largest of them, `(rows, columns, operand bits)` — if it has any.
fn pe_array(micro: &[Micro]) -> Option<(usize, usize, u32)> {
    micro.iter().fold(None, |acc, m| match m {
        Micro::Fixed(Fixed::Matmul { operand, array, .. }) => {
            let bits = precision_int_bits(*operand).unwrap_or(64);
            Some((array.0, array.1, acc.map_or(bits, |a: (usize, usize, u32)| a.2.max(bits))))
        }
        _ => acc,
    })
}

/// A composition's sections each number their own work items, and an op's item coordinate is the coordinate of the
/// item of ITS section the item loop is at — the evaluator's `item_coord`. The tables that hold them are one site's
/// items, so a composition is lowered onto one site.
fn require_composition_on_one_site(p: &SpatialProgram) -> Result<(), SpatialLowerError> {
    if !p.sections.is_empty() && p.sites().len() > 1 {
        return Err(SpatialLowerError::Unsupported(
            "a composition across more than one site — its item coordinates are decoded per section from one site's items",
        ));
    }
    Ok(())
}

/// The read of a gather's index — element 0 of the front page of its buffer — that a tile address with an indirect term
/// drives in the same cycle, or nothing.
fn index_read(ctx: &Ctx, at: &TileAddr) -> String {
    match at.indirect {
        Some(ind) => {
            let ki = ctx.template(ind.index).idx();
            format!(" f{ki}_rd_slot = f{ki}_rd_slot0; f{ki}_rd_elem = 0;")
        }
        None => String::new(),
    }
}

/// The coordinate expressions an op's tile address reads: an uncomposed program's `oy`/`ox` (the item decoded on the
/// plan's grid), or a section's table looked up at the item loop.
struct CoordNames(String, String);

impl CoordNames {
    fn as_tuple(&self) -> (&str, &str) {
        (&self.0, &self.1)
    }
}

fn item_coord_names(section: Option<usize>) -> CoordNames {
    match section {
        Some(k) => CoordNames(format!("item_y_s{k}(lv5_or_zero)"), format!("item_x_s{k}(lv5_or_zero)")),
        None => CoordNames("oy".into(), "ox".into()),
    }
}

// ── The Generic engine's literals and folds ─────────────────────────────────────────────────────────────

/// A signed 64-bit literal.
fn lit64(v: i64) -> String {
    if v < 0 {
        format!("(-64'sd{})", v.unsigned_abs())
    } else {
        format!("64'sd{v}")
    }
}

/// One step of a fold, as the evaluator's `IntFold` takes it: seeded by the first value along the axis (`first`),
/// `Sum` adds, `Max` keeps the larger, `ArgMax` keeps the first strictly greater and its position `at`. Returns the
/// value and position the fold holds after the step.
fn fold_step(pool: ReducePool, first: &str, val_reg: &str, idx_reg: &str, v: &str, at: &str) -> (String, String) {
    match pool {
        ReducePool::Sum => (format!("({first} ? {v} : ($signed({val_reg}) + {v}))"), "32'd0".into()),
        ReducePool::Max => (
            format!("({first} ? {v} : (($signed({v}) > $signed({val_reg})) ? {v} : $signed({val_reg})))"),
            "32'd0".into(),
        ),
        ReducePool::ArgMax => {
            let take = format!("({first} || ($signed({v}) > $signed({val_reg})))");
            (format!("({take} ? {v} : $signed({val_reg}))"), format!("({take} ? {at} : {idx_reg})"))
        }
    }
}

/// A memory of `words` elements of `width` bits in `banks` banks — element `e` in bank `e mod banks`, at row
/// `e div banks` — each bank in [`lanes`], read `lat` cycles after its address. Read through a registered port (`lat`
/// one or more), each bank takes the store that wastes the least of the part: a bank of one row is a register; a banked
/// memory's bank up to 64 rows is distributed RAM, 32 or 64 rows of a few bits a LUT, where a flop array of even four
/// rows costs more LUTs in its read mux than the RAM does; past 64 rows, and an unbanked memory of more than a few
/// words, block RAM. A bank read combinationally (`lat` zero) is a flop array.
/// How a bank of a memory is held, the choice `memory_decl` makes by yosys's `ram_style`: flip-flops with no
/// attribute when its read is unregistered, else registers for a shallow bank, distributed LUT-RAM for a bank up
/// to 64 rows deep across more than one bank, and block RAM otherwise. The budget reads the same choice per bank,
/// so the resources it counts are the ones the printer declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Storage {
    Flops,
    Registers,
    Distributed,
    Block,
}

impl Storage {
    /// The storage a memory of `words` elements in `banks` banks, registered `lat` times, is printed in.
    fn of(words: usize, banks: usize, lat: u64) -> Storage {
        let rows = words.max(1).div_ceil(banks);
        if lat == 0 {
            Storage::Flops
        } else if rows <= 1 || (banks == 1 && rows <= 4) {
            Storage::Registers
        } else if rows <= 64 && banks > 1 {
            Storage::Distributed
        } else {
            Storage::Block
        }
    }

    /// The `(* ram_style = … *)` prefix the declaration carries, exactly as printed.
    fn attr(self) -> &'static str {
        match self {
            Storage::Flops => "",
            Storage::Registers => "(* ram_style = \"registers\" *) ",
            Storage::Distributed => "(* ram_style = \"distributed\" *) ",
            Storage::Block => "(* ram_style = \"block\" *) ",
        }
    }
}

fn memory_decl(name: &str, words: usize, width: u32, banks: usize, lat: u64) -> String {
    let rows = words.max(1).div_ceil(banks);
    let style = Storage::of(words, banks, lat).attr();
    let ls = lanes(width);
    let mut out = Vec::new();
    for b in 0..banks {
        for (l, (_, w)) in ls.iter().enumerate() {
            out.push(format!("  {style}reg [{}:0] {} [0:{}];", w - 1, lane(&bank(name, b, banks), l, ls.len()), rows - 1));
        }
    }
    out.join("\n")
}

/// The read port `data` of memory `mem` of `width` bits in `banks` banks, at element `addr`, registered `lat` times:
/// `data`, the element sign-extended to 64 bits, and — with more than one bank — `data_hi`, the elements after it in a
/// run, lane after lane. Every bank reads the row `addr` sits at; the element is its own bank's, and a run, which starts
/// at a multiple of its width ([`Ctx::run`]), takes its lanes from the banks of its word. The port carries the run widths
/// `runs` ([`port_widths`]).
fn memory_read(data: &str, mem: &str, addr: &str, width: u32, banks: usize, lat: u64, runs: &BTreeSet<usize>) -> String {
    let ls = lanes(width);
    let read = |m: &str, row: &str, q: &str| -> String {
        let mut s = String::new();
        if lat == 0 {
            let parts: Vec<String> = (0..ls.len()).rev().map(|l| format!("{}[{row}]", lane(m, l, ls.len()))).collect();
            s.push_str(&format!(" wire [{}:0] {q} = {{{}}};", width - 1, parts.join(", ")));
            return s;
        }
        for (l, (_, w)) in ls.iter().enumerate() {
            s.push_str(&format!(" reg [{}:0] {q}_l{l}_q0; always @(posedge clk) {q}_l{l}_q0 <= {}[{row}];", w - 1, lane(m, l, ls.len())));
            for n in 1..lat {
                s.push_str(&format!(" reg [{0}:0] {q}_l{l}_q{n}; always @(posedge clk) {q}_l{l}_q{n} <= {q}_l{l}_q{1};", w - 1, n - 1));
            }
        }
        let parts: Vec<String> = (0..ls.len()).rev().map(|l| format!("{q}_l{l}_q{}", lat - 1)).collect();
        s.push_str(&format!(" wire [{}:0] {q} = {{{}}};", width - 1, parts.join(", ")));
        s
    };
    if banks == 1 {
        let s = format!("{} wire [63:0] {data} = $signed({data}_w);", read(mem, addr, &format!("{data}_w")));
        return format!("  {}", s.trim_start());
    }
    let lg = log2_banks(banks);
    let mut s = format!("  wire [31:0] {data}_a = {addr}; wire [31:0] {data}_r = {data}_a >> {lg}; wire [{}:0] {data}_o0 = {data}_a[{}:0];", lg - 1, lg - 1);
    for b in 0..banks {
        s.push_str(&read(&bank(mem, b, banks), &format!("{data}_r"), &format!("{data}_k{b}")));
    }
    // The element's bank, as its data lands.
    for n in 1..=lat {
        s.push_str(&format!(" reg [{}:0] {data}_o{n}; always @(posedge clk) {data}_o{n} <= {data}_o{};", lg - 1, n - 1));
    }
    // Lane `j` of a run is bank `start | j`. A run that reaches lane `j` is wider than `j` — the narrowest such run the
    // port carries sets the multiple it starts at — so the lane chooses among those banks only; a lane no run the port
    // carries reaches reads nothing.
    for j in 0..banks {
        let Some(step) = runs.iter().copied().filter(|w| *w > j).min() else {
            s.push_str(&format!(" wire [{}:0] {data}_v{j} = {}'d0;", width - 1, width));
            continue;
        };
        if step == banks {
            s.push_str(&format!(" wire [{}:0] {data}_v{j} = {data}_k{j};", width - 1));
            continue;
        }
        let sh = step.trailing_zeros();
        s.push_str(&format!(" reg [{}:0] {data}_v{j}; always @* case ({data}_o{lat}[{}:{sh}])", width - 1, lg - 1));
        for m in 0..banks / step {
            s.push_str(&format!(" {}'d{m}: {data}_v{j} = {data}_k{};", lg - sh, m * step + j));
        }
        s.push_str(" endcase");
    }
    let hi: Vec<String> = (1..banks).rev().map(|j| format!("{data}_v{j}")).collect();
    s.push_str(&format!(" wire [63:0] {data} = $signed({data}_v0); wire [{}:0] {data}_hi = {{{}}};", (banks as u32 - 1) * width - 1, hi.join(", ")));
    s
}

/// The assignment of `name` to `src` delayed `lat` cycles through registers (`src` itself when `lat` is zero).
fn delayed(name: &str, src: &str, width: usize, lat: u64) -> String {
    if lat == 0 {
        return format!(" assign {name} = {src};");
    }
    let mut s = String::new();
    for n in 0..lat {
        let from = if n == 0 { src.to_string() } else { format!("{name}_d{}", n - 1) };
        s.push_str(&format!(" reg [{}:0] {name}_d{n}; always @(posedge clk) {name}_d{n} <= {from};", width - 1));
    }
    s.push_str(&format!(" assign {name} = {name}_d{};", lat - 1));
    s
}

/// `v`, a slot of a ring of `depth` plus at most `depth` more — below `2 · depth`, in `[hi:0]` — wrapped onto the ring: a
/// mask where the depth is a power of two, else one compare and one subtraction at `v`'s own width. A remainder by a
/// constant that is no power of two is a divider as wide as its context.
fn ring_wrap(v: &str, depth: usize, hi: usize) -> String {
    if depth.is_power_of_two() {
        format!("{v} % {depth}")
    } else {
        let w = hi + 1;
        format!("({v} >= {w}'d{depth}) ? {v} - {w}'d{depth} : {v}")
    }
}

/// A page FIFO's control, module `name`: `depth` pages, and the reservations, pushes, waits and pops that are the IR's
/// ops — `free`, `count`, the slot the next fill takes and the slot the next read takes. Its pages live in the pool
/// memory its phase colors it into ([`site_pools`]).
pub fn fifo_control_verilog(name: &str, depth: usize) -> String {
    let (sw, cw) = (bits_for(depth), bits_for(depth + 1));
    // The read slot plus a count or a pop: below twice the depth.
    let aw = bits_for(2 * depth);
    let mut e = Emit { s: String::new() };
    e.l(format!("module {name}("));
    e.l("  input clk, input rst,");
    e.l(format!("  input reserve, input [{cw}:0] reserve_n, input push, input [{cw}:0] push_n, input pop, input [{cw}:0] pop_n,"));
    e.l(format!("  output [{cw}:0] free, output [{cw}:0] count, output [{sw}:0] fill_slot0, output [{sw}:0] rd_slot0"));
    e.l(");");
    e.l(format!("  reg [{cw}:0] cnt, rsv; reg [{sw}:0] rd;"));
    e.l(format!("  wire [{aw}:0] fill_at = rd + cnt, rd_at = rd + pop_n;"));
    e.l(format!("  assign free = {depth} - cnt - rsv;"));
    e.l("  assign count = cnt;");
    e.l(format!("  assign fill_slot0 = {};", ring_wrap("fill_at", depth, aw)));
    e.l("  assign rd_slot0 = rd;");
    e.l("  always @(posedge clk) begin");
    e.l("    if (rst) begin cnt <= 0; rsv <= 0; rd <= 0; end");
    e.l("    else begin");
    e.l("      cnt <= cnt + (push ? push_n : 0) - (pop ? pop_n : 0);");
    e.l("      rsv <= rsv + (reserve ? reserve_n : 0) - (push ? push_n : 0);");
    e.l(format!("      if (pop) rd <= {};", ring_wrap("rd_at", depth, aw)));
    e.l("    end");
    e.l("  end");
    e.l("endmodule");
    e.s
}

/// A pool: the memory every buffer of one color keeps its pages in ([`site_pools`]) — `words` elements of `width` bits in
/// `banks` banks, one write port and `nread` read ports, each `lat` cycles from address to data, the write port carrying
/// the run widths `wr` and read port `r` those of `rd[r]`. A write takes `wr_n` elements of a run of `2^wr_lw` from
/// `wr_addr`; a read returns the rest of its run in `rd_hi`.
#[allow(clippy::too_many_arguments)]
fn print_pool(name: &str, words: usize, width: u32, banks: usize, lat: u64, nread: usize, wr: &BTreeSet<usize>, rd: &[BTreeSet<usize>]) -> String {
    let lg = log2_banks(banks);
    let hi = (banks as u32 - 1) * width;
    let mut e = Emit { s: String::new() };
    let mut ports = vec!["input clk".to_string(), "input rst".into(), "input wr_en".into(), "input [31:0] wr_addr".into(), "input [63:0] wr_data".into()];
    if banks > 1 {
        ports.push(format!("input [{lg}:0] wr_n"));
        ports.push("input [3:0] wr_lw".into());
        ports.push(format!("input [{}:0] wr_hi", hi - 1));
    }
    for r in 0..nread {
        ports.push(format!("input [31:0] rd{r}_addr"));
        ports.push(format!("output [63:0] rd{r}_data"));
        if banks > 1 {
            ports.push(format!("output [{}:0] rd{r}_hi", hi - 1));
        }
    }
    e.l(format!("module {name}({});", ports.join(", ")));
    e.l(memory_decl("mem", words, width, banks, lat));
    for (r, runs) in rd.iter().enumerate() {
        e.l(memory_read(&format!("rd{r}_word"), "mem", &format!("rd{r}_addr"), width, banks, lat, runs));
        e.l(format!("  assign rd{r}_data = rd{r}_word;"));
        if banks > 1 {
            e.l(format!("  assign rd{r}_hi = rd{r}_word_hi;"));
        }
    }
    let (wires, put) = memory_write("wm", "mem", "wr_addr", "wr_n", "wr_lw", "wr_data", "wr_hi", width, banks, wr);
    if !wires.is_empty() {
        e.l(wires);
    }
    e.l(format!("  always @(posedge clk) if (wr_en && !rst) begin {put} end"));
    e.l("endmodule");
    e.s
}

/// One pool of a site: the buffers whose pages it holds, its words and width, its read ports and the run widths each of
/// its ports carries.
struct Pool {
    members: Vec<BufferId>,
    words: usize,
    width: u32,
    nread: usize,
    wr: BTreeSet<usize>,
    rd: Vec<BTreeSet<usize>>,
}

/// The elements one page of `b` takes in its pool: its page, padded to start every page on a word.
fn page_stride(ctx: &Ctx, b: BufferId) -> usize {
    ctx.page_words(b).next_multiple_of(ctx.banks)
}

/// ★ A SITE'S FIFO PAGES LIVE IN POOLS, ONE PER BUFFER A PHASE HAS LIVE AT ONCE. Every engine of a site passes a
/// rendezvous between two phases, and every op's own reads and writes land within its cycles, so two buffers of
/// different phases never move data in the same cycle: they can keep their pages in one memory behind one set of ports.
/// Each phase's buffers, largest first, take pools 0, 1, 2, …, so the site has as many pools as its busiest phase has
/// buffers — not one memory and one set of ports per buffer of the program. Buffers outside every section are all one
/// phase, each in a pool of its own. A pool is as deep and as wide as its largest member, and each of its ports carries
/// every run width its members' ports carry ([`port_widths`]).
///
/// Every read port of a pool is a copy of all of it, so buffers share a pool only with buffers read through as many
/// ports: those read through one take pools 0, 1, 2, … as above, and those read through more — a page a gather's lanes
/// read through ports of their own ([`gather_lanes`]) — take pools of their own after them, by their count of ports.
fn site_pools(ctx: &Ctx, plans: &[EnginePlan], site: Site) -> Vec<Pool> {
    let p = ctx.p;
    let phases = p.section_phases();
    let (rd_index, readers) = read_ports(ctx, plans, site);
    let widths: BTreeMap<usize, BTreeMap<Port, BTreeSet<usize>>> =
        plans.iter().enumerate().filter(|(_, pl)| pl.site == site).map(|(ei, pl)| (ei, port_widths(ctx, &pl.micro))).collect();
    let bus_fed: BTreeSet<BufferId> = p
        .stages
        .iter()
        .filter(|st| st.site == Some(site) && matches!(st.kind, StageKind::Recv { .. }))
        .flat_map(|st| st.outs.iter().copied())
        .collect();
    let nread = |b: BufferId| readers.get(&ctx.template(b)).copied().unwrap_or(1).max(1);
    let mut by_ports: BTreeMap<usize, BTreeMap<Option<usize>, Vec<BufferId>>> = BTreeMap::new();
    for b in p.buffers.iter().filter(|b| b.site == Some(site) && !matches!(b.kind, BufferKind::Accumulator { .. })) {
        by_ports.entry(nread(b.id)).or_default().entry(p.section_of_buffer(b.id).map(|k| phases[k])).or_default().push(b.id);
    }
    let words = |b: BufferId| ctx.depth(b) * page_stride(ctx, b);
    let mut pools: Vec<Pool> = Vec::new();
    for (ports, by_phase) in &mut by_ports {
        let first = pools.len();
        for members in by_phase.values_mut() {
            members.sort_by_key(|b| (std::cmp::Reverse(words(*b) * ctx.buffer_width(*b) as usize), b.idx()));
            for (i, b) in members.iter().enumerate() {
                if pools.len() == first + i {
                    pools.push(Pool { members: Vec::new(), words: 0, width: 1, nread: *ports, wr: BTreeSet::new(), rd: Vec::new() });
                }
                let pool = &mut pools[first + i];
                pool.members.push(*b);
                pool.words = pool.words.max(words(*b));
                pool.width = pool.width.max(ctx.buffer_width(*b));
                for found in widths.values().filter_map(|w| w.get(&Port::FifoWr(*b))) {
                    pool.wr.extend(found);
                }
                if bus_fed.contains(b) {
                    pool.wr.insert(1);
                }
            }
        }
    }
    for pool in &mut pools {
        pool.rd = vec![BTreeSet::new(); pool.nread];
        for ((ei, b), r) in &rd_index {
            if pool.members.contains(b) {
                if let Some(found) = widths.get(ei).and_then(|w| w.get(&Port::FifoRd(*b))) {
                    pool.rd[*r].extend(found);
                }
            }
        }
        pool.wr = widths_or_element(std::mem::take(&mut pool.wr));
        pool.rd = pool.rd.drain(..).map(widths_or_element).collect();
    }
    pools
}

impl SpatialBackend for FabricBackend {
    fn name(&self) -> &str {
        self.target.name
    }
    fn target(&self) -> &SpatialTarget {
        &self.target
    }
    fn lower(&self, p: &SpatialProgram) -> Result<SpatialArtifact, SpatialLowerError> {
        require_verified(p)?;
        require_composition_on_one_site(p)?;
        let ctx = context(p, &self.target)?;
        let plans = plans(&ctx)?;
        let pl = p.placement.as_ref().unwrap();
        if p.semaphores.iter().any(|s| s.init != 0) {
            return Err(SpatialLowerError::Unsupported(
                "a semaphore whose initial value is not the counter's reset value",
            ));
        }
        // A site's items must be consecutive: the engines derive `item_id` as `ITEM_START + lv`.
        if pl
            .items_of_site
            .iter()
            .any(|(_, items)| items.windows(2).any(|w| w[1].idx() != w[0].idx() + 1))
        {
            return Err(SpatialLowerError::Unsupported(
                "a strided item enumeration on the fabric",
            ));
        }
        for t in &p.tensors {
            if precision_int_bits(t.dtype).is_none() {
                return Err(SpatialLowerError::Precision { needed: t.dtype });
            }
        }
        // Refuse a fabric past its part's resources before a line of RTL is printed. Only when the target names a
        // part; an abstract target has no budget to exceed.
        budget::check_capacity(p, &self.target)?;
        let mut rtl = Emit { s: String::new() };
        rtl.l(format!("// spatial_top — {} lowered to our fabric from the spatial IR; every module is one IR object.", p.name));
        rtl.l("`timescale 1ns/1ps");
        // Primitives.
        rtl.s.push_str(&emit_semaphore_verilog("sem_prim", 16));
        // A section rendezvous is its own IR object — the counting semaphore every engine program arrives at between
        // two sections — so it is its own primitive, built on the one semaphore.
        if p.semaphores.iter().any(|s| matches!(s.role, SemRole::Barrier { .. })) {
            rtl.l("// barrier_prim — a section rendezvous: the counting semaphore every engine program arrives at between two sections.");
            rtl.l("module barrier_prim(input clk, input rst, input inc, input [15:0] inc_n, input set, input [15:0] set_val, input [15:0] thresh, output [15:0] count, output ge);");
            rtl.l("  sem_prim u(.clk(clk), .rst(rst), .inc(inc), .inc_n(inc_n), .set(set), .set_val(set_val), .thresh(thresh), .count(count), .ge(ge));");
            rtl.l("endmodule");
        }
        let mut bus_widths: BTreeSet<usize> = BTreeSet::new();
        for g in &pl.groups {
            bus_widths.insert(g.rect.len() - 1);
        }
        for n in &bus_widths {
            rtl.s.push_str(&emit_mcast_bus_verilog(
                &format!("mcast_bus_prim_{n}"),
                80,
                *n,
            ));
        }
        // Buffer templates.
        let mut templates: BTreeMap<BufferId, usize> = BTreeMap::new();
        for b in &p.buffers {
            if matches!(b.kind, BufferKind::Accumulator { .. }) {
                continue;
            }
            templates.entry(ctx.template(b.id)).or_insert(ctx.depth(b.id));
        }
        for (t, depth) in &templates {
            rtl.s.push_str(&fifo_control_verilog(&format!("fifo_tpl_{}", t.idx()), *depth));
        }
        // Engine classes: representative = first plan of the class.
        let mut class_rep: BTreeMap<usize, usize> = BTreeMap::new();
        for (i, plan) in plans.iter().enumerate() {
            class_rep.entry(plan.class).or_insert(i);
        }
        let mut grid_names: BTreeMap<usize, String> = BTreeMap::new();
        for (class, rep) in &class_rep {
            let plan = &plans[*rep];
            // A scalar body's placed grid.
            for m in &plan.micro {
                if let Micro::Fixed(Fixed::Unary {
                    kind: UnaryKind::Scalar(body),
                    ..
                }) = m
                {
                    let g = route_loop_body(&body.cells, body.n_in, &[body.result]);
                    let gname = format!("c{class}");
                    rtl.s.push_str(&emit_grid_verilog(&gname, &g));
                    grid_names.insert(*class, gname);
                    break;
                }
            }
            if let Some((k, n, bits)) = pe_array(&plan.micro) {
                rtl.s.push_str(&emit_systolic_pipelined_loadable_verilog_starting(
                    &format!("pe_engine_{class}"),
                    k,
                    n,
                    bits,
                    pe_acc_bits(bits, k),
                    WeightStart::Zero,
                ));
            }
            let class_plans: Vec<&EnginePlan> = plans.iter().filter(|pl| pl.class == *class).collect();
            rtl.s.push_str(&engine::print_engine(
                &ctx,
                plan,
                &class_plans,
                &format!("engine_{class}"),
                &grid_names,
            )?);
        }
        // Sites.
        let sites: Vec<Site> = pl.items_of_site.iter().map(|(s, _)| *s).collect();
        let mut hex: Vec<(String, String)> = Vec::new();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let cols = t.elems.cols;
            let words = match &t.init {
                TensorInit::Values(v) => {
                    tiled_words(p, t, &|i, j| v.get(i * cols + j).copied().unwrap_or(0))
                }
                TensorInit::Pattern { .. } => {
                    tiled_words(p, t, &|i, j| t.init.element(i * cols + j).unwrap_or(0))
                }
                // External data arrives through `input_hex_files`; the artifact still ships a zero image so
                // the memory's `$readmemh` resolves during elaboration and synthesis.
                _ => vec![0i64; ctx.tensor_words(t)],
            };
            hex.extend(tensor_hex(t, &words, tensor_width(t, &ctx.ranges), ctx.banks));
        }
        for site in &sites {
            rtl.s.push_str(&print_site(&ctx, &plans, *site)?);
        }
        // Top.
        rtl.s.push_str(&print_top(&ctx, &plans, &sites)?);
        let census = Census::of(p);
        let cycle_model = fabric_cycle_model(p, &self.target);
        let floor = cycle_floor(&ctx, &plans);
        let testbench = print_testbench(&ctx, &sites, cycle_model);
        Ok(SpatialArtifact::Verilog(VerilogArtifact {
            top: "spatial_top".into(),
            rtl: rtl.s,
            testbench,
            hex,
            census,
            cycle_model,
            cycle_floor: floor,
        }))
    }
}

/// A site: its tensor memories, its buffers, its semaphores and its engines, with the cross-site signals
/// (semaphore increments, valid sets, multicast data) as ports.
fn print_site(ctx: &Ctx, plans: &[EnginePlan], site: Site) -> Result<String, SpatialLowerError> {
    let p = ctx.p;
    let pl = p.placement.as_ref().unwrap();
    let mine: Vec<(usize, &EnginePlan)> = plans
        .iter()
        .enumerate()
        .filter(|(_, pl)| pl.site == site)
        .collect();
    let bufs: Vec<&Buffer> = p
        .buffers
        .iter()
        .filter(|b| b.site == Some(site) && !matches!(b.kind, BufferKind::Accumulator { .. }))
        .collect();
    // Read-port index per (buffer template, reading engine) at this site.
    let (rd_index, readers) = read_ports(ctx, plans, site);
    let widths: BTreeMap<usize, BTreeMap<Port, BTreeSet<usize>>> = mine.iter().map(|(ei, pl)| (*ei, port_widths(ctx, &pl.micro))).collect();
    let tensor_widths = |ei: usize, port: Port| -> BTreeSet<usize> { widths.get(&ei).and_then(|w| w.get(&port)).cloned().unwrap_or_default() };
    let sems: Vec<&Semaphore> = p
        .semaphores
        .iter()
        .filter(|s| s.sites.contains(&site))
        .collect();
    // Which buffers are fed by a multicast bus (the site is a receiver of a group whose Recv stage fills them).
    let mut bus_fed: BTreeMap<BufferId, GroupId> = BTreeMap::new();
    for st in &p.stages {
        if st.site == Some(site) {
            if let StageKind::Recv { group } = st.kind {
                for b in &st.outs {
                    bus_fed.insert(*b, group);
                }
            }
        }
    }
    // Remote semaphore increments arriving here: every engine (any site) whose micro-program targets (sem, site).
    let mut inc_sources: BTreeMap<SemId, Vec<usize>> = BTreeMap::new();
    let mut valid_sources: BTreeMap<SemId, Vec<(usize, GroupId)>> = BTreeMap::new();
    for (ei, plan) in plans.iter().enumerate() {
        for m in &plan.micro {
            match m {
                Micro::Fixed(Fixed::SemInc { sem, at, .. }) | Micro::Then(Fixed::SemInc { sem, at, .. }) if *at == site => {
                    inc_sources.entry(*sem).or_default().push(ei)
                }
                Micro::Fixed(Fixed::McastSetValid { sem, group }) | Micro::Then(Fixed::McastSetValid { sem, group }) => {
                    let g = &pl.groups[group.idx()];
                    if g.rect.sites().contains(&site) && g.sender != site {
                        valid_sources.entry(*sem).or_default().push((ei, *group));
                    }
                }
                _ => {}
            }
        }
    }
    let mut e = Emit { s: String::new() };
    let mut ports = vec![
        "input clk".to_string(),
        "input rst".to_string(),
        "input start".to_string(),
        "output done".to_string(),
        "input host_we".to_string(),
        "input host_re".to_string(),
        "input [15:0] host_tensor".to_string(),
        "input [31:0] host_addr".to_string(),
        "input [63:0] host_wdata".to_string(),
        "output [63:0] host_rdata".to_string(),
    ];
    for (sem, srcs) in &inc_sources {
        for ei in srcs {
            ports.push(format!("input inc_s{}_from_e{ei}", sem.idx()));
            ports.push(format!("input [15:0] inc_s{}_n_from_e{ei}", sem.idx()));
        }
    }
    for (sem, srcs) in &valid_sources {
        for (ei, g) in srcs {
            ports.push(format!(
                "input valid_s{}_from_e{ei}_g{}",
                sem.idx(),
                g.idx()
            ));
        }
    }
    // Outgoing cross-site pulses from this site's engines.
    for (ei, plan) in &mine {
        for sem in sem_incs(&plan.micro) {
            ports.push(format!("output e{ei}_s{}_inc_remote", sem.idx()));
            ports.push(format!("output [15:0] e{ei}_s{}_inc_n", sem.idx()));
        }
        for (sem, g) in mcast_valids(&plan.micro) {
            ports.push(format!("output e{ei}_mv{}_g{}", sem.idx(), g.idx()));
        }
        for g in mcast_sends(&plan.micro) {
            ports.push(format!("output e{ei}_mc{}_send", g.idx()));
            ports.push(format!("output [79:0] e{ei}_mc{}_din", g.idx()));
        }
    }
    for (b, g) in &bus_fed {
        ports.push(format!("input bus_g{}_push_b{}", g.idx(), b.idx()));
        ports.push(format!("input [79:0] bus_g{}_dout_b{}", g.idx(), b.idx()));
    }
    let halting: Vec<usize> = mine.iter().filter(|(_, pl)| halts_in(&pl.micro)).map(|(ei, _)| *ei).collect();
    if !halting.is_empty() {
        ports.push("input [7:0] link_n".to_string());
        ports.push("output halted".to_string());
        ports.push("output [15:0] halt_at".to_string());
    }
    e.l(format!("module site_{}_{}(", site.x, site.y));
    e.l(format!("  {}", ports.join(",\n  ")));
    e.l(");");
    // Tensor memories: inputs as ROM copies, outputs as this site's RAM, each in the site's banks.
    let banks = ctx.banks;
    let hi_of = |width: u32| (banks as u32 - 1) * width;
    for t in &p.tensors {
        let id = t.id.idx();
        let words = ctx.tensor_words(t);
        let rows = words.max(1).div_ceil(banks);
        let width = tensor_width(t, &ctx.ranges);
        let n = lanes(width).len();
        e.l(memory_decl(&format!("mem_t{id}"), words, width, banks, ctx.lat));
        for b in 0..banks {
            for l in 0..n {
                let mem = lane(&bank(&format!("mem_t{id}"), b, banks), l, n);
                if t.role != TensorRole::Output {
                    let stem = bank(&format!("t{id}"), b, banks);
                    let file = if n == 1 { format!("{stem}.hex") } else { format!("{stem}_l{l}.hex") };
                    e.l(format!("  initial $readmemh(\"{file}\", {mem});"));
                } else {
                    let zi = format!("zi{id}_{}{l}", if banks == 1 { String::new() } else { format!("{b}_") });
                    e.l(format!("  integer {zi}; initial for ({zi} = 0; {zi} < {rows}; {zi} = {zi} + 1) {mem}[{zi}] = 0;"));
                }
            }
        }
    }
    // Buffers.
    for b in &bufs {
        let k = b.id.idx();
        let (sw, ew, cw) = (
            bits_for(ctx.depth(b.id)),
            bits_for(ctx.page_words(b.id)),
            bits_for(ctx.depth(b.id) + 1),
        );
        let nrd = readers
            .get(&ctx.template(b.id))
            .copied()
            .unwrap_or(1)
            .max(1);
        e.l(format!("  wire f{k}_reserve, f{k}_push, f{k}_pop, f{k}_wr_en; wire [{cw}:0] f{k}_reserve_n, f{k}_push_n, f{k}_pop_n, f{k}_free, f{k}_count; wire [{sw}:0] f{k}_fill_slot0, f{k}_rd_slot0, f{k}_wr_slot; wire [{ew}:0] f{k}_wr_elem; wire [63:0] f{k}_wr_data;"));
        let bw = ctx.buffer_width(b.id);
        if banks > 1 {
            e.l(format!("  wire [{}:0] f{k}_wr_n; wire [3:0] f{k}_wr_lw; wire [{}:0] f{k}_wr_hi;", log2_banks(banks), hi_of(bw) - 1));
        }
        for r in 0..nrd {
            e.l(format!("  wire [{sw}:0] f{k}_rd{r}_slot; wire [{ew}:0] f{k}_rd{r}_elem; wire [63:0] f{k}_rd{r}_data;"));
            if banks > 1 {
                e.l(format!("  wire [{}:0] f{k}_rd{r}_hi;", hi_of(bw) - 1));
            }
        }
        e.l(format!(
            "  fifo_tpl_{} fifo_{}_{}_{}(.clk(clk), .rst(rst), .reserve(f{k}_reserve), .reserve_n(f{k}_reserve_n), .push(f{k}_push), .push_n(f{k}_push_n), .pop(f{k}_pop), .pop_n(f{k}_pop_n), .free(f{k}_free), .count(f{k}_count), .fill_slot0(f{k}_fill_slot0), .rd_slot0(f{k}_rd_slot0));",
            ctx.template(b.id).idx(),
            ident(&b.name),
            site.x,
            site.y,
        ));
    }
    // Pools: each pool's ports take its members' requests — one member moves data at a time, and an idle member's
    // port is all zeros, so the requests merge by OR — and hand each member the data its own port reads.
    let pools = site_pools(ctx, plans, site);
    for (i, pool) in pools.iter().enumerate() {
        let pn = format!("pool{i}");
        let gated = |en: &str, v: &str, zero: &str| -> String { format!("({en} ? {v} : {zero})") };
        let addr = |b: BufferId, slot: &str, elem: &str| format!("(f{0}_{slot} * {1} + f{0}_{elem})", b.idx(), page_stride(ctx, b));
        let hi_w = hi_of(pool.width);
        e.l(format!("  wire {pn}_we = {};", pool.members.iter().map(|b| format!("f{}_wr_en", b.idx())).collect::<Vec<_>>().join(" | ")));
        e.l(format!(
            "  wire [31:0] {pn}_wa = {};",
            pool.members.iter().map(|b| gated(&format!("f{}_wr_en", b.idx()), &addr(*b, "wr_slot", "wr_elem"), "32'd0")).collect::<Vec<_>>().join(" | ")
        ));
        e.l(format!(
            "  wire [63:0] {pn}_wd = {};",
            pool.members.iter().map(|b| gated(&format!("f{}_wr_en", b.idx()), &format!("f{}_wr_data", b.idx()), "64'd0")).collect::<Vec<_>>().join(" | ")
        ));
        let mut conn = format!(".clk(clk), .rst(rst), .wr_en({pn}_we), .wr_addr({pn}_wa), .wr_data({pn}_wd)");
        if banks > 1 {
            let lgb = log2_banks(banks);
            e.l(format!(
                "  wire [{lgb}:0] {pn}_wn = {}; wire [3:0] {pn}_wl = {};",
                pool.members.iter().map(|b| gated(&format!("f{}_wr_en", b.idx()), &format!("f{}_wr_n", b.idx()), &format!("{}'d0", lgb + 1))).collect::<Vec<_>>().join(" | "),
                pool.members.iter().map(|b| gated(&format!("f{}_wr_en", b.idx()), &format!("f{}_wr_lw", b.idx()), "4'd0")).collect::<Vec<_>>().join(" | ")
            ));
            e.l(format!(
                "  wire [{}:0] {pn}_wh = {};",
                hi_w - 1,
                pool.members
                    .iter()
                    .map(|b| gated(&format!("f{}_wr_en", b.idx()), &resize_lanes(&format!("f{}_wr_hi", b.idx()), ctx.buffer_width(*b), pool.width, banks - 1), &format!("{hi_w}'d0")))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
            conn.push_str(&format!(", .wr_n({pn}_wn), .wr_lw({pn}_wl), .wr_hi({pn}_wh)"));
        }
        if pool.members.len() > 1 {
            e.l("  // synthesis translate_off");
            e.l(format!(
                "  always @(posedge clk) if ({} > 1) begin $display(\"FATAL: {pn}: two of its buffers write in one cycle\"); $finish; end",
                pool.members.iter().map(|b| format!("f{}_wr_en", b.idx())).collect::<Vec<_>>().join(" + ")
            ));
            e.l("  // synthesis translate_on");
        }
        for r in 0..pool.nread {
            let on: Vec<BufferId> = pool.members.iter().copied().filter(|b| r < readers.get(&ctx.template(*b)).copied().unwrap_or(1).max(1)).collect();
            let ra = if on.is_empty() {
                "32'd0".to_string()
            } else {
                on.iter().map(|b| addr(*b, &format!("rd{r}_slot"), &format!("rd{r}_elem"))).collect::<Vec<_>>().join(" | ")
            };
            e.l(format!("  wire [31:0] {pn}_ra{r} = {ra}; wire [63:0] {pn}_rd{r};"));
            conn.push_str(&format!(", .rd{r}_addr({pn}_ra{r}), .rd{r}_data({pn}_rd{r})"));
            if banks > 1 {
                e.l(format!("  wire [{}:0] {pn}_rh{r};", hi_w - 1));
                conn.push_str(&format!(", .rd{r}_hi({pn}_rh{r})"));
            }
            for b in &on {
                e.l(format!("  assign f{0}_rd{r}_data = {pn}_rd{r};", b.idx()));
                if banks > 1 {
                    e.l(format!("  assign f{0}_rd{r}_hi = {1};", b.idx(), resize_lanes(&format!("{pn}_rh{r}"), pool.width, ctx.buffer_width(*b), banks - 1)));
                }
            }
        }
        e.l(format!("  pool_{}_{}_{i} {pn}_{}_{}({conn});", site.x, site.y, site.x, site.y));
    }
    // Semaphores.
    for s in &sems {
        let k = s.idx_str();
        let incs = inc_sources.get(&s.id).cloned().unwrap_or_default();
        let valids = valid_sources.get(&s.id).cloned().unwrap_or_default();
        let inc_any = if incs.is_empty() {
            "1'b0".to_string()
        } else {
            incs.iter()
                .map(|ei| format!("inc_s{k}_from_e{ei}"))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let inc_sum = if incs.is_empty() {
            "16'd0".to_string()
        } else {
            incs.iter()
                .map(|ei| format!("(inc_s{k}_from_e{ei} ? inc_s{k}_n_from_e{ei} : 16'd0)"))
                .collect::<Vec<_>>()
                .join(" + ")
        };
        let local: Vec<usize> = mine
            .iter()
            .filter(|(_, pl)| sems_local(&pl.micro).contains(&s.id))
            .map(|(ei, _)| *ei)
            .collect();
        let local_set = if local.is_empty() {
            "1'b0".to_string()
        } else {
            local
                .iter()
                .map(|ei| format!("e{ei}_s{k}_set"))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let local_val = local
            .iter()
            .map(|ei| format!("e{ei}_s{k}_set ? e{ei}_s{k}_set_val :"))
            .collect::<Vec<_>>()
            .join(" ");
        let valid_any = if valids.is_empty() {
            "1'b0".to_string()
        } else {
            valids
                .iter()
                .map(|(ei, g)| format!("valid_s{k}_from_e{ei}_g{}", g.idx()))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        for ei in &local {
            e.l(format!(
                "  wire e{ei}_s{k}_set; wire [15:0] e{ei}_s{k}_set_val;"
            ));
        }
        e.l(format!("  wire [15:0] s{k}_count; wire s{k}_set_any = ({local_set}) | ({valid_any}); wire [15:0] s{k}_set_v = {local_val} 16'd1;"));
        let prim = if matches!(s.role, SemRole::Barrier { .. }) { "barrier_prim" } else { "sem_prim" };
        e.l(format!("  {prim} sem_{k}_{}_{}(.clk(clk), .rst(rst), .inc({inc_any}), .inc_n({inc_sum}), .set(s{k}_set_any), .set_val(s{k}_set_v), .thresh(16'd0), .count(s{k}_count), .ge());", site.x, site.y));
        let _ = s.init; // the initial value is applied by the top's reset sequence (a set at start)
    }
    // Engines.
    let mut dones = Vec::new();
    for (ei, plan) in &mine {
        let uses = buffer_uses(&plan.micro);
        let gathered = gather_lanes(ctx, &plan.micro);
        let bufs_used: BTreeSet<BufferId> = uses.all();
        let mut conn = vec![
            ".clk(clk)".to_string(),
            ".rst(rst)".to_string(),
            ".start(start)".to_string(),
            format!(".done(e{ei}_done)"),
        ];
        e.l(format!("  wire e{ei}_done;"));
        dones.push(format!("e{ei}_done"));
        for b in &bufs_used {
            // The port carries the buffer TEMPLATE's number (the module is shared); the wire carries this
            // site's instance.
            let (k, w) = (ctx.template(*b).idx(), b.idx());
            if uses.produces.contains(b) {
                for n in ["reserve", "reserve_n", "push", "push_n", "free"] {
                    conn.push(format!(".f{k}_{n}(f{w}_{n})"));
                }
            }
            if uses.consumes.contains(b) {
                for n in ["pop", "pop_n", "count"] {
                    conn.push(format!(".f{k}_{n}(f{w}_{n})"));
                }
            }
            if uses.writes.contains(b) {
                for n in ["wr_en", "wr_slot", "wr_elem", "wr_data", "fill_slot0"] {
                    conn.push(format!(".f{k}_{n}(f{w}_{n})"));
                }
                if banks > 1 {
                    conn.push(format!(".f{k}_wr_n(f{w}_wr_n), .f{k}_wr_lw(f{w}_wr_lw), .f{k}_wr_hi(f{w}_wr_hi)"));
                }
            }
            if uses.reads.contains(b) {
                let r = rd_index.get(&(*ei, *b)).copied().unwrap_or(0);
                for n in ["slot", "elem", "data"] {
                    conn.push(format!(".f{k}_rd_{n}(f{w}_rd{r}_{n})"));
                }
                for l in 1..gathered.get(&ctx.template(*b)).copied().unwrap_or(1) {
                    for n in ["slot", "elem", "data"] {
                        conn.push(format!(".f{k}_l{l}_rd_{n}(f{w}_rd{}_{n})", r + l));
                    }
                }
                if banks > 1 {
                    conn.push(format!(".f{k}_rd_hi(f{w}_rd{r}_hi)"));
                }
                conn.push(format!(".f{k}_rd_slot0(f{w}_rd_slot0)"));
                conn.push(format!(".f{k}_fill_slot0_r(f{w}_fill_slot0)"));
            }
        }
        let twidth = |t: TensorId| tensor_width(p.tensor(t), &ctx.ranges);
        for t in tensors_read(&plan.micro) {
            e.l(format!("  wire [31:0] e{ei}_t{0}_rd_addr; wire [63:0] e{ei}_t{0}_rd_data;", t.idx()));
            conn.push(format!(".t{0}_rd_addr(e{ei}_t{0}_rd_addr)", t.idx()));
            conn.push(format!(".t{0}_rd_data(e{ei}_t{0}_rd_data)", t.idx()));
            if banks > 1 {
                e.l(format!("  wire [{}:0] e{ei}_t{}_rd_hi;", hi_of(twidth(t)) - 1, t.idx()));
                conn.push(format!(".t{0}_rd_hi(e{ei}_t{0}_rd_hi)", t.idx()));
            }
        }
        for t in tensors_written(&plan.micro) {
            e.l(format!("  wire e{ei}_t{0}_wr_en; wire [31:0] e{ei}_t{0}_wr_addr; wire [63:0] e{ei}_t{0}_wr_data;", t.idx()));
            conn.push(format!(".t{0}_wr_en(e{ei}_t{0}_wr_en)", t.idx()));
            conn.push(format!(".t{0}_wr_addr(e{ei}_t{0}_wr_addr)", t.idx()));
            conn.push(format!(".t{0}_wr_data(e{ei}_t{0}_wr_data)", t.idx()));
            if banks > 1 {
                e.l(format!("  wire [{0}:0] e{ei}_t{1}_wr_n; wire [3:0] e{ei}_t{1}_wr_lw; wire [{2}:0] e{ei}_t{1}_wr_hi;", log2_banks(banks), t.idx(), hi_of(twidth(t)) - 1));
                conn.push(format!(".t{0}_wr_n(e{ei}_t{0}_wr_n), .t{0}_wr_lw(e{ei}_t{0}_wr_lw), .t{0}_wr_hi(e{ei}_t{0}_wr_hi)", t.idx()));
            }
        }
        for s in sems_local(&plan.micro) {
            let k = s.idx();
            conn.push(format!(".s{k}_count(s{k}_count)"));
            conn.push(format!(".s{k}_set(e{ei}_s{k}_set)"));
            conn.push(format!(".s{k}_set_val(e{ei}_s{k}_set_val)"));
        }
        for s in sem_incs(&plan.micro) {
            conn.push(format!(".s{0}_inc_remote(e{ei}_s{0}_inc_remote)", s.idx()));
            conn.push(format!(".s{0}_inc_n(e{ei}_s{0}_inc_n)", s.idx()));
        }
        for (s, g) in mcast_valids(&plan.micro) {
            conn.push(format!(".mv{0}_g{1}(e{ei}_mv{0}_g{1})", s.idx(), g.idx()));
        }
        for g in mcast_sends(&plan.micro) {
            conn.push(format!(".mc{0}_send(e{ei}_mc{0}_send)", g.idx()));
            conn.push(format!(".mc{0}_din(e{ei}_mc{0}_din)", g.idx()));
        }
        if halting.contains(ei) {
            e.l(format!("  wire e{ei}_halted; wire [15:0] e{ei}_halt_at;"));
            conn.push(format!(".link_n(link_n), .halted(e{ei}_halted), .halt_at(e{ei}_halt_at)"));
        }
        e.l(format!(
            "  engine_{} #(.ITEM_START({}), .ITEM_COUNT({})) engine_inst_{ei}({});",
            plan.class,
            plan.item_start,
            plan.item_count,
            conn.join(", ")
        ));
    }
    if !halting.is_empty() {
        e.l(format!("  assign halted = {};", halting.iter().map(|ei| format!("e{ei}_halted")).collect::<Vec<_>>().join(" | ")));
        let at: String = halting.iter().map(|ei| format!("e{ei}_halted ? e{ei}_halt_at : ")).collect();
        e.l(format!("  assign halt_at = {at}16'd0;"));
    }
    // Each tensor memory's ports. The host's write takes the memory's write port — the first writing engine's,
    // merged — and its read the first reading engine's read port, or a port of its own where no engine reads the
    // tensor: the host reaches a memory only while the fabric is idle or halted (every engine then waits at a
    // rendezvous), so neither ever contends with an engine, and every memory keeps one write port and one read port
    // beside any more engines it serves.
    let mut host_words: Vec<String> = Vec::new();
    for t in &p.tensors {
        let id = t.id.idx();
        let writers: Vec<usize> = mine.iter().filter(|(_, pl)| tensors_written(&pl.micro).contains(&t.id)).map(|(ei, _)| *ei).collect();
        let readers: Vec<usize> = mine.iter().filter(|(_, pl)| tensors_read(&pl.micro).contains(&t.id)).map(|(ei, _)| *ei).collect();
        let width = tensor_width(t, &ctx.ranges);
        let mem = format!("mem_t{id}");
        e.l(format!("  wire t{id}_hw = host_we && host_tensor == 16'd{id}; wire t{id}_hr = host_re && host_tensor == 16'd{id};"));
        // A host's write is one element: a run of one, whatever the banks.
        let zero_hi = format!("{}'d0", hi_of(width).max(1));
        let put = |e: &mut Emit, tag: &str, en: &str, addr: &str, n: &str, lw: &str, data: &str, hi: &str, runs: &BTreeSet<usize>| {
            let (wires, stmts) = memory_write(tag, &mem, addr, n, lw, data, hi, width, banks, runs);
            if !wires.is_empty() {
                e.l(wires);
            }
            e.l(format!("  always @(posedge clk) if ({en}) begin {stmts} end"));
        };
        match writers.split_first() {
            Some((w0, more)) => {
                e.l(format!("  wire [31:0] t{id}_wa = t{id}_hw ? host_addr : e{w0}_t{id}_wr_addr; wire [63:0] t{id}_wd = t{id}_hw ? host_wdata : e{w0}_t{id}_wr_data;"));
                let (n, lw, hi) = if banks > 1 {
                    e.l(format!(
                        "  wire [{0}:0] t{id}_wn = t{id}_hw ? {0}'d1 : e{w0}_t{id}_wr_n; wire [3:0] t{id}_wl = t{id}_hw ? 4'd0 : e{w0}_t{id}_wr_lw; wire [{1}:0] t{id}_wh = t{id}_hw ? {zero_hi} : e{w0}_t{id}_wr_hi;",
                        log2_banks(banks),
                        hi_of(width) - 1
                    ));
                    (format!("t{id}_wn"), format!("t{id}_wl"), format!("t{id}_wh"))
                } else {
                    (String::new(), String::new(), String::new())
                };
                // The first writer's port is also the host's, which writes an element.
                let mut runs = tensor_widths(*w0, Port::TensorWr(t.id));
                runs.insert(1);
                put(&mut e, &format!("t{id}_w0"), &format!("t{id}_hw || e{w0}_t{id}_wr_en"), &format!("t{id}_wa"), &n, &lw, &format!("t{id}_wd"), &hi, &runs);
                for w in more {
                    put(
                        &mut e,
                        &format!("t{id}_w{w}"),
                        &format!("e{w}_t{id}_wr_en"),
                        &format!("e{w}_t{id}_wr_addr"),
                        &format!("e{w}_t{id}_wr_n"),
                        &format!("e{w}_t{id}_wr_lw"),
                        &format!("e{w}_t{id}_wr_data"),
                        &format!("e{w}_t{id}_wr_hi"),
                        &widths_or_element(tensor_widths(*w, Port::TensorWr(t.id))),
                    );
                }
            }
            None => put(&mut e, &format!("t{id}_wh"), &format!("t{id}_hw"), "host_addr", "1", "4'd0", "host_wdata", &zero_hi, &BTreeSet::from([1])),
        }
        let first = match readers.first() {
            Some(r0) => format!("t{id}_hr ? host_addr : e{r0}_t{id}_rd_addr"),
            None => "host_addr".to_string(),
        };
        // The first reader's port is also the host's, which reads an element.
        let mut first_runs = readers.first().map(|r0| tensor_widths(*r0, Port::TensorRd(t.id))).unwrap_or_default();
        first_runs.insert(1);
        e.l(memory_read(&format!("t{id}_rd0"), &mem, &first, width, banks, ctx.lat, &first_runs));
        if let Some((r0, more)) = readers.split_first() {
            e.l(format!("  assign e{r0}_t{id}_rd_data = t{id}_rd0;"));
            if banks > 1 {
                e.l(format!("  assign e{r0}_t{id}_rd_hi = t{id}_rd0_hi;"));
            }
            for r in more {
                e.l(memory_read(&format!("t{id}_rd_e{r}"), &mem, &format!("e{r}_t{id}_rd_addr"), width, banks, ctx.lat, &widths_or_element(tensor_widths(*r, Port::TensorRd(t.id)))));
                e.l(format!("  assign e{r}_t{id}_rd_data = t{id}_rd_e{r};"));
                if banks > 1 {
                    e.l(format!("  assign e{r}_t{id}_rd_hi = t{id}_rd_e{r}_hi;"));
                }
            }
        }
        host_words.push(format!("(host_sel == 16'd{id}) ? t{id}_rd0 :"));
    }
    // The word a host read returns, selected by the tensor it named `lat` cycles before, when its data lands.
    e.l(format!("  wire [15:0] host_sel;{}", delayed("host_sel", "host_tensor", 16, ctx.lat)));
    e.l(format!("  assign host_rdata = {} 64'd0;", host_words.join(" ")));
    // Tie off every buffer port no engine and no bus drives.
    let mut driven_prod: BTreeSet<BufferId> = BTreeSet::new();
    let mut driven_cons: BTreeSet<BufferId> = BTreeSet::new();
    let mut driven_wr: BTreeSet<BufferId> = BTreeSet::new();
    let mut driven_rd: BTreeSet<BufferId> = BTreeSet::new();
    for (_, plan) in &mine {
        let u = buffer_uses(&plan.micro);
        driven_prod.extend(u.produces.iter().copied());
        driven_cons.extend(u.consumes.iter().copied());
        driven_wr.extend(u.writes.iter().copied());
        driven_rd.extend(u.reads.iter().copied());
    }
    driven_wr.extend(bus_fed.keys().copied());
    for b in &bufs {
        let k = b.id.idx();
        if !driven_prod.contains(&b.id) {
            e.l(format!("  assign f{k}_reserve = 1'b0; assign f{k}_reserve_n = 0; assign f{k}_push = 1'b0; assign f{k}_push_n = 0;"));
        }
        if !driven_cons.contains(&b.id) {
            e.l(format!("  assign f{k}_pop = 1'b0; assign f{k}_pop_n = 0;"));
        }
        if !driven_wr.contains(&b.id) {
            e.l(format!("  assign f{k}_wr_en = 1'b0; assign f{k}_wr_slot = 0; assign f{k}_wr_elem = 0; assign f{k}_wr_data = 64'd0;"));
            if banks > 1 {
                e.l(format!("  assign f{k}_wr_n = 1; assign f{k}_wr_lw = 0; assign f{k}_wr_hi = 0;"));
            }
        }
        let nrd = readers
            .get(&ctx.template(b.id))
            .copied()
            .unwrap_or(1)
            .max(1);
        for r in 0..nrd {
            if !driven_rd.contains(&b.id) {
                e.l(format!(
                    "  assign f{k}_rd{r}_slot = 0; assign f{k}_rd{r}_elem = 0;"
                ));
            }
        }
    }
    // A bus-fed buffer's write port comes from the bus.
    for (b, g) in &bus_fed {
        let k = b.idx();
        e.l(format!("  assign f{k}_wr_en = bus_g{0}_push_b{k}; assign f{k}_wr_slot = f{k}_fill_slot0; assign f{k}_wr_elem = bus_g{0}_dout_b{k}[{1}:64]; assign f{k}_wr_data = bus_g{0}_dout_b{k}[63:0];", g.idx(), 64 + bits_for(ctx.page_elems(*b))));
        if banks > 1 {
            e.l(format!("  assign f{k}_wr_n = 1; assign f{k}_wr_lw = 0; assign f{k}_wr_hi = 0;"));
        }
    }
    e.l(format!(
        "  assign done = {};",
        if dones.is_empty() {
            "1'b1".to_string()
        } else {
            dones.join(" & ")
        }
    ));
    e.l("endmodule");
    let modules: String = pools
        .iter()
        .enumerate()
        .map(|(i, pool)| print_pool(&format!("pool_{}_{}_{i}", site.x, site.y), pool.words, pool.width, banks, ctx.lat, pool.nread, &pool.wr, &pool.rd))
        .collect();
    Ok(format!("{modules}{}", e.s))
}

trait SemIdx {
    fn idx_str(&self) -> String;
}
impl SemIdx for Semaphore {
    fn idx_str(&self) -> String {
        self.id.idx().to_string()
    }
}

fn ident(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn print_top(ctx: &Ctx, plans: &[EnginePlan], sites: &[Site]) -> Result<String, SpatialLowerError> {
    let p = ctx.p;
    let pl = p.placement.as_ref().unwrap();
    let mut e = Emit { s: String::new() };
    // The host port: while the fabric is idle, a host writes any tensor's word — into every site's copy of it — and reads
    // any word back from the site it names, `lat` cycles after its address. A chip's program that halts for the host
    // also takes its continues (`link_go`, a pulse) and says when it is halted, and at which link.
    e.l("module spatial_top(input clk, input rst, input start, output done, output reg [31:0] cycles,");
    e.l("  input host_we, input host_re, input [15:0] host_site, input [15:0] host_tensor, input [31:0] host_addr,");
    if ctx.halts.is_empty() {
        e.l("  input [63:0] host_wdata, output [63:0] host_rdata);");
    } else {
        e.l("  input [63:0] host_wdata, output [63:0] host_rdata, input link_go, output halted, output [15:0] halt_at);");
        // The continues of this step: the reset that starts a step clears them.
        e.l("  reg [7:0] link_n; always @(posedge clk) if (rst) link_n <= 8'd0; else if (link_go) link_n <= link_n + 8'd1;");
    }
    // Cross-site wires.
    for (ei, plan) in plans.iter().enumerate() {
        for s in sem_incs(&plan.micro) {
            e.l(format!(
                "  wire e{ei}_s{0}_inc_remote; wire [15:0] e{ei}_s{0}_inc_n;",
                s.idx()
            ));
        }
        for (s, g) in mcast_valids(&plan.micro) {
            e.l(format!("  wire e{ei}_mv{}_g{};", s.idx(), g.idx()));
        }
        for g in mcast_sends(&plan.micro) {
            e.l(format!(
                "  wire e{ei}_mc{0}_send; wire [79:0] e{ei}_mc{0}_din;",
                g.idx()
            ));
        }
    }
    // Multicast buses: one per group, sender = the engine that sends on it.
    for g in &pl.groups {
        let n = g.rect.len() - 1;
        let sender = plans
            .iter()
            .enumerate()
            .find(|(_, pl)| mcast_sends(&pl.micro).contains(&g.id))
            .map(|(ei, _)| ei);
        let Some(sender) = sender else { continue };
        e.l(format!(
            "  wire [{}:0] bus_g{}_push; wire [79:0] bus_g{}_dout;",
            n - 1,
            g.id.idx(),
            g.id.idx()
        ));
        e.l(format!("  mcast_bus_prim_{n} mcast_bus_{}(.clk(clk), .rst(rst), .send(e{sender}_mc{0}_send), .din(e{sender}_mc{0}_din), .push(bus_g{0}_push), .dout(bus_g{0}_dout));", g.id.idx()));
    }
    let mut dones = Vec::new();
    let mut site_words = Vec::new();
    let mut halted_sites: Vec<Site> = Vec::new();
    for (si, site) in sites.iter().enumerate() {
        let mut conn = vec![
            ".clk(clk)".to_string(),
            ".rst(rst)".to_string(),
            ".start(start)".to_string(),
            format!(".done(site_done_{}_{})", site.x, site.y),
            ".host_we(host_we)".to_string(),
            format!(".host_re(host_re && host_site == 16'd{si})"),
            ".host_tensor(host_tensor)".to_string(),
            ".host_addr(host_addr)".to_string(),
            ".host_wdata(host_wdata)".to_string(),
            format!(".host_rdata(host_rdata_s{si})"),
        ];
        e.l(format!("  wire [63:0] host_rdata_s{si};"));
        site_words.push(format!("(host_from == 16'd{si}) ? host_rdata_s{si} :"));
        e.l(format!("  wire site_done_{}_{};", site.x, site.y));
        dones.push(format!("site_done_{}_{}", site.x, site.y));
        if plans.iter().any(|pl| pl.site == *site && halts_in(&pl.micro)) {
            e.l(format!("  wire site_halted_{0}_{1}; wire [15:0] site_halt_at_{0}_{1};", site.x, site.y));
            conn.push(format!(".link_n(link_n), .halted(site_halted_{0}_{1}), .halt_at(site_halt_at_{0}_{1})", site.x, site.y));
            halted_sites.push(*site);
        }
        // Incoming increments.
        for (ei, plan) in plans.iter().enumerate() {
            for m in &plan.micro {
                match m {
                    Micro::Fixed(Fixed::SemInc { sem, at, .. }) | Micro::Then(Fixed::SemInc { sem, at, .. }) if *at == *site => {
                        conn.push(format!(
                            ".inc_s{0}_from_e{ei}(e{ei}_s{0}_inc_remote)",
                            sem.idx()
                        ));
                        conn.push(format!(
                            ".inc_s{0}_n_from_e{ei}(e{ei}_s{0}_inc_n)",
                            sem.idx()
                        ));
                    }
                    Micro::Fixed(Fixed::McastSetValid { sem, group }) | Micro::Then(Fixed::McastSetValid { sem, group }) => {
                        let g = &pl.groups[group.idx()];
                        if g.rect.sites().contains(site) && g.sender != *site {
                            conn.push(format!(
                                ".valid_s{0}_from_e{ei}_g{1}(e{ei}_mv{0}_g{1})",
                                sem.idx(),
                                group.idx()
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        // Outgoing pulses.
        for (ei, plan) in plans.iter().enumerate().filter(|(_, pl)| pl.site == *site) {
            for s in sem_incs(&plan.micro) {
                conn.push(format!(
                    ".e{ei}_s{0}_inc_remote(e{ei}_s{0}_inc_remote)",
                    s.idx()
                ));
                conn.push(format!(".e{ei}_s{0}_inc_n(e{ei}_s{0}_inc_n)", s.idx()));
            }
            for (s, g) in mcast_valids(&plan.micro) {
                conn.push(format!(
                    ".e{ei}_mv{0}_g{1}(e{ei}_mv{0}_g{1})",
                    s.idx(),
                    g.idx()
                ));
            }
            for g in mcast_sends(&plan.micro) {
                conn.push(format!(".e{ei}_mc{0}_send(e{ei}_mc{0}_send)", g.idx()));
                conn.push(format!(".e{ei}_mc{0}_din(e{ei}_mc{0}_din)", g.idx()));
            }
        }
        // Bus-fed buffers at this site.
        for st in &p.stages {
            if st.site == Some(*site) {
                if let StageKind::Recv { group } = st.kind {
                    let g = &pl.groups[group.idx()];
                    let receivers: Vec<Site> = g
                        .rect
                        .sites()
                        .into_iter()
                        .filter(|s| *s != g.sender)
                        .collect();
                    let idx = receivers.iter().position(|s| s == site).unwrap_or(0);
                    for b in &st.outs {
                        conn.push(format!(
                            ".bus_g{0}_push_b{1}(bus_g{0}_push[{idx}])",
                            group.idx(),
                            b.idx()
                        ));
                        conn.push(format!(
                            ".bus_g{0}_dout_b{1}(bus_g{0}_dout)",
                            group.idx(),
                            b.idx()
                        ));
                    }
                }
            }
        }
        e.l(format!(
            "  site_{}_{} site_inst_{}_{}({});",
            site.x,
            site.y,
            site.x,
            site.y,
            conn.join(", ")
        ));
    }
    e.l(format!("  assign done = {};", dones.join(" & ")));
    e.l(format!("  wire [15:0] host_from;{}", delayed("host_from", "host_site", 16, ctx.lat)));
    e.l(format!("  assign host_rdata = {} 64'd0;", site_words.join(" ")));
    e.l("  reg started;");
    e.l("  always @(posedge clk) begin");
    e.l("    if (rst) begin cycles <= 0; started <= 1'b0; end");
    if ctx.halts.is_empty() {
        e.l("    else begin if (start) started <= 1'b1; if ((start || started) && !done) cycles <= cycles + 1; end");
    } else {
        // The count pauses while the fabric waits for the host: it is the cycles the program has run.
        e.l("    else begin if (start) started <= 1'b1; if ((start || started) && !done && !halted) cycles <= cycles + 1; end");
    }
    e.l("  end");
    if !ctx.halts.is_empty() {
        let names = |f: &str| -> Vec<String> { halted_sites.iter().map(|s| format!("{f}_{}_{}", s.x, s.y)).collect() };
        e.l(format!("  assign halted = {};", names("site_halted").join(" | ")));
        let at: String = halted_sites.iter().map(|s| format!("site_halted_{0}_{1} ? site_halt_at_{0}_{1} : ", s.x, s.y)).collect();
        e.l(format!("  assign halt_at = {at}16'd0;"));
    }
    e.l("endmodule");
    Ok(e.s)
}

fn print_testbench(ctx: &Ctx, sites: &[Site], predicted: u64) -> String {
    let p = ctx.p;
    let pl = p.placement.as_ref().unwrap();
    // Every result the program hands back — each `Output` and each `State` it carries to its next step, or the stored
    // tensor of a program that only updates in place — printed whole, each tile from the site whose work item stores
    // it (the items of the section that writes it, in a composition).
    let mut outs: Vec<&TensorDecl> =
        p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)).collect();
    if outs.is_empty() {
        outs.extend(p.output_tensor());
    }
    let (tr, tc) = (ctx.tr, ctx.tc);
    let owner_of = |out: &TensorDecl| -> BTreeMap<usize, Site> {
        let section = p.sections.iter().find(|s| s.output == Some(out.id));
        let mut owner = BTreeMap::new();
        for (site, items) in &pl.items_of_site {
            for it in items {
                if section.is_some_and(|s| !(s.items.0..s.items.1).contains(&it.idx())) {
                    continue;
                }
                let r = p.work.items[it.idx()].out;
                for ty in r.row0_t..r.row0_t + r.h_t {
                    for tx in r.col0_t..r.col0_t + r.w_t {
                        owner.insert(ty * out.tiles.cols_t + tx, *site);
                    }
                }
            }
        }
        owner
    };
    let mut e = Emit { s: String::new() };
    e.l("`timescale 1ns/1ps");
    e.l("module tb;");
    e.l("  reg clk = 0, rst = 1, start = 0; wire done; wire [31:0] cycles;");
    e.l("  spatial_top dut(.clk(clk), .rst(rst), .start(start), .done(done), .cycles(cycles), .host_we(1'b0), .host_re(1'b0),");
    e.l("    .host_site(16'd0), .host_tensor(16'd0), .host_addr(32'd0), .host_wdata(64'd0), .host_rdata());");
    e.l("  always #5 clk = ~clk;");
    e.l("  integer guard;");
    e.l("  initial begin");
    e.l("    #12 rst = 0; #10 start = 1; #10 start = 0;");
    e.l("    guard = 0;");
    // The watchdog is sized from the design's OWN predicted cycle count, not a round number. A fixed
    // twenty-million-cycle bound is not a bound in practice: on a multi-site design that is hours of event
    // evaluation, so a testbench that should fail in seconds instead parks a whole test run. Eight times the
    // model, floored, is generous enough that a correct design never trips it and small enough that a
    // deadlocked one reports rather than hangs.
    let guard_bound = predicted.saturating_mul(8).max(20_000);
    e.l(format!(
        "    while (!done && guard < {guard_bound}) begin #10 guard = guard + 1; end"
    ));
    e.l("    #10;");
    e.l("    $display(\"CYCLES %0d\", cycles);");
    for out in outs {
        let owner = owner_of(out);
        for i in 0..out.elems.rows {
            for j in 0..out.elems.cols {
                let tile = (i / tr) * out.tiles.cols_t + j / tc;
                let site = owner.get(&tile).copied().unwrap_or(sites[0]);
                let addr = host_word(p, out, i, j);
                let word = memory_word(
                    &format!("dut.site_inst_{}_{}.mem_t{}", site.x, site.y, out.id.idx()),
                    addr,
                    tensor_width(out, &ctx.ranges),
                    ctx.banks,
                );
                e.l(format!("    $display(\"OUT {} {} %0d\", $signed({word}));", out.id.idx(), i * out.elems.cols + j));
            }
        }
    }
    e.l(format!("    if (done) $display(\"PASS: spatial_top done\"); else $display(\"FAIL: timeout after %0d cycles (bound {guard_bound}, model {predicted})\", cycles);"));
    e.l("    $finish;");
    e.l("  end");
    e.l("endmodule");
    e.s
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/spatial/lower/fabric/tests.rs"]
mod tests;
