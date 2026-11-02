//! Stdlib-parity — `regexReplaceAllFunc` (Go regexp.ReplaceAllStringFunc / ReplaceAllFunc): replace
//! each match with the result of a callback applied to the matched text. Minimal dialect. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Replace each match via a callback. go: regexp.ReplaceAllStringFunc / ReplaceAllFunc
#[test]
fn parity_regex_replace_all_func() {
    let src = "## Main\n\
        Show regexReplaceAllFunc(\"a\", \"banana\", (m: Text) -> upper(m)).\n\
        Show regexReplaceAllFunc(\"n\", \"banana\", (m: Text) -> \"X\").\n";
    assert_all_tiers(src, "bAnAnA\nbaXaXa");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "bAnAnA\nbaXaXa");
}
