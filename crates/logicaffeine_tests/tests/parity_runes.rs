//! Stdlib-parity — `runesOf`: decode UTF-8 Text into its Unicode code points (Go `[]rune(s)` /
//! `strings.Runes`/`bytes.Runes`; Python `list(map(ord, s))`). Arithmetic UTF-8 decode of 1-/2-/3-/
//! 4-byte sequences. Expected code points are AUTHORITATIVE (Python `ord`). Byte-identical all tiers.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

const PROG: &str = "## Main
Show runesOf(\"hello\").
Show runesOf(\"héllo\").
Show runesOf(\"€\").
Show runesOf(\"A😀\").
";

/// h/l/o ASCII (1-byte), é (2-byte, 233), € (3-byte, 8364), 😀 (4-byte, U+1F600 = 128512).
#[test]
fn parity_runes_of() {
    assert_all_tiers(
        PROG,
        "[104, 101, 108, 108, 111]\n[104, 233, 108, 108, 111]\n[8364]\n[65, 128512]",
    );
}

#[test]
fn runes_of_on_aot_rust() {
    assert_compiled_equals_interpreted(PROG);
}

#[test]
fn runes_of_on_c() {
    assert_c_output(
        PROG,
        "[104, 101, 108, 108, 111]\n[104, 233, 108, 108, 111]\n[8364]\n[65, 128512]",
    );
}

/// Code-point count (runes, not bytes): "héllo" is 5 runes / 6 bytes, "A😀" is 2 runes / 5 bytes.
#[test]
fn parity_rune_count() {
    assert_all_tiers(
        "## Main\nShow runeCount(\"hello\").\nShow runeCount(\"héllo\").\nShow runeCount(\"€\").\nShow runeCount(\"A😀\").\n",
        "5\n5\n1\n2",
    );
}
