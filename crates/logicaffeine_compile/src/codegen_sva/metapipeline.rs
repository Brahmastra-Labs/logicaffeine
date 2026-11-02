//! The **metapipeline initiation-interval (II) model** — P1 of the Plasticine lane, and the question an RDU
//! engineer asks first: how often can a pipelined stage accept a new iteration, and how deep must the
//! buffers between stages be?
//!
//! Definitions follow Spatial (Koeplinger et al., PLDI 2018, §4): each primitive has a **resource II** (1 for
//! a fully pipelined unit, its occupancy for a sequential one); a loop-carried dependency has a
//! **recurrence II** equal to the path length from the read of a carried value to its write; a controller's
//! II is the maximum of all resource IIs and all recurrence IIs; an outer (metapipelined) controller's
//! throughput is set by its slowest stage; and a memory written in stage `i` and read in stage `j` needs
//! `j - i + 1` buffers (N-buffering — adjacent stages double-buffer).
//!
//! The model is built FROM the certified fabric IR ([`LoopFabric`]): the body is the stage's datapath, the
//! carried state registers are the recurrences, and [`UnitModel`] supplies per-cell latency / occupancy.
//! [`eval_loop_fabric_at_ii`] is the executable meaning of the number: a cycle-timed pipelined execution that
//! issues one iteration every `ii` cycles and reads whatever a carried register holds at issue time — at the
//! modeled II it reproduces the sequential oracle exactly, one cycle faster it reads stale state (a hazard),
//! so `metapipeline_lock` proves the II is neither optimistic nor vacuous. MODELED: cycle counts here are the
//! model's, never a Plasticine simulation.

use std::collections::HashMap;

use super::fabric_rtl::{eval_loop_body, FuncUnit, LoopCell, LoopFabric, Src};
use super::fabric_target::{Capability, FabricTarget, Unplaceable};

/// Per-cell timing: how many cycles a cell's result takes (`latency`) and how many cycles the cell's unit is
/// busy per iteration (`occupancy`, the resource II). A fully pipelined unit has occupancy 1 whatever its
/// latency; a sequential unit (an iterative divider) has occupancy == latency.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitModel {
    /// Latency / occupancy of every combinational tile (arith, select, vector, memory read).
    pub tile_latency: usize,
    /// Per functional unit: (latency, occupancy). Units absent here default to `default_func`.
    pub func: HashMap<FuncUnit, (usize, usize)>,
    pub default_func: (usize, usize),
    /// The unit's pipeline depth: a result leaves the unit no earlier than this many cycles after its
    /// inputs enter, however short its datapath (`0` for an ideal unit whose latency is the datapath's).
    pub pipeline_depth: usize,
    /// Cycles between a carried register's write (when its value leaves the pipeline) and the next
    /// iteration's read of it: the turnaround a loop-carried dependency pays. `0` for ideal forwarding.
    pub register_turnaround: usize,
    /// Where the numbers come from.
    pub cite: &'static str,
}

impl UnitModel {
    /// The ideal fully pipelined PCU: every tile and FU accepts a new operand every cycle (Plasticine ISCA'17
    /// §3.1: "multi-stage, reconfigurable SIMD pipeline" — one stage per FU, pipeline registers between).
    pub fn ideal_pipelined() -> UnitModel {
        UnitModel { tile_latency: 1, func: HashMap::new(), default_func: (1, 1), pipeline_depth: 0, register_turnaround: 0, cite: "Plasticine ISCA'17 §3.1: pipelined FU stages; latency 1 / occupancy 1 per stage, ideal register forwarding (modeled)" }
    }

    /// The ISCA'17 PCU as the public toolchain simulates it: one-cycle FU stages, a result leaves the
    /// `stages`-deep pipeline `stages` cycles after entering it however short the datapath, and a carried
    /// register written at the pipeline's end is read at its start one cycle later — so any loop-carried
    /// dependency that fits one PCU costs `stages + 1` per iteration. CALIBRATED against Tungsten on our own
    /// emitted loops (`benchmarks/silicon/plasticine_calibration.json`): on the 6-stage PCU, 1,000 iterations
    /// of `s = s + i` simulate in 7,003 cycles, 30 of `s = s*3 + i` in 213, 12 of a four-op chain in 87, and
    /// 1,000 iterations with no carried dependency in 1,009.
    pub fn plasticine_pcu(stages: usize) -> UnitModel {
        UnitModel { tile_latency: 1, func: HashMap::new(), default_func: (1, 1), pipeline_depth: stages, register_turnaround: 1, cite: "Plasticine ISCA'17 six-stage PCU; result latency = pipeline depth, carried-register turnaround 1 — calibrated on Tungsten (plasticine_calibration.json)" }
    }

    /// Our own fabric's units: the shared iterative divider family is SEQUENTIAL (occupancy = latency =
    /// `bits` cycles, one quotient bit per cycle); everything else is a one-cycle tile.
    pub fn sky_fabric(bits: usize) -> UnitModel {
        let mut func = HashMap::new();
        for u in [
            FuncUnit::DivS,
            FuncUnit::ModS,
            FuncUnit::FloorDivS,
            FuncUnit::Pow,
            FuncUnit::FDivF,
            FuncUnit::FSqrtF,
        ] {
            func.insert(u, (bits, bits));
        }
        UnitModel { tile_latency: 1, func, default_func: (1, 1), pipeline_depth: 0, register_turnaround: 0, cite: "codegen_sva shared_divider: bit-serial, `bits` cycles per divide (RTL); other units one-cycle tiles" }
    }

    pub fn latency(&self, cell: &LoopCell) -> usize {
        match cell {
            LoopCell::Func { unit, .. } => {
                self.func.get(unit).copied().unwrap_or(self.default_func).0
            }
            _ => self.tile_latency,
        }
    }

    pub fn occupancy(&self, cell: &LoopCell) -> usize {
        match cell {
            LoopCell::Func { unit, .. } => {
                self.func.get(unit).copied().unwrap_or(self.default_func).1
            }
            _ => 1,
        }
    }
}

/// One metapipeline stage's timing.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Stage {
    pub name: String,
    /// max over the stage's units of their occupancy.
    pub resource_ii: usize,
    /// max over carried registers of the read→write path length (0 when nothing is loop-carried).
    pub recurrence_ii: usize,
    /// The stage's critical path in cycles (first input to last output of one iteration).
    pub latency: usize,
    /// Cells in the stage's datapath (what a PCU allocation must hold).
    pub cells: usize,
    /// The costliest vector memory access per iteration in cycles (PMU bank conflicts; 1 = conflict-free).
    pub memory_ii: usize,
}

impl Stage {
    /// The stage's initiation interval: `max(resource II, recurrence II, memory II, 1)`.
    pub fn ii(&self) -> usize {
        self.resource_ii
            .max(self.recurrence_ii)
            .max(self.memory_ii)
            .max(1)
    }
}

/// A metapipeline: stages in issue order, with the producer→consumer buffer edges between them.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Metapipeline {
    pub stages: Vec<Stage>,
    /// `(writer_stage, reader_stage)` pairs, writer < reader.
    pub edges: Vec<(usize, usize)>,
}

impl Metapipeline {
    /// The pipeline's II: its slowest stage.
    pub fn ii(&self) -> usize {
        self.stages.iter().map(Stage::ii).max().unwrap_or(1)
    }
    /// Iterations per cycle at steady state.
    pub fn throughput(&self) -> f64 {
        1.0 / self.ii() as f64
    }
    /// Spatial's N-buffer rule: a memory written in stage `writer` and read in stage `reader` needs
    /// `reader - writer + 1` buffers (adjacent stages: 2, i.e. double-buffering).
    pub fn buffer_depth(writer: usize, reader: usize) -> usize {
        assert!(
            reader >= writer,
            "a buffer's reader stage ({reader}) cannot precede its writer ({writer})"
        );
        reader - writer + 1
    }
    /// `(writer, reader, depth)` for every edge.
    pub fn buffer_depths(&self) -> Vec<(usize, usize, usize)> {
        self.edges
            .iter()
            .map(|&(w, r)| (w, r, Self::buffer_depth(w, r)))
            .collect()
    }
    /// Cycles to run `iters` iterations end to end: fill (sum of stage latencies) + `(iters - 1) * II`.
    pub fn cycles(&self, iters: usize) -> usize {
        let fill: usize = self.stages.iter().map(|s| s.latency).sum();
        if iters == 0 {
            0
        } else {
            fill + (iters - 1) * self.ii()
        }
    }
    /// Gate the pipeline's depth against a target's `Metapipeline` capability — a typed refusal.
    pub fn fits(&self, target: &FabricTarget) -> Result<(), Unplaceable> {
        let cap = Capability::Metapipeline(self.stages.len());
        if target.provides(cap) {
            Ok(())
        } else {
            Err(Unplaceable::MissingCapability {
                needed: cap,
                target: target.name,
            })
        }
    }
}

/// Longest-path depth (in cycles, per `model`) of every SSA value of `lf.body`, and the depth at which each
/// carried register's next value is ready, counted from the reads of the carried state (`Src::In(k)`,
/// `k < n_state`). A value that does not depend on any carried state has state-depth 0.
fn depths(lf: &LoopFabric, model: &UnitModel) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let n = lf.body.len();
    let mut any_depth = vec![0usize; n]; // from any input
    let mut state_depth = vec![0usize; n]; // from a carried-state read (0 = independent of state)
    let src_any = |s: &Src, any: &Vec<usize>| -> usize {
        match s {
            Src::Val(v) => any[*v],
            _ => 0,
        }
    };
    let src_state = |s: &Src, st: &Vec<usize>| -> Option<usize> {
        match s {
            Src::In(k) if *k < lf.n_state => Some(0),
            Src::Val(v) => {
                if st[*v] > 0 {
                    Some(st[*v])
                } else {
                    None
                }
            }
            _ => None,
        }
    };
    for (i, cell) in lf.body.iter().enumerate() {
        let srcs: Vec<Src> = match cell {
            LoopCell::Arith { a, b, .. }
            | LoopCell::Func { a, b, .. }
            | LoopCell::VectorArith { a, b, .. } => vec![*a, *b],
            LoopCell::Select { ca, cb, t, e, .. } => vec![*ca, *cb, *t, *e],
            LoopCell::MemRead { addr } | LoopCell::HeapRead { addr } => vec![*addr],
            LoopCell::HeapWrite { addr, val } => vec![*addr, *val],
            LoopCell::HeapAlloc { size } => vec![*size],
        };
        let lat = model.latency(cell);
        any_depth[i] = srcs
            .iter()
            .map(|s| src_any(s, &any_depth))
            .max()
            .unwrap_or(0)
            + lat;
        let st = srcs.iter().filter_map(|s| src_state(s, &state_depth)).max();
        state_depth[i] = st.map(|d| d + lat).unwrap_or(0);
    }
    // A carried register's next value leaves the unit `max(path depth, pipeline depth)` cycles after its read
    // and is readable `register_turnaround` later; a register that holds (`s' = s`) or is overwritten by
    // something state-independent has no carried path at all.
    let next_state_depth: Vec<usize> = lf
        .next
        .iter()
        .map(|s| match s {
            Src::In(k) if *k < lf.n_state => 0,
            Src::Val(v) => {
                if state_depth[*v] > 0 {
                    state_depth[*v].max(model.pipeline_depth) + model.register_turnaround
                } else {
                    0
                }
            }
            _ => 0,
        })
        .collect();
    (any_depth, state_depth, next_state_depth)
}

/// Model one counted loop as ONE metapipeline stage.
pub fn loop_stage(lf: &LoopFabric, model: &UnitModel, name: &str) -> Stage {
    let (any_depth, _, next_depth) = depths(lf, model);
    Stage {
        name: name.to_string(),
        resource_ii: lf
            .body
            .iter()
            .map(|c| model.occupancy(c))
            .max()
            .unwrap_or(1),
        recurrence_ii: next_depth.iter().copied().max().unwrap_or(0),
        latency: any_depth
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
            .max(model.pipeline_depth)
            .max(1),
        cells: lf.body.len(),
        // A counted loop's `MemRead` streams the sequence at stride 1 across iterations: conflict-free under
        // strided banking at granularity 1 (memory II 1). Explicit vector patterns come through `pmu::memory_ii`.
        memory_ii: 1,
    }
}

/// The result of a cycle-timed pipelined run of a counted loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelinedRun {
    pub value: i64,
    /// Iterations that read a carried register before the previous iteration had written it.
    pub hazards: usize,
    pub iterations: usize,
    pub cycles: usize,
}

/// Execute a counted loop as a pipeline issuing one iteration every `ii` cycles. Iteration `t` issues at
/// cycle `t*ii`, reads each carried register's value as of that cycle (the latest write whose completion
/// cycle `<=` the issue cycle), and completes register `k`'s write at `t*ii + depth_k`. With `ii >=` the
/// recurrence II every read sees the previous iteration's write and the result equals
/// [`eval_loop_fabric`](super::fabric_rtl::eval_loop_fabric); below it, reads are stale (hazards).
pub fn eval_loop_fabric_at_ii(
    lf: &LoopFabric,
    params: &[i64],
    ii: usize,
    model: &UnitModel,
) -> PipelinedRun {
    assert!(ii >= 1, "an initiation interval is at least one cycle");
    assert!(lf.seq_input.is_none() && lf.seq_heap_ptr.is_none() && lf.epilogue.is_empty(),
            "eval_loop_fabric_at_ii models counted range loops (sequence / heap loops and epilogues are the sequential oracle's)");
    let empty: [i64; 0] = [];
    let (_, _, next_depth) = depths(lf, model);
    let init: Vec<i64> = lf.init.iter().map(|s| s.eval(params, &empty)).collect();
    let invs: Vec<i64> = lf
        .invariants
        .iter()
        .map(|s| s.eval(params, &empty))
        .collect();
    let lo = lf.lo.eval(params, &empty);
    let hi = lf.hi.eval(params, &empty);
    // Per register: the history of (completion_cycle, value) writes, initial value at cycle 0.
    let mut writes: Vec<Vec<(usize, i64)>> = init.iter().map(|&v| vec![(0usize, v)]).collect();
    let mut hazards = 0usize;
    let mut iterations = 0usize;
    let mut last_cycle = 0usize;
    let mut iv = lo;
    let mut t = 0usize;
    while iv <= hi {
        let issue = t * ii;
        let mut state = Vec::with_capacity(lf.n_state);
        for k in 0..lf.n_state {
            let ready: Vec<&(usize, i64)> = writes[k].iter().filter(|(c, _)| *c <= issue).collect();
            let (_, v) = ready.last().copied().copied().unwrap_or((0, init[k]));
            if t > 0 && writes[k].len() > t {
                let (prev_done, _) = writes[k][t];
                if prev_done > issue {
                    hazards += 1;
                }
            }
            state.push(v);
        }
        let mut bin: Vec<i64> = Vec::with_capacity(lf.n_state + 1 + invs.len());
        bin.extend_from_slice(&state);
        bin.push(iv);
        bin.extend_from_slice(&invs);
        let vals = eval_loop_body(lf, &bin, &[]);
        for k in 0..lf.n_state {
            let v = lf.next[k].eval(&bin, &vals);
            let done = issue + next_depth[k];
            writes[k].push((done, v));
            last_cycle = last_cycle.max(done);
        }
        iterations += 1;
        if iv == hi {
            break;
        }
        iv += 1;
        t += 1;
    }
    let value = writes[lf.result_state]
        .last()
        .map(|(_, v)| *v)
        .unwrap_or(init[lf.result_state]);
    PipelinedRun {
        value,
        hazards,
        iterations,
        cycles: last_cycle,
    }
}
