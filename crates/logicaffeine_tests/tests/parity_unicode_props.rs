//! Stdlib-parity — Unicode property lookups from the UCD: `unicodedataMirrored`
//! (unicodedata.mirrored), `unicodedataCombining` (unicodedata.combining), `unicodedataDecimal`
//! (unicodedata.decimal), `unicodedataDigit` (unicodedata.digit). Verified vs CPython. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// 1 if a codepoint's glyph is bidi-mirrored, else 0. py: unicodedata.mirrored
#[test]
fn parity_unicode_mirrored() {
    let src = "## Main\nShow unicodedataMirrored(41).\nShow unicodedataMirrored(40).\nShow unicodedataMirrored(65).\n";
    assert_all_tiers(src, "1\n1\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "1\n1\n0");
}

/// Canonical combining class (0 if none). py: unicodedata.combining
#[test]
fn parity_unicode_combining() {
    let src = "## Main\nShow unicodedataCombining(768).\nShow unicodedataCombining(65).\n";
    assert_all_tiers(src, "230\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "230\n0");
}

/// Decimal value of a decimal-digit codepoint, else -1. py: unicodedata.decimal
#[test]
fn parity_unicode_decimal() {
    // '0'=0, Arabic-Indic '٠'(0x660)=0, superscript '²'(0xB2) is NOT a decimal -> -1
    let src = "## Main\nShow unicodedataDecimal(48).\nShow unicodedataDecimal(1632).\nShow unicodedataDecimal(178).\n";
    assert_all_tiers(src, "0\n0\n-1");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "0\n0\n-1");
}

/// Digit value of a digit codepoint, else -1. py: unicodedata.digit
#[test]
fn parity_unicode_digit() {
    // '0'=0, superscript '²'(0xB2)=2 (digit but not decimal), 'A'=-1
    let src = "## Main\nShow unicodedataDigit(48).\nShow unicodedataDigit(178).\nShow unicodedataDigit(65).\n";
    assert_all_tiers(src, "0\n2\n-1");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "0\n2\n-1");
}
