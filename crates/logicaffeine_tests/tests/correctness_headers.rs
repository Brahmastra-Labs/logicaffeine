//! Part I correctness: a typo'd code header FAILS LOUDLY with a suggestion —
//! and a headerless script runs as Main.
//!
//! The audit rows: `## Mian` was silently swallowed (the whole program ran
//! to empty output); a file without `## Main` parsed as logic prose and ran
//! empty. The rules (ambiguity-preserving): an unknown `##` header ERRORS
//! with did-you-mean ONLY when it is edit-distance-close to a known CODE
//! header — `## My Notes` stays literate prose; and a source with NO `##`
//! headers at all that parses as imperative statements runs as an implicit
//! `## Main` (the natural-language pipeline is untouched).

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_interpreter};

#[test]
fn typo_main_header_errors_with_suggestion() {
    let r = run_interpreter(
        r#"## Mian
Show 1.
"#,
    );
    assert!(!r.success, "a typo'd code header must not run to empty output");
    assert!(
        r.error.contains("Main"),
        "the error should suggest the intended header, got: {}",
        r.error
    );
}

#[test]
fn distant_prose_header_stays_literate() {
    // A heading nothing like a code header is documentation, not a typo —
    // the program beside it still runs.
    assert_compiled_equals_interpreted_eq(
        r#"## Design Notes

This paragraph documents the program.

## Main
Show 7.
"#,
        "7",
    );
}

#[test]
fn headerless_script_runs_as_main() {
    assert_compiled_equals_interpreted_eq(
        r#"Let x be 5.
Show x + 1.
"#,
        "6",
    );
}
