//! FORGE PIN-LENGTH GAUNTLET — adversarial soundness net for the pin machinery, focused on the class
//! of bug that let hot `length of <list-param>` callees miscompile: a region LEVER-B cross-function call
//! stages only the arg handle and skips the boundary pin scatter, so a callee that reads a PIN SLOT
//! (length, or a checked bound) rather than deriving from the handle got a STALE value. The callee now
//! refreshes its own pin triple from the handle at entry; these cases exercise every way `length of` and
//! pinned reads flow through a HOT caller (the region path), plus float-plane, mutation, nesting, growth,
//! empty lists, and multi-param cross-reference. Each runs its helper ≥ the tier-up threshold through the
//! forge tier and must be byte-identical to the tree-walker. A divergence here is a live JIT soundness bug.

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn forge_eq_tw(label: &str, src: &str) {
    let tw = tw_outcome_with_args(src, &[]);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    assert!(
        tw.error.is_none(),
        "[{label}] tree-walker errored: {:?}\n{src}",
        tw.error
    );
    assert!(
        forge.error.is_none(),
        "[{label}] forge JIT errored: {:?}\n{src}",
        forge.error
    );
    assert_eq!(
        forge.output.trim(),
        tw.output.trim(),
        "[{label}] forge JIT diverged from the tree-walker:\n{src}"
    );
}

/// A hot driver: define `helper`, call it 500× from a loop (tiers the caller as a region and the helper
/// as a function), `Show` an accumulator over its results.
fn driver(helper: &str, setup: &str, call: &str) -> String {
    format!(
        "{helper}\n## Main\n{setup}Let mutable acc be 0.\nLet mutable k be 0.\nWhile k is less than 500:\n\
        \x20   Set acc to acc + ({call}).\n\
        \x20   Set k to k + 1.\nShow acc.\n"
    )
}

#[test]
fn length_in_arithmetic_not_just_bound() {
    // `length of xs` used in an arithmetic expression, not a loop bound — reads the pin length directly.
    forge_eq_tw(
        "length in arithmetic",
        &driver(
            "## To lenPlus (xs: Seq of Int) -> Int:\n\
        \x20   Return (length of xs) * 3 + (item 1 of xs).",
            "Let xs be [10, 20, 30, 40].\n",
            "lenPlus(xs)",
        ),
    );
}

#[test]
fn float_list_length_bound_early_return() {
    // FLOAT-plane pin + `length of vs` bound + early return (float pin path through a hot caller).
    forge_eq_tw(
        "float length bound + early return",
        &driver(
            "## To firstBig (vs: Seq of Float, t: Float) -> Int:\n\
        \x20   Let n be length of vs.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most n:\n\
        \x20       If (item i of vs) is greater than t:\n\
        \x20           Return i.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return 0.",
            "Let vs be [0.5, 1.5, 2.5, 3.5, 4.5].\n",
            "firstBig(vs, 2.9)",
        ),
    );
}

#[test]
fn two_list_params_one_length_one_indexed() {
    // Two list params: bound by `length of a`, cross-indexing both a and b.
    forge_eq_tw(
        "two list params, cross-referenced",
        &driver(
            "## To dot (a: Seq of Int, b: Seq of Int) -> Int:\n\
        \x20   Let n be length of a.\n\
        \x20   Let mutable s be 0.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most n:\n\
        \x20       Set s to s + (item i of a) * (item i of b).\n\
        \x20       Set i to i + 1.\n\
        \x20   Return s.",
            "Let a be [1, 2, 3, 4].\nLet b be [5, 6, 7, 8].\n",
            "dot(a, b)",
        ),
    );
}

#[test]
fn mutate_and_length_in_same_callee() {
    // A callee that BOTH mutates the passed list (SetIndex) AND reads its length, called hot. The
    // mutation is visible across calls (shared buffer), so tw and forge must agree cumulatively.
    forge_eq_tw(
        "mutate + length in one callee",
        &driver(
            "## To bump (xs: mutable Seq of Int) -> Int:\n\
        \x20   Let n be length of xs.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most n:\n\
        \x20       Set item i of xs to (item i of xs) + 1.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return item 1 of xs.",
            "Let mutable xs be [0, 0, 0].\n",
            "bump(xs)",
        ),
    );
}

#[test]
fn nested_list_param_calls() {
    // fn A calls fn B, both list-param, both read `length of`. Nested cross-function pin flow.
    forge_eq_tw(
        "nested list-param calls",
        &driver(
            "## To sumB (xs: Seq of Int) -> Int:\n\
        \x20   Let n be length of xs.\n\
        \x20   Let mutable s be 0.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most n:\n\
        \x20       Set s to s + (item i of xs).\n\
        \x20       Set i to i + 1.\n\
        \x20   Return s.\n\
        ## To sumA (xs: Seq of Int) -> Int:\n\
        \x20   Return sumB(xs) + (length of xs).",
            "Let xs be [2, 4, 6, 8].\n",
            "sumA(xs)",
        ),
    );
}

#[test]
fn length_of_empty_list() {
    // `length of xs` on an EMPTY list — the bound is 0, the loop must not run, return the default.
    forge_eq_tw(
        "length of empty list",
        &driver(
            "## To sumOr (xs: Seq of Int, d: Int) -> Int:\n\
        \x20   Let n be length of xs.\n\
        \x20   If n is equal to 0:\n\
        \x20       Return d.\n\
        \x20   Let mutable s be 0.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most n:\n\
        \x20       Set s to s + (item i of xs).\n\
        \x20       Set i to i + 1.\n\
        \x20   Return s.",
            "Let xs be a new Seq of Int.\n",
            "sumOr(xs, 99)",
        ),
    );
}

#[test]
fn length_after_growth_inside_callee() {
    // The callee PUSHES to a fresh local list, then reads `length of` that grown list. The site pin's
    // length must track the pushes (this stresses the pin refresh on ListPush, not the param path).
    forge_eq_tw(
        "length after growth",
        &driver(
            "## To build (m: Int) -> Int:\n\
        \x20   Let mutable ys be a new Seq of Int.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most m:\n\
        \x20       Push (i * i) to ys.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return (length of ys) * 100 + (item (length of ys) of ys).",
            "",
            "build(7)",
        ),
    );
}

#[test]
fn length_bound_in_nested_loop_region() {
    // A list-param callee whose `length of`-bounded loop is itself NESTED (loop-in-loop), called hot.
    forge_eq_tw(
        "length bound in nested loop",
        &driver(
            "## To countPairs (xs: Seq of Int) -> Int:\n\
        \x20   Let n be length of xs.\n\
        \x20   Let mutable c be 0.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most n:\n\
        \x20       Let mutable j be i + 1.\n\
        \x20       While j is at most n:\n\
        \x20           If (item i of xs) is less than (item j of xs):\n\
        \x20               Set c to c + 1.\n\
        \x20           Set j to j + 1.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return c.",
            "Let xs be [3, 1, 4, 1, 5, 9, 2, 6].\n",
            "countPairs(xs)",
        ),
    );
}
