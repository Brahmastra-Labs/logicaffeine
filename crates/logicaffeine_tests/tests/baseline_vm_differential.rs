//! BASELINE-TIER DIFFERENTIAL GATE
//!
//! The Studio's interactive `.logos` path runs on the PLAIN VM
//! (`interpret_for_ui_baseline_*`): the bytecode VM with NO run-path optimizer
//! (`with_parsed_program`, not `with_optimized_program`) and NO oracle
//! (`compile_with_types`, not `compile_with_oracle`). `largo run` / benchmarks
//! keep the OPTIMIZED VM (`interpret_for_ui_sync_with_args`).
//!
//! This gate proves the baseline tier is observationally IDENTICAL to the
//! optimized tier across a broad program corpus — so dropping the optimizer for
//! interactive responsiveness can never change a program's meaning. Each sync
//! entry additionally self-checks against the tree-walker (the debug shadow
//! oracle), so in a debug run all three engines — baseline VM, optimized VM,
//! tree-walker — are transitively pinned equal.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::{
    interpret_for_ui_baseline_sync_with_args, interpret_for_ui_sync_with_args,
};

/// The baseline VM and the optimized VM must agree, bit for bit, on output and
/// error — and neither may error on a well-formed program.
fn assert_baseline_matches_optimized(src: &str) {
    let baseline = interpret_for_ui_baseline_sync_with_args(src, &[]);
    let optimized = interpret_for_ui_sync_with_args(src, &[]);
    assert_eq!(
        optimized.error, None,
        "optimized VM errored on:\n{src}\n=> {:?}",
        optimized.error
    );
    assert_eq!(
        baseline.error, None,
        "baseline VM errored on:\n{src}\n=> {:?}",
        baseline.error
    );
    assert_eq!(
        (&baseline.lines, &baseline.error),
        (&optimized.lines, &optimized.error),
        "baseline VM diverged from the optimized VM for:\n{src}"
    );
}

#[test]
fn baseline_matches_optimized_arithmetic() {
    assert_baseline_matches_optimized("## Main\nLet a be 3.\nLet b be 5.\nShow a + b.\n");
    assert_baseline_matches_optimized(
        "## Main\nLet a be 7.\nLet b be a * a - 9.\nShow b * 2 + a.\n",
    );
    // i64 wrapping at the boundary — the optimizer must not change the value.
    assert_baseline_matches_optimized("## Main\nLet a be 9223372036854775807.\nShow a + 1.\n");
}

#[test]
fn baseline_matches_optimized_control_flow() {
    // A while-loop factorial: the classic accumulator the optimizer rewrites.
    assert_baseline_matches_optimized(
        "## Main\n\
         Let mutable n be 10.\n\
         Let mutable acc be 1.\n\
         While n is greater than 1:\n\
         \x20   Set acc to acc * n.\n\
         \x20   Set n to n - 1.\n\
         Show acc.\n",
    );
    // Nested loops — LICM / induction territory.
    assert_baseline_matches_optimized(
        "## Main\n\
         Let mutable total be 0.\n\
         Let mutable i be 1.\n\
         While i is at most 5:\n\
         \x20   Let mutable j be 1.\n\
         \x20   While j is at most 5:\n\
         \x20       Set total to total + i * j.\n\
         \x20       Set j to j + 1.\n\
         \x20   Set i to i + 1.\n\
         Show total.\n",
    );
    // Branch-selected value.
    assert_baseline_matches_optimized(
        "## Main\n\
         Let x be 7.\n\
         Let mutable label be \"small\".\n\
         If x is greater than 5:\n\
         \x20   Set label to \"big\".\n\
         Show label.\n",
    );
}

#[test]
fn baseline_matches_optimized_collections() {
    assert_baseline_matches_optimized(
        "## Main\n\
         Let mutable xs be a new Seq of Int.\n\
         Let mutable i be 1.\n\
         While i is at most 5:\n\
         \x20   Push i to xs.\n\
         \x20   Set i to i + 1.\n\
         Show item 3 of xs.\n",
    );
}

#[test]
fn baseline_matches_optimized_maps() {
    assert_baseline_matches_optimized(
        "## Main\n\
         Let mutable prices be a new Map of Text to Int.\n\
         Set item \"iron\" of prices to 100.\n\
         Set item \"gold\" of prices to 250.\n\
         Let cost be item \"iron\" of prices.\n\
         Show cost.\n",
    );
}

#[test]
fn baseline_matches_optimized_functions_and_recursion() {
    assert_baseline_matches_optimized(
        "## To double (x: Int):\n\
         \x20   Return x * 2.\n\
         \n\
         ## Main\n\
         Let result be double(21).\n\
         Show result.\n",
    );
    assert_baseline_matches_optimized(
        "## To factorial (n: Int):\n\
         \x20   If n is less than 2:\n\
         \x20       Return 1.\n\
         \x20   Return n * factorial(n - 1).\n\
         \n\
         ## Main\n\
         Show factorial(6).\n",
    );
}

#[test]
fn baseline_matches_optimized_strings_and_booleans() {
    assert_baseline_matches_optimized("## Main\nShow \"hello\".\n");
    assert_baseline_matches_optimized("## Main\nLet a be 7.\nShow a is greater than 3.\n");
}

#[test]
fn baseline_matches_optimized_aliasing_double_buffer() {
    // The knapsack double-buffer alias: `Set prev to curr` makes prev and curr
    // the SAME backing store, with a cross-index read while writing. An unsound
    // optimization (swap / distinct-buffer assumption) diverges here — the
    // baseline (no optimizer) is the reference, so they MUST agree.
    assert_baseline_matches_optimized(
        r#"## Main
Let mutable prev be a new Seq of Int.
Let mutable j be 0.
While j is less than 5:
    Push 0 to prev.
    Set j to j + 1.
Let mutable curr be a new Seq of Int.
Set j to 0.
While j is less than 5:
    Push 0 to curr.
    Set j to j + 1.
Let mutable i be 0.
While i is less than 3:
    Let mutable w be 1.
    While w is at most 4:
        Set item (w + 1) of curr to item (w + 1) of prev.
        Let take be item w of prev + 1.
        If take is greater than item (w + 1) of curr:
            Set item (w + 1) of curr to take.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item 5 of prev.
"#,
    );
}
