//! Stdlib-parity — `charAtText`: 1-based single-character read over a Text's UTF-8 bytes, matching
//! the language's native `item N of` addressing (`strings.lg`'s independently-1-based `charAt` is
//! the same shape). `hexByteText` is `charAtText`'s one in-file caller and must move in lockstep.
//! tw ≡ VM ≡ WASM ≡ AOT-Rust ≡ C on both tests — the two pre-existing, unrelated codegen gaps that
//! used to keep AOT-Rust/C off these tiers (a `Text + Text` concat-borrow bug, and a C
//! declaration-order bug for any program auto-importing `web.lg`) are fixed; see
//! `aot_text_concat_operand_matrix_lock.rs` / `c_web_json_forward_decl_lock.rs`.
//!
//! `parity_record_field_concat` locks the ONE row that matrix left open (`p's label + p's
//! label` — a Text-typed struct FIELD, self-concatenated, no call and no literal on the other
//! side of `+` to carry the "this is Text" signal past the concat-fold decision); the full class
//! behind it — every Text-producing expression shape, as a concat operand — is closed by
//! `aot_text_concat_shape_census_lock.rs`.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// The reported bug: a Text-typed struct FIELD concatenated with itself, `p's label + p's
/// label` — no call, no literal on either side of `+` to carry the "this is Text" signal past
/// the concat-fold decision (`is_string_concat` in `codegen/expr.rs`), which could only see the
/// field's type through `record_field_rust_type`, and that needs the type REGISTRY.
/// `codegen_show_stmt` built its `ExprCtx` via `ExprCtx::bare`, whose registry is always empty —
/// AOT-Rust lowered this to a raw `p.label + p.label`, and `String: Add<String>` wants `&str` on
/// the right (rustc E0308). See `aot_text_concat_shape_census_lock.rs` for the full class this
/// bug belongs to (every Text-producing expression shape, as a concat operand).
#[test]
fn parity_record_field_concat() {
    let src = "\
## A Parcel has:
    A label: Text.

## To showConcats (p: Parcel):
    Show p's label + p's label.

## Main
    Let p be a new Parcel with label \"Z\".
    showConcats(p).
";
    assert_all_tiers(src, "ZZ");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "ZZ");
}

/// 1-based character read, in bounds at both ends and past both ends.
#[test]
fn parity_char_at_text() {
    let src = "## Main\n\
        Show charAtText(\"abc\", 1).\n\
        Show charAtText(\"abc\", 2).\n\
        Show charAtText(\"abc\", 3).\n\
        Show length of charAtText(\"abc\", 0).\n\
        Show length of charAtText(\"abc\", 4).\n";
    assert_all_tiers(src, "a\nb\nc\n0\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "a\nb\nc\n0\n0");
}

/// `hexByteText`'s digit-table lookup shifts its nibble value into `charAtText`'s 1-based
/// contract; its own external behavior (a byte as two uppercase hex digits) is unchanged by the
/// base migration — this is the built-in regression oracle for "did the +1 shim land correctly."
/// Its own body is `charAtText(...) + charAtText(...)` — two Text-returning calls concatenated —
/// the exact shape `aot_text_concat_operand_matrix_lock.rs` locks at the unit level.
#[test]
fn parity_hex_byte_text() {
    let src = "## Main\n\
        Show hexByteText(0).\n\
        Show hexByteText(9).\n\
        Show hexByteText(171).\n\
        Show hexByteText(255).\n";
    assert_all_tiers(src, "00\n09\nAB\nFF");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "00\n09\nAB\nFF");
}
