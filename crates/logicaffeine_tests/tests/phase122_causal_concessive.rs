//! Phase 122 — §3.4 Causal / concessive adverbial clauses (work/MISSING_ENGLISH.md).
//!
//! Subordinators encode logical relations beyond temporal precedence:
//!   causal     — "John stayed because it rained."  → Stay(john) ∧ Cause(Rain, Stay(john))
//!   concessive — "Although she was tired, she finished." → Finish(she) ∧ Concessive(Tired(she))
//! A concessive presupposes a DEFEATED expectation (tired → ¬finish, yet she did).

use logicaffeine_language::compile;

#[test]
fn because_introduces_cause() {
    let out = compile("John stayed because it rained.").unwrap();
    eprintln!("because: {out}");
    assert!(out.contains("Stay"), "main clause: {out}");
    assert!(out.contains("Rain"), "subordinate clause: {out}");
    assert!(out.contains("Cause"), "because contributes a Cause relation: {out}");
}

#[test]
fn although_introduces_concessive() {
    let out = compile("Although she was tired, she finished.").unwrap();
    eprintln!("although: {out}");
    assert!(out.contains("Finish"), "main clause: {out}");
    assert!(out.contains("Tired") || out.contains("tired"), "concession clause: {out}");
    assert!(out.contains("Concessive"), "although contributes a Concessive relation: {out}");
}

#[test]
fn though_introduces_concessive() {
    let out = compile("Though it rained, John ran.").unwrap();
    eprintln!("though: {out}");
    assert!(out.contains("Run"), "main clause: {out}");
    assert!(out.contains("Concessive"), "though is concessive: {out}");
}
