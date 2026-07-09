//! The terminal (TTY) side of `largo repl`: rustyline-powered line editing
//! with history, English-vocabulary tab completion, and multi-line
//! continuation prompts.

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Editor, Helper};

use super::multiline::{continuation_indent, input_state, InputState};
use super::{Action, ReplState};

/// Statement-leading keywords every LOGOS program uses.
const KEYWORDS: &[&str] = &[
    "Let ", "Show ", "Set ", "If ", "While ", "Repeat ", "Push ", "Pop ",
    "Add ", "Remove ", "Return ", "Increase ", "Decrease ", "Inspect ",
    "Assert ", "Break", "## To ", "## Main", "## A ",
];

/// Meta-commands offered after a `:` prefix.
const META: &[&str] = &[
    ":help", ":quit", ":mode", ":logic", ":imperative", ":format", ":readings",
    ":discourse", ":reset", ":vars", ":type", ":program", ":load", ":save",
];

/// The rustyline helper: keyword + meta-command + live-binding completion.
struct LogosHelper {
    /// Current user bindings (refreshed after each eval).
    bindings: Vec<String>,
}

impl Helper for LogosHelper {}
impl Hinter for LogosHelper {
    type Hint = String;
}
impl Highlighter for LogosHelper {
    /// Live syntax highlighting from the language crate's classifier — the
    /// exact brain the LSP paints with (`token_class`), in ANSI.
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
        std::borrow::Cow::Owned(super::highlight::highlight_line(line))
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        true
    }
}
impl Validator for LogosHelper {}

impl Completer for LogosHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let head = &line[..pos];
        let word_start = head
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let word = &head[word_start..];
        if word.is_empty() {
            return Ok((pos, Vec::new()));
        }

        let mut candidates: Vec<Pair> = Vec::new();
        let mut offer = |s: &str| {
            if s.starts_with(word) && s != word {
                candidates.push(Pair { display: s.trim_end().to_string(), replacement: s.to_string() });
            }
        };
        if word.starts_with(':') {
            for m in META {
                offer(m);
            }
        } else {
            // At line start, offer statement keywords (which include their
            // trailing space); mid-line, offer bindings.
            if word_start == 0 {
                for k in KEYWORDS {
                    offer(k);
                }
            }
            for b in &self.bindings {
                offer(b);
            }
        }
        Ok((word_start, candidates))
    }
}

/// The interactive loop: banner, prompts, history, Ctrl-C clears the
/// buffer, Ctrl-D (or `:quit`) exits.
pub(crate) fn run_editor(state: &mut ReplState) -> Result<(), Box<dyn std::error::Error>> {
    let mut rl: Editor<LogosHelper, DefaultHistory> = Editor::new()?;
    rl.set_helper(Some(LogosHelper { bindings: Vec::new() }));

    let history_path = dirs::config_dir().map(|d| d.join("logos/history"));
    if let Some(path) = &history_path {
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
        let _ = rl.load_history(path);
    }

    anstream::println!(
        "LOGOS repl {} — :help for commands, :quit to leave",
        env!("CARGO_PKG_VERSION")
    );

    let mut buffer = String::new();
    loop {
        let prompt = if buffer.is_empty() {
            state.mode.prompt().to_string()
        } else {
            format!("  ..> {}", continuation_indent(&buffer))
        };
        match rl.readline(&prompt) {
            Ok(line) => {
                if buffer.is_empty() && line.trim().is_empty() {
                    continue;
                }
                buffer.push_str(&line);
                buffer.push('\n');
                if input_state(state.mode, &buffer) == InputState::Complete {
                    let input = std::mem::take(&mut buffer);
                    let _ = rl.add_history_entry(input.trim_end());
                    if state.handle(&input) == Action::Quit {
                        break;
                    }
                    // Completion feed: a textual scan — NEVER vars(), which
                    // would re-execute the whole program a second time per
                    // submitted line.
                    if let Some(helper) = rl.helper_mut() {
                        helper.bindings = state.imperative.binding_names();
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: drop the current input, keep the session.
                buffer.clear();
            }
            Err(ReadlineError::Eof) => break,
            Err(e) => return Err(Box::new(e)),
        }
    }

    if let Some(path) = &history_path {
        let _ = rl.save_history(path);
    }
    Ok(())
}
