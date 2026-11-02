//! LOCK — the supercompiler's store knows what each name holds where the program reads it.
//!
//! The supercompiler drives a program statement by statement, carrying a store of the values it knows.
//! A block is a scope: a branch taken for certain is driven on a copy of the store, and what it leaves
//! is the store it started from with what the branch assigned to the enclosing bindings — never a name
//! the branch declared. Here dead-branch elimination is off (`## No deadcode`), so a branch whose
//! condition is known reaches the supercompiler as the block it is; the default pipeline's rows are
//! optimizer_name_capture_lock's.
#![cfg(feature = "codegen")]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};
use logicaffeine_compile::optimization::decorate_source;

/// `src` with the passes named switched off.
fn without(src: &str, passes: &[&str]) -> String {
    decorate_source(src, passes)
}

/// The tree-walker's lines for `src`, and the compiled program's, which must be the same.
fn shown_compiled(src: &str) -> Vec<String> {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let run = common::run_logos(src);
    assert!(
        run.success,
        "it builds and runs as Rust:\n{src}\n{}",
        run.stderr
    );
    let (want, got): (Vec<String>, Vec<String>) = (
        tw.output.lines().map(str::to_string).collect(),
        run.stdout.lines().map(str::to_string).collect(),
    );
    assert_eq!(
        got, want,
        "the compiled program reads what the tree-walker reads:\n{src}"
    );
    got
}

#[test]
fn a_known_branchs_binding_ends_with_the_branch() {
    let src = without(
        "## To f -> Int:\n    Let x be 1.\n    If true:\n        Let x be 2.\n        Show x.\n    Return x.\n\n## Main\nShow f().\n",
        &["deadcode"],
    );
    assert_eq!(shown_compiled(&src), ["2", "1"]);
}

#[test]
fn a_known_false_branchs_otherwise_ends_its_binding_too() {
    let src = without(
        "## Main\nLet x be 1.\nIf false:\n    Show 0.\nOtherwise:\n    Let x be 2.\n    Show x.\nShow x.\n",
        &["deadcode"],
    );
    assert_eq!(shown_compiled(&src), ["2", "1"]);
}

/// What a certain branch assigns to an enclosing binding is known after it: the compiled program
/// carries the product the supercompiler worked out, and without the supercompiler it does not. The
/// other passes that fold a known value (`comptime`, `sccp`, `saturate`) are off too, so the product is
/// this store's alone.
#[test]
fn a_known_branch_hands_on_what_it_assigns() {
    let src = "## To f -> Int:\n    Let mutable total be 1.\n    If true:\n        Let step be 3.\n        Set total to 4.\n    \
               Return total * 1009.\n\n## Main\nShow f().\n";
    let alone = ["deadcode", "comptime", "sccp", "saturate"];
    let on = without(src, &alone);
    assert_eq!(shown_compiled(&on), ["4036"]);
    let folded = compile_to_rust(&on).expect("it compiles to Rust");
    assert!(
        folded.contains("4036"),
        "the supercompiler folds `total * 1009` after the branch:\n{folded}"
    );
    let off = compile_to_rust(&without(src, &[&alone[..], &["supercompile"]].concat()))
        .expect("it compiles to Rust");
    assert!(
        !off.contains("4036"),
        "the product is the supercompiler's work:\n{off}"
    );
}

/// A name a certain branch declares and then sets is the branch's own: the enclosing binding of that
/// spelling keeps its value.
#[test]
fn a_known_branch_keeps_what_it_sets_on_its_own_binding() {
    let src = without(
        "## To f -> Int:\n    Let x be 1.\n    If true:\n        Let mutable x be 2.\n        Set x to 3.\n        Show x.\n    \
         Return x.\n\n## Main\nShow f().\n",
        &["deadcode"],
    );
    assert_eq!(shown_compiled(&src), ["3", "1"]);
}

/// A loop's iterable runs before its first pass: the loop reads the program binding its callee wrote.
#[test]
fn a_loops_iterable_writes_what_its_callee_writes() {
    let src = "## To bump -> Seq of Int:\n    Set counter to counter + 1.\n    Return [counter].\n\n\
               ## Main\nLet mutable counter be 0.\nSet counter to 10.\nRepeat for x in bump():\n    Show counter.\n";
    assert_eq!(shown_compiled(src), ["11"]);
}

/// Two branches that each declare a name and set it alike still set only their own bindings: after
/// them the enclosing binding holds what it held.
#[test]
fn both_branches_keep_what_they_set_on_their_own_bindings() {
    let src = "## To f -> Int:\n    Let x be 1.\n    If length of args() is at least 1:\n        Let mutable x be 2.\n        Set x to 3.\n    \
               Otherwise:\n        Let mutable x be 2.\n        Set x to 3.\n    Return x.\n\n## Main\nShow f().\n";
    assert_eq!(shown_compiled(src), ["1"]);
}
