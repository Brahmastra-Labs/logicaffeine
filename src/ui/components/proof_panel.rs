//! Proof panel for Logic mode - displays derivation trees and tactics.

use dioxus::prelude::*;

const PROOF_PANEL_STYLE: &str = r#"
.proof-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #0f1419;
    font-family: 'SF Mono', 'Fira Code', monospace;
}

.proof-header {
    padding: 12px 16px;
    background: rgba(255, 255, 255, 0.02);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    align-items: center;
    gap: 12px;
}

.proof-status {
    font-size: 13px;
    color: rgba(255, 255, 255, 0.7);
}

.proof-status.success {
    color: #98c379;
}

.proof-status.error {
    color: #e06c75;
}

.proof-status.pending {
    color: #e5c07b;
}

/* Tactics bar */
.tactics-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 12px 16px;
    background: rgba(255, 255, 255, 0.02);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.tactic-btn {
    padding: 6px 12px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 4px;
    color: rgba(255, 255, 255, 0.8);
    font-size: 12px;
    cursor: pointer;
    transition: all 0.15s ease;
}

.tactic-btn:hover {
    background: rgba(102, 126, 234, 0.15);
    border-color: rgba(102, 126, 234, 0.3);
    color: #667eea;
}

.tactic-btn.active {
    background: rgba(102, 126, 234, 0.2);
    border-color: #667eea;
    color: #667eea;
}

/* Derivation tree */
.derivation-tree {
    flex: 1;
    overflow: auto;
    padding: 16px;
    white-space: pre-wrap;
    font-size: 13px;
    line-height: 1.8;
    color: #e8eaed;
}

.derivation-tree .rule {
    color: #667eea;
    font-weight: 600;
}

.derivation-tree .conclusion {
    color: #98c379;
}

/* Socratic hint */
.socratic-hint {
    padding: 12px 16px;
    background: rgba(229, 192, 123, 0.1);
    border-top: 1px solid rgba(229, 192, 123, 0.2);
    color: #e5c07b;
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 8px;
}

.hint-icon {
    font-size: 16px;
}

/* Empty state */
.proof-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    color: rgba(255, 255, 255, 0.4);
    text-align: center;
    padding: 40px 20px;
}

.proof-empty-icon {
    font-size: 48px;
    margin-bottom: 16px;
    opacity: 0.5;
}

.proof-empty-text {
    font-size: 14px;
    line-height: 1.6;
}

/* Mobile */
@media (max-width: 768px) {
    .tactics-bar {
        padding: 10px 12px;
        gap: 4px;
    }

    .tactic-btn {
        padding: 8px 10px;
        font-size: 11px;
    }

    .derivation-tree {
        padding: 12px;
        font-size: 12px;
    }
}
"#;

/// Available proof tactics for the UI
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tactic {
    Auto,
    ModusPonens,
    UniversalInst,
    ExistentialIntro,
    Induction,
    Rewrite,
}

impl Tactic {
    pub fn label(&self) -> &'static str {
        match self {
            Tactic::Auto => "Auto",
            Tactic::ModusPonens => "Modus Ponens",
            Tactic::UniversalInst => "\u{2200} Elim",
            Tactic::ExistentialIntro => "\u{2203} Intro",
            Tactic::Induction => "Induction",
            Tactic::Rewrite => "Rewrite",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Tactic::Auto => "Let the prover find a proof automatically",
            Tactic::ModusPonens => "From P\u{2192}Q and P, derive Q",
            Tactic::UniversalInst => "From \u{2200}x.P(x), derive P(c) for some c",
            Tactic::ExistentialIntro => "From P(c), derive \u{2203}x.P(x)",
            Tactic::Induction => "Prove by structural induction on Nat",
            Tactic::Rewrite => "Use an equality to substitute terms",
        }
    }
}

/// Proof status for the UI
#[derive(Clone, PartialEq, Default)]
pub enum ProofStatus {
    #[default]
    Idle,
    Proving,
    Success,
    Failed(String),
}

/// Proof panel component - displays proof status, tactics, and derivation tree
#[component]
pub fn ProofPanel(
    /// The derivation tree as a formatted string (use DerivationTree::display_tree())
    proof_text: String,
    /// Current proof status
    status: ProofStatus,
    /// Optional Socratic hint
    hint: Option<String>,
    /// Callback when a tactic button is clicked
    on_tactic: EventHandler<Tactic>,
) -> Element {
    let has_proof = !proof_text.is_empty();

    let status_class = match &status {
        ProofStatus::Idle => "proof-status",
        ProofStatus::Proving => "proof-status pending",
        ProofStatus::Success => "proof-status success",
        ProofStatus::Failed(_) => "proof-status error",
    };

    let status_text = match &status {
        ProofStatus::Idle => "Ready to prove".to_string(),
        ProofStatus::Proving => "Searching for proof...".to_string(),
        ProofStatus::Success => "\u{2713} Proof found".to_string(),
        ProofStatus::Failed(msg) => format!("\u{2717} {}", msg),
    };

    rsx! {
        style { "{PROOF_PANEL_STYLE}" }

        div { class: "proof-panel",
            // Status header
            div { class: "proof-header",
                span { class: "{status_class}", "{status_text}" }
            }

            // Tactics bar
            div { class: "tactics-bar",
                for tactic in [Tactic::Auto, Tactic::ModusPonens, Tactic::UniversalInst, Tactic::ExistentialIntro, Tactic::Induction, Tactic::Rewrite] {
                    button {
                        class: "tactic-btn",
                        title: "{tactic.description()}",
                        onclick: {
                            let t = tactic;
                            move |_| on_tactic.call(t)
                        },
                        "{tactic.label()}"
                    }
                }
            }

            // Derivation tree or empty state
            if has_proof {
                div { class: "derivation-tree",
                    dangerous_inner_html: "{proof_text}",
                }
            } else {
                div { class: "proof-empty",
                    div { class: "proof-empty-icon", "\u{1F50D}" }
                    div { class: "proof-empty-text",
                        "Enter a logical formula to prove."
                        br {}
                        "Click 'Auto' to search for a proof automatically,"
                        br {}
                        "or use specific tactics to guide the proof."
                    }
                }
            }

            // Socratic hint
            if let Some(hint_text) = &hint {
                div { class: "socratic-hint",
                    span { class: "hint-icon", "\u{1F4A1}" }
                    span { "{hint_text}" }
                }
            }
        }
    }
}
