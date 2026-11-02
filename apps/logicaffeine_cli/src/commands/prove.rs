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

use std::path::PathBuf;

use anstyle::AnsiColor;
use logicaffeine_compile::kernel::certificate::TheoremCertificate;
use logicaffeine_compile::ui_bridge::compile_theory_for_ui;
use logicaffeine_compile::{prove_theorem_trace, theorem_certificate};

use crate::commands::require_project_root;
use crate::project::manifest::Manifest;
use crate::ui::CliError;

/// One proved (or refuted) theorem, however it was reached.
pub(crate) struct Outcome {
    pub(crate) name: String,
    pub(crate) verified: bool,
    pub(crate) error: Option<String>,
    pub(crate) trace: Option<String>,
    /// The independently re-checkable proof certificate. Present ONLY for a proved
    /// English `## Theorem` (the surface that exposes a kernel proof term); `None`
    /// for the Tarski development path (which does not surface a term yet) and for
    /// any unproved goal.
    pub(crate) certificate: Option<TheoremCertificate>,
}

/// Everything one proving pass over a source text established.
pub(crate) struct ProveReport {
    pub(crate) axiom_count: usize,
    /// The NAMES of every admitted axiom — the inventory reports them as named
    /// assumptions and rejects a theorem/axiom name collision (G13-full).
    pub(crate) axiom_names: Vec<String>,
    pub(crate) outcomes: Vec<Outcome>,
}

/// The proving core shared by `cmd_prove` and the MCP `logos_prove` tool:
/// every development theorem and English theorem block in `source`, proved
/// and kernel-certified. `origin` names the source in error messages, and
/// `locate` turns an offset in `source` into the `file:line:col` the author
/// opens — through the closure's `SourceMap` when `source` was assembled.
pub(crate) fn prove_outcomes(
    source: &str,
    origin: &str,
    locate: &dyn Fn(usize) -> String,
) -> Result<ProveReport, CliError> {
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut axiom_count = 0usize;
    let mut axiom_names: Vec<String> = Vec::new();

    // Development path: formal `## Theory` / standalone `## Axiom` blocks.
    let has_development = source
        .lines()
        .any(|l| l.starts_with("## Theory") || l.starts_with("## Axiom"));
    if has_development {
        let result = compile_theory_for_ui(source);
        if let Some(err) = result.parse_error {
            return Err(CliError::new(format!(
                "the development in {origin} did not parse: {err}"
            )));
        }
        axiom_count = result.axiom_count;
        axiom_names = result.axiom_names;
        for t in result.theorems {
            // The Tarski development path (`prove_library_with_axioms`) does not
            // surface a kernel proof term today, so it carries no independent
            // certificate — reported honestly as `None`, never a fabricated one.
            outcomes.push(Outcome {
                name: t.name,
                verified: t.verified,
                error: t.error,
                trace: None,
                certificate: None,
            });
        }
    }

    // English path: each `## Theorem` block through the trace prover.
    let mut searched_from = 0usize;
    for block in split_theorem_blocks(source) {
        let name = theorem_name(&block);
        // Where the block starts in `source`: blocks are cut from it in order, line for line.
        let block_at = source[searched_from..]
            .find(block.as_str())
            .map(|i| searched_from + i)
            .unwrap_or(0);
        searched_from = block_at;
        match prove_theorem_trace(&block) {
            Ok(t) => {
                // Mint the independent certificate from the SAME kernel-certified
                // proof term the trace prover established (only on a real proof).
                let certificate = if t.verified {
                    theorem_certificate(&block).ok().flatten()
                } else {
                    None
                };
                outcomes.push(Outcome {
                    name,
                    verified: t.verified,
                    error: t.error,
                    trace: t.trace,
                    certificate,
                });
            }
            Err(e) => {
                let explanation = logicaffeine_language::socratic_explanation(
                    &e,
                    &logicaffeine_language::Interner::new(),
                );
                return Err(CliError::new(format!(
                    "theorem `{name}` does not parse: {explanation}\n  --> {}",
                    locate(block_at)
                )));
            }
        }
    }

    if outcomes.is_empty() {
        return Err(CliError::with_hint(
            format!("no theorems found in {origin}"),
            "add a `## Theorem` block or a `## Theory` development to prove",
        ));
    }

    // G13-full name-collision gate: a name that is BOTH a theorem and an admitted
    // axiom lets an assumption masquerade as a proved fact. Reject it loudly — the
    // inventory's kinds must partition, so every claim has exactly one kind.
    let theorem_claims: Vec<logicaffeine_compile::testrun::ProvenClaim> = outcomes
        .iter()
        .map(|o| logicaffeine_compile::testrun::ProvenClaim {
            name: o.name.clone(),
            verified: o.verified,
            // A kernel/English theorem is an UNBOUNDED proof (no finite bound to
            // disclose); a finite-domain theorem would carry its bound here.
            finite_bound: None,
            certificate: o.certificate.clone(),
        })
        .collect();
    logicaffeine_compile::testrun::Inventory::from_prove(&theorem_claims, &axiom_names, &[])
        .map_err(CliError::new)?;

    Ok(ProveReport {
        axiom_count,
        axiom_names,
        outcomes,
    })
}

/// Handle `largo prove [FILE] [--trace] [--json] [--certificate]`.
pub(crate) fn cmd_prove(
    file: Option<PathBuf>,
    trace: bool,
    json: bool,
    certificate: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Prove the ASSEMBLED module closure — the same source `largo test`/`build`
    // consume — so a theorem or property in an IMPORTED module is proved, not
    // silently skipped. Byte-identical to the raw entry for a single-file project.
    let (path, closure) = match file {
        Some(f) => {
            let closure = crate::closure::assemble_closure(&f)
                .map_err(|e| CliError::new(format!("could not assemble project: {e}")))?;
            (f, closure)
        }
        None => {
            let root = require_project_root()?;
            let manifest = Manifest::load(&root)?;
            let entry = crate::commands::resolve_entry_path(&root, &manifest)?;
            let dep_roots = crate::project::deps::resolve_dep_roots(&root, &manifest)?;
            let closure = crate::closure::assemble_closure_with_deps(&entry, dep_roots)
                .map_err(|e| CliError::new(format!("could not assemble project: {e}")))?;
            (entry, closure)
        }
    };
    let source = closure.source.as_str();
    let locate = |offset: usize| {
        let span = logicaffeine_language::token::Span::new(offset, offset);
        crate::diagnostics::Placed::locate(span, source, &closure.source_map, &path).pointer()
    };

    let report = prove_outcomes(source, &path.display().to_string(), &locate)?;
    let ProveReport {
        axiom_count,
        axiom_names,
        outcomes,
    } = report;

    let all_verified = outcomes.iter().all(|o| o.verified);

    if certificate {
        println!("{}", render_certificates(&path, &outcomes));
    } else if json {
        println!(
            "{}",
            render_json(&path, axiom_count, &axiom_names, &outcomes, all_verified)
        );
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

/// Render the machine-readable report — including the G13-full claim INVENTORY:
/// every theorem classified `proved`/`unproved` (a kernel-certified theorem is
/// `proof: true`), and every admitted axiom reported as an ASSUMPTION
/// (`proof: false`), never a proven fact. The legacy `theorems`/`axioms` fields
/// are retained so existing consumers keep working.
fn render_json(
    path: &std::path::Path,
    axioms: usize,
    axiom_names: &[String],
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
    // The unified inventory. The name-collision gate already ran in
    // `prove_outcomes`, so this construction cannot fail here.
    let theorem_claims: Vec<logicaffeine_compile::testrun::ProvenClaim> = outcomes
        .iter()
        .map(|o| logicaffeine_compile::testrun::ProvenClaim {
            name: o.name.clone(),
            verified: o.verified,
            finite_bound: None,
            certificate: o.certificate.clone(),
        })
        .collect();
    let inventory =
        logicaffeine_compile::testrun::Inventory::from_prove(&theorem_claims, axiom_names, &[])
            .expect("the name-collision gate already ran in prove_outcomes");
    serde_json::json!({
        "file": path.display().to_string(),
        "axioms": axioms,
        "theorems": theorems,
        "all_verified": all_verified,
        "inventory": inventory.items,
        "summary": inventory.summary,
    })
    .to_string()
}

/// Render every proved theorem's independently re-checkable CERTIFICATE as JSON.
///
/// Each entry carries the portable [`TheoremCertificate`] (the kernel proof term, the
/// goal, and the theorem's disclosed premises) AND the verdict of re-checking it HERE
/// against the kernel — by type-checking the term, not by re-running proof search. A
/// theorem with no certificate (the Tarski development path, which does not surface a
/// kernel term yet, or an unproved goal) is reported with `certificate: null` and the
/// honest `reason` why — never a fabricated proof.
fn render_certificates(path: &std::path::Path, outcomes: &[Outcome]) -> String {
    use logicaffeine_compile::kernel::certificate::recheck_theorem;

    let certs: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|o| match &o.certificate {
            Some(cert) => {
                let out = recheck_theorem(cert);
                serde_json::json!({
                    "name": o.name,
                    "verified": o.verified,
                    "certificate": cert,
                    "rechecks": out.rechecks,
                    "two_kernel_agreement": out.two_kernel_agreement,
                    "recheck_error": out.error,
                })
            }
            None => serde_json::json!({
                "name": o.name,
                "verified": o.verified,
                "certificate": serde_json::Value::Null,
                "reason": if o.verified {
                    "proved, but this proof surface does not expose a re-checkable kernel term \
                     (Tarski development path)"
                } else {
                    "not proved — an unproved goal has no proof term, hence no certificate"
                },
            }),
        })
        .collect();
    serde_json::json!({
        "file": path.display().to_string(),
        "prelude_version": logicaffeine_compile::kernel::certificate::PRELUDE_VERSION,
        "certificates": certs,
    })
    .to_string()
}

/// Whether a line is a `## Theorem` header — the word must end there
/// (`## Theorem`, `## Theorem:`, `## Theorem foo`), so prose headers like
/// `## Theorems overview` never match.
fn is_theorem_header(line: &str) -> bool {
    line.strip_prefix("## Theorem").is_some_and(|rest| {
        rest.is_empty() || rest.starts_with(':') || rest.starts_with(char::is_whitespace)
    })
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
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/prove/tests.rs"]
mod tests;
