//! Opening a theorem and pressing Execute read its result through one function, so the
//! proof panel says the same thing either way — and never "Ready to prove" beside a
//! derivation the kernel refused.
use super::*;
use logicaffeine_compile::compile_theorem_for_ui;
use crate::ui::examples::{Expected, ALL_LOGIC_EXAMPLES, LOGIC_BARBER, LOGIC_LEIBNIZ};

#[test]
fn an_uncertified_derivation_says_it_is_not_certified() {
    let result = compile_theorem_for_ui(LOGIC_BARBER);
    assert!(result.derivation.is_some() && !result.verified, "the Barber is the uncertified demonstration");
    let view = theorem_view(&result);
    assert!(matches!(view.status, ProofStatus::Uncertified(_)), "the Barber's status is not uncertified");
    assert!(view.hint.contains("not certified"), "{}", view.hint);
    assert!(!view.hint.contains("grid"), "the Barber has no grid: {}", view.hint);
    assert!(view.tree.is_some(), "the derivation is still shown, for the reader to inspect");
}

#[test]
fn an_uncertified_status_carries_where_verification_broke() {
    let result = compile_theorem_for_ui(LOGIC_BARBER);
    let view = theorem_view(&result);
    let ProofStatus::Uncertified(reason) = &view.status else { panic!("the Barber's status is not uncertified") };
    let broke = result.verification_error.clone().expect("an uncertified derivation says where it broke");
    assert_eq!(reason, &broke);
}

#[test]
fn a_certified_proof_is_a_success() {
    let view = theorem_view(&compile_theorem_for_ui(LOGIC_LEIBNIZ));
    assert!(view.status == ProofStatus::Success, "Leibniz is proved");
    assert!(view.hint.contains("proved"), "{}", view.hint);
}

#[test]
fn a_theorem_with_nothing_derived_is_ready_with_its_premises() {
    let result = compile_theorem_for_ui("## Theorem: Unrelated\nGiven: John runs.\nGiven: Mary walks.\nProve: Bill sleeps.\n");
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(result.derivation.is_none() && result.grid.is_none() && result.answer.is_none());
    let view = theorem_view(&result);
    assert!(view.status == ProofStatus::Idle);
    assert!(view.html.is_empty() && view.tree.is_none());
    assert!(view.hint.contains("2 premise(s)"), "{}", view.hint);
}

fn premises_and_goal(source: &str) -> (Vec<ProofExpr>, ProofExpr) {
    let result = compile_theorem_for_ui(source);
    (result.premises, result.goal.expect("the theorem has a goal"))
}

#[test]
fn auto_calls_a_certified_derivation_a_proof() {
    let (kb, goal) = premises_and_goal(LOGIC_LEIBNIZ);
    let view = auto_view(&kb, &goal, "goal");
    assert!(view.status == ProofStatus::Success, "{}", view.hint);
    assert!(view.tree.is_some(), "the derivation is drawn");
}

#[test]
fn auto_never_calls_an_uncertified_derivation_a_proof() {
    let (kb, goal) = premises_and_goal(LOGIC_BARBER);
    let view = auto_view(&kb, &goal, "goal");
    assert!(matches!(view.status, ProofStatus::Uncertified(_)), "Auto called the Barber's derivation: {}", view.hint);
    assert!(view.hint.contains("not certified"), "{}", view.hint);
    assert!(view.tree.is_some(), "the derivation is still shown");
}

#[test]
fn auto_agrees_with_opening_the_theorem() {
    for spec in ALL_LOGIC_EXAMPLES.iter().filter(|s| matches!(s.expected, Expected::Proves | Expected::DerivationNotCertified)) {
        let result = compile_theorem_for_ui(spec.source);
        if result.answer.is_some() || result.grid.is_some() {
            continue;
        }
        let opened = theorem_view(&result);
        let auto = auto_view(&result.premises, result.goal.as_ref().expect("a goal"), "goal");
        assert!(
            std::mem::discriminant(&opened.status) == std::mem::discriminant(&auto.status),
            "{}: opening says {:?}, Auto says {:?}",
            spec.vfs_path,
            opened.status.line().1,
            auto.status.line().1
        );
    }
}

#[test]
fn auto_without_a_derivation_fails_with_the_goal_escaped() {
    let (_, goal) = premises_and_goal(LOGIC_LEIBNIZ);
    let view = auto_view(&[], &goal, "x < y & z");
    assert!(matches!(view.status, ProofStatus::Failed(_)), "{}", view.hint);
    assert!(view.html.contains("x &lt; y &amp; z"), "{}", view.html);
    assert!(!view.html.contains("x < y"), "{}", view.html);
}

#[test]
fn every_logic_theorem_reads_as_its_registry_promise() {
    let mut wrong = Vec::new();
    for spec in ALL_LOGIC_EXAMPLES {
        let result = compile_theorem_for_ui(spec.source);
        let view = theorem_view(&result);
        let kept = match spec.expected {
            Expected::Proves => view.status == ProofStatus::Success,
            Expected::DerivationNotCertified => matches!(view.status, ProofStatus::Uncertified(_)),
            _ => continue,
        };
        if !kept {
            wrong.push(format!("{}: {}", spec.vfs_path, view.hint));
        }
    }
    assert!(wrong.is_empty(), "{} theorem(s) read wrong:\n{}", wrong.len(), wrong.join("\n"));
}
