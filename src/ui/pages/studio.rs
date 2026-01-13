//! Studio page - multi-mode playground for Logic, Code, and Math.

use dioxus::prelude::*;
use crate::{
    compile_for_ui, compile_for_proof, compile_theorem_for_ui, generate_rust_code,
    interpret_for_ui, CompileResult, InterpreterResult, ProofCompileResult, TheoremCompileResult,
    BackwardChainer, DerivationTree, ProofExpr,
};
use crate::ui::components::editor::LiveEditor;
use crate::ui::components::logic_output::{LogicOutput, OutputFormat};
use crate::ui::components::ast_tree::AstTree;
use crate::ui::components::socratic_guide::{SocraticGuide, GuideMode, get_success_message, get_context_hint};
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::symbol_dictionary::SymbolDictionary;
use crate::ui::components::vocab_reference::VocabReference;
use crate::ui::components::mode_toggle::ModeToggle;
use crate::ui::components::file_browser::FileBrowser;
use crate::ui::components::repl_output::ReplOutput;
use crate::ui::components::context_view::{ContextView, ContextEntry, EntryKind};
use crate::ui::components::code_editor::{CodeEditor, CodeView, Language};
use crate::ui::components::proof_panel::{ProofPanel, ProofStatus, Tactic};
use crate::ui::state::{StudioMode, FileNode, ReplLine};
use crate::ui::responsive::{MOBILE_BASE_STYLES, MOBILE_TAB_BAR_STYLES};
use crate::interface::Repl;
use crate::proof::hints::{suggest_hint, SuggestedTactic};
use crate::ui::examples::seed_examples;
use logos_core::fs::{get_platform_vfs, Vfs, DirEntry, VfsResult};
use std::rc::Rc;

/// Parse math code into complete statements.
///
/// Handles both Coq-style (period-terminated) and Literate syntax (block-based):
/// - `## To ...` blocks: collect header + all indented lines until non-indented line
/// - `A X is either:` blocks: collect header + indented variants
/// - Traditional commands: accumulate until period-terminator
fn parse_math_statements(code: &str) -> Vec<String> {
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

/// Code mode output toggle - interpret output vs generated Rust
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum CodeOutputMode {
    #[default]
    Interpret,
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
    padding: 8px 16px;
    background: var(--studio-panel-bg);
    border-bottom: 1px solid var(--studio-border);
    gap: 12px;
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
    padding: 6px 14px;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    border: none;
    border-radius: 6px;
    color: white;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
}

.execute-btn:hover {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(102, 126, 234, 0.3);
}

.execute-btn:active {
    transform: translateY(0);
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
@media (max-width: 768px) {
    /* Hide toolbar center on mobile */
    .studio-toolbar-center {
        display: none;
    }

    .studio-toolbar {
        padding: 8px 12px;
    }

    /* Hide desktop resizers */
    .panel-resizer {
        display: none;
    }

    /* Mobile main switches to column with stacked panels */
    .studio-main {
        flex-direction: column;
        position: relative;
        gap: 0;
        background: var(--studio-bg);
    }

    /* Panels are absolute positioned and hidden by default */
    .studio-panel {
        min-width: unset;
        min-height: unset;
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        opacity: 0;
        pointer-events: none;
        transition: opacity 0.15s ease;
        width: 100% !important;
    }

    /* Active panel becomes visible */
    .studio-panel.mobile-active {
        position: relative;
        flex: 1;
        opacity: 1;
        pointer-events: auto;
    }

    /* Hide panel headers on mobile (tabs replace them) */
    .studio-panel .panel-header {
        display: none;
    }

    /* Show header only for Logic panel when it has format toggle */
    .studio-panel.mobile-active.has-controls .panel-header {
        display: flex;
        padding: 10px 14px;
        background: var(--studio-elevated);
        border-bottom: 1px solid var(--studio-border);
    }

    /* Mobile-sized format toggle */
    .format-toggle {
        gap: 6px;
        padding: 4px;
        border-radius: 8px;
    }

    .format-btn {
        padding: 10px 16px;
        font-size: 14px;
        border-radius: 6px;
        min-height: var(--touch-min, 44px);
        min-width: var(--touch-min, 44px);
        display: flex;
        align-items: center;
        justify-content: center;
    }

    /* Footer constraints */
    .studio-footer {
        max-height: 30vh;
        overflow: auto;
    }
}

/* Extra small screens */
@media (max-width: 480px) {
    .format-btn {
        padding: 8px 12px;
        font-size: 13px;
    }
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

#[component]
pub fn Studio() -> Element {
    // Mode state
    let mut mode = use_signal(|| StudioMode::Logic);

    // File browser state
    let mut sidebar_open = use_signal(|| true);
    let mut file_tree = use_signal(FileNode::root);
    let mut current_file = use_signal(|| None::<String>);

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

    // Code mode state (imperative .logos)
    let mut code_input = use_signal(String::new);
    let mut code_output_mode = use_signal(|| CodeOutputMode::Interpret);
    let mut interpreter_result = use_signal(|| InterpreterResult {
        lines: vec![],
        error: None,
    });
    let mut generated_rust = use_signal(String::new);

    // Math mode state (vernacular/theorem proving)
    let mut math_input = use_signal(String::new);
    let mut math_repl = use_signal(Repl::new);
    let mut math_output = use_signal(Vec::<ReplLine>::new);

    // Desktop panel resizing state
    let mut left_width = use_signal(|| 35.0f64);
    let mut right_width = use_signal(|| 25.0f64);
    let mut resizing = use_signal(|| None::<&'static str>);

    // Mobile tab state
    let mut active_tab = use_signal(|| MobileTab::Panel1);

    // Touch gesture state for swipe detection
    let mut touch_start_x = use_signal(|| 0.0f64);
    let mut touch_start_y = use_signal(|| 0.0f64);

    // VFS initialization flag
    let mut vfs_initialized = use_signal(|| false);

    // Initialize VFS and seed examples on mount
    #[cfg(target_arch = "wasm32")]
    use_effect(move || {
        if *vfs_initialized.read() {
            return;
        }
        vfs_initialized.set(true);

        spawn(async move {
            // Get platform VFS (OPFS on WASM)
            match get_platform_vfs().await {
                Ok(vfs) => {
                    // Seed example files if they don't exist
                    if let Err(e) = seed_examples(&vfs).await {
                        web_sys::console::log_1(&format!("Failed to seed examples: {:?}", e).into());
                    }

                    // Build file tree from VFS
                    let mut root = FileNode::root();
                    if let Ok(()) = load_dir_recursive(&vfs, "/", &mut root).await {
                        file_tree.set(root);
                    }

                    // Load default example: Socrates theorem
                    let default_file = "/examples/logic/prover-demo.logic";
                    if let Ok(content) = vfs.read_to_string(default_file).await {
                        current_file.set(Some(default_file.to_string()));
                        input.set(content.clone());

                        // Compile the theorem
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

                            // Set up knowledge base from premises
                            knowledge_base.write().clear();
                            for premise in theorem_result.premises {
                                knowledge_base.write().push(premise);
                            }

                            // Set goal for proof engine
                            if let Some(goal) = theorem_result.goal {
                                current_proof_expr.set(Some(goal));
                            }

                            // If derivation was found (auto-proved), show it
                            if let Some(derivation) = theorem_result.derivation {
                                let tree_html = format_derivation_html(&derivation);
                                proof_text.set(tree_html);
                                proof_status.set(ProofStatus::Success);
                                proof_hint.set(Some(format!("Theorem '{}' proved!", theorem_result.name)));
                            } else {
                                proof_status.set(ProofStatus::Idle);
                                proof_hint.set(Some(format!(
                                    "Theorem '{}' ready. {} premise(s) loaded.",
                                    theorem_result.name,
                                    knowledge_base.read().len()
                                )));
                            }
                        }
                    }
                }
                Err(e) => {
                    web_sys::console::log_1(&format!("Failed to initialize VFS: {:?}", e).into());
                }
            }
        });
    });

    // On native, create a simple placeholder file tree (VFS init is synchronous)
    #[cfg(not(target_arch = "wasm32"))]
    use_effect(move || {
        if *vfs_initialized.read() {
            return;
        }
        vfs_initialized.set(true);

        // For native builds, just set up a placeholder tree
        // Real native builds would use tokio runtime
        let mut root = FileNode::root();
        root.children.push(FileNode::directory("examples".to_string(), "/examples".to_string()));
        file_tree.set(root);
    });

    // Logic mode input handler - compiles for both UI and proof engine
    let handle_logic_input = move |new_value: String| {
        input.set(new_value.clone());

        // Check if input contains a theorem block
        if new_value.contains("## Theorem:") {
            // Handle as theorem block with prover syntax
            let theorem_result = compile_theorem_for_ui(&new_value);

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
                // Set goal as the logic output
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
                for premise in theorem_result.premises {
                    knowledge_base.write().push(premise);
                }

                // Set goal for proof engine
                if let Some(goal) = theorem_result.goal {
                    current_proof_expr.set(Some(goal));
                }

                // If derivation was found (auto-proved), show it
                if let Some(derivation) = theorem_result.derivation {
                    let tree_html = format_derivation_html(&derivation);
                    proof_text.set(tree_html);
                    proof_status.set(ProofStatus::Success);
                    proof_hint.set(Some(format!("Theorem '{}' proved!", theorem_result.name)));
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
            // Handle as plain English sentences
            // Filter out markdown headers (#) and LOGOS comments (--)
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
                // Join all sentences and compile together
                let all_text = sentences.join("\n");
                let compiled = compile_for_ui(&all_text);
                result.set(compiled);

                // Clear knowledge base for plain sentences
                knowledge_base.write().clear();

                // Compile first sentence for proof engine
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

    // Code mode: Run button handler (interpret)
    let handle_code_run = move |_| {
        let code = code_input.read().clone();
        // Switch to Output tab (Panel2) on mobile and switch to Output mode
        active_tab.set(MobileTab::Panel2);
        code_output_mode.set(CodeOutputMode::Interpret);
        spawn(async move {
            let result = interpret_for_ui(&code).await;
            interpreter_result.set(result);
        });
    };

    // Code mode: Compile button handler (generate Rust)
    // Uses generate_rust_code which works on WASM
    let handle_code_compile = move |_| {
        let code = code_input.read().clone();
        // Switch to Output tab (Panel2) on mobile
        active_tab.set(MobileTab::Panel2);
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
    };

    // Math mode execute handler (vernacular REPL)
    // Handles multi-line statements that end with '.'
    let handle_math_execute = move |_| {
        // Switch to Output tab (Panel2) on mobile
        active_tab.set(MobileTab::Panel2);

        let code = math_input.read().clone();

        // Parse math code into complete statements
        // Handles both Coq-style (period-terminated) and Literate syntax (block-based)
        let statements = parse_math_statements(&code);

        for stmt in statements {
            match math_repl.write().execute(&stmt) {
                Ok(output) => {
                    math_output.write().push(ReplLine::success(stmt, output));
                }
                Err(e) => {
                    math_output.write().push(ReplLine::error(stmt, e.to_string()));
                }
            }
        }
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
    let show_panel3 = match *mode.read() {
        StudioMode::Logic => current_result.ast.is_some(),
        StudioMode::Code => interpreter_result.read().error.is_some(),
        StudioMode::Math => !definitions.is_empty() || !inductives.is_empty(),
    };

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
                "left" => {
                    let new_left: f64 = pct.clamp(15.0, 60.0);
                    left_width.set(new_left);
                }
                "right" => {
                    let new_right: f64 = (100.0 - pct).clamp(15.0, 40.0);
                    right_width.set(new_right);
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

    let current_format = *format.read();
    let current_tab = *active_tab.read();
    let current_mode = *mode.read();

    // Panel classes based on active tab
    let panel1_class = if current_tab == MobileTab::Panel1 { "studio-panel mobile-active" } else { "studio-panel" };
    let panel2_class = if current_tab == MobileTab::Panel2 {
        if current_mode == StudioMode::Logic { "studio-panel mobile-active has-controls" } else { "studio-panel mobile-active" }
    } else { "studio-panel" };
    let panel3_class = if current_tab == MobileTab::Panel3 { "studio-panel mobile-active" } else { "studio-panel" };

    // Read code output mode for rendering
    let current_code_output_mode = *code_output_mode.read();

    // Mobile tab labels based on mode
    let (tab1_icon, tab1_label, tab2_icon, tab2_label, tab3_icon, tab3_label) = match current_mode {
        StudioMode::Logic => ("\u{270F}", "Input", "\u{2200}", "Logic", "\u{1F333}", "Tree"),
        StudioMode::Code => ("\u{03BB}", "Editor", "\u{276F}", "Output", "\u{1F4CB}", "Console"),
        StudioMode::Math => ("\u{2200}", "Editor", "\u{276F}", "Output", "\u{1F4CB}", "Context"),
    };

    rsx! {
        style { "{MOBILE_BASE_STYLES}" }
        style { "{MOBILE_TAB_BAR_STYLES}" }
        style { "{STUDIO_STYLE}" }

        div {
            class: "studio-container",
            onmousemove: handle_mouse_move,
            onmouseup: handle_mouse_up,
            onmouseleave: handle_mouse_up,
            ontouchstart: handle_touch_start,
            ontouchend: handle_touch_end,

            MainNav { active: ActivePage::Studio }

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
                    ModeToggle {
                        mode: current_mode,
                        on_change: move |new_mode| {
                            mode.set(new_mode);
                            active_tab.set(MobileTab::Panel1);
                        },
                    }
                }
                div { class: "studio-toolbar-right",
                    if current_mode == StudioMode::Code {
                        button {
                            class: "execute-btn",
                            onclick: handle_code_run,
                            "\u{25B6} Run"
                        }
                        button {
                            class: "execute-btn",
                            style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%); margin-left: 8px;",
                            onclick: handle_code_compile,
                            "\u{2699} Compile"
                        }
                    }
                    if current_mode == StudioMode::Math {
                        button {
                            class: "execute-btn",
                            onclick: handle_math_execute,
                            "\u{25B6} Execute"
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
                if *sidebar_open.read() {
                    FileBrowser {
                        tree: file_tree.read().clone(),
                        selected_path: current_file.read().clone(),
                        collapsed: false,
                        on_select: EventHandler::new(move |path: String| {
                            current_file.set(Some(path.clone()));

                            // Load file content from VFS
                            #[cfg(target_arch = "wasm32")]
                            {
                                let path_clone = path.clone();
                                spawn(async move {
                                    if let Ok(vfs) = get_platform_vfs().await {
                                        if let Ok(content) = vfs.read_to_string(&path_clone).await {
                                            // Load into appropriate editor based on file path/extension
                                            // Math files are .logos but in /examples/math/ directory
                                            let ext = path_clone.rsplit('.').next().unwrap_or("").to_lowercase();
                                            let is_math_dir = path_clone.contains("/math/") || path_clone.contains("/examples/math");

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
                                                            for premise in theorem_result.premises {
                                                                knowledge_base.write().push(premise);
                                                            }

                                                            // Set goal for proof engine
                                                            if let Some(goal) = theorem_result.goal {
                                                                current_proof_expr.set(Some(goal));
                                                            }

                                                            // If derivation was found (auto-proved), show it
                                                            if let Some(derivation) = theorem_result.derivation {
                                                                let tree_html = format_derivation_html(&derivation);
                                                                proof_text.set(tree_html);
                                                                proof_status.set(ProofStatus::Success);
                                                                proof_hint.set(Some(format!("Theorem '{}' proved!", theorem_result.name)));
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
                                                let interp_result = interpret_for_ui(&content).await;
                                                interpreter_result.set(interp_result);
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
                                                        let interp_result = interpret_for_ui(&content).await;
                                                        interpreter_result.set(interp_result);
                                                    }
                                                    StudioMode::Math => math_input.set(content),
                                                }
                                            }
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
                        on_collapse_toggle: EventHandler::new(move |_: ()| {
                            sidebar_open.set(false);
                        }),
                    }
                }

                // Panels - content changes based on mode
                main { class: "studio-main",
                    // Panel 1
                    section {
                        class: "{panel1_class}",
                        style: "width: {left_w}%;",

                        match current_mode {
                            StudioMode::Logic => rsx! {
                                div { class: "panel-header",
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
                                div { class: "panel-header",
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
                                div { class: "panel-header",
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
                        }
                    }

                    // Left resizer
                    div {
                        class: if resizing.read().is_some() { "panel-resizer active" } else { "panel-resizer" },
                        onmousedown: move |_| resizing.set(Some("left")),
                    }

                    // Panel 2
                    section {
                        class: "{panel2_class}",
                        style: "width: {center_w}%;",

                        match current_mode {
                            StudioMode::Logic => rsx! {
                                div { class: "panel-header",
                                    span { "Logic Output" }
                                    div { class: "format-toggle",
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
                                div { class: "panel-content",
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
                            },
                            StudioMode::Code => rsx! {
                                div { class: "panel-header",
                                    span { "Output" }
                                    div { class: "output-mode-toggle",
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
                                div { class: "panel-header",
                                    span { "Output" }
                                }
                                div { class: "panel-content",
                                    ReplOutput {
                                        lines: math_output.read().clone(),
                                        on_clear: move |_| {
                                            math_output.write().clear();
                                            math_repl.set(Repl::new());  // Reset kernel state too
                                        },
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
                            }
                        }
                    }
                }
            }

            // Floating vocab reference button
            VocabReference {}
        }
    }
}
