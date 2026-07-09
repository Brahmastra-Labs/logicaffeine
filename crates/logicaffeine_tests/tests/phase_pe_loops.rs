//! Phase B3 — Loop specialization + MSG (PE_IMPROVE §5, closes gaps G3, G4).
//!
//! G3: `CWhile` never unrolls; `CRepeatRange`/`CRepeat` only unroll fully-static shapes.
//! G4: no MSG — on whistle blow the PE residualizes (cuts) instead of generalizing.
//!
//! B3 makes: statically-bounded loops fully unroll (no loop in residual); dynamic loops
//! with a static-shaped body specialize to a clean residual loop without state explosion
//! (whistle + MSG converge); all of it preserves meaning.
//!
//! NOTE: write programs as `"\` + real newlines + real indentation — NOT `\n\` continuation
//! (it strips the next line's leading whitespace, breaking loop-body indentation).

mod pe_support;

use pe_support::*;

// ===========================================================================
// G3 — static loop unrolling / elimination.
// ===========================================================================

/// A `While` with a statically-decreasing counter fully unrolls: no `While` survives in the
/// residual, and the accumulator folds to its constant.
#[test]
fn while_static_trip_count_unrolls() {
    let program = "\
## Main
Let mutable i be 3.
Let mutable s be 0.
While i is greater than 0:
    Set s to s + i.
    Set i to i - 1.
Show s.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 6"),
        "static-trip-count while must fold the accumulator to 6:\n{}",
        residual
    );
    assert!(
        !residual.contains("While "),
        "no While should survive a static trip count:\n{}",
        residual
    );
    assert_run_equals(program, "6");
}

/// A range that iterates zero times is removed entirely (the accumulator keeps its prior
/// static value).
#[test]
fn repeat_range_zero_iterations_eliminated() {
    let program = "\
## Main
Let mutable s be 7.
Repeat for i from 1 to 0:
    Set s to s + i.
Show s.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 7"),
        "empty range must leave s=7 and fold:\n{}",
        residual
    );
    assert!(
        !residual.contains("Repeat for"),
        "empty range loop must be removed:\n{}",
        residual
    );
    assert_run_equals(program, "7");
}

/// `Break` in iteration 2 of a static unroll stops unrolling there.
#[test]
fn repeat_break_stops_unroll() {
    let program = "\
## Main
Let mutable s be 0.
Repeat for i from 1 to 10:
    Set s to s + i.
    If i equals 2:
        Break.
Show s.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 3"),
        "break after i=2 must fold s to 1+2=3:\n{}",
        residual
    );
    assert!(
        !residual.contains("Repeat for"),
        "static unroll with break must not leave a loop:\n{}",
        residual
    );
    assert_run_equals(program, "3");
}

/// `Return` mid-unroll truncates the unroll correctly.
#[test]
fn repeat_return_stops_unroll() {
    let program = "\
## To f () -> Int:
    Let mutable s be 0.
    Repeat for i from 1 to 10:
        Set s to s + i.
        If i equals 3:
            Return s.
    Return s.

## Main
Show f().";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 6"),
        "return at i=3 must fold f() to 1+2+3=6:\n{}",
        residual
    );
    assert_run_equals(program, "6");
}

/// A very large static range must NOT unroll unboundedly — the PE respects its budget and
/// falls back to a residual loop (totality over optimality).
#[test]
fn loop_large_static_range_does_not_explode() {
    let program = "\
## Main
Let mutable s be 0.
Repeat for i from 1 to 200000:
    Set s to s + 1.
Show s.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Repeat for"),
        "large static range must fall back to a residual loop, not unroll:\n{}",
        residual
    );
    assert_run_equals(program, "200000");
}

// ===========================================================================
// Edge cases.
// ===========================================================================

/// A `While` whose condition is statically false never executes and is removed.
#[test]
fn while_static_false_eliminated() {
    let program = "\
## Main
Let mutable s be 5.
While s is greater than 100:
    Set s to s + 1.
Show s.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 5") && !residual.contains("While "),
        "statically-false while must be removed, s=5:\n{}",
        residual
    );
    assert_run_equals(program, "5");
}

/// Nested static loops both unroll.
#[test]
fn nested_static_loops_unroll() {
    let program = "\
## Main
Let mutable s be 0.
Repeat for i from 1 to 3:
    Repeat for j from 1 to 3:
        Set s to s + 1.
Show s.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 9"),
        "nested 3x3 static loops must fold s to 9:\n{}",
        residual
    );
    assert!(
        !residual.contains("Repeat for"),
        "nested static loops must fully unroll:\n{}",
        residual
    );
    assert_run_equals(program, "9");
}

// ===========================================================================
// G4 — MSG (most-specific generalization) on whistle blow.
// A recursive call whose STATIC argument grows each step (acc = 0,1,2,…) makes the cycle
// key change every recursion → no convergence without MSG. MSG generalizes the growing
// static arg to a fresh dynamic variable so specialization converges to one clean loop.
// ===========================================================================

/// The whistle blows on the growing static accumulator; MSG generalizes it back to the
/// generic recursive function (the most-specific generalization of count(_,0), count(_,1), …
/// is count(_, generic)). The residual CONVERGES to a bounded form instead of exploding to
/// depth-many specializations (or overflowing), and meaning is preserved. The convergence
/// (bounded + correct, no blow-up) is the real G4 signal — without MSG this overflows.
#[test]
fn dynamic_loop_msg_generalizes() {
    let program = "\
## To count (n: Int) and (acc: Int) -> Int:
    If n equals 0:
        Return acc.
    Return count(n - 1, acc + 1).

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + 1.
Show count(d, 0).";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.lines().count() < 40,
        "MSG must converge to a bounded residual, not depth-many specializations ({} lines):\n{}",
        residual.lines().count(),
        residual
    );
    assert!(
        residual.contains("count"),
        "the generalized recursive loop (count) must survive in the residual:\n{}",
        residual
    );
    assert_run_equals(program, "100");
}

/// MSG is a fixpoint: re-specializing the generalized residual yields the same result
/// (no further growth), and it still runs correctly.
#[test]
fn msg_idempotent() {
    let program = "\
## To count (n: Int) and (acc: Int) -> Int:
    If n equals 0:
        Return acc.
    Return count(n - 1, acc + 1).

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + 1.
Show count(d, 0).";
    let r1 = decompile(program).expect("PE should not fail");
    let r2 = decompile(&r1).expect("re-PE should not fail");
    // Fixpoint: the second pass must not grow the residual (MSG converged).
    assert!(
        r2.lines().count() <= r1.lines().count(),
        "PE(PE(loop)) must not grow vs PE(loop):\nR1:\n{}\nR2:\n{}",
        r1,
        r2
    );
    assert_run_equals(&r1, "100");
    assert_run_equals(&r2, "100");
}

/// A while with a static counter but a dynamic accumulator: the loop still unrolls (trip
/// count is static), and the dynamic accumulator residualizes correctly inside the unroll.
#[test]
fn while_static_count_dynamic_accumulator() {
    let program = "\
## Main
Let mutable d be 0.
Repeat for k from 1 to 50000:
    Set d to d + 1.
Let mutable i be 3.
Let mutable s be d.
While i is greater than 0:
    Set s to s + 1.
    Set i to i - 1.
Show s.";
    // d = 50000 (dynamic, residualized big loop); s = d + 3 = 50003.
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("While "),
        "static-trip while must unroll even with a dynamic accumulator:\n{}",
        residual
    );
    assert_run_equals(program, "50003");
}
