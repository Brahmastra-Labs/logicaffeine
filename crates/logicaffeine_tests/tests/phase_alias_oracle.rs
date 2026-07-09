//! O1-A gate — loop-invariant alias snapshots for borrow hoisting.
//!
//! Borrow hoisting (O1) replaces per-access `RefCell` borrows with one
//! borrow per loop, which is sound only when the hoisted handles are
//! PROVABLY distinct allocations at every iteration. These tests define the
//! oracle query that decision rests on:
//!
//!   `OracleFacts::loop_handles_definitely_distinct(loop_stmt, a, b)`
//!
//! The query must answer `true` only when the flow-sensitive alias analysis
//! proves distinctness at the loop invariant — including loop-CARRIED
//! aliasing (an edge created at the end of one iteration must be visible at
//! the top of the next), handles of unknown provenance (extracted from
//! containers, returned from calls, popped, bound by Repeat, function
//! parameters — all "tainted": they may alias anything), and refusal by
//! default for loops the analysis never converged on or never saw.

mod common;

use logicaffeine_compile::optimize::{oracle_analyze_with, OracleFacts};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::ast::stmt::Stmt;
use logicaffeine_compile::intern::{Interner, Symbol};

/// Parse, analyze, and hand the facts + AST + interner to the assertion body.
fn with_alias_facts<T>(
    src: &str,
    f: impl for<'a> FnOnce(&OracleFacts, &'a [Stmt<'a>], &Interner) -> T,
) -> T {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("parse");
        let facts = oracle_analyze_with(stmts, interner);
        f(&facts, stmts, interner)
    })
}

/// Resolve a variable name to its interned symbol.
fn sym(interner: &Interner, name: &str) -> Symbol {
    interner
        .lookup(name)
        .unwrap_or_else(|| panic!("symbol `{}` not interned", name))
}

/// Collect every While/Repeat statement in program order, recursively.
fn collect_loops<'a>(stmts: &'a [Stmt<'a>], out: &mut Vec<&'a Stmt<'a>>) {
    for s in stmts {
        match s {
            Stmt::While { body, .. } => {
                out.push(s);
                collect_loops(body, out);
            }
            Stmt::Repeat { body, .. } => {
                out.push(s);
                collect_loops(body, out);
            }
            Stmt::If { then_block, else_block, .. } => {
                collect_loops(then_block, out);
                if let Some(eb) = else_block {
                    collect_loops(eb, out);
                }
            }
            Stmt::FunctionDef { body, .. } => collect_loops(body, out),
            Stmt::Zone { body, .. } => collect_loops(body, out),
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                collect_loops(tasks, out);
            }
            _ => {}
        }
    }
}

/// The nth loop (While or Repeat) of the whole program, in source order.
fn nth_loop<'a>(stmts: &'a [Stmt<'a>], n: usize) -> &'a Stmt<'a> {
    let mut loops = Vec::new();
    collect_loops(stmts, &mut loops);
    loops
        .get(n)
        .copied()
        .unwrap_or_else(|| panic!("program has no loop #{} ({} found)", n, loops.len()))
}

// ---------------------------------------------------------------------------
// Positive proofs: fresh allocations stay distinct
// ---------------------------------------------------------------------------

#[test]
fn fresh_seqs_are_distinct_in_loop() {
    with_alias_facts(
        r#"## Main
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Push 1 to a.
Push 2 to b.
Let mutable i be 0.
While i is less than 5:
    Set item 1 of a to item 1 of b.
    Set i to i + 1.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "b")),
                "two fresh Seqs must be provably distinct"
            );
        },
    );
}

#[test]
fn knapsack_shape_fresh_relet_inner_distinct() {
    // The headline shape: the outer body re-`Let`s curr FRESH each iteration,
    // so the prev–curr edge created by `Set prev to curr` at the end of one
    // iteration is severed before the inner loop of the next. The inner loop
    // may hoist both handles.
    with_alias_facts(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("3").
Let mutable prev be a new Seq of Int.
Push 0 to prev.
Push 0 to prev.
Let mutable i be 0.
While i is less than n:
    Let mutable curr be a new Seq of Int.
    Push 0 to curr.
    Push 0 to curr.
    Let mutable w be 0.
    While w is less than 2:
        Set item (w + 1) of curr to item (w + 1) of prev.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item 1 of prev.
"#,
        |facts, stmts, interner| {
            let inner = nth_loop(stmts, 1);
            assert!(
                facts.loop_handles_definitely_distinct(
                    inner,
                    sym(interner, "prev"),
                    sym(interner, "curr")
                ),
                "fresh re-Let severs the loop-carried edge: prev/curr distinct in the inner loop"
            );
        },
    );
}

#[test]
fn taint_cleared_by_fresh_rebind() {
    // A call-tainted handle rebound to a fresh allocation is tracked again.
    with_alias_facts(
        r#"## To pick (xs: Seq of Int) -> Seq of Int:
    Return xs.

## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be pick(a).
Set b to a new Seq of Int.
Push 2 to b.
Let mutable i be 0.
While i is less than 3:
    Set item 1 of b to item 1 of a.
    Set i to i + 1.
Show item 1 of b.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "b")),
                "rebinding to a fresh Seq clears call taint"
            );
        },
    );
}

// ---------------------------------------------------------------------------
// Aliasing through bindings
// ---------------------------------------------------------------------------

#[test]
fn let_alias_not_distinct() {
    with_alias_facts(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a.
Let mutable i be 0.
While i is less than 3:
    Set item 1 of b to item 1 of a.
    Set i to i + 1.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                !facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "b")),
                "`Let b be a.` aliases — never distinct"
            );
        },
    );
}

#[test]
fn conditional_alias_not_distinct() {
    // The If-join must keep the may-alias edge from the taken branch.
    with_alias_facts(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let k be parseInt("1").
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a new Seq of Int.
Push 2 to b.
If k is greater than 0:
    Set b to a.
Let mutable i be 0.
While i is less than 3:
    Set item 1 of b to item 1 of a.
    Set i to i + 1.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                !facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "b")),
                "conditional `Set b to a.` may-aliases — not distinct"
            );
        },
    );
}

#[test]
fn loop_carried_alias_not_distinct() {
    // No fresh re-Let: `Set prev to curr` at the end of iteration 1 makes
    // them alias from iteration 2 onward. The loop INVARIANT must include
    // that edge — this is the test that pins the fixpoint alias-join.
    with_alias_facts(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("3").
Let mutable prev be a new Seq of Int.
Push 0 to prev.
Let mutable curr be a new Seq of Int.
Push 0 to curr.
Let mutable i be 0.
While i is less than n:
    Let mutable w be 0.
    While w is less than 1:
        Set item 1 of curr to item 1 of prev.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item 1 of prev.
"#,
        |facts, stmts, interner| {
            let inner = nth_loop(stmts, 1);
            assert!(
                !facts.loop_handles_definitely_distinct(
                    inner,
                    sym(interner, "prev"),
                    sym(interner, "curr")
                ),
                "loop-carried `Set prev to curr` aliases them at the inner loop from iteration 2 on"
            );
        },
    );
}

// ---------------------------------------------------------------------------
// Taint: handles of unknown provenance may alias anything
// ---------------------------------------------------------------------------

#[test]
fn call_result_tainted() {
    // `pick` returns its argument — b and a are one allocation. Any handle
    // produced by a call is tainted.
    with_alias_facts(
        r#"## To pick (xs: Seq of Int) -> Seq of Int:
    Return xs.

## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be pick(a).
Let mutable i be 0.
While i is less than 3:
    Set item 1 of b to item 1 of a.
    Set i to i + 1.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let mut loops = Vec::new();
            collect_loops(stmts, &mut loops);
            // The Main loop is the last loop in program order.
            let lp = *loops.last().expect("a loop");
            assert!(
                !facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "b")),
                "a call result may alias its arguments — tainted, never distinct"
            );
        },
    );
}

#[test]
fn element_extraction_tainted() {
    // A handle pulled out of a container could be any handle ever stored in
    // it.
    with_alias_facts(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable nested be a new Seq of (Seq of Int).
Push a to nested.
Let mutable x be item 1 of nested.
Let mutable i be 0.
While i is less than 3:
    Set item 1 of x to item 1 of a.
    Set i to i + 1.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                !facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "x")),
                "an extracted element may alias any stored handle — tainted"
            );
        },
    );
}

#[test]
fn pop_into_tainted() {
    with_alias_facts(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable nested be a new Seq of (Seq of Int).
Push a to nested.
Pop from nested into v.
Let mutable i be 0.
While i is less than 3:
    Set item 1 of v to item 1 of a.
    Set i to i + 1.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                !facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "v")),
                "a popped handle may alias any stored handle — tainted"
            );
        },
    );
}

#[test]
fn repeat_var_tainted() {
    with_alias_facts(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable nested be a new Seq of (Seq of Int).
Push a to nested.
Repeat for x in nested:
    Let mutable i be 0.
    While i is less than 3:
        Set item 1 of x to item 1 of a.
        Set i to i + 1.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            // Loop #0 is the Repeat; #1 is the While inside it.
            let inner = nth_loop(stmts, 1);
            assert!(
                !facts.loop_handles_definitely_distinct(
                    inner,
                    sym(interner, "a"),
                    sym(interner, "x")
                ),
                "a Repeat-bound element may alias any stored handle — tainted"
            );
        },
    );
}

#[test]
fn function_params_not_distinct() {
    // Two Seq parameters may be handed the same allocation by a caller.
    with_alias_facts(
        r#"## To touch (xs: Seq of Int, ys: Seq of Int) -> Int:
    Let mutable i be 0.
    While i is less than 3:
        Set item 1 of xs to item 1 of ys.
        Set i to i + 1.
    Return item 1 of xs.

## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Show touch(a, a).
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                !facts.loop_handles_definitely_distinct(
                    lp,
                    sym(interner, "xs"),
                    sym(interner, "ys")
                ),
                "two Seq parameters may be the same handle — tainted"
            );
        },
    );
}

// ---------------------------------------------------------------------------
// Refusal by default
// ---------------------------------------------------------------------------

#[test]
fn non_loop_statement_refuses() {
    // A statement that is not an analyzed loop has no snapshot: the query
    // must refuse, never guess.
    with_alias_facts(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a new Seq of Int.
Push 2 to b.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let not_a_loop = &stmts[0];
            assert!(
                !facts.loop_handles_definitely_distinct(
                    not_a_loop,
                    sym(interner, "a"),
                    sym(interner, "b")
                ),
                "no snapshot for a non-loop statement — must refuse"
            );
        },
    );
}

#[test]
fn concurrent_loop_refuses() {
    // Loops under a concurrent block run interleaved: the sequential alias
    // walk is not a sound model, so their snapshots must be withheld.
    with_alias_facts(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a new Seq of Int.
Push 2 to b.
Attempt all of the following:
    Let mutable i be 0.
    While i is less than 3:
        Set item 1 of a to item 1 of b.
        Set i to i + 1.
Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                !facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "b")),
                "loops inside a concurrent block must refuse — sequential alias facts are unsound there"
            );
        },
    );
}

// ---------------------------------------------------------------------------
// Zone interiors ARE analyzed for alias snapshots (expr facts are suppressed
// so the EXODIA region/JIT compiler is unperturbed, but the alias graph and
// loop snapshots stay live). So aliasing established inside a zone is visible,
// and fresh distinct handles inside a zone can still be hoisted.
// ---------------------------------------------------------------------------

#[test]
fn zone_body_aliasing_is_visible() {
    with_alias_facts(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Inside a new zone called "Scratch":
    Let mutable b be a.
    Let mutable i be 0.
    While i is less than 3:
        Set item 1 of b to item 1 of a.
        Set i to i + 1.
    Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                !facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "b")),
                "`Let b be a.` inside a Zone aliases — must be visible to the snapshot"
            );
        },
    );
}

#[test]
fn zone_body_fresh_handles_are_distinct() {
    with_alias_facts(
        r#"## Main
Inside a new zone called "Scratch":
    Let mutable a be a new Seq of Int.
    Push 1 to a.
    Let mutable b be a new Seq of Int.
    Push 2 to b.
    Let mutable i be 0.
    While i is less than 3:
        Set item 1 of a to item 1 of b.
        Set i to i + 1.
    Show item 1 of a.
"#,
        |facts, stmts, interner| {
            let lp = nth_loop(stmts, 0);
            assert!(
                facts.loop_handles_definitely_distinct(lp, sym(interner, "a"), sym(interner, "b")),
                "two fresh Seqs inside a Zone must be provably distinct (snapshot is live)"
            );
        },
    );
}
