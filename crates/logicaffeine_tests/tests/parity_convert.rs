//! Stdlib-parity north-star corpus — domain: convert
//!
//! Auto-scaffolded from benchmarks/parity/catalog/superset.json by
//! scripts/bench/gen-parity-corpus.py. Each test mirrors one deduplicated capability of the
//! C/Python/Rust/Go (+Java) superset. `#[ignore]` = parity-pending (not yet implemented);
//! the tree stays green. To LAND a capability: implement it, replace the body with a real
//! `assert_all_tiers` behavioral spec, DELETE the `#[ignore]` line, flip the catalog entry
//! to `status: green`, and bump GREEN_FLOOR in parity_catalog_lock.rs.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Unicode code point of a character.
/// provenance — python: builtins.ord
/// LOGOS surface: `the code of c`
#[test]
fn parity_convert_char_code() {
    assert_all_tiers(
        "## Main
Show ord(\"A\").
Show ord(\"z\").
",
        "65
122",
    );
}

/// Parse a floating-point number from text.
/// provenance — c: stdlib.h.strtod · python: builtins.float · go: strconv.ParseFloat · java: java.lang.Double.parseDouble
/// LOGOS surface: `s parsed as Real`
/// Convert an integer to a Real. The `* 10.0` and real division distinguish a true
/// `Real` (7/2 = 3.5 -> 35) from accidental integer division (7/2 = 3 -> 30), so a toReal
/// that fails to yield a Real -- or fails to type-check at all -- reddens here. This test
/// exists because a `toReal` that did not type-check (`n * 1.0` on a declared Int param)
/// shipped undetected: NO test referenced toReal, and the stdlib is demand-compiled, so an
/// uncalled function is never checked. Referencing it here forces the check on every tier.
#[test]
fn parity_convert_to_real() {
    assert_all_tiers(
        "## Main
Show trunc(toReal(7) / toReal(2) * 10.0).
",
        "35",
    );
}

#[test]
fn parity_convert_parse_float() {
    assert_all_tiers(
        "## Main
Show floor(parseFloat(\"3.7\")).
",
        "3",
    );
}

/// Parse an integer from text, failing on garbage.
/// provenance — c: stdlib.h.strtol · python: builtins.int · rust: str.parse · go: strconv.Atoi · java: java.lang.Integer.parseInt
/// LOGOS surface: `s parsed as Int`
#[test]
fn parity_convert_parse_int() {
    assert_all_tiers(
        "## Main
Show parseInt(\"42\") + parseInt(\"8\").
",
        "50",
    );
}

/// Render a value as its text form.
/// provenance — python: builtins.str · go: strconv.Itoa · java: java.lang.String.valueOf
/// LOGOS surface: `the text of x`
#[test]
fn parity_convert_to_string() {
    assert_all_tiers(
        r#"## Main
Show numText(42) combined with "!".
"#,
        "42!",
    );
}

/// Integer to binary string. py: bin · java: Integer.toBinaryString
#[test]
fn parity_convert_to_binary() {
    assert_all_tiers("## Main\nShow toBinary(5).\nShow toBinary(0).\n", "101\n0");
}
/// Integer to hex string. py: (format x) · java: Integer.toHexString
#[test]
fn parity_convert_to_hex_int() {
    assert_all_tiers("## Main\nShow toHexInt(255).\n", "ff");
}
/// Integer to octal string. java: Integer.toOctalString
#[test]
fn parity_convert_to_oct() {
    assert_all_tiers("## Main\nShow toOct(8).\n", "10");
}

/// Character from a code point (built-in `chr`). py: builtins.chr
#[test]
fn parity_convert_chr() {
    assert_all_tiers("## Main\nShow chr(65).\nShow chr(97).\n", "A\na");
}

/// Integer to a string in an arbitrary base. go: strconv.FormatInt
#[test]
fn parity_convert_to_base() {
    assert_all_tiers(
        "## Main\nShow toBase(255, 16).\nShow toBase(10, 2).\nShow toBase(100, 8).\n",
        "ff\n1010\n144",
    );
}

/// Format a boolean as its canonical text. go: strconv.FormatBool
#[test]
fn parity_convert_format_bool() {
    assert_all_tiers(
        "## Main\nShow formatBool(true).\nShow formatBool(false).\n",
        "true\nfalse",
    );
}

/// Parse the canonical truthy spellings; everything else is false. go: strconv.ParseBool
#[test]
fn parity_convert_parse_bool() {
    assert_all_tiers(
        "## Main\nShow parseBool(\"true\").\nShow parseBool(\"T\").\nShow parseBool(\"1\").\nShow parseBool(\"false\").\nShow parseBool(\"nope\").\n",
        "true\ntrue\ntrue\nfalse\nfalse",
    );
}

/// Wrap a string in double quotes, escaping quote/backslash/newline/tab. go: strconv.Quote
#[test]
fn parity_convert_quote() {
    assert_all_tiers(
        "## Main\nShow quoteText(\"hi\").\nShow quoteText(\"a\\\"b\").\n",
        "\"hi\"\n\"a\\\"b\"",
    );
}

/// Whether a string can be a raw backquoted literal (no backquote/DEL/control except tab). go: strconv.CanBackquote
#[test]
fn parity_convert_can_backquote() {
    assert_all_tiers(
        "## Main\nShow canBackquote(\"hello world\").\nShow canBackquote(\"a\\nb\").\n",
        "true\nfalse",
    );
}

/// Append the decimal of an int to a prefix. go: strconv.AppendInt
#[test]
fn parity_convert_append_int() {
    assert_all_tiers("## Main\nShow appendInt(\"x=\", 42).\n", "x=42");
}

/// Append the decimal of a non-negative int. go: strconv.AppendUint
#[test]
fn parity_convert_append_uint() {
    assert_all_tiers("## Main\nShow appendUint(\"n\", 7).\n", "n7");
}

/// Append a boolean's text form. go: strconv.AppendBool
#[test]
fn parity_convert_append_bool() {
    assert_all_tiers(
        "## Main\nShow appendBool(\"b:\", true).\nShow appendBool(\"b:\", false).\n",
        "b:true\nb:false",
    );
}

/// Append a double-quoted, escaped string. go: strconv.AppendQuote
#[test]
fn parity_convert_append_quote() {
    assert_all_tiers(
        "## Main\nShow appendQuote(\"val=\", \"hi\").\n",
        "val=\"hi\"",
    );
}

/// A single-quoted rune literal. go: strconv.QuoteRune
#[test]
fn parity_convert_quote_rune() {
    assert_all_tiers("## Main\nShow quoteRune(65).\n", "'A'");
}

/// Append a single-quoted rune literal to a prefix. go: strconv.AppendQuoteRune
#[test]
fn parity_convert_append_quote_rune() {
    assert_all_tiers("## Main\nShow appendQuoteRune(\"c=\", 65).\n", "c='A'");
}

/// `toBase` writes the digits `0`-`9` and then the letters: a base outside 2 to 36 has no digits to write a number in,
/// and every tier refuses it in the same words, after printing what came before (base 1 looped forever).
#[test]
fn parity_convert_to_base_refuses_a_base_without_digits() {
    for base in ["1", "0", "37", "0 - 2", "9223372036854775807"] {
        let src = format!("## Main\nShow toBase(255, 16).\nShow toBase(255, {base}).\n");
        let agreed = common::run_all_tiers(&src);
        assert_eq!(agreed.output.trim(), "ff", "\n{src}");
        let why = agreed.error.expect("a base without digits is refused");
        assert!(why.contains("toBase(): the base must be 2 to 36"), "{why}\n{src}");
    }
    assert_all_tiers("## Main\nShow toBase(35, 36).\nShow toBase(5, 2).\nShow toBase(255, 16).\n", "z\n101\nff");
}
