//! Stdlib-parity — UTF-8 codec: decodeRune (first-rune decode → (codepoint, size)) + encodeRune
//! (codepoint → bytes). Go `unicode/utf8.DecodeRune`/`DecodeRuneInString`/`EncodeRune`.
//! Authoritative values from Python UTF-8 encode/ord. Byte-identical all tiers.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

const DECODE: &str = "## Main
Show decodeRune(\"hello\").
Show decodeRune(\"éllo\").
Show decodeRune(\"€\").
Show decodeRune(\"😀\").
";
const DECODE_EXPECT: &str = "(104, 1)\n(233, 2)\n(8364, 3)\n(128512, 4)";

const ENCODE: &str = "## Main
Show encodeRune(65).
Show encodeRune(233).
Show encodeRune(8364).
Show encodeRune(128512).
";
const ENCODE_EXPECT: &str = "[65]\n[195, 169]\n[226, 130, 172]\n[240, 159, 152, 128]";

#[test]
fn parity_utf8_decode_rune() {
    assert_all_tiers(DECODE, DECODE_EXPECT);
}

#[test]
fn parity_utf8_encode_rune() {
    assert_all_tiers(ENCODE, ENCODE_EXPECT);
}

#[test]
fn utf8_codec_on_native_tiers() {
    assert_compiled_equals_interpreted(DECODE);
    assert_c_output(DECODE, DECODE_EXPECT);
    assert_c_output(ENCODE, ENCODE_EXPECT);
}
