//! Stdlib-parity north-star corpus — `jsonValid` (Go `encoding/json.Valid`): reports whether the
//! text is ONE complete, well-formed JSON document (a value, then only whitespace). Lenient on
//! duplicate object keys, matching Go. Wraps `json.lg`'s `jsonParseDoc` (strict=0) and returns its
//! `ok` flag — pure LOGOS.
//!
//! TIER SCOPE — tw / VM / AOT-Rust / C. `jsonValid` was the FIRST json capability exercised on a native
//! tier, and it drove fixing `json.lg`'s AOT-Rust codegen: the whole `Json` recursive enum now lowers
//! (struct/enum-variant bignum-field widening for the `JNum`/`JDec` number fields, and value-semantics
//! cloning of the parser's `kname`/`vnode`/`en` bindings). `json.parse`/`json.serialize` share this
//! codegen and now compile on AOT-Rust too. On C the parser's number accumulator is the exact tagged
//! `LInt`, and so is every return, parameter and field it reaches, so `json.lg` builds there and answers
//! as the tree-walker does (`c_json_shapes_lock` holds each shape on its own).
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

const SRC: &str = "## Main\n\
    Show jsonValid(\"[1, 2, 3]\").\n\
    Show jsonValid(\"42\").\n\
    Show jsonValid(\"[1, [2, 3]]\").\n\
    Show jsonValid(\"true\").\n\
    Show jsonValid(\"[1, 2,\").\n\
    Show jsonValid(\"1 2\").\n\
    Show jsonValid(\"{{bad}}\").\n";
const EXPECTED: &str = "true\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse";

/// True iff the text is a complete, well-formed JSON document. go: encoding/json.Valid
#[test]
fn parity_json_valid() {
    assert_all_tiers(SRC, EXPECTED);
    assert_compiled_equals_interpreted(SRC);
}

/// The same documents on the C tier.
#[test]
fn parity_json_valid_c() {
    assert_c_output(SRC, EXPECTED);
}
