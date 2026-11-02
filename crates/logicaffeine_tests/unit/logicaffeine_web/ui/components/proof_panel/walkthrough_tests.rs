use super::{dock_prompt, StepState, Walkthrough, STEPS};

#[test]
fn a_blank_page_is_on_its_first_step() {
    let w = Walkthrough::default();
    assert_eq!(w.states(), [StepState::Now, StepState::Next, StepState::Next]);
    assert_eq!(dock_prompt(w, None), "Write a sentence in English to begin.");
}

#[test]
fn a_sentence_written_moves_the_reader_to_execute() {
    let w = Walkthrough { wrote: true, executed: false };
    assert_eq!(w.states(), [StepState::Done, StepState::Now, StepState::Next]);
    assert_eq!(dock_prompt(w, Some("ignored until Execute")), "Press Execute to turn your sentences into logic.");
}

#[test]
fn once_executed_prove_is_next_and_the_prover_has_the_line() {
    let w = Walkthrough { wrote: true, executed: true };
    assert_eq!(w.states(), [StepState::Done, StepState::Done, StepState::Now]);
    assert_eq!(dock_prompt(w, None), "Press Prove to check it follows from the givens.");
    assert_eq!(dock_prompt(w, Some("Proof found!")), "Proof found!");
}

/// Execute without a sentence is nothing done: the steps go in order.
#[test]
fn executing_nothing_counts_for_nothing() {
    let w = Walkthrough { wrote: false, executed: true };
    assert_eq!(w.states(), [StepState::Now, StepState::Next, StepState::Next]);
}

#[test]
fn the_steps_are_write_execute_prove_and_say_what_prove_checks() {
    assert_eq!(STEPS.map(|(title, _)| title), ["Write English", "Execute", "Prove"]);
    assert!(STEPS[2].1.contains("givens"), "the Prove step does not say what it proves from");
    assert!(STEPS[0].1.contains("Given"), "the first step does not say to write Given lines");
}
