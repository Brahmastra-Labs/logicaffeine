//! Wave 3a (AOT counterpart, task #16): numeric-unified map keys. A `Float`
//! used against an `Int`-keyed map coerces to its Int (`1 == 1.0`), exactly as
//! the interpreter does — the compiled path must agree, not fail to compile.

mod common;
use common::assert_compiled_equals_interpreted_eq as eq;

#[test]
fn int_key_int_contains() {
    eq(
        "## Main\nLet m be {1: \"a\", 2: \"b\"}.\nIf m contains 2:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "yes",
    );
}

#[test]
fn int_key_integral_float_contains_hits() {
    // 2.0 coerces to the Int key 2.
    eq(
        "## Main\nLet m be {1: \"a\", 2: \"b\"}.\nIf m contains 2.0:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "yes",
    );
}

#[test]
fn int_key_nonintegral_float_contains_misses() {
    // 2.5 is not an integer, so it matches no Int key.
    eq(
        "## Main\nLet m be {1: \"a\", 2: \"b\"}.\nIf m contains 2.5:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "no",
    );
}

#[test]
fn int_key_absent_float_contains_misses() {
    eq(
        "## Main\nLet m be {1: \"a\", 2: \"b\"}.\nIf m contains 9.0:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "no",
    );
}
