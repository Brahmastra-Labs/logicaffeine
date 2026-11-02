//! Stdlib-parity — UTF-8 primitives: runeLen/validRune/runeStart/validUtf8 (Go `unicode/utf8`).
//! validUtf8 is a full validator (lead/continuation ranges, overlong, surrogate, > U+10FFFF).
//! Expected values are AUTHORITATIVE (Python `bytes.decode('utf-8')` / rune arithmetic).
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

#[test]
fn parity_utf8_rune_len() {
    // 'A'=1, 'é'(233)=2, '€'(8364)=3, '😀'(128512)=4
    assert_all_tiers(
        "## Main\nShow runeLen(65).\nShow runeLen(233).\nShow runeLen(8364).\nShow runeLen(128512).\n",
        "1\n2\n3\n4",
    );
}

#[test]
fn parity_utf8_valid_rune() {
    // 65 ok, U+10FFFF(1114111) ok, 1114112 too big, 55296 is a surrogate.
    assert_all_tiers(
        "## Main\nShow validRune(65).\nShow validRune(1114111).\nShow validRune(1114112).\nShow validRune(55296).\n",
        "true\ntrue\nfalse\nfalse",
    );
}

#[test]
fn parity_utf8_rune_start() {
    // ASCII start, 0x80 continuation, 0xC0 lead, 0xBF continuation.
    assert_all_tiers(
        "## Main\nShow runeStart(65).\nShow runeStart(128).\nShow runeStart(192).\nShow runeStart(191).\n",
        "true\nfalse\ntrue\nfalse",
    );
}

const VALID_PROG: &str = "## Main
Show validUtf8([104, 101]).
Show validUtf8([233]).
Show validUtf8([195, 169]).
Show validUtf8([128]).
Show validUtf8([237, 160, 128]).
Show validUtf8([193, 128]).
Show validUtf8([244, 143, 191, 191]).
Show validUtf8([244, 144, 128, 128]).
";
const VALID_EXPECT: &str = "true\nfalse\ntrue\nfalse\nfalse\nfalse\ntrue\nfalse";

#[test]
fn parity_utf8_valid() {
    // ASCII ok · lone lead 0xE9 bad · "é" ok · lone cont bad · surrogate D800 bad ·
    // overlong 0xC1 bad · U+10FFFF ok · past-max bad.
    assert_all_tiers(VALID_PROG, VALID_EXPECT);
}

#[test]
fn utf8_on_native_tiers() {
    assert_compiled_equals_interpreted(VALID_PROG);
    assert_c_output(VALID_PROG, VALID_EXPECT);
}
