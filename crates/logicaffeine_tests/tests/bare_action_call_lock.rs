//! LOCK — a bare `Call <name>.` inside a component's own `Actions:`/event body that closely
//! misspells one of the SAME component's own OTHER actions is refused at compile time, with a
//! did-you-mean — the static half (`crates/logicaffeine_tests/tests/gauntlet_findings_lock.rs::
//! calling_a_same_components_own_action_by_its_bare_name_runs_it` carries the RUNTIME half, which
//! needs `interpreter.rs`/the lowering and is not this file's concern).
//!
//! ★ WHAT THIS DELIBERATELY DOES NOT REFUSE, AND WHY: a bare `Call <name>.` may also legally reach
//! a top-level `## To` function, a kernel builtin, or a standard-library function the demand-driven
//! prelude splices into the source text BEFORE a real (`largo`/`run_tests`) parse ever runs —
//! invisible to `Parser::parse_program()` called directly, which is exactly what this file's own
//! `parse`/`accepted`/`refusal` harness does (see `language_common::parse`). So the check refuses
//! ONLY a name that is ALSO a close typo of a sibling action (`crate::suggest::did_you_mean`'s own
//! bounded edit-distance) — never merely "unrecognised" — and the fixtures below prove both halves:
//! a typo is caught, and an ordinary unrelated name is left alone.
//!
//! ★ THE SPAN NAMES THE ACTION, NOT THE `Call` TOKEN. `Stmt::Call` carries no span of its own (it
//! is the same generic node an ordinary top-level function call parses to), so this check reports
//! at the ENCLOSING action's/handler's own span — the finest position actually available without a
//! parse-time side table for a bare `Call` (the shape `store_calls`/`journal_action_sites` already
//! have for their own constructs). Sharpening this to the exact `Call bum.` token needs a small
//! grant in `parser/mod.rs` (`parse_call_statement`) this lane does not hold; flagged, not faked.

#[path = "language_common/mod.rs"]
mod common;

use common::{accepted, refusal};

fn app(actions: &str) -> String {
    format!(
        "## Component App\n\
         State:\n\x20   Let n be 0.\n\
         Actions:\n{actions}\
         View:\n\x20   <p>{{n}}</p>\n\n\
         ## Main\nMount App.\n"
    )
}

#[test]
fn a_bare_call_typo_of_a_sibling_action_is_refused_with_did_you_mean() {
    let src = app("    To bumpTwice:\n\
         \x20       Call bum.\n\
         \x20       Call bum.\n\
         \x20   To bump:\n\
         \x20       Set n to n + 1.\n");
    let (message, at) = refusal(&src);
    assert!(
        !at.is_empty(),
        "the refusal must carry a real location, not `Span::default()`: {message}"
    );
    assert!(
        message.contains("did you mean `bump`?"),
        "must suggest the close sibling action: {message}"
    );
    assert!(
        message.contains("## Component App"),
        "must name the component the action belongs to: {message}"
    );
}

#[test]
fn the_typo_is_caught_even_nested_inside_control_flow() {
    let src = app("    To bumpTwice:\n\
         \x20       If n is equal to 0:\n\
         \x20           Call bum.\n\
         \x20   To bump:\n\
         \x20       Set n to n + 1.\n");
    let (message, at) = refusal(&src);
    assert!(
        !at.is_empty(),
        "a typo nested inside an `If` must still carry a real location: {message}"
    );
    assert!(message.contains("did you mean `bump`?"), "{message}");
}

#[test]
fn an_exact_sibling_action_name_is_not_refused() {
    // The decided G23 shape itself: an EXACT match against a sibling action needs no refusal at
    // all — running it is the tree-walker's and the lowering's job, not this check's.
    let src = app("    To bumpTwice:\n\
         \x20       Call bump.\n\
         \x20       Call bump.\n\
         \x20   To bump:\n\
         \x20       Set n to n + 1.\n");
    accepted(&src);
}

#[test]
fn a_top_level_function_this_file_declares_is_not_refused() {
    let src = format!(
        "## To helper:\n\x20   Return.\n\n{}",
        app("    To go:\n\x20       Call helper.\n")
    );
    accepted(&src);
}

#[test]
fn an_unrelated_bare_call_is_left_alone_not_refused() {
    // Not a close typo of any sibling action, and not a top-level function this file declares —
    // this check must NOT refuse it (it may be a builtin or a demand-spliced stdlib function this
    // pass cannot see, per the module doc above). It still parses to an ordinary `Stmt::Call`
    // whose target resolves (or does not) wherever a real compile seam actually runs it.
    let src = app("    To go:\n\x20       Call totallyUnrelatedThing.\n");
    accepted(&src);
}

#[test]
fn an_event_handler_bodys_typo_is_also_caught() {
    let src = format!(
        "## Component App\n\
         State:\n\x20   Let n be 0.\n\
         Actions:\n\
         \x20   When the user presses \"Enter\":\n\
         \x20       Call bum.\n\
         \x20   To bump:\n\
         \x20       Set n to n + 1.\n\
         View:\n\x20   <p>{{n}}</p>\n\n\
         ## Main\nMount App.\n"
    );
    let (message, at) = refusal(&src);
    assert!(!at.is_empty(), "must carry a real location: {message}");
    assert!(message.contains("did you mean `bump`?"), "{message}");
}
