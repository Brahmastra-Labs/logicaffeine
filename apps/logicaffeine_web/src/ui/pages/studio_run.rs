//! What Execute does with a Logic or a Math document, for every surface that runs one.
//!
//! A Logic document is a `## Theorem` block, compiled and handed to the prover, or plain English
//! sentences, each read into logic. A Math document is a `## Theory` development, every theorem
//! certified on its own, or kernel statements run in order from a fresh kernel, so each run
//! replaces the last. The front page's mini Studio runs its examples through these, so it shows
//! what the Studio shows; a Code program runs on `interpret_for_ui_baseline`, and a Hardware design
//! is read by `hardware_report::analyze`.

use logicaffeine_compile::{compile_for_proof, compile_for_ui, compile_theorem_for_ui, CompileResult, ProofCompileResult, TheoremCompileResult};
use logicaffeine_kernel::interface::Repl;

use crate::ui::pages::studio::parse_math_statements;
use crate::ui::state::ReplLine;

/// What running a Logic document produced.
pub enum LogicRun {
    /// A `## Theorem:` block: its compiled result, or why it did not compile.
    Theorem(Result<TheoremCompileResult, String>),
    /// Plain sentences: their readings, and the first of them compiled for the prover.
    Sentences(CompileResult, ProofCompileResult),
    /// Nothing but headers and comments.
    Empty,
}

pub fn run_logic(text: &str) -> LogicRun {
    if text.contains("## Theorem:") {
        let result = compile_theorem_for_ui(text);
        return LogicRun::Theorem(match result.error.clone() {
            Some(error) => Err(error),
            None => Ok(result),
        });
    }
    // Plain English: markdown headers (#) and LOGOS comments (--) are not sentences.
    let sentences: Vec<&str> = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("--")
        })
        .collect();
    match sentences.first() {
        // The proof engine works on the first sentence.
        Some(first) => LogicRun::Sentences(compile_for_ui(&sentences.join("\n")), compile_for_proof(first)),
        None => LogicRun::Empty,
    }
}

/// What running a Math document produced: a line per statement, and, when the statements ran
/// through the kernel, the kernel they left behind, whose environment the Studio's Context panel
/// lists. A `## Theory` development certifies its theorems on its own and leaves no kernel.
pub struct MathRun {
    pub lines: Vec<ReplLine>,
    pub kernel: Option<Repl>,
}

/// Runs a Math document from a fresh kernel, so each run replaces the last rather than piling on.
pub fn run_math(code: &str) -> MathRun {
    if code.lines().any(|line| line.trim_start().starts_with("## Theory")) {
        let theory = logicaffeine_compile::ui_bridge::compile_theory_for_ui(code);
        let mut lines = Vec::new();
        if let Some(error) = &theory.parse_error {
            lines.push(ReplLine::error("## Theory".to_string(), error.clone()));
        } else {
            lines.push(ReplLine::success(
                format!("Theory {} ({} axioms)", theory.theory_name.clone().unwrap_or_default(), theory.axiom_count),
                format!("{} theorems checked", theory.theorems.len()),
            ));
            for theorem in &theory.theorems {
                if theorem.verified {
                    lines.push(ReplLine::success(format!("Theorem {}", theorem.name), "kernel-certified".to_string()));
                } else {
                    lines.push(ReplLine::error(
                        format!("Theorem {}", theorem.name),
                        theorem.error.clone().unwrap_or_else(|| "not certified".to_string()),
                    ));
                }
            }
        }
        return MathRun { lines, kernel: None };
    }

    let mut kernel = Repl::new();
    let lines = parse_math_statements(code)
        .into_iter()
        .map(|statement| match kernel.execute(&statement) {
            Ok(output) => ReplLine::success(statement, output),
            Err(error) => ReplLine::error(statement, error.to_string()),
        })
        .collect();
    MathRun { lines, kernel: Some(kernel) }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio_run/tests.rs"]
mod tests;
