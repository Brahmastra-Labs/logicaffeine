//! Cap'n Proto's entire pitch is "zero-copy, O(1) random access into a big message."
//! These lock-ins prove we match it on BOTH counts — and unlike the `superiority.rs`
//! capnp races (gated behind the `capnproto` toolchain), they assert OUR guarantees in
//! absolute terms, so they run in the DEFAULT suite and can never silently rot:
//!
//!   1. reading one field of one row allocates ZERO bytes on the heap (true zero-copy);
//!   2. that read costs the SAME in a 1,000,000-row message as in a 1,000-row one — it is
//!      O(1) in the row count (a linear scan would be ~1000× slower).
//!
//! The vehicle is the record-list view (`T_STRUCTS_VIEW`): a row-offset table + per-row
//! field-offset table, reached by `view.structs_row_field(row, field)`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hint::black_box;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use logicaffeine_compile::concurrency::marshal::{message_to_wire, view_message, with_struct_view};
use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue, StructValue};

// ── A counting allocator: armed only around the hot read path, so it measures the
//    read's allocations without perturbing setup (which allocates freely). nextest runs
//    each test in its own process, so the global counter never crosses test boundaries.

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// A `Record { id :Int, name :Text, active :Bool }` list of `n` rows — the `List(Record)`
/// shape Cap'n Proto is built for.
fn build_record_list(n: usize) -> RuntimeValue {
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let rows: Vec<RuntimeValue> = (0..n)
        .map(|i| {
            let mut f = HashMap::with_capacity(3);
            f.insert("id".to_string(), RuntimeValue::Int(i as i64 * 7 + 1));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new(names[i % names.len()].to_string())));
            f.insert("active".to_string(), RuntimeValue::Bool(i % 2 == 0));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Record".to_string(), fields: f }))
        })
        .collect();
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))))
}

/// Deterministic scatter of `count` indices over `0..n` (hashed, so reads jump around the
/// message — the worst case for any layout that isn't truly random-access).
fn scatter(count: usize, n: usize) -> Vec<usize> {
    (0..count).map(|k: usize| k.wrapping_mul(2_654_435_761) % n).collect()
}

#[test]
fn record_list_random_field_read_is_zero_alloc() {
    const N: usize = 100_000;
    let msg = with_struct_view(true, || message_to_wire("p", &build_record_list(N)).unwrap());
    let idxs = scatter(512, N);

    // Warm any one-time lazy init (OnceLock integrity mode, etc.) BEFORE arming, so we
    // measure only the steady-state read path.
    {
        let v = view_message(&msg).unwrap();
        black_box(v.structs_row_field(idxs[0], "id").unwrap().as_int().unwrap());
    }

    ARMED.store(true, Ordering::Relaxed);
    let before = ALLOC_COUNT.load(Ordering::Relaxed);
    // The realistic receiver: open the arrived message ONCE, then read scattered fields.
    let view = view_message(&msg).unwrap();
    let mut sink = 0i64;
    for &i in &idxs {
        sink = sink.wrapping_add(view.structs_row_field(i, "id").unwrap().as_int().unwrap());
    }
    let allocs = ALLOC_COUNT.load(Ordering::Relaxed) - before;
    ARMED.store(false, Ordering::Relaxed);
    black_box(sink);

    assert_eq!(
        allocs, 0,
        "open + {} scattered field reads must allocate 0× on the heap (true zero-copy, \
         capnp's signature claim); got {} allocations",
        idxs.len(),
        allocs
    );
}

/// Per-field-read latency (ns), opening the message once then reading every index, taking
/// the min over rounds to filter scheduler noise.
fn per_read_ns(msg: &[u8], idxs: &[usize]) -> f64 {
    let view = view_message(msg).unwrap();
    black_box(view.structs_row_field(idxs[0], "id").unwrap().as_int().unwrap()); // warmup
    let mut best = f64::INFINITY;
    for _ in 0..50 {
        let t = Instant::now();
        let mut sink = 0i64;
        for &i in idxs {
            sink = sink.wrapping_add(view.structs_row_field(i, "id").unwrap().as_int().unwrap());
        }
        black_box(sink);
        let ns = t.elapsed().as_nanos() as f64 / idxs.len() as f64;
        if ns < best {
            best = ns;
        }
    }
    best
}

#[test]
fn record_list_random_field_read_is_constant_time_in_row_count() {
    const SMALL: usize = 1_000;
    const BIG: usize = 1_000_000;

    let small_msg = with_struct_view(true, || message_to_wire("p", &build_record_list(SMALL)).unwrap());
    let big_msg = with_struct_view(true, || message_to_wire("p", &build_record_list(BIG)).unwrap());

    // Same NUMBER of reads against each, so the only variable is the message's row count.
    let reads = 2_000;
    let small_ns = per_read_ns(&small_msg, &scatter(reads, SMALL));
    let big_ns = per_read_ns(&big_msg, &scatter(reads, BIG));

    eprintln!(
        "record-list field read: 1K-row {small_ns:.1}ns vs 1M-row {big_ns:.1}ns ({:.2}×); \
         msg sizes {} vs {} B",
        big_ns / small_ns,
        small_msg.len(),
        big_msg.len()
    );

    // True random access is O(1) in the row count: a 1000× bigger message reads a field in
    // the same ballpark (the only honest slowdown is cache misses on the scattered offset
    // table, not algorithmic). A linear scan would be ~1000× slower; a generous 5×+15ns
    // margin cleanly separates O(1) from O(n) while absorbing memory-hierarchy effects.
    assert!(
        big_ns < small_ns * 5.0 + 15.0,
        "field read must be O(1) in row count, not O(n): 1K-row {small_ns:.1}ns vs \
         1M-row {big_ns:.1}ns ({:.1}× — a linear scan would be ~1000×)",
        big_ns / small_ns
    );
}

#[test]
fn record_list_random_field_read_is_correct() {
    // Correctness anchor for the timing/alloc locks: every sampled (row, field) read returns
    // exactly the encoded value, so a "fast" read can never be a fast WRONG read.
    const N: usize = 50_000;
    let msg = with_struct_view(true, || message_to_wire("p", &build_record_list(N)).unwrap());
    let view = view_message(&msg).unwrap();
    assert_eq!(view.structs_len().unwrap(), N);
    for &i in &scatter(1_000, N) {
        assert_eq!(view.structs_row_field(i, "id").unwrap().as_int().unwrap(), i as i64 * 7 + 1);
        assert_eq!(view.structs_row_field(i, "active").unwrap().as_bool().unwrap(), i % 2 == 0);
    }
}
