//! A tuple holds a record, a variant, or a list of records on every tier, and gives each back as itself.
//!
//! The tree-walker and the VM build tuples dynamically, so any value rides one. The AOT-Rust tier's tuple
//! is a positional `Vec<Value>`, and `Value` carried only scalars and lists of scalars: a tuple holding a
//! `Point` did not build, and a record field typed `(Point, Point)` could not be stored. Each program
//! here must print the same on the tree-walker, the VM, AOT-Rust and C, and an element read back out
//! of the tuple must be usable as the type it went in as (`q's x`, `Inspect`), not only shown.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

const POINT: &str = "## A Point has:\n    An x: Int.\n\n";

const SHAPE: &str = "## A Shape is either:\n    A Circle with radius Int.\n    A Dot.\n\n";

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}

fn program(decls: &[&str], main: &str) -> String {
    format!("{}## Main\n{main}", decls.concat())
}

#[test]
fn a_tuple_of_two_records_shows_as_a_tuple() {
    on_every_tier(
        &program(&[POINT], "Let a be a new Point with x 1.\nLet b be a new Point with x 5.\nLet t be (a, b).\nShow t.\n"),
        "(Point { x: 1 }, Point { x: 5 })",
    );
}

#[test]
fn a_record_read_out_of_a_tuple_is_the_record() {
    on_every_tier(
        &program(
            &[POINT],
            "Let a be a new Point with x 1.\nLet b be a new Point with x 5.\nLet t be (a, b).\nLet q be item 2 of t.\nShow q.\nShow q's x.\n",
        ),
        "Point { x: 5 }\n5",
    );
}

#[test]
fn a_tuple_mixes_a_record_with_scalars() {
    on_every_tier(
        &program(
            &[POINT],
            "Let a be a new Point with x 1.\nLet t be (a, 7, \"seven\").\nShow t.\nLet p be item 1 of t.\nShow p's x.\nShow item 2 of t.\nShow item 3 of t.\n",
        ),
        "(Point { x: 1 }, 7, seven)\n1\n7\nseven",
    );
}

#[test]
fn a_function_returns_a_pair_of_records() {
    on_every_tier(
        &program(
            &[POINT, "## To pair (n: Int) -> (Point, Point):\n    Let a be a new Point with x n.\n    Let b be a new Point with x n * 2.\n    Return (a, b).\n\n"],
            "Let (p, q) be pair(3).\nShow p's x.\nShow q's x.\nShow pair(4).\n",
        ),
        "3\n6\n(Point { x: 4 }, Point { x: 8 })",
    );
}

#[test]
fn a_function_takes_a_pair_of_records() {
    on_every_tier(
        &program(
            &[POINT, "## To width (s: (Point, Point)) -> Int:\n    Let (a, b) be s.\n    Return b's x - a's x.\n\n"],
            "Let a be a new Point with x 2.\nLet b be a new Point with x 9.\nShow width((a, b)).\n",
        ),
        "7",
    );
}

#[test]
fn a_record_field_holds_a_pair_of_records_and_gives_them_back() {
    on_every_tier(
        &program(
            &[POINT, "## A Segment has:\n    An ends: (Point, Point).\n\n"],
            "Let a be a new Point with x 1.\nLet b be a new Point with x 5.\nLet s be a new Segment with ends (a, b).\nShow s.\nLet e be item 2 of s's ends.\nShow e's x.\n",
        ),
        "Segment { ends: (Point { x: 1 }, Point { x: 5 }) }\n5",
    );
}

#[test]
fn a_tuple_holds_a_variant_and_inspects_it_back() {
    on_every_tier(
        &program(
            &[SHAPE],
            "Let c be a new Circle with radius 2.\nLet t be (c, 1).\nShow t.\nLet s be item 1 of t.\nInspect s:\n    When Circle (r): Show r.\n    When Dot: Show \"dot\".\n",
        ),
        "(Circle(2), 1)\n2",
    );
}

#[test]
fn a_tuple_holds_a_list_of_records() {
    on_every_tier(
        &program(
            &[POINT],
            "Let ps be [a new Point with x 1, a new Point with x 2].\nLet t be (ps, 2).\nShow t.\nLet back be item 1 of t.\nShow length of back.\nShow item 2 of back.\n",
        ),
        "([Point { x: 1 }, Point { x: 2 }], 2)\n2\nPoint { x: 2 }",
    );
}

#[test]
fn a_list_holds_tuples_of_records() {
    on_every_tier(
        &program(
            &[POINT],
            "Let a be a new Point with x 1.\nLet b be a new Point with x 5.\nLet ts be [(a, 1), (b, 2)].\nShow ts.\nRepeat for t in ts:\n    Let p be item 1 of t.\n    Show p's x.\n",
        ),
        "[(Point { x: 1 }, 1), (Point { x: 5 }, 2)]\n1\n5",
    );
}
