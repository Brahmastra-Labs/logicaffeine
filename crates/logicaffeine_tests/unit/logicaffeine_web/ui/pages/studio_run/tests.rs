//! Execute in Logic and Math mode: what a document of each shape produces, from a fresh start every
//! run, the way the Studio and the front page's mini Studio both show it.
use super::*;
use crate::ui::examples::{seeded_examples, Expected, MATH_NAT};
use crate::ui::pages::studio::parse_math_statements;

fn shipped(path: &str) -> &'static str {
    seeded_examples().find(|spec| spec.vfs_path == path).unwrap_or_else(|| panic!("{path} is not shipped")).source
}

#[test]
fn a_theorem_compiles_and_the_prover_proves_it() {
    match run_logic(shipped("/examples/logic/syllogism.logic")) {
        LogicRun::Theorem(Ok(theorem)) => {
            assert_eq!(theorem.name, "Chain_Reasoning");
            assert!(theorem.derivation.is_some() && theorem.verified, "the syllogism is not proved and kernel-checked: {:?}", theorem.verification_error);
            assert!(theorem.goal_string.is_some(), "the theorem has no goal to show");
        }
        LogicRun::Theorem(Err(e)) => panic!("the syllogism does not compile: {e}"),
        _ => panic!("a ## Theorem block was not run as a theorem"),
    }
}

#[test]
fn a_theorem_that_does_not_compile_says_why() {
    match run_logic("## Theorem: Broken\nProve:\n") {
        LogicRun::Theorem(Err(e)) => assert!(!e.trim().is_empty(), "the error says nothing"),
        LogicRun::Theorem(Ok(_)) => panic!("a theorem with an empty goal compiled"),
        _ => panic!("a ## Theorem block was not run as a theorem"),
    }
}

#[test]
fn sentences_are_read_into_logic_without_their_headers_and_comments() {
    match run_logic("# Cats\n-- one sentence\n\nEvery cat sleeps.\n") {
        LogicRun::Sentences(result, proof) => {
            assert!(result.error.is_none(), "the sentence does not compile: {:?}", result.error);
            let logic = result.simple_logic.expect("the sentence reads into no logic");
            assert!(logic.contains("Cat") && logic.contains("Sleep"), "the logic is not the sentence's: {logic}");
            assert!(!logic.contains("Cats") && !logic.contains("one sentence"), "a header or a comment was read as a sentence: {logic}");
            assert!(proof.proof_expr.is_some() || proof.error.is_some(), "the first sentence was not compiled for the prover");
        }
        _ => panic!("plain sentences were not read as sentences"),
    }
}

#[test]
fn every_sentence_is_read_not_only_the_first() {
    let LogicRun::Sentences(result, _) = run_logic("Every cat sleeps.\nJohn loves Mary.\n") else { panic!("plain sentences were not read as sentences") };
    let logic = result.simple_logic.expect("the sentences read into no logic");
    assert!(logic.contains("Sleep") && logic.contains("Love"), "a sentence went unread: {logic}");
}

#[test]
fn headers_and_comments_alone_are_empty() {
    assert!(matches!(run_logic("# Title\n-- a note\n\n"), LogicRun::Empty));
    assert!(matches!(run_logic(""), LogicRun::Empty));
}

#[test]
fn math_runs_every_statement_from_a_fresh_kernel() {
    let first = run_math(MATH_NAT);
    assert_eq!(first.lines.len(), parse_math_statements(MATH_NAT).len(), "a statement went unrun");
    assert!(first.lines.iter().all(|line| line.success), "a statement of the natural-numbers example failed");
    assert!(first.kernel.is_some(), "the run kept no kernel for the Context panel");
    let again = run_math(MATH_NAT);
    assert_eq!(again.lines, first.lines, "a second run did not start from a fresh kernel");
}

#[test]
fn a_failing_statement_is_its_own_error_line_and_the_rest_still_run() {
    let run = run_math("Check nowhere_defined.\n\nInductive Unit := tt : Unit.\n\nCheck tt.\n");
    let outcomes: Vec<bool> = run.lines.iter().map(|line| line.success).collect();
    assert_eq!(outcomes, [false, true, true], "the run's lines: {:?}", run.lines);
}

#[test]
fn a_theory_certifies_each_theorem_and_builds_no_context() {
    let tarski = seeded_examples().find(|spec| matches!(spec.expected, Expected::TheoryAllVerified)).expect("a shipped ## Theory");
    let run = run_math(tarski.source);
    assert!(run.kernel.is_none(), "a theory left a statement kernel for the Context panel");
    let first = run.lines.first().expect("the theory printed nothing");
    assert!(first.success && first.input.starts_with("Theory Tarski"), "the first line does not name the theory: {first:?}");
    let theorems: Vec<&ReplLine> = run.lines.iter().filter(|line| line.input.starts_with("Theorem ")).collect();
    assert!(theorems.len() >= 3, "the theory certified {} theorems", theorems.len());
    assert!(theorems.iter().all(|line| line.success), "a theorem of the theory was not certified: {theorems:?}");
}
