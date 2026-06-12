//! Phase 121 — §3.3 Secondary predication (MISSING_ENGLISH.md).
//!
//! A predicate over an argument alongside the verb:
//!   resultative — "John painted the door red."  → … ∧ Result(e, Red(door))
//!   depictive   — "John ate the meat raw."       → … ∧ Depictive(e, Raw(meat))
//! Previously the post-object AP was dropped or mis-attached.

use logicaffeine_language::compile;

#[test]
fn resultative_adds_result_role() {
    let out = compile("John painted the door red.").unwrap();
    eprintln!("resultative: {out}");
    assert!(out.contains("Paint"), "the verb event: {out}");
    assert!(out.contains("Agent") && (out.contains("John") || out.contains('J')), "agent: {out}");
    assert!(out.contains("Door") || out.contains("door"), "the theme/object: {out}");
    // The resultative AP attaches as a Result secondary predicate over the object.
    assert!(out.contains("Result"), "resultative contributes a Result role: {out}");
    assert!(out.contains("Red") || out.contains("red"), "the result state: {out}");
}

#[test]
fn depictive_adds_depictive_role() {
    let out = compile("John ate the meat raw.").unwrap();
    eprintln!("depictive: {out}");
    assert!(out.contains("Eat") || out.contains("Ate"), "the verb event: {out}");
    assert!(out.contains("Meat") || out.contains("meat"), "the object: {out}");
    assert!(out.contains("Depictive"), "depictive contributes a Depictive role: {out}");
    assert!(out.contains("Raw") || out.contains("raw"), "the depictive state: {out}");
}

#[test]
fn plain_transitive_unaffected() {
    // Regression: a transitive with no secondary predicate has no Result/Depictive.
    let out = compile("John painted the door.").unwrap();
    eprintln!("plain: {out}");
    assert!(out.contains("Paint") && (out.contains("Door") || out.contains("door")), "core predication: {out}");
    assert!(!out.contains("Result") && !out.contains("Depictive"), "no spurious secondary predicate: {out}");
}
