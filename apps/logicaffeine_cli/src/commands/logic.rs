//! `largo logic` — English → First-Order Logic from the terminal.
//!
//! The headline LOGOS feature as a command: compile an English sentence to
//! FOL in any output format, list every ambiguous reading, or run a whole
//! discourse with shared anaphora context. Output is bare FOL on stdout —
//! script-friendly by design.

use std::io::Read;
use std::path::PathBuf;

use logicaffeine_language::{compile::*, CompileOptions, OutputFormat};

use crate::ui::{self, CliError};

/// The `--format` choices, mapped onto [`OutputFormat`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum LogicFormat {
    /// Unicode logic symbols: ∀, ∃, ∧, ∨, ¬, →.
    #[default]
    Unicode,
    /// LaTeX markup: \forall, \exists, \land, ...
    Latex,
    /// Plain ASCII FOL.
    Ascii,
    /// Kripke semantics: modals lowered to explicit world quantification.
    Kripke,
}

impl From<LogicFormat> for OutputFormat {
    fn from(f: LogicFormat) -> Self {
        match f {
            LogicFormat::Unicode => OutputFormat::Unicode,
            LogicFormat::Latex => OutputFormat::LaTeX,
            LogicFormat::Ascii => OutputFormat::SimpleFOL,
            LogicFormat::Kripke => OutputFormat::Kripke,
        }
    }
}

/// Handle `largo logic [SENTENCE] [--file F] [--format …] [--all-readings]
/// [--pragmatic] [--discourse]`.
pub(crate) fn cmd_logic(
    sentence: Option<String>,
    file: Option<PathBuf>,
    format: LogicFormat,
    all_readings: bool,
    pragmatic: bool,
    discourse: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let input = read_input(sentence, file)?;
    let options = CompileOptions { format: format.into(), pragmatic };

    if discourse {
        let sentences: Vec<&str> = input
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let fol = compile_discourse_with_options(&sentences, options)
            .map_err(|e| parse_failure(&e, &input))?;
        println!("{fol}");
        return Ok(());
    }

    if all_readings {
        let readings = collect_readings(&input, options);
        if readings.is_empty() {
            // No enumerated reading: fall back to the direct path — its
            // success is reading 1, its failure is the error to surface.
            return match compile_with_options(&input, options) {
                Ok(fol) => {
                    println!("1. {fol}");
                    Ok(())
                }
                Err(e) => Err(parse_failure(&e, &input).into()),
            };
        }
        for (i, reading) in readings.iter().enumerate() {
            println!("{}. {}", i + 1, reading);
        }
        return Ok(());
    }

    let fol = compile_with_options(&input, options).map_err(|e| parse_failure(&e, &input))?;
    println!("{fol}");
    Ok(())
}

/// Every reading of a sentence: quantifier-scope permutations first, then
/// parse-forest readings, deduplicated in that order. Shared by
/// `largo logic --all-readings` and the REPL's `:readings`.
pub(crate) fn collect_readings(input: &str, options: CompileOptions) -> Vec<String> {
    let mut readings: Vec<String> = Vec::new();
    for r in compile_all_scopes_with_options(input, options).unwrap_or_default() {
        if !readings.contains(&r) {
            readings.push(r);
        }
    }
    for r in compile_forest_with_options(input, options) {
        if !readings.contains(&r) {
            readings.push(r);
        }
    }
    readings
}

/// Resolve the input text: inline SENTENCE > `--file` > piped stdin.
fn read_input(
    sentence: Option<String>,
    file: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error>> {
    let raw = if let Some(s) = sentence {
        s
    } else if let Some(path) = file {
        std::fs::read_to_string(&path)
            .map_err(|e| CliError::new(format!("cannot read {}: {e}", path.display())))?
    } else {
        // On a terminal with nothing piped, don't sit silently waiting for
        // EOF — that reads as a hang.
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            return Err(CliError::with_hint(
                "no input to compile",
                "pass a sentence (largo logic \"Every cat sleeps.\"), a --file, or pipe text on stdin — or use `largo repl --logic` for an interactive session",
            )
            .exit_code(ui::EXIT_USAGE)
            .into());
        }
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    };
    let input = raw.trim().to_string();
    if input.is_empty() {
        return Err(CliError::with_hint(
            "no input to compile",
            "pass a sentence (largo logic \"Every cat sleeps.\"), a --file, or pipe text on stdin",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    Ok(input)
}

/// Render a parse failure with the caret-underlined source excerpt the
/// engine produces.
fn parse_failure(e: &logicaffeine_language::ParseError, source: &str) -> CliError {
    CliError::new(format!("could not parse the sentence\n{}", e.display_with_source(source)))
}
