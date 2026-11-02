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
            let block_mode =
                trimmed.starts_with("## ") || trimmed.lines().any(|l| l.trim_end().ends_with(':'));
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
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/repl/multiline/tests.rs"]
mod tests;
