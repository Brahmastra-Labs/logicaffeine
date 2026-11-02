//! Stdlib-parity — `regexFindIndex` (Go regexp.FindStringIndex/FindIndex) + `regexFindString`
//! (regexp.FindString/Find): the leftmost match's [start,end) byte range and substring, over the
//! minimal regex dialect (literal, `.`, `*`). tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Leftmost match [start,end), 1-based half-open, matching the rest of the language's indexing
/// (Go's `regexp.FindStringIndex` is 0-based half-open; this is that shape shifted by the same
/// uniform +1 the whole stdlib's positions carry). ("b","abc")=[2,3]; ("a.c","xabcy")=[2,5];
/// ("z","abc")=[] (empty)
#[test]
fn parity_regex_find_index() {
    let src = "## Main\n\
        Show item 1 of regexFindIndex(\"b\", \"abc\").\n\
        Show item 2 of regexFindIndex(\"b\", \"abc\").\n\
        Show item 1 of regexFindIndex(\"a.c\", \"xabcy\").\n\
        Show item 2 of regexFindIndex(\"a.c\", \"xabcy\").\n\
        Show length of regexFindIndex(\"z\", \"abc\").\n";
    assert_all_tiers(src, "2\n3\n2\n5\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "2\n3\n2\n5\n0");
}

/// Leftmost matched substring. go: regexp.FindString / Find
#[test]
fn parity_regex_find_string() {
    // last line uses length (no-match returns "", whose empty Show line would be trimmed away):
    let src = "## Main\n\
        Show regexFindString(\"a.c\", \"xabcy\").\n\
        Show regexFindString(\"b\", \"abc\").\n\
        Show length of text_bytes(regexFindString(\"z\", \"abc\")).\n";
    assert_all_tiers(src, "abc\nb\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "abc\nb\n0");
}
