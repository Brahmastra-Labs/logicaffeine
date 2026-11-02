//! A function that only looks at a recursive value borrows it.
//!
//! `check (tree: Tree) -> Int` inspects its tree and hands the subtrees to itself; it never keeps, returns or changes
//! one. It lowers to `fn check(tree: &Tree)`: the `Inspect` binds each boxed subtree as a borrow and the recursive
//! calls pass those borrows on, so walking a tree copies nothing. Taken by value, every `Inspect` arm cloned both
//! subtrees and the recursion cloned them again — O(N · depth) copying, which made the Benchmarks Game binary-trees
//! program 40–50× slower through Rust than in C.
//!
//! The borrow is only taken where every mention of the parameter is the scrutinee of an `Inspect` or an argument
//! another borrowing parameter takes (a fixpoint over the call graph), and an arm binding stays a borrow only on the
//! same terms: a value a program returns, stores or passes to an owning parameter is still an owned copy. Each
//! program prints on the compiled tier exactly what it prints on the VM and the tree-walker.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test enum_param_borrow_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_all_tiers, assert_exact_output};
use logicaffeine_compile::compile::compile_to_rust;

const TREE: &str = "## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To build (depth: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Return a new Node with left build(depth - 1) and right build(depth - 1).
";

fn program(functions: &str, main: &str) -> String {
    format!("{TREE}\n{functions}\n## Main\n{main}")
}

/// The body of the generated Rust function `name`, from its signature to the next top-level item.
fn function<'r>(rust: &'r str, name: &str) -> &'r str {
    let start = rust
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no fn {name} in:\n{rust}"));
    let rest = &rust[start..];
    let end = rest.find("\n}\n").map(|i| i + 3).unwrap_or(rest.len());
    &rest[..end]
}

const CHECK: &str = "## To check (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Node (left, right): Return 1 + check(left) + check(right).
";

#[test]
fn a_tree_walk_borrows_the_tree_and_copies_nothing() {
    let src = program(
        CHECK,
        "Let t be build(10).\nShow check(t).\nShow check(build(3)).\nShow check(t).\n",
    );
    assert_all_tiers(&src, "2047\n15\n2047");
    assert_exact_output(&src, "2047\n15\n2047");
    let rust = compile_to_rust(&src).expect("compiles");
    let check = function(&rust, "check");
    assert!(
        check.starts_with("fn check(tree: &Tree)"),
        "check borrows its tree:\n{check}"
    );
    assert!(
        !check.contains("clone()"),
        "walking a borrowed tree copies nothing:\n{check}"
    );
    let main = function(&rust, "_logos_main");
    assert!(
        !main.contains("t.clone()"),
        "the caller lends its tree rather than copying it:\n{main}"
    );
}

#[test]
fn a_tree_the_function_returns_is_still_owned() {
    let src = program(
        "## To leftmost (tree: Tree) -> Tree:
    Inspect tree:
        When Leaf: Return tree.
        When Node (left, right): Return leftmost(left).

## To size (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Node (left, right): Return 1 + size(left) + size(right).
",
        "Let t be build(4).\nShow size(leftmost(t)).\nShow size(t).\n",
    );
    assert_all_tiers(&src, "1\n31");
    assert_exact_output(&src, "1\n31");
    let rust = compile_to_rust(&src).expect("compiles");
    assert!(
        function(&rust, "leftmost").starts_with("fn leftmost(tree: Tree)"),
        "a returned tree is owned:\n{rust}"
    );
    assert!(
        function(&rust, "size").starts_with("fn size(tree: &Tree)"),
        "a walked tree is borrowed:\n{rust}"
    );
}

#[test]
fn an_arm_that_keeps_a_subtree_copies_only_that_subtree() {
    let src = program(
        "## To size (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Node (left, right): Return 1 + size(left) + size(right).

## To rightHalf (tree: Tree) -> Tree:
    Inspect tree:
        When Leaf: Return a new Leaf.
        When Node (left, right):
            Let kept be right.
            Return kept.
",
        "Let t be build(5).\nShow size(rightHalf(t)).\nShow size(t).\n",
    );
    assert_all_tiers(&src, "31\n63");
    assert_exact_output(&src, "31\n63");
    let rust = compile_to_rust(&src).expect("compiles");
    let right_half = function(&rust, "rightHalf");
    assert!(right_half.contains("right"), "{right_half}");
    assert!(
        !right_half.contains("let left = (**left).clone()"),
        "an unused subtree is not copied:\n{right_half}"
    );
}

#[test]
fn nested_inspects_walk_borrowed_subtrees() {
    let src = program(
        "## To depthTwoLeaves (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 0.
        When Node (left, right):
            Inspect left:
                When Leaf: Return 1.
                When Node (ll, lr): Return depthTwoLeaves(ll) + depthTwoLeaves(lr) + depthTwoLeaves(right).
",
        "Show depthTwoLeaves(build(1)).\nShow depthTwoLeaves(build(4)).\n",
    );
    assert_all_tiers(&src, "1\n5");
    assert_exact_output(&src, "1\n5");
    let rust = compile_to_rust(&src).expect("compiles");
    let f = function(&rust, "depthTwoLeaves");
    assert!(f.starts_with("fn depthTwoLeaves(tree: &Tree)"), "{f}");
    assert!(!f.contains("clone()"), "a nested walk copies nothing:\n{f}");
}

#[test]
fn a_tree_passed_to_an_owning_function_is_not_borrowed() {
    let src = program(
        "## To keep (tree: Tree) -> Tree:
    Return tree.

## To viaKeep (tree: Tree) -> Int:
    Let k be keep(tree).
    Inspect k:
        When Leaf: Return 0.
        When Node (left, right): Return 1.
",
        "Show viaKeep(build(2)).\nShow viaKeep(build(0)).\n",
    );
    assert_all_tiers(&src, "1\n0");
    assert_exact_output(&src, "1\n0");
    let rust = compile_to_rust(&src).expect("compiles");
    assert!(
        function(&rust, "viaKeep").starts_with("fn viaKeep(tree: Tree)"),
        "{rust}"
    );
}
