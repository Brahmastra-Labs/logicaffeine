//! Multi-line input detection for the REPL — a pure function from the
//! buffered input to "submit now" vs "keep reading".

use crate::repl::command::Mode;

/// Whether a buffered input is ready to evaluate.
#[derive(Debug, PartialEq, Eq)]
pub enum InputState {
    /// Evaluate the buffer now.
    Complete,
    /// Show a continuation prompt and keep reading.
    NeedsMore,
}

/// Decide whether `buffer` (one or more lines, as typed so far) is a
/// complete input for `mode`.
///
/// - Meta-commands (`:` prefix) are always complete.
/// - Logic mode: a sentence is complete when it ends with `.` or `?`.
/// - Imperative mode: block constructs (`## ` definition headers, or any
///   line opening a block with a trailing `:`) submit on a blank line;
///   plain statements are complete when they end with `.`.
pub fn input_state(mode: Mode, buffer: &str) -> InputState {
    let trimmed = buffer.trim_end();
    if trimmed.is_empty() {
        return InputState::Complete; // nothing to do; caller skips empties
    }
    if trimmed.trim_start().starts_with(':') {
        return InputState::Complete;
    }
    match mode {
        Mode::Logic => {
            if trimmed.ends_with('.') || trimmed.ends_with('?') {
                InputState::Complete
            } else {
                InputState::NeedsMore
            }
        }
        Mode::Imperative => {
            let block_mode = trimmed.starts_with("## ")
                || trimmed.lines().any(|l| l.trim_end().ends_with(':'));
            if block_mode {
                // A blank final line submits the block.
                if buffer.ends_with("\n\n") || buffer.ends_with("\n\r\n") {
                    InputState::Complete
                } else {
                    InputState::NeedsMore
                }
            } else if trimmed.ends_with('.') {
                InputState::Complete
            } else {
                InputState::NeedsMore
            }
        }
    }
}

/// The indentation to pre-fill on a continuation line: 4 spaces after a
/// line that opened a block with `:`, otherwise none.
pub fn continuation_indent(buffer: &str) -> &'static str {
    let last = buffer.lines().last().unwrap_or("").trim_end();
    if last.ends_with(':') {
        "    "
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logic_sentences_end_with_period_or_question() {
        assert_eq!(input_state(Mode::Logic, "Every man is mortal."), InputState::Complete);
        assert_eq!(input_state(Mode::Logic, "Is Socrates mortal?"), InputState::Complete);
        assert_eq!(input_state(Mode::Logic, "Every man"), InputState::NeedsMore);
    }

    #[test]
    fn imperative_statement_ends_with_period() {
        assert_eq!(input_state(Mode::Imperative, "Show 1."), InputState::Complete);
        assert_eq!(input_state(Mode::Imperative, "Let x be"), InputState::NeedsMore);
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
        assert_eq!(input_state(Mode::Logic, ":format latex"), InputState::Complete);
    }

    #[test]
    fn continuation_indents_after_block_opener() {
        assert_eq!(continuation_indent("If x is less than 2:"), "    ");
        assert_eq!(continuation_indent("Show 1."), "");
    }
}
