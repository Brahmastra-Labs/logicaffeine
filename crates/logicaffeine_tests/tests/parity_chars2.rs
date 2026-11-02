#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
#[test]
fn parity_char_to_title() {
    assert_all_tiers(
        "## Main\nShow charToTitle(97).\nShow charToTitle(65).\n",
        "65\n65",
    );
}
#[test]
fn parity_char_is_title() {
    assert_all_tiers(
        "## Main\nShow charIsTitle(65).\nShow charIsTitle(97).\n",
        "true\nfalse",
    );
}
#[test]
fn parity_char_digit_val() {
    assert_all_tiers(
        "## Main\nShow charDigitVal(55).\nShow charDigitVal(97).\n",
        "7\n-1",
    );
}
#[test]
fn parity_str_contains_char() {
    assert_all_tiers(
        "## Main\nShow containsChar(\"hello\", 108).\nShow containsChar(\"hello\", 122).\n",
        "true\nfalse",
    );
}
#[test]
fn parity_str_index_char() {
    assert_all_tiers(
        "## Main\nShow indexChar(\"hello\", 108).\nShow indexChar(\"hello\", 122).\n",
        "3\n0",
    );
}
