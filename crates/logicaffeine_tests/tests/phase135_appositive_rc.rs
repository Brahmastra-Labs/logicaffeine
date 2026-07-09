//! Phase 135 — §3.1 Non-restrictive (appositive) relative clauses (work/MISSING_ENGLISH.md).
//!
//! A comma-delimited relative clause on a referential head is a SIDE-ASSERTION, not
//! a restriction:
//!   "John, who loves Mary, left." → Left(john) ∧ Love(john, mary)
//! (Contrast a restrictive RC: "The man who loves Mary left." restricts the man.)

use logicaffeine_language::compile;

#[test]
fn appositive_rc_is_a_side_assertion() {
    let out = compile("John, who loves Mary, left.").unwrap();
    eprintln!("appositive: {out}");
    assert!(out.contains("Leave") || out.contains("Left"), "the main clause: {out}");
    assert!(out.contains("Love"), "the appositive RC predication: {out}");
    assert!(out.contains("Mary"), "the RC object: {out}");
    // Both predicated of John directly (referential head, no restriction variable).
    assert!(out.matches("John").count() >= 2 || (out.contains("Agent(e, John)") && out.contains("Love")),
        "both clauses are about John: {out}");
}

#[test]
fn restrictive_rc_still_restricts() {
    // Regression: a restrictive (no-comma) RC keeps the restriction reading.
    let out = compile("The man who loves Mary left.").unwrap();
    eprintln!("restrictive: {out}");
    assert!(out.contains("Man"), "the restricted noun: {out}");
    assert!(out.contains('∃') || out.contains('∀'), "a bound restriction variable: {out}");
}
