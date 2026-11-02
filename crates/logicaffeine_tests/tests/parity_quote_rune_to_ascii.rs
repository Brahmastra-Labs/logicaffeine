//! Stdlib-parity — `quoteRuneToASCII` (Go strconv.QuoteRuneToASCII): a rune as a Go single-quoted,
//! ASCII-only character literal. Verified byte-for-byte against a Go-algorithm mimic. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Go single-quoted ASCII-only rune literal. go: strconv.QuoteRuneToASCII
#[test]
fn parity_quote_rune_to_ascii() {
    let src = "## Main\nShow quoteRuneToASCII(97).\nShow quoteRuneToASCII(10).\nShow quoteRuneToASCII(233).\nShow quoteRuneToASCII(39).\nShow quoteRuneToASCII(34).\nShow quoteRuneToASCII(128512).\n";
    assert_all_tiers(
        src,
        "'a'
'\\n'
'\\u00e9'
'\\''
'\"'
'\\U0001f600'",
    );
    assert_compiled_equals_interpreted(src);
    assert_c_output(
        src,
        "'a'
'\\n'
'\\u00e9'
'\\''
'\"'
'\\U0001f600'",
    );
}
