//! A sequence literal COPIES its elements — LOGOS has value semantics, so `[a, a]` holds two copies
//! of `a` and `a` stays usable afterwards. The AOT-Rust tier lowers a literal to
//! `LogosSeq::from_vec(vec![…])`, which MOVES every element it names: a non-`Copy` variable (a
//! record, an enum, a Text, a collection) named twice in one literal, or read again after it, failed
//! to build (E0382 "use of moved value"). Each shape here must build and print exactly what the
//! interpreter prints.

mod common;
use common::assert_compiled_equals_interpreted;

#[test]
fn a_record_named_twice_in_one_literal() {
    assert_compiled_equals_interpreted(
        "## An Atom has:\n    A v: Int.\n\n## Main\nLet a be a new Atom with v 3.\nLet xs be [a, a].\nShow length of xs.\nShow xs.\n",
    );
}

#[test]
fn a_record_read_after_the_literal_holding_it() {
    assert_compiled_equals_interpreted(
        "## An Atom has:\n    A v: Int.\n\n## Main\nLet a be a new Atom with v 3.\nLet xs be [a].\nShow a.\nShow length of xs.\n",
    );
}

#[test]
fn a_sequence_named_twice_in_one_literal() {
    assert_compiled_equals_interpreted(
        "## Main\nLet xs be [1, 2].\nLet ys be [xs, xs].\nShow ys.\n",
    );
}

#[test]
fn a_sequence_read_after_the_literal_holding_it() {
    assert_compiled_equals_interpreted(
        "## Main\nLet mutable xs be [1, 2].\nPush 3 to xs.\nLet ys be [xs].\nShow xs.\nShow ys.\n",
    );
}

#[test]
fn a_computed_text_named_twice_in_one_literal() {
    assert_compiled_equals_interpreted(
        "## To greet (n: Text) -> Text:\n    Return \"hi \" + n.\n\n## Main\nLet s be greet(\"bo\").\nLet xs be [s, s].\nShow xs.\nShow s.\n",
    );
}

#[test]
fn an_enum_value_named_twice_in_one_literal() {
    assert_compiled_equals_interpreted(
        "## A Shape is either:\n    A Circle with radius Int.\n    A Dot.\n\n## Main\nLet c be a new Circle with radius 2.\nLet xs be [c, c].\nShow length of xs.\n",
    );
}

#[test]
fn a_literal_inside_a_function_keeps_its_parameter_usable() {
    assert_compiled_equals_interpreted(
        "## An Atom has:\n    A v: Int.\n\n## To pairOf (a: Atom) -> Int:\n    Let xs be [a, a].\n    Return length of xs + a's v.\n\n## Main\nShow pairOf(a new Atom with v 4).\n",
    );
}

#[test]
fn a_literal_of_records_shown_whole() {
    assert_compiled_equals_interpreted(
        "## A Zone has:\n    A atoms: Seq of Atom.\n    A name: Text.\n\n## An Atom has:\n    A v: Int.\n\n## Main\nLet a be a new Atom with v 3.\nLet z be a new Zone with atoms [a, a] and name \"z\".\nShow z.\nShow a.\n",
    );
}
