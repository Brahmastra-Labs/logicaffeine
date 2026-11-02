//! ★ LOCK — the AOT-C tier carries a user type's sequences and every exact integer's slot as the
//! tree-walker does. ★
//!
//! `json.lg` is the program that needs all of it at once: a `Seq of Json` (a data-carrying user enum)
//! built empty, an element of a `Seq of JEntry` (a user struct) replaced in place, an integer
//! accumulator (`m = m * 10 + digit`) that the C tier carries as the exact tagged `LInt` stored into an
//! `Int` field of an enum variant, and an `LInt` handed to an `Int` parameter (`jsonUtf8Bytes(cp)`).
//! Each shape is held here on its own, beyond `i64` wherever an integer is involved, so a C program is
//! exact where the tree-walker is exact — not merely compilable where the values happen to be small.
//!
//! The tree-walker is the oracle: every program's expected output is pinned against it first, then the
//! C tier must print exactly that.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::assert_c_output;
use logicaffeine_compile::compile::tw_outcome;

fn oracle(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert!(
        tw.error.is_none(),
        "the tree-walker oracle failed: {:?}\n{src}",
        tw.error
    );
    assert_eq!(
        tw.output.trim(),
        expected,
        "the tree-walker oracle moved:\n{src}"
    );
}

fn both(src: &str, expected: &str) {
    oracle(src, expected);
    assert_c_output(src, expected);
}

#[test]
fn an_empty_seq_of_a_user_enum_is_built_by_its_own_family() {
    both(
        "## A Shape is either:\n    A Circle (r: Int).\n    A Square (s: Int).\n\n\
         ## Main\n\
         Let shapes be a new Seq of Shape.\n\
         Push a new Circle with r 3 to shapes.\n\
         Push a new Square with s 4 to shapes.\n\
         Show length of shapes.\n\
         Show item 2 of shapes.\n",
        "2\nSquare(4)",
    );
}

#[test]
fn set_item_replaces_an_element_of_a_seq_of_structs() {
    both(
        "## A Pair has:\n    a key, which is Text.\n    a val, which is Int.\n\n\
         ## Main\n\
         Let ps be a new Seq of Pair.\n\
         Push a new Pair with key \"a\" and val 1 to ps.\n\
         Push a new Pair with key \"b\" and val 2 to ps.\n\
         Let q be a new Pair with key \"c\" and val 3.\n\
         Set item 1 of ps to q.\n\
         Let first be item 1 of ps.\n\
         Show first's key.\n\
         Show first's val.\n\
         Show length of ps.\n",
        "c\n3\n2",
    );
}

#[test]
fn set_item_writes_through_a_struct_fields_seq() {
    both(
        "## A Bag has:\n    an items, which is Seq of Int.\n\n\
         ## Main\n\
         Let b be a new Bag with items [1, 2, 3].\n\
         Set item 2 of b's items to 9.\n\
         Show b's items.\n",
        "[1, 9, 3]",
    );
}

/// `m` grows past `i64` (10^25), so the C tier carries it as `LInt`; the variant's `Int` field it fills
/// holds it exactly, read back through `Inspect` and through the whole-value `Show`.
#[test]
fn a_bignum_fills_an_int_field_of_an_enum_variant() {
    both(
        "## A Num is either:\n    A Big (n: Int).\n    A Small.\n\n\
         ## Main\n\
         Let mutable m be 1.\n\
         Let mutable i be 0.\n\
         While i is less than 25:\n    Set m to m * 10.\n    Set i to i + 1.\n\
         Let v be a new Big with n m.\n\
         Inspect v:\n    When Big (n):\n        Show n.\n    Otherwise:\n        Show \"small\".\n\
         Show v.\n\
         Let w be a new Big with n 12.\n\
         Show w.\n",
        "10000000000000000000000000\nBig(10000000000000000000000000)\nBig(12)",
    );
}

#[test]
fn a_bignum_fills_an_int_field_of_a_struct() {
    both(
        "## A Box has:\n    a label, which is Text.\n    a count, which is Int.\n\n\
         ## To grow (b: Box) -> Int:\n    Return b's count * 2.\n\n\
         ## Main\n\
         Let mutable m be 7.\n\
         Let mutable i be 0.\n\
         While i is less than 22:\n    Set m to m * 10.\n    Set i to i + 1.\n\
         Let bx be a new Box with label \"big\" and count m.\n\
         Show bx's count.\n\
         Show grow(bx).\n\
         Let small be a new Box with label \"small\" and count 4.\n\
         Show grow(small).\n",
        "70000000000000000000000\n140000000000000000000000\n8",
    );
}

/// A parameter declared `Int` takes whatever integer its callers hand it: a 26-digit value at one call,
/// a machine word at another.
#[test]
fn a_bignum_is_handed_to_an_int_parameter() {
    both(
        "## To describe (x: Int) -> Text:\n    Return \"value {x}\".\n\n\
         ## To twice (x: Int) -> Int:\n    Return x + x.\n\n\
         ## Main\n\
         Let mutable m be 1.\n\
         Let mutable i be 0.\n\
         While i is less than 25:\n    Set m to m * 10.\n    Set i to i + 1.\n\
         Show describe(m).\n\
         Show describe(5).\n\
         Show twice(m).\n\
         Show twice(21).\n\
         Show twice(1 + 2 ** 70).\n",
        "value 10000000000000000000000000\nvalue 5\n20000000000000000000000000\n42\n2361183241434822606850",
    );
}

/// An exact return flows on through a local and a parameter of other functions.
#[test]
fn a_bignum_flows_through_a_chain_of_calls() {
    both(
        "## To power10 (k: Int) -> Int:\n\
         \x20   Let mutable m be 1.\n    Let mutable i be 0.\n\
         \x20   While i is less than k:\n        Set m to m * 10.\n        Set i to i + 1.\n\
         \x20   Return m.\n\n\
         ## To viaLocal (k: Int) -> Int:\n    Let r be power10(k).\n    Return r + 1.\n\n\
         ## To viaParam (x: Int) -> Int:\n    Return x - 1.\n\n\
         ## Main\n\
         Show viaLocal(3).\n\
         Show viaLocal(30).\n\
         Let t be viaLocal(21).\n\
         Show viaParam(t).\n\
         Show viaParam(10).\n",
        "1001\n1000000000000000000000000000001\n1000000000000000000000\n9",
    );
}

/// A parameter made exact bounds a counted loop: the loop's counter is a machine word, and the bound
/// narrows into it.
#[test]
fn an_exact_value_bounds_a_counted_loop() {
    both(
        "## To countTo (n: Int) -> Int:\n\
         \x20   Let mutable s be 0.\n\
         \x20   Repeat for i from 1 to n:\n        Set s to s + i.\n\
         \x20   Return s.\n\n\
         ## Main\n\
         Let mutable m be 1.\n\
         Let mutable k be 0.\n\
         While k is less than 3:\n    Set m to m * 10.\n    Set k to k + 1.\n\
         Show countTo(m).\n\
         Show countTo(4).\n",
        "500500\n10",
    );
}

/// A closure is lowered in a context of its own; it sees the program's exact parameters and fields as the
/// function bodies do — a machine word handed to an exact parameter, or stored into an exact field.
#[test]
fn a_closure_meets_exact_parameters_and_fields() {
    both(
        "## A Box has:\n    a count, which is Int.\n\n\
         ## To twice (x: Int) -> Int:\n    Return x + x.\n\n\
         ## Main\n\
         Let mutable m be 1.\n\
         Let mutable k be 0.\n\
         While k is less than 25:\n    Set m to m * 10.\n    Set k to k + 1.\n\
         Show twice(m).\n\
         Let g be (y: Int) -> twice(y).\n\
         Show g(21).\n\
         Let big be a new Box with count m.\n\
         Show big's count.\n\
         Let mk be (c: Int) -> a new Box with count c.\n\
         Let small be mk(4).\n\
         Show small's count.\n",
        "20000000000000000000000000\n42\n10000000000000000000000000\n4",
    );
}

/// An exact value stored into a record's field inside a closure body makes that field exact.
#[test]
fn a_closure_stores_an_exact_value_into_a_field() {
    both(
        "## A Box has:\n    a count, which is Int.\n\n\
         ## Main\n\
         Let mk be (c: Int) -> a new Box with count (2 ** c).\n\
         Let b70 be mk(70).\n\
         Show b70's count.\n\
         Let b3 be mk(3).\n\
         Show b3's count.\n",
        "1180591620717411303424\n8",
    );
}

/// `json.lg` end to end on C: `\u` escapes (a surrogate pair, a lone surrogate), numbers at and past the
/// `i64` edge the module guards, a duplicate key, and a parse → serialize round trip through `JDec`.
#[test]
fn json_parses_and_serializes_on_c_as_on_the_tree_walker() {
    both(
        "## Main\n\
         Show jsonValid(\"\\\"caf\\\\u00e9\\\"\").\n\
         Show jsonValid(\"\\\"\\\\ud83d\\\\ude00\\\"\").\n\
         Show jsonValid(\"\\\"\\\\ud83d\\\"\").\n\
         Show jsonValid(\"-12.50e3\").\n\
         Show jsonValid(\"9223372036854775807\").\n\
         Show jsonValid(\"9223372036854775808\").\n\
         Show jsonValid(\"123456789012345678901234567890\").\n\
         Show jsonValid(\"{{\\\"a\\\": [1, 2.5, {{\\\"b\\\": null}}], \\\"a\\\": true}}\").\n\
         Show jsonSerialize(jsonParse(\"[-12.50e3, 7, 0.001, {{\\\"k\\\": \\\"v\\\\u00e9\\\"}}]\")).\n",
        "true\ntrue\nfalse\ntrue\ntrue\nfalse\nfalse\ntrue\n[-12500,7,0.001,{\"k\":\"vé\"}]",
    );
}
