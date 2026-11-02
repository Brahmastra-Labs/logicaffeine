//! The Studio's Syntax Tree is written for a reader. Every logic expression gets a node in
//! the same notation the Logic Output prints — an event names its verb and hangs its thematic
//! roles beneath it — and no node is ever a Rust `Debug` dump of the engine's internals.

use logicaffeine_compile::{compile_for_ui, AstNode};

fn nodes(node: &AstNode) -> Vec<&AstNode> {
    let mut all = vec![node];
    for child in &node.children {
        all.extend(nodes(child));
    }
    all
}

fn tree(sentence: &str) -> AstNode {
    compile_for_ui(sentence).ast.unwrap_or_else(|| panic!("{sentence:?} has no syntax tree"))
}

fn is_raw_debug(label: &str) -> bool {
    ["Data {", "Symbol(", ": [(", "suppress_existential", "Variable("].iter().any(|raw| label.contains(raw))
}

#[test]
fn an_event_names_its_verb_and_its_roles() {
    let root = tree("Every student read a book.");
    let all = nodes(&root);
    let event = all
        .iter()
        .find(|n| n.node_type == "event")
        .unwrap_or_else(|| panic!("no event node in {root:#?}"));
    assert!(event.label.to_lowercase().contains("read"), "the event is labelled {:?}", event.label);
    let roles: Vec<&str> = event.children.iter().map(|c| c.label.as_str()).collect();
    assert!(roles.contains(&"Agent") && roles.contains(&"Theme"), "the event's roles are {roles:?}");
}

#[test]
fn no_node_is_a_debug_dump() {
    for sentence in [
        "Every student read a book.",
        "John was running.",
        "Mary has eaten.",
        "The train will arrive.",
        "All students must study.",
        "Few birds can fly.",
        "John loves Mary.",
        "Only John runs.",
        "If it rains, the ground is wet.",
        "The book was read by John.",
    ] {
        let root = tree(sentence);
        for node in nodes(&root) {
            assert!(!is_raw_debug(&node.label), "{sentence:?} shows a debug dump: {:?}", node.label);
        }
    }
}
