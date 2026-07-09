//! Region tiering for `NewEmptyList` allocated INSIDE a hot loop body — the
//! fannkuch / knapsack shape. The outer loop allocates a fresh `Seq of Int`
//! each iteration (`Let mutable x be a new Seq of Int`) and then fills it. The
//! VM already does sound buffer reuse for `NewEmptyList` (clear-in-place when
//! the destination holds a sole-owned `Ints` list); the JIT region translator
//! must mirror that semantics rather than bailing the whole tier to bytecode.
//!
//! SOUNDNESS: clear-reuse is valid ONLY when the destination's previous list is
//! dead (sole-owned). When the list handle is copied out of the loop body
//! (`Set prev to curr` — an aliasing `Move`), reusing the buffer would corrupt
//! the alias, so the region must NOT clear-reuse it. Every test here pins VM
//! output bit-identically to the tree-walker; the differential is the spec.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run through the tiered VM + the tree-walker, assert they agree, and return
/// `(normalized output, error, region_attempts, region_successes)`. A region
/// that BAILS (e.g. the `NewEmptyList`-in-loop tier failure) increments
/// `attempts` but not `successes`, so `attempts > successes` is the static
/// signature of a region that refused to tier.
fn tiered(src: &str) -> (String, Option<String>, u32, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on:\n{src}"
    );
    let (attempts, successes) = tier.region_counts();
    (norm(&vm.output), vm.error, attempts, successes)
}

/// fannkuch's outer permutation loop in miniature: each outer iteration
/// allocates a FRESH `perm` (`Let mutable perm be a new Seq of Int`), copies
/// `perm1` into it, then runs the swap/flip loop over `perm`. `perm` is
/// sole-owned (never moved out), so the region may clear-reuse the buffer — and
/// it MUST tier as a region, not run 100% on bytecode.
///
/// CRITICAL register-reuse shape: the inner `While r is greater than 1` loop
/// runs `Set r to r - 1` — a `Sub` whose result the compiler stages through a
/// scratch slot which it then REUSES for `perm`. So the `perm` slot appears as a
/// `Move` source (the staged `Sub` result) ONE OP BEFORE the `NewEmptyList`
/// re-binds it to the list. The over-coarse alias check (any `Move { src: dst }`
/// anywhere) treats that scalar move as a list alias and bails the whole tier;
/// the kind-flow refinement sees the slot holds an Int there and clears-in-place
/// soundly. This is the exact construct that made real fannkuch run 100% on
/// bytecode.
#[test]
fn fannkuch_outer_alloc_loop_tiers_and_matches() {
    let src = "## Main\n\
               Let n be 8.\n\
               Let mutable perm1 be a new Seq of Int.\n\
               Let mutable count be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push i to perm1.\n\
               \x20   Push 0 to count.\n\
               \x20   Set i to i + 1.\n\
               Let mutable checksum be 0.\n\
               Let mutable r be n.\n\
               Let mutable pass be 0.\n\
               While pass is less than 4000:\n\
               \x20   While r is greater than 1:\n\
               \x20       Set item r of count to r.\n\
               \x20       Set r to r - 1.\n\
               \x20   Let mutable perm be a new Seq of Int.\n\
               \x20   Set i to 1.\n\
               \x20   While i is at most n:\n\
               \x20       Push item i of perm1 to perm.\n\
               \x20       Set i to i + 1.\n\
               \x20   Let mutable lo be 1.\n\
               \x20   Let mutable hi be n.\n\
               \x20   While lo is less than hi:\n\
               \x20       Let tmp be item lo of perm.\n\
               \x20       Set item lo of perm to item hi of perm.\n\
               \x20       Set item hi of perm to tmp.\n\
               \x20       Set lo to lo + 1.\n\
               \x20       Set hi to hi - 1.\n\
               \x20   Set checksum to checksum + item 1 of perm.\n\
               \x20   Set r to n.\n\
               \x20   Set pass to pass + 1.\n\
               Show checksum.\n";
    let (out, err, attempts, successes) = tiered(src);
    assert_eq!(err, None);
    // After reversal `item 1 of perm` is the original last element = n - 1 = 7;
    // summed over 4000 passes.
    assert_eq!(out, (7 * 4000).to_string());
    assert!(attempts >= 1, "the hot loops must be attempted as regions");
    // EVERY attempted region must tier — a `NewEmptyList`-in-loop bail leaves a
    // tried-but-failed region (attempts > successes). The over-coarse alias
    // check bailed the outer permutation loop here; the kind-flow refinement
    // tiers it.
    assert_eq!(
        successes, attempts,
        "every hot region must tier (a NewEmptyList bail leaves \
         attempts={attempts} > successes={successes})"
    );
}

/// knapsack's DP-row shape: each outer iteration allocates a fresh `curr`
/// (`Let mutable curr be a new Seq of Int`), fills it from `prev`, then
/// `Set prev to curr` ALIASES the buffer for the next iteration. The region
/// must NOT clear-reuse `curr` (that would wipe the buffer `prev` now points
/// at); correctness against the tree-walker is mandatory whether or not it
/// tiers.
#[test]
fn knapsack_row_alloc_with_alias_stays_correct() {
    let src = "## Main\n\
               Let n be 30.\n\
               Let cols be 60.\n\
               Let mutable prev be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than cols:\n\
               \x20   Push 0 to prev.\n\
               \x20   Set i to i + 1.\n\
               Set i to 0.\n\
               While i is less than n:\n\
               \x20   Let mutable curr be a new Seq of Int.\n\
               \x20   Let mutable w be 0.\n\
               \x20   While w is less than cols:\n\
               \x20       Let best be item (w + 1) of prev.\n\
               \x20       Push best + i to curr.\n\
               \x20       Set w to w + 1.\n\
               \x20   Set prev to curr.\n\
               \x20   Set i to i + 1.\n\
               Show item 1 of prev.\n\
               Show item 60 of prev.\n";
    let (out, err, _attempts, _successes) = tiered(src);
    assert_eq!(err, None);
    // prev[k] accumulates +i each row across n=30 rows: 0+1+...+29 = 435.
    assert_eq!(out, "435\n435");
}

/// Pure correctness: a fresh list per iteration that IS reusable (sole-owned),
/// where each iteration's content differs, so a stale-buffer bug (failing to
/// clear, or clearing an alias) would corrupt the answer.
#[test]
fn fresh_list_per_iter_content_is_independent() {
    let src = "## Main\n\
               Let mutable total be 0.\n\
               Let mutable outer be 0.\n\
               While outer is less than 600:\n\
               \x20   Let mutable buf be a new Seq of Int.\n\
               \x20   Let mutable j be 0.\n\
               \x20   While j is less than 5:\n\
               \x20       Push outer + j to buf.\n\
               \x20       Set j to j + 1.\n\
               \x20   Set total to total + length of buf.\n\
               \x20   Set total to total + item 5 of buf.\n\
               \x20   Set outer to outer + 1.\n\
               Show total.\n";
    let (out, err, attempts, successes) = tiered(src);
    assert_eq!(err, None);
    // Each iteration: length 5 + (outer + 4). Sum over outer=0..599:
    // 600*5 + sum(outer) + 600*4 = 3000 + 179700 + 2400 = 185100.
    assert_eq!(out, "185100");
    assert!(
        successes >= 1,
        "a fresh-list-per-iteration loop must tier \
         (attempts={attempts} successes={successes})"
    );
}
