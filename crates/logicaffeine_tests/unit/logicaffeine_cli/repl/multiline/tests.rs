use super::*;

#[test]
fn logic_sentences_end_with_period_or_question() {
    assert_eq!(
        input_state(Mode::Logic, "Every man is mortal."),
        InputState::Complete
    );
    assert_eq!(
        input_state(Mode::Logic, "Is Socrates mortal?"),
        InputState::Complete
    );
    assert_eq!(input_state(Mode::Logic, "Every man"), InputState::NeedsMore);
}

#[test]
fn imperative_statement_ends_with_period() {
    assert_eq!(
        input_state(Mode::Imperative, "Show 1."),
        InputState::Complete
    );
    assert_eq!(
        input_state(Mode::Imperative, "Let x be"),
        InputState::NeedsMore
    );
}

#[test]
fn colon_opens_a_block_until_blank_line() {
    let open = "If x is less than 2:";
    assert_eq!(input_state(Mode::Imperative, open), InputState::NeedsMore);
    let body = "If x is less than 2:\n    Show 1.";
    assert_eq!(input_state(Mode::Imperative, body), InputState::NeedsMore);
    let done = "If x is less than 2:\n    Show 1.\n\n";
    assert_eq!(input_state(Mode::Imperative, done), InputState::Complete);
}

#[test]
fn definition_blocks_submit_on_blank_line() {
    let def = "## To double (n: Int) -> Int:";
    assert_eq!(input_state(Mode::Imperative, def), InputState::NeedsMore);
    let full = "## To double (n: Int) -> Int:\n    Return n * 2.\n\n";
    assert_eq!(input_state(Mode::Imperative, full), InputState::Complete);
}

#[test]
fn meta_commands_are_always_complete() {
    assert_eq!(input_state(Mode::Imperative, ":help"), InputState::Complete);
    assert_eq!(
        input_state(Mode::Logic, ":format latex"),
        InputState::Complete
    );
}

#[test]
fn continuation_indents_after_block_opener() {
    assert_eq!(continuation_indent("If x is less than 2:"), "    ");
    assert_eq!(continuation_indent("Show 1."), "");
}
