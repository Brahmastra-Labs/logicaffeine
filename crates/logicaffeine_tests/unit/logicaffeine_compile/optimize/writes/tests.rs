//! The shared write analysis, pinned at the unit level: every binder kind is a write, every
//! nested block is walked, a callee's undeclared assignment is a global write (scope-aware),
//! a closure body counts, foreign code is opaque.
use super::*;
use crate::ui_bridge::with_parsed_program;

fn with_program<R>(
    src: &str,
    f: impl for<'a> FnOnce(&'a [Stmt<'a>], &dyn Fn(&str) -> Symbol) -> R,
) -> R {
    with_parsed_program(src, |parsed, interner| match parsed {
        Ok((stmts, _, _)) => {
            let sym = |name: &str| {
                interner
                    .lookup(name)
                    .unwrap_or_else(|| panic!("`{name}` is not interned by this program"))
            };
            f(stmts, &sym)
        }
        Err(advice) => panic!("test program must parse: {advice}"),
    })
}

/// The `## Main` body of a parsed program (the statements after the function definitions).
fn main_body<'a>(stmts: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
    let first = stmts
        .iter()
        .position(|s| !matches!(s, Stmt::FunctionDef { .. } | Stmt::StructDef { .. }))
        .unwrap_or(stmts.len());
    &stmts[first..]
}

#[test]
fn a_callee_assigning_an_undeclared_name_writes_a_global() {
    let src = "## To bump:\n    Let step be 1.\n    Set counter to counter + step.\n## Main\n    Let mutable counter be 1.\n    bump().\n";
    with_program(src, |stmts, sym| {
        let globals = CalleeGlobals::of_program(stmts);
        assert!(
            globals.contains(&sym("counter")),
            "the callee's `Set counter` is a global write"
        );
        assert!(
            !globals.contains(&sym("step")),
            "the callee's own `Let step` is local"
        );
        assert!(!globals.opaque);
    });
}

#[test]
fn a_let_inside_a_branch_does_not_hide_a_later_assignment_of_the_same_name() {
    let src = "## To f (c: Bool):\n    If c is equal to true:\n        Let counter be 0.\n    Set counter to 5.\n## Main\n    Let mutable counter be 1.\n    f(true).\n";
    with_program(src, |stmts, sym| {
        let globals = CalleeGlobals::of_program(stmts);
        assert!(
            globals.contains(&sym("counter")),
            "the branch-scoped `Let` is gone when the `Set` runs: a global write"
        );
    });
}

#[test]
fn a_parameter_reassigned_in_the_callee_is_not_a_global() {
    let src =
        "## To f (n: Int) -> Int:\n    Set n to n + 1.\n    Return n.\n## Main\n    Show f(1).\n";
    with_program(src, |stmts, sym| {
        let globals = CalleeGlobals::of_program(stmts);
        assert!(!globals.contains(&sym("n")));
        assert!(globals.is_empty());
    });
}

#[test]
fn a_closure_body_assigning_an_outer_name_counts_as_a_callee_write() {
    let src = "## Main\n    Let mutable counter be 1.\n    Let bump be (e: Int) ->:\n        Set counter to counter + e.\n    bump(1).\n";
    with_program(src, |stmts, sym| {
        let globals = CalleeGlobals::of_program(stmts);
        assert!(
            globals.contains(&sym("counter")),
            "a closure's body runs at some call: its writes are callee writes"
        );
        assert!(
            !globals.contains(&sym("e")),
            "the closure's parameter is its own"
        );
    });
}

#[test]
fn a_swap_reassigns_both_names_and_declares_neither() {
    let src = "## Main\n    Let mutable x be 1.\n    Let mutable y be 2.\n    Swap x and y.\n";
    with_program(src, |stmts, sym| {
        let swap = WriteSet::of_stmt(&stmts[2]);
        let reassigned: HashSet<Symbol> = swap.reassigned().collect();
        assert_eq!(reassigned, [sym("x"), sym("y")].into_iter().collect());
        assert!(swap.may_write(sym("x")) && swap.may_write(sym("y")));
        let lets = WriteSet::of_block(&stmts[..2]);
        assert_eq!(
            lets.reassigned().count(),
            0,
            "a `Let` declares; it reassigns nothing"
        );
        assert!(lets.may_write(sym("x")));
    });
}

#[test]
fn a_write_inside_an_inspect_arm_is_a_write_of_the_inspect() {
    let src = "## A Shape is either:\n    A Circle.\n    A Square.\n## Main\n    Let mutable p be 1.\n    Let s be a new Square.\n    Inspect s:\n        When Circle:\n            Set p to 2.\n        When Square:\n            Set p to 3.\n";
    with_program(src, |stmts, sym| {
        let body = main_body(stmts);
        let inspect = body
            .iter()
            .find(|s| matches!(s, Stmt::Inspect { .. }))
            .expect("an Inspect");
        assert!(WriteSet::of_stmt(inspect).may_write(sym("p")));
    });
}

#[test]
fn a_push_through_a_field_mutates_the_root_object() {
    let src = "## A Bag has:\n    An items: Seq of Int.\n## Main\n    Let mutable b be a new Bag with items [1].\n    Push 5 to b's items.\n";
    with_program(src, |stmts, sym| {
        let push = main_body(stmts)
            .iter()
            .find(|s| matches!(s, Stmt::Push { .. }))
            .expect("a Push");
        let w = WriteSet::of_stmt(push);
        assert!(w.mutated().contains(&sym("b")));
        assert!(w.may_write(sym("b")));
    });
}

#[test]
fn a_call_statement_folds_the_callee_globals_in() {
    let src = "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let mutable counter be 1.\n    Let a be counter * 100.\n    bump().\n    Let b be counter * 100.\n";
    with_program(src, |stmts, sym| {
        let globals = CalleeGlobals::of_program(stmts);
        let body = main_body(stmts);
        let call = body
            .iter()
            .find(|s| matches!(s, Stmt::Call { .. }))
            .expect("a Call statement");
        let bare = WriteSet::of_stmt(call);
        assert!(bare.has_calls());
        assert!(
            !bare.may_write(sym("counter")),
            "without the program's globals a call is just a call"
        );
        let folded = WriteSet::of_stmt(call).with_callee_globals(&globals);
        assert!(folded.may_write(sym("counter")));
        let a = WriteSet::of_stmt(&body[1]);
        assert!(!a.has_calls() && !a.may_write(sym("counter")));
    });
}

#[test]
fn a_call_inside_a_let_value_is_a_call_of_the_let() {
    let src = "## To bumpAndGet -> Int:\n    Set counter to counter + 1.\n    Return counter.\n## Main\n    Let mutable counter be 1.\n    Let c be bumpAndGet().\n";
    with_program(src, |stmts, sym| {
        let globals = CalleeGlobals::of_program(stmts);
        let body = main_body(stmts);
        let w = WriteSet::of_stmt(&body[1]).with_callee_globals(&globals);
        assert!(w.has_calls());
        assert!(w.may_write(sym("counter")));
        assert!(w.may_write(sym("c")));
    });
}

#[test]
fn a_loop_body_write_set_covers_the_pattern_and_the_nested_blocks() {
    let src = "## Main\n    Let xs be [1, 2, 3].\n    Let mutable total be 0.\n    Let mutable p be 1.\n    Repeat for x in xs:\n        If x is greater than 1:\n            Set p to p + 1.\n        Set total to total + x.\n";
    with_program(src, |stmts, sym| {
        let body = main_body(stmts);
        let loop_stmt = body
            .iter()
            .find(|s| matches!(s, Stmt::Repeat { .. }))
            .expect("a Repeat");
        let w = WriteSet::of_stmt(loop_stmt);
        assert!(
            w.may_write(sym("x")),
            "the pattern variable is bound each iteration"
        );
        assert!(
            w.may_write(sym("p")),
            "a write nested in an If inside the body"
        );
        assert!(w.may_write(sym("total")));
        assert!(!w.may_write(sym("xs")), "the iterable is only read");
    });
}

#[test]
fn count_writes_of_counts_every_binder_of_a_name() {
    let src = "## Main\n    Let mutable i be 0.\n    Let mutable j be 0.\n    Set i to i + 1.\n    If j is equal to 0:\n        Swap i and j.\n";
    with_program(src, |stmts, sym| {
        let body = main_body(stmts);
        let none = CalleeGlobals::default();
        assert_eq!(
            count_writes_of(body, sym("i"), &none),
            3,
            "Let + Set + Swap"
        );
        assert_eq!(count_writes_of(body, sym("j"), &none), 2, "Let + Swap");
    });
}

#[test]
fn count_writes_of_counts_a_call_when_a_callee_assigns_the_name() {
    let src = "## To bump:\n    Set i to i + 1.\n## Main\n    Let mutable i be 0.\n    While i is less than 3:\n        bump().\n";
    with_program(src, |stmts, sym| {
        let globals = CalleeGlobals::of_program(stmts);
        let body = main_body(stmts);
        let loop_stmt = body
            .iter()
            .find(|s| matches!(s, Stmt::While { .. }))
            .expect("a While");
        let Stmt::While {
            body: loop_body, ..
        } = loop_stmt
        else {
            unreachable!()
        };
        assert_eq!(
            count_writes_of(loop_body, sym("i"), &globals),
            1,
            "the call writes `i` once"
        );
        assert_eq!(
            count_writes_of(loop_body, sym("i"), &CalleeGlobals::default()),
            0
        );
    });
}

#[test]
fn rebound_names_flags_reassignments_and_double_declarations_only() {
    let src = "## Main\n    Let a be 1.\n    Let mutable b be 2.\n    Set b to 3.\n    If a is equal to 1:\n        Let c be 4.\n    Let c be 5.\n    Let d be 6.\n";
    with_program(src, |stmts, sym| {
        let rebound = rebound_names(main_body(stmts), &CalleeGlobals::default());
        assert!(!rebound.contains(&sym("a")));
        assert!(rebound.contains(&sym("b")), "reassigned");
        assert!(rebound.contains(&sym("c")), "declared twice");
        assert!(!rebound.contains(&sym("d")));
    });
}
