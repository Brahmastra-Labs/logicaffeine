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
}

impl Debugger {
    /// Compile `src` (exactly as the Studio "Run" path does) and arm a debugger at
    /// the program's entry. The program is debugged on the bytecode tier with no
    /// JIT, so stepping is per-op and output matches a normal run.
    pub fn from_source(src: &str) -> Result<Debugger, String> {
        let program = compile_source(src)?;
        let disasm = disassemble(&program);
        let initial = Vm::new(&program).save_debug_state();
        Ok(Debugger {
            program,
            disasm,
            history: vec![Frame { state: initial, outcome: Outcome::Running }],
            cursor: 0,
            breakpoints: BTreeSet::new(),
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
                pv.frames.last().map(|f| f.registers.iter().cloned().collect()).unwrap_or_default()
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
                        .map(|(idx, val)| DebugReg {
                            index: *idx,
                            name: if f.func.is_none() {
                                main_names.get(idx).cloned()
                            } else {
                                None
                            },
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
        let heap: Vec<HeapObject> = heap_raw
            .iter()
            .enumerate()
            .map(|(i, o)| HeapObject {
                id: format!("#{}", i + 1),
                kind: o.kind.clone(),
                summary: o.summary.clone(),
                rc: o.rc,
                referenced_by: o.referenced_by.clone(),
                shared: o.referenced_by.len() > 1,
            })
            .collect();
        DebugSnapshot {
            pc: view.pc,
            op_text,
            narration,
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

    // ── internals ────────────────────────────────────────────────────────────

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

/// Compile `src` to **un-optimized** bytecode — faithful to the source so the
/// debugger steps `Let`, the arithmetic, and the `Show` as written, rather than the
/// run-path optimizer's folded form (which would erase the very variables you are
/// debugging). Output is identical either way (optimizations are semantics-
/// preserving), so stepping still matches a normal run.
fn compile_source(src: &str) -> Result<CompiledProgram, String> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        crate::vm::Compiler::compile_for_debug(stmts, interner, Some(types))
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
        let dbg_prog = compile_source(PROG).expect("debug compile");
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
