//! Wave 6 keystone (parser tier): bare `is <number>` is EQUALITY — `x is 5` ≡ `x is equal to 5`
//! ≡ `x == 5`. Pure parser sugar (→ `BinaryOpKind::Eq`), no new AST node, all engines free. The
//! addition is guarded to a NUMBER literal after `is`, so every other `is …` form — `is not`,
//! `is even`/`odd`, `is at most`, `is less than`, `is between`, `is approximately`, `is equal to`,
//! `is divisible by` — is untouched.

mod common;
use common::assert_compiled_equals_interpreted_eq as eq;

#[test]
fn is_number_is_equality_true() {
    eq(
        "## Main\nLet x be 5.\nIf x is 5:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "yes",
    );
}

#[test]
fn is_number_is_equality_false() {
    eq(
        "## Main\nLet x be 4.\nIf x is 5:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "no",
    );
}

#[test]
fn is_zero() {
    eq(
        "## Main\nLet x be 0.\nIf x is 0:\n    Show \"zero\".\nOtherwise:\n    Show \"nonzero\".\n",
        "zero",
    );
}

#[test]
fn is_negative_number() {
    eq(
        "## Main\nLet x be 0 - 3.\nIf x is -3:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "yes",
    );
}

#[test]
fn is_float_number() {
    eq(
        "## Main\nLet x be 2.5.\nIf x is 2.5:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "yes",
    );
}

#[test]
fn is_number_in_expression_position() {
    // `x is 5` binds as a Bool value, not only in an `If` head.
    eq(
        "## Main\nLet x be 5.\nLet b be x is 5.\nIf b:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "yes",
    );
}

#[test]
fn is_number_matches_is_equal_to() {
    // The bare form is exactly the verbose spelling.
    for (a, b) in [(7, 7), (7, 8)] {
        let bare = format!(
            "## Main\nLet x be {a}.\nIf x is {b}:\n    Show \"eq\".\nOtherwise:\n    Show \"ne\".\n"
        );
        let verbose = format!(
            "## Main\nLet x be {a}.\nIf x is equal to {b}:\n    Show \"eq\".\nOtherwise:\n    Show \"ne\".\n"
        );
        let expected = if a == b { "eq" } else { "ne" };
        eq(&bare, expected);
        eq(&verbose, expected);
    }
}

// ---------------------------------------------------------------------------
// Guards: the number-literal rule must NOT disturb any other `is …` form.
// ---------------------------------------------------------------------------

#[test]
fn guard_is_not_number_is_still_inequality() {
    eq(
        "## Main\nLet x be 4.\nIf x is not 5:\n    Show \"ne\".\nOtherwise:\n    Show \"eq\".\n",
        "ne",
    );
}

#[test]
fn guard_is_even_untouched() {
    eq(
        "## Main\nLet x be 4.\nIf x is even:\n    Show \"even\".\nOtherwise:\n    Show \"odd\".\n",
        "even",
    );
}

#[test]
fn guard_is_at_most_number_untouched() {
    eq(
        "## Main\nLet x be 5.\nIf x is at most 10:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "yes",
    );
}

#[test]
fn guard_is_less_than_number_untouched() {
    eq(
        "## Main\nLet x be 5.\nIf x is less than 3:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "no",
    );
}

#[test]
fn guard_is_between_untouched() {
    eq(
        "## Main\nLet x be 5.\nIf x is between 1 and 10:\n    Show \"in\".\nOtherwise:\n    Show \"out\".\n",
        "in",
    );
}

#[test]
fn guard_is_divisible_by_untouched() {
    eq(
        "## Main\nIf 12 is divisible by 3:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        "yes",
    );
}

#[test]
fn guard_is_approximately_untouched() {
    eq(
        "## Main\nLet x be 0.1 + 0.2.\nIf x is approximately 0.3:\n    Show \"close\".\nOtherwise:\n    Show \"far\".\n",
        "close",
    );
}
