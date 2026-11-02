//! IDENTIFIER FREEDOM over VALUE-LITERAL WORDS — a live binding wins.
//!
//! LOGOS keywords are contextual, never reserved: `Let <name> be …` (or a
//! function parameter) makes `<name>` a VARIABLE for the rest of its scope,
//! even when the word doubles as a value literal. The parser already held
//! this line for `infinity`/`nan` (the float word-literals defer to
//! `user_bound`); this file extends the same contract to the remaining
//! value-literal words a natural program wants as names:
//!
//!   - `none`  (the Option-None literal)
//!   - `some`  (the Option-Some constructor)
//!   - `nothing` / `empty` (the unit literal's two spellings)
//!
//! The UNSHADOWED readings must stay byte-identical: `Let x be none.` is
//! still the Option literal, `Return some n.` still constructs, and
//! `When Some (v):` / `When None:` arms still match — including in a program
//! whose OTHER function uses `none` as a variable.
//!
//! `true`/`false` stay canonical literal spellings (unshadowable): they are
//! reserved words in generated Rust, so admitting them as names would split
//! the tiers. `noon`/`midnight`/date words are lexed as value-literal TOKENS
//! and rejected in binding position outright — a loud parse error, not a
//! silent misread — so they are out of this file's scope.

mod common;
use common::{assert_all_tiers, assert_output};

// ============================================================================
// Shadowed readings: a live binding turns the literal word into a variable
// ============================================================================

#[test]
fn let_none_names_a_seq_variable() {
    assert_all_tiers(
        r#"## Main
Let none be a new Seq of Int.
Push 1 to none.
Show none.
"#,
        "[1]",
    );
}

#[test]
fn let_some_names_an_int_variable() {
    assert_all_tiers(
        r#"## Main
Let some be 5.
Show some + 1.
"#,
        "6",
    );
}

#[test]
fn fn_param_named_none_shadows_the_literal() {
    assert_all_tiers(
        r#"## To pick (none: Seq of Int) -> Int:
    Return length of none.

## Main
Let xs be a new Seq of Int.
Push 7 to xs.
Show pick(xs).
"#,
        "1",
    );
}

#[test]
fn fn_param_named_some_shadows_the_constructor() {
    assert_all_tiers(
        r#"## To bump (some: Int) -> Int:
    Return some + 1.

## Main
Show bump(7).
"#,
        "8",
    );
}

#[test]
fn let_nothing_names_a_variable() {
    assert_all_tiers(
        r#"## Main
Let nothing be 5.
Show nothing + 2.
"#,
        "7",
    );
}

#[test]
fn let_empty_names_a_variable() {
    assert_all_tiers(
        r#"## Main
Let empty be 3.
Show empty + 4.
"#,
        "7",
    );
}

// ============================================================================
// Unshadowed readings stay intact
// ============================================================================

#[test]
fn unshadowed_none_is_still_the_option_literal() {
    assert_all_tiers(
        r#"## Main
Let x be none.
Show x.
"#,
        "nothing",
    );
}

#[test]
fn unshadowed_some_still_constructs_options() {
    assert_all_tiers(
        r#"## To find (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Let result be find(5).
Show result.
"#,
        "5",
    );
}

#[test]
fn inspect_some_and_none_arms_still_match() {
    assert_all_tiers(
        r#"## To find (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Inspect find(3):
    When Some (v):
        Show v.
    When None:
        Show 0.
Inspect find(0 - 1):
    When Some (v):
        Show v.
    When None:
        Show 0.
"#,
        "3\n0",
    );
}

#[test]
fn unshadowed_nothing_is_still_the_unit_literal() {
    assert_all_tiers(
        r#"## Main
Let x be nothing.
Show "after".
"#,
        "after",
    );
}

// ============================================================================
// Both worlds in ONE program: variable `none` in one function, the Option
// literal in another — scopes do not bleed
// ============================================================================

#[test]
fn variable_none_and_option_none_coexist_across_functions() {
    assert_all_tiers(
        r#"## To useVar () -> Int:
    Let none be 5.
    Return none + 1.

## To useLit (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Show useVar().
Inspect useLit(3):
    When Some (v):
        Show v.
    When None:
        Show 0.
Inspect useLit(0 - 1):
    When Some (v):
        Show v.
    When None:
        Show 0.
"#,
        "6\n3\n0",
    );
}

// ============================================================================
// AOT (compiled Rust) agrees with the interpreter on the shadowed programs
// ============================================================================

#[test]
fn shadowed_none_compiles_native_and_agrees() {
    assert_output(
        r#"## Main
Let none be a new Seq of Int.
Push 1 to none.
Show none.
"#,
        "[1]",
    );
}

#[test]
fn both_worlds_compile_native_and_agree() {
    assert_output(
        r#"## To useVar () -> Int:
    Let none be 5.
    Return none + 1.

## To useLit (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Show useVar().
Show useLit(3).
Show useLit(0 - 1).
"#,
        "6\n3\nnothing",
    );
}

/// CLOSED (pre-existing, confirmed WITHOUT any shadowing involved, fixed
/// 2026-09-01): the AOT-Rust tier emitted `Some { ref v } =>` (struct-variant
/// syntax) for a `When Some (v):` arm over an `Option`-returning CALL result —
/// rustc E0769, because `Some` is a std TUPLE variant. The Inspect-arm codegen
/// (`codegen/stmt.rs`) now emits `Some(ref v)` for `Option` while `Result`
/// keeps its struct-variant form. Interp+VM agreed throughout and are locked
/// above (`variable_none_and_option_none_coexist_across_functions`); this test
/// now runs for real and pins the native tier to them.
#[test]
fn inspect_over_option_call_compiles_native() {
    assert_output(
        r#"## To useLit (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Inspect useLit(3):
    When Some (v):
        Show v.
    When None:
        Show 0.
"#,
        "3",
    );
}
