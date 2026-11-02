//! A coordinated subject is a group, `John ⊕ Mary`, and a DISTRIBUTIVE verb says of the group
//! what it says of each member: "John and Mary left" — so John left, and Mary left. A
//! COLLECTIVE verb does not ("John and Mary met" says nothing of John meeting on his own), nor
//! does one that may be collective ("John and Mary carried the piano" may be joint work). The
//! Studio keeps showing the group; the prover reads each member's share.

use logicaffeine_compile::{compile_for_ui, compile_theorem_for_ui};

fn theorem(name: &str, givens: &[&str], goal: &str) -> logicaffeine_compile::TheoremCompileResult {
    let mut source = format!("## Theorem: {name}\n");
    for given in givens {
        source.push_str(&format!("Given: {given}\n"));
    }
    source.push_str(&format!("Prove: {goal}\nProof: Auto.\n"));
    compile_theorem_for_ui(&source)
}

fn certifies(name: &str, givens: &[&str], goal: &str) {
    let result = theorem(name, givens, goal);
    assert!(result.error.is_none(), "{name} did not compile: {:?}", result.error);
    assert!(
        result.verified,
        "{name}: from {givens:?} the prover does not certify {goal:?} ({:?}); premises {:?}",
        result.verification_error, result.premises
    );
}

fn refuses(name: &str, givens: &[&str], goal: &str) {
    let result = theorem(name, givens, goal);
    assert!(result.error.is_none(), "{name} did not compile: {:?}", result.error);
    assert!(!result.verified, "{name}: from {givens:?} the prover certified {goal:?}; premises {:?}", result.premises);
}

#[test]
fn each_member_of_a_group_that_left_left() {
    certifies("John_Left", &["John and Mary left."], "John left.");
    certifies("Mary_Left", &["John and Mary left."], "Mary left.");
    refuses("Bill_Did_Not", &["John and Mary left."], "Bill left.");
}

#[test]
fn each_member_does_what_a_transitive_verb_says() {
    certifies("John_Sees_Bill", &["John and Mary see Bill."], "John sees Bill.");
    certifies("Mary_Sees_Bill", &["John and Mary see Bill."], "Mary sees Bill.");
}

#[test]
fn a_collective_verb_says_nothing_of_one_member() {
    refuses("John_Met_Alone", &["John and Mary met."], "John met.");
}

#[test]
fn a_verb_that_may_be_collective_does_not_distribute() {
    refuses("John_Carried_Alone", &["John and Mary carried the piano."], "John carried the piano.");
}

#[test]
fn the_studio_still_shows_the_group() {
    let result = compile_for_ui("John and Mary left.");
    assert!(result.error.is_none(), "{:?}", result.error);
    let logic = result.logic.unwrap_or_default();
    assert!(logic.contains("⊕"), "the formula shown keeps the group: {logic}");
}
