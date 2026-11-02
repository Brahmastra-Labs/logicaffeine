//! A record field typed as a tuple is laid out as that tuple on every tier.
//!
//! Discovery read `A p: (Int, Int).` as its first element — `(` then one type then `)` — and the
//! registry's field type had no tuple form, so `p` was registered as an `Int`. The tree-walker and
//! the VM never consult that layout (they build records dynamically) and print `Pair { p: (1, 2) }`;
//! every compiled tier laid the field out as an integer and could not store the tuple at all. Each
//! program must print the same on the tree-walker, the VM, AOT-Rust and C.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}

#[test]
fn a_record_holds_a_pair_of_ints() {
    on_every_tier(
        "## A Pair has:\n    A p: (Int, Int).\n\n## Main\nLet x be a new Pair with p (1, 2).\nShow x.\nShow item 1 of x's p.\n",
        "Pair { p: (1, 2) }\n1",
    );
}

/// A tuple of records, on the tree-walker, the VM and C. The AOT-Rust tier cannot run it yet — for
/// any tuple, not only a field: its tuple is a positional `Vec<Value>`, and `Value` has no record
/// form (`Value: From<Point>` does not hold). That gap is tracked on its own; this row pins the field layout on every tier that has one.
#[test]
fn a_record_holds_a_pair_of_records() {
    let src = "## A Point has:\n    An x: Int.\n\n## A Segment has:\n    An ends: (Point, Point).\n\n## Main\nLet a be a new Point with x 1.\nLet b be a new Point with x 5.\nLet s be a new Segment with ends (a, b).\nShow s.\n";
    let expected = "Segment { ends: (Point { x: 1 }, Point { x: 5 }) }";
    assert_all_tiers(src, expected);
    assert_c_output(src, expected);
}
