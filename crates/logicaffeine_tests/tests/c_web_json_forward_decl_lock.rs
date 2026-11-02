//! C codegen: a program merely TOUCHING `web.lg` (never calling anything JSON-shaped) drags
//! `json.lg`'s types into the program's type registry too, since `web.lg` demand-imports it for
//! `postJson`/`putJson`/…. `codegen_c` used to emit every STRUCT first, then every ENUM
//! (`codegen_c_struct_defs` followed by `codegen_c_enum_defs`), with no ordering between the two
//! kinds — so `JEntry`/`ParseState` (structs holding a `Json` field BY VALUE) always landed before
//! `Json` (an enum) was declared at all: `error: unknown type name 'Json'`. A single topological
//! sort over both kinds is not enough by itself: `JEntry` needs `Json` complete (`val: Json`) but
//! `Json` also mentions `JEntry` (`JObj(entries: Seq of JEntry)`), a genuine mutual reference that
//! only resolves once a `Seq of X` is recognised as needing `X` merely FORWARD-DECLARED, never
//! complete. The fix (`codegen_c/mod.rs`) forward-declares every struct/enum, emits every struct's
//! `Seq_<Name>` container typedef off just that forward declaration, THEN topologically orders the
//! real by-value bodies, and only then emits the `Seq_<Name>` family functions that copy by value —
//! see `type_value_deps`/`ordered_type_defs`/`codegen_c_type_defs`'s doc comments for the 4 phases.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_c_output;

/// The minimal repro: `charAtText` never touches JSON, but importing `web.lg` for it pulls in
/// `json.lg`'s `Json`/`JsonResult`/`JEntry`/`ParseState` family, which must still order and compile.
#[test]
fn a_program_touching_web_lg_compiles_and_runs_on_c() {
    assert_c_output(
        "## Main\nShow charAtText(\"abc\", 1).\nShow charAtText(\"abc\", 3).\n",
        "a\nc",
    );
}

/// `hexByteText`'s own body (`charAtText(...) + charAtText(...)`) also runs on the C tier — the
/// declaration-order fix and the AOT-Rust concat-borrow fix are independent bugs, so this test
/// pins the C side of the exact repro `web/lgx-ease/stdlib.md` §3 recorded.
#[test]
fn hex_byte_text_runs_on_c_after_the_forward_declaration_fix() {
    assert_c_output(
        "## Main\nShow hexByteText(0).\nShow hexByteText(255).\n",
        "00\nFF",
    );
}
