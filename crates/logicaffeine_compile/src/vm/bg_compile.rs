//! Off-thread native compilation (HOTSWAP §6): the interpreter profiles calls and,
//! when a function crosses its tier threshold, ships an OWNED [`CompileRequest`] to a
//! worker thread instead of stalling on `tier.compile_function`. The worker holds an
//! `Arc<dyn NativeTier>` (the installed forge backend, or a test-provided one),
//! compiles, and returns a [`CompileResult`]; the INTERPRETER drains the result at its
//! existing profiling points and remains the sole `FnTable` writer (it calls
//! `publish`). The mpsc send/recv is the acquire/release edge that makes the
//! worker-sealed `JitPage` bytes visible on the interpreter thread.
//!
//! Why an `Arc<dyn NativeTier>`, not the `&'p` the `Vm` holds: only a `Send + 'static`
//! tier can cross to the worker. The live engine installs forge as a process-wide
//! tier; a test passes `Arc::new(ForgeTier::new())`. A `Vm` whose tier is only a
//! borrowed `&'p` (no shareable handle) keeps compiling synchronously — the retained
//! fallback. Execution stays thread-pinned (compiled chains use the thread-local
//! arena), so the worker only ever COMPILES; it never runs a chain.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use super::instruction::{Constant, Op};
use super::native_tier::{CalleeSig, NativeCtx, NativeFn, NativeTier, ParamKind, SlotKind};

/// A function to compile, fully owned so it can cross to the worker thread. Every
/// field is cloned from the immutable `CompiledProgram` at submit time (the `Vm`, its
/// registers, and the `&'p` program never cross).
pub(crate) struct FunctionRequest {
    pub fi: usize,
    pub code: Vec<Op>,
    pub entry_pc: usize,
    pub constants: Arc<[Constant]>,
    pub param_count: u16,
    pub register_count: u16,
    pub param_kinds: Vec<Option<ParamKind>>,
    pub ret_kind: Option<SlotKind>,
    pub callees: Vec<CalleeSig>,
    pub ctx: NativeCtx,
}

/// A unit to compile off-thread. (Regions follow the identical pattern via
/// `compile_region`; functions are the core and land first.)
pub(crate) enum CompileRequest {
    Function(FunctionRequest),
}

/// The worker's reply: the compiled chain (or `None` if the backend bailed), tagged
/// with the unit it belongs to so the interpreter can publish it into the right slot.
pub(crate) enum CompileResult {
    Function {
        fi: usize,
        nf: Option<Box<dyn NativeFn>>,
    },
}

/// The interpreter-side handle to the compile worker. `submit` is non-blocking;
/// `try_drain` is polled at the profiling points; `drain_blocking` is the
/// determinism hook tests use (`Vm::drain_pending_compiles`). On drop the request
/// sender closes, the worker's `recv` errors, and it exits — the worker is detached,
/// never `join`ed on the hot path (joining would reintroduce the stall).
pub(crate) struct BgCompiler {
    req_tx: Sender<CompileRequest>,
    res_rx: Receiver<CompileResult>,
    /// Requests submitted but not yet drained — lets `drain_blocking` know how many
    /// replies to wait for, and `is_idle` answer without blocking.
    inflight: usize,
    _worker: JoinHandle<()>,
}

impl BgCompiler {
    /// Spawn the worker bound to `tier` — the process-installed backend. A
    /// `&'static dyn NativeTier` is `Send` (because `NativeTier: Sync`) and `'static`,
    /// so it moves into the worker thread directly; no `Arc` needed.
    pub(crate) fn new(tier: &'static dyn NativeTier) -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<CompileRequest>();
        let (res_tx, res_rx) = std::sync::mpsc::channel::<CompileResult>();
        let worker = std::thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                let res = match req {
                    CompileRequest::Function(f) => {
                        let nf = tier.compile_function(
                            &f.code,
                            f.entry_pc,
                            &f.constants,
                            f.param_count,
                            f.register_count,
                            f.fi as u16,
                            &f.param_kinds,
                            f.ret_kind,
                            &f.ctx,
                            &f.callees,
                        );
                        CompileResult::Function { fi: f.fi, nf }
                    }
                };
                // The interpreter dropped its receiver (run ended): stop quietly. The
                // unfinished compile is simply never applied — the lower tier already
                // produced the correct answer.
                if res_tx.send(res).is_err() {
                    break;
                }
            }
        });
        BgCompiler { req_tx, res_rx, inflight: 0, _worker: worker }
    }

    /// Queue a unit for compilation. Non-blocking; the interpreter keeps running the
    /// current (lower) tier while the worker compiles.
    pub(crate) fn submit(&mut self, req: CompileRequest) {
        if self.req_tx.send(req).is_ok() {
            self.inflight += 1;
        }
    }

    /// Take one finished result if the worker has produced one — polled at the
    /// interpreter's profiling points. Never blocks.
    pub(crate) fn try_drain(&mut self) -> Option<CompileResult> {
        match self.res_rx.try_recv() {
            Ok(r) => {
                self.inflight = self.inflight.saturating_sub(1);
                Some(r)
            }
            Err(_) => None,
        }
    }

    /// Whether any submitted compile is still outstanding.
    pub(crate) fn is_idle(&self) -> bool {
        self.inflight == 0
    }

    /// Block until every outstanding compile has come back, returning them all — the
    /// synchronous-for-tests hook behind `Vm::drain_pending_compiles`, which makes the
    /// per-tier differential gates deterministic regardless of scheduling.
    pub(crate) fn drain_blocking(&mut self) -> Vec<CompileResult> {
        let mut out = Vec::with_capacity(self.inflight);
        while self.inflight > 0 {
            match self.res_rx.recv() {
                Ok(r) => {
                    self.inflight -= 1;
                    out.push(r);
                }
                Err(_) => break, // worker died; nothing more will arrive
            }
        }
        out
    }
}
