use super::*;
use crate::ui::examples::ALL_LOGIC_EXAMPLES;

const ALL_SWATCHES: [Swatch; 8] = [
    Swatch::Quantifier,
    Swatch::Connective,
    Swatch::Predicate,
    Swatch::Individual,
    Swatch::Role,
    Swatch::Variable,
    Swatch::Modal,
    Swatch::Tense,
];

/// Every kind `logicaffeine_compile::ui_bridge::expr_to_ast_node` names today.
const COMPILER_KINDS: [&str; 38] = [
    "predicate", "identity", "metaphor", "quantifier", "categorical", "relation", "modal", "temporal", "aspect",
    "voice", "binary_op", "unary_op", "question", "atom", "lambda", "application", "intensional", "modifier", "event",
    "role", "speech_act", "implicature", "conditional", "causal", "concessive", "comparative", "superlative",
    "scopal", "control", "presupposition", "focus", "distributive", "constant", "variable", "function", "group",
    "proposition", "term",
];

fn kinds(node: &AstNode, out: &mut Vec<String>) {
    out.push(node.node_type.clone());
    for child in &node.children {
        kinds(child, out);
    }
}

#[test]
fn every_kind_the_compiler_names_has_a_look() {
    let missing: Vec<&str> = COMPILER_KINDS.iter().copied().filter(|k| kind_look(k).is_none()).collect();
    assert!(missing.is_empty(), "no pill look for {missing:?}");
}

#[test]
fn every_kind_a_logic_example_shows_has_a_look() {
    let mut shown = Vec::new();
    for spec in ALL_LOGIC_EXAMPLES {
        if let Some(ast) = logicaffeine_compile::compile_for_ui(spec.source).ast {
            kinds(&ast, &mut shown);
        }
    }
    assert!(!shown.is_empty(), "no Logic example drew a tree");
    shown.sort();
    shown.dedup();
    let missing: Vec<&String> = shown.iter().filter(|k| kind_look(k).is_none()).collect();
    assert!(missing.is_empty(), "the examples show kinds with no pill look: {missing:?}");
}

/// A pill says what the node is in the reader's words, the Symbol Dictionary's where it has one.
#[test]
fn a_pill_says_the_readers_word() {
    for (kind, word) in [("binary_op", "connective"), ("unary_op", "connective"), ("constant", "individual"), ("temporal", "tense"), ("speech_act", "speech act"), ("quantifier", "quantifier"), ("predicate", "predicate")] {
        assert_eq!(kind_look(kind).map(|l| l.word), Some(word), "the {kind} pill");
    }
    assert!(COMPILER_KINDS.iter().filter_map(|k| kind_look(k)).all(|l| !l.word.contains('_')), "a pill shows a program's name for its kind");
}

/// The legend names every colour a pill can wear: each swatch sits in exactly one of its groups.
#[test]
fn the_legend_covers_every_swatch_once() {
    for swatch in ALL_SWATCHES {
        let groups = LEGEND.iter().filter(|(_, swatches)| swatches.contains(&swatch)).count();
        assert_eq!(groups, 1, "{swatch:?} is in {groups} legend groups");
    }
    assert!(LEGEND.iter().all(|(label, swatches)| !label.is_empty() && !swatches.is_empty()), "a legend group is empty");
}

fn leaf(label: &str, kind: &str) -> AstNode {
    AstNode { label: label.to_string(), node_type: kind.to_string(), children: Vec::new() }
}

fn render(ast: AstNode) -> String {
    #[derive(Props, Clone, PartialEq)]
    struct HostProps {
        ast: AstNode,
    }
    #[allow(non_snake_case)]
    fn Host(props: HostProps) -> Element {
        rsx! { AstTree { ast: props.ast } }
    }
    let mut vdom = VirtualDom::new_with_props(Host, HostProps { ast });
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

fn sentence() -> AstNode {
    AstNode {
        label: "∀x".to_string(),
        node_type: "quantifier".to_string(),
        children: vec![AstNode {
            label: "→".to_string(),
            node_type: "binary_op".to_string(),
            children: vec![leaf("Dog(x)", "predicate"), leaf("Bark(x)", "predicate")],
        }],
    }
}

#[test]
fn the_tree_says_how_to_read_it_above_itself() {
    let html = render(sentence());
    let note = html.find("whole sentence").unwrap_or_else(|| panic!("the tree says nothing about how to read it:\n{html}"));
    let tree = html.find(r#"class="syntax-tree" data-tree="syntax-tree""#).unwrap_or_else(|| panic!("the Syntax Tree is not the shared tree:\n{html}"));
    assert!(note < tree, "the note is not above the tree:\n{html}");
    for (label, _) in LEGEND {
        assert!(html[..tree].contains(label), "the legend leaves out {label:?}:\n{html}");
    }
}

#[test]
fn every_level_opens_by_default() {
    let html = render(sentence());
    for label in ["∀x", "→", "Dog(x)", "Bark(x)"] {
        assert!(html.contains(&format!(">{label}<")), "the tree hides {label:?} when it opens:\n{html}");
    }
    assert!(!html.contains(r#"class="chevron""#), "a node starts closed:\n{html}");
}

#[test]
fn a_row_is_its_label_then_its_kind() {
    let html = render(sentence());
    let label = html.find(">→<").unwrap_or_else(|| panic!("no row reads →:\n{html}"));
    let pill = html[label..].find(r#"class="syntax-kind connective""#).map(|at| label + at).unwrap_or_else(|| panic!("the connective wears no connective pill after its label:\n{html}"));
    let text = html[pill..].split('>').nth(1).and_then(|rest| rest.split('<').next()).unwrap_or("");
    assert_eq!(text, "connective", "the connective's pill says {text:?}");
}
