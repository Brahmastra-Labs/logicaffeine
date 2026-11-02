//! ★ LOCK — a page's heap is bounded by the HOST, not by a fixed slab. ★
//!
//! `__logos_alloc`'s bump path used to advance `__heap_ptr` unconditionally, trusting the module's
//! declared minimum (`HEAP_PAGES` = 64 pages, 4 MiB) to cover every page it would ever run: anything
//! past that was an out-of-bounds trap with no message at all. It now checks the module's actual
//! `memory.size` before bumping and calls `memory.grow` — geometrically, so a long run amortizes to
//! O(1) grow calls rather than one per allocation past the boundary — before ever landing past what
//! is mapped. A host that refuses to grow further (a real embedder's own limit, modelled here with
//! `wasmi::StoreLimits`) is reported as `FailCode::OutOfMemory`, not a bare trap.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome_capped;

/// Build a `Seq of Int` `n` elements long inside the module — `n * 8` bytes just for its final
/// buffer, so a large enough `n` needs more than the module's initial heap without any turn/host
/// interaction at all.
fn growing_list(n: usize) -> String {
    format!(
        "## To fill (n: Int) -> Seq of Int:\n\
         \x20   Let mutable xs be a new Seq of Int.\n\
         \x20   Let mutable i be 0.\n\
         \x20   While i is less than n:\n\
         \x20       Push i to xs.\n\
         \x20       Set i to i + 1.\n\
         \x20   Return xs.\n\
         \n\
         ## Main\n\
         Let big be fill({n}).\n\
         Show length of big.\n"
    )
}

/// One wasm page is 64 KiB; the module's declared minimum is 64 pages (4 MiB) — see `HEAP_PAGES` in
/// `vm/wasm/module.rs`. Building a list well past that needs the allocator to grow memory that was
/// never declared, not merely bump within it.
const PAST_INITIAL_HEAP: usize = 700_000; // 700_000 * 8 B ≈ 5.34 MiB > 4 MiB

#[test]
fn a_turn_needing_more_than_the_first_heap_grows_it() {
    let src = growing_list(PAST_INITIAL_HEAP);
    let run = run_outcome_capped(&src, None).unwrap_or_else(|e| panic!("must run: {e}\n{src}"));
    assert_eq!(
        run.failure, None,
        "a page with room to grow into must not fail"
    );
    assert_eq!(
        run.output,
        PAST_INITIAL_HEAP.to_string(),
        "the list must still hold every element"
    );
    let heap = run
        .heap_ptr
        .expect("a heap-using module exports __heap_ptr");
    const INITIAL_HEAP_BYTES: u32 = 64 * 65536;
    assert!(
        heap > INITIAL_HEAP_BYTES,
        "the module's heap pointer ({heap}) never grew past the module's declared minimum \
         ({INITIAL_HEAP_BYTES}) — a program needing more than the first heap must grow it, not \
         silently stay within a boundary it should have outgrown"
    );
}

#[test]
fn a_page_that_cannot_grow_says_out_of_memory() {
    let src = growing_list(PAST_INITIAL_HEAP);
    // Capped at EXACTLY the module's own initial declaration: room for the first heap and no more,
    // so the very first `memory.grow` this run needs is refused by the host, not by chance.
    let run = run_outcome_capped(&src, Some(64)).unwrap_or_else(|e| panic!("must run: {e}\n{src}"));
    assert_eq!(
        run.failure.as_deref(),
        Some("Out of memory"),
        "a page whose host refuses to grow memory further must fail with the allocator's own \
         FailCode::OutOfMemory sentence, not a bare unworded trap"
    );
}

#[test]
fn a_page_with_no_headroom_at_all_still_runs_small_programs() {
    // Capped at the initial size: nothing this program allocates needs to grow past it, so the cap
    // must be transparent to a page that never actually needs the room it forecloses.
    let run = run_outcome_capped("## Main\nShow \"hi\".\n", Some(64))
        .unwrap_or_else(|e| panic!("must run: {e}"));
    assert_eq!(run.failure, None);
    assert_eq!(run.output, "hi");
}
