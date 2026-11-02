//! THE CHECKER REJECTS CALLS TO UNDEFINED FUNCTIONS — at check time, in every
//! position, definition-independently.
//!
//! Audit finding (2026-09-01): `largo check` admitted a call to `readText` — a
//! function that exists NOWHERE (not a user fn, not a builtin, not a prelude
//! native) — in statement position, expression position, inside `Show`, and
//! inside never-taken branches. The failure surfaced only at native build as
//! rustc E0425 ("this is a LOGOS compiler bug"), or at runtime on interp/VM.
//!
//! The contract locked here:
//!   REJECT at check/compile time (`compile_to_rust`, the exact seam
//!   `largo check`'s `compile_project` shares), with the language's
//!   established error text `Unknown function: <name>` — even when the call
//!   sits in a branch that would never execute (definition-independence).
//!
//!   ACCEPT, unchanged: builtins (`upper`, `sqrt`, `mapOf`, …), prelude
//!   natives spliced by `apply_prelude` (`read`, `write`, `randomInt`,
//!   `readBytes`, …), user functions defined LATER in the file (forward
//!   references), enum variant constructors (`a new JOk with …` and bare
//!   variants — those are `NewVariant`, never `Call`), and closure-valued
//!   variables called by name (`Let f be (x: Int) -> x. Show f(42).`).
//!
//! Interp/VM keep their runtime behavior (`Unknown function: <name>` at call
//! time) — locked below so the three tiers tell the user the same story.
//!
//! Residual gaps, deliberately out of this lock's scope:
//!   - `Show x to <recipient>` / `Give x to <recipient>` function-recipient
//!     dispatch (special recipients like `console` make the name space open).
//!   - The LSP's raw-source diagnostics pass (`check_program_collect` on the
//!     PRE-prelude AST) stays lenient — it cannot see the prelude; the deep
//!     flycheck (rustc translation) covers it.

mod common;
use common::{assert_all_tiers, run_all_tiers};
use logicaffeine_compile::compile::compile_to_rust;

/// The check layer (the in-memory `largo check` compile) must refuse `source`
/// with the canonical unknown-function message naming `name`.
fn assert_check_rejects(source: &str, name: &str) {
    match compile_to_rust(source) {
        Ok(_) => panic!(
            "check ADMITTED a call to the undefined function `{name}` — \
             it must be rejected at check time.\nSource:\n{source}"
        ),
        Err(e) => {
            let rendered = format!("{e:?}");
            let expected = format!("Unknown function: {name}");
            assert!(
                rendered.contains(&expected),
                "check rejected the program, but not with `{expected}`.\nGot: {rendered}\nSource:\n{source}"
            );
        }
    }
}

/// The check layer must accept `source` (compile to Rust without error).
fn assert_check_accepts(source: &str) {
    if let Err(e) = compile_to_rust(source) {
        panic!("check must accept this program, but it errored: {e:?}\nSource:\n{source}");
    }
}

// ============================================================================
// REJECT: every position, definition-independently
// ============================================================================

#[test]
fn check_rejects_unknown_call_in_statement_position() {
    assert_check_rejects(
        r#"## Main
Let t be 1.
readText("config.txt").
Show t.
"#,
        "readText",
    );
}

#[test]
fn check_rejects_unknown_call_in_expression_position() {
    assert_check_rejects(
        r#"## Main
Let t be readText("config.txt").
Show t.
"#,
        "readText",
    );
}

#[test]
fn check_rejects_unknown_call_inside_show() {
    assert_check_rejects(
        r#"## Main
Show readText("config.txt").
"#,
        "readText",
    );
}

#[test]
fn check_rejects_unknown_call_in_a_never_taken_branch() {
    // Definition-independence: the branch cannot be constant-folded away (the
    // condition reads argv) and never executes at runtime — the checker still
    // refuses, because the NAME does not exist anywhere in the program.
    assert_check_rejects(
        r#"## Main
Let xs be args().
If length of xs is greater than 99:
    Let t be readText("config.txt").
    Show t.
Show "done".
"#,
        "readText",
    );
}

#[test]
fn check_rejects_unknown_call_inside_a_closure_body() {
    assert_check_rejects(
        r#"## Main
Let f be (x: Int) -> readText("config.txt").
Show "made".
"#,
        "readText",
    );
}

#[test]
fn check_rejects_unknown_call_inside_a_function_body() {
    assert_check_rejects(
        r#"## To helper (n: Int) -> Int:
    Let t be readText("config.txt").
    Return n.

## Main
Show helper(1).
"#,
        "readText",
    );
}

#[test]
fn check_rejects_unknown_call_inside_string_interpolation() {
    assert_check_rejects(
        r#"## Main
Show "value: {readText(1)}".
"#,
        "readText",
    );
}

// ============================================================================
// ACCEPT: everything genuinely callable stays callable
// ============================================================================

#[test]
fn builtins_stay_accepted() {
    assert_check_accepts(
        r#"## Main
Show upper("hi").
Show sqrt(4).
Show length("abc").
"#,
    );
}

#[test]
fn prelude_natives_stay_accepted() {
    assert_check_accepts(
        r#"## Main
Let n be randomInt(1, 6).
Show n.
Let xs be args().
Show length of xs.
"#,
    );
}

#[test]
fn prelude_file_io_natives_stay_accepted() {
    // `read`/`write`/`readBytes` are `## To native` declarations spliced in by
    // apply_prelude; check runs on the POST-prelude program, so they resolve.
    assert_check_accepts(
        r#"## Main
Write "x" to file "/tmp/logos_unknown_call_lock.txt".
Let t be read("/tmp/logos_unknown_call_lock.txt").
Show t.
"#,
    );
}

#[test]
fn forward_referenced_user_functions_stay_accepted() {
    // `first` calls `second`, which is defined LATER in the file — the
    // checker's preregistration pass must resolve it.
    assert_all_tiers(
        r#"## To first (n: Int) -> Int:
    Return second(n) + 1.

## To second (n: Int) -> Int:
    Return n * 2.

## Main
Show first(5).
"#,
        "11",
    );
    assert_check_accepts(
        r#"## To first (n: Int) -> Int:
    Return second(n) + 1.

## To second (n: Int) -> Int:
    Return n * 2.

## Main
Show first(5).
"#,
    );
}

#[test]
fn main_before_definition_stays_accepted_by_check() {
    // The CHECKER accepts a Main that precedes the definition (preregistration
    // sees the whole program) — locked here so check strictness never breaks
    // the layout the VM supports.
    assert_check_accepts(
        r#"## Main
Show later(5).

## To later (n: Int) -> Int:
    Return n.
"#,
    );
}

/// CLOSED (found by this lock 2026-09-01, fixed the same day): the TREE-WALKER
/// executed top-level statements in order, so a `## Main` PRECEDING a `## To`
/// definition failed at runtime with `Unknown function: later` while the VM
/// (whose compiler preregisters `fn_index` over the whole program) printed `5`
/// — an Invariant II tier divergence. The fix is
/// `Interpreter::preregister_definitions` (`interpreter.rs`), a prepass over
/// `FunctionDef`/`StructDef` run from BOTH the async and sync entry points so
/// the twins cannot drift, mirroring the VM compiler's Pass 1. This test now
/// runs for real and holds the tiers together.
#[test]
fn main_before_definition_agrees_across_runtime_tiers() {
    assert_all_tiers(
        r#"## Main
Show later(5).

## To later (n: Int) -> Int:
    Return n.
"#,
        "5",
    );
}

#[test]
fn enum_variant_constructors_stay_accepted() {
    assert_check_accepts(
        r#"## A Maybe is one of:
    A Nothing.
    A Just with value Int.

## Main
Let x be a new Just with value 42.
Inspect x:
    When Nothing: Show "empty".
    When Just (v): Show v.
"#,
    );
}

#[test]
fn option_constructors_stay_accepted() {
    assert_check_accepts(
        r#"## To find (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Show find(5).
"#,
    );
}

#[test]
fn closure_valued_variables_stay_callable() {
    assert_check_accepts(
        r#"## Main
Let f be (x: Int) -> x.
Show f(42).
"#,
    );
    assert_all_tiers(
        r#"## Main
Let f be (x: Int) -> x.
Show f(42).
"#,
        "42",
    );
}

#[test]
fn function_typed_params_stay_callable() {
    assert_check_accepts(
        r#"## To apply (f: fn(Int) -> Int) and (n: Int) -> Int:
    Return f(n).

## Main
Let double be (x: Int) -> x * 2.
Show apply(double, 21).
"#,
    );
}

// ============================================================================
// Interp/VM parity: the runtime tiers name the same missing function
// ============================================================================

#[test]
fn runtime_tiers_agree_on_the_unknown_call_error() {
    let agreed = run_all_tiers(
        r#"## Main
Let t be readText("config.txt").
Show t.
"#,
    );
    let err = agreed
        .error
        .expect("interp/VM must fail on a call to an undefined function");
    assert!(
        err.contains("Unknown function: readText"),
        "runtime tiers must name the missing function; got: {err}"
    );
}
