//! Phase 112 — §5.3 Proportional / partitive quantifiers (MISSING_ENGLISH.md).
//!
//! Quantifiers over a presupposed salient set:
//!   "Two of the three students passed." → |{x:Student(x)∧Pass(x)}|=2  (within the 3-set)
//!   "Most of the students passed."       → MOST x(Student(x) ∧ Pass(x))
//! The "of the [Num]" partitive frame restricts to a contextual definite set; the
//! leading quantifier supplies the count/proportion. (Previously "of the" broke
//! the parse — the definite article was grabbed as the restriction.)

use logicaffeine_language::compile;

#[test]
fn cardinal_partitive_with_superset() {
    let out = compile("Two of the three students passed.").unwrap();
    eprintln!("two-of-three: {out}");
    assert!(out.contains("∃=2") || out.contains("=2"), "count is exactly two: {out}");
    assert!(out.contains("Student"), "restricted to students: {out}");
    assert!(out.contains("Pass"), "the predicate: {out}");
    assert!(!out.contains("(The)") && !out.contains("x(The"), "the definite article must not be the restriction: {out}");
}

#[test]
fn proportional_partitive_most() {
    let out = compile("Most of the students passed.").unwrap();
    eprintln!("most-of: {out}");
    assert!(out.contains("MOST"), "proportional MOST: {out}");
    assert!(out.contains("Student"), "restricted to students: {out}");
    assert!(out.contains("Pass"), "the predicate: {out}");
}

#[test]
fn cardinal_partitive_no_superset() {
    let out = compile("Two of the students passed.").unwrap();
    eprintln!("two-of: {out}");
    assert!(out.contains("∃=2") || out.contains("=2"), "count is two: {out}");
    assert!(out.contains("Student") && out.contains("Pass"), "restriction + predicate: {out}");
}

#[test]
fn some_partitive() {
    let out = compile("Some of the dogs barked.").unwrap();
    eprintln!("some-of: {out}");
    assert!(out.contains('∃'), "existential: {out}");
    assert!(out.contains("Dog") && out.contains("Bark"), "restriction + predicate: {out}");
}

#[test]
fn cardinal_partitive_superset_is_presupposed() {
    // §5.3: "Two of the three students passed." asserts exactly two students passed
    // AND presupposes the salient student set has exactly three members. The superset
    // cardinality (3) must survive as a presupposition — not be silently discarded.
    let out = compile("Two of the three students passed.").unwrap();
    eprintln!("two-of-three superset: {out}");
    assert!(out.contains("∃=2") || out.contains("=2"), "count is exactly two: {out}");
    assert!(out.contains("∃=3") || out.contains("=3"), "superset of exactly three is presupposed: {out}");
    assert!(out.contains("Presup"), "the superset surfaces as a presupposition: {out}");
}

#[test]
fn non_partitive_quantifiers_unchanged() {
    // Regression: plain (non-partitive) quantifiers are unaffected.
    let most = compile("Most students passed.").unwrap();
    assert!(most.contains("MOST") && most.contains("Student"), "plain MOST: {most}");
    let three = compile("Three students passed.").unwrap();
    assert!((three.contains("∃=3") || three.contains("=3")) && three.contains("Student"), "plain cardinal: {three}");
}
