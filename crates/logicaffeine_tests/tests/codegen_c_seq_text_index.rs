//! Regression lock: indexing a `Seq of Text` RETURNED BY A CALL, on the C backend.
//!
//! `infer_expr_type(Expr::Index)` in `codegen_c/types.rs` used to only resolve the element type
//! when the collection was a bare variable — `item N of split(s)` (collection is a call) defaulted
//! to `Int64`, so the emitter printed the element `char*` POINTER as an integer via `show_i64`
//! (e.g. `101427751170992` instead of the string). The fix infers the collection's own C type for
//! non-identifier collections; this pins it byte-for-byte through gcc.

#![allow(unused_imports, dead_code)]
mod common;
use common::assert_c_output;

/// `item N` of a user function returning `Seq of Text` must show the string, not its pointer.
#[test]
fn index_of_seq_text_returning_call_on_c() {
    assert_c_output(
        "## To pair () -> Seq of Text:\n    Let out be a new Seq of Text.\n    Let cur be a new Seq of Int.\n    Push 97 to cur.\n    Push 98 to cur.\n    Push text_from_bytes(cur) to out.\n    Push \"zz\" to out.\n    Return out.\n## Main\nShow item 1 of pair().\nShow item 2 of pair().\n",
        "ab\nzz",
    );
}
