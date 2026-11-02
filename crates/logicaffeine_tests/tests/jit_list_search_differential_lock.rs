//! The forge (copy-and-patch) JIT must compute list-element reads in hot loops byte-identically to
//! the tree-walker — a differential lock over the exact pattern that exposed a JIT soundness bug:
//! a hot function that reads `item i of xs` (a list parameter) inside a loop with an EARLY `Return`.
//!
//! The bug: once such a function tiered up to native, the in-loop reads came back stale, so a search
//! never matched and fell through to its default. Caught here by running a search fuzz under the forge
//! tier and asserting it equals the tree-walker (which is the trusted reference). The fix lives in
//! `logicaffeine_jit` (the JIT declines the miscompiling pattern to the correct VM bytecode).

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.trim().to_string()
}

/// Run `src` under the forge JIT and assert it matches the tree-walker reference exactly.
fn forge_eq_tw(src: &str) {
    let tw = tw_outcome_with_args(src, &[]);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    assert!(tw.error.is_none(), "tree-walker errored: {:?}", tw.error);
    assert!(
        forge.error.is_none(),
        "forge JIT errored: {:?}",
        forge.error
    );
    assert_eq!(
        norm(&forge.output),
        norm(&tw.output),
        "forge JIT diverged from the tree-walker on a hot list-element read:\n{src}"
    );
}

/// `firstGE` — a list parameter read (`item i of xs`) in a loop with an early `Return`, called until
/// it tiers up. Every call must return the first index ≥ the threshold (7), on the forge and the tw.
#[test]
fn forge_matches_tw_on_early_return_list_search() {
    forge_eq_tw(
        "## To firstGE (xs: Seq of Int, thresh: Int) -> Int:\n\
        \x20   Let n be length of xs.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most n:\n\
        \x20       If (item i of xs) is at least thresh:\n\
        \x20           Return i.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return 0.\n\
        ## Main\n\
        Let xs be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].\n\
        Let mutable k be 0.\n\
        Let mutable bad be 0.\n\
        While k is less than 400:\n\
        \x20   If firstGE(xs, 7) is equal to 7:\n\
        \x20       Set bad to bad.\n\
        \x20   Otherwise:\n\
        \x20       Set bad to bad + 1.\n\
        \x20   Set k to k + 1.\n\
        Show bad.\n",
    );
}

/// `matchAt` in a hot scan — a list read in a loop with an early `Return false`, the stdlib substring
/// primitive. The forge must agree with the tree-walker that the substring occurs at its true index.
#[test]
fn forge_matches_tw_on_hot_substring_scan() {
    forge_eq_tw(
        "## To matchAt2 (sb: Seq of Int, pb: Seq of Int, at: Int) -> Bool:\n\
        \x20   Let np be length of pb.\n\
        \x20   Let mutable j be 1.\n\
        \x20   While j is at most np:\n\
        \x20       If (item (at + j - 1) of sb) is equal to (item j of pb):\n\
        \x20           Set j to j + 1.\n\
        \x20       Otherwise:\n\
        \x20           Return false.\n\
        \x20   Return true.\n\
        ## Main\n\
        Let sb be [66, 67, 67, 65, 67, 65, 65, 66, 66, 66, 66, 67, 67, 66, 66, 65].\n\
        Let pb be [67, 67, 66].\n\
        Let mutable k be 0.\n\
        Let mutable bad be 0.\n\
        While k is less than 400:\n\
        \x20   If matchAt2(sb, pb, 12) is equal to true:\n\
        \x20       Set bad to bad.\n\
        \x20   Otherwise:\n\
        \x20       Set bad to bad + 1.\n\
        \x20   Set k to k + 1.\n\
        Show bad.\n",
    );
}
