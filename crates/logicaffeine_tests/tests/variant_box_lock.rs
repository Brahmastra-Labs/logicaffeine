//! `Opt::VariantBox`: a recursive enum boxes each variant that holds itself, not each field.
//!
//! `A Node with left Tree and right Tree` used to compile to `Node { left: Box<Tree>, right: Box<Tree> }`: every
//! child a heap allocation of its own, a `Leaf` child included, so a tree of `n` leaves paid `2n - 2` allocations
//! and a pointer chase to reach every leaf. A variant that holds the enum itself now compiles to one box around all
//! its fields (`Node(Box<…>)` over a struct whose `left` and `right` are `Tree`s in place), and a variant with no
//! fields is a value, no allocation at all: the representation OCaml gives a datatype. A tree of `n` leaves pays
//! `n - 1` allocations, one per node. binary_trees ran 2.11x faster with it, 1.67x the speed of C.
//!
//! The representation is invisible to the program: every tier shows the same values, compares them the same way,
//! and copies them as values.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};
use logicaffeine_compile::optimization::decorate_source;

const TREE: &str = "## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To build (depth: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Return a new Node with left build(depth - 1) and right build(depth - 1).

## To count (t: Tree) -> Int:
    Inspect t:
        When Leaf: Return 1.
        When Node (l, r): Return 1 + count(l) + count(r).

## Main
Show count(build(10)).
Show build(2).
";

/// The Rust declaration of enum `name`: from `pub enum name` to its closing brace.
fn enum_decl<'r>(rust: &'r str, name: &str) -> &'r str {
    let at = rust
        .find(&format!("pub enum {name}"))
        .unwrap_or_else(|| panic!("no `pub enum {name}` in:\n{rust}"));
    let end = rust[at..].find("\n    }").map_or(rust.len(), |e| at + e);
    &rust[at..end]
}

#[test]
fn a_recursive_variant_holds_its_fields_in_one_box() {
    let rust = compile_to_rust(TREE).unwrap();
    let decl = enum_decl(&rust, "Tree");
    assert!(
        decl.contains("Node(Box<") && !decl.contains("Box<Tree>"),
        "`Node` is one box around its fields, not a box per field. Got:\n{decl}\n\n{rust}"
    );
    assert!(
        decl.contains("Leaf,"),
        "`Leaf` holds nothing and is a plain value. Got:\n{decl}"
    );
}

#[test]
fn turning_the_optimization_off_boxes_each_field() {
    let rust = compile_to_rust(&decorate_source(TREE, &["variantbox"])).unwrap();
    assert!(
        enum_decl(&rust, "Tree").contains("left: Box<Tree>"),
        "Got:\n{rust}"
    );
}

#[test]
fn a_tree_counts_and_shows_alike_on_every_tier() {
    let agreed = run_all_tiers(TREE);
    assert_eq!(
        (agreed.output.trim(), agreed.error.as_deref()),
        ("2047\nNode(Node(Leaf, Leaf), Node(Leaf, Leaf))", None)
    );
    assert_compiled_equals_interpreted(TREE);
}

#[test]
fn a_generic_tree_keeps_its_values() {
    assert_compiled_equals_interpreted(
        "## A Tree of [T] is either:\n    A Leaf.\n    A Node with value T and left Tree of T and right Tree of T.\n\n## To size of [T] (t: Tree of T) -> Int:\n    Inspect t:\n        When Leaf: Return 0.\n        When Node (v, l, r):\n            Return 1 + size(l) + size(r).\n\n## Main\nLet leaf be a new Leaf.\nLet t be a new Node with value \"a\" and left (a new Node with value \"b\" and left leaf and right leaf) and right (a new Node with value \"c\" and left leaf and right leaf).\nShow size(t).\nShow t.\n",
    );
}

#[test]
fn a_list_built_in_a_loop_moves_its_tail() {
    let source = "## A MyList is either:\n    An Empty.\n    A Cons with head Int and tail MyList.\n\n## To total (l: MyList) -> Int:\n    Inspect l:\n        When Empty: Return 0.\n        When Cons (h, t): Return h + total(t).\n\n## Main\nLet mutable l be a new Empty.\nLet mutable i be 1.\nWhile i is at most 25:\n    Set l to a new Cons with head i and tail l.\n    Set i to i + 1.\nShow total(l).\nInspect l:\n    When Empty: Show 0.\n    When Cons (h, t): Show h.\n";
    let rust = compile_to_rust(source).unwrap();
    assert!(
        enum_decl(&rust, "MyList").contains("Cons(Box<"),
        "Got:\n{rust}"
    );
    assert_compiled_equals_interpreted(source);
}

#[test]
fn an_expression_tree_compares_copies_and_shows_alike() {
    let source = "## An Expr is either:
    A Lit with value Int.
    A Neg with inner Expr.
    An Add with left Expr and right Expr.

## To eval (e: Expr) -> Int:
    Inspect e:
        When Lit (v): Return v.
        When Neg (x): Return 0 - eval(x).
        When Add (a, b): Return eval(a) + eval(b).

## Main
Let one be a new Lit with value 1.
Let e be a new Add with left (a new Neg with inner one) and right (a new Add with left one and right (a new Lit with value 5)).
Let mutable f be e.
Set f to a new Neg with inner f.
Show eval(e).
Show eval(f).
Show e.
Show f.
If e equals e:
    Show \"same\".
If e equals f:
    Show \"different trees compare equal\".
";
    let rust = compile_to_rust(source).unwrap();
    let decl = enum_decl(&rust, "Expr");
    assert!(
        decl.contains("Neg(Box<") && decl.contains("Add(Box<") && !decl.contains("Lit(Box<"),
        "the two variants that hold an `Expr` are boxed; `Lit` holds an Int in place. Got:\n{decl}"
    );
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    assert!(
        agreed.output.contains("same") && !agreed.output.contains("compare equal"),
        "{}",
        agreed.output
    );
    assert_compiled_equals_interpreted(source);
}

#[test]
fn a_variant_inside_a_list_and_a_record_keeps_its_shape() {
    assert_compiled_equals_interpreted(
        "## A Tree is either:\n    A Leaf.\n    A Node with left Tree and right Tree.\n\n## A Forest has:\n    A trees: Seq of Tree.\n    A favourite: Tree.\n\n## To leaves (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 1.\n        When Node (l, r): Return leaves(l) + leaves(r).\n\n## Main\nLet small be a new Node with left (a new Leaf) and right (a new Leaf).\nLet f be a new Forest with trees [small, a new Leaf, small] and favourite small.\nLet mutable total be 0.\nRepeat for t in f's trees:\n    Set total to total + leaves(t).\nShow total.\nShow leaves(f's favourite).\nShow f's favourite.\n",
    );
}

#[test]
fn the_binary_trees_benchmark_boxes_its_nodes_whole() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/programs/binary_trees/main.lg");
    let rust = compile_to_rust(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let decl = enum_decl(&rust, "Tree");
    assert!(
        decl.contains("Node(Box<") && !decl.contains("Box<Tree>"),
        "Got:\n{decl}"
    );
}
