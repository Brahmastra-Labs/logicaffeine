//! `largo prove` — kernel-certified theorem proving from source text.
//!
//! Two proof surfaces, both engine-native:
//!
//! - **Developments** (`## Theory` blocks with formal `Axiom`/`Theorem`
//!   declarations, plus standalone `## Axiom` blocks) run through the
//!   multi-theorem driver ([`compile_theory_for_ui`]) — the Tarski path.
//! - **English theorems** (`## Theorem:` blocks with `Given:`/`Prove:`
//!   lines) prove one by one through the backward chainer with a rendered
//!   derivation trace ([`prove_theorem_trace`]).
//!
//! Every ✓ is kernel-certified — a derivation alone never counts.

use std::fs;
use std::path::PathBuf;

use anstyle::AnsiColor;
use logicaffeine_compile::ui_bridge::compile_theory_for_ui;
use logicaffeine_compile::prove_theorem_trace;

use crate::commands::require_project_root;
use crate::project::manifest::Manifest;
use crate::ui::CliError;

/// One proved (or refuted) theorem, however it was reached.
struct Outcome {
    name: String,
    verified: bool,
    error: Option<String>,
    trace: Option<String>,
}

/// Handle `largo prove [FILE] [--trace] [--json]`.
pub(crate) fn cmd_prove(
    file: Option<PathBuf>,
    trace: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = match file {
        Some(f) => f,
        None => {
            let root = require_project_root()?;
            let manifest = Manifest::load(&root)?;
            root.join(&manifest.package.entry)
        }
    };
    let source = fs::read_to_string(&path)
        .map_err(|e| CliError::new(format!("cannot read {}: {e}", path.display())))?;

    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut axiom_count = 0usize;

    // Development path: formal `## Theory` / standalone `## Axiom` blocks.
    let has_development = source
        .lines()
        .any(|l| l.starts_with("## Theory") || l.starts_with("## Axiom"));
    if has_development {
        let result = compile_theory_for_ui(&source);
        if let Some(err) = result.parse_error {
            return Err(CliError::new(format!(
                "the development in {} did not parse: {err}",
                path.display()
            ))
            .into());
        }
        axiom_count = result.axiom_count;
        for t in result.theorems {
            outcomes.push(Outcome { name: t.name, verified: t.verified, error: t.error, trace: None });
        }
    }

    // English path: each `## Theorem` block through the trace prover.
    for block in split_theorem_blocks(&source) {
        let name = theorem_name(&block);
        match prove_theorem_trace(&block) {
            Ok(t) => outcomes.push(Outcome {
                name,
                verified: t.verified,
                error: t.error,
                trace: t.trace,
            }),
            Err(e) => {
                return Err(CliError::new(format!(
                    "theorem `{name}` in {} did not parse: {e:?}",
                    path.display()
                ))
                .into());
            }
        }
    }

    if outcomes.is_empty() {
        return Err(CliError::with_hint(
            format!("no theorems found in {}", path.display()),
            "add a `## Theorem` block or a `## Theory` development to prove",
        )
        .into());
    }

    let all_verified = outcomes.iter().all(|o| o.verified);

    if json {
        println!("{}", render_json(&path, axiom_count, &outcomes, all_verified));
    } else {
        render_human(&path, axiom_count, &outcomes, trace);
    }

    if all_verified {
        Ok(())
    } else {
        let failed = outcomes.iter().filter(|o| !o.verified).count();
        Err(CliError::new(format!("{failed} theorem(s) failed to verify")).into())
    }
}

/// Render the ✓/✗ report to stdout.
fn render_human(path: &std::path::Path, axioms: usize, outcomes: &[Outcome], trace: bool) {
    let green = AnsiColor::Green.on_default().bold();
    let red = AnsiColor::Red.on_default().bold();
    if axioms > 0 {
        anstream::println!("Proving {} ({axioms} axiom(s) in scope)", path.display());
    } else {
        anstream::println!("Proving {}", path.display());
    }
    for o in outcomes {
        if o.verified {
            anstream::println!("{green}✓{green:#} {}", o.name);
        } else {
            match &o.error {
                Some(err) => anstream::println!("{red}✗{red:#} {} — {err}", o.name),
                None => anstream::println!("{red}✗{red:#} {} — not proved", o.name),
            }
        }
        if trace {
            if let Some(t) = &o.trace {
                for line in t.lines() {
                    anstream::println!("    {line}");
                }
            }
        }
    }
}

/// Render the machine-readable report.
fn render_json(
    path: &std::path::Path,
    axioms: usize,
    outcomes: &[Outcome],
    all_verified: bool,
) -> String {
    let theorems: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|o| {
            serde_json::json!({
                "name": o.name,
                "verified": o.verified,
                "error": o.error,
            })
        })
        .collect();
    serde_json::json!({
        "file": path.display().to_string(),
        "axioms": axioms,
        "theorems": theorems,
        "all_verified": all_verified,
    })
    .to_string()
}

/// Whether a line is a `## Theorem` header — the word must end there
/// (`## Theorem`, `## Theorem:`, `## Theorem foo`), so prose headers like
/// `## Theorems overview` never match.
fn is_theorem_header(line: &str) -> bool {
    line.strip_prefix("## Theorem")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(':') || rest.starts_with(char::is_whitespace))
}

/// Split out each `## Theorem` block: from its header line to the next
/// `## ` header (exclusive).
fn split_theorem_blocks(source: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in source.lines() {
        if is_theorem_header(line) {
            if let Some(done) = current.take() {
                blocks.push(done);
            }
            current = Some(format!("{line}\n"));
        } else if line.starts_with("## ") {
            if let Some(done) = current.take() {
                blocks.push(done);
            }
        } else if let Some(block) = current.as_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    if let Some(done) = current.take() {
        blocks.push(done);
    }
    blocks
}

/// The display name of an English theorem block (`## Theorem: Name`).
fn theorem_name(block: &str) -> String {
    block
        .lines()
        .next()
        .and_then(|l| l.split_once(':'))
        .map(|(_, name)| name.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "theorem".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitter_finds_each_theorem_block() {
        let src = "## Note\n\nx\n\n## Theorem: A\nGiven: p.\nProve: p.\n\n## Theorem: B\nProve: q.\n\n## Main\nShow 1.\n";
        let blocks = split_theorem_blocks(src);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].starts_with("## Theorem: A"));
        assert!(blocks[0].contains("Given: p."));
        assert!(!blocks[0].contains("## Theorem: B"));
        assert!(blocks[1].starts_with("## Theorem: B"));
        assert!(!blocks[1].contains("Show 1."), "Main must not leak into a block");
    }

    #[test]
    fn splitter_ignores_theory_blocks() {
        let src = "## Theory T\n\nAxiom a: for all x, P(x).\nTheorem t: prove for all x, P(x).\n";
        assert!(split_theorem_blocks(src).is_empty());
    }

    #[test]
    fn splitter_requires_a_word_boundary_after_theorem() {
        // `## Theorems overview` is prose, not a theorem block.
        let src = "## Theorems overview\n\nSome prose.\n\n## Theorem: Real\nProve: p.\n";
        let blocks = split_theorem_blocks(src);
        assert_eq!(blocks.len(), 1, "only the real block: {blocks:?}");
        assert!(blocks[0].starts_with("## Theorem: Real"));
    }

    #[test]
    fn theorem_name_extracts_after_colon() {
        assert_eq!(theorem_name("## Theorem: Socrates\nProve: x.\n"), "Socrates");
        assert_eq!(theorem_name("## Theorem\nProve: x.\n"), "theorem");
    }
}
