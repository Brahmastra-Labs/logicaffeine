//! Stdlib-parity — `regexMatchStart` (Python re.match, anchored at start) and `regexFullMatch`
//! (Python re.fullmatch, anchored at both ends), over the minimal regex dialect (literal, `.`, `*`).
//! Built on the existing `reMatchEnd`. Verified against CPython `re`. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Match anchored at the start of the text. py: re.match
#[test]
fn parity_regex_match_start() {
    let src = "## Main\n\
        Show regexMatchStart(\"ab\", \"abc\").\n\
        Show regexMatchStart(\"bc\", \"abc\").\n\
        Show regexMatchStart(\"a.c\", \"abc\").\n";
    assert_all_tiers(src, "true\nfalse\ntrue");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "true\nfalse\ntrue");
}

/// Match the entire text (anchored both ends). py: re.fullmatch
#[test]
fn parity_regex_full_match() {
    let src = "## Main\n\
        Show regexFullMatch(\"abc\", \"abc\").\n\
        Show regexFullMatch(\"ab\", \"abc\").\n\
        Show regexFullMatch(\"a*\", \"aaa\").\n\
        Show regexFullMatch(\"a.c\", \"abc\").\n";
    assert_all_tiers(src, "true\nfalse\ntrue\ntrue");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "true\nfalse\ntrue\ntrue");
}
