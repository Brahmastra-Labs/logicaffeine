//! Background AOT-native compilation (HOTSWAP §Axis-3 / P18).
//!
//! A very-hot function's OPTIMIZED form is compiled to a cdylib and loaded on a WORKER
//! thread — `rustc` is seconds, so it must never block the interpreter — then the loaded
//! function is installed via `Vm::install_aot_native` at a drain point and runs at
//! compiled-binary speed from then on. This is the AOT path to the coarse-tiered
//! "optimize hot functions during the run" goal: it reuses the proven on-demand build
//! ([`crate::compile::aot_build_function`]) off-thread, the same mpsc worker shape as
//! the forge background compiler (§6), and the existing `NativeSlot::Ready` dispatch —
//! no new VM seam. Because an AOT function is all-or-nothing per call (it bails to the
//! lower tier on a signature mismatch rather than precise-deopting), it sidesteps the
//! warm-bytecode run-loop change the forge coarse path would need.
//!
//! Native-only: the browser uses pre-bundled wasm, and there is no `rustc` there.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use super::native_tier::NativeFn;

/// A request to background-compile function `fi` (named `fn_name`) from `source`,
/// caching the artifact under `cache_dir`. All fields owned so the request crosses to
/// the worker thread.
pub struct AotRequest {
    pub fi: usize,
    pub source: String,
    pub fn_name: String,
    pub cache_dir: PathBuf,
}

/// The worker's reply: the function index and the loaded AOT function (or `None` if the
/// function was outside the scalar subset or the build/load failed — the interpreter
/// then keeps it on VM+JIT).
pub struct AotResult {
    pub fi: usize,
    pub nf: Option<Box<dyn NativeFn>>,
}

/// The interpreter-side handle to the background AOT compiler. `submit` is
/// non-blocking; `try_drain` is polled at profiling points and `drain_blocking` is the
/// determinism hook tests use. The worker detaches on drop.
pub struct BgAotCompiler {
    req_tx: Sender<AotRequest>,
    res_rx: Receiver<AotResult>,
    inflight: usize,
    _worker: JoinHandle<()>,
}

impl BgAotCompiler {
    /// Spawn the AOT build worker.
    pub fn new() -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<AotRequest>();
        let (res_tx, res_rx) = std::sync::mpsc::channel::<AotResult>();
        let worker = std::thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                let nf = crate::compile::aot_build_function(&req.source, &req.fn_name, &req.cache_dir);
                if res_tx.send(AotResult { fi: req.fi, nf }).is_err() {
                    break; // interpreter gone — stop quietly
                }
            }
        });
        BgAotCompiler { req_tx, res_rx, inflight: 0, _worker: worker }
    }

    /// Queue an AOT build. Non-blocking; the interpreter keeps running on the lower
    /// tier (bytecode / forge) while `rustc` works.
    pub fn submit(&mut self, req: AotRequest) {
        if self.req_tx.send(req).is_ok() {
            self.inflight += 1;
        }
    }

    /// Take one finished AOT build if ready — polled at the profiling points.
    pub fn try_drain(&mut self) -> Option<AotResult> {
        match self.res_rx.try_recv() {
            Ok(r) => {
                self.inflight = self.inflight.saturating_sub(1);
                Some(r)
            }
            Err(_) => None,
        }
    }

    /// Whether any submitted build is still outstanding.
    pub fn is_idle(&self) -> bool {
        self.inflight == 0
    }

    /// Block until every outstanding build has come back — the determinism hook for
    /// the differential gates (a single AOT build is seconds, so tests opt in).
    pub fn drain_blocking(&mut self) -> Vec<AotResult> {
        let mut out = Vec::with_capacity(self.inflight);
        while self.inflight > 0 {
            match self.res_rx.recv() {
                Ok(r) => {
                    self.inflight -= 1;
                    out.push(r);
                }
                Err(_) => break,
            }
        }
        out
    }
}

impl Default for BgAotCompiler {
    fn default() -> Self {
        Self::new()
    }
}
