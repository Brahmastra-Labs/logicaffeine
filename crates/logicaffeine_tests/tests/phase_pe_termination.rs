//! Phase A — Soundness & termination foundation for the partial evaluator (PE_IMPROVE §5).
//!
//! Property under test: totality + semantic preservation; no state explosion. Each test
//! pairs a totality assertion (the PE halts within budget) with a correctness assertion
//! (the residual run-equals the tree-walker). RED-first per CLAUDE.md.

mod pe_support;

use pe_support::*;

/// Smoke test for the in-process harness: the genuine LOGOS PE folds a constant
/// expression and the residual runs to the same value as the tree-walker. This
/// validates that `interpret_program` can execute the combined PE source (the
/// load-bearing fast-path assumption) before the real Phase A tests rely on it.
#[test]
fn harness_in_process_path_works() {
    let program = "## Main\nLet x be 2 + 3 * 4.\nShow x.";
    assert_run_equals(program, "14");
}

/// The Jones oracle reads a clean residual as dispatch-free.
#[test]
fn harness_count_dispatch_on_clean_residual() {
    assert_eq!(count_dispatch("## Main\nShow 14."), 0);
}

/// And flags surviving interpreter dispatch.
#[test]
fn harness_count_dispatch_flags_dispatch() {
    let dispatchy = "## Main\nLet v be item \"x\" of env.\nShow v.";
    assert!(count_dispatch(dispatchy) >= 1, "should detect env lookup");
}

// ===========================================================================
// Phase A — soundness & termination. RED-first.
// ===========================================================================

/// Edge case: a function with an early return followed by dead code that also
/// returns. Execution returns at the FIRST return; the PE must agree. `extractReturn`
/// scans bottom-up, so this guards against it picking the last (dead) return.
#[test]
fn early_return_dead_code_after() {
    let program = "\
## To pick (n: Int) -> Int:
    Return 5.
    Return 10.

## Main
Show pick(0).";
    assert_run_equals(program, "5");
}

/// A-3: a return buried in nested `If`s with statically-true guards. The PE selects
/// both branches statically; the residual must fold to the returned constant.
#[test]
fn pe_return_inside_nested_if() {
    let program = "\
## To f (n: Int) -> Int:
    If n > 0:
        If n > 1:
            Return 7.
    Return 0.

## Main
Show f(5).";
    assert_run_equals(program, "7");
}

/// A-1 (terminating form): deep mutual recursion with static argument. The PE must
/// fully unfold even↔odd without blowing the whistle prematurely, halt within budget,
/// and the residual must be correct.
#[test]
fn pe_terminates_on_mutual_recursion() {
    let program = "\
## To isEven (n: Int) -> Bool:
    If n == 0:
        Return true.
    Return isOdd(n - 1).

## To isOdd (n: Int) -> Bool:
    If n == 0:
        Return false.
    Return isEven(n - 1).

## Main
Show isEven(10).";
    assert_run_equals(program, "true");
}

/// A-4 (direct form): a variable written inside a dynamic `While` must lose its static
/// fact after the loop; the residual must read the runtime value, not the pre-loop
/// constant.
#[test]
fn pe_while_invalidates_direct_writes() {
    let program = "\
## Main
Let x be 5.
Let i be 0.
While i < 3:
    Set x to x + i.
    Set i to i + 1.
Show x.";
    assert_run_equals(program, "8");
}

/// A-2: a `Return` buried inside an `Inspect` arm. With a statically-known variant the
/// PE selects the arm; the residual must fold to the returned value rather than leaving
/// a dangling call (which `extractReturn` would cause if it ignored Inspect arms).
#[test]
fn pe_return_inside_inspect_arm() {
    let program = "\
## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.

## To area (sh: Shape) -> Int:
    Inspect sh:
        When Circle (radius):
            Return radius * 3.
        When Square (side):
            Return side * side.

## Main
Let c be a new Circle with radius 5.
Show area(c).";
    assert_run_equals(program, "15");
}

/// A-6: two genuinely different all-static calls whose memo keys collide under the
/// underscore-joined `makeKey` scheme (`cat_tx_ty_tz` for both). The PE must specialize
/// them distinctly, not reuse the first's cached residual for the second.
#[test]
fn pe_memo_key_no_collision() {
    let program = "\
## To cat (a: Text) and (b: Text) -> Text:
    Return a + b.

## Main
Let p be cat(\"x\", \"y_tz\").
Let q be cat(\"x_ty\", \"z\").
Show \"{p}|{q}\".";
    assert_run_equals(program, "xy_tz|x_tyz");
}

/// Edge: an inner `Let` inside an inlined function must not corrupt an outer static
/// binding of a different (or same) name.
#[test]
fn inner_let_shadows_outer_static() {
    let program = "\
## To g (a: Int) -> Int:
    Let x be a + 1.
    Return x.

## Main
Let x be 100.
Let r be g(5).
Show \"{x}|{r}\".";
    assert_run_equals(program, "100|6");
}

/// Edge: a `Break` mid static-range unroll stops unrolling at that iteration.
#[test]
fn break_mid_unroll() {
    let program = "\
## Main
Let mutable total be 0.
Repeat for i from 1 to 5:
    If i == 3:
        Break.
    Set total to total + i.
Show total.";
    assert_run_equals(program, "3");
}

/// Edge: the same function reached from two distinct static call sites specializes
/// each correctly without conflating their memoized results.
#[test]
fn function_reached_by_two_static_paths() {
    let program = "\
## To sq (n: Int) -> Int:
    Return n * n.

## Main
Let a be sq(3).
Let b be sq(4).
Show \"{a}|{b}\".";
    assert_run_equals(program, "9|16");
}

/// Soundness: a variable modified inside a residualized (non-unrolled, range > 64) loop
/// must be a clean dynamic var after the loop — the residual must NOT leak the loop
/// variable (the bug: the loop body's `Set` re-adds the var to staticEnv as `acc + i`
/// after invalidation, leaking `i` out of scope). Mirrors CWhile's post-body invalidation.
#[test]
fn residualized_range_loop_var_read_after_is_dynamic() {
    let program = "\
## Main
Let mutable acc be 0.
Repeat for i from 1 to 100:
    Set acc to acc + i.
Show acc.";
    assert_run_equals(program, "5050");
}

/// Same soundness property for `Repeat for x in <large static list>` — guards CRepeat's
/// residualize path. A 100-element list exceeds nothing special, but the accumulator must
/// still be dynamic after if the loop residualizes. (Static lists unroll, so use a value
/// the loop carries; the key assertion is run-equality.)
#[test]
fn residualized_range_loop_accumulator_correct() {
    let program = "\
## Main
Let mutable total be 0.
Repeat for k from 1 to 200:
    Set total to total + 1.
Show total.";
    assert_run_equals(program, "200");
}

/// A-4 (Inspect-nested form): a write to a previously-static variable buried inside an
/// `Inspect` arm within a dynamic loop must invalidate that variable's static fact. This
/// exercises `collectSetVars` recursing into Inspect arms.
#[test]
fn pe_while_invalidates_inspect_nested_writes() {
    let program = "\
## A Cmd is one of:
    A Inc with amount Int.
    A Halt with amount Int.

## Main
Let mutable x be 0.
Let c be a new Inc with amount 1.
Let mutable i be 0.
While i < 3:
    Inspect c:
        When Inc (amount):
            Set x to x + amount.
        When Halt (amount):
            Set x to x.
    Set i to i + 1.
Show x.";
    assert_run_equals(program, "3");
}
