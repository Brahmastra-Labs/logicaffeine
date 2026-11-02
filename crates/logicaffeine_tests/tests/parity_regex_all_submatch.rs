//! Stdlib-parity — all-matches capture groups: `regexFindAllSubmatchIndex`
//! (Go regexp.FindAllStringSubmatchIndex/FindAllSubmatchIndex) + `regexFindAllSubmatch`
//! (FindAllStringSubmatch/FindAllSubmatch). Minimal dialect + flat groups. These return a nested
//! `Seq of Seq of Int` / `Seq of Seq of Text`. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// All matches with group index pairs, 1-based half-open (see `regexFindIndex`'s note).
/// "(a)" over "aba": matches at [1,2,1,2] and [3,4,3,4]
#[test]
fn parity_regex_find_all_submatch_index() {
    let src = "## Main\n\
        Let all be regexFindAllSubmatchIndex(\"(a)\", \"aba\").\n\
        Show length of all.\n\
        Show item 1 of item 1 of all. Show item 2 of item 1 of all.\n\
        Show item 1 of item 2 of all. Show item 2 of item 2 of all.\n\
        Show item 3 of item 2 of all.\n";
    assert_all_tiers(src, "2\n1\n2\n3\n4\n3");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "2\n1\n2\n3\n4\n3");
}

/// All matches with groups as substrings. go: regexp.FindAllStringSubmatch / FindAllSubmatch
#[test]
fn parity_regex_find_all_submatch() {
    // "a(.)" over "axaybz": matches "ax"(g="x"), "ay"(g="y")
    let src = "## Main\n\
        Let all be regexFindAllSubmatch(\"a(.)\", \"axaybz\").\n\
        Show length of all.\n\
        Show item 1 of item 1 of all. Show item 2 of item 1 of all.\n\
        Show item 1 of item 2 of all. Show item 2 of item 2 of all.\n";
    assert_all_tiers(src, "2\nax\nx\nay\ny");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "2\nax\nx\nay\ny");
}
