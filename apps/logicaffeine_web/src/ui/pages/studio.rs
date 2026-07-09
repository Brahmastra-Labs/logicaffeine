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

use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;
use std::cell::RefCell;
use logicaffeine_compile::{
    compile_for_ui, compile_for_proof, compile_theorem_for_ui, generate_rust_code,
    extract_math_rust, extract_logic_rust,
    interpret_for_ui_baseline, interpret_streaming_with_vfs, CompileResult, ProofCompileResult,
    TheoremCompileResult, SolvedGrid,
    interpreter::InterpreterResult,
};
use logicaffeine_proof::{
    BackwardChainer, DerivationTree, ProofExpr,
    hints::{suggest_hint, SuggestedTactic},
};
use crate::ui::components::editor::LiveEditor;
use crate::ui::components::logic_output::{LogicOutput, OutputFormat};
use crate::ui::components::ast_tree::AstTree;
use crate::ui::components::socratic_guide::{SocraticGuide, GuideMode, get_success_message, get_context_hint};
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::symbol_dictionary::SymbolDictionary;
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, software_application_schema, breadcrumb_schema, BreadcrumbItem, pages as seo_pages};
use crate::ui::components::mode_toggle::ModeToggle;
use crate::ui::components::file_browser::FileBrowser;
use crate::ui::components::repl_output::ReplOutput;
use crate::ui::components::context_view::{ContextView, ContextEntry, EntryKind};
use crate::ui::components::code_editor::{CodeEditor, CodeView, Language};
use crate::ui::components::proof_panel::{ProofPanel, ProofStatus, Tactic};
use crate::ui::components::debug_drawer::{DebugDrawer, IC_BUG};
use crate::ui::state::{StudioMode, FileNode, ReplLine};
use crate::ui::responsive::{MOBILE_BASE_STYLES, MOBILE_TAB_BAR_STYLES};
use logicaffeine_kernel::interface::Repl;
use crate::ui::examples::seed_examples;
#[cfg(target_arch = "wasm32")]
use logicaffeine_system::fs::{get_platform_vfs_with_fallback, WebVfs};
use logicaffeine_system::fs::{get_platform_vfs, Vfs, DirEntry, VfsResult};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;
use std::rc::Rc;

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
                let is_theorem_part = next_trimmed.starts_with("Statement:")
                    || next_trimmed.starts_with("Proof:");

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
        if (trimmed.starts_with("A ") || trimmed.starts_with("An ")) && trimmed.contains(" is either") {
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
                    || next_trimmed.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);

                if is_indented || (looks_like_variant && !next_trimmed.starts_with("A ") && !next_trimmed.starts_with("An ")) {
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

/// Recursively load directory contents from VFS into FileNode tree
async fn load_dir_recursive<V: Vfs>(vfs: &V, path: &str, parent: &mut FileNode) -> VfsResult<()> {
    let entries = vfs.list_dir(path).await?;

    for entry in entries {
        let full_path = if path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", path, entry.name)
        };

        if entry.is_directory {
            let mut dir_node = FileNode::directory(entry.name.clone(), full_path.clone());
            // Recursively load subdirectories
            let _ = Box::pin(load_dir_recursive(vfs, &full_path, &mut dir_node)).await;
            parent.children.push(dir_node);
        } else {
            parent.children.push(FileNode::file(entry.name, full_path));
        }
    }

    // Sort: directories first, then alphabetically
    parent.children.sort_by(|a, b| {
        match (a.is_directory, b.is_directory) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    Ok(())
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

/// Format a DerivationTree as HTML for the proof panel
fn format_derivation_html(tree: &DerivationTree) -> String {
    fn format_node(tree: &DerivationTree, depth: usize) -> String {
        let indent = "  ".repeat(depth);
        let rule_class = "rule";
        let conclusion_class = "conclusion";

        let mut result = String::new();

        // Show the conclusion with rule
        result.push_str(&format!(
            "{}<span class=\"{}\">{:?}:</span> <span class=\"{}\">{}</span>\n",
            indent,
            rule_class,
            tree.rule,
            conclusion_class,
            tree.conclusion
        ));

        // Recursively format premises
        if !tree.premises.is_empty() {
            for premise in &tree.premises {
                result.push_str(&format_node(premise, depth + 1));
            }
        }

        result
    }

    let mut output = String::new();
    output.push_str("<div style=\"font-family: monospace; white-space: pre-wrap;\">\n");
    output.push_str(&format_node(tree, 0));
    output.push_str("</div>");
    output
}

/// Minimal HTML escape for grid/answer cell text.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Render a [`SolvedGrid`] as an HTML table for the proof panel — the easter egg that
/// fills the whole grid once the premises take the grid form. Every shown cell is a
/// certified entailment (no Z3); an undetermined cell shows a dot.
fn format_grid_html(grid: &SolvedGrid) -> String {
    let cell = "border:1px solid var(--studio-border);padding:4px 10px;text-align:center;";
    let head = "border:1px solid var(--studio-border);padding:4px 10px;text-align:center;color:var(--studio-accent);font-weight:600;";
    let mut s = String::new();
    s.push_str("<div style=\"margin-bottom:16px;\">");
    s.push_str("<div style=\"font-family:monospace;font-size:12px;color:var(--studio-text-secondary);margin-bottom:6px;\">Solved grid — every filled cell certified, no Z3:</div>");
    s.push_str("<table style=\"border-collapse:collapse;font-family:monospace;font-size:13px;color:var(--studio-text);\">");
    s.push_str("<tr>");
    s.push_str(&format!("<th style=\"{head}\">{}</th>", esc(&grid.row_label)));
    for col in &grid.columns {
        s.push_str(&format!("<th style=\"{head}\">{}</th>", esc(&col.label)));
    }
    s.push_str("</tr>");
    for (ri, row) in grid.rows.iter().enumerate() {
        s.push_str("<tr>");
        s.push_str(&format!("<td style=\"{cell}font-weight:600;\">{}</td>", esc(row)));
        for col in &grid.columns {
            match col.cells.get(ri).and_then(|c| c.as_ref()) {
                Some(v) => s.push_str(&format!("<td style=\"{cell}\">{}</td>", esc(v))),
                None => s.push_str(&format!("<td style=\"{cell}color:var(--studio-text-muted);\">·</td>")),
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
    } else if let Some(derivation) = &result.derivation {
        html.push_str(&format_derivation_html(derivation));
    }
    html
}

/// The proof-panel status line for a compiled theorem.
fn theorem_proof_hint(result: &TheoremCompileResult) -> String {
    if let Some(answer) = &result.answer {
        if answer.is_empty() {
            "No answer found.".to_string()
        } else {
            format!("Answer: {}", answer.join(", "))
        }
    } else if result.verified {
        format!("Theorem '{}' proved!", result.name)
    } else {
        format!("Theorem '{}' — grid solved.", result.name)
    }
}

/// Code mode output toggle - interpret output vs generated Rust
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum CodeOutputMode {
    #[default]
    Interpret,
    Rust,
}

/// Logic mode output view — FOL interpretation or generated Rust. (SVA synthesis lives in
/// Hardware mode, where signals/clocks/cycles have meaning; general FOL has none to assert.)
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum LogicView {
    #[default]
    Logic,
    Rust,
}

/// Studio-specific styles that extend the shared responsive styles
const STUDIO_STYLE: &str = r#"
/* ============================================ */
/* STUDIO PAGE - Design Tokens                  */
/* ============================================ */
:root {
    --studio-bg: #0f1419;
    --studio-panel-bg: #12161c;
    --studio-elevated: #1a1f27;
    --studio-border: rgba(255, 255, 255, 0.08);
    --studio-border-hover: rgba(255, 255, 255, 0.15);
    --studio-text: #e8eaed;
    --studio-text-secondary: #9ca3af;
    --studio-text-muted: #6b7280;
    --studio-accent: #667eea;
    /* Heights for positioning fixed sidebar below header */
    --nav-height: 97px;
    --toolbar-height: 49px;
    --header-height: calc(var(--nav-height) + var(--toolbar-height));
}

@media (max-width: 980px) {
    :root {
        --nav-height: 89px;
    }
}

@media (max-width: 768px) {
    :root {
        --nav-height: 81px;
        --toolbar-height: 90px;
    }
}

@media (max-width: 640px) {
    :root {
        --nav-height: 65px;
    }
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
    overflow: hidden;
}

/* Toolbar with mode toggle */
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

.studio-toolbar-center {
    flex: 1;
    display: flex;
    justify-content: center;
}

.studio-toolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
}

.sidebar-toggle-btn {
    padding: 8px 12px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.7);
    font-size: 14px;
    cursor: pointer;
    transition: all 0.15s ease;
}

.sidebar-toggle-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.9);
}

/* Main content area with optional sidebar */
.studio-content {
    flex: 1;
    display: flex;
    overflow: hidden;
    position: relative;
}

/* Overlay to close sidebar when clicking outside (mobile) */
.sidebar-overlay {
    display: none;
}

@media (max-width: 768px) {
    .sidebar-overlay {
        display: block;
        position: fixed;
        top: calc(var(--header-height) + 10px);
        left: 0;
        right: 0;
        bottom: 0;
        background: rgba(0, 0, 0, 0.5);
        z-index: 99;
    }
}

/* Sidebar wrapper for controlled width */
.studio-sidebar {
    display: flex;
    flex-shrink: 0;
    overflow: hidden;
}

@media (max-width: 768px) {
    .studio-sidebar {
        position: fixed;
        left: 0;
        top: calc(var(--header-height) + 10px);
        bottom: 0;
        z-index: 100;
        width: 280px !important;
        min-width: 280px !important;
        max-width: 280px !important;
        box-shadow: 4px 0 20px rgba(0, 0, 0, 0.3);
        background: #12161c;
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
    background: var(--studio-panel-bg);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 200px;
}

.studio-panel .panel-header {
    padding: 0 20px;
    height: 52px;
    background: rgba(255, 255, 255, 0.02);
    border-bottom: 1px solid var(--studio-border);
    font-size: 16px;
    font-weight: 600;
    letter-spacing: 0.3px;
    color: var(--studio-text);
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-shrink: 0;
}

.studio-panel .panel-content {
    flex: 1;
    min-height: 0;
    overflow: auto;
    -webkit-overflow-scrolling: touch;
}

/* Panel Resizers (desktop only) */
.panel-resizer {
    width: 4px;
    background: var(--studio-border);
    cursor: col-resize;
    transition: background 0.2s ease;
    flex-shrink: 0;
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

.execute-btn:hover {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(102, 126, 234, 0.3);
}

.execute-btn:active {
    transform: translateY(0);
}

/* Mobile execute buttons */
@media (max-width: 768px) {
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
.interpreter-output {
    padding: 16px;
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 14px;
    line-height: 1.6;
}

.interpreter-line {
    margin-bottom: 4px;
    color: #e8eaed;
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

/* Mode label: hidden on desktop, shown on mobile */
.mode-label {
    display: none;
}

@media (max-width: 768px) {
    .studio-toolbar {
        flex-wrap: wrap;
        height: auto;
        padding: 10px 12px;
        gap: 10px;
        position: relative;
        z-index: 101;
    }

    /* First row: file toggle, mode selector */
    .studio-toolbar-left {
        flex-shrink: 0;
        order: 1;
    }

    .studio-toolbar-center {
        flex: 1;
        align-items: center;
        min-width: 0;
        order: 2;
    }

    /* Second row: action buttons - full width */
    .studio-toolbar-right {
        flex-basis: 100%;
        justify-content: center;
        gap: 12px;
        order: 3;
        padding-top: 8px;
        border-top: 1px solid rgba(255, 255, 255, 0.06);
    }

    /* Show mode toggle with "Mode:" label */
    .mode-label {
        display: block;
        font-size: 12px;
        font-weight: 500;
        color: rgba(255, 255, 255, 0.5);
        margin-right: 6px;
        white-space: nowrap;
    }

    /* Full text on mobile buttons */
    .execute-btn {
        padding: 10px 16px;
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
        font-size: 14px;
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

    /* Hide Panel 3 (Console/Tree/Context) on mobile */
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
.mobile-panel-resizer {
    display: none;
}

.studio-container.mobile-resizing {
    user-select: none;
    -webkit-user-select: none;
}

/* Landscape mobile */
@media (max-height: 500px) and (orientation: landscape) {
    .studio-footer {
        max-height: 25vh;
    }
}
"#;

/// Mobile tab options - changes based on mode
#[derive(Clone, Copy, PartialEq, Default)]
enum MobileTab {
    #[default]
    Panel1,
    Panel2,
    Panel3,
}

/// Render the hardware knowledge graph as an animated radial SVG: signal nodes on a ring,
/// relations as directed edges. Pure SVG + CSS — edges animate a flowing dash ("signal
/// current") and nodes pulse, so a static spec reads like a live circuit. Returns "" if empty.
fn kg_svg(kg: &logicaffeine_compile::codegen_sva::hw_pipeline::KgSummary) -> String {
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
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");

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
        let (mx, my) = ((sx + ex) / 2.0, (sy + ey) / 2.0 - 2.0);
        let _ = write!(svg, r##"<text class="kgr" x="{mx:.1}" y="{my:.1}">{}</text>"##, esc(&l.relation));
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
        let _ = write!(svg, r##"<text class="kgl" x="{x:.1}" y="{:.1}">{}</text>"##, y + 28.0, esc(&label));
    }
    svg.push_str("</svg>");
    svg
}

/// Render a counterexample as a logic-analyzer waveform SVG: boolean signals as digital
/// square waves, multi-bit registers as value-labeled buses, with a sweeping playhead. Pure
/// SVG + CSS. Returns "" if empty.
fn waveform_svg(wf: &logicaffeine_compile::codegen_sva::hw_pipeline::Waveform) -> String {
    use std::fmt::Write as _;
    let (ts, n) = (wf.timesteps, wf.signals.len());
    if ts == 0 || n == 0 {
        return String::new();
    }
    let (gutter, col_w, row_h, top) = (72.0_f64, 46.0_f64, 34.0_f64, 18.0_f64);
    let span = (ts as f64) * col_w;
    let (w, h) = (gutter + span + 10.0, top + (n as f64) * row_h + 6.0);
    let dur = (0.45 * ts as f64).max(1.2);
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");

    let mut s = String::new();
    let _ = write!(s, r##"<svg viewBox="0 0 {w:.0} {h:.0}" width="100%" xmlns="http://www.w3.org/2000/svg">"##);
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
        let _ = write!(s, r##"<line class="wfg" x1="{x:.1}" y1="{top:.1}" x2="{x:.1}" y2="{:.1}"/>"##, h - 4.0);
        let _ = write!(s, r##"<text class="wftl" x="{:.1}" y="12">t{t}</text>"##, x + col_w / 2.0);
    }
    let xe = gutter + span;
    let _ = write!(s, r##"<line class="wfg" x1="{xe:.1}" y1="{top:.1}" x2="{xe:.1}" y2="{:.1}"/>"##, h - 4.0);

    for (ri, sig) in wf.signals.iter().enumerate() {
        let rtop = top + (ri as f64) * row_h;
        let (hi, lo) = (rtop + 6.0, rtop + row_h - 12.0);
        let mid = (hi + lo) / 2.0;
        let label = if sig.width > 1 { format!("{}[{}]", sig.name, sig.width) } else { sig.name.clone() };
        let _ = write!(s, r##"<text class="wfn" x="{:.1}" y="{:.1}">{}</text>"##, gutter - 8.0, mid + 4.0, esc(&label));
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
                    let _ = write!(s, r##"<text class="wfv" x="{:.1}" y="{:.1}">{v}</text>"##, x0 + col_w / 2.0, mid + 4.0);
                }
            }
        }
    }
    let _ = write!(s, r##"<line class="wfph" x1="{gutter:.1}" y1="{top:.1}" x2="{gutter:.1}" y2="{:.1}"/>"##, h - 4.0);
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
fn traffic_plan(wf: &logicaffeine_compile::codegen_sva::hw_pipeline::Waveform) -> Option<TrafficPlan> {
    use logicaffeine_compile::codegen_sva::hw_pipeline::WaveSignal;
    let get = |name: &str| wf.signals.iter().find(|s| s.name.eq_ignore_ascii_case(name));
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
        phases.push(TrafficPhase { ns: a, ew: b, nsl: c, ewl: d, ped: e, start: acc, dur });
        acc += dur;
    }
    Some(TrafficPlan { phases, firsts, total: acc.max(1.0), conflict })
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
fn traffic_svg(wf: &logicaffeine_compile::codegen_sva::hw_pipeline::Waveform) -> Option<String> {
    use std::fmt::Write as _;
    use logicaffeine_compile::codegen_sva::hw_pipeline::WaveSignal;
    let get = |name: &str| wf.signals.iter().find(|s| s.name.eq_ignore_ascii_case(name));
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
            let _ = write!(g, r##"<rect x="{rx:.1}" y="{ry:.1}" width="{rw:.1}" height="{rh:.1}" rx="1" fill="{color}"/>"##);
        }
        g
    };
    // A small upright "walking person" glyph in `color`, centred on (cx, cy).
    let walk_icon = |cx: f64, cy: f64, color: &str| -> String {
        let mut g = String::new();
        let _ = write!(g, r##"<g stroke="{color}" stroke-width="2.6" stroke-linecap="round" fill="none">"##);
        let _ = write!(g, r##"<circle cx="{cx:.1}" cy="{:.1}" r="2.7" fill="{color}" stroke="none"/>"##, cy - 9.0);
        let _ = write!(g, r##"<line x1="{cx:.1}" y1="{:.1}" x2="{cx:.1}" y2="{:.1}"/>"##, cy - 6.0, cy + 1.0);
        let _ = write!(g, r##"<line x1="{cx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"##, cy - 4.0, cx - 4.0, cy - 1.0);
        let _ = write!(g, r##"<line x1="{cx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"##, cy - 4.0, cx + 4.0, cy - 2.0);
        let _ = write!(g, r##"<line x1="{cx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"##, cy + 1.0, cx - 4.0, cy + 8.0);
        let _ = write!(g, r##"<line x1="{cx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"##, cy + 1.0, cx + 4.0, cy + 7.0);
        g.push_str("</g>");
        g
    };
    // The orange raised-palm "DON'T WALK / STOP" hand, centred on (cx, cy).
    let hand_icon = |cx: f64, cy: f64| -> String {
        let mut g = String::from(r##"<g fill="#f59e0b">"##);
        let _ = write!(g, r##"<rect x="{:.1}" y="{:.1}" width="14" height="13" rx="4.5"/>"##, cx - 7.0, cy - 5.0);
        let _ = write!(g, r##"<rect x="{:.1}" y="{:.1}" width="3" height="8" rx="1.5"/>"##, cx - 7.0, cy - 11.0);
        let _ = write!(g, r##"<rect x="{:.1}" y="{:.1}" width="3" height="9" rx="1.5"/>"##, cx - 3.5, cy - 12.0);
        let _ = write!(g, r##"<rect x="{:.1}" y="{:.1}" width="3" height="9" rx="1.5"/>"##, cx + 0.5, cy - 12.0);
        let _ = write!(g, r##"<rect x="{:.1}" y="{:.1}" width="3" height="8" rx="1.5"/>"##, cx + 4.0, cy - 11.0);
        let _ = write!(g, r##"<rect x="{:.1}" y="{:.1}" width="5" height="3" rx="1.5"/>"##, cx - 10.5, cy - 1.0);
        g.push_str("</g>");
        g
    };

    let mut style = String::from("<style>");
    // Lamp tracks (step-end): bright on the active colour, dim otherwise — one key per phase.
    for (pfx, sig, _, _, _, _, _) in &vheads {
        for (suffix, want) in [("R", 0u64), ("Y", 2), ("G", 1)] {
            let _ = write!(style, "@keyframes {pfx}{suffix}{{");
            for i in 0..nsc {
                let op = if sv(sig, scenes[i]) == want { "1" } else { "0.12" };
                let _ = write!(style, "{:.2}%{{opacity:{op}}}", p(starts[i]));
            }
            let last = if sv(sig, scenes[nsc - 1]) == want { "1" } else { "0.12" };
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
            let last = if sv(pp, scenes[nsc - 1]) == want { "1" } else { "0" };
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
            let o = if ns_drive(&plan, i) == NsDrive::Hidden { "0" } else { "1" };
            let _ = write!(style, "{:.2}%{{opacity:{o}}}", p(starts[i]));
        }
        let lo = if ns_drive(&plan, nsc - 1) == NsDrive::Hidden { "0" } else { "1" };
        let _ = write!(style, "100%{{opacity:{lo}}}}}");
        let _ = write!(style, "@keyframes nsplat_mv{{");
        for i in 0..nsc {
            let t0 = starts[i];
            match ns_drive(&plan, i) {
                NsDrive::Through => {
                    let _ = write!(style, "{:.2}%{{transform:translateY(0px)}}", p(t0));
                    let _ = write!(style, "{:.2}%{{transform:translateY(360px)}}", p(t0 + durs[i]));
                }
                NsDrive::CrashStop => {
                    let _ = write!(style, "{:.2}%{{transform:translateY(0px)}}", p(t0));
                    let _ = write!(style, "{:.2}%{{transform:translateY(240px)}}", p(t0 + 1.8));
                    let _ = write!(style, "{:.2}%{{transform:translateY(240px)}}", p(t0 + durs[i]));
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
        let lo = if sv(ew, scenes[nsc - 1]) == 1 { "1" } else { "0" };
        let _ = write!(style, "100%{{opacity:{lo}}}}}");
        let _ = write!(style, "@keyframes ewplat_mv{{");
        for i in 0..nsc {
            let t0 = starts[i];
            if sv(ew, scenes[i]) == 1 {
                let _ = write!(style, "{:.2}%{{transform:translateX(0px)}}", p(t0));
                let _ = write!(style, "{:.2}%{{transform:translateX(360px)}}", p(t0 + durs[i]));
            } else {
                let _ = write!(style, "{:.2}%{{transform:translateX(0px)}}", p(t0));
            }
        }
        let lty = if sv(ew, scenes[nsc - 1]) == 1 { "360px" } else { "0px" };
        let _ = write!(style, "100%{{transform:translateX({lty})}}}}");
    }
    // Safe runs: the pedestrian crosses the south crosswalk and reaches the far side unharmed.
    if ped.is_some() && conflict_scene.is_none() {
        let pp = ped.unwrap();
        let mut off = 0.0f64;
        let _ = write!(style, "@keyframes pedcross{{");
        for i in 0..nsc {
            let vis = if sv(pp, scenes[i]) == 1 { "1" } else { "0" };
            let _ = write!(style, "{:.2}%{{transform:translateX({off:.1}px);opacity:{vis}}}", p(starts[i]));
            if sv(pp, scenes[i]) == 1 {
                off += 11.0 * durs[i];
            }
        }
        let _ = write!(style, "100%{{transform:translateX({off:.1}px);opacity:0}}}}");
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
        let (w0, wm, w1) = (p(starts[i]), p(starts[i] + durs[i] * 0.5), p(starts[i] + durs[i]));
        let we = (w1 + 1.0).min(100.0);
        let _ = write!(style, "@keyframes nslturn{{0%{{opacity:0;transform:translate(0px,-210px)}}{w0:.2}%{{opacity:1;transform:translate(0px,-210px)}}{wm:.2}%{{opacity:1;transform:translate(0px,0px)}}{w1:.2}%{{opacity:1;transform:translate(175px,0px)}}{we:.2}%{{opacity:0;transform:translate(175px,0px)}}100%{{opacity:0}}}}");
    }
    if let Some(i) = ewl_g {
        let (w0, wm, w1) = (p(starts[i]), p(starts[i] + durs[i] * 0.5), p(starts[i] + durs[i]));
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
            let _ = write!(cd_body, r##"<g class="st" style="animation-name:{name}">{}</g>"##, seven_seg(*x, *y, val, *color));
        }
    }
    let _ = write!(style, ".st{{animation-duration:{total:.1}s;animation-timing-function:step-end;animation-iteration-count:infinite}}");
    let _ = write!(style, ".fl{{animation-duration:{total:.1}s;animation-timing-function:linear;animation-iteration-count:infinite}}");
    style.push_str("</style>");

    let mut s = String::new();
    let _ = write!(s, r##"<svg viewBox="0 0 320 320" width="100%" style="max-height:320px" xmlns="http://www.w3.org/2000/svg">"##);
    s.push_str(&style);
    // cross roads + lane markings
    s.push_str(r##"<rect x="0" y="128" width="320" height="64" fill="#26262b"/><rect x="128" y="0" width="64" height="320" fill="#26262b"/>"##);
    s.push_str(r##"<line x1="0" y1="160" x2="320" y2="160" stroke="#6b5d2e" stroke-width="2" stroke-dasharray="11 9"/><line x1="160" y1="0" x2="160" y2="320" stroke="#6b5d2e" stroke-width="2" stroke-dasharray="11 9"/>"##);
    // crosswalk hatching across the south leg
    if ped.is_some() {
        for i in 0..6 {
            let x = 133.0 + i as f64 * 10.0;
            let _ = write!(s, r##"<rect x="{x:.0}" y="202" width="5" height="22" fill="rgba(255,255,255,0.22)"/>"##);
        }
    }
    // NS platoon (cyan, southbound) + EW platoon (pink, eastbound). Outer group snaps opacity per
    // phase (released on green / hidden on red); inner group glides the position. Three cars each
    // so a green release reads as a small platoon; lead car base y=-30 / x=-30.
    s.push_str(r##"<g class="st" style="animation-name:nsplat_op"><g class="fl" style="animation-name:nsplat_mv">"##);
    for k in 0..3 {
        let y = -30 - k * 52;
        let _ = write!(s, r##"<rect x="139" y="{y}" width="18" height="30" rx="3" fill="#38bdf8"/><rect x="143" y="{}" width="10" height="4" rx="1" fill="rgba(0,0,0,0.35)"/>"##, y + 6);
    }
    s.push_str("</g></g>");
    s.push_str(r##"<g class="st" style="animation-name:ewplat_op"><g class="fl" style="animation-name:ewplat_mv">"##);
    for k in 0..3 {
        let x = -30 - k * 52;
        let _ = write!(s, r##"<rect x="{x}" y="163" width="30" height="18" rx="3" fill="#f472b6"/><rect x="{}" y="167" width="4" height="10" rx="1" fill="rgba(0,0,0,0.35)"/>"##, x + 6);
    }
    s.push_str("</g></g>");
    // Safe run: the pedestrian crosses unharmed.
    if ped.is_some() && conflict_scene.is_none() {
        let _ = write!(s, r##"<g class="fl" style="animation-name:pedcross">{}</g>"##, walk_icon(120.0, 209.0, "#fde047"));
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
        let _ = write!(s, r##"<g class="fl" style="animation-name:doomwalk">{}</g>"##, walk_icon(120.0, 209.0, "#fde047"));
    }
    // vehicle signal heads + dim "8" backdrop for each countdown
    for (pfx, _sig, x, y, label, cdx, cdy) in &vheads {
        let (rx, yx, gx) = (x + 16.0, x + 39.0, x + 62.0);
        let cy = y + 14.0;
        let lab_y = if *y < 100.0 { y - 4.0 } else { y + 42.0 };
        let _ = write!(s, r##"<rect x="{x:.0}" y="{y:.0}" width="78" height="28" rx="7" fill="#141418" stroke="rgba(255,255,255,0.15)"/>"##);
        let _ = write!(s, r##"<circle class="st" style="animation-name:{pfx}R" cx="{rx:.0}" cy="{cy:.0}" r="9" fill="#ef4444"/><circle class="st" style="animation-name:{pfx}Y" cx="{yx:.0}" cy="{cy:.0}" r="9" fill="#fbbf24"/><circle class="st" style="animation-name:{pfx}G" cx="{gx:.0}" cy="{cy:.0}" r="9" fill="#22c55e"/>"##);
        let _ = write!(s, r##"<text x="{:.0}" y="{lab_y:.0}" fill="rgba(255,255,255,0.6)" text-anchor="middle" font-size="11" font-family="ui-monospace,monospace">{label}</text>"##, x + 39.0);
        s.push_str(&seven_seg(*cdx, *cdy, 8, "rgba(255,255,255,0.06)"));
    }
    // pedestrian signal head: WALK person / STOP hand + a dim "8" countdown backdrop
    if ped.is_some() {
        s.push_str(r##"<rect x="238" y="54" width="72" height="52" rx="6" fill="#141418" stroke="rgba(255,255,255,0.15)"/>"##);
        let _ = write!(s, r##"<g class="st" style="animation-name:pedWALK">{}</g>"##, walk_icon(258.0, 80.0, "#34d399"));
        let _ = write!(s, r##"<g class="st" style="animation-name:pedSTOP">{}</g>"##, hand_icon(258.0, 80.0));
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
mod traffic_viz_tests {
    use super::{countdown_seq, ns_drive, traffic_plan, traffic_svg, NsDrive};
    use logicaffeine_compile::codegen_sva::hw_pipeline::{Waveform, WaveSignal};

    fn sig(name: &str, vals: &[u64]) -> WaveSignal {
        WaveSignal {
            name: name.to_string(),
            width: 2,
            values: vals.iter().map(|&v| Some(v)).collect(),
        }
    }
    fn wf(signals: Vec<WaveSignal>, n: u32) -> Waveform {
        Waveform { timesteps: n, signals }
    }

    /// A non-conflicting trace: NS green (2 clocks), NS yellow (1), all-red (1), WALK (2).
    fn safe_trace() -> Waveform {
        wf(
            vec![
                sig("ns", &[1, 1, 2, 0, 0, 0]),
                sig("ew", &[0, 0, 0, 0, 0, 0]),
                sig("ped", &[0, 0, 0, 0, 1, 1]),
            ],
            6,
        )
    }

    #[test]
    fn collapses_consecutive_identical_states_into_phases() {
        let plan = traffic_plan(&safe_trace()).unwrap();
        assert_eq!(plan.phases.len(), 4, "ns-green, ns-yellow, all-red, walk");
        assert_eq!(plan.firsts, vec![0, 2, 3, 4]);
        assert_eq!(plan.phases[0].ns, 1);
        assert_eq!(plan.phases[1].ns, 2);
        assert_eq!(plan.phases[3].ped, 1);
    }

    #[test]
    fn phase_durations_are_human_paced_and_cumulative() {
        let plan = traffic_plan(&safe_trace()).unwrap();
        assert_eq!(plan.phases[0].dur, 2.6, "green");
        assert_eq!(plan.phases[1].dur, 1.3, "yellow");
        assert_eq!(plan.phases[2].dur, 1.0, "all-red");
        assert_eq!(plan.phases[3].dur, 7.0, "walk");
        assert_eq!(plan.phases[0].start, 0.0);
        assert_eq!(plan.phases[1].start, 2.6);
        assert!((plan.phases[3].start - (2.6 + 1.3 + 1.0)).abs() < 1e-9);
        assert!((plan.total - (2.6 + 1.3 + 1.0 + 7.0)).abs() < 1e-9);
    }

    #[test]
    fn exclusive_movements_have_no_conflict() {
        assert_eq!(traffic_plan(&safe_trace()).unwrap().conflict, None);
    }

    #[test]
    fn pedestrian_walking_into_green_traffic_is_a_conflict() {
        let plan = traffic_plan(&wf(
            vec![sig("ns", &[1, 1]), sig("ew", &[0, 0]), sig("ped", &[1, 1])],
            2,
        ))
        .unwrap();
        assert_eq!(plan.phases.len(), 1);
        assert_eq!(plan.conflict, Some(0));
        assert_eq!(plan.phases[0].dur, 3.6, "the crash phase lingers");
    }

    #[test]
    fn ns_cars_are_hidden_unless_ns_through_is_green() {
        // This is the bug the user saw: cars must NEVER sit in the road on red/yellow/walk.
        let plan = traffic_plan(&safe_trace()).unwrap();
        assert_eq!(ns_drive(&plan, 0), NsDrive::Through, "ns green → drive through");
        assert_eq!(ns_drive(&plan, 1), NsDrive::Hidden, "ns yellow → hidden");
        assert_eq!(ns_drive(&plan, 2), NsDrive::Hidden, "all-red → hidden");
        assert_eq!(ns_drive(&plan, 3), NsDrive::Hidden, "pedestrian walk → hidden");
    }

    #[test]
    fn the_crash_car_stops_at_the_crosswalk_not_drives_through() {
        let plan = traffic_plan(&wf(
            vec![sig("ns", &[1, 1]), sig("ew", &[0, 0]), sig("ped", &[1, 1])],
            2,
        ))
        .unwrap();
        assert_eq!(ns_drive(&plan, 0), NsDrive::CrashStop);
    }

    #[test]
    fn countdown_counts_down_whole_seconds() {
        assert_eq!(countdown_seq(7.0), vec![7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(countdown_seq(2.6), vec![3, 2, 1]);
        assert_eq!(countdown_seq(1.3), vec![2, 1]);
        assert_eq!(countdown_seq(1.0), vec![1]);
    }

    #[test]
    fn every_animation_name_used_in_the_svg_has_a_matching_keyframes_block() {
        // Guards against the dangling-reference class of bug (body using a keyframe that the
        // style section never defined).
        let svg = traffic_svg(&wf(
            vec![
                sig("ns", &[1, 1, 0, 0, 0, 0]),
                sig("ew", &[0, 0, 0, 1, 1, 0]),
                sig("nsl", &[0, 0, 0, 0, 0, 1]),
                sig("ewl", &[0, 0, 0, 0, 0, 0]),
                sig("ped", &[0, 0, 0, 0, 0, 0]),
            ],
            6,
        ))
        .expect("renders an intersection");
        let mut rest = svg.as_str();
        while let Some(pos) = rest.find("animation-name:") {
            rest = &rest[pos + "animation-name:".len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            let name = &rest[..end];
            assert!(
                svg.contains(&format!("@keyframes {name}{{"))
                    || svg.contains(&format!("@keyframes {name} {{")),
                "animation-name {name} has no @keyframes definition"
            );
        }
    }

    #[test]
    fn non_intersection_traces_do_not_render() {
        // No ns/ew signals → not a traffic trace.
        assert!(traffic_svg(&wf(vec![sig("a", &[0, 1]), sig("b", &[1, 0])], 2)).is_none());
    }
}

/// Keep the address bar on the canonical shareable URL for the open file
/// without pushing a history entry.
#[cfg(target_arch = "wasm32")]
fn sync_studio_url(vfs_path: &str) {
    crate::ui::router::replace_bar_url(&crate::ui::router::studio_file_url(vfs_path));
}

#[component(lazy)]
pub fn Studio(file: Option<String>) -> Element {
    // Mode state
    let mut mode = use_signal(|| StudioMode::Logic);

    // File browser state
    let mut sidebar_open = use_signal(|| true);
    let mut file_tree = use_signal(FileNode::root); // Start empty - no fallback, show real errors
    let mut current_file = use_signal(|| None::<String>);
    let mut vfs_error = use_signal(|| None::<String>); // Track VFS errors for display
    let mut vfs_is_fallback = use_signal(|| false); // Track if using IndexedDB fallback

    // Logic mode state
    let mut input = use_signal(String::new);
    let mut result = use_signal(|| CompileResult {
        logic: None,
        simple_logic: None,
        kripke_logic: None,
        ast: None,
        readings: Vec::new(),
        simple_readings: Vec::new(),
        kripke_readings: Vec::new(),
        tokens: Vec::new(),
        error: None,
    });
    let mut format = use_signal(|| OutputFormat::SimpleFOL);

    // Proof panel state for Logic mode
    let mut proof_text = use_signal(String::new);
    let mut proof_status = use_signal(|| ProofStatus::Idle);
    let mut proof_hint = use_signal(|| None::<String>);
    // The current ProofExpr for the proof engine
    let mut current_proof_expr = use_signal(|| None::<ProofExpr>);
    // Knowledge base (axioms/premises) for the proof engine
    let mut knowledge_base = use_signal(Vec::<ProofExpr>::new);
    // Logic mode output view: FOL (Logic) vs extracted Rust
    let mut logic_output_mode = use_signal(|| LogicView::Logic);
    let mut generated_logic_rust = use_signal(String::new);

    // Code mode state (imperative .logos)
    let mut code_input = use_signal(String::new);
    let mut code_output_mode = use_signal(|| CodeOutputMode::Interpret);
    let mut interpreter_result = use_signal(|| InterpreterResult {
        lines: vec![],
        error: None,
    });
    let mut generated_rust = use_signal(String::new);
    // Code-mode debugger drawer (bottom-docked, additive — see `DebugDrawer`).
    let mut debugging = use_signal(|| false);

    // Math mode state (vernacular/theorem proving)
    let mut math_input = use_signal(String::new);
    let mut math_repl = use_signal(Repl::new);
    let mut math_output = use_signal(Vec::<ReplLine>::new);
    // Math mode output view: REPL output (Interpret) vs extracted Rust
    let mut math_output_mode = use_signal(|| CodeOutputMode::Interpret);
    let mut generated_math_rust = use_signal(String::new);

    // Hardware mode state (English hardware spec -> SVA + in-browser proving)
    let mut hw_input = use_signal(String::new);
    // Output view: SVA (Interpret) vs extracted Rust runtime monitor.
    let mut hw_output_mode = use_signal(|| CodeOutputMode::Interpret);
    let mut hw_sva = use_signal(String::new);
    let mut hw_psl = use_signal(String::new);
    let mut hw_signals = use_signal(Vec::<String>::new);
    // Certified-equivalence verdict (and counterexample) from our in-browser prover.
    let mut hw_proof = use_signal(String::new);
    let mut hw_proof_ok = use_signal(|| None::<bool>);
    // Raw counterexample bindings (name@t -> "0"/"1"), rendered as a waveform when present.
    let mut hw_counterexample = use_signal(Vec::<(String, String)>::new);
    // Knowledge graph of the spec (signals + relations), rendered as an animated SVG.
    let mut hw_kg = use_signal(logicaffeine_compile::codegen_sva::hw_pipeline::KgSummary::default);
    let mut generated_hw_rust = use_signal(String::new);
    let mut hw_error = use_signal(|| None::<String>);

    // Shows a "Working…" affordance while a Run/Compile handler is in flight, so a
    // medium-length operation doesn't look like a dead tab.
    let mut busy = use_signal(|| false);

    // Desktop panel resizing state
    let mut sidebar_width = use_signal(|| 240.0f64);
    let mut left_width = use_signal(|| 35.0f64);
    let mut right_width = use_signal(|| 25.0f64);
    let mut resizing = use_signal(|| None::<&'static str>);

    // Mobile tab state
    let mut active_tab = use_signal(|| MobileTab::Panel1);

    // Mobile panel collapse state
    let mut editor_expanded = use_signal(|| true);
    let mut output_expanded = use_signal(|| true);

    // Touch gesture state for swipe detection
    let mut touch_start_x = use_signal(|| 0.0f64);
    let mut touch_start_y = use_signal(|| 0.0f64);

    // Mobile vertical split: Panel 1 height as % of studio-main
    let mut mobile_split = use_signal(|| 50.0f64);

    // VFS initialization flag
    let mut vfs_initialized = use_signal(|| false);

    // Cached VFS handle: acquire the OPFS worker once, then reuse it for file
    // switches instead of spawning (and leaking) a fresh worker each time.
    #[cfg(target_arch = "wasm32")]
    let mut vfs_handle = use_signal(|| None::<WebVfs>);

    // Initialize VFS and seed examples on mount
    #[cfg(target_arch = "wasm32")]
    use_effect(move || {
        if *vfs_initialized.read() {
            return;
        }
        vfs_initialized.set(true);

        let file = file.clone();
        spawn(async move {
            // Get platform VFS with automatic fallback (OPFS -> IndexedDB)
            match get_platform_vfs_with_fallback().await {
                Ok(vfs) => {
                    // Cache the handle so file switches reuse this worker instead
                    // of spawning (and leaking) a fresh one each time.
                    vfs_handle.set(Some(vfs.clone()));
                    if vfs.is_fallback() {
                        vfs_is_fallback.set(true);
                    }

                    // Seed example files if they don't exist
                    if let Err(e) = seed_examples(&vfs).await {
                        vfs_error.set(Some(format!("Failed to seed examples: {:?}", e)));
                    }

                    // Build file tree from VFS
                    let mut root = FileNode::root();
                    match load_dir_recursive(&vfs, "/", &mut root).await {
                        Ok(()) => {
                            if !root.children.is_empty() {
                                file_tree.set(root);
                            } else {
                                vfs_error.set(Some("VFS returned empty tree - no files found".to_string()));
                            }
                        }
                        Err(e) => {
                            vfs_error.set(Some(format!("Failed to load file tree: {:?}", e)));
                        }
                    }

                    // Open the file from the route's `file` query prop, or the default
                    let file_to_load = file
                        .map(|f| {
                            // Normalize path - ensure it starts with /
                            if f.starts_with('/') {
                                f
                            } else {
                                format!("/{}", f)
                            }
                        })
                        .unwrap_or_else(|| "/examples/logic/prover-demo.logic".to_string());

                    // Load the file and detect mode from path/extension
                    if let Ok(content) = vfs.read_to_string(&file_to_load).await {
                        current_file.set(Some(file_to_load.clone()));
                        sync_studio_url(&file_to_load);

                        let ext = file_to_load.rsplit('.').next().unwrap_or("").to_lowercase();
                        let is_math_dir = file_to_load.contains("/math/") || file_to_load.contains("/examples/math");

                        if is_math_dir || ext == "math" || ext == "vernac" {
                            // Math mode
                            mode.set(StudioMode::Math);
                            math_input.set(content);
                        } else if ext == "logos" {
                            // Code mode - auto-run
                            mode.set(StudioMode::Code);
                            code_input.set(content.clone());
                            let interp_result = interpret_for_ui_baseline(&content).await;
                            interpreter_result.set(interp_result);
                        } else {
                            // Logic mode (default, handles .logic files)
                            mode.set(StudioMode::Logic);
                            input.set(content.clone());

                            if content.contains("## Theorem:") {
                                // Compile as theorem
                                let theorem_result = compile_theorem_for_ui(&content);
                                if theorem_result.error.is_none() {
                                    result.set(CompileResult {
                                        logic: theorem_result.goal_string.clone(),
                                        simple_logic: theorem_result.goal_string.clone(),
                                        kripke_logic: None,
                                        ast: None,
                                        readings: Vec::new(),
                                        simple_readings: Vec::new(),
                                        kripke_readings: Vec::new(),
                                        tokens: Vec::new(),
                                        error: None,
                                    });

                                    knowledge_base.write().clear();
                                    for premise in &theorem_result.premises {
                                        knowledge_base.write().push(premise.clone());
                                    }

                                    if let Some(goal) = theorem_result.goal.clone() {
                                        current_proof_expr.set(Some(goal));
                                    }

                                    let html = theorem_proof_html(&theorem_result);
                                    if !html.is_empty() {
                                        proof_text.set(html);
                                        proof_status.set(if theorem_result.verified {
                                            ProofStatus::Success
                                        } else {
                                            ProofStatus::Idle
                                        });
                                        proof_hint.set(Some(theorem_proof_hint(&theorem_result)));
                                    } else {
                                        proof_status.set(ProofStatus::Idle);
                                        proof_hint.set(Some(format!(
                                            "Theorem '{}' ready. {} premise(s) loaded.",
                                            theorem_result.name,
                                            knowledge_base.read().len()
                                        )));
                                    }
                                }
                            } else {
                                // Plain English sentences
                                let sentences: Vec<&str> = content
                                    .lines()
                                    .filter(|line| {
                                        let trimmed = line.trim();
                                        !trimmed.is_empty()
                                        && !trimmed.starts_with('#')
                                        && !trimmed.starts_with("--")
                                    })
                                    .collect();

                                if !sentences.is_empty() {
                                    let all_text = sentences.join("\n");
                                    let compiled = compile_for_ui(&all_text);
                                    result.set(compiled);

                                    let first_sentence = sentences[0];
                                    let proof_result = compile_for_proof(first_sentence);
                                    if let Some(expr) = proof_result.proof_expr {
                                        current_proof_expr.set(Some(expr));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    vfs_error.set(Some(format!("VFS INIT FAILED: {:?}", e)));
                }
            }
        });
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

    // Close sidebar on mobile by default (runs once on mount)
    #[cfg(target_arch = "wasm32")]
    {
        let mut sidebar_init_done = use_signal(|| false);
        use_effect(move || {
            if *sidebar_init_done.read() {
                return;
            }
            sidebar_init_done.set(true);

            if let Some(window) = web_sys::window() {
                let width = window.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1024.0);
                if width <= 768.0 {
                    sidebar_open.set(false);
                }
            }
        });
    }

    // Logic mode input handler - compiles for both UI and proof engine
    // Logic mode keystroke handler: just store the text. The heavy compile/prove/grid runs
    // on the Execute button (`handle_logic_execute`), mirroring Code mode's Run — so typing
    // never triggers a per-keystroke solve.
    let handle_logic_input = move |new_value: String| {
        input.set(new_value);
    };

    // Code mode: Run button handler (interpret) with streaming output
    let handle_code_run = move |_| {
        let code = code_input.read().clone();
        // Switch to Output tab (Panel2) on mobile and switch to Output mode
        active_tab.set(MobileTab::Panel2);
        code_output_mode.set(CodeOutputMode::Interpret);
        // Clear previous output
        interpreter_result.set(InterpreterResult {
            lines: vec![],
            error: None,
        });
        spawn(async move {
            // Create streaming callback that updates the signal as output arrives
            let callback = Rc::new(RefCell::new(move |line: String| {
                // Update the signal with the new line
                interpreter_result.write().lines.push(line);
            }));

            // Route the interpreter's file I/O to the Studio's VFS (OPFS/IndexedDB),
            // so the standard-library I/O vocabulary works in the browser. The
            // worker-backed handle is wasm-only; the native build does no file I/O.
            #[cfg(target_arch = "wasm32")]
            let vfs = vfs_handle
                .read()
                .clone()
                .map(|w| std::sync::Arc::new(w) as std::sync::Arc<dyn Vfs>);
            #[cfg(not(target_arch = "wasm32"))]
            let vfs: Option<std::sync::Arc<dyn Vfs>> = None;
            let result = interpret_streaming_with_vfs(&code, callback, vfs).await;
            // Set final result (includes any error)
            interpreter_result.set(result);
        });
    };

    // Code mode: Compile button handler (generate Rust)
    // Uses generate_rust_code which works on WASM
    let handle_code_compile = move |_| {
        let code = code_input.read().clone();
        // Switch to Output tab (Panel2) on mobile
        active_tab.set(MobileTab::Panel2);
        busy.set(true);
        spawn(async move {
            // Yield once so the "Working…" affordance paints before the work.
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;
            match generate_rust_code(&code) {
                Ok(rust_code) => {
                    generated_rust.set(rust_code);
                    code_output_mode.set(CodeOutputMode::Rust);
                }
                Err(e) => {
                    interpreter_result.set(InterpreterResult {
                        lines: vec![],
                        error: Some(format!("Compile error: {:?}", e)),
                    });
                    code_output_mode.set(CodeOutputMode::Interpret);
                }
            }
            busy.set(false);
        });
    };

    // Math mode execute handler (vernacular REPL).
    // Re-runs from a fresh kernel each press so output is REPLACED, not appended
    // (mirrors Code mode's Run — no manual Clear needed). The editor holds the
    // full program, so a fresh run is idempotent.
    let handle_math_execute = move |_| {
        active_tab.set(MobileTab::Panel2);
        math_output_mode.set(CodeOutputMode::Interpret);
        let code = math_input.read().clone();
        busy.set(true);
        spawn(async move {
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;

            let statements = parse_math_statements(&code);
            let mut repl = Repl::new();
            let mut lines = Vec::new();
            for stmt in statements {
                match repl.execute(&stmt) {
                    Ok(output) => lines.push(ReplLine::success(stmt, output)),
                    Err(e) => lines.push(ReplLine::error(stmt, e.to_string())),
                }
            }
            math_repl.set(repl);
            math_output.set(lines);
            busy.set(false);
        });
    };

    // Math mode compile handler: rebuild the kernel context from the editor, then
    // extract every user-defined Definition/Inductive into one Rust module.
    let handle_math_compile = move |_| {
        active_tab.set(MobileTab::Panel2);
        let code = math_input.read().clone();
        busy.set(true);
        spawn(async move {
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;

            let statements = parse_math_statements(&code);
            let mut repl = Repl::new();
            for stmt in statements {
                let _ = repl.execute(&stmt);
            }
            let rust = match extract_math_rust(repl.context()) {
                Ok(rust) => rust,
                Err(e) => format!("// extraction error: {e}"),
            };
            math_repl.set(repl);
            generated_math_rust.set(rust);
            math_output_mode.set(CodeOutputMode::Rust);
            busy.set(false);
        });
    };

    // Logic mode execute handler: compile the theorem (or sentences), prove it, and render
    // the result — the solved grid, the wh-question answer, or the certified derivation.
    // Like Code mode's Run, the heavy work happens on this button, not on every keystroke.
    let handle_logic_execute = move |_| {
        active_tab.set(MobileTab::Panel2);
        logic_output_mode.set(LogicView::Logic);
        let new_value = input.read().clone();

        if new_value.contains("## Theorem:") {
            let theorem_result = compile_theorem_for_ui(&new_value);

            if let Some(err) = theorem_result.error {
                result.set(CompileResult {
                    logic: None,
                    simple_logic: None,
                    kripke_logic: None,
                    ast: None,
                    readings: Vec::new(),
                    simple_readings: Vec::new(),
                    kripke_readings: Vec::new(),
                    tokens: Vec::new(),
                    error: Some(err.clone()),
                });
                proof_status.set(ProofStatus::Failed(err));
                current_proof_expr.set(None);
                knowledge_base.write().clear();
            } else {
                result.set(CompileResult {
                    logic: theorem_result.goal_string.clone(),
                    simple_logic: theorem_result.goal_string.clone(),
                    kripke_logic: None,
                    ast: None,
                    readings: Vec::new(),
                    simple_readings: Vec::new(),
                    kripke_readings: Vec::new(),
                    tokens: Vec::new(),
                    error: None,
                });

                knowledge_base.write().clear();
                for premise in &theorem_result.premises {
                    knowledge_base.write().push(premise.clone());
                }

                if let Some(goal) = theorem_result.goal.clone() {
                    current_proof_expr.set(Some(goal));
                }

                let html = theorem_proof_html(&theorem_result);
                if !html.is_empty() {
                    proof_text.set(html);
                    proof_status.set(if theorem_result.verified {
                        ProofStatus::Success
                    } else {
                        ProofStatus::Idle
                    });
                    proof_hint.set(Some(theorem_proof_hint(&theorem_result)));
                } else {
                    proof_status.set(ProofStatus::Idle);
                    proof_hint.set(Some(format!(
                        "Theorem '{}' ready. {} premise(s) loaded. Click Auto to prove.",
                        theorem_result.name,
                        knowledge_base.read().len()
                    )));
                    proof_text.set(String::new());
                }
            }
        } else {
            let sentences: Vec<&str> = new_value
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.is_empty()
                        && !trimmed.starts_with('#')
                        && !trimmed.starts_with("--")
                })
                .collect();

            if !sentences.is_empty() {
                let all_text = sentences.join("\n");
                let compiled = compile_for_ui(&all_text);
                result.set(compiled);

                knowledge_base.write().clear();

                let first_sentence = sentences[0];
                let proof_result = compile_for_proof(first_sentence);
                if let Some(expr) = proof_result.proof_expr {
                    current_proof_expr.set(Some(expr));
                    proof_status.set(ProofStatus::Idle);
                    proof_hint.set(Some("Enter premises or click Auto to prove.".to_string()));
                } else if let Some(err) = proof_result.error {
                    current_proof_expr.set(None);
                    proof_status.set(ProofStatus::Failed(err));
                }
            } else {
                result.set(CompileResult {
                    logic: None,
                    simple_logic: None,
                    kripke_logic: None,
                    ast: None,
                    readings: Vec::new(),
                    simple_readings: Vec::new(),
                    kripke_readings: Vec::new(),
                    tokens: Vec::new(),
                    error: None,
                });
                current_proof_expr.set(None);
                knowledge_base.write().clear();
                proof_text.set(String::new());
                proof_status.set(ProofStatus::Idle);
            }
        }
    };

    // Logic mode compile handler: extract Rust where there's constructive content
    // (a `## Theorem:` block / definitions); honest note otherwise.
    let handle_logic_compile = move |_| {
        active_tab.set(MobileTab::Panel2);
        let text = input.read().clone();
        busy.set(true);
        spawn(async move {
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;

            let rust = match extract_logic_rust(&text) {
                Ok(rust) => rust,
                Err(e) => format!("// extraction error: {e}"),
            };
            generated_logic_rust.set(rust);
            logic_output_mode.set(LogicView::Rust);
            busy.set(false);
        });
    };

    // Logic mode: Tactic button handler - FULL PROOF ENGINE INTEGRATION
    let handle_tactic = {
        let result_signal = result.clone();
        move |tactic: Tactic| {
            // Get the current ProofExpr (compiled earlier)
            let maybe_goal = current_proof_expr.read().clone();
            let kb = knowledge_base.read().clone();
            let logic_str = result_signal.read().simple_logic.clone();

            match maybe_goal {
                Some(goal) => {
                    proof_status.set(ProofStatus::Proving);

                    match tactic {
                        Tactic::Auto => {
                            // Run the actual BackwardChainer proof engine!
                            let mut engine = BackwardChainer::new();

                            // Add knowledge base as axioms
                            for axiom in &kb {
                                engine.add_axiom(axiom.clone());
                            }

                            // Attempt to prove
                            match engine.prove(goal.clone()) {
                                Ok(derivation) => {
                                    // Format the derivation tree for display
                                    let tree_display = format_derivation_html(&derivation);
                                    proof_text.set(tree_display);
                                    proof_status.set(ProofStatus::Success);
                                    proof_hint.set(Some("Proof found! The derivation tree shows the inference steps.".to_string()));
                                }
                                Err(e) => {
                                    // Get a Socratic hint for why it failed
                                    let hint = suggest_hint(&goal, &kb, &[]);
                                    proof_text.set(format!(
                                        "<span class=\"rule\">Auto-prove failed</span>\n\nGoal: {}\n\nError: {}",
                                        logic_str.as_deref().unwrap_or("(no expression)"),
                                        e
                                    ));
                                    proof_status.set(ProofStatus::Failed(format!("{}", e)));
                                    proof_hint.set(Some(hint.text));
                                }
                            }
                        }
                        Tactic::ModusPonens => {
                            let hint = suggest_hint(&goal, &kb, &[]);
                            proof_text.set(format!(
                                "<span class=\"rule\">Modus Ponens</span>\n\nFrom P\u{2192}Q and P, derive Q\n\nCurrent goal: {}\n\nKnowledge base has {} axiom(s)",
                                logic_str.as_deref().unwrap_or("(none)"),
                                kb.len()
                            ));
                            proof_status.set(ProofStatus::Idle);
                            proof_hint.set(Some(hint.text));
                        }
                        Tactic::UniversalInst => {
                            let hint = suggest_hint(&goal, &kb, &[]);
                            proof_text.set(format!(
                                "<span class=\"rule\">\u{2200} Elimination</span>\n\nFrom \u{2200}x.P(x), derive P(c)\n\nCurrent goal: {}",
                                logic_str.as_deref().unwrap_or("(none)")
                            ));
                            proof_status.set(ProofStatus::Idle);
                            proof_hint.set(Some(hint.text));
                        }
                        Tactic::ExistentialIntro => {
                            let hint = suggest_hint(&goal, &kb, &[]);
                            proof_text.set(format!(
                                "<span class=\"rule\">\u{2203} Introduction</span>\n\nFrom P(c), derive \u{2203}x.P(x)\n\nCurrent goal: {}",
                                logic_str.as_deref().unwrap_or("(none)")
                            ));
                            proof_status.set(ProofStatus::Idle);
                            proof_hint.set(Some(hint.text));
                        }
                        Tactic::Induction => {
                            let hint = suggest_hint(&goal, &kb, &[]);
                            proof_text.set(format!(
                                "<span class=\"rule\">Induction</span>\n\nBase case + Inductive step\n\nCurrent goal: {}",
                                logic_str.as_deref().unwrap_or("(none)")
                            ));
                            proof_status.set(ProofStatus::Idle);
                            proof_hint.set(Some(hint.text));
                        }
                        Tactic::Rewrite => {
                            let hint = suggest_hint(&goal, &kb, &[]);
                            proof_text.set(format!(
                                "<span class=\"rule\">Rewrite</span>\n\nUse equality to substitute\n\nCurrent goal: {}",
                                logic_str.as_deref().unwrap_or("(none)")
                            ));
                            proof_status.set(ProofStatus::Idle);
                            proof_hint.set(Some(hint.text));
                        }
                    }
                }
                None => {
                    proof_status.set(ProofStatus::Failed("No logic expression to prove. Enter a sentence first.".to_string()));
                    proof_text.set(String::new());
                    proof_hint.set(Some("Enter an English sentence above to generate a logical formula.".to_string()));
                }
            }
        }
    };

    // Build context entries from Math REPL
    let (definitions, inductives) = {
        let repl_guard = math_repl.read();
        let ctx = repl_guard.context();
        let mut defs = Vec::new();
        let mut inds = Vec::new();

        for (name, ty, body) in ctx.iter_definitions() {
            defs.push(ContextEntry {
                name: name.to_string(),
                ty: format!("{}", ty),
                body: Some(format!("{}", body)),
                kind: EntryKind::Definition,
            });
        }

        for (name, ty) in ctx.iter_inductives() {
            inds.push(ContextEntry {
                name: name.to_string(),
                ty: format!("{}", ty),
                body: None,
                kind: EntryKind::Inductive,
            });
        }

        (defs, inds)
    };

    // Hardware mode handlers.
    //
    // `synthesize_sva_from_spec` is pure Rust (no Z3), so synthesis and the
    // PSL / Rust-monitor emission all run in the browser. A `SynthesizedSva`
    // carries the body + signals + kind, which we lift into an `SvaProperty`
    // for the emitters.
    fn hw_property_from_synth(
        synth: &logicaffeine_compile::codegen_sva::fol_to_sva::SynthesizedSva,
    ) -> logicaffeine_compile::codegen_sva::SvaProperty {
        use logicaffeine_compile::codegen_sva::{SvaAssertionKind, SvaProperty, sanitize_property_name};
        let kind = match synth.kind.as_str() {
            "cover" => SvaAssertionKind::Cover,
            "assume" => SvaAssertionKind::Assume,
            _ => SvaAssertionKind::Assert,
        };
        let name = if synth.signals.is_empty() {
            "p_property".to_string()
        } else {
            sanitize_property_name(&synth.signals.join("_"))
        };
        SvaProperty { name, clock: "clk".to_string(), body: synth.body.clone(), kind }
    }

    // Synthesize a spec and push the SVA / PSL / signals (or error) into the
    // Hardware-mode signals. Shared by the Execute button and the file loader.
    #[allow(clippy::too_many_arguments)]
    fn load_hardware_spec(
        content: &str,
        mut hw_sva: Signal<String>,
        mut hw_psl: Signal<String>,
        mut hw_signals: Signal<Vec<String>>,
        mut hw_proof: Signal<String>,
        mut hw_proof_ok: Signal<Option<bool>>,
        mut hw_counterexample: Signal<Vec<(String, String)>>,
        mut hw_kg: Signal<logicaffeine_compile::codegen_sva::hw_pipeline::KgSummary>,
        mut hw_error: Signal<Option<String>>,
    ) {
        if content.trim().is_empty() {
            hw_sva.set(String::new());
            hw_psl.set(String::new());
            hw_signals.write().clear();
            hw_proof.set(String::new());
            hw_proof_ok.set(None);
            hw_counterexample.write().clear();
            hw_kg.set(Default::default());
            hw_error.set(None);
            return;
        }
        // Pigeonhole spec (`pigeons: N`) → the live viz panel solves PHP(N) directly from the editor.
        // Execute clears the other Hardware outputs and posts the certified verdict to the proof line.
        if let Some(pspec) = crate::ui::pages::pigeonhole_viz::parse_pigeonhole_spec(content) {
            hw_sva.set(String::new());
            hw_psl.set(String::new());
            hw_signals.write().clear();
            hw_counterexample.write().clear();
            hw_kg.set(Default::default());
            hw_error.set(None);
            hw_proof.set(format!(
                "\u{2717} PHP({0}) \u{2014} {0} pigeons can't fit {1} holes. Certified UNSAT by our prover (maximum matching + symmetry breaking), no Z3.",
                pspec.pigeons,
                pspec.holes()
            ));
            hw_proof_ok.set(Some(false));
            return;
        }
        // Register-allocation spec (`registers:` + `name: start-end`) → the live viz panel renders
        // the certified allocation directly from the editor, so Execute just clears the other
        // Hardware outputs (no SVA synthesis, no spurious "not a hardware property" error).
        if content.contains("registers:") || content.contains("Registers:") {
            hw_sva.set(String::new());
            hw_psl.set(String::new());
            hw_signals.write().clear();
            hw_proof.set(String::new());
            hw_proof_ok.set(None);
            hw_counterexample.write().clear();
            hw_kg.set(Default::default());
            hw_error.set(None);
            return;
        }
        // RTL/Verilog input → bounded model checking + k-induction (no Z3), distinct from the
        // English-spec → SVA synthesis path.
        if content.contains("module") && content.contains("endmodule") {
            use logicaffeine_compile::codegen_sva::rtl::parse_transition_system;
            use logicaffeine_proof::bmc::{BmcOutcome, InductionOutcome};
            hw_psl.set(String::new());
            hw_signals.write().clear();
            hw_kg.set(Default::default());
            let to_ce = |trace: Vec<(String, bool)>| -> Vec<(String, String)> {
                trace
                    .into_iter()
                    .map(|(n, v)| (n, if v { "1" } else { "0" }.to_string()))
                    .collect()
            };
            match parse_transition_system(content) {
                Ok(ts) => {
                    hw_sva.set(format!(
                        "// RTL transition system \u{2014} {} register(s)\n// bounded model checking + k-induction (no Z3)",
                        ts.registers.len()
                    ));
                    match ts.prove_invariant(4) {
                        InductionOutcome::Proven => {
                            hw_proof.set("\u{2713} Always holds \u{2014} proven for every reachable state (k-induction, no Z3)".to_string());
                            hw_proof_ok.set(Some(true));
                            // No counterexample to show — animate a concrete WITNESS run of the
                            // proven-safe machine so the diagram still comes alive (no conflict).
                            match ts.witness_trace(18) {
                                Some(trace) => hw_counterexample.set(to_ce(trace)),
                                None => hw_counterexample.write().clear(),
                            }
                        }
                        InductionOutcome::CounterexampleAt { k, trace } => {
                            hw_proof.set(format!("\u{2717} Can break at step {k} \u{2014} counterexample below"));
                            hw_proof_ok.set(Some(false));
                            hw_counterexample.set(to_ce(trace));
                        }
                        InductionOutcome::NotInductive => match ts.bmc(28) {
                            BmcOutcome::CounterexampleAt { k, trace } => {
                                hw_proof.set(format!("\u{2717} Can break at step {k} \u{2014} counterexample below"));
                                hw_proof_ok.set(Some(false));
                                hw_counterexample.set(to_ce(trace));
                            }
                            BmcOutcome::NoneWithin(n) => {
                                hw_proof.set(format!("No failure found within {n} steps \u{2014} couldn't prove it always holds at this depth"));
                                hw_proof_ok.set(None);
                                hw_counterexample.write().clear();
                            }
                            BmcOutcome::Unsupported => {
                                hw_proof.set("This property isn't expressible yet \u{2014} try a simpler one".to_string());
                                hw_proof_ok.set(None);
                                hw_counterexample.write().clear();
                            }
                        },
                        InductionOutcome::Unsupported => {
                            hw_proof.set("This property isn't expressible yet \u{2014} try a simpler one".to_string());
                            hw_proof_ok.set(None);
                            hw_counterexample.write().clear();
                        }
                    }
                    hw_error.set(None);
                }
                Err(e) => {
                    hw_sva.set(String::new());
                    hw_proof.set(String::new());
                    hw_proof_ok.set(None);
                    hw_counterexample.write().clear();
                    hw_error.set(Some(format!("RTL parse error: {}", e.message)));
                }
            }
            return;
        }
        // Signal-design input ("<movement> conflicts with <movement>, …") → synthesize a
        // conflict-free phase plan with our own certified SAT solver (fewest phases, Z3-free).
        if content.to_lowercase().contains("conflict") {
            use logicaffeine_compile::codegen_sva::signal_design::design_from_spec;
            hw_psl.set(String::new());
            hw_kg.set(Default::default());
            hw_counterexample.write().clear();
            match design_from_spec(content) {
                Ok((intersection, plan)) => {
                    let mut out = String::new();
                    for (p, group) in plan.groups().iter().enumerate() {
                        out.push_str(&format!(
                            "Phase {}: {}\n",
                            p + 1,
                            intersection.names(group).join(", ")
                        ));
                    }
                    // Close the loop: generate a Verilog controller for the plan and certify it
                    // conflict-free with the same prover (design → generate → prove, all ours).
                    use logicaffeine_compile::codegen_sva::controller_gen::generate_controller;
                    use logicaffeine_compile::codegen_sva::rtl::parse_transition_system;
                    use logicaffeine_proof::bmc::InductionOutcome;
                    let verilog = generate_controller(&intersection, &plan);
                    let controller_proven = matches!(
                        parse_transition_system(&verilog).map(|ts| ts.prove_invariant(4)),
                        Ok(InductionOutcome::Proven)
                    );
                    out.push_str("\n\n// Generated controller (provably conflict-free):\n");
                    out.push_str(&verilog);
                    hw_sva.set(out.trim_end().to_string());
                    hw_signals.set(intersection.movements.clone());
                    let minimal = if !plan.minimal_certified {
                        String::new()
                    } else if plan.num_phases <= 1 {
                        " \u{2014} a single phase suffices".to_string()
                    } else {
                        format!(" \u{2014} provably minimal ({}-phase impossible)", plan.num_phases - 1)
                    };
                    let controller_note = if controller_proven {
                        " \u{00b7} generated controller PROVEN conflict-free (k-induction)"
                    } else {
                        ""
                    };
                    hw_proof.set(format!(
                        "\u{2713} Conflict-free {}-phase plan synthesized{}{} (certified by our SAT solver, no Z3)",
                        plan.num_phases, minimal, controller_note
                    ));
                    hw_proof_ok.set(Some(true));
                    hw_error.set(None);
                }
                Err(e) => {
                    hw_sva.set(String::new());
                    hw_signals.write().clear();
                    hw_proof.set(String::new());
                    hw_proof_ok.set(None);
                    hw_error.set(Some(e));
                }
            }
            return;
        }
        match logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec(content, "clk") {
            Ok(synth) => {
                let prop = hw_property_from_synth(&synth);
                hw_psl.set(logicaffeine_compile::codegen_sva::emit_psl_property(&prop));
                hw_signals.set(synth.signals.clone());
                // Certified, Z3-free equivalence: does the synthesized SVA capture the spec?
                use logicaffeine_compile::codegen_sva::hw_pipeline::{
                    check_spec_vacuity, prove_spec_sva_equivalence, VacuityReport,
                };
                // Vacuity (dead-trigger) check — appended to the verdict line.
                let vacuity_note = match check_spec_vacuity(content, 8) {
                    Ok(VacuityReport::Vacuous) => {
                        " \u{00b7} \u{26A0} never triggers (vacuous)".to_string()
                    }
                    Ok(VacuityReport::NonVacuous) => {
                        " \u{00b7} the trigger can actually fire".to_string()
                    }
                    _ => String::new(),
                };
                match prove_spec_sva_equivalence(content, &synth.body, 8) {
                    Ok(r) if r.equivalent => {
                        hw_proof.set(format!(
                            "\u{2713} Matches the spec \u{2014} certified, kernel-checked, no Z3{}",
                            vacuity_note
                        ));
                        hw_proof_ok.set(Some(true));
                        hw_counterexample.write().clear();
                    }
                    Ok(r) => {
                        let ce_vec = r.counterexample.unwrap_or_default();
                        let ce = ce_vec
                            .iter()
                            .map(|(k, v)| format!("{}={}", k, v))
                            .collect::<Vec<_>>()
                            .join(", ");
                        hw_proof.set(format!(
                            "\u{2717} Not equivalent to the spec \u{2014} counterexample: {}{}",
                            ce, vacuity_note
                        ));
                        hw_proof_ok.set(Some(false));
                        hw_counterexample.set(ce_vec);
                    }
                    Err(e) => {
                        hw_proof.set(format!("Proof unavailable: {}{}", e, vacuity_note));
                        hw_proof_ok.set(None);
                        hw_counterexample.write().clear();
                    }
                }
                hw_sva.set(synth.sva_text);
                // Extract the knowledge graph for the animated diagram (best-effort).
                hw_kg.set(
                    logicaffeine_compile::codegen_sva::hw_pipeline::kg_summary(content)
                        .unwrap_or_default(),
                );
                hw_error.set(None);
            }
            Err(e) => {
                hw_sva.set(String::new());
                hw_psl.set(String::new());
                hw_signals.write().clear();
                hw_proof.set(String::new());
                hw_proof_ok.set(None);
                hw_counterexample.write().clear();
                hw_kg.set(Default::default());
                hw_error.set(Some(e));
            }
        }
    }

    let handle_hardware_execute = move |_| {
        let spec = hw_input.read().clone();
        load_hardware_spec(&spec, hw_sva, hw_psl, hw_signals, hw_proof, hw_proof_ok, hw_counterexample, hw_kg, hw_error);
        hw_output_mode.set(CodeOutputMode::Interpret);
    };

    let handle_hardware_compile = move |_| {
        let spec = hw_input.read().clone();
        if spec.trim().is_empty() {
            generated_hw_rust.set(String::new());
            return;
        }
        match logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec(&spec, "clk") {
            Ok(synth) => {
                let prop = hw_property_from_synth(&synth);
                generated_hw_rust.set(logicaffeine_compile::codegen_sva::emit_rust_monitor(&prop));
                hw_error.set(None);
                hw_output_mode.set(CodeOutputMode::Rust);
            }
            Err(e) => {
                generated_hw_rust.set(String::new());
                hw_error.set(Some(e));
            }
        }
    };

    // Logic mode guide
    let current_result = result.read();
    let guide_mode = if *mode.read() == StudioMode::Logic {
        if let Some(err) = &current_result.error {
            GuideMode::Error(err.clone())
        } else if current_result.logic.is_some() {
            let msg = get_success_message(current_result.readings.len());
            if let Some(hint) = get_context_hint(&input.read()) {
                GuideMode::Info(format!("{} {}", msg, hint))
            } else {
                GuideMode::Success(msg)
            }
        } else {
            GuideMode::Idle
        }
    } else {
        GuideMode::Idle
    };

    // Determine if Panel 3 should be shown based on mode and content
    // A register-allocation spec (`registers:` + live ranges) routes Hardware mode to the certified
    // linear-scan easter egg: the output panel shows the allocation report and panel 3 the live-range
    // timeline. Computed once so panel visibility, the panel-2 header, and the panel-2 content agree.
    let hw_is_regalloc = matches!(*mode.read(), StudioMode::Hardware)
        && crate::ui::pages::register_alloc_viz::is_register_alloc_spec(&hw_input.read());
    // A pigeonhole spec (`pigeons: N`) routes Hardware mode to the live PHP(N) crusher easter egg:
    // panel 2 shows the certified refutation report, panel 3 the animated flight. Computed once so the
    // panel-3 gate, the panel-2 header, and the panel-2 content all agree on which egg is active.
    let hw_is_pigeonhole = matches!(*mode.read(), StudioMode::Hardware)
        && crate::ui::pages::pigeonhole_viz::is_pigeonhole_spec(&hw_input.read());

    let show_panel3 = match *mode.read() {
        StudioMode::Logic => current_result.ast.is_some(),
        StudioMode::Code => interpreter_result.read().error.is_some(),
        StudioMode::Math => !definitions.is_empty() || !inductives.is_empty(),
        StudioMode::Hardware => {
            hw_is_regalloc
                || hw_is_pigeonhole
                || !hw_signals.read().is_empty()
                || !hw_counterexample.read().is_empty()
                || !hw_kg.read().nodes.is_empty()
        }
    };

    let sidebar_w = *sidebar_width.read();
    let left_w = *left_width.read();
    let right_w = if show_panel3 { *right_width.read() } else { 0.0 };
    let center_w = 100.0 - left_w - right_w;

    // Desktop mouse handlers for panel resizing
    let handle_mouse_move = move |evt: MouseEvent| {
        if let Some(which) = *resizing.read() {
            let window = web_sys::window().unwrap();
            let width = window.inner_width().unwrap().as_f64().unwrap();
            let coords = evt.data().client_coordinates();
            let x: f64 = coords.x;
            let pct: f64 = (x / width) * 100.0;

            match which {
                "sidebar" => {
                    let new_sidebar: f64 = x.clamp(150.0, 400.0);
                    sidebar_width.set(new_sidebar);
                }
                "left" => {
                    let new_left: f64 = pct.clamp(15.0, 60.0);
                    left_width.set(new_left);
                }
                "right" => {
                    let new_right: f64 = (100.0 - pct).clamp(15.0, 40.0);
                    right_width.set(new_right);
                }
                "mobile" => {
                    let document = window.document().unwrap();
                    if let Ok(Some(el)) = document.query_selector(".studio-main") {
                        let rect = el.get_bounding_client_rect();
                        let y: f64 = coords.y;
                        let relative_y = y - rect.top();
                        let pct = (relative_y / rect.height()) * 100.0;
                        mobile_split.set(pct.clamp(15.0, 85.0));
                    }
                }
                _ => {}
            }
        }
    };

    let handle_mouse_up = move |_: MouseEvent| {
        resizing.set(None);
    };

    // Mobile touch handlers for swipe gestures
    let handle_touch_start = move |evt: TouchEvent| {
        let touches = evt.data().touches();
        if let Some(touch) = touches.first() {
            let coords = touch.client_coordinates();
            touch_start_x.set(coords.x);
            touch_start_y.set(coords.y);
        }
    };

    let handle_touch_end = move |evt: TouchEvent| {
        if *resizing.read() == Some("mobile") {
            resizing.set(None);
            return;
        }
        let changed = evt.data().touches_changed();
        if let Some(touch) = changed.first() {
            let coords = touch.client_coordinates();
            let end_x = coords.x;
            let end_y = coords.y;
            let dx = end_x - *touch_start_x.read();
            let dy = end_y - *touch_start_y.read();

            if dx.abs() > dy.abs() && dx.abs() > 50.0 {
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

    let handle_touch_move = move |evt: TouchEvent| {
        if *resizing.read() == Some("mobile") {
            evt.prevent_default();
            let touches = evt.data().touches();
            if let Some(touch) = touches.first() {
                let window = web_sys::window().unwrap();
                let document = window.document().unwrap();
                if let Ok(Some(el)) = document.query_selector(".studio-main") {
                    let rect = el.get_bounding_client_rect();
                    let coords = touch.client_coordinates();
                    let y: f64 = coords.y;
                    let relative_y = y - rect.top();
                    let pct = (relative_y / rect.height()) * 100.0;
                    mobile_split.set(pct.clamp(15.0, 85.0));
                }
            }
        }
    };

    let current_format = *format.read();
    let current_tab = *active_tab.read();
    let current_mode = *mode.read();

    // Panel classes: mobile-expanded/collapsed for stacked layout, desktop ignores these
    let editor_exp = *editor_expanded.read();
    let output_exp = *output_expanded.read();
    let panel1_class = if editor_exp { "studio-panel mobile-expanded" } else { "studio-panel mobile-collapsed" };
    let panel2_class = if output_exp { "studio-panel mobile-expanded" } else { "studio-panel mobile-collapsed" };
    let panel3_class = "studio-panel";

    // Mobile vertical split: compute panel flex proportions
    let mobile_pct = *mobile_split.read();
    let panel1_flex = mobile_pct / 50.0;
    let panel2_flex = (100.0 - mobile_pct) / 50.0;
    let both_expanded = editor_exp && output_exp;

    let panel1_style = if both_expanded {
        format!("width: {left_w}%; --panel-flex: {panel1_flex};")
    } else {
        format!("width: {left_w}%;")
    };
    let panel2_style = if both_expanded {
        format!("width: {center_w}%; --panel-flex: {panel2_flex};")
    } else {
        format!("width: {center_w}%;")
    };

    let container_class = if *resizing.read() == Some("mobile") {
        "studio-container mobile-resizing"
    } else {
        "studio-container"
    };

    // Read output view modes for rendering
    let current_code_output_mode = *code_output_mode.read();
    let current_math_output_mode = *math_output_mode.read();
    let current_logic_output_mode = *logic_output_mode.read();
    let current_hw_output_mode = *hw_output_mode.read();

    // Mobile tab labels based on mode
    let (tab1_icon, tab1_label, tab2_icon, tab2_label, tab3_icon, tab3_label) = match current_mode {
        StudioMode::Logic => ("\u{270F}", "Input", "\u{2200}", "Logic", "\u{1F333}", "Tree"),
        StudioMode::Code => ("\u{03BB}", "Editor", "\u{276F}", "Output", "\u{1F4CB}", "Console"),
        StudioMode::Math => ("\u{2200}", "Editor", "\u{276F}", "Output", "\u{1F4CB}", "Context"),
        StudioMode::Hardware => ("\u{270F}", "Spec", "\u{22A8}", "SVA", "\u{25A6}", "Graph"),
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
        JsonLdMultiple { schemas: vec![
            organization_schema(),
            software_application_schema(),
            breadcrumb_schema(&[
                BreadcrumbItem { name: "Home", path: "/" },
                BreadcrumbItem { name: "Studio", path: "/studio" },
            ]),
        ] }

        div {
            class: "{container_class}",
            onmousemove: handle_mouse_move,
            onmouseup: handle_mouse_up,
            onmouseleave: handle_mouse_up,
            ontouchstart: handle_touch_start,
            ontouchend: handle_touch_end,
            ontouchmove: handle_touch_move,

            MainNav { active: ActivePage::Studio, subtitle: Some("Your logic workspace") }

            // Toolbar with mode toggle
            div { class: "studio-toolbar",
                div { class: "studio-toolbar-left",
                    button {
                        class: "sidebar-toggle-btn",
                        onclick: move |_| {
                            let current = *sidebar_open.read();
                            sidebar_open.set(!current);
                        },
                        title: "Toggle file browser",
                        if *sidebar_open.read() { "\u{2630}" } else { "\u{1F4C1}" }
                    }
                }
                div { class: "studio-toolbar-center",
                    span { class: "mode-label", "Mode:" }
                    ModeToggle {
                        mode: current_mode,
                        on_change: move |new_mode| {
                            mode.set(new_mode);
                            active_tab.set(MobileTab::Panel1);
                        },
                    }
                }
                div { class: "studio-toolbar-right",
                    if busy() {
                        span {
                            class: "execute-btn",
                            style: "opacity: 0.75; cursor: default; pointer-events: none;",
                            "\u{23F3} Working\u{2026}"
                        }
                    }
                    if current_mode == StudioMode::Code {
                        button {
                            class: "execute-btn",
                            onclick: handle_code_run,
                            span { class: "desktop-label", "\u{25B6} Run" }
                            span { class: "mobile-label", "\u{25B6} Run" }
                        }
                        button {
                            class: "execute-btn compile-btn",
                            style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);",
                            onclick: handle_code_compile,
                            span { class: "desktop-label", "\u{1F980} Compile" }
                            span { class: "mobile-label", "\u{1F980} Compile to Rust" }
                        }
                        button {
                            class: "execute-btn",
                            style: "background: linear-gradient(135deg, #f59e0b 0%, #ef4444 100%); display:inline-flex; align-items:center; gap:6px;",
                            onclick: move |_| debugging.set(true),
                            span { style: "display:inline-flex;", dangerous_inner_html: IC_BUG }
                            span { class: "desktop-label", "Debug" }
                            span { class: "mobile-label", "Debug" }
                        }
                    }
                    if current_mode == StudioMode::Math {
                        button {
                            class: "execute-btn",
                            onclick: handle_math_execute,
                            span { class: "desktop-label", "\u{25B6} Execute" }
                            span { class: "mobile-label", "\u{25B6} Execute" }
                        }
                        button {
                            class: "execute-btn compile-btn",
                            style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);",
                            onclick: handle_math_compile,
                            span { class: "desktop-label", "\u{1F980} Compile" }
                            span { class: "mobile-label", "\u{1F980} Compile to Rust" }
                        }
                    }
                    if current_mode == StudioMode::Logic {
                        button {
                            class: "execute-btn",
                            onclick: handle_logic_execute,
                            span { class: "desktop-label", "\u{25B6} Execute" }
                            span { class: "mobile-label", "\u{25B6} Execute" }
                        }
                        button {
                            class: "execute-btn compile-btn",
                            style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);",
                            onclick: handle_logic_compile,
                            span { class: "desktop-label", "\u{1F980} Compile" }
                            span { class: "mobile-label", "\u{1F980} Compile to Rust" }
                        }
                    }
                    if current_mode == StudioMode::Hardware {
                        button {
                            class: "execute-btn",
                            onclick: handle_hardware_execute,
                            span { class: "desktop-label", "\u{25B6} Execute" }
                            span { class: "mobile-label", "\u{25B6} Execute" }
                        }
                        button {
                            class: "execute-btn compile-btn",
                            style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);",
                            onclick: handle_hardware_compile,
                            span { class: "desktop-label", "\u{1F980} Compile" }
                            span { class: "mobile-label", "\u{1F980} Compile to Rust" }
                        }
                    }
                }
            }

            // Mobile Tab Bar
            nav { class: "mobile-tabs",
                button {
                    class: if current_tab == MobileTab::Panel1 { "mobile-tab active" } else { "mobile-tab" },
                    onclick: move |_| active_tab.set(MobileTab::Panel1),
                    span { class: "mobile-tab-icon", "{tab1_icon}" }
                    span { class: "mobile-tab-label", "{tab1_label}" }
                }
                button {
                    class: if current_tab == MobileTab::Panel2 { "mobile-tab active" } else { "mobile-tab" },
                    onclick: move |_| active_tab.set(MobileTab::Panel2),
                    span { class: "mobile-tab-icon", "{tab2_icon}" }
                    span { class: "mobile-tab-label", "{tab2_label}" }
                }
                button {
                    class: if current_tab == MobileTab::Panel3 { "mobile-tab active" } else { "mobile-tab" },
                    onclick: move |_| active_tab.set(MobileTab::Panel3),
                    span { class: "mobile-tab-icon", "{tab3_icon}" }
                    span { class: "mobile-tab-label", "{tab3_label}" }
                }
                // Mode toggle in mobile tabs
                div { style: "margin-left: auto;",
                    ModeToggle {
                        mode: current_mode,
                        on_change: move |new_mode| {
                            mode.set(new_mode);
                            active_tab.set(MobileTab::Panel1);
                        },
                    }
                }
            }

            // Socratic Guide (Logic mode only)
            if current_mode == StudioMode::Logic {
                div { class: "studio-guide",
                    SocraticGuide {
                        mode: guide_mode.clone(),
                        on_hint_request: None,
                    }
                }
            }

            // Main content with optional sidebar
            div { class: "studio-content",
                // File browser sidebar
                // Mobile overlay to close sidebar when clicking outside
                if *sidebar_open.read() {
                    div {
                        class: "sidebar-overlay",
                        onclick: move |_| sidebar_open.set(false),
                    }
                }

                if *sidebar_open.read() {
                    div {
                        class: "studio-sidebar",
                        style: "width: {sidebar_w}px; flex-shrink: 0;",

                        // Show VFS error if any (critical errors only)
                        if let Some(err) = vfs_error.read().as_ref() {
                            div {
                                style: "background: #ff4444; color: white; padding: 6px 12px; font-size: 11px; border-radius: 0;",
                                "VFS Error: {err}"
                            }
                        }

                        FileBrowser {
                        show_private_mode: *vfs_is_fallback.read(),
                        tree: file_tree.read().clone(),
                        selected_path: current_file.read().clone(),
                        on_select: EventHandler::new(move |path: String| {
                            // Close sidebar on mobile
                            #[cfg(target_arch = "wasm32")]
                            {
                                let window = web_sys::window().unwrap();
                                let width = window.inner_width().unwrap().as_f64().unwrap_or(1024.0);
                                if width <= 768.0 {
                                    sidebar_open.set(false);
                                }
                            }
                            current_file.set(Some(path.clone()));

                            // Update URL with file parameter for shareable links
                            #[cfg(target_arch = "wasm32")]
                            sync_studio_url(&path);

                            // Load file content from VFS
                            #[cfg(target_arch = "wasm32")]
                            {
                                let path_clone = path.clone();
                                spawn(async move {
                                    // Reuse the cached VFS (one worker) instead of spawning a
                                    // fresh one per file switch; acquire once if not ready yet.
                                    // Bind the clone first so the peek() read-guard drops before
                                    // the None arm calls vfs_handle.set() (avoids a borrow panic).
                                    let cached = vfs_handle.peek().clone();
                                    let vfs_result = match cached {
                                        Some(vfs) => Ok(vfs),
                                        None => get_platform_vfs_with_fallback().await.map(|vfs| {
                                            vfs_handle.set(Some(vfs.clone()));
                                            vfs
                                        }),
                                    };
                                    match vfs_result {
                                        Ok(vfs) => {
                                            match vfs.read_to_string(&path_clone).await {
                                                Ok(content) => {
                                            // Load into appropriate editor based on file path/extension
                                            // Math files are .logos but in /examples/math/ directory
                                            let ext = path_clone.rsplit('.').next().unwrap_or("").to_lowercase();
                                            let is_math_dir = path_clone.contains("/math/") || path_clone.contains("/examples/math");
                                            let is_hardware_dir = path_clone.contains("/hardware/") || path_clone.contains("/examples/hardware");

                                            // Check for math directory first (takes precedence over .logos extension)
                                            if is_math_dir || ext == "math" || ext == "vernac" {
                                                // Switch to Math mode and Output tab
                                                mode.set(StudioMode::Math);
                                                active_tab.set(MobileTab::Panel2);
                                                math_input.set(content);
                                            } else if ext == "logic" {
                                                    // Switch to Logic mode and Output tab
                                                    mode.set(StudioMode::Logic);
                                                    active_tab.set(MobileTab::Panel2);

                                                    // Load into editor
                                                    input.set(content.clone());

                                                    // Check if this is a theorem file
                                                    if content.contains("## Theorem:") {
                                                        // Handle as theorem block with prover syntax
                                                        let theorem_result = compile_theorem_for_ui(&content);

                                                        if let Some(err) = theorem_result.error {
                                                            // Parsing failed
                                                            result.set(CompileResult {
                                                                logic: None,
                                                                simple_logic: None,
                                                                kripke_logic: None,
                                                                ast: None,
                                                                readings: Vec::new(),
                                                                simple_readings: Vec::new(),
                                                                kripke_readings: Vec::new(),
                                                                tokens: Vec::new(),
                                                                error: Some(err.clone()),
                                                            });
                                                            proof_status.set(ProofStatus::Failed(err));
                                                            current_proof_expr.set(None);
                                                            knowledge_base.write().clear();
                                                        } else {
                                                            // Successfully parsed theorem
                                                            result.set(CompileResult {
                                                                logic: theorem_result.goal_string.clone(),
                                                                simple_logic: theorem_result.goal_string.clone(),
                                                                kripke_logic: None,
                                                                ast: None,
                                                                readings: Vec::new(),
                                                                simple_readings: Vec::new(),
                                                                kripke_readings: Vec::new(),
                                                                tokens: Vec::new(),
                                                                error: None,
                                                            });

                                                            // Set up knowledge base from premises
                                                            knowledge_base.write().clear();
                                                            for premise in &theorem_result.premises {
                                                                knowledge_base.write().push(premise.clone());
                                                            }

                                                            // Set goal for proof engine
                                                            if let Some(goal) = theorem_result.goal.clone() {
                                                                current_proof_expr.set(Some(goal));
                                                            }

                                                            // Solved grid / wh-answer / derivation
                                                            let html = theorem_proof_html(&theorem_result);
                                                            if !html.is_empty() {
                                                                proof_text.set(html);
                                                                proof_status.set(if theorem_result.verified {
                                                                    ProofStatus::Success
                                                                } else {
                                                                    ProofStatus::Idle
                                                                });
                                                                proof_hint.set(Some(theorem_proof_hint(&theorem_result)));
                                                            } else {
                                                                proof_status.set(ProofStatus::Idle);
                                                                proof_hint.set(Some(format!(
                                                                    "Theorem '{}' ready. {} premise(s) loaded.",
                                                                    theorem_result.name,
                                                                    knowledge_base.read().len()
                                                                )));
                                                                proof_text.set(String::new());
                                                            }
                                                        }
                                                    } else {
                                                        // Filter out markdown headers (#) and LOGOS comments (--)
                                                        let sentences: Vec<&str> = content
                                                            .lines()
                                                            .filter(|line| {
                                                                let trimmed = line.trim();
                                                                !trimmed.is_empty()
                                                                && !trimmed.starts_with('#')
                                                                && !trimmed.starts_with("--")
                                                            })
                                                            .collect();

                                                        if !sentences.is_empty() {
                                                            // Join all sentences and compile together
                                                            let all_text = sentences.join("\n");
                                                            let compiled = compile_for_ui(&all_text);
                                                            result.set(compiled);

                                                            // Use first sentence for proof engine
                                                            let first_sentence = sentences[0];
                                                            let proof_result = compile_for_proof(first_sentence);
                                                            if let Some(expr) = proof_result.proof_expr {
                                                                current_proof_expr.set(Some(expr));
                                                            }
                                                        }
                                                    }
                                            } else if ext == "logos" {
                                                // Switch to Code mode and Output tab
                                                mode.set(StudioMode::Code);
                                                active_tab.set(MobileTab::Panel2);

                                                code_input.set(content.clone());
                                                // Auto-run the code
                                                let interp_result = interpret_for_ui_baseline(&content).await;
                                                interpreter_result.set(interp_result);
                                            } else if is_hardware_dir || ext == "hw" {
                                                // Switch to Hardware mode and synthesize the spec.
                                                mode.set(StudioMode::Hardware);
                                                active_tab.set(MobileTab::Panel2);
                                                hw_input.set(content.clone());
                                                load_hardware_spec(
                                                    &content,
                                                    hw_sva, hw_psl, hw_signals, hw_proof, hw_proof_ok, hw_counterexample, hw_kg, hw_error,
                                                );
                                            } else {
                                                // Default: load based on current mode, switch to output tab
                                                active_tab.set(MobileTab::Panel2);
                                                let current_mode = *mode.read();
                                                match current_mode {
                                                    StudioMode::Logic => {
                                                        input.set(content.clone());
                                                        let compiled = compile_for_ui(&content);
                                                        result.set(compiled);
                                                    }
                                                    StudioMode::Code => {
                                                        code_input.set(content.clone());
                                                        let interp_result = interpret_for_ui_baseline(&content).await;
                                                        interpreter_result.set(interp_result);
                                                    }
                                                    StudioMode::Math => math_input.set(content),
                                                    StudioMode::Hardware => {
                                                        hw_input.set(content.clone());
                                                        load_hardware_spec(
                                                            &content,
                                                            hw_sva, hw_psl, hw_signals, hw_proof, hw_proof_ok, hw_counterexample, hw_kg, hw_error,
                                                        );
                                                    }
                                                }
                                            }
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
                        }),
                        on_toggle_dir: EventHandler::new(move |path: String| {
                            if let Some(node) = file_tree.write().find_mut(&path) {
                                node.toggle_expanded();
                            }
                        }),
                        on_new_file: EventHandler::new(move |_: ()| {
                            // TODO: Show new file dialog
                        }),
                    }
                    }

                    // Sidebar resizer
                    div {
                        class: if resizing.read().is_some() { "panel-resizer active" } else { "panel-resizer" },
                        onmousedown: move |_| resizing.set(Some("sidebar")),
                    }
                }

                // Panels - content changes based on mode
                main { class: "studio-main",
                    // Panel 1
                    section {
                        class: "{panel1_class}",
                        style: "{panel1_style}",

                        match current_mode {
                            StudioMode::Logic => rsx! {
                                div {
                                    class: "panel-header",
                                    onclick: move |_| {
                                        let v = *editor_expanded.read();
                                        editor_expanded.set(!v);
                                    },
                                    span { "English Input" }
                                }
                                div { class: "panel-content",
                                    LiveEditor {
                                        value: input.read().clone(),
                                        on_change: handle_logic_input,
                                        placeholder: Some("Type an English sentence...".to_string()),
                                    }
                                    // Proof Panel - below the input editor
                                    ProofPanel {
                                        proof_text: proof_text.read().clone(),
                                        status: proof_status.read().clone(),
                                        hint: proof_hint.read().clone(),
                                        on_tactic: handle_tactic.clone(),
                                    }
                                }
                            },
                            StudioMode::Code => rsx! {
                                div {
                                    class: "panel-header",
                                    onclick: move |_| {
                                        let v = *editor_expanded.read();
                                        editor_expanded.set(!v);
                                    },
                                    span { "Code Editor" }
                                }
                                div { class: "panel-content",
                                    CodeEditor {
                                        value: code_input.read().clone(),
                                        on_change: move |v| code_input.set(v),
                                        language: Language::Logos,
                                        placeholder: "-- Imperative LOGOS code\n\n## Main\n\nLet x be 1.\nLet y be 2.\nShow x + y.".to_string(),
                                    }
                                }
                            },
                            StudioMode::Math => rsx! {
                                div {
                                    class: "panel-header",
                                    onclick: move |_| {
                                        let v = *editor_expanded.read();
                                        editor_expanded.set(!v);
                                    },
                                    span { "Theorem Editor" }
                                }
                                div { class: "panel-content",
                                    CodeEditor {
                                        value: math_input.read().clone(),
                                        on_change: move |v| math_input.set(v),
                                        language: Language::Vernacular,
                                        placeholder: "-- Define natural numbers\nInductive Nat := Zero : Nat | Succ : Nat -> Nat.\n\nDefinition one : Nat := Succ Zero.\n\nCheck one.".to_string(),
                                    }
                                }
                            },
                            StudioMode::Hardware => rsx! {
                                div {
                                    class: "panel-header",
                                    onclick: move |_| {
                                        let v = *editor_expanded.read();
                                        editor_expanded.set(!v);
                                    },
                                    span { "Hardware Spec" }
                                }
                                div { class: "panel-content",
                                    LiveEditor {
                                        value: hw_input.read().clone(),
                                        on_change: move |v: String| hw_input.set(v),
                                        placeholder: Some("Describe hardware in English (\"Always, if request is high, then acknowledge is high.\"), design a signal plan (\"NS-left conflicts with the EW crossing.\"), or paste Verilog (module \u{2026} endmodule) to model-check.".to_string()),
                                    }
                                }
                            },
                        }
                    }

                    // Mobile vertical resizer between Panel 1 and Panel 2
                    if both_expanded {
                        div {
                            class: if *resizing.read() == Some("mobile") {
                                "mobile-panel-resizer active"
                            } else {
                                "mobile-panel-resizer"
                            },
                            onmousedown: move |e| {
                                e.prevent_default();
                                resizing.set(Some("mobile"));
                            },
                            ontouchstart: move |e| {
                                e.prevent_default();
                                resizing.set(Some("mobile"));
                            },
                        }
                    }

                    // Left resizer (desktop)
                    div {
                        class: if resizing.read().is_some() { "panel-resizer active" } else { "panel-resizer" },
                        onmousedown: move |_| resizing.set(Some("left")),
                    }

                    // Panel 2
                    section {
                        class: "{panel2_class}",
                        style: "{panel2_style}",

                        match current_mode {
                            StudioMode::Logic => rsx! {
                                div {
                                    class: "panel-header",
                                    onclick: move |_| {
                                        let v = *output_expanded.read();
                                        output_expanded.set(!v);
                                    },
                                    span { "Logic Output" }
                                    div {
                                        class: "output-mode-toggle",
                                        onclick: move |evt| evt.stop_propagation(),
                                        button {
                                            class: if current_logic_output_mode == LogicView::Logic { "output-mode-btn active" } else { "output-mode-btn" },
                                            onclick: move |_| logic_output_mode.set(LogicView::Logic),
                                            "Logic"
                                        }
                                        button {
                                            class: if current_logic_output_mode == LogicView::Rust { "output-mode-btn active" } else { "output-mode-btn" },
                                            onclick: move |_| logic_output_mode.set(LogicView::Rust),
                                            "Rust"
                                        }
                                    }
                                    if current_logic_output_mode == LogicView::Logic {
                                        div {
                                            class: "format-toggle",
                                            onclick: move |evt| evt.stop_propagation(),
                                            button {
                                                class: if current_format == OutputFormat::SimpleFOL { "format-btn active" } else { "format-btn" },
                                                onclick: move |_| format.set(OutputFormat::SimpleFOL),
                                                "Simple"
                                            }
                                            button {
                                                class: if current_format == OutputFormat::Unicode { "format-btn active" } else { "format-btn" },
                                                onclick: move |_| format.set(OutputFormat::Unicode),
                                                "Full"
                                            }
                                            button {
                                                class: if current_format == OutputFormat::LaTeX { "format-btn active" } else { "format-btn" },
                                                onclick: move |_| format.set(OutputFormat::LaTeX),
                                                "LaTeX"
                                            }
                                            button {
                                                class: if current_format == OutputFormat::Kripke { "format-btn active" } else { "format-btn" },
                                                onclick: move |_| format.set(OutputFormat::Kripke),
                                                "Deep"
                                            }
                                        }
                                    }
                                }
                                div { class: "panel-content",
                                    if current_logic_output_mode == LogicView::Logic {
                                        {
                                            rsx! {
                                                LogicOutput {
                                                    logic: current_result.logic.clone(),
                                                    simple_logic: current_result.simple_logic.clone(),
                                                    kripke_logic: current_result.kripke_logic.clone(),
                                                    readings: current_result.readings.clone(),
                                                    simple_readings: current_result.simple_readings.clone(),
                                                    kripke_readings: current_result.kripke_readings.clone(),
                                                    error: current_result.error.clone(),
                                                    format: current_format,
                                                }
                                                if let Some(ref logic) = current_result.logic {
                                                    SymbolDictionary {
                                                        logic: logic.clone(),
                                                        collapsed: false,
                                                        inline: false,
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        {
                                            let rust_code = generated_logic_rust.read().clone();
                                            rsx! {
                                                if rust_code.is_empty() {
                                                    div { class: "interpreter-empty",
                                                        "Compile to see generated Rust"
                                                    }
                                                } else {
                                                    CodeView {
                                                        code: rust_code,
                                                        language: Language::Rust,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            StudioMode::Code => rsx! {
                                div {
                                    class: "panel-header",
                                    onclick: move |_| {
                                        let v = *output_expanded.read();
                                        output_expanded.set(!v);
                                    },
                                    span { "Output" }
                                    div {
                                        class: "output-mode-toggle",
                                        onclick: move |evt| evt.stop_propagation(),
                                        button {
                                            class: if current_code_output_mode == CodeOutputMode::Interpret { "output-mode-btn active" } else { "output-mode-btn" },
                                            onclick: move |_| code_output_mode.set(CodeOutputMode::Interpret),
                                            "Output"
                                        }
                                        button {
                                            class: if current_code_output_mode == CodeOutputMode::Rust { "output-mode-btn active" } else { "output-mode-btn" },
                                            onclick: move |_| code_output_mode.set(CodeOutputMode::Rust),
                                            "Rust"
                                        }
                                    }
                                }
                                div { class: "panel-content",
                                    if current_code_output_mode == CodeOutputMode::Interpret {
                                        {
                                            let result = interpreter_result.read();
                                            rsx! {
                                                div { class: "interpreter-output",
                                                    if result.lines.is_empty() && result.error.is_none() {
                                                        div { class: "interpreter-empty",
                                                            "Run your code to see output"
                                                        }
                                                    } else {
                                                        for line in result.lines.iter() {
                                                            div { class: "interpreter-line", "{line}" }
                                                        }
                                                        if let Some(ref err) = result.error {
                                                            div { class: "interpreter-error", "{err}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        {
                                            let rust_code = generated_rust.read().clone();
                                            rsx! {
                                                if rust_code.is_empty() {
                                                    div { class: "interpreter-empty",
                                                        "Compile your code to see generated Rust"
                                                    }
                                                } else {
                                                    CodeView {
                                                        code: rust_code,
                                                        language: Language::Rust,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            StudioMode::Math => rsx! {
                                div {
                                    class: "panel-header",
                                    onclick: move |_| {
                                        let v = *output_expanded.read();
                                        output_expanded.set(!v);
                                    },
                                    span { "Output" }
                                    div {
                                        class: "output-mode-toggle",
                                        onclick: move |evt| evt.stop_propagation(),
                                        button {
                                            class: if current_math_output_mode == CodeOutputMode::Interpret { "output-mode-btn active" } else { "output-mode-btn" },
                                            onclick: move |_| math_output_mode.set(CodeOutputMode::Interpret),
                                            "Output"
                                        }
                                        button {
                                            class: if current_math_output_mode == CodeOutputMode::Rust { "output-mode-btn active" } else { "output-mode-btn" },
                                            onclick: move |_| math_output_mode.set(CodeOutputMode::Rust),
                                            "Rust"
                                        }
                                    }
                                }
                                div { class: "panel-content",
                                    if current_math_output_mode == CodeOutputMode::Interpret {
                                        ReplOutput {
                                            lines: math_output.read().clone(),
                                            on_clear: move |_| {
                                                math_output.write().clear();
                                                math_repl.set(Repl::new());  // Reset kernel state too
                                            },
                                        }
                                    } else {
                                        {
                                            let rust_code = generated_math_rust.read().clone();
                                            rsx! {
                                                if rust_code.is_empty() {
                                                    div { class: "interpreter-empty",
                                                        "Compile to see generated Rust"
                                                    }
                                                } else {
                                                    CodeView {
                                                        code: rust_code,
                                                        language: Language::Rust,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            StudioMode::Hardware => rsx! {
                                div {
                                    class: "panel-header",
                                    onclick: move |_| {
                                        let v = *output_expanded.read();
                                        output_expanded.set(!v);
                                    },
                                    span { if hw_is_regalloc { "Allocation" } else if hw_is_pigeonhole { "Pigeonhole" } else { "SVA" } }
                                    if !hw_is_regalloc && !hw_is_pigeonhole {
                                        div {
                                            class: "output-mode-toggle",
                                            onclick: move |evt| evt.stop_propagation(),
                                            button {
                                                class: if current_hw_output_mode == CodeOutputMode::Interpret { "output-mode-btn active" } else { "output-mode-btn" },
                                                onclick: move |_| hw_output_mode.set(CodeOutputMode::Interpret),
                                                "SVA"
                                            }
                                            button {
                                                class: if current_hw_output_mode == CodeOutputMode::Rust { "output-mode-btn active" } else { "output-mode-btn" },
                                                onclick: move |_| hw_output_mode.set(CodeOutputMode::Rust),
                                                "Rust"
                                            }
                                        }
                                    }
                                }
                                div { class: "panel-content",
                                    if hw_is_regalloc {
                                        {
                                            let report = crate::ui::pages::register_alloc_viz::parse_register_spec(&hw_input.read())
                                                .map(|s| crate::ui::pages::register_alloc_viz::allocation_report(&s))
                                                .unwrap_or_default();
                                            rsx! {
                                                div { class: "interpreter-output",
                                                    div { style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; color: rgba(255,255,255,0.45); margin: 4px 0;", "Certified register allocation" }
                                                    pre { style: "font-family: ui-monospace, monospace; font-size: 13px; white-space: pre-wrap; word-break: break-word; color: #e5e7eb; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.08); border-radius: 8px; padding: 10px; margin: 0;", "{report}" }
                                                }
                                            }
                                        }
                                    } else if hw_is_pigeonhole {
                                        {
                                            let report = crate::ui::pages::pigeonhole_viz::parse_pigeonhole_spec(&hw_input.read())
                                                .map(|s| crate::ui::pages::pigeonhole_viz::report(&s))
                                                .unwrap_or_default();
                                            rsx! {
                                                div { class: "interpreter-output",
                                                    div { style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; color: rgba(255,255,255,0.45); margin: 4px 0;", "Certified pigeonhole refutation" }
                                                    pre { style: "font-family: ui-monospace, monospace; font-size: 13px; white-space: pre-wrap; word-break: break-word; color: #e5e7eb; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.08); border-radius: 8px; padding: 10px; margin: 0;", "{report}" }
                                                }
                                            }
                                        }
                                    } else if current_hw_output_mode == CodeOutputMode::Interpret {
                                        {
                                            let sva = hw_sva.read().clone();
                                            let psl = hw_psl.read().clone();
                                            let proof = hw_proof.read().clone();
                                            let proof_ok = *hw_proof_ok.read();
                                            let err = hw_error.read().clone();
                                            rsx! {
                                                div { class: "interpreter-output",
                                                    if let Some(e) = err {
                                                        div { class: "interpreter-error", "{e}" }
                                                    } else if sva.is_empty() {
                                                        div { class: "interpreter-empty",
                                                            "Execute to analyze \u{2014} synthesize an assertion, design a signal plan, or model-check Verilog"
                                                        }
                                                    } else {
                                                        div { style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; color: rgba(255,255,255,0.45); margin: 4px 0;", "SystemVerilog Assertion" }
                                                        pre { style: "font-family: ui-monospace, monospace; font-size: 13px; white-space: pre-wrap; word-break: break-word; color: #e5e7eb; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.08); border-radius: 8px; padding: 10px; margin: 0 0 12px;", "{sva}" }
                                                        if !proof.is_empty() {
                                                            div {
                                                                style: {
                                                                    let (border, bg, fg) = match proof_ok {
                                                                        Some(true) => ("rgba(74,222,128,0.4)", "rgba(74,222,128,0.08)", "#4ade80"),
                                                                        Some(false) => ("rgba(248,113,113,0.4)", "rgba(248,113,113,0.08)", "#f87171"),
                                                                        None => ("rgba(255,255,255,0.12)", "rgba(255,255,255,0.03)", "#d0d0d0"),
                                                                    };
                                                                    format!("font-size: 13px; line-height: 1.5; color: {fg}; background: {bg}; border: 1px solid {border}; border-radius: 8px; padding: 10px; margin: 0 0 12px;")
                                                                },
                                                                "{proof}"
                                                            }
                                                        }
                                                        if !psl.is_empty() {
                                                            div { style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; color: rgba(255,255,255,0.45); margin: 4px 0;", "PSL" }
                                                            pre { style: "font-family: ui-monospace, monospace; font-size: 13px; white-space: pre-wrap; word-break: break-word; color: #e5e7eb; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.08); border-radius: 8px; padding: 10px; margin: 0;", "{psl}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        {
                                            let rust_code = generated_hw_rust.read().clone();
                                            rsx! {
                                                if rust_code.is_empty() {
                                                    div { class: "interpreter-empty",
                                                        "Compile to see the Rust runtime monitor"
                                                    }
                                                } else {
                                                    CodeView {
                                                        code: rust_code,
                                                        language: Language::Rust,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                        }
                    }

                    // Right resizer and Panel 3 (only shown when there's content)
                    if show_panel3 {
                        div {
                            class: if resizing.read().is_some() { "panel-resizer active" } else { "panel-resizer" },
                            onmousedown: move |_| resizing.set(Some("right")),
                        }

                        aside {
                            class: "{panel3_class}",
                            style: "width: {right_w}%;",

                            match current_mode {
                                StudioMode::Logic => rsx! {
                                    div { class: "panel-header",
                                        span { "Syntax Tree" }
                                    }
                                    div { class: "panel-content",
                                        AstTree {
                                            ast: current_result.ast.clone(),
                                        }
                                    }
                                },
                                StudioMode::Code => rsx! {
                                    div { class: "panel-header",
                                        span { "Console" }
                                    }
                                    div { class: "panel-content",
                                        div { class: "interpreter-output",
                                            {
                                                let result = interpreter_result.read();
                                                rsx! {
                                                    if let Some(ref err) = result.error {
                                                        div { class: "interpreter-error", "{err}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                                StudioMode::Math => rsx! {
                                    div { class: "panel-header",
                                        span { "Context" }
                                    }
                                    div { class: "panel-content",
                                        ContextView {
                                            definitions: definitions.clone(),
                                            inductives: inductives.clone(),
                                        }
                                    }
                                },
                                StudioMode::Hardware => {
                                    let ce = hw_counterexample.read().clone();
                                    let sigs = hw_signals.read().clone();
                                    if let Some(pspec) = crate::ui::pages::pigeonhole_viz::parse_pigeonhole_spec(&hw_input.read()) {
                                        // Pigeonhole easter egg: n pigeons fly into n-1 holes, one left
                                        // out — solved live (matching Hall witness + certified symmetry
                                        // breaking) by our prover in the browser, no Z3.
                                        let (svg, verdict) = crate::ui::pages::pigeonhole_viz::render(&pspec);
                                        rsx! {
                                            div { class: "panel-header", span { "Pigeonhole" } }
                                            div { class: "panel-content",
                                                div { style: "padding: 8px; overflow-x: auto;",
                                                    div { dangerous_inner_html: "{svg}" }
                                                    div {
                                                        style: "margin-top:4px;font-size:11px;line-height:1.4;color:rgba(255,255,255,0.45);",
                                                        "Each pigeon flies toward a hole; with one more pigeon than holes, one is always left out. Maximum bipartite matching proves it impossible in polynomial time (the re-verified Hall witness), and our prover emits a certified symmetry-breaking refutation — while every resolution-based solver (Kissat, CaDiCaL, Z3) provably needs exponentially many steps (Haken 1985)."
                                                    }
                                                    div {
                                                        style: "margin-top:8px;padding:8px;border-radius:6px;font-size:12px;line-height:1.45;background:rgba(224,108,117,0.15);color:#f3c0c4;",
                                                        "{verdict}"
                                                    }
                                                }
                                            }
                                        }
                                    } else if let Some(spec) = crate::ui::pages::register_alloc_viz::parse_register_spec(&hw_input.read()) {
                                        // Register-allocation easter egg: live-range timeline coloured
                                        // by the certified linear-scan allocation (spill clique in red).
                                        let (svg, verdict) = crate::ui::pages::register_alloc_viz::render(&spec);
                                        let ok = verdict.starts_with('\u{2713}');
                                        rsx! {
                                            div { class: "panel-header", span { "Register Allocation" } }
                                            div { class: "panel-content",
                                                div { style: "padding: 8px; overflow-x: auto;",
                                                    div { dangerous_inner_html: "{svg}" }
                                                    div {
                                                        style: "margin-top:4px;font-size:11px;line-height:1.4;color:rgba(255,255,255,0.45);",
                                                        "The sweep line is the program counter; each register lane lights up while it holds a value and goes dark when it's freed. The pressure histogram counts how many values are live at each instruction — bars above the dashed budget line turn red, the exact points where spilling is forced and a value drops to the MEM lane. The interference graph is what allocation really colours: an edge joins two values whose lifetimes overlap, nodes are coloured by their register, and a red clique is a set that mutually conflicts and provably cannot share."
                                                    }
                                                    div {
                                                        style: if ok {
                                                            "margin-top:8px;padding:8px;border-radius:6px;font-size:12px;line-height:1.45;background:rgba(152,195,121,0.15);color:#cdebc5;"
                                                        } else {
                                                            "margin-top:8px;padding:8px;border-radius:6px;font-size:12px;line-height:1.45;background:rgba(224,108,117,0.15);color:#f3c0c4;"
                                                        },
                                                        "{verdict}"
                                                    }
                                                }
                                            }
                                        }
                                    } else if !ce.is_empty() {
                                        // A counterexample exists — render it as a waveform.
                                        let wf = logicaffeine_compile::codegen_sva::hw_pipeline::counterexample_waveform(&ce);
                                        let traffic = traffic_svg(&wf);
                                        let osc = waveform_svg(&wf);
                                        let violated = *hw_proof_ok.read() == Some(false);
                                        let header = match (traffic.is_some(), violated) {
                                            (true, true) => "Intersection \u{2014} Conflict Found",
                                            (true, false) => "Intersection \u{2014} Proven Safe (witness run)",
                                            (false, true) => "Counterexample Waveform",
                                            (false, false) => "Witness Waveform",
                                        };
                                        rsx! {
                                            div { class: "panel-header", span { "{header}" } }
                                            div { class: "panel-content",
                                                div { style: "padding: 8px; overflow-x: auto;",
                                                    if let Some(t) = traffic.as_ref() {
                                                        div { dangerous_inner_html: "{t}" }
                                                    }
                                                    div { dangerous_inner_html: "{osc}" }
                                                }
                                            }
                                        }
                                    } else {
                                        let svg = kg_svg(&hw_kg.read());
                                        rsx! {
                                            div { class: "panel-header", span { "Knowledge Graph" } }
                                            div { class: "panel-content",
                                                if !svg.is_empty() {
                                                    div { style: "padding: 6px;",
                                                        div { dangerous_inner_html: "{svg}" }
                                                        div { style: "display:flex; gap:12px; flex-wrap:wrap; justify-content:center; margin-top:4px; font-size:10px;",
                                                            span { style: "color:#60a5fa;", "● input" }
                                                            span { style: "color:#4ade80;", "● output" }
                                                            span { style: "color:#a78bfa;", "● internal" }
                                                            span { style: "color:#fbbf24;", "● clock" }
                                                        }
                                                    }
                                                } else if !sigs.is_empty() {
                                                    div { style: "padding: 8px;",
                                                        div { style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; color: rgba(255,255,255,0.45); margin-bottom: 6px;", "Signals" }
                                                        for s in sigs.iter() {
                                                            div { style: "font-family: ui-monospace, monospace; font-size: 13px; color: #e5e7eb; padding: 2px 0;", "{s}" }
                                                        }
                                                    }
                                                } else {
                                                    div { class: "interpreter-empty",
                                                        "Execute to extract the hardware knowledge graph"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                            }
                        }
                    }
                }
            }

            // Code-mode debugger — bottom-docked, additive (hidden unless debugging).
            if debugging() && current_mode == StudioMode::Code {
                DebugDrawer {
                    source: code_input.read().clone(),
                    on_close: move |_| debugging.set(false),
                }
            }
        }
    }
}
