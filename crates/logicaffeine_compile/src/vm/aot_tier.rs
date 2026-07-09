//! AOT-native tier loader (HOTSWAP §Axis-3 / P15): `dlopen` a rustc-built cdylib
//! (produced by [`crate::compile::build_native_cdylib`]) and wrap an exported
//! `logos_native_<fn>` symbol as a [`NativeFn`] the VM dispatches through the existing
//! `NativeSlot::Ready` path — no new dispatch, the hot-swap seam already exists.
//!
//! Sound subset: ALL-INT signatures (every param + the return `Int`). Those cross by
//! value in general-purpose registers, so a single `i64` calling convention is exact.
//! Float/Bool need per-type fn-pointer ABIs (f64 rides XMM) and are deferred — a
//! function outside the subset simply gets no AOT fn and stays on VM+JIT (no gap).
//!
//! The interpreter is the only caller of the loaded symbol, and ownership of the
//! `Library` is held for the program's lifetime (an `Arc` inside the `NativeFn`), so
//! the code stays mapped while any call can occur.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::native_tier::{NativeFn, NativeOutcome, NativeRet, SlotKind};

/// One AOT-native function: the resolved symbol address, its arity, the `Library` it
/// lives in (kept alive), and a call counter for observability/tests. `Send + Sync`
/// because the address is immutable and the code is read-only once loaded.
pub struct AotNativeFn {
    addr: usize,
    arity: usize,
    _lib: Arc<libloading::Library>,
    calls: Arc<AtomicU64>,
}

// SAFETY: `addr` is an immutable code address in a mapped, read-only library kept
// alive by `_lib`; calling it is the same on any thread (the interpreter is the sole
// caller). `calls` is atomic.
unsafe impl Send for AotNativeFn {}
unsafe impl Sync for AotNativeFn {}

impl AotNativeFn {
    /// Number of times this AOT function has been invoked (observability/tests).
    pub fn call_count(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

impl NativeFn for AotNativeFn {
    fn call(&self, args: &[i64], _pins: &[i64], _depth: usize) -> NativeOutcome {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let a = self.addr;
        // SAFETY: `a` is the address of a `extern "C" fn(i64..) -> i64` (the
        // `logos_native_<fn>` shim) for an all-Int signature of this arity; the VM's
        // entry guard already proved every `args[k]` is an `Int`.
        let r = unsafe {
            match self.arity {
                0 => std::mem::transmute::<usize, extern "C" fn() -> i64>(a)(),
                1 => std::mem::transmute::<usize, extern "C" fn(i64) -> i64>(a)(args[0]),
                2 => std::mem::transmute::<usize, extern "C" fn(i64, i64) -> i64>(a)(args[0], args[1]),
                3 => std::mem::transmute::<usize, extern "C" fn(i64, i64, i64) -> i64>(a)(
                    args[0], args[1], args[2],
                ),
                4 => std::mem::transmute::<usize, extern "C" fn(i64, i64, i64, i64) -> i64>(a)(
                    args[0], args[1], args[2], args[3],
                ),
                // Higher arities aren't loaded yet — bail to the bytecode path.
                _ => return NativeOutcome::Deopt,
            }
        };
        NativeOutcome::Return(r)
    }

    fn ret(&self) -> NativeRet {
        NativeRet::Scalar(SlotKind::Int)
    }

    fn entry_ptr(&self) -> i64 {
        // AOT functions are reached via the Rust-level `NativeSlot::Ready` dispatch,
        // NOT the FnTable stencil fast-path (whose raw-pointer ABI differs). They are
        // never published to the FnTable, so this is unused.
        0
    }

    fn published_regc(&self) -> i64 {
        0
    }
}

/// `dlopen` `path` and resolve the AOT-native `symbol` (`logos_native_<fn>`) as an
/// all-Int function of `arity`, wrapping it as a [`NativeFn`] plus its shared call
/// counter. Returns `None` if the library or symbol cannot be loaded (the caller then
/// keeps the function on VM+JIT — no gap at the seam).
pub fn load_aot_native(
    path: &std::path::Path,
    symbol: &str,
    arity: usize,
) -> Option<(Box<dyn NativeFn>, Arc<AtomicU64>)> {
    // SAFETY: loading an arbitrary library runs its initializers; the caller supplies a
    // path it just built (or a cache entry validated by toolchain hash).
    let lib = unsafe { libloading::Library::new(path).ok()? };
    // Probe the symbol exists (typed as a fn so resolution is a function lookup).
    let addr = unsafe {
        let sym: libloading::Symbol<'_, extern "C" fn() -> i64> = lib.get(symbol.as_bytes()).ok()?;
        (*sym) as usize
    };
    let calls = Arc::new(AtomicU64::new(0));
    let nf = AotNativeFn {
        addr,
        arity,
        _lib: Arc::new(lib),
        calls: calls.clone(),
    };
    Some((Box::new(nf), calls))
}
