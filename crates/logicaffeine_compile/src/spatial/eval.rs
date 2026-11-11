//! `eval_choreography` — the executable MEANING of a placed + scheduled spatial program.
//!
//! Every engine program runs as a coroutine over bounded FIFOs, semaphores and accumulators with tt-metal's
//! enabling rules (a reserve needs free capacity, a push needs a reservation carrying data, a wait needs
//! pages, a multicast needs every receiver's reservation, a semaphore wait needs its value). A round in
//! which no program can step while some are unfinished is a `Deadlock`; every protocol slip is a typed
//! `ChoreoFault`. Five schedulers must agree (Kahn determinacy, executed). The numerics per tile are the
//! systolic references: integer operands accumulate in wrapping i64 (`I32` truncates at pack), float
//! operands accumulate in f32 K-ascending exactly as `eval_systolic_bf16`, and a spill to a narrower
//! intermediate rounds exactly where the schedule spills. This is the golden oracle every lowering is
//! proven against.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::ir::*;
use super::passes::{LV_ITEM, LV_ITEM_X, LV_ITEM_Y, LV_SUB_H, LV_SUB_W};
use super::target::{precision_int_bits, precision_is_int};
use crate::codegen_sva::fabric_rtl::{eval_decision_fabric_heap, DecisionFabric};
use crate::codegen_sva::fabric_target::Precision;
use crate::codegen_sva::simd::{f16_to_f32, f32_to_f16};
use crate::codegen_sva::systolic::{round_bf16, trunc_signed};

/// The refusal an element-level SCATTER earns: an indirect term on a compute stage's OUTPUT map. Gathers (an
/// indirect term on an operand map, an `IndirectTile` on a Load) and tile scatters (an `IndirectTile` on a
/// Store) have semantics; an element scatter does not, and nothing lowers one.
pub const INDIRECT_MAP_DEFERRED: &str = "indirect access (B-P5): an element-level scatter (an indirect term on a compute's output map) has no evaluation semantics";

/// A tensor's elements, row-major over its element shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Int(Vec<i64>),
    Float(Vec<f64>),
}

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub tensors: BTreeMap<TensorId, Value>,
}

impl Inputs {
    /// Every tensor of `p` but its outputs, materialized from its recorded fill: a float fill as floats, every other
    /// as integers. Refuses, naming it, a tensor whose fill is not deterministic (data the compiler cannot see).
    pub fn from_fills(p: &SpatialProgram) -> Result<Inputs, String> {
        let mut inputs = Inputs::default();
        for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
            let n = t.elems.rows * t.elems.cols;
            let value = if t.init.is_float() {
                (0..n).map(|i| t.init.element_f64(i)).collect::<Option<Vec<f64>>>().map(Value::Float)
            } else {
                (0..n).map(|i| t.init.element(i)).collect::<Option<Vec<i64>>>().map(Value::Int)
            };
            inputs.tensors.insert(t.id, value.ok_or_else(|| format!("`{}` has no deterministic fill", t.name))?);
        }
        Ok(inputs)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outputs {
    pub tensors: BTreeMap<TensorId, Value>,
    pub steps: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Trace {
    /// Peak pages resident (queued + filled-pending) per buffer.
    pub high_water: BTreeMap<BufferId, usize>,
    pub steps: u64,
    /// Tiles each tensor had read, per section (`0` for a program of one workload): what the run actually moved.
    pub tiles_read: BTreeMap<(TensorId, usize), u64>,
    /// The least and the greatest integer each buffer's pages held.
    pub held: BTreeMap<BufferId, (i64, i64)>,
    /// For each stage running an integer Generic body, the least and the greatest value each of its nodes computed —
    /// in [`TensorBody::post_order`], both arms of a select at every point, `None` for a node that never computed one.
    pub nodes: BTreeMap<StageId, Vec<Option<(i64, i64)>>>,
    /// The least and the greatest integer each compute stage kept in its accumulator: a contraction's and a fold's
    /// partial results, a matmul's partial sums, the values it wrote.
    pub stage_held: BTreeMap<StageId, (i64, i64)>,
}

fn widen(slot: Option<(i64, i64)>, v: i64) -> (i64, i64) {
    slot.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheduler {
    RoundRobin,
    Reverse,
    Seeded(u64),
    MovementFirst,
    ComputeFirst,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChoreoFault {
    NotScheduled,
    MissingInput(TensorId),
    /// A trip's input holds a value the program does not admit ([`Trip::count_of`]) — refused before any op runs,
    /// against the admitted range it names, so no loop ever runs a count its bound was not proved for.
    TripOutOfRange {
        trip: TripId,
        value: f64,
        admitted: (i64, i64),
    },
    Deadlock {
        runnable: Vec<(Site, Engine)>,
        blocked_at: Vec<(Site, Engine, usize, ChoreoOp)>,
    },
    BufferOverflow {
        buf: BufferId,
        at: (Site, Engine, usize),
    },
    Underflow {
        buf: BufferId,
        at: (Site, Engine, usize),
    },
    PushWithoutData {
        buf: BufferId,
        at: (Site, Engine, usize),
    },
    /// A consumer read a page it never waited for (hardware would read stale local memory).
    ReadBeforeWait {
        buf: BufferId,
        at: (Site, Engine, usize),
    },
    /// A consumer popped more pages than it had waited for.
    PopBeforeWait {
        buf: BufferId,
        at: (Site, Engine, usize),
    },
    Unconsumed {
        buf: BufferId,
        pages_left: usize,
    },
    OutstandingReservation {
        buf: BufferId,
        pages: usize,
    },
    McastIntoUnreserved {
        group: GroupId,
        receiver: Site,
    },
    SemaphoreProtocol {
        sem: SemId,
        expected: SemCond,
        found: usize,
    },
    AccOverflow {
        requested: usize,
        available: usize,
    },
    /// A page was written into local memory an ALIAS of the buffer still holds: a page the alias pushed and its
    /// consumer has not popped. Two buffers sharing one region (`Buffer::alias_of`) keep separate counters, so
    /// nothing else stops a producer of one from overwriting what the consumer of the other is about to read.
    AliasClobber {
        buf: BufferId,
        alias: BufferId,
        at: (Site, Engine, usize),
    },
    AccProtocol {
        at: (Site, Engine, usize),
    },
    StoreConflict {
        tensor: TensorId,
        tile: usize,
        first: Site,
        second: Site,
    },
    OutputTileNeverStored {
        tensor: TensorId,
        tile: usize,
    },
    ReadOutOfRange {
        tensor: TensorId,
        tile: usize,
    },
    /// A CARRIED tensor was read out of step with its writer: a section after the writing section read
    /// a tile the writer has not produced yet (last step's value), or a section before it read a tile
    /// the writer had already overwritten (this step's value).
    ///
    /// ★ THIS FAULT EXISTS BECAUSE STATE CANNOT FAULT ANY OTHER WAY. An activation starts a run EMPTY,
    /// so reading it early is `ReadOutOfRange` and the evaluator notices for free. A state tensor
    /// starts a run FULL — that is what carrying means — so an unordered read returns a perfectly
    /// well-formed number from the previous token, and every check downstream passes. Dropping a
    /// barrier would be silently, plausibly wrong. So the read is checked against the section the
    /// writer lives in, which is a fact about the program, not about this scheduler's luck.
    StaleStateRead {
        tensor: TensorId,
        tile: usize,
        read_in: usize,
        written_in: usize,
        already_written: bool,
    },
    NonDeterministic {
        scheduler_a: &'static str,
        scheduler_b: &'static str,
        tensor: TensorId,
    },
    /// A GATHER read an index outside its declared extent — `[0, modulus)` of its `IndirectTerm` — or one
    /// that is not a whole number. ★ AN OUT-OF-RANGE INDEX IS A FAULT, NEVER A WRAP. A token id past the
    /// vocabulary, a cache position past the cache: a wrapped or clamped index reads SOMEBODY ELSE'S row
    /// and every check downstream passes. So the data run stops, naming the value and the extent.
    GatherOutOfRange {
        at: (Site, Engine, usize),
        index: f64,
        extent: usize,
    },
    /// An integer stage's arithmetic left `i64` — a product, a sum or a shifted value past ±2⁶³ — or divided by zero.
    /// The VM carries such a value exactly, as a big integer, or refuses the division; an evaluator that wrapped or
    /// rounded would report another number as the answer, so the run stops here, naming the operation.
    IntegerArithmetic {
        at: (Site, Engine, usize),
        what: &'static str,
    },
    /// A TILE gather or scatter (`IndirectTile`) read an index outside its declared `bound`, or one that is not
    /// a whole number. The same promise as [`ChoreoFault::GatherOutOfRange`] at tile granularity: a row that
    /// is not the table's is never read, and a write that has nowhere legal to land is never silently dropped.
    TileGatherOutOfBound {
        at: (Site, Engine, usize),
        index: f64,
        bound: (usize, usize),
    },
    Unsupported(String),
}

#[derive(Clone, Debug)]
enum TileData {
    Int(Vec<i64>),
    Float(Vec<f64>),
    /// A token without data — the dry run carries choreography, never arithmetic.
    Empty,
}

type Page = Vec<TileData>;

#[derive(Clone, Debug)]
enum AccData {
    Int(Vec<i64>),
    F32(Vec<f32>),
}

struct FifoState {
    capacity: usize,
    queue: VecDeque<Page>,
    pending: VecDeque<Page>,
    reserved: usize,
    high_water: usize,
    /// Pages at the front the consumer has waited for and not yet popped — the only pages it may read.
    visible: usize,
    /// Pages popped since the program began. With the front page being the only one a compute may read,
    /// `(buffer, popped)` names exactly WHICH page an accumulation folded in — the identity the accumulator
    /// protocol needs and a boolean cannot carry.
    popped: usize,
    /// The backing store the pages live in, addressed by the write pointer: a page the producer overwrites
    /// or ACCUMULATES INTO is the one at the slot it is about to fill, whether or not the consumer has
    /// already drained it. Only an accumulating packer reads this, and it is what makes the partial sum stay
    /// in local memory across K blocks.
    slots: Vec<Page>,
    wr: usize,
}

impl FifoState {
    fn resident(&self) -> usize {
        self.queue.len() + self.pending.len()
    }
}

struct TensorStore {
    dtype: Precision,
    cols_t: usize,
    tiles: Vec<Option<TileData>>,
    /// The site and section of each tile's latest store.
    stored_by: Vec<Option<(Site, usize)>>,
    /// For a CARRIED tensor: the sections whose `Store`s produce this step's value, in order. `None` for every
    /// other role — nothing else is read out of step.
    writer_sections: Option<Vec<usize>>,
    /// For a carried tensor: the section that last wrote each tile this step, `None` while it holds last step's.
    written_in: Vec<Option<usize>>,
    /// The dry run's item window cut short an item loop that writes this tensor, so a tile it has not
    /// written may be one the elided items write: an unwritten in-range tile is not provably absent.
    elided: bool,
}

#[derive(Clone, Debug)]
enum Instr {
    /// An op, TAGGED with the stage it belongs to. The tag is what makes `item_coord` section-scoped:
    /// a composed program's item list is every section's concatenated, and an item loop must index its
    /// OWN section's items or every section after the first addresses the wrong tiles.
    Op(ChoreoOp, StageId),
    LoopStart {
        var: LoopVar,
        count: usize,
        end: usize,
    },
    LoopEnd {
        start: usize,
    },
}

struct Coroutine {
    site: Site,
    engine: Engine,
    code: std::rc::Rc<Vec<Instr>>,
    pc: usize,
    loops: Vec<(LoopVar, usize, usize)>,
    env: BTreeMap<LoopVar, usize>,
    acc: Option<Vec<AccData>>,
    /// The held accumulator has been computed into since it was acquired (packing a fresh one is a fault).
    acc_computed: bool,
    /// Operand pages already folded into the held accumulator, as `(buffer, pop cursor)`.
    ///
    /// A reduction legitimately computes many times inside ONE accumulator window — that is how the K loop
    /// accumulates — so "compute at most once per window" would reject a correct program. What separates a
    /// real repeat from a duplicated op is that the real one consumes FRESH pages: a pop and a new wait sit
    /// between them, so the operands advance. Folding a page that is already in this set is the mutation
    /// that silently doubles a partial sum.
    acc_folded: std::collections::BTreeSet<(usize, usize)>,
    done: bool,
}

/// Add each tensor's `WriteTiles` executions in flattened `code`, per section — every op counted once per iteration
/// of the loops around it — to `out`.
fn count_writes(p: &SpatialProgram, code: &[Instr], out: &mut [BTreeMap<usize, u64>]) {
    let mut mult: Vec<u64> = vec![1];
    for i in code {
        match i {
            Instr::LoopStart { count, .. } => {
                let m = *mult.last().expect("an outer multiplicity");
                mult.push(m * *count as u64);
            }
            Instr::LoopEnd { .. } => {
                mult.pop();
            }
            Instr::Op(ChoreoOp::WriteTiles { tensor, .. }, stage) => {
                *out[tensor.idx()]
                    .entry(p.section_of_stage(*stage).unwrap_or(0))
                    .or_default() += *mult.last().expect("an outer multiplicity")
            }
            _ => {}
        }
    }
}

/// Flatten a tagged choreography into straight-line code, each loop run to a trip counted `counts[trip]`. Under
/// `item_cap`, every item loop runs at most `cap` rounds, and `elided[t]` is set for each tensor `t` a cut-short loop
/// writes.
fn flatten(
    ops: &[TaggedOp],
    out: &mut Vec<Instr>,
    counts: &[usize],
    item_cap: Option<usize>,
    elided: &mut [bool],
) {
    for t in ops {
        match &t.op {
            ChoreoOp::Loop { var, count, body } => {
                let start = out.len();
                let full = count.at(counts);
                // A token-only run walks the item loop within its provable window (see `certificate::item_window`);
                // every engine and every group member caps the same way, so the choreography stays balanced. The
                // window is a bound on the TOKEN protocol, which is periodic past it; the data the elided rounds
                // write is not modelled, so the tensors they write are recorded.
                let n = match item_cap {
                    Some(cap) if *var == LV_ITEM => full.min(cap),
                    _ => full,
                };
                if n < full {
                    walk_tagged_ops(body, 1, &mut |op, _| {
                        if let ChoreoOp::WriteTiles { tensor, .. } = op {
                            elided[tensor.idx()] = true;
                        }
                    });
                }
                out.push(Instr::LoopStart {
                    var: *var,
                    count: n,
                    end: 0,
                });
                flatten(body, out, counts, item_cap, elided);
                let end = out.len();
                out.push(Instr::LoopEnd { start });
                if let Instr::LoopStart { end: e, .. } = &mut out[start] {
                    *e = end;
                }
            }
            other => out.push(Instr::Op(other.clone(), t.stage)),
        }
    }
}

fn scheduler_name(s: Scheduler) -> &'static str {
    match s {
        Scheduler::RoundRobin => "round-robin",
        Scheduler::Reverse => "reverse",
        Scheduler::Seeded(_) => "seeded",
        Scheduler::MovementFirst => "movement-first",
        Scheduler::ComputeFirst => "compute-first",
    }
}

/// Element-wise sum of two pages — the packer's L1 accumulate. A missing or empty base is zero.
fn add_pages(base: &Page, add: &Page) -> Page {
    if base.len() != add.len() {
        return add.clone();
    }
    base.iter()
        .zip(add)
        .map(|(b, a)| match (b, a) {
            (TileData::Int(x), TileData::Int(y)) => {
                TileData::Int(x.iter().zip(y).map(|(u, v)| u.wrapping_add(*v)).collect())
            }
            (TileData::Float(x), TileData::Float(y)) => {
                TileData::Float(x.iter().zip(y).map(|(u, v)| u + v).collect())
            }
            (TileData::Empty, other) | (other, TileData::Empty) => other.clone(),
            (_, y) => y.clone(),
        })
        .collect()
}

/// One tile as a buffer of precision `p` holds it — the rounding a load performs on the way in.
fn narrow(tile: TileData, p: Precision) -> TileData {
    match tile {
        TileData::Float(v) => TileData::Float(v.into_iter().map(|x| round_to(p, x)).collect()),
        other => other,
    }
}

/// A value as the datapath of precision `p` stores it (bf16 / fp16 / fp32 rounding; integers untouched).
pub fn round_to(p: Precision, x: f64) -> f64 {
    match p {
        Precision::Bf16 => round_bf16(x),
        Precision::Fp16 => f16_to_f32(f32_to_f16(x as f32)) as f64,
        Precision::Fp32 => x as f32 as f64,
        _ => x,
    }
}

struct Eval<'a> {
    p: &'a SpatialProgram,
    t: usize,
    fifos: Vec<FifoState>,
    tensors: Vec<TensorStore>,
    sems: BTreeMap<(SemId, Site), usize>,
    recv_buffer: BTreeMap<(GroupId, Site), BufferId>,
    programs: Vec<Coroutine>,
    /// The work items of each site within each section, which every tile address of an item loop indexes.
    site_items: SiteItems<'a>,
    /// Per tensor, the `WriteTiles` each section's schedule runs, and how many have run.
    writes_expected: Vec<BTreeMap<usize, u64>>,
    writes_done: Vec<BTreeMap<usize, u64>>,
    steps: u64,
    /// Token-only: pages carry no data and compute ops do no arithmetic (every protocol check still runs).
    dry: bool,
    /// Tiles read per `(tensor, section)`.
    tiles_read: BTreeMap<(TensorId, usize), u64>,
    /// Per buffer, the other buffers whose pages live in the same local memory (`Buffer::alias_of`).
    partners: Vec<Vec<BufferId>>,
    held: BTreeMap<BufferId, (i64, i64)>,
    nodes: BTreeMap<StageId, Vec<Option<(i64, i64)>>>,
    stage_held: BTreeMap<StageId, (i64, i64)>,
}

impl<'a> Eval<'a> {
    fn new(p: &'a SpatialProgram, inputs: &Inputs) -> Result<Eval<'a>, ChoreoFault> {
        let counts = trip_counts(p, inputs)?;
        Self::build(p, inputs, false, &counts)
    }

    fn new_dry(p: &'a SpatialProgram, counts: &[usize]) -> Result<Eval<'a>, ChoreoFault> {
        Self::build(p, &Inputs::default(), true, counts)
    }

    fn build(
        p: &'a SpatialProgram,
        inputs: &Inputs,
        dry: bool,
        counts: &[usize],
    ) -> Result<Eval<'a>, ChoreoFault> {
        let sched = p.schedule.as_ref().ok_or(ChoreoFault::NotScheduled)?;
        let t = p.tile.rows;
        assert_eq!(p.tile.rows, p.tile.cols, "square native tiles");
        let mut store_sections: Vec<Vec<usize>> = vec![Vec::new(); p.tensors.len()];
        for s in &p.stages {
            if let StageKind::Store { tensor, .. } = &s.kind {
                if let Some(v) = store_sections.get_mut(tensor.idx()) {
                    v.push(p.section_of_stage(s.id).unwrap_or(0));
                }
            }
        }
        let mut tensors = Vec::new();
        for td in &p.tensors {
            let n_tiles = td.tiles.count();
            // ★ A ROLE DECIDES WHETHER A RUN STARTS WITH THE DATA. An `Input`, a `Param` and the
            // carried `State` do; an `Output` and an `Activation` do not, which is why a section that
            // reads an activation before its producer ran faults instead of reading zeros.
            let writer_sections = (td.role == TensorRole::State).then(|| {
                let mut w = store_sections.get(td.id.idx()).cloned().unwrap_or_default();
                w.sort_unstable();
                w.dedup();
                w
            });
            let mut store = TensorStore {
                dtype: td.dtype,
                cols_t: td.tiles.cols_t,
                tiles: vec![None; n_tiles],
                stored_by: vec![None; n_tiles],
                writer_sections,
                written_in: vec![None; n_tiles],
                elided: false,
            };
            if td.role.starts_filled() && dry {
                store.tiles = vec![Some(TileData::Empty); n_tiles];
            } else if td.role.starts_filled() {
                let value = match inputs.tensors.get(&td.id) {
                    Some(v) => v.clone(),
                    None => {
                        let n = td.elems.rows * td.elems.cols;
                        if td.init.is_float() {
                            let vals: Option<Vec<f64>> =
                                (0..n).map(|i| td.init.element_f64(i)).collect();
                            Value::Float(vals.ok_or(ChoreoFault::MissingInput(td.id))?)
                        } else {
                            let vals: Option<Vec<i64>> =
                                (0..n).map(|i| td.init.element(i)).collect();
                            Value::Int(vals.ok_or(ChoreoFault::MissingInput(td.id))?)
                        }
                    }
                };
                let (rows, cols) = (td.elems.rows, td.elems.cols);
                let is_int = precision_is_int(td.dtype);
                let bits = precision_int_bits(td.dtype).unwrap_or(64);
                for rt in 0..td.tiles.rows_t {
                    for ct in 0..td.tiles.cols_t {
                        let mut ints = Vec::new();
                        let mut floats = Vec::new();
                        for r in 0..t {
                            for c in 0..t {
                                let (er, ec) = (rt * t + r, ct * t + c);
                                let inside = er < rows && ec < cols;
                                let idx = er * cols + ec;
                                match (&value, is_int) {
                                    (Value::Int(v), true) => ints.push(if inside {
                                        trunc_signed(v[idx], bits)
                                    } else {
                                        0
                                    }),
                                    (Value::Int(v), false) => floats.push(if inside {
                                        round_to(td.dtype, v[idx] as f64)
                                    } else {
                                        0.0
                                    }),
                                    (Value::Float(v), false) => floats.push(if inside {
                                        round_to(td.dtype, v[idx])
                                    } else {
                                        0.0
                                    }),
                                    (Value::Float(v), true) => ints.push(if inside {
                                        trunc_signed(v[idx] as i64, bits)
                                    } else {
                                        0
                                    }),
                                }
                            }
                        }
                        let tile = if is_int {
                            TileData::Int(ints)
                        } else {
                            TileData::Float(floats)
                        };
                        store.tiles[rt * td.tiles.cols_t + ct] = Some(tile);
                    }
                }
            }
            tensors.push(store);
        }
        let fifos = p
            .buffers
            .iter()
            .map(|b| FifoState {
                capacity: b.kind.depth_pages(),
                slots: vec![Vec::new(); b.kind.depth_pages().max(1)],
                wr: 0,
                popped: 0,
                queue: VecDeque::new(),
                pending: VecDeque::new(),
                reserved: 0,
                high_water: 0,
                visible: 0,
            })
            .collect();
        let mut recv_buffer = BTreeMap::new();
        for s in &p.stages {
            if let (StageKind::Recv { group }, Some(site)) = (&s.kind, s.site) {
                if let Some(out) = s.outs.first() {
                    recv_buffer.insert((*group, site), *out);
                }
            }
        }
        let mut programs = Vec::new();
        let mut writes_expected = vec![BTreeMap::new(); p.tensors.len()];
        let mut elided = vec![false; p.tensors.len()];
        for prog in &sched.programs {
            let mut code = Vec::new();
            flatten(
                &prog.ops,
                &mut code,
                counts,
                if dry {
                    Some(crate::spatial::certificate::item_window(p))
                } else {
                    None
                },
                &mut elided,
            );
            count_writes(p, &code, &mut writes_expected);
            programs.push(Coroutine {
                site: prog.site,
                engine: prog.engine,
                code: std::rc::Rc::new(code),
                pc: 0,
                loops: Vec::new(),
                env: BTreeMap::new(),
                acc: None,
                acc_computed: false,
                acc_folded: Default::default(),
                done: false,
            });
        }
        for (store, cut) in tensors.iter_mut().zip(elided) {
            store.elided = cut;
        }
        let writes_done = vec![BTreeMap::new(); p.tensors.len()];
        let mut regions: BTreeMap<BufferId, Vec<BufferId>> = BTreeMap::new();
        for b in &p.buffers {
            if let Some(owner) = b.alias_of {
                regions
                    .entry(owner)
                    .or_insert_with(|| vec![owner])
                    .push(b.id);
            }
        }
        let mut partners = vec![Vec::new(); p.buffers.len()];
        for sharing in regions.values() {
            for &b in sharing {
                partners[b.idx()] = sharing.iter().copied().filter(|o| *o != b).collect();
            }
        }
        Ok(Eval {
            p,
            t,
            fifos,
            tensors,
            sems: BTreeMap::new(),
            recv_buffer,
            programs,
            site_items: p.site_items(),
            writes_expected,
            writes_done,
            steps: 0,
            dry,
            tiles_read: BTreeMap::new(),
            partners,
            held: BTreeMap::new(),
            nodes: BTreeMap::new(),
            stage_held: BTreeMap::new(),
        })
    }

    fn item_coord(
        &self,
        site: Site,
        env: &BTreeMap<LoopVar, usize>,
        stage: StageId,
    ) -> (usize, usize) {
        let items = self.site_items.at(self.p.section_of_stage(stage), site);
        let idx = env.get(&LV_ITEM).copied().unwrap_or(0);
        match items.get(idx) {
            Some(id) => self.p.work.items[id.idx()].coord,
            None => (0, 0),
        }
    }

    /// Resolve a tile address under the program's loop environment plus the current item's coordinates,
    /// without materializing an extended environment. An `IndirectTile` adds `(v div elems_per_tile) ·
    /// tile_stride` for the index `v` at the front of its buffer — see [`Self::indirect_offset`].
    fn resolve_addr(
        &self,
        site: Site,
        env: &BTreeMap<LoopVar, usize>,
        addr: &TileAddr,
        stage: StageId,
        at: (Site, Engine, usize),
    ) -> Result<usize, ChoreoFault> {
        let (oy, ox) = self.item_coord(site, env, stage);
        let mut a = addr.base;
        for (v, stride) in &addr.steps {
            let idx = if *v == LV_ITEM_Y {
                oy
            } else if *v == LV_ITEM_X {
                ox
            } else {
                env.get(v).copied().unwrap_or(0)
            };
            a += idx * stride;
        }
        if let Some(ind) = &addr.indirect {
            a += self.indirect_offset(ind, at)?;
        }
        Ok(a)
    }

    /// The tile offset an `IndirectTile` contributes: `(v div elems_per_tile) · tile_stride`, `v` the integer at
    /// element 0 of the index buffer's front page (which the schedule waited on).
    ///
    /// ★ THE INDEX IS CHECKED, NEVER TRUSTED. A `v` outside `bound`, or not a whole number, is
    /// `TileGatherOutOfBound` — never a wrapped or clamped row. The token-only dry run has no index to read, so
    /// it takes the LOW end of the bound here and [`Self::check_indirect_span`] walks the high end: a
    /// certificate is issued only when every index the bound admits addresses tiles the tensor has.
    fn indirect_offset(
        &self,
        ind: &IndirectTile,
        at: (Site, Engine, usize),
    ) -> Result<usize, ChoreoFault> {
        let v = if self.dry {
            ind.bound.0 as f64
        } else {
            let page = self.front_page(ind.index, at)?;
            match page.first() {
                Some(TileData::Int(v)) => v.first().copied().unwrap_or(0) as f64,
                Some(TileData::Float(v)) => v.first().copied().unwrap_or(0.0),
                _ => {
                    return Err(ChoreoFault::Unsupported(
                        "an indirect tile index page holds no data".into(),
                    ))
                }
            }
        };
        if v.fract() != 0.0 || v < ind.bound.0 as f64 || v > ind.bound.1 as f64 {
            return Err(ChoreoFault::TileGatherOutOfBound {
                at,
                index: v,
                bound: ind.bound,
            });
        }
        Ok((v as usize / ind.elems_per_tile) * ind.tile_stride)
    }

    /// The dry run's other half of an indirect address: the block at the HIGH end of the bound must lie inside
    /// the tensor too. `base` is the address resolved at the low end.
    fn check_indirect_span(
        &self,
        addr: &TileAddr,
        base: usize,
        block: (usize, usize, usize, usize),
        tensor: TensorId,
    ) -> Result<(), ChoreoFault> {
        let Some(ind) = &addr.indirect else {
            return Ok(());
        };
        let (block_h, block_w, stride_h, stride_w) = (block.0, block.1, block.2, block.3);
        let low = (ind.bound.0 / ind.elems_per_tile) * ind.tile_stride;
        let high = (ind.bound.1 / ind.elems_per_tile) * ind.tile_stride;
        let top = base - low + high;
        let last =
            top + block_h.saturating_sub(1) * stride_h + block_w.saturating_sub(1) * stride_w;
        let tiles = self.tensors[tensor.idx()].tiles.len();
        if last >= tiles {
            return Err(ChoreoFault::ReadOutOfRange { tensor, tile: last });
        }
        Ok(())
    }

    /// The buffer whose local memory `buf`'s pages live in: its own, or the one it aliases.
    fn storage(&self, buf: BufferId) -> BufferId {
        self.p.buffer(buf).alias_of.unwrap_or(buf)
    }

    /// A page written into `slot` of `buf`'s memory must not overwrite a page an alias of that memory still holds:
    /// one it pushed and its consumer has not yet popped.
    fn check_alias_clobber(
        &self,
        buf: BufferId,
        slot: usize,
        at: (Site, Engine, usize),
    ) -> Result<(), ChoreoFault> {
        let len = self.fifos[self.storage(buf).idx()].slots.len().max(1);
        for &alias in &self.partners[buf.idx()] {
            let f = &self.fifos[alias.idx()];
            if (f.popped..f.wr).any(|n| n % len == slot) {
                return Err(ChoreoFault::AliasClobber { buf, alias, at });
            }
        }
        Ok(())
    }

    fn touch(&mut self, buf: BufferId) {
        let f = &mut self.fifos[buf.idx()];
        f.high_water = f.high_water.max(f.resident());
        // Every page enters a buffer as a pending page, each followed by this call.
        let mut held = self.held.get(&buf).copied();
        for tile in f.pending.iter().flatten() {
            if let TileData::Int(v) = tile {
                for x in v {
                    held = Some(widen(held, *x));
                }
            }
        }
        if let Some(h) = held {
            self.held.insert(buf, h);
        }
    }

    /// Notes every integer the program's accumulator holds against `stage`.
    fn note_acc(&mut self, pi: usize, stage: StageId) {
        let Some(acc) = self.programs[pi].acc.as_ref() else { return };
        let mut held = self.stage_held.get(&stage).copied();
        for a in acc {
            if let AccData::Int(v) = a {
                for x in v {
                    held = Some(widen(held, *x));
                }
            }
        }
        if let Some(h) = held {
            self.stage_held.insert(stage, h);
        }
    }

    /// Try to execute one op; `Ok(true)` stepped, `Ok(false)` blocked.
    fn step(&mut self, pi: usize) -> Result<bool, ChoreoFault> {
        let (site, engine, pc) = {
            let c = &self.programs[pi];
            if c.done {
                return Ok(false);
            }
            (c.site, c.engine, c.pc)
        };
        let at = (site, engine, pc);
        let code = std::rc::Rc::clone(&self.programs[pi].code);
        let Some(instr) = code.get(pc) else {
            self.programs[pi].done = true;
            return Ok(true);
        };
        match instr {
            Instr::LoopStart { var, count, end } => {
                let (var, count, end) = (*var, *count, *end);
                let c = &mut self.programs[pi];
                if count == 0 {
                    c.pc = end + 1;
                } else {
                    c.loops.push((var, 0, count));
                    c.env.insert(var, 0);
                    c.pc += 1;
                }
                Ok(true)
            }
            Instr::LoopEnd { start } => {
                let start = *start;
                let c = &mut self.programs[pi];
                let (var, iter, count) = c.loops.pop().expect("loop frame");
                if iter + 1 < count {
                    c.loops.push((var, iter + 1, count));
                    c.env.insert(var, iter + 1);
                    c.pc = start + 1;
                } else {
                    c.env.remove(&var);
                    c.pc += 1;
                }
                Ok(true)
            }
            Instr::Op(op, stage) => {
                let stage = *stage;
                let stepped = self.exec(pi, site, engine, at, &op, stage)?;
                if stepped {
                    self.programs[pi].pc += 1;
                }
                Ok(stepped)
            }
        }
    }

    fn exec(
        &mut self,
        pi: usize,
        site: Site,
        engine: Engine,
        at: (Site, Engine, usize),
        op: &ChoreoOp,
        stage: StageId,
    ) -> Result<bool, ChoreoFault> {
        match op {
            ChoreoOp::Reserve { buf, pages } => {
                let f = &mut self.fifos[buf.idx()];
                if f.queue.len() + f.reserved + *pages > f.capacity {
                    return Ok(false);
                }
                f.reserved += *pages;
                Ok(true)
            }
            ChoreoOp::Push { buf, pages } => {
                {
                    let f = &self.fifos[buf.idx()];
                    if f.reserved < *pages {
                        return Err(ChoreoFault::BufferOverflow { buf: *buf, at });
                    }
                    if f.pending.len() < *pages {
                        return Err(ChoreoFault::PushWithoutData { buf: *buf, at });
                    }
                }
                let owner = self.storage(*buf);
                for _ in 0..*pages {
                    let slot =
                        self.fifos[buf.idx()].wr % self.fifos[owner.idx()].slots.len().max(1);
                    self.check_alias_clobber(*buf, slot, at)?;
                    let f = &mut self.fifos[buf.idx()];
                    let page = f.pending.pop_front().unwrap();
                    f.wr = f.wr.wrapping_add(1);
                    f.queue.push_back(page.clone());
                    let o = &mut self.fifos[owner.idx()];
                    if !o.slots.is_empty() {
                        o.slots[slot] = page;
                    }
                }
                self.fifos[buf.idx()].reserved -= *pages;
                self.touch(*buf);
                Ok(true)
            }
            ChoreoOp::Wait { buf, pages } => {
                let f = &mut self.fifos[buf.idx()];
                if f.queue.len() < *pages {
                    return Ok(false);
                }
                f.visible = f.visible.max(*pages);
                Ok(true)
            }
            ChoreoOp::Pop { buf, pages } => {
                let f = &mut self.fifos[buf.idx()];
                if f.queue.len() < *pages {
                    return Err(ChoreoFault::Underflow { buf: *buf, at });
                }
                if f.visible < *pages {
                    return Err(ChoreoFault::PopBeforeWait { buf: *buf, at });
                }
                for _ in 0..*pages {
                    f.queue.pop_front();
                }
                f.visible -= *pages;
                f.popped += *pages;
                Ok(true)
            }
            ChoreoOp::ReadTiles {
                tensor,
                at: addr,
                block_h,
                block_w,
                stride_w,
                stride_h,
                into,
            } => {
                let base = self.resolve_addr(site, &self.programs[pi].env, addr, stage, at)?;
                if self.dry {
                    self.check_indirect_span(
                        addr,
                        base,
                        (*block_h, *block_w, *stride_h, *stride_w),
                        *tensor,
                    )?;
                }
                let read_in = self.p.section_of_stage(stage).unwrap_or(0);
                *self.tiles_read.entry((*tensor, read_in)).or_default() +=
                    (*block_h * *block_w) as u64;
                let dst_dtype = self.p.buffer(*into).dtype;
                let store = &self.tensors[tensor.idx()];
                let mut page = Vec::with_capacity(block_h * block_w);
                for h in 0..*block_h {
                    for w in 0..*block_w {
                        let idx = base + h * stride_h + w * stride_w;
                        // ★ A CARRIED TILE IS READ IN STEP OR NOT AT ALL. The writer's section is a
                        // fact about the PROGRAM; whether the tile is written yet is a fact about this
                        // RUN. They must agree, and when a barrier is missing they do not — which is
                        // the only way a state tensor can announce that it was read out of order,
                        // because unlike an activation it always holds a plausible number.
                        //
                        // ★ AND A CARRY MAY BE UPDATED IN PART. A row appended to a cache writes that row's tiles
                        // and no others, so a later section reading an untouched tile reads this step's value —
                        // PROVIDED every write the schedule holds for the tensor has run: then "not written" means
                        // "not written this step", while before the last write it means a read racing the writer.
                        //
                        // ★ AND BY MORE THAN ONE SECTION. A table written row by row has a writer per row, so the
                        // rule is per writer: no section at or after the reader has written the tile yet, and every
                        // writer before the reader has either written it or run all of its writes.
                        if let Some(writers) = &store.writer_sections {
                            let last = store.written_in.get(idx).copied().flatten();
                            let done = |w: usize| {
                                let t = tensor.idx();
                                self.writes_done[t].get(&w).copied().unwrap_or(0)
                                    >= self.writes_expected[t].get(&w).copied().unwrap_or(0)
                            };
                            let late = last.filter(|l| *l >= read_in);
                            let early = writers
                                .iter()
                                .copied()
                                .find(|w| *w < read_in && !done(*w) && last.is_none_or(|l| l < *w));
                            if let Some(w_sec) = late.or(early) {
                                return Err(ChoreoFault::StaleStateRead {
                                    tensor: *tensor,
                                    tile: idx,
                                    read_in,
                                    written_in: w_sec,
                                    already_written: last.is_some(),
                                });
                            }
                        }
                        // An unwritten tile is a read before its writer — unless the dry run's window elided
                        // writes to this tensor, when it may be a tile the elided rounds write. Whether a later
                        // section reads after an earlier one wrote is the ordering the IR proves, not the window.
                        let tile = match store.tiles.get(idx) {
                            Some(Some(tile)) => tile.clone(),
                            Some(None) if store.elided => TileData::Empty,
                            _ => {
                                return Err(ChoreoFault::ReadOutOfRange {
                                    tensor: *tensor,
                                    tile: idx,
                                })
                            }
                        };
                        // ★ A LOAD NARROWS TO THE BUFFER IT LANDS IN. Memory holds what the producer
                        // wrote; the operand register holds what its own width can represent. A
                        // composed program stores an fp32 matmul result and reads it into a bf16
                        // datapath one section later, and modelling that copy as lossless would make
                        // the oracle disagree with the part in the one place it is supposed to be
                        // exact about. No single-op program changes: every one of them declares its
                        // buffers at the tensor's own precision.
                        page.push(if self.dry {
                            TileData::Empty
                        } else {
                            narrow(tile, dst_dtype)
                        });
                    }
                }
                let f = &mut self.fifos[into.idx()];
                if f.pending.len() >= f.reserved {
                    return Err(ChoreoFault::BufferOverflow { buf: *into, at });
                }
                f.pending.push_back(page);
                self.touch(*into);
                Ok(true)
            }
            ChoreoOp::ReadBarrier | ChoreoOp::WriteBarrier | ChoreoOp::Flush { .. } => Ok(true),
            ChoreoOp::WriteTiles {
                from,
                tensor,
                at: addr,
                block_h,
                block_w,
                stride_w,
                stride_h,
            } => {
                let base = self.resolve_addr(site, &self.programs[pi].env, addr, stage, at)?;
                if self.dry {
                    self.check_indirect_span(
                        addr,
                        base,
                        (*block_h, *block_w, *stride_h, *stride_w),
                        *tensor,
                    )?;
                }
                let page = self.front_page(*from, at)?;
                if page.len() != block_h * block_w {
                    return Err(ChoreoFault::Unsupported(format!(
                        "write page of {} tiles for a {block_h}x{block_w} block",
                        page.len()
                    )));
                }
                let section = self.p.section_of_stage(stage).unwrap_or(0);
                let store = &mut self.tensors[tensor.idx()];
                for h in 0..*block_h {
                    for w in 0..*block_w {
                        let idx = base + h * stride_h + w * stride_w;
                        if idx >= store.tiles.len() {
                            return Err(ChoreoFault::ReadOutOfRange {
                                tensor: *tensor,
                                tile: idx,
                            });
                        }
                        // ★ A SECOND STORE OF A TILE IS A RACE — within one section. A carried tensor's later
                        // section rewriting a tile an earlier one wrote is the next update, ordered by the barrier
                        // between them (the certificate's `StateOrder`); anything else storing a tile twice is two
                        // writers of one value.
                        if let Some((first, first_section)) = store.stored_by[idx] {
                            if store.writer_sections.is_none() || first_section == section {
                                return Err(ChoreoFault::StoreConflict {
                                    tensor: *tensor,
                                    tile: idx,
                                    first,
                                    second: site,
                                });
                            }
                        }
                        store.tiles[idx] = Some(page[h * block_w + w].clone());
                        store.stored_by[idx] = Some((site, section));
                        store.written_in[idx] = Some(section);
                    }
                }
                *self.writes_done[tensor.idx()].entry(section).or_default() += 1;
                Ok(true)
            }
            ChoreoOp::McastSend {
                buf, pages, group, ..
            } => {
                let pl = self.p.placement.as_ref().expect("placed");
                let g = &pl.groups[group.idx()];
                let sender_pending: Vec<Page> = {
                    let f = &self.fifos[buf.idx()];
                    if f.pending.len() < *pages {
                        return Err(ChoreoFault::PushWithoutData { buf: *buf, at });
                    }
                    f.pending.iter().rev().take(*pages).rev().cloned().collect()
                };
                let receivers: Vec<Site> = g
                    .rect
                    .sites()
                    .into_iter()
                    .filter(|s| *s != g.sender)
                    .collect();
                for r in &receivers {
                    let rb = *self.recv_buffer.get(&(*group, *r)).ok_or(
                        ChoreoFault::McastIntoUnreserved {
                            group: *group,
                            receiver: *r,
                        },
                    )?;
                    let f = &self.fifos[rb.idx()];
                    if f.pending.len() + pages > f.reserved {
                        return Err(ChoreoFault::McastIntoUnreserved {
                            group: *group,
                            receiver: *r,
                        });
                    }
                }
                for r in &receivers {
                    let rb = self.recv_buffer[&(*group, *r)];
                    let f = &mut self.fifos[rb.idx()];
                    for page in &sender_pending {
                        f.pending.push_back(page.clone());
                    }
                    self.touch(rb);
                }
                Ok(true)
            }
            ChoreoOp::McastSetValid { sem, group } => {
                let pl = self.p.placement.as_ref().expect("placed");
                let g = &pl.groups[group.idx()];
                for r in g.rect.sites().into_iter().filter(|s| *s != g.sender) {
                    self.sems
                        .insert((*sem, r), SemValue::Valid.as_u32() as usize);
                }
                Ok(true)
            }
            ChoreoOp::SemSet { sem, value } => {
                self.sems.insert((*sem, site), value.as_u32() as usize);
                Ok(true)
            }
            ChoreoOp::SemInc {
                sem,
                at: target,
                by,
            } => {
                *self.sems.entry((*sem, *target)).or_insert(0) += by;
                Ok(true)
            }
            ChoreoOp::SemWait { sem, until } => {
                let v = self.sems.get(&(*sem, site)).copied().unwrap_or(0);
                Ok(match until {
                    SemCond::Equals(n) => v == *n,
                    SemCond::AtLeast(n) => v >= *n,
                })
            }
            ChoreoOp::AcquireAcc { tiles } => {
                if *tiles > self.p.acc_tiles_used.max(1) {
                    return Err(ChoreoFault::AccOverflow {
                        requested: *tiles,
                        available: self.p.acc_tiles_used,
                    });
                }
                let c = &mut self.programs[pi];
                if c.acc.is_some() {
                    return Err(ChoreoFault::AccProtocol { at });
                }
                let n = self.t * self.t;
                // The accumulator is the one this op's SECTION was lowered for; a composed program's own plan
                // is only its last section's.
                let acc_float = self
                    .p
                    .plan_of_stage(stage)
                    .map(|pl| pl.acc == AccPolicy::F32)
                    .unwrap_or(false);
                c.acc = Some(
                    (0..*tiles)
                        .map(|_| {
                            if acc_float {
                                AccData::F32(vec![0.0; n])
                            } else {
                                AccData::Int(vec![0; n])
                            }
                        })
                        .collect(),
                );
                c.acc_computed = false;
                c.acc_folded.clear();
                Ok(true)
            }
            ChoreoOp::ReleaseAcc => {
                let c = &mut self.programs[pi];
                if c.acc.is_none() {
                    return Err(ChoreoFault::AccProtocol { at });
                }
                c.acc = None;
                Ok(true)
            }
            ChoreoOp::Compute {
                op,
                ins,
                acc_reload,
            } => {
                let stepped = self.compute(pi, site, engine, at, stage, op, ins, *acc_reload)?;
                if stepped && !self.dry {
                    self.note_acc(pi, stage);
                }
                Ok(stepped)
            }
            ChoreoOp::PackAcc {
                to,
                tiles,
                convert,
                accumulate,
            } => {
                let acc = match self.programs[pi].acc.as_ref() {
                    Some(a) => a.clone(),
                    None => return Err(ChoreoFault::AccProtocol { at }),
                };
                if !self.programs[pi].acc_computed {
                    return Err(ChoreoFault::AccProtocol { at });
                }
                if acc.len() < *tiles {
                    return Err(ChoreoFault::AccOverflow {
                        requested: *tiles,
                        available: acc.len(),
                    });
                }
                let plan_acc = self
                    .p
                    .plan_of_stage(stage)
                    .map(|pl| pl.acc)
                    .unwrap_or(AccPolicy::I64);
                let page: Page = if self.dry {
                    vec![TileData::Empty; *tiles]
                } else {
                    acc.iter()
                        .take(*tiles)
                        .map(|a| match a {
                            AccData::Int(v) => {
                                let vals: Vec<i64> = v
                                    .iter()
                                    .map(|x| {
                                        if plan_acc == AccPolicy::I32 {
                                            trunc_signed(*x, 32)
                                        } else {
                                            *x
                                        }
                                    })
                                    .collect();
                                if precision_is_int(*convert) {
                                    TileData::Int(vals)
                                } else {
                                    TileData::Float(
                                        vals.iter()
                                            .map(|x| round_to(*convert, *x as f64))
                                            .collect(),
                                    )
                                }
                            }
                            AccData::F32(v) => {
                                if precision_is_int(*convert) {
                                    TileData::Int(v.iter().map(|x| *x as i64).collect())
                                } else {
                                    TileData::Float(
                                        v.iter().map(|x| round_to(*convert, *x as f64)).collect(),
                                    )
                                }
                            }
                        })
                        .collect()
                };
                let f = &self.fifos[to.idx()];
                if f.pending.len() >= f.reserved {
                    return Err(ChoreoFault::BufferOverflow { buf: *to, at });
                }
                let page = if *accumulate && !self.dry {
                    // The page accumulated into is the one at the slot about to be filled, in whichever buffer's
                    // memory this one's pages live.
                    let store = &self.fifos[self.storage(*to).idx()].slots;
                    let slot = (f.wr + f.pending.len()) % store.len().max(1);
                    let base = store.get(slot).cloned().unwrap_or_default();
                    add_pages(&base, &page)
                } else {
                    page
                };
                self.fifos[to.idx()].pending.push_back(page);
                self.touch(*to);
                Ok(true)
            }
            ChoreoOp::Barrier { .. } => Err(ChoreoFault::Unsupported(
                "barrier groups are not lowered by any pass yet".into(),
            )),
            ChoreoOp::Loop { .. } => unreachable!("loops are flattened"),
        }
    }

    fn front_page(&self, buf: BufferId, at: (Site, Engine, usize)) -> Result<Page, ChoreoFault> {
        let f = &self.fifos[buf.idx()];
        if f.visible == 0 {
            return Err(if f.queue.is_empty() {
                ChoreoFault::Underflow { buf, at }
            } else {
                ChoreoFault::ReadBeforeWait { buf, at }
            });
        }
        f.queue
            .front()
            .cloned()
            .ok_or(ChoreoFault::Underflow { buf, at })
    }

    #[allow(clippy::too_many_arguments)]
    fn compute(
        &mut self,
        pi: usize,
        site: Site,
        _engine: Engine,
        at: (Site, Engine, usize),
        stage: StageId,
        op: &ComputeOp,
        ins: &[BufferId],
        acc_reload: Option<BufferId>,
    ) -> Result<bool, ChoreoFault> {
        let t = self.t;
        let n = t * t;
        if self.programs[pi].acc.is_none() {
            return Err(ChoreoFault::AccProtocol { at });
        }
        // ★ THE GUARD IS AGAINST FOLDING A PAGE TWICE, NOT AGAINST USING AN OPERAND TWICE. It exists
        // so two separate compute ops cannot accumulate the same page of the same buffer into the
        // register file — the double-count a K loop makes easy. An op that names ONE buffer on both
        // of its ports (`x · x`, the square a sum-of-squares starts with) reads a single page once
        // and is not that mistake, so the fold is keyed per distinct operand within an op.
        let mut folded: Vec<_> = Vec::new();
        for b in ins {
            let key = (b.idx(), self.fifos[b.idx()].popped);
            if folded.contains(&key) {
                continue;
            }
            folded.push(key);
            if !self.programs[pi].acc_folded.insert(key) {
                return Err(ChoreoFault::AccProtocol { at });
            }
        }
        self.programs[pi].acc_computed = true;
        // ★ A GATHER IS REFUSED BEFORE THE DRY RUN DECLINES TO LOOK. The token-only run skips the
        // arithmetic, which is exactly where an indirect map would otherwise slip past — and a program
        // the certificate blesses must be one the data run can evaluate. So a map carrying an
        // `IndirectTerm` is refused here, in both modes, naming the phase that gives it meaning.
        // A GATHER (an indirect term on an OPERAND map) is evaluated below; a SCATTER (one on the output
        // map) is not — nothing lowers one — and it is refused in both modes, naming the phase it waits on.
        if let ComputeOp::Generic { out, .. } = op {
            if out.indirect.is_some() {
                return Err(ChoreoFault::Unsupported(INDIRECT_MAP_DEFERRED.into()));
            }
        }
        if self.dry {
            // Token-only: the protocol (accumulator held, operand pages present, reload page present) is checked;
            // the arithmetic is not performed.
            for b in ins {
                self.front_page(*b, at)?;
            }
            if let Some(interm) = acc_reload {
                let page = self.front_page(interm, at)?;
                let acc = self.programs[pi].acc.as_ref().unwrap();
                if page.len() < acc.len() {
                    return Err(ChoreoFault::Unsupported(format!(
                        "reload page of {} tiles for {} accumulators",
                        page.len(),
                        acc.len()
                    )));
                }
            }
            return Ok(true);
        }
        if let Some(interm) = acc_reload {
            let page = self.front_page(interm, at)?;
            let acc = self.programs[pi].acc.as_mut().unwrap();
            for (a, tile) in acc.iter_mut().zip(page.iter()) {
                *a = match (tile, &*a) {
                    (TileData::Int(v), AccData::Int(_)) => AccData::Int(v.clone()),
                    (TileData::Float(v), AccData::F32(_)) => {
                        AccData::F32(v.iter().map(|x| *x as f32).collect())
                    }
                    (TileData::Int(v), AccData::F32(_)) => {
                        AccData::F32(v.iter().map(|x| *x as f32).collect())
                    }
                    (TileData::Float(v), AccData::Int(_)) => {
                        AccData::Int(v.iter().map(|x| *x as i64).collect())
                    }
                    (TileData::Empty, a) => a.clone(),
                };
            }
        }
        match op {
            ComputeOp::MatmulTiles {
                block,
                sub_blocks,
                operand,
                acc,
                ..
            } => {
                let a_page = self.front_page(ins[0], at)?;
                let b_page = self.front_page(ins[1], at)?;
                let env = &self.programs[pi].env;
                let sh = env.get(&LV_SUB_H).copied().unwrap_or(0);
                let sw = env.get(&LV_SUB_W).copied().unwrap_or(0);
                let (osh, osw, ibw) = (block.osh, block.osw, block.ibw);
                let per_site_n = sub_blocks.1 * osw;
                let bits = precision_int_bits(*operand).unwrap_or(64);
                let accs = self.programs[pi].acc.as_mut().unwrap();
                if accs.len() < osh * osw {
                    return Err(ChoreoFault::AccOverflow {
                        requested: osh * osw,
                        available: accs.len(),
                    });
                }
                for i in 0..osh {
                    for j in 0..osw {
                        for kk in 0..ibw {
                            let dst = &mut accs[i * osw + j];
                            let a_tile = a_page
                                .get((sh * osh + i) * ibw + kk)
                                .ok_or(ChoreoFault::Unsupported("A page shape".into()))?;
                            let b_tile = b_page
                                .get(kk * per_site_n + (sw * osw + j))
                                .ok_or(ChoreoFault::Unsupported("B page shape".into()))?;
                            match (dst, a_tile, b_tile) {
                                (AccData::Int(d), TileData::Int(a), TileData::Int(b)) => {
                                    for r in 0..t {
                                        for c in 0..t {
                                            let mut s = d[r * t + c];
                                            for x in 0..t {
                                                s = s.wrapping_add(
                                                    trunc_signed(a[r * t + x], bits).wrapping_mul(
                                                        trunc_signed(b[x * t + c], bits),
                                                    ),
                                                );
                                            }
                                            d[r * t + c] = s;
                                        }
                                    }
                                }
                                (AccData::F32(d), TileData::Float(a), TileData::Float(b)) => {
                                    for r in 0..t {
                                        for c in 0..t {
                                            let mut s = d[r * t + c];
                                            for x in 0..t {
                                                s += a[r * t + x] as f32 * b[x * t + c] as f32;
                                            }
                                            d[r * t + c] = s;
                                        }
                                    }
                                }
                                _ => {
                                    return Err(ChoreoFault::Unsupported(format!(
                                        "operand/accumulator mix for {acc:?}"
                                    )))
                                }
                            }
                        }
                    }
                }
                Ok(true)
            }
            ComputeOp::Requant { m, s } => {
                let page = self.front_page(ins[0], at)?;
                let accs = self.programs[pi].acc.as_mut().unwrap();
                for (k, dst) in accs.iter_mut().enumerate() {
                    let Some(TileData::Int(v)) = page.get(k) else {
                        return Err(ChoreoFault::Unsupported(
                            "requant needs integer tiles".into(),
                        ));
                    };
                    *dst = AccData::Int(v.iter().map(|x| m.wrapping_mul(*x) >> s).collect());
                }
                Ok(true)
            }
            ComputeOp::Bias { .. } => {
                let page = self.front_page(ins[0], at)?;
                let bias = self.front_page(ins[1], at)?;
                let accs = self.programs[pi].acc.as_mut().unwrap();
                for (k, dst) in accs.iter_mut().enumerate() {
                    let (Some(TileData::Int(v)), Some(TileData::Int(b))) =
                        (page.get(k), bias.get(k % bias.len().max(1)))
                    else {
                        return Err(ChoreoFault::Unsupported("bias needs integer tiles".into()));
                    };
                    let mut out = vec![0i64; n];
                    for r in 0..t {
                        for c in 0..t {
                            out[r * t + c] = v[r * t + c].wrapping_add(b[c]);
                        }
                    }
                    *dst = AccData::Int(out);
                }
                Ok(true)
            }
            ComputeOp::Relu => {
                let page = self.front_page(ins[0], at)?;
                let accs = self.programs[pi].acc.as_mut().unwrap();
                for (k, dst) in accs.iter_mut().enumerate() {
                    match page.get(k) {
                        Some(TileData::Int(v)) => {
                            *dst = AccData::Int(v.iter().map(|x| (*x).max(0)).collect())
                        }
                        Some(TileData::Float(v)) => {
                            *dst = AccData::F32(v.iter().map(|x| x.max(0.0) as f32).collect())
                        }
                        Some(TileData::Empty) | None => {
                            return Err(ChoreoFault::Unsupported("relu page shape".into()))
                        }
                    }
                }
                Ok(true)
            }
            // ── THE GENERAL FORM ───────────────────────────────────────────────────────────────
            //
            // ★ THIS ARM IS THE CONTRACT. Every legalization the compiler performs must agree with
            // what is written here, and `spatial_rewrite_lock` is what checks that it does. The
            // arithmetic is deliberately plain: walk the iteration space, read each operand through
            // its own access map, apply the body, fold the reduce axes.
            //
            // The two facts the algebra rests on are visible in `coord` alone: an operand whose map
            // omits an axis reads coordinate 0 along it — which IS a broadcast — and an axis marked
            // `Reduce` never reaches the output's map, which IS a reduction.
            ComputeOp::Generic {
                iters,
                ins: maps,
                out,
                body,
            } => {
                let (row_ax, col_ax, k_ax) = match iters.as_slice() {
                    [r, c] => (*r, *c, None),
                    [r, c, k] => (*r, *c, Some(*k)),
                    _ => return Err(ChoreoFault::Unsupported("a Generic iteration space that is not (rows, cols) or (rows, cols, contraction)".into())),
                };
                let k_var = k_ax
                    .map(|k| k.var)
                    .unwrap_or(crate::spatial::passes::LV_GEN_K);
                if maps.len() != ins.len() {
                    return Err(ChoreoFault::Unsupported(
                        "a Generic whose access maps do not match its operands".into(),
                    ));
                }
                for it in [row_ax, col_ax] {
                    if matches!(it.kind, IterKind::Reduce { .. }) && out.varies_along(it.var) {
                        return Err(ChoreoFault::Unsupported(
                            "a Generic whose output varies along a reduced axis".into(),
                        ));
                    }
                }
                if body.max_operand().is_some_and(|i| i >= ins.len()) {
                    return Err(ChoreoFault::Unsupported(
                        "a Generic body naming an absent operand".into(),
                    ));
                }
                let pages: Vec<Page> = ins
                    .iter()
                    .map(|b| self.front_page(*b, at))
                    .collect::<Result<_, _>>()?;
                // ★ THE PAGE IS A FLAT ROW-MAJOR PLANE — `t` rows by `page_tiles · t` columns — and a
                // map is a SIGNED offset into it. `(off / width, off % width)` recovers the element;
                // an offset outside the plane reads 0.0, never faults, so an unselected `Select`
                // branch may point past the page. The old per-axis read (`coord` took only the axis'
                // own term) could never let a row depend on the column iterator; this is what makes
                // a transpose, a head slice and a rope pair read all MAPS.
                let read_flat = |page: &Page, off: i64| -> f64 {
                    let width = (page.len() * t) as i64;
                    if off < 0 || width == 0 || off >= t as i64 * width {
                        return 0.0;
                    }
                    let (r, c) = ((off / width) as usize, (off % width) as usize);
                    let (tile, e) = (c / t, r * t + c % t);
                    match page.get(tile) {
                        Some(TileData::Float(v)) => v.get(e).copied().unwrap_or(0.0),
                        Some(TileData::Int(v)) => v.get(e).copied().unwrap_or(0) as f64,
                        _ => 0.0,
                    }
                };
                fn holds(
                    p: &Pred,
                    operand: &dyn Fn(usize) -> f64,
                    axis: &dyn Fn(LoopVar) -> usize,
                ) -> bool {
                    let bound = match p.rhs() {
                        PredRhs::Const(k) => k as f64,
                        PredRhs::Operand(i) => operand(i),
                    };
                    let x = axis(p.var()) as f64;
                    match p {
                        Pred::AxisLt(..) => x < bound,
                        Pred::AxisGe(..) => x >= bound,
                    }
                }
                fn apply(
                    b: &TensorBody,
                    operand: &dyn Fn(usize) -> f64,
                    axis: &dyn Fn(LoopVar) -> usize,
                    held: fn(f64) -> f64,
                ) -> f64 {
                    let at = |x: &TensorBody| apply(x, operand, axis, held);
                    match b {
                        TensorBody::Operand(i) => operand(*i),
                        TensorBody::Const(bits) => held(f64::from_bits(*bits)),
                        TensorBody::Add(x, y) => held(at(x) + at(y)),
                        TensorBody::Sub(x, y) => held(at(x) - at(y)),
                        TensorBody::Mul(x, y) => held(at(x) * at(y)),
                        TensorBody::Min(x, y) => at(x).min(at(y)),
                        TensorBody::Max(x, y) => at(x).max(at(y)),
                        TensorBody::FloorDiv(x, y) => held((at(x) / at(y)).floor()),
                        TensorBody::Shr { arg, by } => held((at(arg) / 2f64.powi(*by as i32)).floor()),
                        TensorBody::Unary { kind, arg } => {
                            let v = at(arg);
                            held(match kind {
                                SfpuKind::Exp => v.exp(),
                                SfpuKind::Recip => 1.0 / v,
                                SfpuKind::Rsqrt { eps_nano, over } => {
                                    1.0 / (v / (*over).max(1) as f64 + *eps_nano as f64 * 1e-9)
                                        .sqrt()
                                }
                                SfpuKind::Silu => v / (1.0 + (-v).exp()),
                                SfpuKind::Relu => v.max(0.0),
                                SfpuKind::Ln => v.ln(),
                                SfpuKind::Sign => {
                                    if v > 0.0 {
                                        1.0
                                    } else if v < 0.0 {
                                        -1.0
                                    } else {
                                        0.0
                                    }
                                }
                            })
                        }
                        // Branch-free: decided by POSITION, both branches meaningful at every point.
                        TensorBody::Select {
                            cond,
                            then,
                            otherwise,
                        } => {
                            if holds(cond, operand, axis) {
                                at(then)
                            } else {
                                at(otherwise)
                            }
                        }
                    }
                }
                // ★ AN FP32 DATAPATH ROUNDS EVERY OPERATION. A pointwise or folded body runs on the float datapath, whose
                // registers hold fp32: each operation's result, and each constant, is the fp32 value the part keeps. So a
                // body `fuse_generic` spliced from two stages computes exactly what the two computed apart, the fp32 page
                // between them holding what a register holds, and a value never depends on whether a pass fused — nor on
                // which form a program's size chose. A contraction's body is one product of its dot, summed exact and
                // rounded once at the accumulator, as the matrix path's is.
                let held: fn(f64) -> f64 = if k_ax.is_some() { |x| x } else { |x| x as f32 as f64 };
                // ★ A GATHER IS AN OFFSET WHOSE LAST TERM IS DATA. An operand map with an `IndirectTerm` adds
                // `v · stride`, where `v` is the INDEX operand read through ITS OWN map at the same point —
                // so a per-row index broadcasts along the row, a per-element index moves with it. The index
                // must be a whole number in `[0, modulus)`; anything else is recorded as the first fault and
                // the run stops (`GatherOutOfRange`), because a wrapped or clamped index is a silently wrong
                // row, not an error anybody would see.
                let fault: std::cell::Cell<Option<(f64, usize)>> = std::cell::Cell::new(None);
                let offset = |i: usize, r: usize, c: usize, x: usize| -> i64 {
                    let affine = maps[i].offset_at_k(row_ax.var, r, col_ax.var, c, k_var, x);
                    let Some(ind) = maps[i].indirect else {
                        return affine;
                    };
                    let v = read_flat(
                        &pages[ind.operand],
                        maps[ind.operand].offset_at_k(row_ax.var, r, col_ax.var, c, k_var, x),
                    );
                    if v.fract() != 0.0 || v < 0.0 || v >= ind.modulus as f64 {
                        if fault.get().is_none() {
                            fault.set(Some((v, ind.modulus)));
                        }
                        return affine;
                    }
                    affine + v as i64 * ind.stride
                };
                // The body at one point of the iteration space — summed over the contraction axis when the space
                // has one, innermost, so a row or column fold consumes each point's whole product.
                let value_at = |r: usize, c: usize, x: usize| -> f64 {
                    let operand = |i: usize| read_flat(&pages[i], offset(i, r, c, x));
                    let axis = |v: LoopVar| {
                        if v == row_ax.var {
                            r
                        } else if v == col_ax.var {
                            c
                        } else if k_ax.is_some() && v == k_var {
                            x
                        } else {
                            0
                        }
                    };
                    apply(body, &operand, &axis, held)
                };
                let value = |r: usize, c: usize| -> f64 {
                    match k_ax {
                        None => value_at(r, c, 0),
                        Some(k) => (0..k.extent).map(|x| value_at(r, c, x)).sum(),
                    }
                };
                // ★ A PAGE HOLDS `t` ROWS. A point's row lands at tile row `r % t`, so a parallel row axis longer than
                // the tile writes two rows into one place and keeps whichever came last. Refused, never overwritten:
                // on a scalar-tile part (t = 1) a fold over `h > 1` heads as rows is exactly that.
                if matches!(row_ax.kind, IterKind::Parallel) && row_ax.extent > t {
                    return Err(ChoreoFault::Unsupported(format!(
                        "a Generic with {} parallel rows on a {t}-row tile — a page holds {t} rows, so rows past it would overwrite earlier ones",
                        row_ax.extent
                    )));
                }
                let out_tiles = if matches!(col_ax.kind, IterKind::Reduce { .. }) {
                    1
                } else {
                    col_ax.extent.div_ceil(t)
                };
                // ★ INTEGER ARITHMETIC STAYS INTEGER. Over integer operands a body with no transcendental is
                // evaluated in `i64`, every operation checked: a product, sum or fold past ±2⁶³, or a zero
                // divisor, stops the run (`IntegerArithmetic`) where the VM would carry a big integer or refuse
                // — never a wrapped or rounded number. Through `f64` the same body is exact only while every
                // intermediate stays below 2⁵³, and a requant's `x · M` can pass that while its shifted result
                // does not. The folds and the output convention are the float arm's, below, in `i64`.
                if ins
                    .iter()
                    .all(|b| precision_int_bits(self.p.buffer(*b).dtype).is_some())
                    && body.integral()
                {
                    fn apply_int(
                        b: &TensorBody,
                        operand: &dyn Fn(usize) -> i64,
                        axis: &dyn Fn(LoopVar) -> usize,
                    ) -> Result<i64, &'static str> {
                        let two = |x: &TensorBody, y: &TensorBody| -> Result<(i64, i64), &'static str> {
                            Ok((apply_int(x, operand, axis)?, apply_int(y, operand, axis)?))
                        };
                        Ok(match b {
                            TensorBody::Operand(i) => operand(*i),
                            TensorBody::Const(bits) => f64::from_bits(*bits) as i64,
                            TensorBody::Add(x, y) => {
                                let (a, b) = two(x, y)?;
                                a.checked_add(b).ok_or("an integer sum past ±2⁶³")?
                            }
                            TensorBody::Sub(x, y) => {
                                let (a, b) = two(x, y)?;
                                a.checked_sub(b).ok_or("an integer difference past ±2⁶³")?
                            }
                            TensorBody::Mul(x, y) => {
                                let (a, b) = two(x, y)?;
                                a.checked_mul(b).ok_or("an integer product past ±2⁶³")?
                            }
                            TensorBody::Min(x, y) => {
                                let (a, b) = two(x, y)?;
                                a.min(b)
                            }
                            TensorBody::Max(x, y) => {
                                let (a, b) = two(x, y)?;
                                a.max(b)
                            }
                            TensorBody::FloorDiv(x, y) => {
                                let (a, d) = two(x, y)?;
                                if d == 0 {
                                    return Err("an integer division by zero");
                                }
                                let q = a.checked_div(d).ok_or("an integer quotient past ±2⁶³")?;
                                if a % d != 0 && ((a < 0) != (d < 0)) {
                                    q - 1
                                } else {
                                    q
                                }
                            }
                            TensorBody::Shr { arg, by } => apply_int(arg, operand, axis)? >> by,
                            TensorBody::Unary { .. } => return Err("a transcendental in an integer body"),
                            TensorBody::Select {
                                cond,
                                then,
                                otherwise,
                            } => {
                                let as_f64 = |i: usize| operand(i) as f64;
                                if holds(cond, &as_f64, axis) {
                                    apply_int(then, operand, axis)?
                                } else {
                                    apply_int(otherwise, operand, axis)?
                                }
                            }
                        })
                    }
                    let read_int = |page: &Page, off: i64| -> i64 {
                        let width = (page.len() * t) as i64;
                        if off < 0 || width == 0 || off >= t as i64 * width {
                            return 0;
                        }
                        let (r, c) = ((off / width) as usize, (off % width) as usize);
                        let (tile, e) = (c / t, r * t + c % t);
                        match page.get(tile) {
                            Some(TileData::Int(v)) => v.get(e).copied().unwrap_or(0),
                            Some(TileData::Float(v)) => v.get(e).copied().unwrap_or(0.0) as i64,
                            _ => 0,
                        }
                    };
                    let value_at = |r: usize, c: usize, x: usize| -> Result<i64, &'static str> {
                        let operand = |i: usize| read_int(&pages[i], offset(i, r, c, x));
                        let axis = |v: LoopVar| {
                            if v == row_ax.var {
                                r
                            } else if v == col_ax.var {
                                c
                            } else if k_ax.is_some() && v == k_var {
                                x
                            } else {
                                0
                            }
                        };
                        apply_int(body, &operand, &axis)
                    };
                    // Every partial result the contraction and the folds hold, for the trace.
                    let kept: std::cell::Cell<Option<(i64, i64)>> = std::cell::Cell::new(None);
                    let keep = |v: i64| kept.set(Some(widen(kept.get(), v)));
                    let value = |r: usize, c: usize| -> Result<i64, &'static str> {
                        match k_ax {
                            None => value_at(r, c, 0),
                            Some(k) => (0..k.extent).try_fold(0i64, |sum, x| {
                                let sum = sum
                                    .checked_add(value_at(r, c, x)?)
                                    .ok_or("an integer contraction past ±2⁶³")?;
                                keep(sum);
                                Ok(sum)
                            }),
                        }
                    };
                    struct IntFold {
                        pool: ReducePool,
                        acc: i64,
                        idx: usize,
                        seeded: bool,
                        held: Option<(i64, i64)>,
                    }
                    impl IntFold {
                        fn new(pool: ReducePool) -> IntFold {
                            IntFold {
                                pool,
                                acc: 0,
                                idx: 0,
                                seeded: false,
                                held: None,
                            }
                        }
                        fn step(&mut self, i: usize, v: i64) -> Result<(), &'static str> {
                            match self.pool {
                                ReducePool::Sum => {
                                    self.acc = self
                                        .acc
                                        .checked_add(v)
                                        .ok_or("an integer row sum past ±2⁶³")?
                                }
                                ReducePool::Max => {
                                    self.acc = if self.seeded { self.acc.max(v) } else { v }
                                }
                                ReducePool::ArgMax => {
                                    if !self.seeded || v > self.acc {
                                        self.acc = v;
                                        self.idx = i;
                                    }
                                }
                            }
                            self.seeded = true;
                            self.held = Some(widen(self.held, self.acc));
                            Ok(())
                        }
                        fn result(&self) -> i64 {
                            match self.pool {
                                ReducePool::Sum | ReducePool::Max => self.acc,
                                ReducePool::ArgMax => self.idx as i64,
                            }
                        }
                    }
                    let kept_fold = |f: &IntFold| {
                        if let Some((lo, hi)) = f.held {
                            keep(lo);
                            keep(hi);
                        }
                    };
                    let fail = |what: &'static str| ChoreoFault::IntegerArithmetic { at, what };
                    let mut plane = vec![vec![0i64; n]; out_tiles.max(1)];
                    match (row_ax.kind, col_ax.kind) {
                        (IterKind::Parallel, IterKind::Parallel) => {
                            for r in 0..row_ax.extent {
                                for c in 0..col_ax.extent {
                                    plane[c / t][(r % t) * t + (c % t)] = value(r, c).map_err(fail)?;
                                }
                            }
                        }
                        (IterKind::Parallel, IterKind::Reduce { pool }) => {
                            for r in 0..row_ax.extent {
                                let mut fold = IntFold::new(pool);
                                for c in 0..col_ax.extent {
                                    fold.step(c, value(r, c).map_err(fail)?).map_err(fail)?;
                                }
                                kept_fold(&fold);
                                let v = fold.result();
                                for cc in 0..t {
                                    plane[0][(r % t) * t + cc] = v;
                                }
                            }
                        }
                        (IterKind::Reduce { pool }, IterKind::Parallel) => {
                            for c in 0..col_ax.extent {
                                let mut fold = IntFold::new(pool);
                                for r in 0..row_ax.extent {
                                    fold.step(r, value(r, c).map_err(fail)?).map_err(fail)?;
                                }
                                kept_fold(&fold);
                                let v = fold.result();
                                for rr in 0..t {
                                    plane[c / t][rr * t + (c % t)] = v;
                                }
                            }
                        }
                        (IterKind::Reduce { pool: row_pool }, IterKind::Reduce { pool: col_pool }) => {
                            let mut outer = IntFold::new(row_pool);
                            for r in 0..row_ax.extent {
                                let mut inner = IntFold::new(col_pool);
                                for c in 0..col_ax.extent {
                                    inner.step(c, value(r, c).map_err(fail)?).map_err(fail)?;
                                }
                                kept_fold(&inner);
                                outer.step(r, inner.result()).map_err(fail)?;
                            }
                            kept_fold(&outer);
                            let v = outer.result();
                            for e in plane[0].iter_mut() {
                                *e = v;
                            }
                        }
                    }
                    if let Some((index, extent)) = fault.get() {
                        return Err(ChoreoFault::GatherOutOfRange { at, index, extent });
                    }
                    // Every node at every point, both arms of a select as hardware evaluates them — a node with no
                    // value (a division by zero in the arm not taken) is not one.
                    fn note_nodes(
                        b: &TensorBody,
                        operand: &dyn Fn(usize) -> i64,
                        axis: &dyn Fn(LoopVar) -> usize,
                        next: &mut usize,
                        seen: &mut [Option<(i64, i64)>],
                    ) -> Option<i64> {
                        let two = |x: &TensorBody, y: &TensorBody, next: &mut usize, seen: &mut [Option<(i64, i64)>]| {
                            let a = note_nodes(x, operand, axis, next, seen);
                            let c = note_nodes(y, operand, axis, next, seen);
                            a.zip(c)
                        };
                        let v = match b {
                            TensorBody::Operand(i) => Some(operand(*i)),
                            TensorBody::Const(bits) => Some(f64::from_bits(*bits) as i64),
                            TensorBody::Add(x, y) => two(x, y, next, seen).and_then(|(a, c)| a.checked_add(c)),
                            TensorBody::Sub(x, y) => two(x, y, next, seen).and_then(|(a, c)| a.checked_sub(c)),
                            TensorBody::Mul(x, y) => two(x, y, next, seen).and_then(|(a, c)| a.checked_mul(c)),
                            TensorBody::Min(x, y) => two(x, y, next, seen).map(|(a, c)| a.min(c)),
                            TensorBody::Max(x, y) => two(x, y, next, seen).map(|(a, c)| a.max(c)),
                            TensorBody::FloorDiv(x, y) => two(x, y, next, seen).and_then(|(a, d)| {
                                let q = a.checked_div(d)?;
                                Some(if a % d != 0 && ((a < 0) != (d < 0)) { q - 1 } else { q })
                            }),
                            TensorBody::Shr { arg, by } => note_nodes(arg, operand, axis, next, seen).map(|a| a >> by),
                            TensorBody::Unary { arg, .. } => {
                                note_nodes(arg, operand, axis, next, seen);
                                None
                            }
                            TensorBody::Select { cond, then, otherwise } => {
                                let t = note_nodes(then, operand, axis, next, seen);
                                let o = note_nodes(otherwise, operand, axis, next, seen);
                                let as_f64 = |i: usize| operand(i) as f64;
                                if holds(cond, &as_f64, axis) {
                                    t
                                } else {
                                    o
                                }
                            }
                        };
                        let i = *next;
                        *next += 1;
                        if let Some(v) = v {
                            seen[i] = Some(widen(seen[i], v));
                        }
                        v
                    }
                    let mut seen = self.nodes.get(&stage).cloned().unwrap_or_else(|| vec![None; body.post_order().len()]);
                    for r in 0..row_ax.extent {
                        for c in 0..col_ax.extent {
                            for x in 0..k_ax.map_or(1, |k| k.extent) {
                                let operand = |i: usize| read_int(&pages[i], offset(i, r, c, x));
                                let axis = |v: LoopVar| {
                                    if v == row_ax.var {
                                        r
                                    } else if v == col_ax.var {
                                        c
                                    } else if k_ax.is_some() && v == k_var {
                                        x
                                    } else {
                                        0
                                    }
                                };
                                let mut next = 0;
                                note_nodes(body, &operand, &axis, &mut next, &mut seen);
                            }
                        }
                    }
                    self.nodes.insert(stage, seen);
                    if let Some((lo, hi)) = kept.get() {
                        let h = widen(Some(widen(self.stage_held.get(&stage).copied(), lo)), hi);
                        self.stage_held.insert(stage, h);
                    }
                    let accs = self.programs[pi].acc.as_mut().unwrap();
                    for (k, dst) in accs.iter_mut().enumerate().take(plane.len()) {
                        *dst = AccData::Int(plane[k].clone());
                    }
                    return Ok(true);
                }
                // ★ A FOLD IS A PROPERTY OF THE ITERATOR. One accumulator per pool: `Sum` adds, `Max`
                // keeps the larger, `ArgMax` is the VM's `>` scan — SEEDED by the first element, replaced
                // only by a strictly greater one — and yields the index it holds. Seeding is what makes a
                // NaN first the answer, as it is on the VM; an accumulator starting at `−∞` would skip it.
                struct Fold {
                    pool: ReducePool,
                    acc: f64,
                    idx: usize,
                    seeded: bool,
                }
                impl Fold {
                    fn new(pool: ReducePool) -> Fold {
                        Fold {
                            pool,
                            acc: match pool {
                                ReducePool::Sum => 0.0,
                                ReducePool::Max | ReducePool::ArgMax => f64::NEG_INFINITY,
                            },
                            idx: 0,
                            seeded: false,
                        }
                    }
                    fn step(&mut self, i: usize, v: f64) {
                        match self.pool {
                            ReducePool::Sum => self.acc += v,
                            ReducePool::Max => self.acc = self.acc.max(v),
                            ReducePool::ArgMax => {
                                if !self.seeded || v > self.acc {
                                    self.acc = v;
                                    self.idx = i;
                                    self.seeded = true;
                                }
                            }
                        }
                    }
                    fn result(&self) -> f64 {
                        match self.pool {
                            ReducePool::Sum | ReducePool::Max => self.acc,
                            ReducePool::ArgMax => self.idx as f64,
                        }
                    }
                }
                // ★ THE SYMMETRIC OUTPUT CONVENTION. Folding the COLUMN axis leaves one scalar per row,
                // written ACROSS the whole output tile (a per-row broadcast reads it back from column
                // 0). Folding the ROW axis leaves one scalar per column, written DOWN the output tile
                // (a per-column broadcast reads it back from row 0). Folding both leaves one scalar,
                // on the whole tile.
                let mut plane = vec![vec![0f64; n]; out_tiles.max(1)];
                match (row_ax.kind, col_ax.kind) {
                    (IterKind::Parallel, IterKind::Parallel) => {
                        for r in 0..row_ax.extent {
                            for c in 0..col_ax.extent {
                                plane[c / t][(r % t) * t + (c % t)] = value(r, c);
                            }
                        }
                    }
                    (IterKind::Parallel, IterKind::Reduce { pool }) => {
                        for r in 0..row_ax.extent {
                            let mut fold = Fold::new(pool);
                            for c in 0..col_ax.extent {
                                fold.step(c, value(r, c));
                            }
                            let v = fold.result();
                            for cc in 0..t {
                                plane[0][(r % t) * t + cc] = v;
                            }
                        }
                    }
                    (IterKind::Reduce { pool }, IterKind::Parallel) => {
                        for c in 0..col_ax.extent {
                            let mut fold = Fold::new(pool);
                            for r in 0..row_ax.extent {
                                fold.step(r, value(r, c));
                            }
                            let v = fold.result();
                            for rr in 0..t {
                                plane[c / t][rr * t + (c % t)] = v;
                            }
                        }
                    }
                    (IterKind::Reduce { pool: row_pool }, IterKind::Reduce { pool: col_pool }) => {
                        // Rows outer, columns inner: the row fold consumes each row's column fold.
                        let mut outer = Fold::new(row_pool);
                        for r in 0..row_ax.extent {
                            let mut inner = Fold::new(col_pool);
                            for c in 0..col_ax.extent {
                                inner.step(c, value(r, c));
                            }
                            outer.step(r, inner.result());
                        }
                        let v = outer.result();
                        for e in plane[0].iter_mut() {
                            *e = v;
                        }
                    }
                }
                if let Some((index, extent)) = fault.get() {
                    return Err(ChoreoFault::GatherOutOfRange { at, index, extent });
                }
                // ★ INDEX ARITHMETIC RUNS ON THE INTEGER DATAPATH. An arg-max, a gather offset, a token id is
                // computed here too, and `f32` holds integers exactly only below 2²⁴ — a Qwen3.8 embedding
                // offset (token · 5120) reaches 1.27·10⁹. A stage whose operands are ALL integer buffers, or
                // whose fold is an `ArgMax` (an index, which the IR already requires to pack into an integer
                // buffer), yields the exact `f64` plane (exact to 2⁵³) as integers; any other stage is the
                // float datapath and accumulates in f32, whatever the program-wide accumulator policy says.
                let integer = ins
                    .iter()
                    .all(|b| precision_int_bits(self.p.buffer(*b).dtype).is_some())
                    || [row_ax, col_ax].iter().any(|a| {
                        matches!(
                            a.kind,
                            IterKind::Reduce {
                                pool: ReducePool::ArgMax
                            }
                        )
                    });
                let accs = self.programs[pi].acc.as_mut().unwrap();
                for (k, dst) in accs.iter_mut().enumerate().take(plane.len()) {
                    *dst = if integer {
                        AccData::Int(plane[k].iter().map(|v| *v as i64).collect())
                    } else {
                        AccData::F32(plane[k].iter().map(|v| *v as f32).collect())
                    };
                }
                Ok(true)
            }
            ComputeOp::Scalar(body) => {
                let pages: Vec<Page> = ins
                    .iter()
                    .map(|b| self.front_page(*b, at))
                    .collect::<Result<_, _>>()?;
                let df = DecisionFabric {
                    n_in: body.n_in,
                    cells: body.cells.clone(),
                    result: body.result,
                    escaped_first_return: false,
                };
                let accs = self.programs[pi].acc.as_mut().unwrap();
                let mut heap: Vec<i64> = Vec::new();
                for (k, dst) in accs.iter_mut().enumerate() {
                    let mut out = vec![0i64; n];
                    for e in 0..n {
                        let inputs: Vec<i64> = pages
                            .iter()
                            .map(|p| match p.get(k) {
                                Some(TileData::Int(v)) => v[e],
                                Some(TileData::Float(v)) => v[e] as i64,
                                Some(TileData::Empty) | None => 0,
                            })
                            .collect();
                        out[e] = eval_decision_fabric_heap(&df, &inputs, &mut heap);
                    }
                    *dst = AccData::Int(out);
                }
                let _ = site;
                Ok(true)
            }
        }
    }

    fn order(&self, sched: Scheduler, round: u64) -> Vec<usize> {
        let n = self.programs.len();
        let mut idx: Vec<usize> = (0..n).collect();
        match sched {
            Scheduler::RoundRobin => {}
            Scheduler::Reverse => idx.reverse(),
            Scheduler::Seeded(seed) => {
                let mut x = seed ^ round.wrapping_mul(0x9E3779B97F4A7C15);
                for i in (1..n).rev() {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    let j = (x % (i as u64 + 1)) as usize;
                    idx.swap(i, j);
                }
            }
            Scheduler::MovementFirst => {
                idx.sort_by_key(|i| matches!(self.programs[*i].engine, Engine::Compute))
            }
            Scheduler::ComputeFirst => {
                idx.sort_by_key(|i| !matches!(self.programs[*i].engine, Engine::Compute))
            }
        }
        idx
    }

    fn run(&mut self, sched: Scheduler) -> Result<(), ChoreoFault> {
        let mut round = 0u64;
        loop {
            let order = self.order(sched, round);
            let mut progressed = false;
            let mut all_done = true;
            for pi in order {
                if self.programs[pi].done {
                    continue;
                }
                all_done = false;
                // Let a program run ahead while it can: bounded by its blocking op.
                let mut budget = 64;
                while budget > 0 && !self.programs[pi].done {
                    if self.step(pi)? {
                        progressed = true;
                        self.steps += 1;
                    } else {
                        break;
                    }
                    budget -= 1;
                }
            }
            if all_done {
                return Ok(());
            }
            if !progressed {
                let blocked_at = self
                    .programs
                    .iter()
                    .filter(|c| !c.done)
                    .map(|c| {
                        let op = match c.code.get(c.pc) {
                            Some(Instr::Op(op, _)) => op.clone(),
                            _ => ChoreoOp::ReadBarrier,
                        };
                        (c.site, c.engine, c.pc, op)
                    })
                    .collect();
                return Err(ChoreoFault::Deadlock {
                    runnable: Vec::new(),
                    blocked_at,
                });
            }
            round += 1;
        }
    }

    fn finish(self) -> Result<(Outputs, Trace), ChoreoFault> {
        self.finish_with(false)
    }

    /// `every`: collect every tensor the program holds at the end — its activations and parameters too — not only
    /// its results (`Output` and `State`).
    fn finish_with(mut self, every: bool) -> Result<(Outputs, Trace), ChoreoFault> {
        for (i, f) in self.fifos.iter().enumerate() {
            if !f.queue.is_empty() || !f.pending.is_empty() {
                return Err(ChoreoFault::Unconsumed {
                    buf: BufferId::new(i),
                    pages_left: f.queue.len() + f.pending.len(),
                });
            }
            if f.reserved != 0 {
                return Err(ChoreoFault::OutstandingReservation {
                    buf: BufferId::new(i),
                    pages: f.reserved,
                });
            }
        }
        let t = self.t;
        let mut tensors = BTreeMap::new();
        for (td, store) in self.p.tensors.iter().zip(self.tensors.iter()) {
            // ★ THE CARRY LEAVES WITH THE ANSWER. A step's state is as much a result as its logits —
            // it is what the NEXT token reads — so it rides in the outputs and a differential can
            // compare it. Without this a wrong GDN memory would be invisible until the second token.
            if !every && !matches!(td.role, TensorRole::Output | TensorRole::State) {
                continue;
            }
            let (rows, cols) = (td.elems.rows, td.elems.cols);
            let is_int = precision_is_int(store.dtype);
            let mut ints = vec![0i64; rows * cols];
            let mut floats = vec![0f64; rows * cols];
            for (idx, tile) in store.tiles.iter().enumerate() {
                // ★ OUTPUT COVERAGE IS NOT PROVABLE HERE, AND THE OLD COMMENT CLAIMED OTHERWISE.
                //
                // What stood here said coverage "is the structural invariant `validate` proves (every
                // output tile stored exactly once by the work list)". `validate` proves no such thing: it
                // checks that a Store writes an Output tensor and a Load does not read one. Nothing
                // connected the WORK LIST (over which coverage is stated) to the SCHEDULE (which is what
                // actually runs) — so `certify_choreography`, which reaches coverage only through
                // `eval_choreography_dry`, could issue a certificate for a program that never stores part
                // of its result. `spatial_certificate_lock` found it at case 105 by deleting a scheduled op.
                //
                // ★ AND IT CANNOT BE FIXED BY CHECKING `stored_by` HERE. A dry run is deliberately bounded:
                // `Eval::new` flattens with `Some(item_window(p))` and `unroll` caps the ITEM loop at that
                // window, so a program with more items per site than the window leaves later tiles unwritten
                // AS DESIGNED. Faulting on those rejects correct GEMMs — measured, five locks across
                // handshake/buffers/work-model — and because `verify` is the last pass in `PASS_ORDER` that
                // is the compiler refusing to compile, not a test going red.
                //
                // The real guarantee is structural and lives in `validate_schedule`
                // (`IrViolation::StageNotScheduled`): a store stage must issue at least one `WriteTiles` in
                // the schedule. That runs after every pass, so a pass that drops a store fails immediately
                // with a precise violation instead of surviving to certification. The substitution below is
                // therefore correct AS A DATA FALLBACK ONLY — it says nothing about coverage, and must not
                // be read as saying anything about it.
                let Some(tile) = tile.as_ref().or(if self.dry {
                    Some(&TileData::Empty)
                } else {
                    None
                }) else {
                    return Err(ChoreoFault::OutputTileNeverStored {
                        tensor: td.id,
                        tile: idx,
                    });
                };
                let (rt, ct) = (idx / store.cols_t, idx % store.cols_t);
                for r in 0..t {
                    for c in 0..t {
                        let (er, ec) = (rt * t + r, ct * t + c);
                        if er < rows && ec < cols {
                            match tile {
                                TileData::Int(v) => {
                                    ints[er * cols + ec] = v[r * t + c];
                                    floats[er * cols + ec] = v[r * t + c] as f64;
                                }
                                TileData::Float(v) => {
                                    floats[er * cols + ec] = v[r * t + c];
                                    ints[er * cols + ec] = v[r * t + c] as i64;
                                }
                                TileData::Empty => {}
                            }
                        }
                    }
                }
            }
            tensors.insert(
                td.id,
                if is_int {
                    Value::Int(ints)
                } else {
                    Value::Float(floats)
                },
            );
        }
        let trace = Trace {
            high_water: self
                .fifos
                .iter()
                .enumerate()
                .map(|(i, f)| (BufferId::new(i), f.high_water))
                .collect(),
            steps: self.steps,
            tiles_read: std::mem::take(&mut self.tiles_read),
            held: std::mem::take(&mut self.held),
            nodes: std::mem::take(&mut self.nodes),
            stage_held: std::mem::take(&mut self.stage_held),
        };
        Ok((
            Outputs {
                tensors,
                steps: self.steps,
            },
            trace,
        ))
    }
}

pub fn eval_choreography_with(
    p: &SpatialProgram,
    inputs: &Inputs,
    sched: Scheduler,
) -> Result<Outputs, ChoreoFault> {
    eval_choreography_traced_with(p, inputs, sched).map(|(o, _)| o)
}

pub fn eval_choreography_traced_with(
    p: &SpatialProgram,
    inputs: &Inputs,
    sched: Scheduler,
) -> Result<(Outputs, Trace), ChoreoFault> {
    let mut e = Eval::new(p, inputs)?;
    e.run(sched)?;
    e.finish()
}

/// A token-only run: every choreography check (reservations, pushes, waits, semaphore protocol, multicast
/// landing, accumulator protocol, store coverage) with no arithmetic — the certificate's dry run, every trip at its
/// greatest count.
pub fn eval_choreography_dry(p: &SpatialProgram) -> Result<Trace, ChoreoFault> {
    eval_choreography_dry_at(p, &p.bound_counts())
}

/// [`eval_choreography_dry`] with each trip `t` counting `counts[t]`.
pub fn eval_choreography_dry_at(
    p: &SpatialProgram,
    counts: &[usize],
) -> Result<Trace, ChoreoFault> {
    let mut e = Eval::new_dry(p, counts)?;
    e.run(Scheduler::RoundRobin)?;
    e.finish().map(|(_, t)| t)
}

/// The count of each of `p`'s trips on `inputs`, in trip order: element `elem` of the trip's input — as the tensor
/// holds it, at its own precision — through [`Trip::count_of`]. An input `inputs` does not carry reads the tensor's
/// declared fill.
pub fn trip_counts(p: &SpatialProgram, inputs: &Inputs) -> Result<Vec<usize>, ChoreoFault> {
    p.trips
        .iter()
        .enumerate()
        .map(|(i, trip)| {
            let td = p.tensor(trip.tensor);
            let raw = match inputs.tensors.get(&td.id) {
                Some(Value::Int(v)) => v.get(trip.elem).map(|x| *x as f64),
                Some(Value::Float(v)) => v.get(trip.elem).copied(),
                None if td.init.is_float() => td.init.element_f64(trip.elem),
                None => td.init.element(trip.elem).map(|x| x as f64),
            }
            .ok_or(ChoreoFault::MissingInput(td.id))?;
            let held = match precision_int_bits(td.dtype) {
                Some(bits) => trunc_signed(raw as i64, bits) as f64,
                None => round_to(td.dtype, raw),
            };
            trip.count_of(held).ok_or(ChoreoFault::TripOutOfRange {
                trip: TripId::new(i),
                value: held,
                admitted: trip.admit,
            })
        })
        .collect()
}

/// [`eval_choreography`] with EVERY tensor the program holds at the end in the outputs — its activations and
/// parameters as well as its results: the values a segment of a composed program starts from, and must reach.
pub fn eval_choreography_every_tensor(
    p: &SpatialProgram,
    inputs: &Inputs,
) -> Result<Outputs, ChoreoFault> {
    let mut e = Eval::new(p, inputs)?;
    e.run(Scheduler::RoundRobin)?;
    e.finish_with(true).map(|(o, _)| o)
}

/// The meaning of a scheduled program under the round-robin scheduler.
pub fn eval_choreography(p: &SpatialProgram, inputs: &Inputs) -> Result<Outputs, ChoreoFault> {
    eval_choreography_with(p, inputs, Scheduler::RoundRobin)
}

pub fn eval_choreography_traced(
    p: &SpatialProgram,
    inputs: &Inputs,
) -> Result<(Outputs, Trace), ChoreoFault> {
    eval_choreography_traced_with(p, inputs, Scheduler::RoundRobin)
}

/// Run five schedulers and demand bit-equal outputs — Kahn determinacy, executed.
pub fn eval_choreography_confluent(
    p: &SpatialProgram,
    inputs: &Inputs,
) -> Result<Outputs, ChoreoFault> {
    let scheds = [
        Scheduler::RoundRobin,
        Scheduler::Reverse,
        Scheduler::Seeded(0x5EED),
        Scheduler::MovementFirst,
        Scheduler::ComputeFirst,
    ];
    let first = eval_choreography_with(p, inputs, scheds[0])?;
    for s in &scheds[1..] {
        let o = eval_choreography_with(p, inputs, *s)?;
        if o.tensors != first.tensors {
            let tensor = first
                .tensors
                .keys()
                .find(|k| o.tensors.get(k) != first.tensors.get(k))
                .copied()
                .unwrap_or(TensorId(0));
            return Err(ChoreoFault::NonDeterministic {
                scheduler_a: scheduler_name(scheds[0]),
                scheduler_b: scheduler_name(*s),
                tensor,
            });
        }
    }
    Ok(first)
}

/// Every site any program runs on (a placement view for the lowerings).
pub fn program_sites(p: &SpatialProgram) -> BTreeSet<Site> {
    p.schedule
        .as_ref()
        .map(|s| s.programs.iter().map(|q| q.site).collect())
        .unwrap_or_default()
}
