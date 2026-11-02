//! W11 — `DebugSession`: the one seam the DAP adapter and the Studio drawer both
//! sit on.
//!
//! A thin DAP-vocabulary facade over the bytecode [`Debugger`](crate::debug), so
//! the two consumers can never disagree about the paused program: `threads`,
//! `stackTrace`, `scopes`/`variables`, the stepping verbs (including the headline
//! `stepBack`/`reverseContinue` time-travel), and `evaluate` — routed through the
//! Wave-10 [`frame_eval`](crate::frame_eval), the tree-walker oracle, so a watch
//! expression means exactly what the same expression means in a program.
//!
//! The stdio DAP server (`largo dap`) is a serializer *over* this facade; the
//! Studio debug drawer is a renderer *over* it. `session_parity_lock` proves both
//! observe the identical state the underlying `Debugger` reports — the facade is
//! a lens, never a second source of truth.

use serde::Serialize;

use crate::debug::Debugger;
use crate::frame_eval::{frame_eval, Binding};

/// A DAP thread. Single-task debugging (real tasks arrive with Wave 12's
/// concurrency debugger) ⇒ exactly one main thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DapThread {
    pub id: i64,
    pub name: String,
}

/// A DAP stack frame, innermost-first (DAP orders the current frame at index 0;
/// the debugger snapshot lists Main first, current last, so the facade reverses).
/// `line` carries the time-travel step — the honest bytecode-tier analogue of a
/// source line, and what makes the scrubber and the editor gutter agree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DapFrame {
    pub id: i64,
    pub name: String,
    pub line: usize,
}

/// A DAP variable in a frame's Locals scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DapVariable {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
}

/// A debug session: owns a [`Debugger`] and answers DAP-shaped queries against it.
pub struct DebugSession {
    dbg: Debugger,
}

impl DebugSession {
    /// `launch` — compile the program and stop before the first op.
    pub fn launch(src: &str) -> Result<DebugSession, String> {
        Ok(DebugSession {
            dbg: Debugger::from_source(src)?,
        })
    }

    /// `threads` — the single main thread.
    pub fn threads(&self) -> Vec<DapThread> {
        vec![DapThread {
            id: 1,
            name: "main".to_string(),
        }]
    }

    /// `stackTrace` — the call frames, innermost-first (DAP order).
    pub fn stack_trace(&self) -> Vec<DapFrame> {
        let snap = self.dbg.snapshot();
        let line = snap.step;
        snap.frames
            .iter()
            .rev()
            .enumerate()
            .map(|(i, f)| DapFrame {
                id: i as i64,
                name: f.function.clone().unwrap_or_else(|| "Main".to_string()),
                line,
            })
            .collect()
    }

    /// `variables` for a frame (addressed by its `stackTrace` id) — every register
    /// in that frame, named when the compiler's debug-info pass knew the source
    /// name (else `R{index}`).
    pub fn variables(&self, frame_id: i64) -> Vec<DapVariable> {
        let snap = self.dbg.snapshot();
        let n = snap.frames.len();
        // The DAP id is the reversed index; map back to Main-first order.
        let idx = match n.checked_sub(1).and_then(|last| {
            usize::try_from(frame_id)
                .ok()
                .and_then(|f| last.checked_sub(f))
        }) {
            Some(i) => i,
            None => return Vec::new(),
        };
        match snap.frames.get(idx) {
            None => Vec::new(),
            Some(frame) => frame
                .registers
                .iter()
                .map(|r| DapVariable {
                    name: r.name.clone().unwrap_or_else(|| format!("R{}", r.index)),
                    kind: r.kind.clone(),
                    value: r.value.clone(),
                })
                .collect(),
        }
    }

    // ── the stepping verbs ─────────────────────────────────────────────────
    /// `next` — step over the current op.
    pub fn next(&mut self) {
        self.dbg.step_over();
    }
    /// `stepIn` — execute exactly one op.
    pub fn step_in(&mut self) {
        self.dbg.step();
    }
    /// `stepOut` — run to the caller.
    pub fn step_out(&mut self) {
        self.dbg.step_out();
    }
    /// `continue` — run to the next breakpoint or the end.
    pub fn continue_(&mut self) {
        self.dbg.resume();
    }
    /// `stepBack` — the time-travel headline (`supportsStepBack: true`).
    pub fn step_back(&mut self) {
        self.dbg.step_back();
    }
    /// `reverseContinue` — run backwards to the previous breakpoint or the start.
    pub fn reverse_continue(&mut self) {
        self.dbg.reverse_resume();
    }
    /// `setBreakpoints` — arm one op-address breakpoint.
    pub fn set_breakpoint(&mut self, pc: usize) {
        self.dbg.set_breakpoint(pc);
    }
    /// Clear one op-address breakpoint.
    pub fn clear_breakpoint(&mut self, pc: usize) {
        self.dbg.clear_breakpoint(pc);
    }

    // ── state queries ──────────────────────────────────────────────────────
    /// The time-travel cursor (ops executed so far).
    pub fn step(&self) -> usize {
        self.dbg.snapshot().step
    }
    /// The pc the session is stopped at (the op about to execute).
    pub fn pc(&self) -> usize {
        self.dbg.snapshot().pc
    }
    /// Whether the program is still running (not finished, not errored).
    pub fn is_running(&self) -> bool {
        self.dbg.is_running()
    }
    /// The program output emitted so far (the `Show` lines).
    pub fn output(&self) -> Vec<String> {
        self.dbg.snapshot().output
    }

    /// `evaluate` — evaluate a LOGOS expression against the current (innermost)
    /// frame's named variables, through the Wave-10 frame evaluator (the
    /// tree-walker oracle). Scalar bindings round-trip through LOGOS source;
    /// complex values fail exactly as the tree-walker would.
    pub fn evaluate(&self, expr: &str) -> Result<String, String> {
        let snap = self.dbg.snapshot();
        let frame = match snap.frames.last() {
            Some(f) => f,
            None => return Err("no active frame".to_string()),
        };
        let bindings: Vec<(String, String)> = frame
            .registers
            .iter()
            .filter_map(|r| r.name.clone().map(|n| (n, r.value.clone())))
            .collect();
        let refs: Vec<Binding<'_>> = bindings
            .iter()
            .map(|(n, v)| Binding { name: n, value: v })
            .collect();
        frame_eval(&refs, expr)
    }
}
