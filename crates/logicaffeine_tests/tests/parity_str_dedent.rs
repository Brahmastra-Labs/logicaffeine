//! Stdlib-parity — `dedentText` (Python textwrap.dedent): removes the longest common leading
//! whitespace from every line; whitespace-only lines are normalized to empty. Algorithm verified
//! against CPython (including the whitespace-only-line case). tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Strip the common leading-whitespace margin from each line. py: textwrap.dedent
#[test]
fn parity_str_dedent() {
    // dedent("  a\n    b\n")        = "a\n  b\n"   (len 6; common margin 2 spaces)
    // dedent("  hello\n  world")    = "hello\nworld"
    // dedent("    a\n      \n    b\n") = "a\n\nb\n" (len 5; blank whitespace line normalized)
    let src = "## Main\n\
        Let a be dedentText(\"  a\\n    b\\n\").\n\
        Show length of a.\n\
        Show dedentText(\"  hello\\n  world\").\n\
        Let c be dedentText(\"    a\\n      \\n    b\\n\").\n\
        Show length of c.\n";
    let expected = "6\nhello\nworld\n5";
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}
