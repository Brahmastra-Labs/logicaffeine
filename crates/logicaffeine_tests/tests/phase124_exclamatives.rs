//! Phase 124 — §1.1 Exclamatives (MISSING_ENGLISH.md).
//!
//! Clauses expressing affective stance toward a DEGREE, marked by how/what with
//! NO subject-aux inversion, and asserting a degree is surprisingly high:
//!   "How tall she is!"          → Exclaim(∃d(Tall(she,d) ∧ d ≫ θ))   (presupposes Tall(she))
//!   "What a fool he is!"
//! They must resolve as exclamatives, NOT wh-questions.

use logicaffeine_language::compile;

#[test]
fn how_adj_exclamative_asserts_high_degree() {
    let out = compile("How tall she is!").unwrap();
    eprintln!("how-tall: {out}");
    assert!(
        out.contains("Exclaim") || out.contains("Excl") || out.contains('!'),
        "exclamative force marker: {out}"
    );
    assert!(out.contains("Tall") || out.contains("tall"), "the gradable predicate: {out}");
    // A surprisingly-high degree: d ≫ θ.
    assert!(
        out.contains('≫') || out.contains("θ") || out.contains(">>"),
        "asserts a degree far above the standard (d ≫ θ): {out}"
    );
    // Not a wh-question.
    assert!(!out.contains("Question") && !out.contains('?'), "exclamative, not a question: {out}");
}

#[test]
fn what_a_n_exclamative() {
    let out = compile("What a fool he is!").unwrap();
    eprintln!("what-a-fool: {out}");
    assert!(
        out.contains("Exclaim") || out.contains("Excl") || out.contains('!'),
        "exclamative force: {out}"
    );
    assert!(out.contains("Fool") || out.contains("fool"), "the noun: {out}");
    assert!(!out.contains("Question"), "exclamative, not a question: {out}");
}
