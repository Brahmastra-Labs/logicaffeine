//! `largo repl` — the interactive LOGOS session.
//!
//! Two modes behind one loop:
//!
//! - **imperative** (`logos>`): statements run against a persistent
//!   [`ReplSession`] — the replay engine in `logicaffeine_compile` (the
//!   exact `largo run --interpret` engine, so REPL semantics never drift
//!   from program semantics).
//! - **logic** (`logic>`): English sentences compile to FOL through a
//!   persistent discourse [`Session`] (cross-sentence anaphora), with
//!   `:format`, `:readings`, and `:discourse` control.
//!
//! On a terminal the loop runs through rustyline (history, completion,
//! Ctrl-C clears / Ctrl-D exits); on a pipe it degrades to a plain line
//! loop with no prompts or color — `echo '…' | largo repl` is scriptable
//! and is exactly what the e2e suite drives.

pub(crate) mod command;
pub(crate) mod editor;
mod highlight;
pub(crate) mod multiline;
pub(crate) mod render;

use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};

use logicaffeine_compile::ReplSession;
use logicaffeine_language::{CompileOptions, Session};

use crate::commands::logic::{collect_readings, LogicFormat};
use crate::ui;
use command::{parse_meta, MetaCommand, Mode, HELP};
use multiline::{input_state, InputState};
use render::{colorize_fol, render_vars};

/// What the loop should do after handling one input.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Continue,
    Quit,
}

/// The whole REPL state: both sessions live side by side, so switching
/// modes never loses work.
pub(crate) struct ReplState {
    mode: Mode,
    imperative: ReplSession,
    logic: Session,
    format: LogicFormat,
    discourse: bool,
    last_sentence: Option<String>,
    /// Insert ANSI colors into rendered FOL (terminal sessions only).
    color: bool,
}

impl ReplState {
    fn new(start_in_logic: bool, format: LogicFormat, color: bool) -> Self {
        ReplState {
            mode: if start_in_logic { Mode::Logic } else { Mode::Imperative },
            imperative: ReplSession::new(),
            logic: Session::with_format(format.into()),
            format,
            discourse: true,
            last_sentence: None,
            color,
        }
    }

    /// Handle one complete input (a statement/sentence/block or a
    /// meta-command).
    fn handle(&mut self, input: &str) -> Action {
        let input = input.trim_end();
        if input.trim().is_empty() {
            return Action::Continue;
        }
        if let Some(meta) = parse_meta(input) {
            return match meta {
                Ok(cmd) => self.dispatch(cmd),
                Err(msg) => {
                    print_error(&msg);
                    Action::Continue
                }
            };
        }
        match self.mode {
            Mode::Imperative => self.eval_imperative(input),
            Mode::Logic => self.eval_logic(input),
        }
        Action::Continue
    }

    fn eval_imperative(&mut self, input: &str) {
        let outcome = self.imperative.eval_sync(input);
        for line in &outcome.new_lines {
            anstream::println!("{line}");
        }
        if let Some(err) = outcome.error {
            print_error(&err);
        }
    }

    fn eval_logic(&mut self, sentence: &str) {
        if !self.discourse {
            self.logic = Session::with_format(self.format.into());
        }
        match self.logic.eval(sentence) {
            Ok(fol) => {
                anstream::println!("{}", colorize_fol(&fol, self.color));
                self.last_sentence = Some(sentence.to_string());
            }
            Err(e) => print_error(&format!("could not parse: {e:?}")),
        }
    }

    fn dispatch(&mut self, cmd: MetaCommand) -> Action {
        match cmd {
            MetaCommand::Quit => return Action::Quit,
            MetaCommand::Help => anstream::println!("{HELP}"),
            MetaCommand::ModeShow => anstream::println!(
                "{}",
                match self.mode {
                    Mode::Imperative => "imperative",
                    Mode::Logic => "logic",
                }
            ),
            MetaCommand::ModeSet(mode) => self.mode = mode,
            MetaCommand::Format(format) => {
                self.format = format;
                self.logic.set_format(format.into());
            }
            MetaCommand::Readings => match &self.last_sentence {
                Some(sentence) => {
                    let options =
                        CompileOptions { format: self.format.into(), pragmatic: false };
                    let readings = collect_readings(sentence, options);
                    if readings.is_empty() {
                        print_error("no readings for the last sentence");
                    }
                    for (i, r) in readings.iter().enumerate() {
                        anstream::println!("{}. {}", i + 1, colorize_fol(r, self.color));
                    }
                }
                None => print_error("no sentence yet — say something in logic mode first"),
            },
            MetaCommand::Discourse(on) => {
                self.discourse = on;
                if !on {
                    self.logic = Session::with_format(self.format.into());
                }
            }
            MetaCommand::Reset => match self.mode {
                Mode::Imperative => self.imperative.reset(),
                Mode::Logic => {
                    self.logic = Session::with_format(self.format.into());
                    self.last_sentence = None;
                }
            },
            MetaCommand::Vars => {
                anstream::println!("{}", render_vars(&self.imperative.vars()));
            }
            MetaCommand::Type(name) => {
                match self.imperative.vars().iter().find(|(n, _, _)| *n == name) {
                    Some((_, ty, _)) => anstream::println!("{name} : {ty}"),
                    None => print_error(&format!("`{name}` is not bound")),
                }
            }
            MetaCommand::Program => anstream::print!("{}", self.imperative.source()),
            MetaCommand::Explain(word) => {
                anstream::println!("{}", render::render_explain(&word, self.color));
            }
            MetaCommand::Save(path) => self.save(&path),
            MetaCommand::Load(path) => self.load(&path),
        }
        Action::Continue
    }

    fn save(&self, path: &Path) {
        let content = match self.mode {
            Mode::Imperative => self.imperative.source(),
            Mode::Logic => self.logic.history(),
        };
        match std::fs::write(path, content) {
            Ok(()) => anstream::println!("Saved {}", path.display()),
            Err(e) => print_error(&format!("cannot save {}: {e}", path.display())),
        }
    }

    fn load(&mut self, path: &Path) {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                print_error(&format!("cannot load {}: {e}", path.display()));
                return;
            }
        };
        match self.mode {
            Mode::Imperative => {
                let outcome = self.imperative.load_source(&source);
                for line in &outcome.new_lines {
                    anstream::println!("{line}");
                }
                if let Some(err) = outcome.error {
                    print_error(&err);
                }
            }
            Mode::Logic => {
                for line in source.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    self.eval_logic(line);
                }
            }
        }
    }
}

/// Print a REPL-level error (conversational — never kills the session).
fn print_error(msg: &str) {
    anstream::eprintln!("{}error:{:#} {msg}", ui::ERROR_STYLE, ui::ERROR_STYLE);
}

/// Handle `largo repl [--logic] [--format F] [--load FILE]`.
pub(crate) fn cmd_repl(
    logic: bool,
    format: Option<LogicFormat>,
    load: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let interactive = std::io::stdin().is_terminal();
    let mut state = ReplState::new(logic, format.unwrap_or_default(), interactive);
    if let Some(path) = load {
        state.load(&path);
    }
    if interactive {
        editor::run_editor(&mut state)
    } else {
        run_pipe(&mut state)
    }
}

/// The non-TTY loop: plain lines in, results out — no prompts, no banner.
fn run_pipe(state: &mut ReplState) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let mut buffer = String::new();
    for line in stdin.lock().lines() {
        let line = line?;
        if buffer.is_empty() && line.trim().is_empty() {
            continue;
        }
        buffer.push_str(&line);
        buffer.push('\n');
        if input_state(state.mode, &buffer) == InputState::Complete {
            let input = std::mem::take(&mut buffer);
            if state.handle(&input) == Action::Quit {
                return Ok(());
            }
        }
    }
    if !buffer.trim().is_empty() {
        state.handle(&buffer);
    }
    Ok(())
}
