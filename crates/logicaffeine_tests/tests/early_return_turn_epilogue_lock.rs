//! ★ LOCK — POSSIBLE DATA LOSS. An early `Return` inside a turn must still reach the turn's own
//! epilogue (state write-back, `noteTurn`) on every lowered tier, exactly as it does on the
//! reference tier. ★
//!
//! `noteTurn` and the state write-back both sit AFTER a turn's own body, in the SAME compiled
//! function (`turn_body_with`, the store-action loop, the mount-time `PageLoads`/`Appears`
//! handlers) — a `Return` lowered as a native function return used to leave that function
//! immediately, skipping BOTH. `To add: If draft is equal to "": Return. …` (`examples/todo`,
//! safety2's own finding) is the real shape: an early-exit GUARD clause, the single most common
//! reason an author writes `Return` at all. Before the fix: `Set n to n + 1. If n is greater than
//! 0: Return.` left `n` at its turn-START value on the lowered VM tier (never written back) and
//! never journaled the turn either — confirmed with `page_host::compare_tests` directly, unfiltered,
//! same technique `lgx_note_turn_lock.rs` uses for the journal axis specifically.

use logicaffeine_compile::page_host::{compare_tests, AxisDivergence, PageTierKind, TierRun};

const BUDGET: u64 = 50_000_000;

/// `source`'s one page `## Test`, run on the reference tier and the lowered VM, with every axis
/// the two disagree on.
fn divergent_axes(source: &str) -> Vec<AxisDivergence> {
    let all = compare_tests(source, &[PageTierKind::LoweredVm], 0, BUDGET, None)
        .expect("the page scenario parses");
    assert_eq!(all.len(), 1, "exactly one `## Test` that works a page");
    let t = match &all[0].candidates[0].1 {
        TierRun::Refused { reason, .. } => panic!("the lowered tier must not refuse: {reason}"),
        TierRun::Traced(t) => t,
    };
    all[0].reference.compare(t).axes
}

fn agrees(source: &str) {
    let axes = divergent_axes(source);
    assert!(
        axes.is_empty(),
        "the lowered VM tier must agree with the reference: {axes:?}"
    );
}

#[test]
fn an_early_return_after_a_set_still_commits_the_state_and_journals_the_turn() {
    agrees(
        "## Component App\n\
         State:\n    Let n be 0.\n\
         Actions:\n    To bump:\n        Set n to n + 1.\n        If n is greater than 0:\n            Return.\n        Set n to 999.\n\
         View:\n    <p id=\"n\">{n}</p><button id=\"b\" on-click={bump}>x</button>\n\n\
         ## Test: an early return still commits and journals\n\
         \x20   Mount App.\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"1\".\n",
    );
}

/// The `examples/todo` shape exactly: a GUARD clause at the very TOP of the action, before any
/// state write at all — the state must stay unchanged (not merely "not overwritten by later
/// code"), and the turn must still journal (the reference tier journals a turn that changed
/// nothing, same as one that changed everything).
#[test]
fn a_guard_clause_before_any_write_still_journals_and_leaves_state_untouched() {
    agrees(
        "## Component App\n\
         State:\n    Let draft be \"\".\n    Let items be a new Seq of Text.\n\
         Actions:\n    To add:\n        If draft is equal to \"\":\n            Return.\n        Add draft to items.\n        Set draft to \"\".\n\
         View:\n    <p id=\"n\">{length of items}</p><button id=\"b\" on-click={add}>x</button>\n\n\
         ## Test: an empty draft adds nothing but still journals\n\
         \x20   Mount App.\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"0\".\n",
    );
}

/// A `Return` nested inside the author's OWN `While` loop must break all the way out to the turn's
/// epilogue, not merely out of that one loop — the flag-and-recheck cascade `protect_early_return`
/// adds after every nested loop is what this exercises.
#[test]
fn a_return_nested_inside_the_authors_own_while_loop_still_reaches_the_epilogue() {
    agrees(
        "## Component App\n\
         State:\n    Let found be 0.\n    Let i be 0.\n\
         Actions:\n    To search:\n        While i is less than 10:\n            Set i to i + 1.\n            If i is equal to 3:\n                Set found to i.\n                Return.\n        Set found to 999.\n\
         View:\n    <p id=\"n\">{found}</p><button id=\"b\" on-click={search}>x</button>\n\n\
         ## Test: a nested return breaks out of both the loop and the turn\n\
         \x20   Mount App.\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"3\".\n",
    );
}

/// A `## Store`'s own action can guard-clause early too — the store-action lowering loop is a
/// SEPARATE site from `turn_body_with` (no shared helper between them beyond `protect_early_return`
/// itself), so this is its own, independent proof rather than an inference from the component case.
#[test]
fn a_store_actions_early_return_still_commits_and_persists() {
    agrees(
        "## Store Cart\n\
         State:\n    Let count be 0.\n\
         Actions:\n    To add (n: Int):\n        Set count to count + n.\n        If count is greater than 0:\n            Return.\n        Set count to -1.\n\n\
         ## Component App\nUse Cart.\n\
         Actions:\n    To bump:\n        Call Cart's add with 5.\n\
         View:\n    <p id=\"n\">{Cart's count}</p><button id=\"b\" on-click={bump}>x</button>\n\n\
         ## Test: a store action's early return still commits\n\
         \x20   Mount App.\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"5\".\n",
    );
}

/// A turn with NO `Return` anywhere must lower exactly as before — `protect_early_return`'s own
/// gate (`body_contains_return`) exists so a page that never uses `Return` pays nothing for a
/// feature it does not use (`bundle_size_lock` polices per-component overhead like this).
#[test]
fn a_turn_with_no_return_is_unaffected() {
    agrees(
        "## Component App\n\
         State:\n    Let n be 0.\n\
         Actions:\n    To bump:\n        Set n to n + 1.\n\
         View:\n    <p id=\"n\">{n}</p><button id=\"b\" on-click={bump}>x</button>\n\n\
         ## Test: an ordinary turn with no return\n\
         \x20   Mount App.\n\
         \x20   Click \"#b\".\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"2\".\n",
    );
}
