//! Phase 118 — §5.4 Floating quantifiers (MISSING_ENGLISH.md).
//!
//! A stranded all/each/both re-associates to the subject NP and distributes the
//! predicate over its members:
//!   "The boys all left."                → distributive over the boys
//!   "The students each solved a problem."→ ∀ student ∃ problem (distributive)
//!   "Each student passed."               → ∀x(Student(x) → Pass(x))  (leading 'each')
//! Previously the stranded quantifier broke the parse ("The boys all left." → "Boys").

use logicaffeine_language::compile;

#[test]
fn floating_all_distributes() {
    let out = compile("The boys all left.").unwrap();
    eprintln!("all: {out}");
    assert!(out.contains("Leave") || out.contains("Left"), "the predicate survives 'all': {out}");
    assert!(out.contains('*') || out.contains('∀'), "distributive/universal over the boys: {out}");
    assert!(out != "Boys", "must not collapse to the bare subject: {out}");
}

#[test]
fn floating_each_distributes() {
    let out = compile("The students each solved a problem.").unwrap();
    eprintln!("each: {out}");
    assert!(out.contains("Solve"), "the predicate survives 'each': {out}");
    assert!(out.contains("Problem"), "the object survives: {out}");
    assert!(out.contains('*') || out.contains('∀'), "distributive over the students: {out}");
}

#[test]
fn floating_both_distributes() {
    let out = compile("The boys both ran.").unwrap();
    eprintln!("both: {out}");
    assert!(out.contains("Run"), "the predicate survives 'both': {out}");
    assert!(out.contains('*') || out.contains('∀'), "distributive over the two boys: {out}");
}

#[test]
fn leading_each_is_universal() {
    let out = compile("Each student passed.").unwrap();
    eprintln!("lead-each: {out}");
    assert!(out.contains('∀'), "leading 'each' is a universal quantifier: {out}");
    assert!(out.contains("Student") && out.contains("Pass"), "restriction + predicate: {out}");
}

#[test]
fn reciprocal_each_other_preserved() {
    // Regression: "each other" is a reciprocal MWE, unaffected by 'each' as a quantifier.
    let out = compile("John and Mary love each other.").unwrap();
    eprintln!("recip: {out}");
    assert!(out.contains("Love") && out.contains("John") && out.contains("Mary"), "reciprocal intact: {out}");
}
