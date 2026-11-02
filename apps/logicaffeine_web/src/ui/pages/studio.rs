//! Studio page - multi-mode playground for Logic, Code, and Math.
//!
//! The main development environment with three modes:
//!
//! - **Logic Mode**: Parse English sentences to First-Order Logic with AST
//!   visualization and proof checking
//! - **Code Mode**: Write imperative LOGOS code with REPL execution and
//!   Rust code generation
//! - **Math Mode**: Define theorems and types with interactive proofs and
//!   tactic guidance
//!
//! # Layout
//!
//! - **Left sidebar**: File browser with example files
//! - **Center**: Live editor with syntax highlighting
//! - **Right panel**: Context-sensitive output (FOL, AST, REPL, proofs)
//!
//! # Route
//!
//! Accessed via [`Route::Studio`](crate::ui::router::Route::Studio).

use crate::ui::components::context_view::{ContextEntry, EntryKind};
use crate::ui::components::logic_output::OutputFormat;
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::components::proof_panel::{ProofNode, ProofStatus};
use crate::ui::fault::{contain, ActionFault};
use crate::ui::pages::studio_files::examples_tree;
#[cfg(target_arch = "wasm32")]
use crate::ui::pages::studio_files::{example_text, load_dir_recursive, open_store, StudioFiles};
#[cfg(target_arch = "wasm32")]
use crate::ui::pages::studio_opening::Arrival;
use crate::ui::pages::studio_opening::StudioOpening;
use crate::ui::pages::studio_layout::MobileTab;
use crate::ui::pages::studio_run;
#[cfg(target_arch = "wasm32")]
use crate::ui::pages::studio_workbench::WorkbenchState;
use crate::ui::pages::studio_workbench::{use_workbench, Host, StoreNote, Workbench};
use crate::ui::responsive::{MOBILE_BASE_STYLES, MOBILE_TAB_BAR_STYLES};
use crate::ui::seo::{
    breadcrumb_schema, organization_schema, pages as seo_pages, software_application_schema,
    BreadcrumbItem, JsonLdMultiple, PageHead,
};
#[cfg(target_arch = "wasm32")]
use crate::ui::state::StudioMode;
use crate::ui::state::{FileNode, ReplLine};
use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;
use logicaffeine_compile::{CompileResult, ProofCompileResult, SolvedGrid, TheoremCompileResult};
use logicaffeine_kernel::interface::Repl;
use logicaffeine_proof::{hints::suggest_hint, DerivationTree, InferenceRule, ProofExpr};
#[cfg(target_arch = "wasm32")]
use logicaffeine_system::fs::{get_platform_vfs_with_fallback, Vfs, WebVfs};

/// Parse math code into complete statements.
///
/// Handles both Coq-style (period-terminated) and Literate syntax (block-based):
/// - `## To ...` blocks: collect header + all indented lines until non-indented line
/// - `A X is either:` blocks: collect header + indented variants
/// - Traditional commands: accumulate until period-terminator
pub fn parse_math_statements(code: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with("--") {
            i += 1;
            continue;
        }

        // Check for Literate function definition: "## To ..."
        if trimmed.starts_with("## To ") {
            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;

            // Collect all indented lines (body of the function)
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();

                // Empty lines within block are OK
                if next_trimmed.is_empty() {
                    i += 1;
                    continue;
                }

                // Comments within block are skipped
                if next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }

                // Check if line is indented (part of block body)
                // A line is indented if it starts with whitespace
                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');

                // Also check if it's a continuation keyword (Consider, When, Yield)
                let is_continuation = next_trimmed.starts_with("Consider ")
                    || next_trimmed.starts_with("When ")
                    || next_trimmed.starts_with("Yield ");

                if is_indented || is_continuation {
                    block.push(' ');
                    block.push_str(next_trimmed);
                    i += 1;
                } else {
                    // Non-indented, non-continuation line: end of block
                    break;
                }
            }

            statements.push(block);
            continue;
        }

        // Check for Literate theorem: "## Theorem: ..."
        // Collects header + Statement: + Proof: lines as one block
        if trimmed.starts_with("## Theorem:") {
            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;

            // Collect indented lines (Statement: and Proof:)
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();

                // Empty lines within block are OK
                if next_trimmed.is_empty() {
                    i += 1;
                    continue;
                }

                // Comments within block are skipped
                if next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }

                // Check if line is indented (part of theorem block)
                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');

                // Also check for Statement: or Proof: keywords (may not be indented)
                let is_theorem_part =
                    next_trimmed.starts_with("Statement:") || next_trimmed.starts_with("Proof:");

                if is_indented || is_theorem_part {
                    block.push('\n');
                    block.push_str(next_line);
                    i += 1;

                    // If we just added a Proof: line that ends with period, block is complete
                    if next_trimmed.starts_with("Proof:") && next_trimmed.ends_with('.') {
                        break;
                    }
                } else {
                    // Non-indented, non-theorem-part line: end of block
                    break;
                }
            }

            statements.push(block);
            continue;
        }

        // Check for Literate inductive: "A X is either..." or "An X is either..."
        if (trimmed.starts_with("A ") || trimmed.starts_with("An "))
            && trimmed.contains(" is either")
        {
            // Check if this is a single-line definition (ends with period and no colon at end)
            if trimmed.ends_with('.') && !trimmed.trim_end_matches('.').ends_with(':') {
                statements.push(trimmed.to_string());
                i += 1;
                continue;
            }

            // Multi-line definition: collect header + indented variants
            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;

            // Collect indented variant lines
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();

                // Empty lines are OK
                if next_trimmed.is_empty() {
                    i += 1;
                    continue;
                }

                // Comments are skipped
                if next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }

                // Check if indented
                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');

                // Also accept "a Variant" or variant names starting with capital
                let looks_like_variant = next_trimmed.starts_with("a ")
                    || next_trimmed
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);

                if is_indented
                    || (looks_like_variant
                        && !next_trimmed.starts_with("A ")
                        && !next_trimmed.starts_with("An "))
                {
                    // For indented lines, join with " or " for the parser
                    if !block.ends_with(':') {
                        block.push_str(" or ");
                    } else {
                        block.push(' ');
                    }
                    block.push_str(next_trimmed.trim_end_matches('.'));
                    i += 1;
                } else {
                    break;
                }
            }

            // Ensure ends with period
            if !block.ends_with('.') {
                block.push('.');
            }
            statements.push(block);
            continue;
        }

        // Traditional Coq-style: accumulate until period
        let mut current_stmt = String::new();
        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            if trimmed.is_empty() || trimmed.starts_with("--") {
                i += 1;
                continue;
            }

            if !current_stmt.is_empty() {
                current_stmt.push(' ');
            }
            current_stmt.push_str(trimmed);

            i += 1;

            // Check if statement is complete
            if trimmed.ends_with('.') {
                break;
            }
        }

        if !current_stmt.is_empty() {
            statements.push(current_stmt);
        }
    }

    statements
}

/// Count total files and directories in a tree (for debugging)
fn count_files(node: &FileNode) -> usize {
    let mut count = node.children.len();
    for child in &node.children {
        if child.is_directory {
            count += count_files(child);
        }
    }
    count
}

/// A rule's word for the drawer's pill, and what it was applied with when that says more than the
/// word: the term a universal was instantiated at, the variable or witness a quantifier rule took.
/// Every rule the chainer has gets a word; one it gains later reads as its name, spaced.
fn rule_look(rule: &InferenceRule) -> (String, Option<String>) {
    use InferenceRule::*;
    let word = |w: &str| (w.to_string(), None);
    match rule {
        PremiseMatch => word("Given"),
        Axiom => word("Axiom"),
        ModusPonens => word("Modus Ponens"),
        ModusTollens => word("Modus Tollens"),
        ConjunctionIntro => word("\u{2227} intro"),
        ConjunctionElim => word("\u{2227} elim"),
        DisjunctionIntro => word("\u{2228} intro"),
        DisjunctionElim | DisjunctionCases => word("\u{2228} elim"),
        ImpliesIntro => word("\u{2192} intro"),
        BicondIntro => word("\u{2194} intro"),
        DoubleNegation => word("\u{AC}\u{AC} elim"),
        ExFalso => word("Ex falso"),
        ClassicalReductio | ReductioAdAbsurdum => word("Reductio"),
        Contradiction => word("Contradiction"),
        UniversalInst(term) => ("\u{2200} elim".to_string(), Some(term.clone())),
        UniversalInstTerm(term) => ("\u{2200} elim".to_string(), Some(term.to_string())),
        UniversalIntro { variable, .. } => ("\u{2200} intro".to_string(), Some(variable.clone())),
        ExistentialIntro { .. } | ExistentialIntroTerm { .. } => word("\u{2203} intro"),
        ExistentialElim { witness } => ("\u{2203} elim".to_string(), Some(witness.clone())),
        CaseAnalysis { .. } => word("Cases"),
        StructuralInduction { .. } | InductionScheme { .. } => word("Induction"),
        Rewrite { .. } | RewriteAt { .. } => word("Rewrite"),
        EqualitySymmetry => word("= symmetry"),
        EqualityTransitivity => word("= transitivity"),
        Reflexivity => word("Reflexivity"),
        ArithDecision | LeTrans | LeRefl | LeAddMono | LinFalse | LeMulNonneg | LeSub | LtSuccLe | LtAdd1Le | LeSuccLt | LeTotal => word("Arithmetic"),
        NativeDecide => word("Decide"),
        GroundRefutation => word("Refutation"),
        OracleVerification(name) => ("Oracle".to_string(), Some(name.clone())),
        other => {
            let name = format!("{other:?}");
            let name = name.split(|c| c == '(' || c == ' ' || c == '{').next().unwrap_or("").to_string();
            let mut spaced = String::new();
            for (i, c) in name.chars().enumerate() {
                if i > 0 && c.is_uppercase() {
                    spaced.push(' ');
                }
                spaced.push(c);
            }
            (spaced, None)
        }
    }
}

/// A derivation as the drawer's tree: each step its rule, what it took, its conclusion and the
/// steps it rests on.
pub(super) fn proof_nodes(tree: &DerivationTree) -> ProofNode {
    let (rule, detail) = rule_look(&tree.rule);
    ProofNode { rule, detail, conclusion: tree.conclusion.to_string(), premises: tree.premises.iter().map(proof_nodes).collect() }
}

/// Minimal HTML escape for grid/answer cell text.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render a [`SolvedGrid`] as an HTML table for the proof panel — the easter egg that
/// fills the whole grid once the premises take the grid form. Every shown cell is a
/// certified entailment (no Z3); an undetermined cell shows a dot.
fn format_grid_html(grid: &SolvedGrid) -> String {
    let cell = "border:1px solid var(--studio-border);padding:4px 10px;text-align:center;";
    let head = "border:1px solid var(--studio-border);padding:4px 10px;text-align:center;color:var(--studio-accent);font-weight:600;";
    let mut s = String::new();
    s.push_str("<div style=\"margin-bottom:16px;\">");
    s.push_str("<div style=\"font-family:monospace;font-size:12px;color:var(--studio-text-secondary);margin-bottom:6px;\">Solved grid (every filled cell certified, no Z3):</div>");
    s.push_str("<table style=\"border-collapse:collapse;font-family:monospace;font-size:13px;color:var(--studio-text);\">");
    s.push_str("<tr>");
    s.push_str(&format!(
        "<th style=\"{head}\">{}</th>",
        esc(&grid.row_label)
    ));
    for col in &grid.columns {
        s.push_str(&format!("<th style=\"{head}\">{}</th>", esc(&col.label)));
    }
    s.push_str("</tr>");
    for (ri, row) in grid.rows.iter().enumerate() {
        s.push_str("<tr>");
        s.push_str(&format!(
            "<td style=\"{cell}font-weight:600;\">{}</td>",
            esc(row)
        ));
        for col in &grid.columns {
            match col.cells.get(ri).and_then(|c| c.as_ref()) {
                Some(v) => s.push_str(&format!("<td style=\"{cell}\">{}</td>", esc(v))),
                None => s.push_str(&format!(
                    "<td style=\"{cell}color:var(--studio-text-muted);\">·</td>"
                )),
            }
        }
        s.push_str("</tr>");
    }
    s.push_str("</table></div>");
    s
}

/// Render a wh-question's answer ("Who is in Florida?" → "Beta") for the proof panel.
fn format_answer_html(answers: &[String]) -> String {
    let body = if answers.is_empty() {
        "<span style=\"color:var(--studio-text-muted);\">no individual satisfies the question</span>".to_string()
    } else {
        format!(
            "<span style=\"color:#34d399;font-weight:600;\">{}</span>",
            esc(&answers.join(", "))
        )
    };
    format!(
        "<div style=\"font-family:monospace;font-size:14px;margin-bottom:12px;\"><span style=\"color:var(--studio-text-secondary);\">Answer: </span>{body}</div>"
    )
}

/// Assemble the proof-panel HTML for a compiled theorem: the solved grid (when the form
/// triggers it), then the wh-question answer or the certified derivation.
fn theorem_proof_html(result: &TheoremCompileResult) -> String {
    let mut html = String::new();
    if let Some(grid) = &result.grid {
        html.push_str(&format_grid_html(grid));
    }
    if let Some(answer) = &result.answer {
        html.push_str(&format_answer_html(answer));
    }
    html
}

/// What the proof panel shows for a compiled theorem: its derivation as the drawer's tree (unless
/// the theorem asks a question, whose answer the HTML carries), a grid's or an answer's HTML, its
/// status, and the hint beneath.
pub(super) struct TheoremView {
    pub(super) html: String,
    pub(super) tree: Option<ProofNode>,
    pub(super) status: ProofStatus,
    pub(super) hint: String,
}

/// One reading of a compiled theorem, for opening its file and for Execute alike. A derivation
/// the kernel refused is shown for inspection and called uncertified — never "Ready to prove",
/// never "solved".
fn theorem_view(result: &TheoremCompileResult) -> TheoremView {
    let html = theorem_proof_html(result);
    let tree = result.derivation.as_ref().filter(|_| result.answer.is_none()).map(proof_nodes);
    let name = &result.name;
    if html.is_empty() && tree.is_none() {
        return TheoremView {
            html,
            tree,
            status: ProofStatus::Idle,
            hint: format!(
                "Theorem '{name}' ready. {} premise(s) loaded. Press Prove to prove it.",
                result.premises.len()
            ),
        };
    }
    let (status, hint) = if let Some(answer) = &result.answer {
        let hint = if answer.is_empty() {
            "No answer found.".to_string()
        } else {
            format!("Answer: {}", answer.join(", "))
        };
        (
            if result.verified {
                ProofStatus::Success
            } else {
                ProofStatus::Idle
            },
            hint,
        )
    } else if result.verified {
        (ProofStatus::Success, format!("Theorem '{name}' proved!"))
    } else if result.derivation.is_some() {
        let broke = result.verification_error.clone().unwrap_or_default();
        let hint = if broke.is_empty() {
            format!("Theorem '{name}': a derivation was found, but it is not certified.")
        } else {
            format!("Theorem '{name}': a derivation was found, but it is not certified. {broke}")
        };
        (ProofStatus::Uncertified(broke), hint)
    } else {
        (
            ProofStatus::Idle,
            format!("Theorem '{name}': grid solved."),
        )
    };
    TheoremView { html, tree, status, hint }
}

/// The Auto tactic's reading, through the same door as opening a theorem
/// (`prove_certify_check`): a certified derivation is a proof, a derivation the kernel refused
/// is shown and called uncertified, and no derivation is a failure with a Socratic hint.
pub(super) fn auto_view(kb: &[ProofExpr], goal: &ProofExpr, goal_text: &str) -> TheoremView {
    let outcome = logicaffeine_proof::verify::prove_certify_check(kb, goal);
    let broke = outcome.verification_error.unwrap_or_default();
    match outcome.derivation {
        Some(derivation) if outcome.verified => TheoremView {
            html: String::new(),
            tree: Some(proof_nodes(&derivation)),
            status: ProofStatus::Success,
            hint: "Proof found! The derivation tree shows the inference steps.".to_string(),
        },
        Some(derivation) => TheoremView {
            html: String::new(),
            tree: Some(proof_nodes(&derivation)),
            hint: if broke.is_empty() {
                "A derivation was found, but it is not certified.".to_string()
            } else {
                format!("A derivation was found, but it is not certified: {broke}")
            },
            status: ProofStatus::Uncertified(broke),
        },
        None => {
            let error = if broke.is_empty() {
                "no derivation found".to_string()
            } else {
                broke
            };
            TheoremView {
                tree: None,
                html: format!(
                    "<span class=\"rule\">Auto-prove failed</span>\n\nGoal: {}\n\nError: {}",
                    crate::ui::components::hover_hint::escape(goal_text),
                    crate::ui::components::hover_hint::escape(&error)
                ),
                hint: suggest_hint(goal, kb, &[]).text,
                status: ProofStatus::Failed(error),
            }
        }
    }
}

/// Runs one of the Studio's actions behind the fault boundary. A fault nobody planned for becomes
/// the toolbar's note and the action gives back nothing, so the panels stay as they were; a clean
/// run takes down the note an earlier run left.
pub(super) fn attempt<T>(
    mut fault: Signal<Option<ActionFault>>,
    action: &'static str,
    work: impl FnOnce() -> T,
) -> Option<T> {
    let outcome = contain(work);
    let note = outcome.as_ref().err().map(|f| ActionFault {
        action,
        fault: f.clone(),
    });
    if *fault.peek() != note {
        fault.set(note);
    }
    outcome.ok()
}

/// What running a Logic document produced, computed away from the page's signals so the work can
/// run behind the fault boundary.
pub(super) enum LogicRun {
    /// A `## Theorem:` block: its compiled result and how the proof panel reads it, or why it did
    /// not compile.
    Theorem(Result<(TheoremCompileResult, TheoremView), String>),
    /// Plain sentences: their readings, and the first of them compiled for the prover.
    Sentences(CompileResult, ProofCompileResult),
    /// Nothing but headers and comments.
    Empty,
}

/// A Logic document run by the one runner every surface shares (`studio_run`), with the proof panel's
/// reading of a theorem added.
pub(super) fn run_logic(text: &str) -> LogicRun {
    match studio_run::run_logic(text) {
        studio_run::LogicRun::Theorem(theorem) => LogicRun::Theorem(theorem.map(|result| {
            let view = theorem_view(&result);
            (result, view)
        })),
        studio_run::LogicRun::Sentences(compiled, proof) => LogicRun::Sentences(compiled, proof),
        studio_run::LogicRun::Empty => LogicRun::Empty,
    }
}

/// The kernel environment as the Math Context panel lists it: its inductive types and its
/// definitions, each by name. The kernel keeps them in hash maps, whose order changes from one process
/// to the next, so by name is the one order the server's render, the browser and every visit share.
/// Built once per run, behind the fault boundary — printing a term walks it, and a deep enough term
/// runs the stack out.
#[derive(Clone, PartialEq)]
pub(super) struct MathContext {
    pub(super) inductives: Vec<ContextEntry>,
    pub(super) definitions: Vec<ContextEntry>,
}

impl MathContext {
    pub(super) fn of(ctx: &logicaffeine_kernel::Context) -> Self {
        let by_name = |mut entries: Vec<ContextEntry>| {
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            entries
        };
        MathContext {
            inductives: by_name(
                ctx.iter_inductives()
                    .map(|(name, sort)| ContextEntry {
                        name: name.to_string(),
                        ty: sort.to_string(),
                        body: None,
                        kind: EntryKind::Inductive,
                    })
                    .collect(),
            ),
            definitions: by_name(
                ctx.iter_definitions()
                    .map(|(name, ty, body)| ContextEntry {
                        name: name.to_string(),
                        ty: ty.to_string(),
                        body: Some(body.to_string()),
                        kind: EntryKind::Definition,
                    })
                    .collect(),
            ),
        }
    }

    /// What a fresh kernel holds before any program runs: the standard library.
    pub(super) fn prelude() -> Self {
        MathContext::of(Repl::new().context())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.inductives.is_empty() && self.definitions.is_empty()
    }
}

/// What running a Math document produced: a line per statement, and — when the statements ran
/// through the REPL — the environment they built. A `## Theory` development certifies its theorems
/// on its own and leaves the Context panel as it was.
pub(super) struct MathRun {
    pub(super) lines: Vec<ReplLine>,
    pub(super) context: Option<MathContext>,
}

/// A Math document run from a fresh kernel by the one runner every surface shares (`studio_run`), so
/// each run replaces the last rather than piling on, with the environment it built as the Context panel
/// lists it.
pub(super) fn run_math(code: &str) -> MathRun {
    let run = studio_run::run_math(code);
    MathRun {
        lines: run.lines,
        context: run.kernel.map(|kernel| MathContext::of(kernel.context())),
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/math_context_tests.rs"]
mod math_context_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/math_context_order_tests.rs"]
mod math_context_order_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/theorem_view_tests.rs"]
mod theorem_view_tests;

/// The file browser toggle while the tree is hidden: two stacked pages, the files it will show.
pub(super) const FILES_SVG: &str = r#"<svg data-icon="files" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 1.5h4.5l3 3v7a.5.5 0 0 1-.5.5H6a.5.5 0 0 1-.5-.5V2a.5.5 0 0 1 .5-.5z"/><path d="M10.5 1.5v3h3"/><path d="M3 4v9.5a1 1 0 0 0 1 1h6.5"/></svg>"#;

/// The file browser toggle while the tree shows: an arrow folding left into a line.
pub(super) const COLLAPSE_FILES_SVG: &str = r#"<svg data-icon="collapse" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2.5 2.5v11"/><path d="M13.5 8H5.5"/><path d="M9 4.5 5.5 8 9 11.5"/></svg>"#;

/// The formula the Logic Output shows in `format`, with the view that printed it: the Symbol
/// Dictionary reads it with the names that view prints. Deep falls back to the full formula when
/// there is no Kripke reading, as the output does.
pub(super) fn formula_on_screen(
    result: &CompileResult,
    format: OutputFormat,
) -> Option<(String, OutputFormat)> {
    match format {
        OutputFormat::SimpleFOL => result
            .simple_logic
            .clone()
            .map(|l| (l, OutputFormat::SimpleFOL)),
        OutputFormat::Kripke => result
            .kripke_logic
            .clone()
            .map(|l| (l, OutputFormat::Kripke))
            .or_else(|| result.logic.clone().map(|l| (l, OutputFormat::Unicode))),
        OutputFormat::Unicode | OutputFormat::LaTeX => {
            result.logic.clone().map(|l| (l, OutputFormat::Unicode))
        }
    }
}

/// Code mode output toggle - interpret output vs generated Rust
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CodeOutputMode {
    #[default]
    Interpret,
    Rust,
}

/// Logic mode output view — FOL interpretation or generated Rust. (SVA synthesis lives in
/// Hardware mode, where signals/clocks/cycles have meaning; general FOL has none to assert.)
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum LogicView {
    #[default]
    Logic,
    Rust,
}

/// Studio-specific styles that extend the shared responsive styles
pub const STUDIO_STYLE: &str = r#"
/* ============================================ */
/* STUDIO PAGE - Design Tokens                  */
/* ============================================ */
:root {
    --studio-bg: #0f1419;
    --studio-panel-bg: #12161c;
    /* The one surface every column paints — the editor's — and how far every column holds its
       content from its edges. A column's scroll ends at its last line: nothing to scroll into. */
    --studio-surface: #0f1419;
    --studio-inset: 20px;
    --editor-overscroll: var(--studio-inset);
    --studio-elevated: #1a1f27;
    --studio-border: rgba(255, 255, 255, 0.08);
    /* The column bars' grey: opaque (the border over --studio-bg), so every bar reads the
       same whatever sits behind it. */
    --studio-resizer: #22272b;
    --studio-border-hover: rgba(255, 255, 255, 0.15);
    --studio-text: #e8eaed;
    --studio-text-secondary: #9ca3af;
    --studio-text-muted: #6b7280;
    --studio-accent: #667eea;
    /* The one syntax palette: a colour for each kind of thing a formula is made of (One Dark), worn
       by the Syntax Tree's pills and legend. Kinds that share a colour share a value. */
    --syntax-quantifier: #c678dd;
    --syntax-connective: #c678dd;
    --syntax-predicate: #98c379;
    --syntax-individual: #e5c07b;
    --syntax-role: #e5c07b;
    --syntax-variable: #61afef;
    --syntax-modal: #56b6c2;
    --syntax-tense: #56b6c2;
    /* The toolbar's one row on a desk. */
    --toolbar-height: 49px;
}

/* ============================================ */
/* STUDIO PAGE - Desktop Layout                 */
/* ============================================ */
.studio-container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    height: 100dvh;
    background: var(--studio-bg);
    color: var(--studio-text);
    scrollbar-color: var(--studio-border-hover) transparent;
    overflow: hidden;
}

/* Toolbar */
.studio-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--toolbar-height);
    padding: 0 16px;
    background: var(--studio-panel-bg);
    border-bottom: 1px solid var(--studio-border);
    gap: 12px;
    flex-shrink: 0;
}

.studio-toolbar-left {
    display: flex;
    align-items: center;
    gap: 12px;
}

.studio-toolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
}

/* A deep link to a file the prerendered copy does not show (`prerender_guard`): the copy's panels
   keep their place unpainted until the app draws the file the link names. */
.studio-opens-elsewhere #main .studio-main {
    visibility: hidden;
}

/* What stopped an action nobody expected to fail: one warning-orange line just left of the
   buttons. The whole story — the full message and where it was raised — is in its tooltip. */
.studio-fault {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 44vw;
    color: var(--color-warning);
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: help;
}

/* One square button whose drawn icon names what a press does: show the files, or fold them away. */
.sidebar-toggle-btn {
    width: 32px;
    height: 32px;
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.7);
    cursor: pointer;
    transition: all 0.15s ease;
}

.sidebar-toggle-btn .toggle-icon {
    display: flex;
}

/* Beside the folded file browser's toggle: what it opens, muted, set like the action buttons' text. */
.files-label {
    color: var(--studio-text-muted);
    font-size: 13px;
    user-select: none;
}

.sidebar-toggle-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.9);
}

/* Main content area with optional sidebar */
.studio-content {
    flex: 1 1 0;
    min-height: 100px;
    display: flex;
    overflow: hidden;
    position: relative;
}

/* Overlay to close sidebar when clicking outside (mobile) */
.sidebar-overlay {
    display: none;
}

/* Where the files are a drawer (responsive::media::FILES_DRAWER), it opens over the panels, inside
   the workbench's own content, and stays folded until the reader opens it: no tree covers the
   editor in the first frame. */
@media (max-width: 768px), (min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse), (max-height: 500px) and (orientation: landscape) {
    .sidebar-overlay {
        display: block;
        position: absolute;
        inset: 0;
        background: rgba(0, 0, 0, 0.5);
        z-index: 99;
    }

    .sidebar-overlay:not(.drawer-open),
    .studio-sidebar:not(.drawer-open) {
        display: none !important;
    }
}

/* Sidebar wrapper for controlled width */
.studio-sidebar {
    display: flex;
    flex-shrink: 0;
    overflow: hidden;
}

@media (max-width: 768px), (min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse), (max-height: 500px) and (orientation: landscape) {
    .studio-sidebar {
        position: absolute;
        left: 0;
        top: 0;
        bottom: 0;
        z-index: 100;
        width: 280px !important;
        min-width: 280px !important;
        max-width: 280px !important;
        box-shadow: 4px 0 20px rgba(0, 0, 0, 0.3);
        background: var(--studio-surface);
    }

    /* Text color overrides for Safari compatibility */
    .studio-sidebar .file-tree-item {
        color: rgba(255, 255, 255, 0.9) !important;
        -webkit-text-fill-color: rgba(255, 255, 255, 0.9);
    }

    .studio-sidebar .file-tree-item.selected {
        color: #00d4ff !important;
        -webkit-text-fill-color: #00d4ff;
    }

    .studio-sidebar .file-tree-item .name {
        color: inherit;
        -webkit-text-fill-color: inherit;
    }
}

/* Desktop: 3-column panel layout */
.studio-main {
    flex: 1;
    display: flex;
    overflow: hidden;
    gap: 1px;
    background: var(--studio-border);
}

.studio-panel {
    background: var(--studio-surface);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
}

/* Desktop columns. Their widths come from the layout model (studio_layout.rs) as custom
   properties on .studio-content, so a drag repaints them without re-rendering the page. */
@media (min-width: 769px) {
    .studio-sidebar { width: var(--col-sidebar); }
    .studio-main > .col-left { flex: 0 0 var(--col-left); }
    .studio-main > .col-center { flex: 1 1 0; }
    .studio-main > .col-right { flex: 0 0 var(--col-right); }
}

.studio-panel .panel-header {
    padding: 0 20px;
    height: 52px;
    background: rgba(255, 255, 255, 0.02);
    border-bottom: 1px solid var(--studio-border);
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-shrink: 0;
}

/* The Studio's one title type: every panel header's title ("English Input", "Logic Output"), the
   Files header and each collapsible block's heading ("Symbol Dictionary"). */
.studio-panel .panel-header,
.panel-title {
    font-size: 16px;
    font-weight: 600;
    letter-spacing: 0.3px;
    color: var(--studio-text);
}

@media (max-width: 768px), (min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse) {
    .studio-panel .panel-header,
    .panel-title {
        font-size: 14px;
    }
}

.studio-panel .panel-content {
    flex: 1;
    min-height: 0;
    overflow: auto;
    -webkit-overflow-scrolling: touch;
}

/* Panel Resizers (desktop only). The bar draws 6px (RESIZER in studio_layout.rs); its grab
   area reaches 4px either side. */
.panel-resizer {
    position: relative;
    width: 6px;
    background: var(--studio-resizer);
    cursor: col-resize;
    transition: background 0.2s ease;
    flex-shrink: 0;
    touch-action: none;
}

.panel-resizer::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: -4px;
    right: -4px;
    z-index: 1;
}

.panel-resizer:hover,
.panel-resizer.active {
    background: var(--studio-accent);
}

/* Format Toggle (Unicode/LaTeX) */
.format-toggle {
    display: flex;
    gap: 4px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--studio-border);
    border-radius: 6px;
    padding: 2px;
}

.format-btn {
    padding: 4px 10px;
    border: none;
    background: transparent;
    color: var(--studio-text-muted);
    font-size: 12px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
    line-height: 1;
}

.format-btn:hover {
    color: var(--studio-text);
    background: rgba(255, 255, 255, 0.04);
}

.format-btn.active {
    background: rgba(255, 255, 255, 0.08);
    color: var(--studio-text);
}

/* Guide Bar - above panels */
.studio-guide {
    background: var(--studio-panel-bg);
    border-bottom: 1px solid var(--studio-border);
    flex-shrink: 0;
}

/* The dock bar: one amber line along the bottom, above the status bar, that says what to do next
   and holds the way into the drawer that rises from it (Logic's proof, Code's debugger). */
.studio-dock {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 40px;
    padding: 4px 12px;
    background: rgba(245, 158, 11, 0.10);
    border-top: 1px solid rgba(245, 158, 11, 0.35);
    color: #fbbf24;
    font-size: 13px;
    flex-shrink: 0;
}

.studio-dock-icon {
    display: inline-flex;
    font-size: 15px;
}

.studio-dock-text {
    flex: 1 1 auto;
    min-width: 0;
    line-height: 1.4;
}

.studio-dock-btn {
    flex: none;
    min-height: 30px;
    padding: 5px 12px;
    border: 1px solid rgba(245, 158, 11, 0.45);
    border-radius: 6px;
    background: transparent;
    color: #fbbf24;
    font: 600 13px/1 inherit;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.15s ease;
}

.studio-dock-btn:hover:not(:disabled) { background: rgba(245, 158, 11, 0.18); }

.studio-dock-btn.primary {
    background: #f59e0b;
    border-color: #f59e0b;
    color: #111;
}

.studio-dock-btn.primary:hover:not(:disabled) { background: #fbbf24; }

.studio-dock-btn:disabled {
    opacity: 0.4;
    cursor: default;
}

/* Under a finger the dock's buttons are a finger tall. */
@media (pointer: coarse) {
    .studio-dock-btn { min-height: 44px; }
}

/* Execute button for Code mode */
.execute-btn {
    padding: 8px 14px;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    border: none;
    border-radius: 8px;
    color: white;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
}

/* Desktop: show short labels */
.execute-btn .mobile-label {
    display: none;
}

.execute-btn .desktop-label {
    display: inline;
}

.execute-btn:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(102, 126, 234, 0.3);
}

/* An action the open file cannot take: in its place, dimmed, its tooltip saying why. */
.execute-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
}

.execute-btn:active {
    transform: translateY(0);
}

/* Mobile execute buttons */
@media (max-width: 768px), (min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse) {
    .execute-btn {
        padding: 6px 10px;
        font-size: 12px;
        white-space: nowrap;
    }
}

@media (max-width: 480px) {
    .execute-btn {
        padding: 5px 8px;
        font-size: 11px;
    }
}

/* Output mode toggle for Code mode */
/* Labelled output on a panel's own surface — a small-caps label over flat code, set like the
   code editor's text — and a verdict marked by its colour and a rule, not a box. */
.panel-section-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--studio-text-secondary);
    margin: 4px 0;
}

/* The first label starts at the panel's inset, like every column's first line. */
.panel-section-label:first-child {
    margin-top: 0;
}

.panel-code {
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 14px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--studio-text);
    margin: 0 0 12px;
}

.panel-verdict {
    font-size: 13px;
    line-height: 1.5;
    color: var(--studio-text-secondary);
    border-left: 2px solid var(--studio-border-hover);
    padding: 2px 0 2px 10px;
    margin: 0 0 12px;
}

/* A picture's caption: what the drawing shows, in the secondary voice. */
.panel-caption {
    margin-top: 6px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--studio-text-secondary);
}

.panel-verdict.proved {
    color: #4ade80;
    border-left-color: #4ade80;
}

.panel-verdict.refuted {
    color: #f87171;
    border-left-color: #f87171;
}

/* The controls on a panel header's right: segmented toggles and one-off actions. */
.panel-header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
}

.output-mode-toggle {
    display: flex;
    gap: 4px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--studio-border);
    border-radius: 6px;
    padding: 2px;
}

.output-mode-btn {
    padding: 4px 10px;
    border: none;
    background: transparent;
    color: var(--studio-text-muted);
    font-size: 12px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
    line-height: 1;
}

.output-mode-btn:hover {
    color: var(--studio-text);
    background: rgba(255, 255, 255, 0.04);
}

.output-mode-btn.active {
    background: rgba(255, 255, 255, 0.08);
    color: var(--studio-text);
}

/* Interpreter output display */
/* A panel's own content — a picture, a list — held at the Studio's inset; `wide` scrolls a
   picture wider than its column instead of shrinking it. */
.panel-body {
    padding: var(--studio-inset);
}

.panel-body.wide {
    overflow-x: auto;
}

.interpreter-output {
    padding: var(--studio-inset);
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 14px;
    line-height: 1.6;
}

/* A program's output keeps its spacing — padded columns and indents are what it printed. */
.interpreter-line {
    margin-bottom: 4px;
    color: #e8eaed;
    white-space: pre-wrap;
}

.interpreter-error {
    color: #e06c75;
    padding: 12px;
    background: rgba(224, 108, 117, 0.1);
    border-radius: 6px;
    margin-top: 8px;
}

.interpreter-empty {
    color: rgba(255, 255, 255, 0.4);
    text-align: center;
    padding: 40px 20px;
}

/* ============================================ */
/* STUDIO PAGE - Mobile Overrides               */
/* ============================================ */

/* Phones and upright tablets: the panels stack. */
@media (max-width: 768px), (min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse) {
    .studio-toolbar {
        flex-wrap: wrap;
        height: auto;
        padding: 10px 12px;
        gap: 10px;
        position: relative;
        z-index: 101;
    }

    /* First row: file toggle */
    .studio-toolbar-left {
        flex-shrink: 0;
        order: 1;
    }

    /* Second row: action buttons - full width */
    .studio-toolbar-right {
        flex-basis: 100%;
        flex-wrap: wrap;
        justify-content: center;
        gap: 12px;
        order: 3;
        padding-top: 8px;
        border-top: 1px solid rgba(255, 255, 255, 0.06);
    }

    /* The fault note takes a line of its own above the buttons. */
    .studio-fault {
        flex-basis: 100%;
        max-width: 100%;
        text-align: center;
    }

    /* Full text on mobile buttons */
    .execute-btn {
        padding: 10px 16px;
        font-size: 14px;
    }

    .files-label {
        font-size: 14px;
    }

    .execute-btn .mobile-label {
        display: inline;
    }

    .execute-btn .desktop-label {
        display: none;
    }

    /* Hide mobile tab bar - panels stack instead */
    .mobile-tabs {
        display: none !important;
    }

    /* Hide desktop resizers */
    .panel-resizer {
        display: none;
    }

    /* Stacked vertical panel layout */
    .studio-main {
        flex-direction: column;
        gap: 0;
        background: var(--studio-bg);
    }

    /* Both panels visible, stacked vertically */
    .studio-panel {
        min-width: unset;
        min-height: 0;
        width: 100% !important;
    }

    .studio-panel.mobile-expanded {
        flex: var(--panel-flex, 1);
        overflow: hidden;
    }

    .studio-panel.mobile-collapsed {
        flex: 0 0 auto;
    }

    .studio-panel.mobile-collapsed .panel-content {
        display: none;
    }

    /* Show panel headers with collapse affordance */
    .studio-panel .panel-header {
        display: flex !important;
        cursor: pointer;
        padding: 0 14px;
        height: 44px;
    }

    /* Collapse chevron indicator */
    .studio-panel .panel-header::after {
        content: '\25BC';
        font-size: 10px;
        color: rgba(255, 255, 255, 0.4);
        margin-left: 8px;
        transition: transform 0.15s ease;
    }

    .studio-panel.mobile-collapsed .panel-header::after {
        content: '\25B6';
    }

    /* Divider between stacked panels */
    .studio-panel + .studio-panel {
        border-top: 1px solid var(--studio-border);
    }

    /* Hide Panel 3 (Console/Context/Graph) on mobile */
    .studio-main > aside {
        display: none !important;
    }

    /* Mobile-sized format toggle */
    .format-toggle {
        gap: 4px;
        padding: 2px;
        border-radius: 6px;
    }

    .format-btn {
        padding: 6px 10px;
        font-size: 12px;
        border-radius: 4px;
        min-height: 32px;
        min-width: 32px;
        display: flex;
        align-items: center;
        justify-content: center;
    }

    /* Footer constraints */
    .studio-footer {
        max-height: 30vh;
        overflow: auto;
    }

    .mobile-panel-resizer {
        display: flex;
        align-items: center;
        justify-content: center;
        height: 16px;
        cursor: row-resize;
        flex-shrink: 0;
        touch-action: none;
        -webkit-tap-highlight-color: transparent;
        z-index: 2;
        position: relative;
    }

    .mobile-panel-resizer::after {
        content: "";
        position: absolute;
        left: 50%;
        top: 50%;
        transform: translate(-50%, -50%);
        width: 48px;
        height: 4px;
        border-radius: 2px;
        background: rgba(255, 255, 255, 0.15);
        transition: background 0.15s ease;
    }

    .mobile-panel-resizer:hover::after,
    .mobile-panel-resizer.active::after {
        background: rgba(102, 126, 234, 0.6);
    }
}

/* Extra small screens */
@media (max-width: 480px) {
    .format-btn {
        padding: 4px 8px;
        font-size: 11px;
    }
}

/* Desktop: hide mobile resizer */
@media (min-width: 769px) {
    .mobile-panel-resizer {
        display: none;
    }
}

/* Landscape mobile */
@media (max-height: 500px) and (orientation: landscape) {
    .studio-footer {
        max-height: 25vh;
    }
}

/* ============================================ */
/* STUDIO PAGE - Phones, tablets and touch      */
/* ============================================ */
/* Every rule from here on sits under a phone, tablet or touch query (responsive::media), so the
   desktop Studio stays as it was (`the_desktop_studio_is_unchanged`). The bands:
   phone portrait   one pane at a time, chosen by the switch
   phone landscape  the editor beside the output, half and half
   tablet portrait  the editor over the output, the split bar between
   tablet landscape the desktop's columns, for a finger
   The workbench's wrapper lays nothing out of its own on a desk. */
.studio-workbench {
    display: contents;
}

/* The switch between a phone's panes, and the lines that stand in for a hover, show only where
   they are needed. */
.mobile-tabs,
.studio-reasons,
.studio-fault-detail {
    display: none;
}

/* The switch: a row of tabs under the toolbar, each a finger wide. */
@media (max-width: 600px), (max-height: 500px) and (orientation: landscape), (min-width: 601px) and (max-width: 768px), (min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse) {
    .mobile-tabs {
        gap: 4px;
        padding: 4px max(12px, var(--safe-right, 0px)) 6px max(12px, var(--safe-left, 0px));
        background: var(--studio-panel-bg);
        border-bottom: 1px solid var(--studio-border);
        flex-shrink: 0;
    }

    .mobile-tabs .mobile-tab {
        flex: 1 1 0;
        min-width: 0;
        min-height: 44px;
        display: flex;
        flex-direction: row;
        align-items: center;
        justify-content: center;
        gap: 6px;
        padding: 0 8px;
        border: 1px solid transparent;
        border-radius: 8px;
        background: rgba(255, 255, 255, 0.04);
        color: var(--studio-text-muted);
        font-size: 13px;
        font-weight: 500;
        cursor: pointer;
        -webkit-tap-highlight-color: transparent;
    }

    .mobile-tabs .mobile-tab.active {
        background: rgba(102, 126, 234, 0.2);
        border-color: rgba(102, 126, 234, 0.4);
        color: var(--studio-text);
    }

    .mobile-tabs .mobile-tab-icon {
        font-size: 15px;
        line-height: 1;
    }

    .mobile-tabs .mobile-tab-label {
        font-size: 13px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
}

/* One row of toolbar on a phone, either way up: the files toggle on the left, the mode's actions on
   the right under their short labels, every one a finger wide. */
@media (max-width: 600px), (max-height: 500px) and (orientation: landscape) {
    .studio-toolbar {
        flex-wrap: wrap;
        height: auto;
        padding: 8px max(12px, var(--safe-right, 0px)) 8px max(12px, var(--safe-left, 0px));
        gap: 8px;
    }

    .studio-toolbar-right {
        flex-basis: auto;
        flex: 0 1 auto;
        margin-left: auto;
        order: 2;
        justify-content: flex-end;
        gap: 6px;
        padding-top: 0;
        border-top: none;
    }

    .execute-btn {
        min-height: 44px;
        padding: 0 10px;
        font-size: 13px;
    }

    .execute-btn .mobile-label {
        display: none;
    }

    .execute-btn .desktop-label {
        display: inline;
    }

    .files-label {
        font-size: 13px;
    }

    /* A pane's header folds nothing where the panes are fixed. */
    .studio-panel .panel-header {
        cursor: default;
    }

    .studio-panel .panel-header::after {
        content: none;
    }

    .studio-panel + .studio-panel {
        border-top: none;
    }

    .studio-panel.mobile-collapsed .panel-content {
        display: block;
    }

    .studio-panel.mobile-collapsed .panel-content
    .mobile-panel-resizer,
    .panel-resizer {
        display: none !important;
    }
}

/* A phone held upright: the pane the switch names, alone, the whole height. */
@media (max-width: 600px) {
    .mobile-tabs {
        display: flex !important;
    }

    .studio-main > .studio-panel:not(.phone-shown) {
        display: none !important;
    }

    .studio-main > .studio-panel.phone-shown {
        display: flex !important;
        flex: 1 1 0;
        min-height: 0;
    }
}

/* A phone on its side: the editor beside the output (or the third panel the switch names), half and
   half, the status bar left out for the height, and the page's nav free to scroll away above a
   workbench the window's height. */
@media (max-height: 500px) and (orientation: landscape) {
    .studio-main {
        flex-direction: row;
        gap: 1px;
        background: var(--studio-border);
    }

    .studio-main > .studio-panel {
        flex: 1 1 0 !important;
        min-width: 0;
        width: auto !important;
    }

    .studio-main > .col-center:not(.phone-right),
    .studio-main > .col-right:not(.phone-right) {
        display: none !important;
    }

    .studio-main > .col-right.phone-right {
        display: flex !important;
    }

    .mobile-tabs.has-third {
        display: flex !important;
    }

    .mobile-tabs .mobile-tab:first-child {
        display: none;
    }

    .studio-statusbar {
        display: none;
    }

    .studio-container {
        height: auto;
    }

    .studio-container > .studio-workbench {
        display: flex;
        flex-direction: column;
        flex: none;
        height: 100svh;
    }
}

/* A tablet held upright: the editor over the lower pane, which shows the output or the third panel
   the switch names; the split bar between them takes a finger. */
@media (min-width: 601px) and (max-width: 768px), (min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse) {
    .mobile-tabs.has-third {
        display: flex !important;
    }

    .mobile-tabs .mobile-tab:first-child {
        display: none;
    }

    .studio-main > .col-center:not(.phone-right) {
        display: none !important;
    }

    .studio-main > aside.phone-right {
        display: flex !important;
    }

    .mobile-panel-resizer {
        display: flex;
    }

    .mobile-panel-resizer::before {
        content: "";
        position: absolute;
        left: 0;
        right: 0;
        top: -14px;
        bottom: -14px;
    }
}

/* Every control a finger wide wherever a finger, a phone or an upright tablet may press it: the
   files toggle, the actions, the output and format toggles, the proof's tactics, the guide's hint,
   the readings and New file. */
@media (max-width: 768px), (pointer: coarse), (max-height: 500px) and (orientation: landscape) {
    .sidebar-toggle-btn {
        width: 44px;
        height: 44px;
    }

    .execute-btn {
        min-height: 44px;
    }

    .output-mode-btn,
    .format-btn {
        min-height: 44px;
        min-width: 44px;
    }

    .output-mode-toggle,
    .format-toggle {
        padding: 0;
    }

    .studio-panel .panel-header {
        height: 52px;
    }

    .studio-workbench .tactic-btn,
    .studio-workbench .guide-btn,
    .studio-workbench .reading-btn,
    .studio-workbench .file-browser-btn {
        min-height: 44px;
        min-width: 44px;
    }
}

/* Under a finger: the bars' grab areas a finger wide around what they draw, the panes' scrolling
   kept to the panes on the Studio page, and the page sized above an on-screen keyboard. */
@media (pointer: coarse) {
    .panel-resizer::after {
        left: -19px;
        right: -19px;
    }

    .studio-container {
        height: calc(100dvh - var(--keyboard-inset, 0px));
        overscroll-behavior: none;
    }

    .studio-container .panel-content,
    .studio-container .code-editor-textarea {
        overscroll-behavior: contain;
    }

    .code-editor-textarea {
        touch-action: pan-x pan-y;
    }
}

/* Where nothing hovers: each disabled action says why on a line under the toolbar, and the fault
   note wraps and opens its whole story to a tap. */
@media (hover: none) {
    .studio-reasons {
        display: block;
        padding: 6px max(12px, var(--safe-right, 0px)) 6px max(12px, var(--safe-left, 0px));
        background: var(--studio-panel-bg);
        border-bottom: 1px solid var(--studio-border);
        color: var(--studio-text-muted);
        font-size: 12px;
        line-height: 1.4;
        flex-shrink: 0;
    }

    .studio-reasons > span {
        display: block;
    }

    .studio-fault {
        white-space: normal;
        text-overflow: clip;
        max-width: none;
    }

    .studio-fault-detail {
        display: block;
        padding: 6px max(12px, var(--safe-right, 0px)) 6px max(12px, var(--safe-left, 0px));
        background: var(--studio-panel-bg);
        border-bottom: 1px solid var(--studio-border);
        color: var(--color-warning);
        font-size: 12px;
        line-height: 1.4;
        white-space: pre-wrap;
        flex-shrink: 0;
    }
}

/* LLM Heal (work/LLM_HEAL.md): the blue button with the fairy, the fairy swirling in to the mended
   line and resting there still, and her card, readable whole with Keep and Undo always in view. */
.heal-btn {
    background: linear-gradient(135deg, var(--accent-primary, #00d4ff) 0%, #2563eb 100%);
    display: inline-flex;
    align-items: center;
    gap: 6px;
}
.heal-btn-icon {
    width: 18px;
    height: 18px;
    object-fit: contain;
    image-rendering: pixelated;
}
.col-left { position: relative; }
.heal-overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
    z-index: 30;
}
.heal-fairy {
    position: absolute;
    width: 56px;
    height: 56px;
    object-fit: contain;
    image-rendering: pixelated;
    left: 8px;
    top: calc(48px + var(--heal-line, 0) * 1.5em);
    transform: translateY(-50%);
    transition: top 0.7s ease;
    filter: drop-shadow(0 0 10px rgba(0, 212, 255, 0.7));
}
/* The flight: one swirl from the toolbar's side down into the editor, then still. */
.heal-fairy.flying { animation: heal-swirl 1.1s cubic-bezier(0.22, 0.8, 0.3, 1) both; }
@keyframes heal-swirl {
    0%   { transform: translate(60vw, -60px) rotate(0deg) scale(0.4); opacity: 0; }
    35%  { transform: translate(28vw, 40px) rotate(-200deg) scale(0.8); opacity: 1; }
    70%  { transform: translate(8vw, -16px) rotate(-330deg) scale(1.05); }
    100% { transform: translateY(-50%) rotate(-360deg) scale(1); }
}
.heal-fairy.failed { left: auto; right: 18px; top: 10px; transform: none; }
.heal-card {
    position: absolute;
    left: 70px;
    right: 12px;
    top: 44px;
    bottom: 12px;
    display: flex;
    flex-direction: column;
    pointer-events: auto;
    background: #1b1f27;
    border: 1px solid rgba(0, 212, 255, 0.45);
    border-radius: 10px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.55);
    padding: 12px 14px;
    color: #e6edf3;
    font-size: 13px;
}
.heal-card-cols {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 14px;
}
.heal-card-col { min-width: 0; }
.heal-card-col h3 { margin: 0 0 6px; font-size: 12px; letter-spacing: 0.04em; text-transform: uppercase; color: #9aa3b2; }
.heal-card pre { margin: 0; white-space: pre-wrap; word-break: break-word; font: 13px/1.6 var(--font-mono, ui-monospace, monospace); color: #e6edf3; }
.heal-before { color: #b7bec9; }
.heal-line.mended { background: rgba(0, 212, 255, 0.14); box-shadow: -4px 0 0 rgba(0, 212, 255, 0.7); border-radius: 2px; }
.heal-line.replaced { background: rgba(239, 68, 68, 0.14); box-shadow: -4px 0 0 rgba(239, 68, 68, 0.7); border-radius: 2px; text-decoration: line-through; text-decoration-color: rgba(239, 68, 68, 0.6); }
.heal-word { background: rgba(0, 212, 255, 0.35); color: #fff; border-radius: 2px; font-weight: 600; }
.heal-line.replaced .heal-word { background: rgba(239, 68, 68, 0.4); }
.heal-card-actions { flex: none; display: flex; align-items: center; gap: 8px; margin-top: 10px; padding-top: 10px; border-top: 1px solid rgba(255, 255, 255, 0.08); }
.heal-card-note { flex: 1 1 auto; color: #9aa3b2; }
.heal-card .studio-dock-btn { border-color: rgba(0, 212, 255, 0.45); color: var(--accent-primary, #00d4ff); }
.heal-card .studio-dock-btn.primary { background: rgba(0, 212, 255, 0.16); }
.heal-card-failed { bottom: auto; max-width: 420px; }
.heal-card-failed p { margin: 0 0 10px; }
@media (max-width: 768px) {
    .heal-card { left: 12px; }
    .heal-card-cols { grid-template-columns: minmax(0, 1fr); }
}
@media (prefers-reduced-motion: reduce) {
    .heal-fairy { transition: none; animation: none; }
}
}
"#;

/// Render the hardware knowledge graph as an animated radial SVG: signal nodes on a ring,
/// relations as directed edges. Pure SVG + CSS — edges animate a flowing dash ("signal
/// current") and nodes pulse, so a static spec reads like a live circuit. Returns "" if empty.
pub(super) fn kg_svg(kg: &logicaffeine_compile::codegen_sva::hw_pipeline::KgSummary) -> String {
    use std::fmt::Write as _;
    let n = kg.nodes.len();
    if n == 0 {
        return String::new();
    }
    let (w, h) = (340.0_f64, 300.0_f64);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let ring = (w.min(h) / 2.0 - 50.0).max(34.0);
    let node_r = 15.0_f64;
    let pos: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let ang = (i as f64) / (n as f64) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
            (cx + ring * ang.cos(), cy + ring * ang.sin())
        })
        .collect();
    let color = |role: &str| match role {
        "input" => "#60a5fa",
        "output" => "#4ade80",
        "clock" => "#fbbf24",
        _ => "#a78bfa",
    };
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };

    let mut svg = String::new();
    let _ = write!(
        svg,
        r##"<svg viewBox="0 0 {w} {h}" width="100%" style="max-height:300px" xmlns="http://www.w3.org/2000/svg">"##
    );
    svg.push_str(
        r##"<defs><marker id="kgar" markerWidth="9" markerHeight="9" refX="8" refY="3" orient="auto"><path d="M0,0 L8,3 L0,6 Z" fill="rgba(255,255,255,0.45)"/></marker></defs>"##,
    );
    svg.push_str(
        r##"<style>
.kge{stroke:rgba(167,139,250,0.55);stroke-width:1.5;fill:none;stroke-dasharray:5 4;animation:kgflow .9s linear infinite}
.kgn{stroke:rgba(255,255,255,0.9);stroke-width:1.5;animation:kgpulse 2.6s ease-in-out infinite}
.kgr{fill:rgba(255,255,255,0.5);font:9px ui-monospace,monospace;text-anchor:middle}
.kgl{fill:#e5e7eb;font:11px ui-sans-serif,system-ui;text-anchor:middle;font-weight:600}
@keyframes kgflow{to{stroke-dashoffset:-18}}
@keyframes kgpulse{0%,100%{opacity:1}50%{opacity:.72}}
</style>"##,
    );
    // Relations between one pair of signals run along one line, either way round, so they
    // share one label naming each relation once — apart, they overprint at one midpoint.
    let mut labels: Vec<((usize, usize), (f64, f64), Vec<&str>)> = Vec::new();
    for l in &kg.links {
        let (x1, y1) = pos[l.from];
        let (x2, y2) = pos[l.to];
        let (dx, dy) = (x2 - x1, y2 - y1);
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let (ux, uy) = (dx / len, dy / len);
        let (sx, sy) = (x1 + ux * node_r, y1 + uy * node_r);
        let (ex, ey) = (x2 - ux * (node_r + 6.0), y2 - uy * (node_r + 6.0));
        let _ = write!(
            svg,
            r##"<line class="kge" x1="{sx:.1}" y1="{sy:.1}" x2="{ex:.1}" y2="{ey:.1}" marker-end="url(#kgar)"/>"##
        );
        let pair = (l.from.min(l.to), l.from.max(l.to));
        match labels.iter_mut().find(|(p, _, _)| *p == pair) {
            Some((_, _, names)) => {
                if !names.contains(&l.relation.as_str()) {
                    names.push(&l.relation);
                }
            }
            None => labels.push((
                pair,
                ((sx + ex) / 2.0, (sy + ey) / 2.0 - 2.0),
                vec![&l.relation],
            )),
        }
    }
    for (_, (mx, my), names) in &labels {
        let _ = write!(
            svg,
            r##"<text class="kgr" x="{mx:.1}" y="{my:.1}">{}</text>"##,
            esc(&names.join(" · "))
        );
    }
    for (i, node) in kg.nodes.iter().enumerate() {
        let (x, y) = pos[i];
        let _ = write!(
            svg,
            r##"<circle class="kgn" cx="{x:.1}" cy="{y:.1}" r="{node_r}" fill="{}"/>"##,
            color(&node.role)
        );
        let label = if node.width > 1 {
            format!("{}[{}]", node.name, node.width)
        } else {
            node.name.clone()
        };
        let _ = write!(
            svg,
            r##"<text class="kgl" x="{x:.1}" y="{:.1}">{}</text>"##,
            y + 28.0,
            esc(&label)
        );
    }
    svg.push_str("</svg>");
    svg
}

/// Render a counterexample as a logic-analyzer waveform SVG: boolean signals as digital
/// square waves, multi-bit registers as value-labeled buses, with a sweeping playhead. Pure
/// SVG + CSS, drawn at its own size — a long trace scrolls in its panel rather than shrinking
/// its labels. Returns "" if empty.
pub(super) fn waveform_svg(wf: &logicaffeine_compile::codegen_sva::hw_pipeline::Waveform) -> String {
    use std::fmt::Write as _;
    let (ts, n) = (wf.timesteps, wf.signals.len());
    if ts == 0 || n == 0 {
        return String::new();
    }
    let (gutter, col_w, row_h, top) = (72.0_f64, 46.0_f64, 34.0_f64, 18.0_f64);
    let span = (ts as f64) * col_w;
    let (w, h) = (gutter + span + 10.0, top + (n as f64) * row_h + 6.0);
    let dur = (0.45 * ts as f64).max(1.2);
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };

    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg viewBox="0 0 {w:.0} {h:.0}" width="{w:.0}" height="{h:.0}" xmlns="http://www.w3.org/2000/svg">"##
    );
    let _ = write!(
        s,
        r##"<style>
.wfg{{stroke:rgba(255,255,255,0.07);stroke-width:1}}
.wft{{fill:none;stroke:#22d3ee;stroke-width:2;stroke-linejoin:round}}
.wfb{{fill:rgba(96,165,250,0.12);stroke:#60a5fa;stroke-width:1.5}}
.wfv{{fill:#dbeafe;font:11px ui-monospace,monospace;text-anchor:middle}}
.wfn{{fill:#e5e7eb;font:11px ui-monospace,monospace;text-anchor:end}}
.wftl{{fill:rgba(255,255,255,0.4);font:9px ui-monospace,monospace;text-anchor:middle}}
.wfph{{stroke:#f472b6;stroke-width:1.5;opacity:.8;animation:wfsweep {dur:.1}s linear infinite}}
@keyframes wfsweep{{from{{transform:translateX(0)}}to{{transform:translateX({span:.0}px)}}}}
</style>"##
    );
    for t in 0..ts {
        let x = gutter + (t as f64) * col_w;
        let _ = write!(
            s,
            r##"<line class="wfg" x1="{x:.1}" y1="{top:.1}" x2="{x:.1}" y2="{:.1}"/>"##,
            h - 4.0
        );
        let _ = write!(
            s,
            r##"<text class="wftl" x="{:.1}" y="12">t{t}</text>"##,
            x + col_w / 2.0
        );
    }
    let xe = gutter + span;
    let _ = write!(
        s,
        r##"<line class="wfg" x1="{xe:.1}" y1="{top:.1}" x2="{xe:.1}" y2="{:.1}"/>"##,
        h - 4.0
    );

    for (ri, sig) in wf.signals.iter().enumerate() {
        let rtop = top + (ri as f64) * row_h;
        let (hi, lo) = (rtop + 6.0, rtop + row_h - 12.0);
        let mid = (hi + lo) / 2.0;
        let label = if sig.width > 1 {
            format!("{}[{}]", sig.name, sig.width)
        } else {
            sig.name.clone()
        };
        let _ = write!(
            s,
            r##"<text class="wfn" x="{:.1}" y="{:.1}">{}</text>"##,
            gutter - 8.0,
            mid + 4.0,
            esc(&label)
        );
        if sig.width == 1 {
            let mut pts = String::new();
            for t in 0..ts {
                let x0 = gutter + (t as f64) * col_w;
                let lvl = match sig.values.get(t as usize) {
                    Some(Some(v)) => {
                        if *v != 0 {
                            hi
                        } else {
                            lo
                        }
                    }
                    _ => mid,
                };
                let _ = write!(pts, "{x0:.1},{lvl:.1} {:.1},{lvl:.1} ", x0 + col_w);
            }
            let _ = write!(s, r##"<polyline class="wft" points="{}"/>"##, pts.trim());
        } else {
            for t in 0..ts {
                if let Some(Some(v)) = sig.values.get(t as usize) {
                    let x0 = gutter + (t as f64) * col_w;
                    let _ = write!(
                        s,
                        r##"<rect class="wfb" x="{:.1}" y="{hi:.1}" width="{:.1}" height="{:.1}" rx="3"/>"##,
                        x0 + 2.0,
                        col_w - 4.0,
                        lo - hi
                    );
                    let _ = write!(
                        s,
                        r##"<text class="wfv" x="{:.1}" y="{:.1}">{v}</text>"##,
                        x0 + col_w / 2.0,
                        mid + 4.0
                    );
                }
            }
        }
    }
    let _ = write!(
        s,
        r##"<line class="wfph" x1="{gutter:.1}" y1="{top:.1}" x2="{gutter:.1}" y2="{:.1}"/>"##,
        h - 4.0
    );
    s.push_str("</svg>");
    s
}

fn wf_val(sig: &logicaffeine_compile::codegen_sva::hw_pipeline::WaveSignal, t: usize) -> u64 {
    sig.values.get(t).and_then(|v| *v).unwrap_or(0)
}

/// One signalized-intersection PHASE: a maximal run of the trace with one identical signal
/// state, plus the human wall-clock duration we give it for the animation.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TrafficPhase {
    ns: u64,
    ew: u64,
    nsl: u64,
    ewl: u64,
    ped: u64,
    start: f64,
    dur: f64,
}

impl TrafficPhase {
    fn val(&self, pfx: &str) -> u64 {
        match pfx {
            "ns" => self.ns,
            "ew" => self.ew,
            "nsl" => self.nsl,
            "ewl" => self.ewl,
            "ped" => self.ped,
            _ => 0,
        }
    }
}

/// The whole animation plan derived from a counterexample/witness trace: the phase sequence on a
/// seconds timeline, plus the index of the phase whose state violates safety (the crash).
struct TrafficPlan {
    phases: Vec<TrafficPhase>,
    /// First trace timestep of each phase (so callers can still sample the raw signals).
    firsts: Vec<usize>,
    total: f64,
    conflict: Option<usize>,
}

/// What the NS-through platoon does during phase `i`: nothing visible (red/yellow), drive straight
/// through (a normal green), or drive to the crosswalk and stop (the crash phase).
#[derive(Clone, Copy, Debug, PartialEq)]
enum NsDrive {
    Hidden,
    Through,
    CrashStop,
}

/// Collapse a trace into the phase plan. `None` unless both `ns` and `ew` are present.
fn traffic_plan(
    wf: &logicaffeine_compile::codegen_sva::hw_pipeline::Waveform,
) -> Option<TrafficPlan> {
    use logicaffeine_compile::codegen_sva::hw_pipeline::WaveSignal;
    let get = |name: &str| {
        wf.signals
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    };
    let ns = get("ns")?;
    let ew = get("ew")?;
    let nsl = get("nsl");
    let ewl = get("ewl");
    let ped = get("ped");
    let n = (wf.timesteps as usize).max(1);
    let sv = |sig: &WaveSignal, t: usize| wf_val(sig, t);
    let svo = |sig: Option<&WaveSignal>, t: usize| sig.map_or(0u64, |s| wf_val(s, t));
    let state = |t: usize| (sv(ns, t), sv(ew, t), svo(nsl, t), svo(ewl, t), svo(ped, t));

    let mut firsts: Vec<usize> = Vec::new();
    let mut prev: Option<(u64, u64, u64, u64, u64)> = None;
    for t in 0..n {
        let k = state(t);
        if Some(k) != prev {
            firsts.push(t);
            prev = Some(k);
        }
    }
    let states: Vec<(u64, u64, u64, u64, u64)> = firsts.iter().map(|&t| state(t)).collect();
    let nsc = states.len();
    let conflict = (0..nsc).find(|&i| {
        let (a, b, c, d, e) = states[i];
        (a != 0 && b != 0) || (e == 1 && (a != 0 || b != 0 || c != 0 || d != 0))
    });
    let mut phases = Vec::with_capacity(nsc);
    let mut acc = 0.0f64;
    for i in 0..nsc {
        let (a, b, c, d, e) = states[i];
        let dur = if Some(i) == conflict {
            3.6
        } else if e == 1 {
            7.0
        } else if a == 2 || b == 2 || c == 2 || d == 2 {
            1.3
        } else if a == 1 || b == 1 || c == 1 || d == 1 {
            2.6
        } else {
            1.0
        };
        phases.push(TrafficPhase {
            ns: a,
            ew: b,
            nsl: c,
            ewl: d,
            ped: e,
            start: acc,
            dur,
        });
        acc += dur;
    }
    Some(TrafficPlan {
        phases,
        firsts,
        total: acc.max(1.0),
        conflict,
    })
}

fn ns_drive(plan: &TrafficPlan, i: usize) -> NsDrive {
    if plan.phases[i].ns != 1 {
        NsDrive::Hidden
    } else if plan.conflict == Some(i) {
        NsDrive::CrashStop
    } else {
        NsDrive::Through
    }
}

/// The whole-second countdown shown on a phase's timer: `ceil(dur)`, `ceil(dur)-1`, … 1 (clamped
/// to a single digit).
fn countdown_seq(dur: f64) -> Vec<u32> {
    let secs = dur.ceil() as i32;
    (0..secs).map(|j| (secs - j).clamp(0, 9) as u32).collect()
}

/// Easter egg: if a counterexample carries the signals of a signalized intersection
/// (`ns`/`ew` through, optional `nsl`/`ewl` protected left turns, optional `ped` pedestrian),
/// render a live intersection whose lamps step through the REAL trace — then flash CONFLICT at
/// the exact cycle the safety property is violated. Pure SVG + CSS, no JS. `None` otherwise.
///
/// Vehicle signals encode 0=red, 1=green, 2=yellow; `ped` encodes 0=don't-walk, 1=walk.
pub(super) fn traffic_svg(wf: &logicaffeine_compile::codegen_sva::hw_pipeline::Waveform) -> Option<String> {
    use logicaffeine_compile::codegen_sva::hw_pipeline::WaveSignal;
    use std::fmt::Write as _;
    let get = |name: &str| {
        wf.signals
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    };
    let ns = get("ns")?;
    let ew = get("ew")?;
    let nsl = get("nsl");
    let ewl = get("ewl");
    let ped = get("ped");
    let sv = |sig: &WaveSignal, t: usize| wf_val(sig, t);

    // The phase plan + per-phase predicates are pure and unit-tested (see `traffic_viz_tests`).
    let plan = traffic_plan(wf)?;
    let scenes = &plan.firsts;
    let nsc = plan.phases.len();
    let conflict_scene = plan.conflict;
    let total = plan.total;
    let starts: Vec<f64> = plan.phases.iter().map(|ph| ph.start).collect();
    let durs: Vec<f64> = plan.phases.iter().map(|ph| ph.dur).collect();
    let p = |sec: f64| (sec / total * 100.0).clamp(0.0, 100.0);

    // (prefix, signal, head_x, head_y, label, countdown_x, countdown_y)
    let mut vheads: Vec<(&str, &WaveSignal, f64, f64, &str, f64, f64)> = vec![
        ("ns", ns, 18.0, 16.0, "NS", 100.0, 17.0),
        ("ew", ew, 18.0, 278.0, "EW", 100.0, 279.0),
    ];
    if let Some(s) = nsl {
        vheads.push(("nsl", s, 184.0, 16.0, "NS \u{21B0}", 266.0, 17.0));
    }
    if let Some(s) = ewl {
        vheads.push(("ewl", s, 184.0, 278.0, "EW \u{21B0}", 266.0, 279.0));
    }

    // A single seven-segment digit (lit segments only) at top-left (x, y).
    let seven_seg = |x: f64, y: f64, d: u32, color: &str| -> String {
        let segs: &str = match d {
            0 => "abcdef",
            1 => "bc",
            2 => "abged",
            3 => "abgcd",
            4 => "fgbc",
            5 => "afgcd",
            6 => "afgecd",
            7 => "abc",
            8 => "abcdefg",
            9 => "abcdfg",
            _ => "",
        };
        let (w, h, t) = (13.0f64, 22.0f64, 2.6f64);
        let half = h / 2.0;
        let mut g = String::new();
        for c in segs.chars() {
            let (rx, ry, rw, rh) = match c {
                'a' => (x + t, y, w - 2.0 * t, t),
                'b' => (x + w - t, y + t, t, half - 1.5 * t),
                'c' => (x + w - t, y + half + 0.5 * t, t, half - 1.5 * t),
                'd' => (x + t, y + h - t, w - 2.0 * t, t),
                'e' => (x, y + half + 0.5 * t, t, half - 1.5 * t),
                'f' => (x, y + t, t, half - 1.5 * t),
                'g' => (x + t, y + half - 0.5 * t, w - 2.0 * t, t),
                _ => (0.0, 0.0, 0.0, 0.0),
            };
            let _ = write!(
                g,
                r##"<rect x="{rx:.1}" y="{ry:.1}" width="{rw:.1}" height="{rh:.1}" rx="1" fill="{color}"/>"##
            );
        }
        g
    };
    // A small upright "walking person" glyph in `color`, centred on (cx, cy).
    let walk_icon = |cx: f64, cy: f64, color: &str| -> String {
        let mut g = String::new();
        let _ = write!(
            g,
            r##"<g stroke="{color}" stroke-width="2.6" stroke-linecap="round" fill="none">"##
        );
        let _ = write!(
            g,
            r##"<circle cx="{cx:.1}" cy="{:.1}" r="2.7" fill="{color}" stroke="none"/>"##,
            cy - 9.0
        );
        let _ = write!(
            g,
            r##"<line x1="{cx:.1}" y1="{:.1}" x2="{cx:.1}" y2="{:.1}"/>"##,
            cy - 6.0,
            cy + 1.0
        );
        let _ = write!(
            g,
            r##"<line x1="{cx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"##,
            cy - 4.0,
            cx - 4.0,
            cy - 1.0
        );
        let _ = write!(
            g,
            r##"<line x1="{cx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"##,
            cy - 4.0,
            cx + 4.0,
            cy - 2.0
        );
        let _ = write!(
            g,
            r##"<line x1="{cx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"##,
            cy + 1.0,
            cx - 4.0,
            cy + 8.0
        );
        let _ = write!(
            g,
            r##"<line x1="{cx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"##,
            cy + 1.0,
            cx + 4.0,
            cy + 7.0
        );
        g.push_str("</g>");
        g
    };
    // The orange raised-palm "DON'T WALK / STOP" hand, centred on (cx, cy).
    let hand_icon = |cx: f64, cy: f64| -> String {
        let mut g = String::from(r##"<g fill="#f59e0b">"##);
        let _ = write!(
            g,
            r##"<rect x="{:.1}" y="{:.1}" width="14" height="13" rx="4.5"/>"##,
            cx - 7.0,
            cy - 5.0
        );
        let _ = write!(
            g,
            r##"<rect x="{:.1}" y="{:.1}" width="3" height="8" rx="1.5"/>"##,
            cx - 7.0,
            cy - 11.0
        );
        let _ = write!(
            g,
            r##"<rect x="{:.1}" y="{:.1}" width="3" height="9" rx="1.5"/>"##,
            cx - 3.5,
            cy - 12.0
        );
        let _ = write!(
            g,
            r##"<rect x="{:.1}" y="{:.1}" width="3" height="9" rx="1.5"/>"##,
            cx + 0.5,
            cy - 12.0
        );
        let _ = write!(
            g,
            r##"<rect x="{:.1}" y="{:.1}" width="3" height="8" rx="1.5"/>"##,
            cx + 4.0,
            cy - 11.0
        );
        let _ = write!(
            g,
            r##"<rect x="{:.1}" y="{:.1}" width="5" height="3" rx="1.5"/>"##,
            cx - 10.5,
            cy - 1.0
        );
        g.push_str("</g>");
        g
    };

    let mut style = String::from("<style>");
    // Lamp tracks (step-end): bright on the active colour, dim otherwise — one key per phase.
    for (pfx, sig, _, _, _, _, _) in &vheads {
        for (suffix, want) in [("R", 0u64), ("Y", 2), ("G", 1)] {
            let _ = write!(style, "@keyframes {pfx}{suffix}{{");
            for i in 0..nsc {
                let op = if sv(sig, scenes[i]) == want {
                    "1"
                } else {
                    "0.12"
                };
                let _ = write!(style, "{:.2}%{{opacity:{op}}}", p(starts[i]));
            }
            let last = if sv(sig, scenes[nsc - 1]) == want {
                "1"
            } else {
                "0.12"
            };
            let _ = write!(style, "100%{{opacity:{last}}}}}");
        }
    }
    // Pedestrian WALK / STOP icon swap (full on/off).
    if let Some(pp) = ped {
        for (name, want) in [("pedWALK", 1u64), ("pedSTOP", 0)] {
            let _ = write!(style, "@keyframes {name}{{");
            for i in 0..nsc {
                let op = if sv(pp, scenes[i]) == want { "1" } else { "0" };
                let _ = write!(style, "{:.2}%{{opacity:{op}}}", p(starts[i]));
            }
            let last = if sv(pp, scenes[nsc - 1]) == want {
                "1"
            } else {
                "0"
            };
            let _ = write!(style, "100%{{opacity:{last}}}}}");
        }
    }
    // Vehicle platoons — cars are released on GREEN, drive straight THROUGH, and are hidden on
    // red (none left drifting/parked in the road). Opacity snaps per phase (step-end); position
    // glides (linear); nested groups compose them. On a crash run the NS platoon drives to the
    // crosswalk and FREEZES on impact, so its real lead car is the one that strikes the
    // pedestrian. Lead glyph base y=-30 / x=-30 → +240px lands on the south crosswalk, +360px
    // drives clear off-screen.
    {
        let _ = write!(style, "@keyframes nsplat_op{{");
        for i in 0..nsc {
            let o = if ns_drive(&plan, i) == NsDrive::Hidden {
                "0"
            } else {
                "1"
            };
            let _ = write!(style, "{:.2}%{{opacity:{o}}}", p(starts[i]));
        }
        let lo = if ns_drive(&plan, nsc - 1) == NsDrive::Hidden {
            "0"
        } else {
            "1"
        };
        let _ = write!(style, "100%{{opacity:{lo}}}}}");
        let _ = write!(style, "@keyframes nsplat_mv{{");
        for i in 0..nsc {
            let t0 = starts[i];
            match ns_drive(&plan, i) {
                NsDrive::Through => {
                    let _ = write!(style, "{:.2}%{{transform:translateY(0px)}}", p(t0));
                    let _ = write!(
                        style,
                        "{:.2}%{{transform:translateY(360px)}}",
                        p(t0 + durs[i])
                    );
                }
                NsDrive::CrashStop => {
                    let _ = write!(style, "{:.2}%{{transform:translateY(0px)}}", p(t0));
                    let _ = write!(style, "{:.2}%{{transform:translateY(240px)}}", p(t0 + 1.8));
                    let _ = write!(
                        style,
                        "{:.2}%{{transform:translateY(240px)}}",
                        p(t0 + durs[i])
                    );
                }
                NsDrive::Hidden => {
                    let _ = write!(style, "{:.2}%{{transform:translateY(0px)}}", p(t0));
                }
            }
        }
        let lty = match ns_drive(&plan, nsc - 1) {
            NsDrive::Through => "360px",
            NsDrive::CrashStop => "240px",
            NsDrive::Hidden => "0px",
        };
        let _ = write!(style, "100%{{transform:translateY({lty})}}}}");
    }
    {
        let _ = write!(style, "@keyframes ewplat_op{{");
        for i in 0..nsc {
            let o = if sv(ew, scenes[i]) == 1 { "1" } else { "0" };
            let _ = write!(style, "{:.2}%{{opacity:{o}}}", p(starts[i]));
        }
        let lo = if sv(ew, scenes[nsc - 1]) == 1 {
            "1"
        } else {
            "0"
        };
        let _ = write!(style, "100%{{opacity:{lo}}}}}");
        let _ = write!(style, "@keyframes ewplat_mv{{");
        for i in 0..nsc {
            let t0 = starts[i];
            if sv(ew, scenes[i]) == 1 {
                let _ = write!(style, "{:.2}%{{transform:translateX(0px)}}", p(t0));
                let _ = write!(
                    style,
                    "{:.2}%{{transform:translateX(360px)}}",
                    p(t0 + durs[i])
                );
            } else {
                let _ = write!(style, "{:.2}%{{transform:translateX(0px)}}", p(t0));
            }
        }
        let lty = if sv(ew, scenes[nsc - 1]) == 1 {
            "360px"
        } else {
            "0px"
        };
        let _ = write!(style, "100%{{transform:translateX({lty})}}}}");
    }
    // Safe runs: the pedestrian crosses the south crosswalk and reaches the far side unharmed.
    if ped.is_some() && conflict_scene.is_none() {
        let pp = ped.unwrap();
        let mut off = 0.0f64;
        let _ = write!(style, "@keyframes pedcross{{");
        for i in 0..nsc {
            let vis = if sv(pp, scenes[i]) == 1 { "1" } else { "0" };
            let _ = write!(
                style,
                "{:.2}%{{transform:translateX({off:.1}px);opacity:{vis}}}",
                p(starts[i])
            );
            if sv(pp, scenes[i]) == 1 {
                off += 11.0 * durs[i];
            }
        }
        let _ = write!(
            style,
            "100%{{transform:translateX({off:.1}px);opacity:0}}}}"
        );
    }
    // Crash runs: the pedestrian walks into the lane and is struck as the queue reaches them.
    if let Some(csi) = conflict_scene {
        let c0 = starts[csi];
        let impact = c0 + 1.8;
        let _ = write!(style, "@keyframes doomwalk{{0%{{opacity:0;transform:translateX(0px)}}{:.2}%{{opacity:0;transform:translateX(0px)}}{:.2}%{{opacity:1;transform:translateX(0px)}}{:.2}%{{opacity:1;transform:translateX(30px)}}{:.2}%{{opacity:0}}100%{{opacity:0}}}}", p(c0), p(c0 + 0.2), p(impact), p(impact + 0.25));
        let _ = write!(style, "@keyframes impact{{0%{{opacity:0}}{:.2}%{{opacity:0}}{:.2}%{{opacity:1}}100%{{opacity:0.95}}}}", p(impact), p(impact + 0.2));
    }
    // Protected-left turn cars: one vehicle curves through the intersection during each nsl/ewl
    // GREEN, so the left phases aren't visually dead. An L-path (approach, then turn).
    let nsl_g = nsl.and_then(|sig| (0..nsc).find(|&i| sv(sig, scenes[i]) == 1));
    let ewl_g = ewl.and_then(|sig| (0..nsc).find(|&i| sv(sig, scenes[i]) == 1));
    if let Some(i) = nsl_g {
        let (w0, wm, w1) = (
            p(starts[i]),
            p(starts[i] + durs[i] * 0.5),
            p(starts[i] + durs[i]),
        );
        let we = (w1 + 1.0).min(100.0);
        let _ = write!(style, "@keyframes nslturn{{0%{{opacity:0;transform:translate(0px,-210px)}}{w0:.2}%{{opacity:1;transform:translate(0px,-210px)}}{wm:.2}%{{opacity:1;transform:translate(0px,0px)}}{w1:.2}%{{opacity:1;transform:translate(175px,0px)}}{we:.2}%{{opacity:0;transform:translate(175px,0px)}}100%{{opacity:0}}}}");
    }
    if let Some(i) = ewl_g {
        let (w0, wm, w1) = (
            p(starts[i]),
            p(starts[i] + durs[i] * 0.5),
            p(starts[i] + durs[i]),
        );
        let we = (w1 + 1.0).min(100.0);
        let _ = write!(style, "@keyframes ewlturn{{0%{{opacity:0;transform:translate(-210px,0px)}}{w0:.2}%{{opacity:1;transform:translate(-210px,0px)}}{wm:.2}%{{opacity:1;transform:translate(0px,0px)}}{w1:.2}%{{opacity:1;transform:translate(0px,175px)}}{we:.2}%{{opacity:0;transform:translate(0px,175px)}}100%{{opacity:0}}}}");
    }
    // Seven-segment countdown timers: one digit per second of each lit phase, on the pedestrian
    // signal (amber) and beside each vehicle head (green while go, amber while caution).
    let mut cds: Vec<(f64, f64, &str, usize)> = Vec::new();
    if let Some(pp) = ped {
        for i in 0..nsc {
            if sv(pp, scenes[i]) == 1 {
                cds.push((286.0, 68.0, "#fbbf24", i));
            }
        }
    }
    for (_pfx, sig, _hx, _hy, _lab, cdx, cdy) in &vheads {
        for i in 0..nsc {
            let v = sv(sig, scenes[i]);
            if v == 1 || v == 2 {
                cds.push((*cdx, *cdy, if v == 1 { "#22c55e" } else { "#fbbf24" }, i));
            }
        }
    }
    let mut cd_body = String::new();
    let mut cdc = 0usize;
    for (x, y, color, i) in &cds {
        for (k, val) in countdown_seq(durs[*i]).into_iter().enumerate() {
            let t0 = starts[*i] + k as f64;
            let t1 = (t0 + 1.0).min(starts[*i] + durs[*i]);
            let name = format!("cd{cdc}");
            cdc += 1;
            let _ = write!(style, "@keyframes {name}{{0%{{opacity:0}}{:.2}%{{opacity:0}}{:.2}%{{opacity:1}}{:.2}%{{opacity:0}}100%{{opacity:0}}}}", p(t0), p(t0), p(t1));
            let _ = write!(
                cd_body,
                r##"<g class="st" style="animation-name:{name}">{}</g>"##,
                seven_seg(*x, *y, val, *color)
            );
        }
    }
    let _ = write!(style, ".st{{animation-duration:{total:.1}s;animation-timing-function:step-end;animation-iteration-count:infinite}}");
    let _ = write!(style, ".fl{{animation-duration:{total:.1}s;animation-timing-function:linear;animation-iteration-count:infinite}}");
    style.push_str("</style>");

    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg viewBox="0 0 320 320" width="100%" style="max-height:320px" xmlns="http://www.w3.org/2000/svg">"##
    );
    s.push_str(&style);
    // cross roads + lane markings
    s.push_str(r##"<rect x="0" y="128" width="320" height="64" fill="#26262b"/><rect x="128" y="0" width="64" height="320" fill="#26262b"/>"##);
    s.push_str(r##"<line x1="0" y1="160" x2="320" y2="160" stroke="#6b5d2e" stroke-width="2" stroke-dasharray="11 9"/><line x1="160" y1="0" x2="160" y2="320" stroke="#6b5d2e" stroke-width="2" stroke-dasharray="11 9"/>"##);
    // crosswalk hatching across the south leg
    if ped.is_some() {
        for i in 0..6 {
            let x = 133.0 + i as f64 * 10.0;
            let _ = write!(
                s,
                r##"<rect x="{x:.0}" y="202" width="5" height="22" fill="rgba(255,255,255,0.22)"/>"##
            );
        }
    }
    // NS platoon (cyan, southbound) + EW platoon (pink, eastbound). Outer group snaps opacity per
    // phase (released on green / hidden on red); inner group glides the position. Three cars each
    // so a green release reads as a small platoon; lead car base y=-30 / x=-30.
    s.push_str(r##"<g class="st" style="animation-name:nsplat_op"><g class="fl" style="animation-name:nsplat_mv">"##);
    for k in 0..3 {
        let y = -30 - k * 52;
        let _ = write!(
            s,
            r##"<rect x="139" y="{y}" width="18" height="30" rx="3" fill="#38bdf8"/><rect x="143" y="{}" width="10" height="4" rx="1" fill="rgba(0,0,0,0.35)"/>"##,
            y + 6
        );
    }
    s.push_str("</g></g>");
    s.push_str(r##"<g class="st" style="animation-name:ewplat_op"><g class="fl" style="animation-name:ewplat_mv">"##);
    for k in 0..3 {
        let x = -30 - k * 52;
        let _ = write!(
            s,
            r##"<rect x="{x}" y="163" width="30" height="18" rx="3" fill="#f472b6"/><rect x="{}" y="167" width="4" height="10" rx="1" fill="rgba(0,0,0,0.35)"/>"##,
            x + 6
        );
    }
    s.push_str("</g></g>");
    // Safe run: the pedestrian crosses unharmed.
    if ped.is_some() && conflict_scene.is_none() {
        let _ = write!(
            s,
            r##"<g class="fl" style="animation-name:pedcross">{}</g>"##,
            walk_icon(120.0, 209.0, "#fde047")
        );
    }
    // Protected-left turn cars (one per left phase) — cyan for NS-left, pink for EW-left.
    if nsl_g.is_some() {
        s.push_str(r##"<g class="fl" style="animation-name:nslturn"><rect x="140" y="142" width="16" height="16" rx="3" fill="#38bdf8"/><rect x="143" y="145" width="10" height="4" rx="1" fill="rgba(0,0,0,0.35)"/></g>"##);
    }
    if ewl_g.is_some() {
        s.push_str(r##"<g class="fl" style="animation-name:ewlturn"><rect x="140" y="142" width="16" height="16" rx="3" fill="#f472b6"/><rect x="143" y="145" width="10" height="4" rx="1" fill="rgba(0,0,0,0.35)"/></g>"##);
    }
    // Crash run: the doomed pedestrian who walks into the lane and is struck.
    if conflict_scene.is_some() && ped.is_some() {
        let _ = write!(
            s,
            r##"<g class="fl" style="animation-name:doomwalk">{}</g>"##,
            walk_icon(120.0, 209.0, "#fde047")
        );
    }
    // vehicle signal heads + dim "8" backdrop for each countdown
    for (pfx, _sig, x, y, label, cdx, cdy) in &vheads {
        let (rx, yx, gx) = (x + 16.0, x + 39.0, x + 62.0);
        let cy = y + 14.0;
        let lab_y = if *y < 100.0 { y - 4.0 } else { y + 42.0 };
        let _ = write!(
            s,
            r##"<rect x="{x:.0}" y="{y:.0}" width="78" height="28" rx="7" fill="#141418" stroke="rgba(255,255,255,0.15)"/>"##
        );
        let _ = write!(
            s,
            r##"<circle class="st" style="animation-name:{pfx}R" cx="{rx:.0}" cy="{cy:.0}" r="9" fill="#ef4444"/><circle class="st" style="animation-name:{pfx}Y" cx="{yx:.0}" cy="{cy:.0}" r="9" fill="#fbbf24"/><circle class="st" style="animation-name:{pfx}G" cx="{gx:.0}" cy="{cy:.0}" r="9" fill="#22c55e"/>"##
        );
        let _ = write!(
            s,
            r##"<text x="{:.0}" y="{lab_y:.0}" fill="rgba(255,255,255,0.6)" text-anchor="middle" font-size="11" font-family="ui-monospace,monospace">{label}</text>"##,
            x + 39.0
        );
        s.push_str(&seven_seg(*cdx, *cdy, 8, "rgba(255,255,255,0.06)"));
    }
    // pedestrian signal head: WALK person / STOP hand + a dim "8" countdown backdrop
    if ped.is_some() {
        s.push_str(r##"<rect x="238" y="54" width="72" height="52" rx="6" fill="#141418" stroke="rgba(255,255,255,0.15)"/>"##);
        let _ = write!(
            s,
            r##"<g class="st" style="animation-name:pedWALK">{}</g>"##,
            walk_icon(258.0, 80.0, "#34d399")
        );
        let _ = write!(
            s,
            r##"<g class="st" style="animation-name:pedSTOP">{}</g>"##,
            hand_icon(258.0, 80.0)
        );
        s.push_str(&seven_seg(286.0, 68.0, 8, "rgba(255,255,255,0.06)"));
        s.push_str(r##"<text x="274" y="100" fill="rgba(255,255,255,0.55)" text-anchor="middle" font-size="9" font-family="ui-monospace,monospace">PED</text>"##);
    }
    // the ticking countdown digits, drawn over their dim backdrops
    s.push_str(&cd_body);
    // Impact: the felled pedestrian (red), an impact burst, and the banner.
    if conflict_scene.is_some() {
        let mut g = String::from(r##"<g class="st" style="animation-name:impact">"##);
        g.push_str(r##"<circle cx="150" cy="207" r="15" fill="rgba(251,191,36,0.6)"/>"##);
        g.push_str(r##"<path d="M150,190 L155,202 L168,207 L155,212 L150,224 L145,212 L132,207 L145,202 Z" fill="#fbbf24"/>"##);
        if ped.is_some() {
            g.push_str(r##"<ellipse cx="174" cy="216" rx="12" ry="4.5" fill="#ef4444"/><circle cx="160" cy="214" r="4" fill="#ef4444"/>"##);
            g.push_str(r##"<text x="160" y="116" fill="#fca5a5" text-anchor="middle" font-size="13" font-weight="700" font-family="ui-sans-serif">⚠ CAR HITS PEDESTRIAN</text>"##);
        } else {
            g.push_str(r##"<text x="160" y="116" fill="#fca5a5" text-anchor="middle" font-size="14" font-weight="700" font-family="ui-sans-serif">⚠ CONFLICT</text>"##);
        }
        g.push_str("</g>");
        s.push_str(&g);
    }
    s.push_str("</svg>");
    Some(s)
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/formula_on_screen_tests.rs"]
mod formula_on_screen_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/panel_title_tests.rs"]
mod panel_title_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/logic_column_tests.rs"]
mod logic_column_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/knowledge_graph_tests.rs"]
mod knowledge_graph_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/waveform_tests.rs"]
mod waveform_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/traffic_viz_tests.rs"]
mod traffic_viz_tests;

/// The prerendered Studio is the page for a bare `/studio`, opened on `opened`, and the site serves
/// that one copy for every deep link to a file as well. So, before the copy paints its panels, this
/// reads the link's `file` and, when it names another file, sets the class that keeps the copy's
/// panels unpainted until the app draws the file the link asked for. It runs only in the prerendered
/// copy, ahead of the app and its router, and only compares: nothing from the address reaches the
/// page.
fn prerender_guard(opened: &str) -> String {
    let opened = serde_json::to_string(opened)
        .unwrap_or_default()
        .replace("</", "<\\/");
    PRERENDER_GUARD.replace("__OPENED__", &opened)
}

/// [`prerender_guard`]'s script. It reads `file` exactly as the router does: the whole query
/// percent-decoded (a `%` without two hex digits kept as it is, and the query left undecoded when the
/// bytes are not UTF-8), then split into pairs at `&` and `=`, the last `file` winning. Then it reads
/// the path as the opening does: an empty one is no file, and a path without a leading slash gains
/// one.
const PRERENDER_GUARD: &str = r#"(function () {
  var opened = __OPENED__;
  function decode(text) {
    var bytes = [], at = 0, utf8 = new TextEncoder();
    while (at < text.length) {
      if (text[at] === '%' && /^[0-9A-Fa-f]{2}$/.test(text.slice(at + 1, at + 3))) {
        bytes.push(parseInt(text.slice(at + 1, at + 3), 16));
        at += 3;
      } else {
        var ch = String.fromCodePoint(text.codePointAt(at));
        utf8.encode(ch).forEach(function (b) { bytes.push(b); });
        at += ch.length;
      }
    }
    try { return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(new Uint8Array(bytes)); }
    catch (e) { return text; }
  }
  var file = null;
  decode(location.search.slice(1)).split('&').forEach(function (pair) {
    var at = pair.indexOf('=');
    if (at >= 0 && pair.slice(0, at) === 'file') file = pair.slice(at + 1);
  });
  if (file === null || file === '' || (file.charAt(0) === '/' ? file : '/' + file) === opened) return;
  document.documentElement.classList.add('studio-opens-elsewhere');
})();"#;

/// Why New file cannot make a file yet: it writes to the reader's store, which has not answered, or
/// could not be opened. `None` once the store is ready.
fn why_new_file_waits(ready: bool, failed: bool) -> Option<String> {
    match (ready, failed) {
        (true, _) => None,
        (false, true) => Some("Your files could not be opened".to_string()),
        (false, false) => Some("Your files are still loading".to_string()),
    }
}

/// Keep the address bar on the canonical shareable URL for the open file
/// without pushing a history entry.
#[cfg(target_arch = "wasm32")]
fn sync_studio_url(vfs_path: &str) {
    crate::ui::router::replace_bar_url(&crate::ui::router::studio_file_url(vfs_path));
}

/// The next free `untitled[-N].<ext>` filename for a new Studio file, skipping any
/// name that already exists in `/workspace` (via `exists`). `untitled.logos`, then
/// `untitled-1.logos`, `untitled-2.logos`, … — so repeated "New file" clicks never
/// collide or overwrite an existing user file.
#[cfg(any(target_arch = "wasm32", test))]
fn fresh_untitled_name(ext: &str, exists: impl Fn(&str) -> bool) -> String {
    let base = format!("untitled.{ext}");
    if !exists(&base) {
        return base;
    }
    let mut n = 1;
    loop {
        let candidate = format!("untitled-{n}.{ext}");
        if !exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/new_file_tests.rs"]
mod new_file_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio/new_file_wait_tests.rs"]
mod new_file_wait_tests;

#[component(lazy)]
pub fn Studio(file: Option<String>) -> Element {
    // What the Studio opens on, decided from the route alone — the same on the server that
    // prerenders the page as in the browser — so its first frame is already the open file's mode
    // and text (studio_opening.rs).
    let opening = use_hook(|| StudioOpening::for_route(file.as_deref()));
    // The workbench: the Studio's toolbar, files, editor and output columns (studio_workbench.rs).
    // The examples' part of the tree is the site's own, there from the first frame. The reader's
    // files join it once their store answers.
    let state = use_workbench(&opening, examples_tree);
    #[cfg(target_arch = "wasm32")]
    let WorkbenchState { mut file_tree, mut current_file, mut fault, mut program_store, mode, .. } = state;
    let mut active_tab = state.active_tab;

    let mut vfs_error = use_signal(|| None::<String>); // Track VFS errors for display
    let vfs_is_fallback = use_signal(|| false); // Track if using IndexedDB fallback

    // VFS initialization flag
    let mut vfs_initialized = use_signal(|| false);

    // The Studio's files once the reader's store has answered: the shipped examples over that
    // store (studio_files.rs). Acquired once, then reused for every file switch instead of
    // spawning (and leaking) a fresh worker each time.
    #[cfg(target_arch = "wasm32")]
    let mut vfs_handle = use_signal(|| None::<StudioFiles<WebVfs>>);

    // Touch gesture state for swipe detection
    let mut touch_start_x = use_signal(|| 0.0f64);
    let mut touch_start_y = use_signal(|| 0.0f64);
    // Whether the touch began where a sideways swipe may change the pane: not in the editor or a wide
    // picture, which scroll sideways themselves, nor in the files or the debugger.
    let mut swipe_switches = use_signal(|| false);

    // On mount, the address bar names the opening's file, and the reader's store loads beside the
    // workbench (which runs that file at once when the site carries its text): its files join the
    // tree, and it answers for the opening's file, which changes the page only as
    // `StudioOpening::arrival` allows. Nothing is written to open the Studio; afterwards, copies of
    // the examples an earlier Studio wrote are removed.
    #[cfg(target_arch = "wasm32")]
    use_effect({
        let opening = opening.clone();
        move || {
            if *vfs_initialized.read() {
                return;
            }
            vfs_initialized.set(true);

            let opening = opening.clone();
            let mut vfs_is_fallback = vfs_is_fallback;
            if opening.text.is_some() {
                sync_studio_url(&opening.path);
            }

            spawn(async move {
                // The reader's store: OPFS, or IndexedDB where the browser offers no OPFS.
                let read = match get_platform_vfs_with_fallback().await {
                    Ok(store) => {
                        if store.is_fallback() {
                            vfs_is_fallback.set(true);
                        }
                        let files = StudioFiles::new(store);
                        vfs_handle.set(Some(files.clone()));
                        program_store.set(Some(std::sync::Arc::new(files.clone()) as std::sync::Arc<dyn Vfs>));
                        let (tree, read) = open_store(&files, &opening.path).await;
                        match tree {
                            Ok(tree) => {
                                let tree = tree.with_expansion_of(&file_tree.peek());
                                if *file_tree.peek() != tree {
                                    file_tree.set(tree);
                                }
                            }
                            Err(e) => {
                                vfs_error.set(Some(format!("Failed to load file tree: {:?}", e)));
                            }
                        }
                        read
                    }
                    Err(e) => {
                        vfs_error.set(Some(format!("VFS INIT FAILED: {:?}", e)));
                        None
                    }
                };

                let open = current_file.peek().clone();
                let shown = state.editor(opening.mode).peek().clone();
                let replacement = match opening.arrival(open.as_deref(), &shown, read) {
                    Arrival::Keep => None,
                    Arrival::Open(content) => Some((opening.path, content)),
                    Arrival::Fallback(first) => {
                        Some((first.path, first.text.unwrap_or_default().to_string()))
                    }
                };
                if let Some((path, content)) = replacement {
                    current_file.set(Some(path.clone()));
                    sync_studio_url(&path);
                    state.open(path, content).await;
                }

                let files = vfs_handle.peek().clone();
                if let Some(files) = files {
                    let _ = files.retire_old_copies().await;
                }
            });
        }
    });

    // On native, no VFS - just show empty tree with message
    #[cfg(not(target_arch = "wasm32"))]
    use_effect(move || {
        if *vfs_initialized.read() {
            return;
        }
        vfs_initialized.set(true);
        vfs_error.set(Some("VFS not available on native".to_string()));
    });

    // Mobile touch handlers for swipe gestures
    let panes_fixed = state.panes_fixed;
    let handle_touch_start = move |evt: TouchEvent| {
        let touches = evt.data().touches();
        if let Some(touch) = touches.first() {
            let coords = touch.client_coordinates();
            touch_start_x.set(coords.x);
            touch_start_y.set(coords.y);
        }
        #[cfg(target_arch = "wasm32")]
        let scrolls_itself = {
            use wasm_bindgen::JsCast;
            evt.data()
                .downcast::<web_sys::TouchEvent>()
                .and_then(|e| e.target())
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                .and_then(|el| el.closest(".code-editor, .panel-body.wide, .studio-sidebar, .dbg-drawer").ok().flatten())
                .is_some()
        };
        #[cfg(not(target_arch = "wasm32"))]
        let scrolls_itself = false;
        swipe_switches.set(*panes_fixed.peek() && !scrolls_itself);
    };

    let handle_touch_end = move |evt: TouchEvent| {
        let changed = evt.data().touches_changed();
        if let Some(touch) = changed.first() {
            let coords = touch.client_coordinates();
            let end_x = coords.x;
            let end_y = coords.y;
            let dx = end_x - *touch_start_x.read();
            let dy = end_y - *touch_start_y.read();

            if *swipe_switches.peek() && dx.abs() > dy.abs() && dx.abs() > 50.0 {
                let current = *active_tab.read();
                if dx < 0.0 {
                    match current {
                        MobileTab::Panel1 => active_tab.set(MobileTab::Panel2),
                        MobileTab::Panel2 => active_tab.set(MobileTab::Panel3),
                        MobileTab::Panel3 => {}
                    }
                } else {
                    match current {
                        MobileTab::Panel1 => {}
                        MobileTab::Panel2 => active_tab.set(MobileTab::Panel1),
                        MobileTab::Panel3 => active_tab.set(MobileTab::Panel2),
                    }
                }
            }
        }
    };

    // New file writes to the reader's store, so it waits, and says so, until the store has answered.
    #[cfg(target_arch = "wasm32")]
    let files_ready = vfs_handle.read().is_some();
    #[cfg(not(target_arch = "wasm32"))]
    let files_ready = false;
    let new_file_waits = why_new_file_waits(files_ready, vfs_error.read().is_some());

    // A file chosen in the Files tree, which the workbench has opened when the site carries it: the
    // address bar names it, and a reader's own file is read from their store.
    let on_choose = move |path: String| {
        #[cfg(target_arch = "wasm32")]
        {
            // Update URL with file parameter for shareable links
            sync_studio_url(&path);
            if example_text(&path).is_none() {
                let path_clone = path.clone();
                spawn(async move {
                    // Reuse the cached files (one worker) instead of spawning a
                    // fresh one per file switch; acquire once if not ready yet.
                    // Bind the clone first so the peek() read-guard drops before
                    // the None arm calls vfs_handle.set() (avoids a borrow panic).
                    let cached = vfs_handle.peek().clone();
                    let vfs_result = match cached {
                        Some(vfs) => Ok(vfs),
                        None => get_platform_vfs_with_fallback().await.map(|store| {
                            let files = StudioFiles::new(store);
                            vfs_handle.set(Some(files.clone()));
                            program_store.set(Some(std::sync::Arc::new(files.clone()) as std::sync::Arc<dyn Vfs>));
                            files
                        }),
                    };
                    match vfs_result {
                        Ok(vfs) => {
                            match vfs.read_to_string(&path_clone).await {
                                Ok(content) => {
                                    active_tab.set(MobileTab::Panel2);
                                    state.open(path_clone, content).await;
                                }
                                Err(e) => {
                                    vfs_error.set(Some(format!("Failed to read file: {:?}", e)));
                                }
                            }
                        }
                        Err(e) => {
                            vfs_error.set(Some(format!("VFS INIT FAILED: {:?}", e)));
                        }
                    }
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = path;
    };

    // New file writes a file into the reader's store and opens it.
    let on_new_file = move |_: ()| {
        // VFS + editor signals are the browser's; the SSR prerender has no
        // filesystem, so the whole action is wasm-only (mirrors on_select).
        #[cfg(target_arch = "wasm32")]
        {
            let vfs = vfs_handle.read().clone();
            let current = *mode.read();
            spawn(async move {
                let Some(vfs) = vfs else { return; };
                let (ext, template): (&str, &str) = match current {
                    StudioMode::Code => ("logos", "Show \"Hello from a new file!\".\n"),
                    StudioMode::Math => ("math", "-- A fresh proof. Define your terms, then prove them.\n"),
                    StudioMode::Hardware => ("hw", "The grant signal is asserted within 2 cycles of a request.\n"),
                    StudioMode::Logic => ("logic", "Every philosopher thinks.\n"),
                    // The Studio's own counter example, minus the commentary: a new Web
                    // file has to COMPILE, because the preview builds it on the next
                    // keystroke — an empty or prose placeholder would greet the user
                    // with a build error.
                    StudioMode::Web => (
                        "web",
                        "## Component App\nState:\n    Let count be 0.\nActions:\n    To bump:\n        Set count to count + 1.\nView:\n    <h1>Count: {count}</h1>\n    <button on-click={bump}>+1</button>\n\n## Main\nMount App.\n",
                    ),
                };
                // A user file lives under /workspace, apart from the shipped examples
                // (so it survives reloads), and we create the folder on demand so
                // the tree never shows an empty workspace.
                let existing: std::collections::HashSet<String> = match vfs.list_dir("/workspace").await {
                    Ok(entries) => entries.into_iter().map(|e| e.name).collect(),
                    Err(_) => std::collections::HashSet::new(),
                };
                let path = format!("/workspace/{}", fresh_untitled_name(ext, |n| existing.contains(n)));
                if vfs.create_dir_all("/workspace").await.is_err() { return; }
                if vfs.write(&path, template.as_bytes()).await.is_err() { return; }
                // Rebuild the tree so the new file appears, every folder left open
                // or closed as the reader had it.
                let mut root = FileNode::root();
                if load_dir_recursive(&vfs, "/", &mut root).await.is_ok() {
                    let root = root.with_expansion_of(&file_tree.peek());
                    file_tree.set(root);
                }
                // Open it in the current mode's editor.
                current_file.set(Some(path.clone()));
                if fault.peek().is_some() {
                    fault.set(None);
                }
                state.editor(current).set(template.to_string());
                sync_studio_url(&path);
            });
        }
    };

    rsx! {
        PageHead {
            title: seo_pages::STUDIO.title,
            description: seo_pages::STUDIO.description,
            canonical_path: seo_pages::STUDIO.canonical_path,
        }
        style { "{MOBILE_BASE_STYLES}" }
        style { "{MOBILE_TAB_BAR_STYLES}" }
        style { "{STUDIO_STYLE}" }
        style { "{crate::ui::drag::DRAG_STYLE}" }
        if cfg!(not(target_arch = "wasm32")) {
            script { dangerous_inner_html: prerender_guard(&opening.path) }
        }
        JsonLdMultiple { schemas: vec![
            organization_schema(),
            software_application_schema(),
            breadcrumb_schema(&[
                BreadcrumbItem { name: "Home", path: "/" },
                BreadcrumbItem { name: "Studio", path: "/studio" },
            ]),
        ] }

        div {
            class: "studio-container",
            ontouchstart: handle_touch_start,
            ontouchend: handle_touch_end,

            MainNav { active: ActivePage::Studio, subtitle: Some("Your logic workspace") }
            Workbench {
                state,
                host: Host::Page,
                store: StoreNote {
                    error: vfs_error.read().clone(),
                    private_mode: *vfs_is_fallback.read(),
                    new_file_waits,
                },
                on_new_file: Some(EventHandler::new(on_new_file)),
                on_choose: Some(EventHandler::new(on_choose)),
            }
        }
    }
}
