//! Stdlib-parity north-star corpus - domain: chars
//!
//! ASCII code-point classification and case-folding (chars.lg), the char-level counterpart of
//! the string predicates, proven byte-identical on every execution tier. Mirrors C's ctype.h
//! and Go's unicode rune predicates.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

#[test]
fn parity_chars_is_digit() {
    assert_all_tiers("## Main\nShow charIsDigit(53).\n", "true");
}

#[test]
fn parity_chars_is_digit_neg() {
    assert_all_tiers("## Main\nShow charIsDigit(65).\n", "false");
}

#[test]
fn parity_chars_is_upper() {
    assert_all_tiers("## Main\nShow charIsUpper(65).\n", "true");
}

#[test]
fn parity_chars_is_lower() {
    assert_all_tiers("## Main\nShow charIsLower(97).\n", "true");
}

#[test]
fn parity_chars_is_letter() {
    assert_all_tiers("## Main\nShow charIsLetter(90).\n", "true");
}

#[test]
fn parity_chars_is_alnum() {
    assert_all_tiers("## Main\nShow charIsAlnum(48).\n", "true");
}

#[test]
fn parity_chars_is_space() {
    assert_all_tiers("## Main\nShow charIsSpace(9).\n", "true");
}

#[test]
fn parity_chars_is_hex_digit() {
    assert_all_tiers("## Main\nShow charIsHexDigit(70).\n", "true");
}

#[test]
fn parity_chars_is_punct() {
    assert_all_tiers("## Main\nShow charIsPunct(64).\n", "true");
}

#[test]
fn parity_chars_to_upper() {
    assert_all_tiers("## Main\nShow charToUpper(97).\n", "65");
}

#[test]
fn parity_chars_to_lower() {
    assert_all_tiers("## Main\nShow charToLower(65).\n", "97");
}

#[test]
fn parity_chars_is_control() {
    assert_all_tiers(
        "## Main\nShow charIsControl(9).\nShow charIsControl(65).\n",
        "true\nfalse",
    );
}

#[test]
fn parity_chars_is_print() {
    assert_all_tiers(
        "## Main\nShow charIsPrint(32).\nShow charIsPrint(31).\n",
        "true\nfalse",
    );
}

#[test]
fn parity_chars_is_blank() {
    assert_all_tiers(
        "## Main\nShow charIsBlank(9).\nShow charIsBlank(65).\n",
        "true\nfalse",
    );
}

#[test]
fn parity_chars_is_graph() {
    assert_all_tiers(
        "## Main\nShow charIsGraph(33).\nShow charIsGraph(32).\n",
        "true\nfalse",
    );
}
