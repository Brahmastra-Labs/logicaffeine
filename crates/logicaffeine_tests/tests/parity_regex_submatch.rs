//! Stdlib-parity — capture groups: `regexFindSubmatchIndex` (Go regexp.FindStringSubmatchIndex /
//! FindSubmatchIndex) + `regexFindStringSubmatch` (FindStringSubmatch / FindSubmatch), over the
//! minimal dialect (literal, `.`, `*`) plus flat `(...)` groups. Verified vs Go. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Leftmost match + group index pairs, 1-based half-open (see `regexFindIndex`'s note).
/// "a(.)c" @ "xabcy" = [2,5,3,4]; "(a)(b)" @ "zabw" = [2,4,2,3,3,4]; "(a*)b" @ "aaab" = [1,5,1,4]
#[test]
fn parity_regex_find_submatch_index() {
    let src = "## Main\n\
        Let r be regexFindSubmatchIndex(\"a.c\", \"xabcy\").\n\
        Let s be regexFindSubmatchIndex(\"(a)(b)\", \"zabw\").\n\
        Show length of s.\n\
        Show item 3 of s. Show item 4 of s. Show item 5 of s. Show item 6 of s.\n\
        Let u be regexFindSubmatchIndex(\"(a*)b\", \"aaab\").\n\
        Show item 3 of u. Show item 4 of u.\n\
        Show length of regexFindSubmatchIndex(\"z\", \"abc\").\n";
    assert_all_tiers(src, "6\n2\n3\n3\n4\n1\n4\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "6\n2\n3\n3\n4\n1\n4\n0");
}

/// Leftmost match + groups as substrings. go: regexp.FindStringSubmatch / FindSubmatch
#[test]
fn parity_regex_find_submatch() {
    let src = "## Main\n\
        Let r be regexFindStringSubmatch(\"a(.)c\", \"xabcy\").\n\
        Show item 1 of r. Show item 2 of r.\n\
        Let s be regexFindStringSubmatch(\"(a)(b)\", \"zabw\").\n\
        Show item 1 of s. Show item 2 of s. Show item 3 of s.\n\
        Let u be regexFindStringSubmatch(\"(a*)b\", \"aaab\").\n\
        Show item 1 of u. Show item 2 of u.\n";
    assert_all_tiers(src, "abc\nb\nab\na\nb\naaab\naaa");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "abc\nb\nab\na\nb\naaab\naaa");
}
