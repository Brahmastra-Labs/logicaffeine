//! Phase 126 — §7.2 Implicit comparison class / standard (MISSING_ENGLISH.md).
//!
//! Bare gradable predication relies on a contextual standard θ_C:
//!   "John is tall." → ∃d(Tall(john,d) ∧ d > θ_C)
//! "Tall" means exceeding a context standard of height, not a crisp predicate.
//! Non-gradable predicates ("John is Canadian") get no degree/standard.

use logicaffeine_language::compile_pragmatic as compile;

#[test]
fn bare_gradable_gets_degree_and_standard() {
    let out = compile("John is tall.").unwrap();
    eprintln!("tall: {out}");
    assert!(out.contains("Tall"), "the gradable predicate: {out}");
    // A degree argument bound existentially, exceeding the context standard θ.
    assert!(out.contains('∃'), "introduces a degree variable: {out}");
    assert!(out.contains('θ') || out.contains('Θ') || out.contains("theta"), "compares to a context standard θ: {out}");
    assert!(out.contains('>'), "the degree exceeds the standard: {out}");
}

#[test]
fn another_gradable_adjective() {
    let out = compile("Mary is heavy.").unwrap();
    eprintln!("heavy: {out}");
    assert!(out.contains("Heavy"), "the gradable predicate: {out}");
    assert!(out.contains('θ') || out.contains('Θ') || out.contains("theta"), "context standard: {out}");
}

#[test]
fn non_gradable_predicate_has_no_standard() {
    // Regression: a non-gradable predicate is crisp — no degree/standard.
    let out = compile("John is Canadian.").unwrap();
    eprintln!("canadian: {out}");
    assert!(out.contains("Canadian"), "the predicate: {out}");
    assert!(!out.contains('θ') && !out.contains('Θ'), "no spurious degree standard on a non-gradable predicate: {out}");
}
