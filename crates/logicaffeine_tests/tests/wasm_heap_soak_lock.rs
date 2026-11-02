//! ★ LOCK — a reclaiming page runs for tens of thousands of turns at a constant cost (S11). ★
//!
//! With reclaim on, every counted value a turn drops (a list replaced, a filtered copy, a record's
//! list field copied on write) goes back to the allocator at the next safepoint, and the census —
//! every counted value's stored holder count against the in-degree recomputed from the page's own
//! globals — stays exact. Fully reclaimed workloads may have zero heap growth in both windows.
//!
//! So the lock is: the heap a turn costs late in a long run is the heap it cost early — no creeping
//! growth from a counted value leaked or kept alive — and the census is exact at every checkpoint.
//! The default run is 20,000 turns; the 1,000,000-turn run is `#[ignore]`d (run it on purpose).

#![cfg(feature = "wasm-jit")]

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::{mount_interactive_checked, InteractivePage};

/// A page whose state churns counted values every turn: a list of records grows by one and, past
/// eight, is replaced by a filtered copy holding the newest eight; a map of counts is rewritten.
const CHURN: &str = "\
## A Note has:
    a n, which is Int.

## Component App
State:
    Let notes be a new Seq of Note.
    Let counts be a new Map of Text to Int.
    Let tick be 0.
Actions:
    To add:
        Set tick to tick + 1.
        Push (a new Note with n tick) to notes.
        If length of notes is greater than 8:
            Let mutable kept be a new Seq of Note.
            Repeat for x in notes:
                If x's n is greater than tick - 8:
                    Push x to kept.
            Set notes to kept.
        Set item \"turns\" of counts to tick.
View:
    <div><button id=\"a\" on-click={add}>a</button><p>{length of notes}</p></div>

## Main
Mount App.
";

/// Runs `turns` clicks with a safepoint after each and a census every `every` turns, and answers
/// the gross heap after each of the two measuring windows `[w0, w1)` and `[w2, turns)`, as bytes
/// per turn.
fn rates(turns: usize, every: usize) -> (f64, f64) {
    let mut page: InteractivePage = mount_interactive_checked(CHURN);
    page.safepoint();
    let button = *page
        .listening("click")
        .last()
        .expect("the page listens for a click");
    let (w0, w1, w2) = (turns / 10, turns / 5, turns * 4 / 5);
    let (mut at_w0, mut at_w1, mut at_w2) = (0u32, 0u32, 0u32);
    for turn in 0..turns {
        match turn {
            t if t == w0 => at_w0 = page.heap_allocated(),
            t if t == w1 => at_w1 = page.heap_allocated(),
            t if t == w2 => at_w2 = page.heap_allocated(),
            _ => {}
        }
        page.dispatch(button, "click", "")
            .unwrap_or_else(|e| panic!("turn {turn} traps: {e}"));
        page.safepoint();
        if turn % every == 0 {
            assert_eq!(
                page.census(),
                0,
                "turn {turn}: every counted value's holder count is exact"
            );
        }
    }
    assert_eq!(page.census(), 0, "the census is exact after {turns} turns");
    let end = page.heap_allocated();
    let early = f64::from(at_w1 - at_w0) / (w1 - w0) as f64;
    let late = f64::from(end - at_w2) / (turns - w2) as f64;
    (early, late)
}

fn assert_constant(turns: usize, every: usize) {
    let (early, late) = rates(turns, every);
    assert!(
        late <= early * 1.10,
        "a turn late in a {turns}-turn run costs {late:.1} bytes against {early:.1} early — something the turns drop is kept"
    );
}

#[test]
fn twenty_thousand_turns_cost_what_the_first_thousands_did() {
    assert_constant(20_000, 1_000);
}

#[test]
#[ignore = "a million turns — run on purpose"]
fn a_million_turns_cost_what_the_first_thousands_did() {
    assert_constant(1_000_000, 50_000);
}
