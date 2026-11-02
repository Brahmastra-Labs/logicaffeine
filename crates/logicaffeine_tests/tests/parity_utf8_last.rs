//! Stdlib-parity — the rest of `unicode/utf8`: decodeLastRune (last-rune decode, back-scan) +
//! fullRune (complete-encoding check). Byte-identical across all tiers (tw/VM + AOT-Rust + C).
//! Values authoritative (Go utf8 semantics; an invalid lead is a width-1 error rune).
//! (appendRune is implemented in strings.lg but cataloged separately once its C-tier is verified.)
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

const LAST: &str = "## Main
Show decodeLastRune(\"héllo\").
Show decodeLastRune(\"café\").
Show decodeLastRune(\"€\").
Show decodeLastRune(\"A😀\").
";
const LAST_EXPECT: &str = "(111, 1)\n(233, 2)\n(8364, 3)\n(128512, 4)";

const FULL: &str = "## Main
Show fullRune([104]).
Show fullRune([195]).
Show fullRune([195, 169]).
Show fullRune([128]).
Show fullRune([226, 130]).
Show fullRune([226, 130, 172]).
";
const FULL_EXPECT: &str = "true\nfalse\ntrue\ntrue\nfalse\ntrue";

#[test]
fn parity_utf8_decode_last_rune() {
    assert_all_tiers(LAST, LAST_EXPECT);
}

#[test]
fn parity_utf8_full_rune() {
    assert_all_tiers(FULL, FULL_EXPECT);
}

#[test]
fn decode_last_rune_on_native_tiers() {
    assert_compiled_equals_interpreted(LAST);
    assert_c_output(LAST, LAST_EXPECT);
}

#[test]
fn full_rune_on_c() {
    assert_c_output(FULL, FULL_EXPECT);
}

const APPEND: &str = "## Main
Show appendRune([104], 233).
Let mutable e be a new Seq of Int.
Show appendRune(e, 65).
";
const APPEND_EXPECT: &str = "[104, 195, 169]\n[65]";

#[test]
fn parity_utf8_append_rune() {
    assert_all_tiers(APPEND, APPEND_EXPECT);
}

#[test]
fn append_rune_on_c() {
    assert_c_output(APPEND, APPEND_EXPECT);
}
