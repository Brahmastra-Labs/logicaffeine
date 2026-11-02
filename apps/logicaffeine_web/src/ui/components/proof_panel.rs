//! The proof drawer for Logic mode: the Studio's bottom drawer, risen from the dock bar.
//!
//! The drawer docks below the panel row, on the Studio page and in a frame alike, and is dragged
//! by the grip on its top edge the way the debugger's drawer is. Its bar names it and says how the
//! proof stands ([`ProofStatus`]); its body is the derivation once there is one, drawn by the
//! Studio's own tree (`components::tree`, the Files tree's and the Syntax Tree's): each step a row,
//! its rule in a pill and its conclusion beside it, its premises its parts, down to the givens
//! ([`ProofNode`]). Until there is one, the walkthrough: the three steps a first visit takes, each
//! marked done, next, or still to come ([`Walkthrough`]). The one action is Prove, in the dock bar
//! and nowhere else: the prover searches on its own (`auto_view`), so the panel offers no tactic a
//! reader would have to choose.
//!
//! # Props
//!
//! - `proof_text` - Formatted derivation tree HTML
//! - `status` - Current proof state
//! - `steps` - What the reader has done so far, for the walkthrough
//! - `height` - The drawer's height, which its grip changes
//! - `on_prove` - Prove was pressed
//! - `on_close` - The drawer's close was pressed

use dioxus::prelude::*;

use crate::ui::components::tree::{Tree, TreeNode};

pub(crate) const PROOF_PANEL_STYLE: &str = r#"
.proof-drawer {
    display: flex;
    flex-direction: column;
    flex: 0 1 auto;
    min-height: 44px;
    overflow: hidden;
    background: var(--studio-panel-bg, #12161c);
    border-top: 1px solid var(--studio-border, rgba(255, 255, 255, 0.08));
    color: var(--studio-text, #e8eaed);
    font-size: 13px;
}

/* The grab bar is the column bars turned on their side: solid 8px, in their grey. */
.proof-grip {
    flex: none;
    height: 8px;
    background: var(--studio-resizer, #22272b);
    cursor: row-resize;
    touch-action: none;
    transition: background 0.2s ease;
}

.proof-grip:hover,
.proof-grip.active {
    background: var(--studio-accent, #667eea);
}

/* Under a finger the grip takes a finger: its grab area reaches 18px past what it draws. */
@media (pointer: coarse) {
    .proof-grip { position: relative; }
    .proof-grip::before { content: ""; position: absolute; left: 0; right: 0; top: -18px; bottom: -18px; z-index: 1; }
}

.proof-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 36px;
    padding: 4px 12px;
    border-bottom: 1px solid var(--studio-border, rgba(255, 255, 255, 0.08));
}

.proof-title {
    font-weight: 600;
}

.proof-status {
    font-size: 13px;
    color: rgba(255, 255, 255, 0.7);
}

.proof-status.success { color: #98c379; }
.proof-status.error { color: #e06c75; }
.proof-status.pending { color: #e5c07b; }
.proof-status.uncertified { color: #e5c07b; }

.proof-bar-space { flex: 1 1 auto; }

.proof-close {
    padding: 3px 8px;
    border: 1px solid var(--studio-border, rgba(255, 255, 255, 0.08));
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.05);
    color: var(--studio-text, #e8eaed);
    font-size: 13px;
    line-height: 1;
    cursor: pointer;
}

.proof-close:hover { background: rgba(255, 255, 255, 0.1); }

.proof-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
}

/* The derivation, as the Studio's trees are drawn: a note on how to read it, then the rows. The
   tree runs the drawer's whole width, so its first line stands one step from the edge. */
.proof-derivation {
    padding: 12px var(--studio-inset, 16px) 0;
}

.proof-tree-note {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--studio-text-secondary, rgba(255, 255, 255, 0.6));
}

.proof-tree {
    --tree-pad-top: 10px;
    --tree-pad-bottom: 12px;
    margin: 0 calc(-1 * var(--studio-inset, 16px));
    overflow-x: auto;
}

/* A step's rule, a pill as the Syntax Tree's kinds are: the word on a tint of its colour. */
.proof-rule {
    flex: none;
    margin-right: 8px;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 13px;
    line-height: 16px;
    white-space: nowrap;
    color: var(--studio-accent, #667eea);
    background: color-mix(in srgb, var(--studio-accent, #667eea) 18%, transparent);
}

.proof-rule.given {
    color: #98c379;
    background: color-mix(in srgb, #98c379 18%, transparent);
}

.proof-rule-detail {
    flex: none;
    margin-right: 8px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 13px;
    color: var(--studio-text-secondary, rgba(255, 255, 255, 0.6));
    white-space: nowrap;
}

.proof-conclusion {
    flex: none;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 13px;
    color: var(--studio-text, #e8eaed);
    white-space: nowrap;
}

/* What else a proof shows: a grid's answer, or why a search failed, as the prover wrote it. */
.derivation-tree {
    padding: var(--studio-inset, 16px);
    white-space: pre-wrap;
    font-family: 'SF Mono', 'Fira Code', monospace;
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

/* The walkthrough: the three steps across the drawer, each what to do and what it shows. The
   step to take now is lit in the dock bar's amber; a step taken is ticked. */
.proof-steps {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
    margin: 0;
    padding: 14px 16px;
    list-style: none;
}

.proof-step {
    display: flex;
    gap: 10px;
    min-width: 0;
    padding: 10px 12px;
    border: 1px solid var(--studio-border, rgba(255, 255, 255, 0.08));
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.02);
}

.proof-step.now {
    border-color: rgba(245, 158, 11, 0.6);
    background: rgba(245, 158, 11, 0.08);
}

.proof-step.done { opacity: 0.75; }

.proof-step-n {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.08);
    font-size: 12px;
    font-weight: 700;
}

.proof-step.now .proof-step-n { background: #f59e0b; color: #111; }
.proof-step.done .proof-step-n { background: #22c55e; color: #111; }

.proof-step-title {
    margin-bottom: 2px;
    font-weight: 600;
}

.proof-step-line {
    color: var(--studio-text-secondary, rgba(255, 255, 255, 0.6));
    line-height: 1.45;
}

@media (max-width: 768px) {
    .proof-steps { grid-template-columns: minmax(0, 1fr); }
    .derivation-tree { padding: 12px; font-size: 12px; }
}
"#;

/// One step of a derivation, as the drawer draws it: the rule's word, what the rule was applied
/// with (the term a universal was instantiated at, a witness), the conclusion it reached, and the
/// steps it rests on.
#[derive(Clone, Debug, PartialEq)]
pub struct ProofNode {
    pub rule: String,
    pub detail: Option<String>,
    pub conclusion: String,
    pub premises: Vec<ProofNode>,
}

impl ProofNode {
    /// A premise taken as given: the leaf a derivation rests on.
    pub fn is_given(&self) -> bool {
        self.rule == "Given"
    }
}

/// What the note above the derivation says.
pub const DERIVATION_NOTE: &str =
    "Read it from the top: the first row is what was proved, and each row below is a step it rests on, down to the givens. Press a row with a chevron to fold its steps away.";

/// Proof status for the UI
#[derive(Clone, PartialEq, Default)]
pub enum ProofStatus {
    #[default]
    Idle,
    Proving,
    Success,
    /// The prover found a derivation the kernel did not certify — shown, but not a proof.
    /// Carries where verification broke.
    Uncertified(String),
    Failed(String),
}

impl ProofStatus {
    /// The status header's class and text.
    pub fn line(&self) -> (&'static str, String) {
        match self {
            ProofStatus::Idle => ("proof-status", "Ready to prove".to_string()),
            ProofStatus::Proving => ("proof-status pending", "Searching for proof...".to_string()),
            ProofStatus::Success => ("proof-status success", "\u{2713} Proof found".to_string()),
            ProofStatus::Uncertified(_) => ("proof-status uncertified", "\u{26A0} Derivation not certified".to_string()),
            ProofStatus::Failed(msg) => ("proof-status error", format!("\u{2717} {}", msg)),
        }
    }
}

/// The three steps a first visit takes, in order: what to do, and what it shows.
pub const STEPS: [(&str, &str); 3] = [
    ("Write English", "Given lines, then what to prove, one sentence each, as in the example."),
    ("Execute", "Turns every sentence into logic, shown in Logic Output."),
    ("Prove", "Asks the kernel whether it follows from the givens. Plain sentences prove the first one."),
];

/// Where a step stands for this reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    Now,
    Next,
}

impl StepState {
    /// The class a step is painted with.
    pub fn class(self) -> &'static str {
        match self {
            StepState::Done => "proof-step done",
            StepState::Now => "proof-step now",
            StepState::Next => "proof-step",
        }
    }
}

/// What the reader has done so far: written something, and run Execute on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Walkthrough {
    pub wrote: bool,
    pub executed: bool,
}

impl Walkthrough {
    /// Each step's state: the first two follow what was done, and Prove is next once both are.
    pub fn states(self) -> [StepState; 3] {
        let done = [self.wrote, self.wrote && self.executed, false];
        let now = done.iter().position(|d| !d).unwrap_or(STEPS.len());
        std::array::from_fn(|i| {
            if done[i] {
                StepState::Done
            } else if i == now {
                StepState::Now
            } else {
                StepState::Next
            }
        })
    }
}

/// The dock bar's line: the step to take now, or, once Execute has run, what the prover last said
/// (`hint`), else the invitation to prove.
pub fn dock_prompt(steps: Walkthrough, hint: Option<&str>) -> String {
    if !steps.wrote {
        "Write a sentence in English to begin.".to_string()
    } else if !steps.executed {
        "Press Execute to turn your sentences into logic.".to_string()
    } else {
        hint.unwrap_or("Press Prove to check it follows from the givens.").to_string()
    }
}

/// The proof drawer: its bar with the proof's status, and the derivation tree or the walkthrough.
#[component]
pub fn ProofPanel(
    /// What else the proof shows, as HTML: a grid's answer, or why a search failed.
    proof_text: String,
    /// The derivation, once there is one.
    tree: Option<ProofNode>,
    /// Current proof status
    status: ProofStatus,
    /// What the reader has done so far, for the walkthrough
    steps: Walkthrough,
    /// The drawer's height, which its grip changes
    height: Signal<f64>,
    /// Prove was pressed
    on_prove: EventHandler<()>,
    /// The drawer's close was pressed
    on_close: EventHandler<()>,
) -> Element {
    // Dragging the grip on the drawer's top edge paints each frame straight onto the drawer's
    // height and commits it once, on release (see ui::drag). The panel row above gives way.
    let start_grip_drag = move |evt: PointerEvent| {
        #[cfg(target_arch = "wasm32")]
        {
            use crate::ui::pages::studio_layout::proof_dragged;
            use dioxus::html::input_data::MouseButton;
            use wasm_bindgen::JsCast;
            let mut height = height;
            if evt.data().trigger_button().is_some_and(|b| b != MouseButton::Primary) {
                return;
            }
            evt.prevent_default();
            let Some(document) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let find = |selector: &str| document.query_selector(selector).ok().flatten();
            let (Some(grip), Some(drawer), Some(panels)) = (
                find(".proof-grip"),
                find(".proof-drawer").and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok()),
                find(".studio-content"),
            ) else {
                return;
            };
            let start = drawer.get_bounding_client_rect().height();
            let room = start + panels.get_bounding_client_rect().height();
            let point = evt.data().client_coordinates();
            let _ = grip.class_list().add_1("active");
            let released = grip.clone();
            crate::ui::drag::begin(
                grip,
                evt.data().pointer_id(),
                (point.x, point.y),
                crate::ui::drag::Axis::Rows,
                move |_, dy| {
                    let _ = drawer.style().set_property("height", &format!("{}px", proof_dragged(start, dy, room)));
                },
                move |_, dy| {
                    let _ = released.class_list().remove_1("active");
                    height.set(proof_dragged(start, dy, room));
                },
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (evt, height);
    };
    let drawer_style = format!("height: {}px;", height());
    let has_proof = tree.is_some() || !proof_text.is_empty();
    let (status_class, status_text) = status.line();
    let states = steps.states();

    rsx! {
        style { "{PROOF_PANEL_STYLE}" }

        div { class: "proof-drawer", style: "{drawer_style}",
            div { class: "proof-grip", onpointerdown: move |evt| start_grip_drag(evt) }
            div { class: "proof-bar",
                span { class: "proof-title", "Proof" }
                span { class: "{status_class}", "{status_text}" }
                span { class: "proof-bar-space" }
                button { class: "proof-close", title: "Hide the proof", onclick: move |_| on_close.call(()), "\u{2715}" }
            }
            div { class: "proof-body",
                if let Some(root) = tree.clone() {
                    div { class: "proof-derivation",
                        p { class: "proof-tree-note", "{DERIVATION_NOTE}" }
                        Tree { kind: "proof-tree", label: "Derivation",
                            ProofTreeNode { node: root, depth: 0, last: true }
                        }
                    }
                }
                if !proof_text.is_empty() {
                    div { class: "derivation-tree", dangerous_inner_html: "{proof_text}" }
                }
                if !has_proof {
                    ol { class: "proof-steps", "aria-label": "How to prove a sentence",
                        for (i, (title, line)) in STEPS.iter().enumerate() {
                            li { key: "{i}", class: states[i].class(), "aria-current": if states[i] == StepState::Now { "step" } else { "false" },
                                span { class: "proof-step-n", "aria-hidden": "true",
                                    if states[i] == StepState::Done { "\u{2713}" } else { "{i + 1}" }
                                }
                                div {
                                    div { class: "proof-step-title", "{title}" }
                                    div { class: "proof-step-line", "{line}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One step and the steps it rests on. Every level opens as the derivation opens.
#[component]
fn ProofTreeNode(node: ProofNode, depth: usize, last: bool) -> Element {
    let mut open = use_signal(|| true);
    let expandable = !node.premises.is_empty();
    let count = node.premises.len();
    let rule_class = if node.is_given() { "proof-rule given" } else { "proof-rule" };

    rsx! {
        TreeNode {
            depth,
            last,
            expandable,
            open: open(),
            stop: depth == 0,
            on_press: if expandable {
                Some(EventHandler::new(move |_| {
                    let now = open();
                    open.set(!now);
                }))
            } else {
                None
            },
            row: rsx! {
                span { class: "{rule_class}", "{node.rule}" }
                if let Some(detail) = &node.detail {
                    span { class: "proof-rule-detail", "{detail}" }
                }
                span { class: "proof-conclusion", "{node.conclusion}" }
            },
            for (i, premise) in node.premises.iter().enumerate() {
                ProofTreeNode { key: "{i}", node: premise.clone(), depth: depth + 1, last: i + 1 == count }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/proof_panel/status_line_tests.rs"]
mod status_line_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/proof_panel/walkthrough_tests.rs"]
mod walkthrough_tests;
