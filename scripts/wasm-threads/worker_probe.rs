// WS5 (work/FINISH_INTERPRETER.md Phase 12) — the true-multicore WebAssembly threads primitive.
//
// NOT a crate — a single source file compiled by `scripts/test-wasm-threads.sh` with a raw
// `rustc --target wasm32-unknown-unknown --crate-type=cdylib` and `+atomics` + shared,
// imported memory (the atomic ops inline here, so stock `core` is fine — no `build-std`).
// It's the irreducible proof that genuine browser multicore is real and verifiable headlessly:
// the exact build a browser Web-Worker pool needs, driven by node `worker_threads` (real OS
// threads — the headless analog of Web Workers) over one shared `WebAssembly.Memory`.
//
// Correctness is double-checked by the driver (`run.mjs`):
// - Atomicity — every worker increments a shared counter `iters` times under contention; the
//   total must equal exactly `num_workers * iters` (a lost update ⇒ broken atomics / unshared
//   memory).
// - True concurrency — a sense-reversing barrier the last arriver releases. If the workers ran
//   serially, the first to arrive would spin forever; the bounded spin then returns a failure
//   sentinel and the test fails loudly. Clearing it is only possible if N threads run at once.
//
// Uses no thread-locals, no static data, and only leaf arithmetic + atomic RMW on fixed
// addresses, so it needs no per-thread TLS/stack bootstrap — each worker just instantiates and
// calls. The cells sit at 1 MiB, above the module's data/stack (shared memory is zero-init'd by
// the host, so they start at 0).

#![no_std]
#![allow(internal_features)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

const COUNTER: usize = 0x10_0000;
const ARRIVED: usize = 0x10_0004;
const GENERATION: usize = 0x10_0008;

/// A typed atomic view of a fixed shared-memory cell. Sound: every address is a 4-byte aligned
/// slot inside the host-supplied shared linear memory, used only through atomics.
#[inline(always)]
fn cell(addr: usize) -> &'static AtomicU32 {
    unsafe { &*(addr as *const AtomicU32) }
}

/// A worker thread's entry point. Increments the shared counter `iters` times (contended), then
/// crosses a sense-reversing barrier. Returns 0 on success, 1 if the barrier spun past its
/// bound — i.e. the workers did NOT run concurrently (no true multicore).
#[no_mangle]
pub extern "C" fn worker_main(num_workers: u32, iters: u32) -> u32 {
    let counter = cell(COUNTER);
    let mut i = 0;
    while i < iters {
        counter.fetch_add(1, Ordering::SeqCst);
        i += 1;
    }

    let arrived = cell(ARRIVED);
    let generation = cell(GENERATION);
    let gen = generation.load(Ordering::SeqCst);
    let n = arrived.fetch_add(1, Ordering::SeqCst) + 1;
    if n == num_workers {
        arrived.store(0, Ordering::SeqCst);
        generation.fetch_add(1, Ordering::SeqCst);
        0
    } else {
        let mut spins: u64 = 0;
        while generation.load(Ordering::SeqCst) == gen {
            core::hint::spin_loop();
            spins += 1;
            if spins > 5_000_000_000 {
                return 1;
            }
        }
        0
    }
}

/// Read the shared counter — the main thread's check that every increment landed.
#[no_mangle]
pub extern "C" fn get_counter() -> u32 {
    cell(COUNTER).load(Ordering::SeqCst)
}

/// Re-zero the coordination cells so one shared memory can drive several rounds.
#[no_mangle]
pub extern "C" fn reset() {
    cell(COUNTER).store(0, Ordering::SeqCst);
    cell(ARRIVED).store(0, Ordering::SeqCst);
    cell(GENERATION).store(0, Ordering::SeqCst);
}
