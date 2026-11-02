//! AOT-Rust: a variable named more than once in one enum-variant construction.
//!
//! The construction clones a variable at every mention but its last, which moves it — so a list
//! built by `Set l to a new Cons with head i and tail l` never deep-copies `l`. The count used to
//! span only one constructor's own fields: an inner `Node` taking `leaf` twice moved it at its
//! second mention, and the outer `Node` taking it again moved it a second time (E0382 "use of moved
//! value"). The count now spans the whole outermost construction. Each program must build and print
//! what the interpreter prints.

mod common;
use common::assert_compiled_equals_interpreted;

#[test]
fn a_variable_named_at_two_nesting_levels() {
    assert_compiled_equals_interpreted(
        "## A Tree is either:\n    A Leaf.\n    A Node with value Int and left Tree and right Tree.\n\n## To depth (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 0.\n        When Node (v, l, r):\n            Return 1 + max(depth(l), depth(r)).\n\n## Main\nLet leaf be a new Leaf.\nLet t be a new Node with value 1 and left (a new Node with value 2 and left leaf and right leaf) and right leaf.\nShow depth(t).\n",
    );
}

#[test]
fn a_generic_tree_naming_one_leaf_everywhere() {
    assert_compiled_equals_interpreted(
        "## A Tree of [T] is either:\n    A Leaf.\n    A Node with value T and left Tree of T and right Tree of T.\n\n## To size of [T] (t: Tree of T) -> Int:\n    Inspect t:\n        When Leaf: Return 0.\n        When Node (v, l, r):\n            Return 1 + size(l) + size(r).\n\n## Main\nLet leaf be a new Leaf.\nLet t be a new Node with value \"a\" and left (a new Node with value \"b\" and left leaf and right leaf) and right (a new Node with value \"c\" and left leaf and right leaf).\nShow size(t).\n",
    );
}

#[test]
fn a_list_built_in_a_loop_moves_its_tail() {
    assert_compiled_equals_interpreted(
        "## A MyList is either:\n    An Empty.\n    A Cons with head Int and tail MyList.\n\n## To total (l: MyList) -> Int:\n    Inspect l:\n        When Empty: Return 0.\n        When Cons (h, t): Return h + total(t).\n\n## Main\nLet mutable l be a new Empty.\nLet mutable i be 1.\nWhile i is at most 25:\n    Set l to a new Cons with head i and tail l.\n    Set i to i + 1.\nShow total(l).\n",
    );
}

#[test]
fn a_mention_inside_a_nested_list_then_a_direct_field() {
    assert_compiled_equals_interpreted(
        "## A Point has:\n    An x: Int.\n\n## A Shape is either:\n    A Dot.\n    A Poly with points Seq of Point and anchor Point.\n\n## Main\nLet p be a new Point with x 3.\nLet s be a new Poly with points [p, p] and anchor p.\nInspect s:\n    When Dot: Show 0.\n    When Poly (ps, a): Show length of ps + a's x.\n",
    );
}
