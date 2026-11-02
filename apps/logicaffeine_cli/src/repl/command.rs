//! REPL meta-commands (`:help`, `:quit`, `:format latex`, …) — a pure
//! parser from an input line to a [`MetaCommand`].

use std::path::PathBuf;

use crate::commands::logic::LogicFormat;

/// Which language the REPL is currently speaking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Imperative LOGOS statements against the persistent session.
    Imperative,
    /// English sentences → FOL.
    Logic,
}

impl Mode {
    /// The prompt for this mode.
    pub fn prompt(self) -> &'static str {
        match self {
            Mode::Imperative => "logos> ",
            Mode::Logic => "logic> ",
        }
    }
}

/// A parsed meta-command.
#[derive(Debug, PartialEq, Eq)]
pub enum MetaCommand {
    /// `:help` — show the command table.
    Help,
    /// `:quit` / `:q` — leave the REPL.
    Quit,
    /// `:mode` — print the current mode.
    ModeShow,
    /// `:mode logic|imperative`, `:logic`, `:imperative` — switch modes.
    ModeSet(Mode),
    /// `:format unicode|latex|ascii|kripke` — logic output format.
    Format(LogicFormat),
    /// `:readings` — every reading of the last logic sentence.
    Readings,
    /// `:discourse on|off` — shared anaphora context across sentences.
    Discourse(bool),
    /// `:reset` — clear the current mode's session.
    Reset,
    /// `:vars` / `:env` — the imperative session's global bindings.
    Vars,
    /// `:type <name>` — the type of one binding.
    Type(String),
    /// `:program` — the accumulated imperative program.
    Program,
    /// `:load <path>` — load a saved program / discourse.
    Load(PathBuf),
    /// `:save <path>` — save the session.
    Save(PathBuf),
    /// `:explain <word>` — teach one construct from the shared lesson table.
    Explain(String),
}

/// Parse a line as a meta-command. `None` when the line isn't one
/// (doesn't start with `:`); `Some(Err(…))` for an unknown or malformed
/// meta-command, with a message pointing at `:help`.
pub fn parse_meta(line: &str) -> Option<Result<MetaCommand, String>> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix(':')?;
    let mut words = rest.split_whitespace();
    let head = words.next().unwrap_or("");

    // Path-taking commands take the REST of the line verbatim, so paths with
    // spaces work without quoting.
    if head == "load" || head == "save" {
        let path = rest[head.len()..].trim();
        if path.is_empty() {
            return Some(Err(format!(":{head} needs a file path")));
        }
        return Some(Ok(match head {
            "load" => MetaCommand::Load(PathBuf::from(path)),
            _ => MetaCommand::Save(PathBuf::from(path)),
        }));
    }

    let arg = words.next();
    let extra = words.next();

    let need_no_arg = |cmd: MetaCommand| -> Result<MetaCommand, String> {
        if arg.is_some() {
            Err(format!(":{head} takes no argument (see :help)"))
        } else {
            Ok(cmd)
        }
    };

    let parsed = match head {
        "help" | "h" | "?" => need_no_arg(MetaCommand::Help),
        "quit" | "q" | "exit" => need_no_arg(MetaCommand::Quit),
        "logic" => need_no_arg(MetaCommand::ModeSet(Mode::Logic)),
        "imperative" => need_no_arg(MetaCommand::ModeSet(Mode::Imperative)),
        "mode" => match arg {
            None => Ok(MetaCommand::ModeShow),
            Some("logic") => Ok(MetaCommand::ModeSet(Mode::Logic)),
            Some("imperative") => Ok(MetaCommand::ModeSet(Mode::Imperative)),
            Some(other) => Err(format!("unknown mode `{other}` (logic | imperative)")),
        },
        "format" => match arg {
            Some("unicode") => Ok(MetaCommand::Format(LogicFormat::Unicode)),
            Some("latex") => Ok(MetaCommand::Format(LogicFormat::Latex)),
            Some("ascii") => Ok(MetaCommand::Format(LogicFormat::Ascii)),
            Some("kripke") => Ok(MetaCommand::Format(LogicFormat::Kripke)),
            Some(other) => Err(format!(
                "unknown format `{other}` (unicode | latex | ascii | kripke)"
            )),
            None => Err(":format needs an argument (unicode | latex | ascii | kripke)".into()),
        },
        "readings" => need_no_arg(MetaCommand::Readings),
        "discourse" => match arg {
            Some("on") => Ok(MetaCommand::Discourse(true)),
            Some("off") => Ok(MetaCommand::Discourse(false)),
            _ => Err(":discourse needs `on` or `off`".into()),
        },
        "reset" => need_no_arg(MetaCommand::Reset),
        "vars" | "env" => need_no_arg(MetaCommand::Vars),
        "type" => match arg {
            Some(name) => Ok(MetaCommand::Type(name.to_string())),
            None => Err(":type needs a binding name".into()),
        },
        "explain" => match arg {
            Some(word) => Ok(MetaCommand::Explain(word.to_string())),
            None => Err(":explain needs a word (try :explain give)".into()),
        },
        "program" | "source" => need_no_arg(MetaCommand::Program),
        other => Err(format!(
            "unknown command `:{other}` — type :help for the list"
        )),
    };

    // A stray extra argument is always an error.
    if extra.is_some() {
        return Some(Err(format!(":{head} got too many arguments (see :help)")));
    }
    Some(parsed)
}

/// The `:help` text.
pub const HELP: &str = "\
Commands:
  :help                     show this table
  :quit (:q)                leave the REPL
  :mode [logic|imperative]  show or switch the mode (:logic / :imperative)
  :format <fmt>             logic output: unicode | latex | ascii | kripke
  :readings                 every reading of the last logic sentence
  :discourse on|off         shared anaphora context across sentences
  :vars (:env)              the session's bindings        [imperative]
  :type <name>              the type of one binding       [imperative]
  :explain <word>           teach one construct (keyword, type, or ## block)
  :program                  the accumulated program       [imperative]
  :reset                    clear the current mode's session
  :load <file>              load a saved program
  :save <file>              save the session as a runnable program

Imperative statements end with a period; blocks (If …:, ## To …:) submit
on a blank line. Note: each line re-runs the accumulated program, so
non-deterministic effects (time, random, I/O) are re-evaluated as the
session grows — :reset starts clean.";

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/repl/command/tests.rs"]
mod tests;
