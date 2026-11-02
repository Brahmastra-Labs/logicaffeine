//! Stdlib-parity — `lastIndexFuncText` (go strings.LastIndexFunc) and `fieldsFuncText`
//! (go strings.FieldsFunc). Predicate-closure string functions, verified to lower to the native
//! tiers (tw ≡ VM ≡ AOT-Rust ≡ C) — closures compile native, as `trimLeftFuncText` established.
//! `fieldsFuncText` returns a `Seq of Text`, joined with `"|"` for a deterministic assertion.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// 1-based index of the LAST predicate-matching byte, 0 if none. go: strings.LastIndexFunc ('x'=120)
#[test]
fn parity_str_last_index_func() {
    let src = "## Main\n\
        Show lastIndexFuncText((c: Int) -> c is equal to 120, \"axbxc\").\n\
        Show lastIndexFuncText((c: Int) -> c is equal to 122, \"abc\").\n";
    assert_all_tiers(src, "4\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "4\n0");
}

/// Split around runs of predicate-matching bytes, non-empty fields. go: strings.FieldsFunc (','=44)
#[test]
fn parity_str_fields_func() {
    let src =
        "## Main\nShow join(fieldsFuncText((c: Int) -> c is equal to 44, \"a,,bb,c\"), \"|\").\n";
    assert_all_tiers(src, "a|bb|c");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "a|bb|c");
}
