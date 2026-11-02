//! ════════════════════════════════════════════════════════════════════════════
//! METAMORPHIC LOCK — semantics-preserving transforms and determinism.
//!
//! For ANY program, a transform that provably does not change meaning must not change
//! output — on EVERY tier (via `run_all_tiers`). A transform that alters observable output
//! is a miscompile. Conservative, airtight relations only (five real ones beat fifty
//! dubious): dead-binding insertion, trailing whitespace, and re-running (determinism).

mod common;
use common::run_all_tiers;

/// Proven-valid sequential programs (from the interpreter-golden suites) with room for a
/// dead `Let` before the final `Show`.
const BASE: &[(&str, &str)] = &[
    ("## Main\nLet a be 3.\nLet b be 4.\nShow a + b.", "7"),
    ("## Main\nLet m be (5 + 4) / 2.\nShow m.", "4"),
    ("## Main\nLet items be [1, 2, 3].\nShow items.", "[1, 2, 3]"),
    (
        "## Main\nLet r: Rational be 1 / 2.\nLet s be r + 3.\nShow s.",
        "7/2",
    ),
];

/// Semantics-preserving: insert an UNUSED binding. Dead code cannot change output.
fn insert_dead_let(src: &str) -> String {
    src.replacen(
        "## Main\n",
        "## Main\nLet unused_metamorphic_binding be 99.\n",
        1,
    )
}

/// Semantics-preserving: trailing whitespace is not observable.
fn append_whitespace(src: &str) -> String {
    format!("{src}\n\n   \n")
}

#[test]
fn dead_binding_insertion_preserves_output_on_all_tiers() {
    for (src, expected) in BASE {
        let base = run_all_tiers(src);
        assert_eq!(base.output.trim(), *expected, "baseline mismatch:\n{src}");
        let transformed = run_all_tiers(&insert_dead_let(src));
        assert_eq!(
            base.output.trim(),
            transformed.output.trim(),
            "METAMORPHIC VIOLATION (Invariant VIII): inserting a dead binding changed output.\n{src}"
        );
        assert_eq!(
            base.error, transformed.error,
            "dead binding changed error text:\n{src}"
        );
    }
}

#[test]
fn trailing_whitespace_preserves_output_on_all_tiers() {
    for (src, _) in BASE {
        let base = run_all_tiers(src);
        let ws = run_all_tiers(&append_whitespace(src));
        assert_eq!(
            base.output.trim(),
            ws.output.trim(),
            "METAMORPHIC VIOLATION: trailing whitespace changed output.\n{src}"
        );
        assert_eq!(base.error, ws.error);
    }
}

#[test]
fn execution_is_deterministic_across_reruns() {
    for (src, _) in BASE {
        let a = run_all_tiers(src);
        let b = run_all_tiers(src);
        assert_eq!(
            a.output, b.output,
            "DETERMINISM VIOLATION (Invariant IX):\n{src}"
        );
        assert_eq!(
            a.error, b.error,
            "DETERMINISM VIOLATION (error text):\n{src}"
        );
    }
}
