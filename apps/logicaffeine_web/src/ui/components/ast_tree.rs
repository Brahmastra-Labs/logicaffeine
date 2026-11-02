//! The Syntax Tree: how a sentence parses, drawn with the Studio's tree.
//!
//! The top row is the whole sentence as parsed, and each row below splits its phrase into its
//! parts. A row is a node's label — the formula piece it stands for, in the notation the Logic
//! Output prints — then a pill naming its kind, in the reader's word and the colour of the one
//! syntax palette (`--syntax-*`). A short note above the tree says how to read it, with a legend
//! of the pill colours. Every level opens as the tree opens; a node's row folds its parts away.

use crate::ui::components::tree::{Tree, TreeNode};
use crate::AstNode;
use dioxus::prelude::*;

const SYNTAX_TREE_STYLE: &str = r#"
/* How to read the tree, quietly above it: a line of prose, then the pill colours. */
.syntax-tree-guide {
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--studio-text-secondary);
}

.syntax-tree-note {
    margin: 0;
}

.syntax-tree-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
}

/* The tree runs the column's whole width, so its first line stands one step from the column's
   edge, and a tree wider than the column scrolls sideways across it. */
.syntax-tree {
    --tree-pad-top: 12px;
    --tree-pad-bottom: 0px;
    margin: 0 calc(-1 * var(--studio-inset, 20px));
    overflow-x: auto;
}

.syntax-label {
    flex: none;
    margin-right: 8px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 13px;
    color: var(--studio-text);
    white-space: nowrap;
}

/* A kind's pill: its word in its swatch, on a tint of the same. */
.syntax-kind {
    flex: none;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 13px;
    line-height: 16px;
    white-space: nowrap;
    color: var(--kind, var(--studio-text-secondary));
    background: color-mix(in srgb, var(--kind, var(--studio-text-secondary)) 18%, transparent);
}

.syntax-kind.quantifier { --kind: var(--syntax-quantifier); }
.syntax-kind.connective { --kind: var(--syntax-connective); }
.syntax-kind.predicate { --kind: var(--syntax-predicate); }
.syntax-kind.individual { --kind: var(--syntax-individual); }
.syntax-kind.role { --kind: var(--syntax-role); }
.syntax-kind.variable { --kind: var(--syntax-variable); }
.syntax-kind.modal { --kind: var(--syntax-modal); }
.syntax-kind.tense { --kind: var(--syntax-tense); }
"#;

/// A swatch of the one syntax palette (`--syntax-*` in the Studio's tokens).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swatch {
    Quantifier,
    Connective,
    Predicate,
    Individual,
    Role,
    Variable,
    Modal,
    Tense,
}

impl Swatch {
    /// Its name in the palette: the pill's class and its token, `--syntax-{name}`.
    pub fn name(self) -> &'static str {
        match self {
            Swatch::Quantifier => "quantifier",
            Swatch::Connective => "connective",
            Swatch::Predicate => "predicate",
            Swatch::Individual => "individual",
            Swatch::Role => "role",
            Swatch::Variable => "variable",
            Swatch::Modal => "modal",
            Swatch::Tense => "tense",
        }
    }
}

/// A node kind as a reader sees it: the word its pill says and the swatch it wears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindLook {
    pub word: &'static str,
    pub swatch: Swatch,
}

/// The look of each kind the compiler names a node (`ui_bridge::expr_to_ast_node`), in the words
/// the Symbol Dictionary uses where it has one.
pub fn kind_look(node_type: &str) -> Option<KindLook> {
    use Swatch::*;
    let (word, swatch) = match node_type {
        "quantifier" => ("quantifier", Quantifier),
        "question" => ("question", Quantifier),
        "binary_op" | "unary_op" => ("connective", Connective),
        "identity" => ("identity", Connective),
        "conditional" => ("conditional", Connective),
        "causal" => ("causal", Connective),
        "concessive" => ("concessive", Connective),
        "lambda" => ("lambda", Connective),
        "application" => ("application", Connective),
        "speech_act" => ("speech act", Connective),
        "implicature" => ("implicature", Connective),
        "presupposition" => ("presupposition", Connective),
        "focus" => ("focus", Connective),
        "scopal" => ("scopal", Connective),
        "distributive" => ("distributive", Connective),
        "predicate" => ("predicate", Predicate),
        "event" => ("event", Predicate),
        "relation" => ("relation", Predicate),
        "atom" => ("atom", Predicate),
        "categorical" => ("categorical", Predicate),
        "comparative" => ("comparative", Predicate),
        "superlative" => ("superlative", Predicate),
        "metaphor" => ("metaphor", Predicate),
        "modifier" => ("modifier", Predicate),
        "control" => ("control", Predicate),
        "constant" => ("individual", Individual),
        "term" => ("term", Individual),
        "function" => ("function", Individual),
        "group" => ("group", Individual),
        "proposition" => ("proposition", Individual),
        "role" => ("role", Role),
        "variable" => ("variable", Variable),
        "modal" => ("modal", Modal),
        "intensional" => ("intensional", Modal),
        "temporal" => ("tense", Tense),
        "aspect" => ("aspect", Tense),
        "voice" => ("voice", Tense),
        _ => return None,
    };
    Some(KindLook { word, swatch })
}

/// The legend's groups: the kinds each pill colour marks, and the swatches that wear it. The
/// swatches of one group share a colour in the palette.
pub const LEGEND: &[(&str, &[Swatch])] = &[
    (
        "quantifier · connective",
        &[Swatch::Quantifier, Swatch::Connective],
    ),
    ("predicate · event", &[Swatch::Predicate]),
    ("individual · role", &[Swatch::Individual, Swatch::Role]),
    ("variable", &[Swatch::Variable]),
    ("modal · tense", &[Swatch::Modal, Swatch::Tense]),
];

#[component]
pub fn AstTree(ast: AstNode) -> Element {
    rsx! {
        style { "{SYNTAX_TREE_STYLE}" }
        div { class: "syntax-tree-guide",
            p { class: "syntax-tree-note",
                "Read it from the top: the first row is the whole sentence as parsed, and each row below splits its phrase into its parts. Press a row with a chevron to fold its parts away."
            }
            div { class: "syntax-tree-legend",
                for (label, swatches) in LEGEND.iter() {
                    span { key: "{label}", class: "syntax-kind {swatches[0].name()}", "{label}" }
                }
            }
        }
        Tree { kind: "syntax-tree", label: "Syntax Tree",
            SyntaxNode { node: ast, depth: 0, last: true }
        }
    }
}

/// One node of the sentence and its parts. Every level opens as the tree opens.
#[component]
fn SyntaxNode(node: AstNode, depth: usize, last: bool) -> Element {
    let mut open = use_signal(|| true);
    let expandable = !node.children.is_empty();
    let (word, swatch) = match kind_look(&node.node_type) {
        Some(look) => (look.word.to_string(), look.swatch.name()),
        None => (node.node_type.replace('_', " "), ""),
    };
    let count = node.children.len();

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
                span { class: "syntax-label", "{node.label}" }
                span { class: "syntax-kind {swatch}", "{word}" }
            },
            for (i, child) in node.children.iter().enumerate() {
                SyntaxNode { key: "{i}", node: child.clone(), depth: depth + 1, last: i + 1 == count }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/ast_tree/tests.rs"]
mod tests;
