//! Phase B4 — Flow-sensitive refinement (PE_IMPROVE §5, closes gap G5).
//!
//! A guard refines `staticEnv` inside the taken branch (positive info: `If x == 3` ⇒ x=3 in
//! the then-branch) and its negation in the other branch (negative info: x≠3 in the else,
//! usable to fold a nested guard). Provably-dead arms are eliminated; facts never leak across
//! branches; a later mutation invalidates the fact. Correctness over optimization — never fold
//! unsoundly. RED-first per CLAUDE.md.
//!
//! NOTE: write programs as `"\` + real newlines + real indentation (NOT `\n\` continuation).
//! A `Repeat for i from 1 to 100` loop keeps the accumulator genuinely dynamic (range > 64 is
//! not unrolled), so guards over it actually exercise refinement rather than constant folding.

mod pe_support;

use pe_support::*;

/// A static guard that is true prunes to the then-branch only (no residual `If`).
#[test]
fn if_eq_static_prunes_branch() {
    let program = "\
## Main
Let x be 3.
If x equals 3:
    Show \"yes\".
Otherwise:
    Show \"no\".";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("yes") && !residual.contains("no"),
        "static-true guard must keep only the then-branch:\n{}",
        residual
    );
    assert_run_equals(program, "yes");
}

/// A static guard that is false takes the else-branch only.
#[test]
fn if_eq_static_false_takes_else() {
    let program = "\
## Main
Let x be 4.
If x equals 3:
    Show \"yes\".
Otherwise:
    Show \"no\".";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("no") && !residual.contains("yes"),
        "static-false guard must keep only the else-branch:\n{}",
        residual
    );
    assert_run_equals(program, "no");
}

/// Positive refinement: inside the then-branch of `If x == 100` (x dynamic), x folds to 100.
#[test]
fn then_branch_gets_positive_fact() {
    let program = "\
## Main
Let mutable x be 0.
Repeat for i from 1 to 100:
    Set x to x + 1.
If x equals 100:
    Show x.
Otherwise:
    Show 0.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 100"),
        "then-branch must refine x to 100 and fold `Show x` to `Show 100`:\n{}",
        residual
    );
    assert_run_equals(program, "100");
}

/// Negative refinement: in the else of `If x == 5` (x dynamic), x ≠ 5 folds a nested `If x == 5`
/// to false, eliminating its then-arm.
#[test]
fn else_branch_gets_negative_fact() {
    let program = "\
## Main
Let mutable x be 0.
Repeat for i from 1 to 100:
    Set x to x + 1.
If x equals 5:
    Show \"a\".
Otherwise:
    If x equals 5:
        Show \"b\".
    Otherwise:
        Show \"c\".";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("\"b\""),
        "else negative fact (x != 5) must prune the nested then-arm (\"b\"):\n{}",
        residual
    );
    assert_run_equals(program, "c");
}

/// A then-branch fact must NOT leak into the else-branch.
#[test]
fn no_fact_leak_across_branches() {
    let program = "\
## Main
Let mutable x be 0.
Repeat for i from 1 to 100:
    Set x to x + 1.
If x equals 5:
    Show x.
Otherwise:
    Show x.";
    let residual = decompile(program).expect("PE should not fail");
    // then folds to `Show 5`; else must remain `Show x` (no leak of x=5).
    assert!(
        residual.contains("Show x"),
        "else-branch must not inherit the then-branch fact (x=5); expected a dynamic `Show x`:\n{}",
        residual
    );
    assert_run_equals(program, "100");
}

/// A mutation inside the branch invalidates the guard fact: after `Set x to 4`, x reads 4.
#[test]
fn refinement_invalidated_by_later_mutation() {
    let program = "\
## Main
Let mutable x be 0.
Repeat for i from 1 to 100:
    Set x to x + 1.
If x equals 5:
    Set x to 4.
    Show x.
Otherwise:
    Show 0.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 4") && !residual.contains("Show 5"),
        "the x=5 fact must be dropped after `Set x to 4`; expected `Show 4`:\n{}",
        residual
    );
    assert_run_equals(program, "0");
}

/// Compound guard `and`: an equality conjunct still refines the then-branch.
#[test]
fn compound_guard_and_refines() {
    let program = "\
## Main
Let mutable x be 0.
Repeat for i from 1 to 100:
    Set x to x + 1.
If x equals 100 and x is greater than 0:
    Show x.
Otherwise:
    Show 0.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 100"),
        "an `and` with an equality conjunct must refine x to 100:\n{}",
        residual
    );
    assert_run_equals(program, "100");
}

/// Compound guards `or` / `not` must remain sound (fold conservatively, never unsoundly).
#[test]
fn compound_guard_or_not_sound() {
    let program = "\
## Main
Let mutable x be 0.
Repeat for i from 1 to 100:
    Set x to x + 1.
If x equals 5 or x is greater than 50:
    Show \"hi\".
Otherwise:
    Show \"lo\".";
    // x = 100 ⇒ second disjunct true ⇒ \"hi\". Must be correct regardless of refinement.
    assert_run_equals(program, "hi");
}
