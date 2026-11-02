//! `Opt::FoldFusion`: a consumer of a datatype applied to what a builder builds of it is one function that never
//! builds it.
//!
//! binary_trees builds a tree of `2^(d+1) - 1` nodes with `bottomUpTree(d)` so that `check` can visit each node once
//! and count it, and does so for every tree it checks. The fused `check_of_bottomUpTree(d)` is `bottomUpTree`'s own
//! recursion with each construction replaced by `check`'s arm for that variant, `If d = 0: Return 1. Return
//! 1 + f(d - 1) + f(d - 1)`, which the compiler then computes in closed form. The program still prints exactly what it
//! printed, on every tier, for every depth. A consumer or builder outside the shape — an arm that drops a subtree, a
//! field that can fail, a tree used twice, a consumer that calls out — keeps building its tree.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};
use logicaffeine_compile::optimization::decorate_source;

/// The text of the generated function `name`, from its `fn` to the next top-level `fn`.
fn function<'r>(rust: &'r str, name: &str) -> Option<&'r str> {
    let at = rust.find(&format!("fn {name}("))?;
    let end = rust[at + 1..]
        .find("\nfn ")
        .map_or(rust.len(), |e| at + 1 + e);
    Some(&rust[at..end])
}

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// binary_trees at a depth given in the source instead of the command line.
fn binary_trees_at(n: i64) -> String {
    benchmark("binary_trees").replace(
        "Let n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

#[test]
fn checking_a_built_tree_builds_no_tree() {
    let rust = compile_to_rust(&benchmark("binary_trees")).unwrap();
    assert!(
        rust.contains("fn check_of_bottomUpTree("),
        "`check(bottomUpTree(d))` is one fused function. Got:\n{rust}"
    );
    let main = function(&rust, "_logos_main")
        .or_else(|| function(&rust, "main"))
        .unwrap();
    assert!(
        !main
            .replace("check_of_bottomUpTree(", "")
            .contains("bottomUpTree("),
        "Main builds no tree: the totals and the long-lived tree's check both fuse. Got:\n{main}"
    );
}

#[test]
fn turning_the_optimization_off_builds_the_trees() {
    let rust =
        compile_to_rust(&decorate_source(&benchmark("binary_trees"), &["foldfuse"])).unwrap();
    assert!(!rust.contains("check_of_bottomUpTree"), "Got:\n{rust}");
}

#[test]
fn binary_trees_prints_alike_at_every_depth() {
    for n in [0, 1, 4, 6, 7, 10] {
        let program = binary_trees_at(n);
        let agreed = run_all_tiers(&program);
        assert!(agreed.error.is_none(), "n = {n}: {:?}", agreed.error);
        common::assert_c_output(&program, agreed.output.trim());
        assert_compiled_equals_interpreted(&program);
        let off = decorate_source(&program, &["foldfuse"]);
        assert_eq!(
            run_all_tiers(&off).output,
            agreed.output,
            "n = {n}: fused and unfused print alike"
        );
    }
}

/// A tree that carries values: the consumer sums the payloads its builder computed.
const PAYLOAD: &str = "## A Tree is either:
    A Leaf with value Int.
    A Node with value Int and left Tree and right Tree.

## To build (d: Int) and (seed: Int) -> Tree:
    If d equals 0:
        Return a new Leaf with value seed * 3 + 1.
    Let s be seed * 7 + d.
    Return a new Node with value s - 2 and left build(d - 1, s) and right build(d - 1, s + 1).

## To total (t: Tree) -> Int:
    Inspect t:
        When Leaf (v): Return v.
        When Node (v, l, r): Return v + 2 * total(l) - total(r).

## Main
Show total(build(0, 5)).
Show total(build(5, 9)).
Let kept be build(12, 1).
Show \"kept\".
Show total(kept).
";

#[test]
fn a_tree_of_values_sums_without_being_built() {
    let rust = compile_to_rust(PAYLOAD).unwrap();
    assert!(rust.contains("fn total_of_build("), "Got:\n{rust}");
    let agreed = run_all_tiers(PAYLOAD);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    common::assert_c_output(PAYLOAD, agreed.output.trim());
    assert_compiled_equals_interpreted(PAYLOAD);
    assert_eq!(
        run_all_tiers(&decorate_source(PAYLOAD, &["foldfuse"])).output,
        agreed.output
    );
}

const TREE: &str = "## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To build (d: Int) -> Tree:
    If d equals 0:
        Return a new Leaf.
    Return a new Node with left build(d - 1) and right build(d - 1).
";

/// Programs whose consumer or builder falls outside the fusable shape, or whose tree is used twice.
fn unfusable() -> Vec<(&'static str, String)> {
    vec![
        (
            "an arm that drops a subtree",
            format!("{TREE}\n## To leftmost (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 1.\n        When Node (l, r): Return 1 + leftmost(l).\n\n## Main\nShow leftmost(build(8)).\n"),
        ),
        (
            "a builder field that can fail",
            "## A Tree is either:\n    A Leaf.\n    A Node with left Tree and right Tree.\n\n## To build (d: Int) and (k: Int) -> Tree:\n    If d equals 0:\n        Return a new Leaf.\n    Return a new Node with left build(d - 1, k) and right build((d * k) / k - 1, k).\n\n## To size (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 1.\n        When Node (l, r): Return 1 + size(l) + size(r).\n\n## Main\nShow size(build(6, 2)).\n".to_string(),
        ),
        (
            "a tree used twice",
            format!("{TREE}\n## To size (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 1.\n        When Node (l, r): Return 1 + size(l) + size(r).\n\n## Main\nLet t be build(7).\nShow size(t).\nShow size(t).\n"),
        ),
        (
            "a consumer that calls another function",
            format!("{TREE}\n## To twice (n: Int) -> Int:\n    Return n * 2.\n\n## To size (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 1.\n        When Node (l, r): Return twice(size(l)) + size(r).\n\n## Main\nShow size(build(7)).\n"),
        ),
    ]
}

#[test]
fn a_consumer_or_builder_outside_the_shape_builds_its_tree_and_answers_alike() {
    for (why, program) in unfusable() {
        let rust = compile_to_rust(&program).unwrap_or_else(|e| panic!("{why}: {e:?}"));
        assert!(
            !rust.contains("_of_build"),
            "{why}: must not fuse. Got:\n{rust}"
        );
        let agreed = run_all_tiers(&program);
        assert!(agreed.error.is_none(), "{why}: {:?}", agreed.error);
        assert_compiled_equals_interpreted(&program);
    }
}

/// `## No foldfuse` on either function keeps that pair apart; the other pair still fuses.
#[test]
fn a_function_that_turns_it_off_keeps_its_tree() {
    let sizes = "## To size (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 1.\n        When Node (l, r): Return 1 + size(l) + size(r).\n\n## To leaves (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 1.\n        When Node (l, r): Return leaves(l) + leaves(r).\n";
    let main = "\n## Main\nShow size(build(9)).\nShow leaves(build(9)).\n";
    let consumer_off = format!(
        "{TREE}\n{}{main}",
        sizes.replacen("## To size", "## No foldfuse\n## To size", 1)
    );
    let rust = compile_to_rust(&consumer_off).unwrap();
    assert!(
        !rust.contains("size_of_build"),
        "size opted out. Got:\n{rust}"
    );
    assert!(
        rust.contains("fn leaves_of_build("),
        "leaves did not. Got:\n{rust}"
    );
    let builder_off = format!(
        "{}\n{sizes}{main}",
        TREE.replacen("## To build", "## No foldfuse\n## To build", 1)
    );
    let rust = compile_to_rust(&builder_off).unwrap();
    assert!(
        !rust.contains("_of_build"),
        "the builder opted out. Got:\n{rust}"
    );
    for program in [&consumer_off, &builder_off] {
        let agreed = run_all_tiers(program);
        assert_eq!(
            (agreed.output.trim(), agreed.error.as_deref()),
            ("1023\n512", None)
        );
        assert_compiled_equals_interpreted(program);
    }
}

/// A builder whose field divides by a parameter fails on zero: unfused, the failure comes while building, before
/// the consumer runs; the program must fail identically on every tier.
const FAILING_BUILD: &str = "## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To build (d: Int) and (k: Int) -> Tree:
    If d equals 0:
        Return a new Leaf.
    Return a new Node with left build(d - 1, k) and right build(d / k - 1, k).

## To size (t: Tree) -> Int:
    Inspect t:
        When Leaf: Return 1.
        When Node (l, r): Return 1 + size(l) + size(r).

## Main
Show \"before\".
Show size(build(3, 0)).
";

#[test]
fn a_builder_that_fails_fails_alike() {
    let rust = compile_to_rust(FAILING_BUILD).unwrap();
    assert!(
        !rust.contains("size_of_build"),
        "the builder divides: it does not fuse. Got:\n{rust}"
    );
    let agreed = run_all_tiers(FAILING_BUILD);
    assert_eq!(agreed.output.trim(), "before");
    assert!(agreed.error.is_some(), "a division by zero while building");
    assert_compiled_equals_interpreted(FAILING_BUILD);
}
