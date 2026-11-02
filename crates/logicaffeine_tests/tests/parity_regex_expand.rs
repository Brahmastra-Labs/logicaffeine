//! Stdlib-parity — `regexExpandString` (Go regexp.ExpandString / Expand): expand a template against
//! a match's capture groups — `$0` whole match, `$N`/`${N}` group N, `$$` literal `$`. Minimal
//! dialect + flat groups. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Expand a template with group references. go: regexp.ExpandString / Expand
#[test]
fn parity_regex_expand() {
    let src = "## Main\n\
        Show regexExpandString(\"(a)(b)\", \"zabw\", \"$2-$1\").\n\
        Show regexExpandString(\"a(.)c\", \"xabcy\", \"[$1]\").\n\
        Show regexExpandString(\"(a)(b)\", \"zabw\", \"${1}${2}\").\n\
        Show regexExpandString(\"(a)\", \"a\", \"$$x\").\n";
    assert_all_tiers(src, "b-a\n[b]\nab\n$x");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "b-a\n[b]\nab\n$x");
}
