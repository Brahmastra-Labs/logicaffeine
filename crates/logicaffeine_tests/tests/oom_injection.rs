//! oom_injection.rs — Invariant X (Graceful under adversity), memory axis.
//!
//! A hostile wire frame can *claim* an astronomical
//! element count or expansion factor; a decoder that trusts the claim and preallocates is a
//! one-packet remote OOM (we already found and fixed two: a 476 GB `T_SET_STRINGS` prealloc and
//! a 4.26 GB dictionary decompression bomb). This lock proves those doors stay bounded by
//! running EVERY wire decoder under a genuinely memory-constrained global allocator.
//!
//! The mechanism is real fault injection, not a mock: a `#[global_allocator]` caps any single
//! allocation at [`LIMIT_BYTES`]. An over-cap request is recorded (its size is visible to the
//! assert) and then denied by returning null — so
//!   * a decoder using INFALLIBLE allocation (`Vec::with_capacity`) hits `handle_alloc_error` →
//!     process abort → the runner reports the crashed test, and
//!   * a decoder using FALLIBLE allocation (`try_reserve`) gets the error, degrades to a clean
//!     `None`, and the recorded peak trips the assert here —
//! either way a regressed, unbounded decoder cannot pass. Denying the allocation (rather than
//! letting it through) is also what keeps the bomb corpus from actually exhausting the host.
//!
//! Today, admission control (`MAX_DECODE_BYTES`) means no decoder ever *requests* an over-cap
//! block, so the peak stays far under the cap and the suite is green.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use logicaffeine_compile::concurrency::marshal::{
    decode_value_raw, message_from_wire, parse_handshake_frame,
};

// ───────────────────────────── constrained global allocator ─────────────────────────────

struct GuardAlloc;

/// When false the guard is a transparent pass-through to the system allocator (zero behavior
/// change for every other test in the binary); flipped on only inside [`under_limit`].
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Any single allocation strictly larger than this (while `ACTIVE`) is recorded and denied.
static LIMIT: AtomicUsize = AtomicUsize::new(usize::MAX);
/// The largest single allocation *requested* during the active window — recorded even when the
/// request is over the cap and denied, so fallible-allocation paths still surface the regression.
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for GuardAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ACTIVE.load(Ordering::Relaxed) {
            let sz = layout.size();
            PEAK.fetch_max(sz, Ordering::Relaxed);
            if sz > LIMIT.load(Ordering::Relaxed) {
                return std::ptr::null_mut();
            }
        }
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Growth (e.g. a `push`-in-a-loop bomb) flows through here, not `alloc` — guard it too.
        if ACTIVE.load(Ordering::Relaxed) {
            PEAK.fetch_max(new_size, Ordering::Relaxed);
            if new_size > LIMIT.load(Ordering::Relaxed) {
                return std::ptr::null_mut();
            }
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: GuardAlloc = GuardAlloc;

/// Serializes the constrained window. `cargo test` runs tests as threads in ONE process (shared
/// `ACTIVE`); `cargo nextest` gives per-test process isolation. The mutex is correct under both —
/// and because the cap only bites allocations over half a gigabyte, unrelated tests running
/// concurrently are never affected.
static GATE: Mutex<()> = Mutex::new(());

/// A memory-constrained host: no single allocation may exceed 512 MiB. Chosen well above any
/// legitimate decode working set and far below the multi-gigabyte bombs.
const LIMIT_BYTES: usize = 512 * 1024 * 1024;

/// Runs `f` with the allocator capped at [`LIMIT_BYTES`]; returns `f`'s value and the peak single
/// allocation it requested.
fn under_limit<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    PEAK.store(0, Ordering::Relaxed);
    LIMIT.store(LIMIT_BYTES, Ordering::Relaxed);
    ACTIVE.store(true, Ordering::Relaxed);
    let r = f();
    ACTIVE.store(false, Ordering::Relaxed);
    LIMIT.store(usize::MAX, Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);
    (r, peak)
}

// ───────────────────────────── adversarial "huge claim" corpus ─────────────────────────────

/// The 10-byte LEB128 encoding of the largest value a varint reader can yield — the canonical
/// "claim a preposterous length/count" payload.
fn max_varint_10() -> [u8; 10] {
    [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]
}

/// Frames engineered to trip an unbounded preallocation, if one exists on any decoder path.
fn oom_corpus() -> Vec<Vec<u8>> {
    let mut c: Vec<Vec<u8>> = Vec::new();

    // The proven dictionary DECOMPRESSION BOMB: 6 bytes that pre-fix expanded to 4.26 GB.
    c.push(vec![0x06, 0xff, 0xfd, 0xff, 0xfd, 0x84]);

    // Exhaustive single-level sweep: every possible leading tag byte, followed by a max varint
    // (the "count"/"length"), then a little trailing data. Whatever a given tag means to a
    // decoder, if it reads a count here and preallocates, this frame demands ~2^64 units.
    for tag in 0u16..=255 {
        let mut f = vec![tag as u8];
        f.extend_from_slice(&max_varint_10());
        f.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        c.push(f);
    }

    // Two-level sweep: a huge outer count whose elements each also claim a huge count — catches a
    // decoder that bounds the outer allocation but recurses into an unbounded inner one.
    for tag in 0u16..=255 {
        let mut f = vec![tag as u8];
        f.extend_from_slice(&max_varint_10());
        f.push(tag as u8);
        f.extend_from_slice(&max_varint_10());
        c.push(f);
    }

    c
}

// ───────────────────────────── the locks ─────────────────────────────

/// No adversarial frame may push any wire decoder past the memory cap. A regression to an
/// unbounded prealloc is caught as either a crashed test (infallible alloc) or a failed assert
/// (fallible alloc) — never a silent pass.
#[test]
fn wire_decoders_stay_within_memory_budget_under_adversarial_frames() {
    for frame in oom_corpus() {
        let (_, peak) = under_limit(|| {
            let _ = message_from_wire(&frame);
            let _ = decode_value_raw(&frame);
            let _ = parse_handshake_frame(&frame);
        });
        assert!(
            peak <= LIMIT_BYTES,
            "a wire decoder requested a {peak}-byte allocation (cap {LIMIT_BYTES}) on frame \
             {frame:02x?} — admission control regressed; this frame is a remote-OOM vector",
        );
    }
}

/// Positive control: the guard is not so tight it strangles legitimate work. A 64 MiB allocation
/// (well under the 512 MiB cap) must succeed, and the peak must be recorded — proving the injected
/// allocator is actually in the path and the cap is calibrated above real working sets, so the
/// lock above cannot pass merely because nothing ever allocates.
#[test]
fn allocator_guard_permits_sub_cap_allocation() {
    let (len, peak) = under_limit(|| {
        let v: Vec<u64> = vec![0u64; 8 * 1024 * 1024]; // 64 MiB
        v.len()
    });
    assert_eq!(len, 8 * 1024 * 1024);
    assert!(
        peak >= 64 * 1024 * 1024 && peak <= LIMIT_BYTES,
        "expected a ~64 MiB allocation to be observed and permitted, saw peak={peak}",
    );
}
