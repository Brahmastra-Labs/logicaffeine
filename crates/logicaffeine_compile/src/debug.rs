//! The bytecode debugger bridge — drives the LOGOS VM one op at a time for the
//! Studio debug drawer, with breakpoints, step-over / step-out, and **time-travel**
//! (step backwards through executed ops). Single-task, bytecode tier (`tier: None`)
//! so granularity is exactly one op and behaviour is identical to a normal run.
//!
//! ZERO production cost: nothing here is on the execution path. The stepping rides
//! the VM's `STEPPED = true` monomorphization (the default `run_until_block` is
//! byte-for-byte the old hot loop), and this whole module dead-strips from any
//! binary that never constructs a [`Debugger`].
//!
//! Time-travel falls out of the architecture for free: the debugger owns the
//! [`CompiledProgram`] and rebuilds a fresh VM each step from a saved state
//! snapshot, so it keeps the full snapshot *history* — stepping back is just
//! dropping the last snapshot.

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use crate::vm::{disassemble, CompiledProgram, DebugVmState, DisasmLine, Vm, VmStep};

/// Browser-safety cap on a single Continue / Step-Over / Step-Out (an infinite loop
/// must not hang the tab — the run pauses and reports it hit the limit).
const STEP_LIMIT: usize = 5_000_000;

/// One register in a [`DebugFrame`]. `name` is the source variable when known
/// (filled by the compiler's debug-info pass; `None` ⇒ show `R{index}`).
#[derive(Clone, Serialize)]
pub struct DebugReg {
    pub index: u16,
    pub name: Option<String>,
    /// The value's type — `Int`, `Float`, `List`, `Text`, … (teaches the type system).
    pub kind: String,
    pub value: String,
    /// The value changed in the last executed op (drives the highlight / pulse).
    pub changed: bool,
}

/// One call frame's registers. `function` is `None` for Main; `base` is the frame's
/// start address in the linear register file (its stack base), for the Stack view.
#[derive(Clone, Serialize)]
pub struct DebugFrame {
    pub function: Option<String>,
    pub base: usize,
    pub registers: Vec<DebugReg>,
}

/// One heap object for the Heap view: a list / map / set / struct / text reachable
/// from a variable or global. `referenced_by` lists every root that points at it, so
/// `shared` (more than one) makes aliasing visible — the thing assembly debuggers
/// can't show because they only have raw bytes, not typed objects.
#[derive(Clone, Serialize, Debug)]
pub struct HeapObject {
    /// A short stable label within this snapshot, e.g. `#1`.
    pub id: String,
    pub kind: String,
    pub summary: String,
    /// The underlying storage layout (`packed Vec<i64>`, `columnar`, …) — teaches how
    /// the data is really laid out, not just its printed form.
    pub storage: String,
    /// Reference count (how many handles point at this allocation).
    pub rc: usize,
    pub referenced_by: Vec<String>,
    /// Referenced by more than one root — an alias.
    pub shared: bool,
}

/// A complete picture of the paused VM for the debug drawer (serde for the UI).
#[derive(Clone, Serialize)]
pub struct DebugSnapshot {
    /// The pc the program is stopped at (the op about to execute).
    pub pc: usize,
    /// The disassembled current op (empty once finished).
    pub op_text: String,
    /// A plain-English description of the op about to run — the teaching narration
    /// (e.g. "add x(6) + y(7) → R4"). Empty for the long-tail ops.
    pub narration: String,
    /// The first-order-logic semantics of the op about to run — the formal meaning of
    /// this step (e.g. `sum = x + y`, `t ⟺ (i < n)`, `¬cond → goto 12`). Empty for the
    /// long-tail ops.
    pub fol: String,
    /// A **Socratic** prompt for the step about to run — a guiding question that invites
    /// the learner to predict the outcome before stepping ("x (6) and y (7) — what is
    /// their sum?"). Empty for ops with nothing to anticipate (the UI then shows the
    /// plain narration). Matches the engine's voice: concrete values, second person, `—`.
    pub socratic: String,
    /// Registers the current op reads / writes — drives the datapath animation.
    pub op_reads: Vec<u16>,
    pub op_writes: Option<u16>,
    /// `"paused" | "done" | "blocked" | "error"`.
    pub state: String,
    pub error: Option<String>,
    /// Position in the execution history (0 = before any op) — the time-travel cursor.
    pub step: usize,
    /// Highest step explored so far — the scrubber's maximum (>= `step`).
    pub total_steps: usize,
    /// Number of instructions in the program (the bytecode tape length).
    pub total_ops: usize,
    /// Call frames, Main first and the current frame last.
    pub frames: Vec<DebugFrame>,
    /// Heap objects reachable from the current frame + globals (the Heap view).
    pub heap: Vec<HeapObject>,
    /// Promoted globals (name → display value).
    pub globals: Vec<(String, String)>,
    /// Output emitted so far (the `Show` lines).
    pub output: Vec<String>,
    /// Whether execution is currently stopped on a breakpoint.
    pub at_breakpoint: bool,
}

/// One variable's value over the whole recorded execution — a trace on the
/// [`VarTimeline`] oscilloscope. `points[i]` is the value at explored step
/// `timeline.start + i`.
#[derive(Clone, Serialize)]
pub struct VarTrace {
    /// Main-frame register slot this variable lives in.
    pub reg: u16,
    pub name: String,
    /// The value's type, sampled the first step it holds one (`Int`, `List`, …).
    pub kind: String,
    pub points: Vec<TimelinePoint>,
}

/// One sample on a [`VarTrace`]: the variable's display value at a single step, and
/// whether it just changed (the waveform "edge").
#[derive(Clone, Serialize)]
pub struct TimelinePoint {
    pub value: String,
    /// The slot held a value at this step (always true for Main locals once the
    /// frame exists; `false` before it is allocated).
    pub present: bool,
    /// Differs from the previous step's value — drives the waveform transition.
    pub changed: bool,
}

/// A logic-analyzer view of the run: every Main-frame variable's value across the
/// recorded history, with a playhead at the time-travel cursor. Deterministic replay
/// makes this exact — it is the program's *entire* observable past, not a sample.
#[derive(Clone, Serialize)]
pub struct VarTimeline {
    /// Step index of column 0 (non-zero only when the history was tail-windowed).
    pub start: usize,
    /// Number of columns (explored steps) in the window.
    pub steps: usize,
    /// The time-travel cursor's absolute step (place the playhead at `cursor - start`).
    pub cursor: usize,
    /// History longer than the window cap was tail-trimmed to the most recent steps.
    pub truncated: bool,
    pub vars: Vec<VarTrace>,
}

/// One variable's **observed invariants** — facts that held over the recorded run
/// (constant, monotonic, range, distinct count). Dynamic likely-invariants, labelled
/// as observed (not proven), the empirical companion to the Oracle's static facts.
#[derive(Clone, Serialize)]
pub struct VarInsight {
    pub name: String,
    pub kind: String,
    pub facts: Vec<String>,
}

/// A variable's program-wide **proven** facts (from the Oracle's static abstract
/// interpretation): a finite integer range, non-negativity, a concrete scalar type.
/// Every field is a sound guarantee that holds on *every* run — the static companion
/// to [`VarInsight`]'s observed-this-run facts.
#[derive(Clone, Serialize, Default)]
pub struct ProvenFacts {
    pub scalar: Option<String>,
    pub int_range: Option<(i64, i64)>,
    pub nonneg: bool,
}

impl From<crate::optimize::VarProvenFacts> for ProvenFacts {
    fn from(v: crate::optimize::VarProvenFacts) -> Self {
        ProvenFacts {
            scalar: v.scalar.map(|s| format!("{s:?}")),
            int_range: v.int_range,
            nonneg: v.nonneg,
        }
    }
}

impl ProvenFacts {
    /// Human-readable proven facts, e.g. `["∈ [0, 9]", "type Int"]`. A finite range
    /// subsumes non-negativity, so `≥ 0` is shown only when the upper bound is open.
    fn labels(&self) -> Vec<String> {
        let mut out = Vec::new();
        match self.int_range {
            Some((lo, hi)) => out.push(format!("\u{2208} [{lo}, {hi}]")),
            None if self.nonneg => out.push("\u{2265} 0".to_string()),
            None => {}
        }
        if let Some(s) = &self.scalar {
            out.push(format!("type {s}"));
        }
        out
    }
}

/// One variable's proven invariants, pre-rendered for the UI (mirrors [`VarInsight`]).
#[derive(Clone, Serialize)]
pub struct ProvenInsight {
    pub name: String,
    pub facts: Vec<String>,
}

/// The verdict of a **live proof** at a breakpoint — whether a predicate is statically
/// guaranteed across every run (`ProvenTrue`), statically refuted (`ProvenFalse`), or
/// undecided from the proven facts (`Unknown`, where only the concrete check speaks).
#[derive(Clone, Serialize, PartialEq, Debug)]
pub enum ProofVerdict {
    ProvenTrue,
    ProvenFalse,
    Unknown,
}

/// The result of asserting a predicate at the cursor — the dual lens the debugger is
/// built for: what is true **now** (concrete, from live values) and what is **proven**
/// for every run (from the Oracle's static facts, kernel-style interval entailment, no
/// Z3). `now` is `None` when a term has no live integer value; the verdict is `Unknown`
/// when a term has no proven range.
#[derive(Clone, Serialize)]
pub struct AssertionResult {
    pub query: String,
    /// Whether the predicate parsed as a comparison (`a < b`, `x >= 0`, …).
    pub parsed: bool,
    /// Concrete truth at the current state (`None` if a term isn't a live integer).
    pub now: Option<bool>,
    pub now_detail: String,
    /// Static guarantee over every run.
    pub verdict: ProofVerdict,
    pub verdict_detail: String,
}

/// One side of a comparison: a variable reference or an integer literal.
enum Operand {
    Var(String),
    Int(i64),
}

#[derive(Clone, Copy)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// A node in a **causal provenance tree** — the exact op that produced a value, and
/// (recursively) the ops that produced its inputs. Because execution is deterministic
/// and fully recorded, this lineage is exact, not heuristic: the answer to "why is
/// this value here?" with no guessing.
#[derive(Clone, Serialize)]
pub struct CausalNode {
    /// Explored step whose op produced this value (`0` ⇒ an initial/never-written slot).
    pub step: usize,
    /// The producing op's pc, its disassembly, and its English narration.
    pub pc: usize,
    pub op_text: String,
    pub narration: String,
    /// The slot, its source name if any, and the value it ended up holding.
    pub reg: u16,
    pub name: Option<String>,
    pub kind: String,
    pub value: String,
    /// The values this op consumed — each itself traced to where it came from.
    pub inputs: Vec<CausalNode>,
}

/// Tail-window cap on the oscilloscope (a runaway loop must not force a million VM
/// rebuilds nor an unviewable waveform — the most recent steps are what you watch).
const TIMELINE_MAX_STEPS: usize = 512;

/// Depth / breadth caps on a provenance walk so a deep dependency chain stays a
/// readable tree rather than an explosion.
const PROVENANCE_MAX_DEPTH: usize = 24;
const PROVENANCE_MAX_NODES: usize = 96;

/// Per-frame terminal status (the program is deterministic, so each explored op has
/// exactly one outcome that travels with its history entry — seeking back to it
/// correctly reads "paused", not the program's final "done").
#[derive(Clone)]
enum Outcome {
    Running,
    Done,
    Blocked,
    Error(String),
}

/// One explored point in the execution: the VM state plus its outcome.
struct Frame {
    state: DebugVmState,
    outcome: Outcome,
}

/// A self-contained stepping debugger over one compiled program.
///
/// Execution is deterministic, so `history` is the single execution prefix explored
/// so far and `cursor` is simply where you are looking. Step / step-back / seek /
/// restart all just move the cursor; a new VM op is only ever computed when the
/// cursor reaches the unexplored frontier. That makes step-back, restart, redo, and
/// the time-travel scrubber instant, and reverse-continue a pure cursor walk.
pub struct Debugger {
    program: CompiledProgram,
    disasm: Vec<DisasmLine>,
    history: Vec<Frame>,
    cursor: usize,
    breakpoints: BTreeSet<usize>,
    /// Program-wide PROVEN facts per variable name, from the Oracle's abstract
    /// interpretation (computed once at compile, then read-only). The static
    /// counterpart to the dynamic [`Debugger::observed_invariants`].
    proven: HashMap<String, ProvenFacts>,
}

impl Debugger {
    /// Compile `src` (exactly as the Studio "Run" path does) and arm a debugger at
    /// the program's entry. The program is debugged on the bytecode tier with no
    /// JIT, so stepping is per-op and output matches a normal run.
    pub fn from_source(src: &str) -> Result<Debugger, String> {
        let (program, proven) = compile_source(src)?;
        let disasm = disassemble(&program);
        let initial = Vm::new(&program).save_debug_state();
        Ok(Debugger {
            program,
            disasm,
            history: vec![Frame { state: initial, outcome: Outcome::Running }],
            cursor: 0,
            breakpoints: BTreeSet::new(),
            proven,
        })
    }

    /// Execute exactly one op (Step Into).
    pub fn step(&mut self) {
        self.run_one();
    }

    /// Execute one op, but run any function it calls to completion (Step Over).
    pub fn step_over(&mut self) {
        let start = self.current_depth();
        if !self.run_one() {
            return;
        }
        let mut budget = STEP_LIMIT;
        while self.is_paused() && self.current_depth() > start {
            if self.at_breakpoint() {
                break;
            }
            if !self.run_one() {
                break;
            }
            budget -= 1;
            if budget == 0 {
                break;
            }
        }
    }

    /// Run until the current function returns (Step Out); from Main, runs to the end.
    pub fn step_out(&mut self) {
        let start = self.current_depth();
        let mut budget = STEP_LIMIT;
        loop {
            if !self.is_paused() || self.current_depth() < start {
                break;
            }
            if !self.run_one() {
                break;
            }
            if self.at_breakpoint() {
                break;
            }
            budget -= 1;
            if budget == 0 {
                break;
            }
        }
    }

    /// Run until the next breakpoint, a block, completion, or the step limit (Continue).
    pub fn resume(&mut self) {
        let mut budget = STEP_LIMIT;
        loop {
            if !self.is_paused() {
                break;
            }
            if !self.run_one() {
                break;
            }
            if self.at_breakpoint() {
                break;
            }
            budget -= 1;
            if budget == 0 {
                break;
            }
        }
    }

    /// Run BACKWARD to the previous breakpoint, or the program entry — reverse
    /// continue. Pure cursor motion over the recorded history, a time-travel feature
    /// almost no debugger has.
    pub fn reverse_resume(&mut self) {
        while self.cursor > 0 {
            self.cursor -= 1;
            if self.breakpoints.contains(&self.current().pc()) {
                break;
            }
        }
    }

    /// Undo the last executed op — time-travel one step backwards.
    pub fn step_back(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Jump the time-travel cursor to any already-explored step (the scrubber).
    pub fn seek(&mut self, step: usize) {
        self.cursor = step.min(self.history.len().saturating_sub(1));
    }

    /// Rewind to the program entry, keeping the explored history (re-stepping is then
    /// instant) and the breakpoints.
    pub fn restart(&mut self) {
        self.cursor = 0;
    }

    /// Toggle a breakpoint on a bytecode pc.
    pub fn toggle_breakpoint(&mut self, pc: usize) {
        if !self.breakpoints.remove(&pc) {
            self.breakpoints.insert(pc);
        }
    }

    pub fn set_breakpoint(&mut self, pc: usize) {
        self.breakpoints.insert(pc);
    }

    pub fn clear_breakpoint(&mut self, pc: usize) {
        self.breakpoints.remove(&pc);
    }

    pub fn breakpoints(&self) -> Vec<usize> {
        self.breakpoints.iter().copied().collect()
    }

    /// The full disassembly (the bytecode tape).
    pub fn disassembly(&self) -> &[DisasmLine] {
        &self.disasm
    }

    /// Whether the program is still paused mid-execution at the current cursor (cheap,
    /// no snapshot build). `false` once it has finished, blocked, or errored here.
    pub fn is_running(&self) -> bool {
        self.is_paused()
    }

    /// Build a serde snapshot of the current paused state for the UI.
    pub fn snapshot(&self) -> DebugSnapshot {
        let (view, heap_raw) = self.view_and_heap(self.current());
        // The previous step's innermost-frame registers, to mark what just changed —
        // but ONLY when the frame context is the same (same call depth). Crossing into
        // or out of a function changes which frame is innermost, so comparing by index
        // would flag unrelated registers; suppress it across that boundary.
        let prev_inner: HashMap<u16, String> = if self.cursor >= 1 {
            let pv = self.view_of(&self.history[self.cursor - 1].state);
            if pv.frames.len() == view.frames.len() {
                pv.frames
                    .last()
                    .map(|f| f.registers.iter().map(|(idx, _kind, val)| (*idx, val.clone())).collect())
                    .unwrap_or_default()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };
        // Main-frame variable names (`R0` → `x`), captured by the debug compile path.
        let main_names: HashMap<u16, String> =
            self.program.reg_names.iter().cloned().collect();
        let n = view.frames.len();
        let frames: Vec<DebugFrame> = view
            .frames
            .iter()
            .enumerate()
            .map(|(fi, f)| {
                let inner = fi + 1 == n;
                DebugFrame {
                    function: f.func.map(|i| format!("fn#{i}")),
                    base: f.base,
                    registers: f
                        .registers
                        .iter()
                        .map(|(idx, kind, val)| DebugReg {
                            index: *idx,
                            name: if f.func.is_none() {
                                main_names.get(idx).cloned()
                            } else {
                                None
                            },
                            kind: kind.clone(),
                            value: val.clone(),
                            changed: inner
                                && prev_inner.get(idx).map(|p| p != val).unwrap_or(false),
                        })
                        .collect(),
                }
            })
            .collect();
        let (state, error) = match &self.cur().outcome {
            Outcome::Running => ("paused", None),
            Outcome::Done => ("done", None),
            Outcome::Blocked => ("blocked", None),
            Outcome::Error(e) => ("error", Some(e.clone())),
        };
        let op_text = self.disasm.get(view.pc).map(|d| d.text.clone()).unwrap_or_default();
        // Teaching narration + the registers this op touches (for the datapath).
        let inner_regs: HashMap<u16, (Option<String>, String)> = frames
            .last()
            .map(|f: &DebugFrame| {
                f.registers.iter().map(|r| (r.index, (r.name.clone(), r.value.clone()))).collect()
            })
            .unwrap_or_default();
        let cur_op = self.program.code.get(view.pc).copied();
        let (narration, op_reads, op_writes) = match (&self.cur().outcome, cur_op) {
            (Outcome::Error(e), _) => (format!("error: {e}"), Vec::new(), None),
            (Outcome::Done, _) => ("the program has finished".to_string(), Vec::new(), None),
            (Outcome::Blocked, _) => ("waiting on a concurrency operation".to_string(), Vec::new(), None),
            (Outcome::Running, Some(op)) => {
                let io = crate::vm::op_io(&op);
                (narrate(&op, &inner_regs, &self.program), io.reads, io.writes)
            }
            _ => (String::new(), Vec::new(), None),
        };
        let fol = match (&self.cur().outcome, cur_op) {
            (Outcome::Running, Some(op)) => fol_of_op(&op, &inner_regs, &self.program),
            _ => String::new(),
        };
        let socratic = match (&self.cur().outcome, cur_op) {
            (Outcome::Running, Some(op)) => socratic_of_op(&op, &inner_regs),
            (Outcome::Done, _) => "The program has finished — did the result match what you expected?".to_string(),
            _ => String::new(),
        };
        let heap: Vec<HeapObject> = heap_raw
            .iter()
            .enumerate()
            .map(|(i, o)| HeapObject {
                id: format!("#{}", i + 1),
                kind: o.kind.clone(),
                summary: o.summary.clone(),
                storage: o.storage.clone(),
                rc: o.rc,
                referenced_by: o.referenced_by.clone(),
                shared: o.referenced_by.len() > 1,
            })
            .collect();
        DebugSnapshot {
            pc: view.pc,
            op_text,
            narration,
            fol,
            socratic,
            op_reads,
            op_writes,
            state: state.to_string(),
            error,
            step: self.cursor,
            total_steps: self.history.len().saturating_sub(1),
            total_ops: self.disasm.len(),
            frames,
            heap,
            globals: view.globals.clone(),
            output: view.output.clone(),
            at_breakpoint: self.at_breakpoint(),
        }
    }

    /// The **variable oscilloscope**: every Main-frame variable's value across the
    /// recorded execution, with a playhead at the cursor. On-demand (only the Timeline
    /// tab calls it), tail-windowed to the most recent [`TIMELINE_MAX_STEPS`] steps so
    /// a long loop stays cheap and viewable.
    pub fn variable_timeline(&self) -> VarTimeline {
        let names = self.main_names();
        let total = self.history.len();
        let start = total.saturating_sub(TIMELINE_MAX_STEPS);
        let truncated = start > 0;
        let window = &self.history[start..];
        let n = window.len();

        let mut order: Vec<u16> = names.keys().copied().collect();
        order.sort_unstable();
        // (kind, value) per (reg, column); None before the slot exists at that step.
        let mut series: HashMap<u16, Vec<Option<(String, String)>>> =
            order.iter().map(|r| (*r, vec![None; n])).collect();
        for (col, frame) in window.iter().enumerate() {
            let view = self.view_of(&frame.state);
            if let Some(main) = view.frames.iter().find(|f| f.func.is_none()) {
                for (idx, kind, val) in &main.registers {
                    if let Some(slot) = series.get_mut(idx) {
                        slot[col] = Some((kind.clone(), val.clone()));
                    }
                }
            }
        }

        let vars = order
            .iter()
            .map(|reg| {
                let name = names.get(reg).cloned().unwrap_or_else(|| format!("R{reg}"));
                let raw = &series[reg];
                let mut kind = String::new();
                let mut points = Vec::with_capacity(n);
                let mut prev: Option<String> = None;
                for cell in raw {
                    match cell {
                        Some((k, v)) => {
                            // The settled type: a slot reads `Nothing` before assignment,
                            // so the last meaningful kind is the variable's real type.
                            if k != "Nothing" || kind.is_empty() {
                                kind = k.clone();
                            }
                            // An "edge" needs a prior present value to differ from — the
                            // first time a variable appears is not a transition.
                            let changed = matches!(&prev, Some(p) if p != v);
                            points.push(TimelinePoint { value: v.clone(), present: true, changed });
                            prev = Some(v.clone());
                        }
                        None => {
                            points.push(TimelinePoint {
                                value: String::new(),
                                present: false,
                                changed: false,
                            });
                            prev = None;
                        }
                    }
                }
                VarTrace { reg: *reg, name, kind, points }
            })
            // A variable never observed in the window is noise; drop it.
            .filter(|t| t.points.iter().any(|p| p.present))
            .collect();

        VarTimeline { start, steps: n, cursor: self.cursor, truncated, vars }
    }

    /// **Observed invariants** (Daikon-style dynamic detection): for each variable,
    /// reduce its recorded trace into the facts that held over *this run* — constant,
    /// monotonic, value range, distinct count. Dynamic, not a static proof (the
    /// formally-proven counterpart comes from the Oracle), but exact for what happened.
    pub fn observed_invariants(&self) -> Vec<VarInsight> {
        let tl = self.variable_timeline();
        tl.vars
            .iter()
            .filter_map(|v| {
                // Sample only from the variable's first real assignment (its first edge)
                // — before that the slot is uninitialised and would pollute the facts.
                let first = v.points.iter().position(|p| p.present && p.changed)?;
                let vals: Vec<&str> =
                    v.points[first..].iter().filter(|p| p.present).map(|p| p.value.as_str()).collect();
                if vals.is_empty() {
                    return None;
                }
                let distinct: BTreeSet<&str> = vals.iter().copied().collect();
                let nums: Option<Vec<f64>> = vals.iter().map(|s| s.parse::<f64>().ok()).collect();
                let mut facts = Vec::new();
                if distinct.len() == 1 {
                    facts.push(format!("constant {}", vals[0]));
                } else {
                    if let Some(ns) = &nums {
                        let min = ns.iter().cloned().fold(f64::INFINITY, f64::min);
                        let max = ns.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                        facts.push(format!("range [{}, {}]", fmt_num(min), fmt_num(max)));
                        if ns.windows(2).all(|w| w[1] >= w[0]) {
                            facts.push("only increases".to_string());
                        } else if ns.windows(2).all(|w| w[1] <= w[0]) {
                            facts.push("only decreases".to_string());
                        }
                    }
                    facts.push(format!("{} distinct values", distinct.len()));
                }
                Some(VarInsight { name: v.name.clone(), kind: v.kind.clone(), facts })
            })
            .collect()
    }

    /// **Proven invariants**: the Oracle's statically-verified facts per variable
    /// (range, non-negativity, scalar type) — guarantees that hold on *every* run, not
    /// just this one. The formal companion to [`Debugger::observed_invariants`]. Only
    /// variables with at least one non-trivial proven fact are returned, sorted by name.
    pub fn proven_invariants(&self) -> Vec<ProvenInsight> {
        let mut out: Vec<ProvenInsight> = self
            .proven
            .iter()
            .filter_map(|(name, pf)| {
                let facts = pf.labels();
                if facts.is_empty() {
                    None
                } else {
                    Some(ProvenInsight { name: name.clone(), facts })
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// **Live proof at a breakpoint**: assert a comparison predicate (`x < y`, `x >= 0`,
    /// `sum == 13`) and get both lenses — whether it holds **now** (concretely, from the
    /// live values) and whether it is **proven for every run** (from the Oracle's proven
    /// ranges, by sound interval entailment; pure Rust, no Z3). The dual answer is the
    /// point: a thing can be true now yet unproven in general, or proven yet about a value
    /// not yet reached.
    pub fn assert_at_cursor(&self, predicate: &str) -> AssertionResult {
        let query = predicate.trim().to_string();
        let Some((lhs, cmp, rhs)) = parse_comparison(&query) else {
            return AssertionResult {
                query,
                parsed: false,
                now: None,
                now_detail: "couldn't parse \u{2014} try a comparison like `x < y` or `x >= 0`".to_string(),
                verdict: ProofVerdict::Unknown,
                verdict_detail: String::new(),
            };
        };

        // Concrete: resolve each side to its live integer value at the cursor.
        let frame = self.snapshot();
        let live = |o: &Operand| -> Option<i64> {
            match o {
                Operand::Int(c) => Some(*c),
                Operand::Var(name) => frame
                    .frames
                    .last()
                    .and_then(|f| f.registers.iter().find(|r| r.name.as_deref() == Some(name.as_str())))
                    .and_then(|r| r.value.parse::<i64>().ok()),
            }
        };
        let now = match (live(&lhs), live(&rhs)) {
            (Some(a), Some(b)) => Some(apply_cmp(a, cmp, b)),
            _ => None,
        };
        let now_detail = {
            let mut parts = Vec::new();
            for o in [&lhs, &rhs] {
                if let Operand::Var(name) = o {
                    match live(o) {
                        Some(v) => parts.push(format!("{name} = {v}")),
                        None => parts.push(format!("{name} = ?")),
                    }
                }
            }
            parts.join(", ")
        };

        // Static: resolve each side to its proven range and check entailment.
        let prange = |o: &Operand| -> Option<(i64, i64)> {
            match o {
                Operand::Int(c) => Some((*c, *c)),
                Operand::Var(name) => self.proven.get(name).and_then(|pf| pf.int_range),
            }
        };
        let (verdict, verdict_detail) = match (prange(&lhs), prange(&rhs)) {
            (Some(a), Some(b)) => {
                let v = entail(a, cmp, b);
                let mut srcs = Vec::new();
                if let Operand::Var(n) = &lhs {
                    srcs.push(format!("{n} \u{2208} [{}, {}]", a.0, a.1));
                }
                if let Operand::Var(n) = &rhs {
                    srcs.push(format!("{n} \u{2208} [{}, {}]", b.0, b.1));
                }
                let detail = match v {
                    ProofVerdict::Unknown => "the proven ranges don't decide it".to_string(),
                    _ => format!("from {}", srcs.join(", ")),
                };
                (v, detail)
            }
            _ => (ProofVerdict::Unknown, "no proven range for one of the terms".to_string()),
        };

        AssertionResult { query, parsed: true, now, now_detail, verdict, verdict_detail }
    }

    /// **Causal provenance**: trace the value currently in innermost-frame register
    /// `reg` back to the exact op that produced it, and recursively the ops that
    /// produced *that* op's inputs — the precise answer to "why is this value here?".
    /// Returns `None` only if the register holds nothing at the cursor.
    pub fn provenance(&self, reg: u16) -> Option<CausalNode> {
        let mut budget = PROVENANCE_MAX_NODES;
        self.trace_value(reg, self.cursor, PROVENANCE_MAX_DEPTH, &mut budget)
    }

    /// Walk back from `at_step` to the most recent op (at or before it) that wrote
    /// `reg`, then recurse on that op's source registers as of *its* input state.
    /// Strictly decreasing `at_step` guarantees termination.
    fn trace_value(&self, reg: u16, at_step: usize, depth: usize, budget: &mut usize) -> Option<CausalNode> {
        let (name, kind, value) = self.reg_value_at(at_step, reg)?;
        if *budget == 0 || depth == 0 {
            return Some(CausalNode {
                step: 0,
                pc: 0,
                op_text: String::new(),
                narration: String::new(),
                reg,
                name,
                kind,
                value,
                inputs: Vec::new(),
            });
        }
        *budget -= 1;

        // Find the producing op: stepping from state[i-1] runs the op at state[i-1].pc()
        // and lands its write in state[i]. Scan back for the latest write of `reg`.
        for i in (1..=at_step).rev() {
            let producer_pc = self.history[i - 1].state.pc();
            let Some(op) = self.program.code.get(producer_pc).copied() else { continue };
            let io = crate::vm::op_io(&op);
            if io.writes != Some(reg) {
                continue;
            }
            // A `Move` is a compiler-inserted copy (temp → named slot). It carries no
            // computation, so fold it: the value's real lineage is its source's, just
            // relabelled as this destination — the tree then matches the SOURCE data
            // flow ("w = z + x"), not the bytecode's temp shuffles.
            if let crate::vm::Op::Move { src, .. } = op {
                if let Some(mut child) = self.trace_value(src, i - 1, depth, budget) {
                    if let Some((nm, k, v)) = self.reg_value_at(i, reg) {
                        child.reg = reg;
                        child.name = nm;
                        child.kind = k;
                        child.value = v;
                    }
                    return Some(child);
                }
            }
            // Resolve this op's value as of the step it produced, its inputs as of the
            // input state (i-1) — register numbers are frame-relative to each state.
            let (pname, pkind, pvalue) = self
                .reg_value_at(i, reg)
                .unwrap_or_else(|| (name.clone(), kind.clone(), value.clone()));
            let inner = self.input_regs(i - 1);
            let narration = narrate(&op, &inner, &self.program);
            let op_text = self.disasm.get(producer_pc).map(|d| d.text.clone()).unwrap_or_default();
            let inputs = io
                .reads
                .iter()
                .filter_map(|r| self.trace_value(*r, i - 1, depth - 1, budget))
                .collect();
            return Some(CausalNode {
                step: i,
                pc: producer_pc,
                op_text,
                narration,
                reg,
                name: pname,
                kind: pkind,
                value: pvalue,
                inputs,
            });
        }

        // No op in history wrote this slot — it is an initial / constant / parameter.
        Some(CausalNode {
            step: 0,
            pc: 0,
            op_text: String::new(),
            narration: String::new(),
            reg,
            name,
            kind,
            value,
            inputs: Vec::new(),
        })
    }

    // ── internals ────────────────────────────────────────────────────────────

    /// Main-frame variable names (`R0` → `x`), as captured by the debug compile pass.
    fn main_names(&self) -> HashMap<u16, String> {
        self.program.reg_names.iter().cloned().collect()
    }

    /// The `(name, kind, value)` of innermost-frame register `reg` at explored `step`,
    /// or `None` if the slot is absent there. Names are applied for the Main frame.
    fn reg_value_at(&self, step: usize, reg: u16) -> Option<(Option<String>, String, String)> {
        let frame = self.history.get(step)?;
        let view = self.view_of(&frame.state);
        let f = view.frames.last()?;
        let is_main = f.func.is_none();
        f.registers.iter().find(|(idx, _, _)| *idx == reg).map(|(idx, kind, val)| {
            let name = if is_main { self.main_names().get(idx).cloned() } else { None };
            (name, kind.clone(), val.clone())
        })
    }

    /// The innermost-frame `(reg → (name, value))` map at `step`, for narrating an op
    /// in terms of the operands it actually read.
    fn input_regs(&self, step: usize) -> HashMap<u16, (Option<String>, String)> {
        let names = self.main_names();
        let Some(frame) = self.history.get(step) else { return HashMap::new() };
        let view = self.view_of(&frame.state);
        let Some(f) = view.frames.last() else { return HashMap::new() };
        let is_main = f.func.is_none();
        f.registers
            .iter()
            .map(|(idx, _, val)| {
                let name = if is_main { names.get(idx).cloned() } else { None };
                (*idx, (name, val.clone()))
            })
            .collect()
    }

    fn cur(&self) -> &Frame {
        &self.history[self.cursor]
    }

    fn current(&self) -> &DebugVmState {
        &self.cur().state
    }

    fn is_paused(&self) -> bool {
        matches!(self.cur().outcome, Outcome::Running)
    }

    fn current_depth(&self) -> usize {
        self.current().call_depth()
    }

    fn at_breakpoint(&self) -> bool {
        self.is_paused() && self.breakpoints.contains(&self.current().pc())
    }

    /// Rebuild a `tier: None` VM in `st`'s state and read its debug view.
    fn view_of(&self, st: &DebugVmState) -> crate::vm::DebugView {
        let mut vm = Vm::new(&self.program);
        vm.restore_debug_state(st.clone());
        vm.debug_view()
    }

    /// Like [`view_of`], plus the heap objects reachable from that state (one VM
    /// rebuild for both).
    fn view_and_heap(
        &self,
        st: &DebugVmState,
    ) -> (crate::vm::DebugView, Vec<crate::vm::HeapObjView>) {
        let mut vm = Vm::new(&self.program);
        vm.restore_debug_state(st.clone());
        (vm.debug_view(), vm.debug_heap())
    }

    /// Advance the cursor by one op. When the cursor is rewound into already-explored
    /// history this just steps it forward — execution is deterministic, so the cached
    /// frame is exactly what re-running would produce. At the frontier it computes and
    /// records the next frame. Returns whether the new position is still running.
    fn run_one(&mut self) -> bool {
        if !self.is_paused() {
            return false;
        }
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            return self.is_paused();
        }
        let cur_state = self.current().clone();
        let mut vm = Vm::new(&self.program);
        vm.restore_debug_state(cur_state.clone());
        let frame = match vm.run_steps(1) {
            Ok(VmStep::Paused) => Frame { state: vm.save_debug_state(), outcome: Outcome::Running },
            Ok(VmStep::Done(_)) => Frame { state: vm.save_debug_state(), outcome: Outcome::Done },
            Ok(VmStep::Blocked) => Frame { state: vm.save_debug_state(), outcome: Outcome::Blocked },
            // Keep the pre-error state so the snapshot shows the failing op's pc.
            Err(e) => Frame { state: cur_state, outcome: Outcome::Error(e) },
        };
        self.history.push(frame);
        self.cursor += 1;
        self.is_paused()
    }
}

/// A plain-English description of the op about to run, using variable names and live
/// values where available (e.g. `add x(6) + y(7) → R4`). The teaching narration; empty
/// for ops with no pedagogical value, so the UI falls back to the disassembly.
fn narrate(
    op: &crate::vm::Op,
    regs: &HashMap<u16, (Option<String>, String)>,
    prog: &CompiledProgram,
) -> String {
    use crate::vm::Op;
    let name = |r: u16| regs.get(&r).and_then(|(n, _)| n.clone()).unwrap_or_else(|| format!("R{r}"));
    let operand = |r: u16| match regs.get(&r) {
        Some((_, v)) if !v.is_empty() => format!("{}({})", name(r), v),
        _ => name(r),
    };
    match *op {
        Op::LoadConst { dst, idx } => {
            format!("load {} into {}", crate::vm::format_constant(prog, idx), name(dst))
        }
        Op::Move { dst, src } => format!("copy {} into {}", operand(src), name(dst)),
        Op::Add { dst, lhs, rhs } => format!("add {} + {} \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::Sub { dst, lhs, rhs } => format!("subtract {} - {} \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::Mul { dst, lhs, rhs } => format!("multiply {} \u{00d7} {} \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::Div { dst, lhs, rhs } | Op::ExactDiv { dst, lhs, rhs } => {
            format!("divide {} \u{00f7} {} \u{2192} {}", operand(lhs), operand(rhs), name(dst))
        }
        Op::Mod { dst, lhs, rhs } => format!("{} mod {} \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::Lt { dst, lhs, rhs } => format!("is {} < {}? \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::Gt { dst, lhs, rhs } => format!("is {} > {}? \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::LtEq { dst, lhs, rhs } => format!("is {} <= {}? \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::GtEq { dst, lhs, rhs } => format!("is {} >= {}? \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::Eq { dst, lhs, rhs } => format!("is {} == {}? \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::NotEq { dst, lhs, rhs } => format!("is {} != {}? \u{2192} {}", operand(lhs), operand(rhs), name(dst)),
        Op::AddAssign { dst, src } => format!("append {} onto {}", operand(src), name(dst)),
        Op::Not { dst, src } => format!("negate {} \u{2192} {}", operand(src), name(dst)),
        Op::Show { src } => format!("print {}", operand(src)),
        Op::Return { src } => format!("return {}", operand(src)),
        Op::ReturnNothing => "return".to_string(),
        Op::Jump { target } => format!("jump to step {target}"),
        Op::JumpIfFalse { cond, target } => format!("if {} is false, jump to {target}", operand(cond)),
        Op::JumpIfTrue { cond, target } => format!("if {} is true, jump to {target}", operand(cond)),
        Op::Index { dst, collection, index } | Op::IndexUnchecked { dst, collection, index } => {
            format!("read {}[{}] \u{2192} {}", name(collection), operand(index), name(dst))
        }
        Op::SetIndex { collection, index, value } | Op::SetIndexUnchecked { collection, index, value } => {
            format!("set {}[{}] = {}", name(collection), operand(index), operand(value))
        }
        Op::Length { dst, collection } => format!("count {} \u{2192} {}", name(collection), name(dst)),
        Op::ListPush { list, value } => format!("push {} onto {}", operand(value), name(list)),
        Op::NewEmptyList { dst } | Op::NewEmptyListI32 { dst } => format!("make an empty list \u{2192} {}", name(dst)),
        Op::Call { .. } => "call a function".to_string(),
        Op::Halt => "halt \u{2014} the program is done".to_string(),
        _ => String::new(),
    }
}

/// The **first-order-logic semantics** of the op about to run — its formal meaning as
/// a formula over the (named) registers: an assignment `dst = lhs + rhs`, a boolean
/// definition `dst ⟺ (lhs < rhs)`, a guarded jump `¬cond → goto L`. Symbolic (names,
/// not live values), so it reads as the verification condition the step establishes.
/// Empty for ops with no crisp logical reading (the UI then falls back to narration).
fn fol_of_op(
    op: &crate::vm::Op,
    regs: &HashMap<u16, (Option<String>, String)>,
    prog: &CompiledProgram,
) -> String {
    use crate::vm::Op;
    let name = |r: u16| regs.get(&r).and_then(|(n, _)| n.clone()).unwrap_or_else(|| format!("R{r}"));
    match *op {
        Op::LoadConst { dst, idx } => format!("{} = {}", name(dst), crate::vm::format_constant(prog, idx)),
        Op::Move { dst, src } => format!("{} = {}", name(dst), name(src)),
        Op::Add { dst, lhs, rhs } => format!("{} = {} + {}", name(dst), name(lhs), name(rhs)),
        Op::Sub { dst, lhs, rhs } => format!("{} = {} \u{2212} {}", name(dst), name(lhs), name(rhs)),
        Op::Mul { dst, lhs, rhs } => format!("{} = {} \u{00d7} {}", name(dst), name(lhs), name(rhs)),
        Op::Div { dst, lhs, rhs } | Op::ExactDiv { dst, lhs, rhs } => {
            format!("{} = {} \u{00f7} {}", name(dst), name(lhs), name(rhs))
        }
        Op::Mod { dst, lhs, rhs } => format!("{} = {} mod {}", name(dst), name(lhs), name(rhs)),
        Op::Lt { dst, lhs, rhs } => format!("{} \u{27fa} ({} < {})", name(dst), name(lhs), name(rhs)),
        Op::Gt { dst, lhs, rhs } => format!("{} \u{27fa} ({} > {})", name(dst), name(lhs), name(rhs)),
        Op::LtEq { dst, lhs, rhs } => format!("{} \u{27fa} ({} \u{2264} {})", name(dst), name(lhs), name(rhs)),
        Op::GtEq { dst, lhs, rhs } => format!("{} \u{27fa} ({} \u{2265} {})", name(dst), name(lhs), name(rhs)),
        Op::Eq { dst, lhs, rhs } => format!("{} \u{27fa} ({} = {})", name(dst), name(lhs), name(rhs)),
        Op::NotEq { dst, lhs, rhs } => format!("{} \u{27fa} ({} \u{2260} {})", name(dst), name(lhs), name(rhs)),
        Op::Not { dst, src } => format!("{} \u{27fa} \u{00ac}{}", name(dst), name(src)),
        Op::AddAssign { dst, src } => format!("{} \u{2254} {} \u{29fa} {}", name(dst), name(dst), name(src)),
        Op::Index { dst, collection, index } | Op::IndexUnchecked { dst, collection, index } => {
            format!("{} = {}[{}]", name(dst), name(collection), name(index))
        }
        Op::SetIndex { collection, index, value } | Op::SetIndexUnchecked { collection, index, value } => {
            format!("{}[{}] \u{2254} {}", name(collection), name(index), name(value))
        }
        Op::Length { dst, collection } => format!("{} = |{}|", name(dst), name(collection)),
        Op::ListPush { list, value } => format!("{} \u{2254} {} \u{2295} {}", name(list), name(list), name(value)),
        Op::NewEmptyList { dst } | Op::NewEmptyListI32 { dst } => format!("{} = \u{2205}", name(dst)),
        Op::Return { src } => format!("result = {}", name(src)),
        Op::Jump { target } => format!("goto {target}"),
        Op::JumpIfFalse { cond, target } => format!("\u{00ac}{} \u{2192} goto {target}", name(cond)),
        Op::JumpIfTrue { cond, target } => format!("{} \u{2192} goto {target}", name(cond)),
        _ => String::new(),
    }
}

/// A **Socratic** prompt for the op about to run — the "ask before telling" move: a
/// guiding question grounded in the live operand values that invites the learner to
/// predict the outcome, which stepping then reveals. Returns empty for ops with nothing
/// to anticipate (a literal load, a bare jump), so the UI falls back to the narration.
/// Matches the Socratic engine's voice (concrete values, second person, em-dash).
fn socratic_of_op(op: &crate::vm::Op, regs: &HashMap<u16, (Option<String>, String)>) -> String {
    use crate::vm::Op;
    let name = |r: u16| regs.get(&r).and_then(|(n, _)| n.clone()).unwrap_or_else(|| format!("R{r}"));
    // "x (6)" when a live value is known, else just the name.
    let nv = |r: u16| match regs.get(&r) {
        Some((_, v)) if !v.is_empty() => format!("{} ({})", name(r), v),
        _ => name(r),
    };
    match *op {
        Op::Add { lhs, rhs, .. } => format!("{} and {} \u{2014} what is their sum?", nv(lhs), nv(rhs)),
        Op::Sub { lhs, rhs, .. } => format!("{} minus {} \u{2014} what's left?", nv(lhs), nv(rhs)),
        Op::Mul { lhs, rhs, .. } => format!("{} times {} \u{2014} what do you get?", nv(lhs), nv(rhs)),
        Op::Div { lhs, rhs, .. } | Op::ExactDiv { lhs, rhs, .. } => {
            format!("{} divided by {} \u{2014} what is the quotient?", nv(lhs), nv(rhs))
        }
        Op::Mod { lhs, rhs, .. } => format!("What remains when {} is divided by {}?", nv(lhs), nv(rhs)),
        Op::Lt { lhs, rhs, .. } => format!("Is {} less than {}?", nv(lhs), nv(rhs)),
        Op::Gt { lhs, rhs, .. } => format!("Is {} greater than {}?", nv(lhs), nv(rhs)),
        Op::LtEq { lhs, rhs, .. } => format!("Is {} at most {}?", nv(lhs), nv(rhs)),
        Op::GtEq { lhs, rhs, .. } => format!("Is {} at least {}?", nv(lhs), nv(rhs)),
        Op::Eq { lhs, rhs, .. } => format!("Does {} equal {}?", nv(lhs), nv(rhs)),
        Op::NotEq { lhs, rhs, .. } => format!("Are {} and {} different?", nv(lhs), nv(rhs)),
        Op::Not { src, .. } => format!("{} \u{2014} what is its negation?", nv(src)),
        Op::JumpIfFalse { cond, .. } => {
            format!("{} \u{2014} will the program take this branch, or fall through?", nv(cond))
        }
        Op::JumpIfTrue { cond, .. } => {
            format!("{} \u{2014} is the condition met, so the program jumps?", nv(cond))
        }
        Op::Index { collection, index, .. } | Op::IndexUnchecked { collection, index, .. } => {
            format!("What sits at position {} of {}?", nv(index), name(collection))
        }
        Op::Length { collection, .. } => format!("How many items does {} hold?", name(collection)),
        Op::Return { src } => {
            format!("The result is about to be {} \u{2014} is that what you predicted?", nv(src))
        }
        _ => String::new(),
    }
}

/// Parse a comparison predicate `<term> <op> <term>` (e.g. `x < y`, `sum >= 0`). Two-
/// char operators are tried before single-char so `<=` isn't read as `<`. A term is an
/// integer literal or a variable name. `None` if it isn't a single comparison.
fn parse_comparison(s: &str) -> Option<(Operand, Cmp, Operand)> {
    for (sym, cmp) in [("<=", Cmp::Le), (">=", Cmp::Ge), ("==", Cmp::Eq), ("!=", Cmp::Ne)] {
        if let Some(i) = s.find(sym) {
            return build_cmp(&s[..i], &s[i + sym.len()..], cmp);
        }
    }
    for (sym, cmp) in [('<', Cmp::Lt), ('>', Cmp::Gt), ('=', Cmp::Eq)] {
        if let Some(i) = s.find(sym) {
            return build_cmp(&s[..i], &s[i + 1..], cmp);
        }
    }
    None
}

fn build_cmp(l: &str, r: &str, cmp: Cmp) -> Option<(Operand, Cmp, Operand)> {
    Some((operand_of(l)?, cmp, operand_of(r)?))
}

fn operand_of(s: &str) -> Option<Operand> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    Some(match t.parse::<i64>() {
        Ok(n) => Operand::Int(n),
        Err(_) => Operand::Var(t.to_string()),
    })
}

fn apply_cmp(a: i64, cmp: Cmp, b: i64) -> bool {
    match cmp {
        Cmp::Lt => a < b,
        Cmp::Le => a <= b,
        Cmp::Gt => a > b,
        Cmp::Ge => a >= b,
        Cmp::Eq => a == b,
        Cmp::Ne => a != b,
    }
}

/// Decide a comparison between two PROVEN ranges `a ∈ [al, ah]`, `b ∈ [bl, bh]`. Sound:
/// `ProvenTrue`/`ProvenFalse` are returned only when the relation holds (resp. fails) for
/// EVERY pair in the ranges; overlapping ranges that don't settle it give `Unknown`.
fn entail((al, ah): (i64, i64), cmp: Cmp, (bl, bh): (i64, i64)) -> ProofVerdict {
    use ProofVerdict::{ProvenFalse, ProvenTrue, Unknown};
    let singleton_eq = al == ah && bl == bh && al == bl;
    let disjoint = ah < bl || bh < al;
    match cmp {
        Cmp::Lt => {
            if ah < bl {
                ProvenTrue
            } else if al >= bh {
                ProvenFalse
            } else {
                Unknown
            }
        }
        Cmp::Le => {
            if ah <= bl {
                ProvenTrue
            } else if al > bh {
                ProvenFalse
            } else {
                Unknown
            }
        }
        Cmp::Gt => {
            if al > bh {
                ProvenTrue
            } else if ah <= bl {
                ProvenFalse
            } else {
                Unknown
            }
        }
        Cmp::Ge => {
            if al >= bh {
                ProvenTrue
            } else if ah < bl {
                ProvenFalse
            } else {
                Unknown
            }
        }
        Cmp::Eq => {
            if singleton_eq {
                ProvenTrue
            } else if disjoint {
                ProvenFalse
            } else {
                Unknown
            }
        }
        Cmp::Ne => {
            if disjoint {
                ProvenTrue
            } else if singleton_eq {
                ProvenFalse
            } else {
                Unknown
            }
        }
    }
}

/// Format an observed numeric bound without a spurious `.0` on whole numbers, so a
/// range over integers reads `[1, 5]`, not `[1.0, 5.0]`.
fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 9.007e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Compile `src` to **un-optimized** bytecode — faithful to the source so the
/// debugger steps `Let`, the arithmetic, and the `Show` as written, rather than the
/// run-path optimizer's folded form (which would erase the very variables you are
/// debugging). Output is identical either way (optimizations are semantics-
/// preserving), so stepping still matches a normal run.
///
/// Also runs the Oracle's abstract interpretation ONCE here and rolls its per-
/// occurrence facts up by variable name — the proven invariants the debugger shows.
/// This is the only caller of that rollup, so it is **zero cost** for every non-debug
/// compile (the production VM/JIT/AOT paths never touch it).
fn compile_source(src: &str) -> Result<(CompiledProgram, HashMap<String, ProvenFacts>), String> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        let program = crate::vm::Compiler::compile_for_debug(stmts, interner, Some(types))?;
        let facts = crate::optimize::oracle_analyze_with(stmts, interner);
        let proven = facts
            .summarize_variables(stmts)
            .into_iter()
            .map(|(sym, vf)| (interner.resolve(sym).to_string(), ProvenFacts::from(vf)))
            .collect();
        Ok((program, proven))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROG: &str = "## Main\n\nLet x be 6.\nLet y be 7.\nShow x + y.";

    fn run_to_done(dbg: &mut Debugger) {
        for _ in 0..10_000 {
            if dbg.snapshot().state != "paused" {
                break;
            }
            dbg.step();
        }
    }

    /// The pc of the first op whose disassembly starts with `prefix` (self-calibrating
    /// so the tests survive a change in the exact bytecode layout).
    fn pc_of(dbg: &Debugger, prefix: &str) -> usize {
        dbg.disassembly()
            .iter()
            .find(|l| l.text.starts_with(prefix))
            .unwrap_or_else(|| panic!("no `{prefix}` op in the disassembly"))
            .pc
    }

    #[test]
    fn arms_at_entry() {
        let dbg = Debugger::from_source(PROG).expect("compiles");
        let s = dbg.snapshot();
        assert_eq!(s.step, 0, "history cursor at the start");
        assert_eq!(s.pc, 0, "stopped before the first op");
        assert_eq!(s.state, "paused");
        assert!(s.output.is_empty());
        assert!(s.total_ops > 0, "the program has instructions");
        assert!(!s.frames.is_empty(), "at least the Main frame");
    }

    #[test]
    fn stepping_to_done_matches_a_normal_run() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        run_to_done(&mut dbg);
        let s = dbg.snapshot();
        assert_eq!(s.state, "done", "reaches completion");
        // The debugger's output is exactly the interpreter's output for the same source.
        let interp = crate::ui_bridge::interpret_for_ui_sync_with_args(PROG, &[]);
        assert_eq!(interp.error, None);
        assert_eq!(s.output, interp.lines, "stepped output == single-shot run output");
    }

    #[test]
    fn disassembly_is_faithful_to_the_source() {
        let dbg = Debugger::from_source(PROG).expect("compiles");
        let texts: Vec<&str> = dbg.disassembly().iter().map(|l| l.text.as_str()).collect();
        assert!(texts[0].starts_with("LoadConst"), "first op loads a literal");
        assert_eq!(texts.last(), Some(&"Halt"), "program ends in Halt");
        assert!(texts.iter().any(|t| t.starts_with("Add")), "the `x + y` add is present");
        assert!(texts.iter().any(|t| t.starts_with("Show")), "the `Show` is present");
    }

    #[test]
    fn step_into_advances_one_op() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        assert!(!dbg.snapshot().op_text.is_empty(), "paused on a real op");
        dbg.step();
        let s = dbg.snapshot();
        assert_eq!(s.step, 1, "history cursor advanced one op");
        assert_eq!(s.pc, 1, "straight-line pc advanced");
    }

    #[test]
    fn registers_carry_their_values() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        // Run up to (not through) the Show — by then x, y, and x+y have been computed.
        let show = pc_of(&dbg, "Show");
        dbg.set_breakpoint(show);
        dbg.resume();
        let s = dbg.snapshot();
        assert_eq!(s.pc, show);
        let main = &s.frames[0];
        let values: Vec<&str> = main.registers.iter().map(|r| r.value.as_str()).collect();
        assert!(values.contains(&"6"), "x's value is live in a register: {values:?}");
        assert!(values.contains(&"7"), "y's value is live in a register: {values:?}");
        assert!(values.contains(&"13"), "x + y was computed: {values:?}");
        // The op that wrote the sum marked its register changed on the prior step.
        assert!(
            main.registers.iter().any(|r| r.changed),
            "the most recent write is flagged changed"
        );
    }

    #[test]
    fn registers_carry_their_type() {
        // A learning aid: every live register reports the type of the value it holds,
        // so a student watching `x = 6` also sees `x : Int`.
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        let show = pc_of(&dbg, "Show");
        dbg.set_breakpoint(show);
        dbg.resume();
        let s = dbg.snapshot();
        let typed: Vec<(&str, &str)> = s.frames[0]
            .registers
            .iter()
            .map(|r| (r.kind.as_str(), r.value.as_str()))
            .collect();
        assert!(typed.contains(&("Int", "6")), "x:6 is typed Int: {typed:?}");
        assert!(typed.contains(&("Int", "13")), "x+y:13 is typed Int: {typed:?}");
        assert!(
            s.frames[0].registers.iter().all(|r| !r.kind.is_empty()),
            "no live register is left untyped: {typed:?}"
        );
    }

    #[test]
    fn variable_timeline_tracks_each_variable_over_time() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.resume();
        let tl = dbg.variable_timeline();
        let names: Vec<&str> = tl.vars.iter().map(|v| v.name.as_str()).collect();
        assert!(names.contains(&"x"), "x is a traced variable: {names:?}");
        assert!(names.contains(&"y"), "y is a traced variable: {names:?}");

        // Every trace spans the full (windowed) history, so the oscilloscope is aligned.
        for v in &tl.vars {
            assert_eq!(v.points.len(), tl.steps, "trace {} spans the timeline", v.name);
        }
        let x = tl.vars.iter().find(|v| v.name == "x").unwrap();
        assert_eq!(x.kind, "Int", "x is typed on its trace");
        // x takes the value 6 at some edge and is holding 6 once the program has run.
        assert!(x.points.iter().any(|p| p.value == "6" && p.changed), "x has a 6-edge");
        assert_eq!(x.points.last().unwrap().value, "6", "x ends at 6");
        assert_eq!(tl.cursor, dbg.snapshot().step, "playhead is at the cursor");
    }

    #[test]
    fn snapshot_carries_fol_semantics() {
        // Each executing op exposes its first-order-logic meaning.
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        let mut fols = Vec::new();
        while dbg.is_running() {
            let s = dbg.snapshot();
            if !s.fol.is_empty() {
                fols.push(s.fol.clone());
            }
            dbg.step();
        }
        assert!(fols.iter().any(|f| f == "R0 = 6"), "literal load reads as R0 = 6: {fols:?}");
        assert!(fols.iter().any(|f| f == "x = R0"), "the copy into x reads as x = R0: {fols:?}");
        assert!(fols.iter().any(|f| f.contains("x + y")), "the addition reads over named vars: {fols:?}");
    }

    #[test]
    fn fol_renders_a_comparison_as_a_biconditional() {
        // `is x < y` compiles to a comparison op whose FOL is a biconditional.
        let src = "## Main\n\nLet x be 6.\nLet y be 7.\nLet t be x < y.\nShow t.";
        let mut dbg = Debugger::from_source(src).expect("compiles");
        let mut fols = Vec::new();
        while dbg.is_running() {
            let s = dbg.snapshot();
            if !s.fol.is_empty() {
                fols.push(s.fol.clone());
            }
            dbg.step();
        }
        assert!(
            fols.iter().any(|f| f.contains("\u{27fa}") && f.contains("x < y")),
            "the comparison reads as a biconditional: {fols:?}"
        );
    }

    #[test]
    fn socratic_prompt_asks_the_learner_to_predict() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        let mut qs = Vec::new();
        while dbg.is_running() {
            let s = dbg.snapshot();
            if !s.socratic.is_empty() {
                qs.push(s.socratic.clone());
            }
            dbg.step();
        }
        assert!(
            qs.iter().any(|q| q.ends_with('?') && q.contains("sum") && q.contains("(6)") && q.contains("(7)")),
            "the addition asks the learner to predict the sum from the live operands: {qs:?}"
        );
        assert!(qs.iter().all(|q| q.contains('?')), "every Socratic prompt is a question: {qs:?}");
    }

    #[test]
    fn socratic_poses_a_yes_no_question_for_a_comparison() {
        let src = "## Main\n\nLet x be 6.\nLet y be 7.\nLet t be x < y.\nShow t.";
        let mut dbg = Debugger::from_source(src).expect("compiles");
        let mut qs = Vec::new();
        while dbg.is_running() {
            let s = dbg.snapshot();
            if !s.socratic.is_empty() {
                qs.push(s.socratic.clone());
            }
            dbg.step();
        }
        assert!(
            qs.iter().any(|q| q.starts_with("Is ") && q.contains("less than") && q.ends_with('?')),
            "the comparison is posed as a yes/no question: {qs:?}"
        );
    }

    #[test]
    fn socratic_done_state_invites_reflection() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.resume();
        let s = dbg.snapshot();
        assert!(s.socratic.contains("expected"), "the finished prompt invites reflection: {}", s.socratic);
    }

    #[test]
    fn live_proof_is_true_now_and_proven_for_every_run() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.resume();
        let r = dbg.assert_at_cursor("x < y");
        assert!(r.parsed, "parsed the comparison");
        assert_eq!(r.now, Some(true), "x=6 < y=7 holds now: {}", r.now_detail);
        assert_eq!(r.verdict, ProofVerdict::ProvenTrue, "proven for every run: {}", r.verdict_detail);
        assert!(r.verdict_detail.contains("[6, 6]"), "cites the proven ranges: {}", r.verdict_detail);
    }

    #[test]
    fn live_proof_statically_refutes_a_false_predicate() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.resume();
        let r = dbg.assert_at_cursor("x > y");
        assert_eq!(r.now, Some(false), "x=6 > y=7 is false now");
        assert_eq!(r.verdict, ProofVerdict::ProvenFalse, "refuted for every run: {}", r.verdict_detail);
    }

    #[test]
    fn live_proof_proves_a_constant_equality_and_a_bound() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.resume();
        assert_eq!(dbg.assert_at_cursor("x == 6").verdict, ProofVerdict::ProvenTrue, "x is provably 6");
        assert_eq!(dbg.assert_at_cursor("x >= 0").verdict, ProofVerdict::ProvenTrue, "x is provably non-negative");
        assert_eq!(dbg.assert_at_cursor("y <= 100").verdict, ProofVerdict::ProvenTrue, "y is provably under 100");
    }

    #[test]
    fn live_proof_rejects_unparseable_input() {
        let dbg = Debugger::from_source(PROG).expect("compiles");
        let r = dbg.assert_at_cursor("hello world");
        assert!(!r.parsed, "garbage is not a comparison");
        assert_eq!(r.verdict, ProofVerdict::Unknown);
    }

    #[test]
    fn proven_invariants_prove_constant_values_and_types() {
        // The Oracle statically proves x ≡ 6 and y ≡ 7 (singleton ranges) and Int type —
        // facts that hold on EVERY run, available before stepping.
        let dbg = Debugger::from_source(PROG).expect("compiles");
        let proven = dbg.proven_invariants();
        let names: Vec<&str> = proven.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"x"), "x has proven facts: {names:?}");
        let x = proven.iter().find(|p| p.name == "x").unwrap();
        assert!(x.facts.iter().any(|f| f.contains("[6, 6]")), "x proven \u{2208} [6,6]: {:?}", x.facts);
        assert!(x.facts.iter().any(|f| f.contains("Int")), "x proven Int: {:?}", x.facts);
        let y = proven.iter().find(|p| p.name == "y").unwrap();
        assert!(y.facts.iter().any(|f| f.contains("[7, 7]")), "y proven \u{2208} [7,7]: {:?}", y.facts);
    }

    #[test]
    fn proven_facts_are_available_without_stepping() {
        // Proven facts come from compile-time analysis, not execution — present at cursor 0.
        let dbg = Debugger::from_source(PROG).expect("compiles");
        assert_eq!(dbg.snapshot().step, 0, "fresh debugger, nothing executed");
        assert!(!dbg.proven_invariants().is_empty(), "proven facts ready before any step");
    }

    #[test]
    fn observed_invariants_report_a_constant() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.resume();
        let ins = dbg.observed_invariants();
        let x = ins.iter().find(|i| i.name == "x").expect("x has insights");
        assert!(
            x.facts.iter().any(|f| f.contains("constant") && f.contains('6')),
            "x is observed constant 6: {:?}",
            x.facts
        );
    }

    #[test]
    fn observed_invariants_detect_a_monotonic_range() {
        let src = "## Main\n\nLet n be 1.\nSet n to 2.\nSet n to 3.\nShow n.";
        let mut dbg = Debugger::from_source(src).expect("compiles");
        dbg.resume();
        let ins = dbg.observed_invariants();
        let n = ins.iter().find(|i| i.name == "n").expect("n has insights");
        assert!(
            n.facts.iter().any(|f| f.contains("range") && f.contains('1') && f.contains('3')),
            "n ranges over [1,3]: {:?}",
            n.facts
        );
        assert!(n.facts.iter().any(|f| f.contains("increase")), "n only increases: {:?}", n.facts);
    }

    #[test]
    fn provenance_explains_why_a_value_exists() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.resume();
        let snap = dbg.snapshot();
        // The slot holding x + y = 13.
        let sum = snap.frames.last().unwrap().registers.iter().find(|r| r.value == "13").unwrap().index;
        let node = dbg.provenance(sum).expect("13 has a provenance");
        assert_eq!(node.value, "13");
        assert!(
            node.narration.contains("add") || node.op_text.to_lowercase().contains("add"),
            "the sum was produced by an add: {} / {}",
            node.op_text,
            node.narration
        );
        // Its inputs trace back to the two literals 6 and 7.
        let input_vals: Vec<&str> = node.inputs.iter().map(|n| n.value.as_str()).collect();
        assert!(input_vals.contains(&"6"), "one input is 6: {input_vals:?}");
        assert!(input_vals.contains(&"7"), "one input is 7: {input_vals:?}");
    }

    #[test]
    fn provenance_bottoms_out_at_a_constant_load() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.resume();
        let snap = dbg.snapshot();
        let x = snap
            .frames
            .last()
            .unwrap()
            .registers
            .iter()
            .find(|r| r.name.as_deref() == Some("x"))
            .unwrap()
            .index;
        let node = dbg.provenance(x).expect("x has a provenance");
        assert_eq!(node.value, "6");
        assert!(node.inputs.is_empty(), "a literal load consumes no registers");
        assert!(
            node.narration.contains("load") || node.op_text.to_lowercase().contains("load"),
            "x came from a load: {} / {}",
            node.op_text,
            node.narration
        );
    }

    #[test]
    fn provenance_chains_through_a_dependent_assignment() {
        // z depends on the sum, which depends on x and y — a two-level lineage.
        let src = "## Main\n\nLet x be 6.\nLet y be 7.\nLet z be x + y.\nLet w be z + x.\nShow w.";
        let mut dbg = Debugger::from_source(src).expect("compiles");
        dbg.resume();
        let snap = dbg.snapshot();
        let w = snap
            .frames
            .last()
            .unwrap()
            .registers
            .iter()
            .find(|r| r.name.as_deref() == Some("w"))
            .unwrap()
            .index;
        let node = dbg.provenance(w).expect("w has a provenance");
        assert_eq!(node.value, "19", "w = (6+7) + 6");
        // One input is z = 13, and z itself decomposes into 6 and 7.
        let z = node.inputs.iter().find(|n| n.value == "13").expect("w reads z=13");
        let z_inputs: Vec<&str> = z.inputs.iter().map(|n| n.value.as_str()).collect();
        assert!(z_inputs.contains(&"6") && z_inputs.contains(&"7"), "z traces to 6 and 7: {z_inputs:?}");
    }

    #[test]
    fn variable_names_are_resolved() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        let show = pc_of(&dbg, "Show");
        dbg.set_breakpoint(show);
        dbg.resume();
        let s = dbg.snapshot();
        let named: Vec<(&str, &str)> = s.frames[0]
            .registers
            .iter()
            .filter_map(|r| r.name.as_deref().map(|n| (n, r.value.as_str())))
            .collect();
        assert!(named.contains(&("x", "6")), "x = 6 shown by name: {named:?}");
        assert!(named.contains(&("y", "7")), "y = 7 shown by name: {named:?}");
    }

    #[test]
    fn production_compile_carries_no_debug_names() {
        // The debugger's compile path captures variable names…
        let (dbg_prog, _proven) = compile_source(PROG).expect("debug compile");
        assert!(!dbg_prog.reg_names.is_empty(), "debug path records reg_names");
        // …but the production compile path records NOTHING — the debug info is gated,
        // so it strips out of shipped builds at zero cost.
        let prod_prog = crate::ui_bridge::with_parsed_program(PROG, |parsed, interner| {
            let (stmts, types, _policies) = parsed?;
            crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
        })
        .expect("production compile");
        assert!(prod_prog.reg_names.is_empty(), "production path captures no debug names");
    }

    #[test]
    fn breakpoint_halts_continue() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        let show = pc_of(&dbg, "Show");
        dbg.set_breakpoint(show);
        dbg.resume();
        let s = dbg.snapshot();
        assert_eq!(s.pc, show, "Continue stopped at the breakpoint");
        assert!(s.at_breakpoint);
        assert_eq!(s.state, "paused");
        assert!(s.output.is_empty(), "the Show has not run yet");
        // One more step emits the output.
        dbg.step();
        assert!(!dbg.snapshot().output.is_empty(), "stepping the Show emits a line");
    }

    #[test]
    fn time_travel_steps_backwards() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        dbg.step();
        dbg.step();
        dbg.step();
        let three = dbg.snapshot();
        assert_eq!(three.step, 3);
        let pc_at_three = three.pc;
        dbg.step_back();
        assert_eq!(dbg.snapshot().step, 2, "rewound one op");
        // Forward again reproduces the same state (deterministic replay).
        dbg.step();
        let again = dbg.snapshot();
        assert_eq!(again.step, 3);
        assert_eq!(again.pc, pc_at_three, "replay is deterministic");
    }

    #[test]
    fn restart_rewinds_to_entry() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        run_to_done(&mut dbg);
        assert_eq!(dbg.snapshot().state, "done");
        dbg.restart();
        let s = dbg.snapshot();
        assert_eq!(s.step, 0);
        assert_eq!(s.pc, 0);
        assert_eq!(s.state, "paused");
        assert!(s.output.is_empty());
    }

    // ── Audit: loops, calls, recursion, errors, time-travel ───────────────────

    const FUNC: &str =
        "## To double (x: Int) -> Int:\n    Return x + x.\n\n## Main\nLet result be double(5).\nShow result.";
    const WHILE: &str = "## Main\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set sum to sum + i.\n    Set i to i + 1.\nShow sum.";

    /// Step a program op-by-op to completion and return the final snapshot.
    fn drive_to_end(src: &str) -> DebugSnapshot {
        let mut dbg = Debugger::from_source(src)
            .unwrap_or_else(|e| panic!("compile failed: {e}\n--- src ---\n{src}"));
        let mut guard = 0;
        while dbg.is_running() && guard < 500_000 {
            dbg.step();
            guard += 1;
        }
        dbg.snapshot()
    }

    /// THE soundness invariant: stepping any program op-by-op (rebuilding VM state
    /// each step) must produce output BYTE-IDENTICAL to a normal interpreter run.
    /// This exercises every save/restore path — `iter_stack` (loops), call frames
    /// (functions/recursion), globals, and lists.
    #[test]
    fn corpus_stepped_output_matches_interpreter() {
        let corpus: &[(&str, &str)] = &[
            ("while_loop", WHILE),
            ("repeat_range", "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 5:\n    Set total to total + i.\nShow total."),
            ("for_in", "## Main\nLet mutable sum be 0.\nRepeat for x in [10, 20, 30]:\n    Set sum to sum + x.\nShow sum."),
            ("conditional", "## Main\nLet x be 3.\nIf x is greater than 5:\n    Show \"big\".\nOtherwise:\n    Show \"small\"."),
            ("function", FUNC),
            ("recursion", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(6)."),
            ("list", "## Main\nLet xs be [1, 2, 3].\nPush 4 to xs.\nShow length of xs."),
        ];
        for (name, src) in corpus {
            let oracle = crate::ui_bridge::interpret_for_ui_sync_with_args(src, &[]);
            assert_eq!(oracle.error, None, "{name}: the interpreter itself errored: {:?}", oracle.error);
            let snap = drive_to_end(src);
            assert_eq!(
                snap.state, "done",
                "{name}: debugger did not finish (state={}, out={:?})", snap.state, snap.output
            );
            assert_eq!(snap.output, oracle.lines, "{name}: stepped output diverged from the interpreter");
        }
    }

    #[test]
    fn stepping_a_while_loop_runs_all_iterations() {
        let snap = drive_to_end(WHILE);
        assert_eq!(snap.state, "done");
        assert_eq!(snap.output, vec!["15".to_string()], "op-by-op stepping summed 1..=5");
    }

    #[test]
    fn call_stack_descends_into_the_function() {
        let mut dbg = Debugger::from_source(FUNC).expect("compiles");
        let mut entered = false;
        let mut guard = 0;
        while dbg.is_running() && guard < 10_000 {
            let s = dbg.snapshot();
            if s.frames.len() >= 2 {
                entered = true;
                assert!(s.frames.last().unwrap().function.is_some(), "inner frame is a function");
                break;
            }
            dbg.step();
            guard += 1;
        }
        assert!(entered, "stepping descends into the called function");
    }

    #[test]
    fn step_over_runs_the_call_without_descending() {
        let mut dbg = Debugger::from_source(FUNC).expect("compiles");
        let call_pc = pc_of(&dbg, "Call");
        let mut guard = 0;
        while dbg.snapshot().pc != call_pc && dbg.is_running() && guard < 1000 {
            dbg.step();
            guard += 1;
        }
        assert_eq!(dbg.snapshot().pc, call_pc, "reached the Call op");
        let depth = dbg.snapshot().frames.len();
        dbg.step_over();
        let s = dbg.snapshot();
        assert_eq!(s.frames.len(), depth, "step-over did not leave us inside the callee");
        assert!(s.pc > call_pc || s.state == "done", "advanced past the call");
    }

    #[test]
    fn step_out_returns_to_the_caller() {
        let mut dbg = Debugger::from_source(FUNC).expect("compiles");
        let mut guard = 0;
        while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
            dbg.step();
            guard += 1;
        }
        assert_eq!(dbg.snapshot().frames.len(), 2, "inside the function");
        dbg.step_out();
        assert!(dbg.snapshot().frames.len() <= 1, "step-out returns to the caller");
    }

    #[test]
    fn runtime_error_surfaces_without_panicking() {
        let mut dbg = Debugger::from_source("## Main\nLet x be 1 / 0.\nShow x.").expect("compiles");
        let mut guard = 0;
        while dbg.is_running() && guard < 100 {
            dbg.step();
            guard += 1;
        }
        let s = dbg.snapshot();
        assert_eq!(s.state, "error", "division by zero surfaces as an error, not a panic");
        assert!(s.error.is_some(), "the error carries a message");
    }

    #[test]
    fn entering_a_function_does_not_flag_spurious_changes() {
        let mut dbg = Debugger::from_source(FUNC).expect("compiles");
        let mut guard = 0;
        while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
            dbg.step();
            guard += 1;
        }
        let s = dbg.snapshot();
        assert_eq!(s.frames.len(), 2, "entered the function");
        assert!(
            s.frames.last().unwrap().registers.iter().all(|r| !r.changed),
            "crossing into a new frame must not flag stale registers as changed"
        );
    }

    #[test]
    fn time_travel_across_a_call_restores_the_frame() {
        let mut dbg = Debugger::from_source(FUNC).expect("compiles");
        let mut guard = 0;
        while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
            dbg.step();
            guard += 1;
        }
        let inside = dbg.snapshot();
        assert_eq!(inside.frames.len(), 2, "inside the function");
        let (pc, step) = (inside.pc, inside.step);
        dbg.step();
        dbg.step_back();
        let back = dbg.snapshot();
        assert_eq!(back.step, step, "rewound to the in-function step");
        assert_eq!(back.pc, pc, "pc restored exactly");
        assert_eq!(back.frames.len(), 2, "still inside the function after the rewind");
    }

    #[test]
    fn narration_explains_each_step_in_english() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        let mut narrations = Vec::new();
        let mut guard = 0;
        while dbg.is_running() && guard < 1000 {
            let s = dbg.snapshot();
            if !s.narration.is_empty() {
                narrations.push(s.narration.clone());
            }
            dbg.step();
            guard += 1;
        }
        let all = narrations.join(" | ");
        assert!(all.contains("add"), "an add step is narrated in English: {all}");
        assert!(all.contains("print"), "the print is narrated: {all}");
        assert!(
            narrations.iter().any(|n| n.contains("x(6)") && n.contains("y(7)")),
            "the add narration names the variables and their live values: {narrations:?}"
        );
    }

    #[test]
    fn op_io_in_snapshot_targets_the_operands() {
        let mut dbg = Debugger::from_source(PROG).expect("compiles");
        let add = pc_of(&dbg, "Add");
        let mut guard = 0;
        while dbg.snapshot().pc != add && dbg.is_running() && guard < 100 {
            dbg.step();
            guard += 1;
        }
        let s = dbg.snapshot();
        assert_eq!(s.pc, add);
        assert!(s.op_writes.is_some(), "the Add writes a destination register");
        assert_eq!(s.op_reads.len(), 2, "the Add reads its two operands (for the datapath)");
    }

    #[test]
    fn seek_scrubs_anywhere_in_history() {
        let mut dbg = Debugger::from_source(WHILE).expect("compiles");
        while dbg.is_running() {
            dbg.step();
        }
        let end = dbg.snapshot();
        assert_eq!(end.state, "done");
        let total = end.total_steps;
        assert!(total > 3, "the loop took several ops");
        // Scrub to the very start.
        dbg.seek(0);
        let s0 = dbg.snapshot();
        assert_eq!(s0.step, 0);
        assert_eq!(s0.state, "paused");
        assert!(s0.output.is_empty(), "no output has happened at the entry");
        // A middle step reads "paused" (per-frame outcome), not the program's "done".
        dbg.seek(total / 2);
        assert_eq!(dbg.snapshot().step, total / 2);
        assert_eq!(dbg.snapshot().state, "paused");
        // Scrub back to the end → final output restored.
        dbg.seek(total);
        let again = dbg.snapshot();
        assert_eq!(again.step, total);
        assert_eq!(again.state, "done");
        assert_eq!(again.output, end.output, "scrubbing to the end restores the final output");
    }

    #[test]
    fn reverse_continue_runs_back_to_a_breakpoint() {
        let mut dbg = Debugger::from_source(WHILE).expect("compiles");
        let show = pc_of(&dbg, "Show");
        while dbg.is_running() {
            dbg.step();
        }
        assert_eq!(dbg.snapshot().state, "done");
        dbg.set_breakpoint(show);
        dbg.reverse_resume();
        let s = dbg.snapshot();
        assert_eq!(s.pc, show, "reverse-continue landed on the breakpoint");
        assert!(s.at_breakpoint);
        assert!(s.output.is_empty(), "rewound to the moment before the Show ran");
    }

    #[test]
    fn restart_rewinds_but_keeps_explored_history() {
        let mut dbg = Debugger::from_source(WHILE).expect("compiles");
        while dbg.is_running() {
            dbg.step();
        }
        let total = dbg.snapshot().total_steps;
        dbg.restart();
        let s = dbg.snapshot();
        assert_eq!(s.step, 0, "back at the entry");
        assert_eq!(s.state, "paused");
        assert_eq!(s.total_steps, total, "explored history is retained, so re-stepping is instant");
    }

    #[test]
    fn heap_view_lists_a_distinct_object() {
        let src = "## Main\nLet xs be [1, 2, 3].\nShow length of xs.";
        let mut dbg = Debugger::from_source(src).expect("compiles");
        while dbg.is_running() {
            dbg.step();
        }
        let s = dbg.snapshot();
        assert!(
            s.heap.iter().any(|o| o.kind == "list" && o.referenced_by.contains(&"xs".to_string())),
            "the list `xs` shows up as a heap object: {:?}", s.heap
        );
    }

    #[test]
    fn heap_view_shows_storage_layout() {
        // A list of ints is stored as a PACKED `Vec<i64>`, not boxed values — the
        // memory layout the debugger teaches.
        let src = "## Main\nLet xs be [1, 2, 3].\nShow length of xs.";
        let mut dbg = Debugger::from_source(src).expect("compiles");
        while dbg.is_running() {
            dbg.step();
        }
        let s = dbg.snapshot();
        let list = s.heap.iter().find(|o| o.kind == "list").expect("a list on the heap");
        assert_eq!(list.storage, "packed Vec<i64>", "an int list is densely packed: {list:?}");
    }

    #[test]
    fn heap_view_reveals_aliasing() {
        // `Let b be a` aliases the SAME list — the classic beginner trap. The heap view
        // must show ONE allocation referenced by both, not two copies.
        let src = "## Main\nLet a be [1, 2, 3].\nLet b be a.\nShow a.";
        let mut dbg = Debugger::from_source(src).expect("compiles");
        while dbg.is_running() {
            dbg.step();
        }
        let s = dbg.snapshot();
        let lists: Vec<&HeapObject> = s.heap.iter().filter(|o| o.kind == "list").collect();
        assert_eq!(lists.len(), 1, "a and b share ONE list allocation, not two: {:?}", s.heap);
        let list = lists[0];
        assert!(list.referenced_by.contains(&"a".to_string()), "`a` references it: {list:?}");
        assert!(list.referenced_by.contains(&"b".to_string()), "`b` references it: {list:?}");
        assert!(list.shared, "aliasing is flagged");
    }

    #[test]
    fn stack_frames_carry_their_base_address() {
        let mut dbg = Debugger::from_source(FUNC).expect("compiles");
        let mut guard = 0;
        while dbg.is_running() && dbg.snapshot().frames.len() < 2 && guard < 1000 {
            dbg.step();
            guard += 1;
        }
        let s = dbg.snapshot();
        assert_eq!(s.frames.len(), 2, "inside the function");
        assert_eq!(s.frames[0].base, 0, "the Main frame starts at stack address 0");
        assert!(s.frames[1].base > 0, "the callee frame is stacked above Main (higher address)");
    }
}
