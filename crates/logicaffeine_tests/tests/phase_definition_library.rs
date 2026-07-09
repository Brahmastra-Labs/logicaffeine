//! Rung 0b seed — a multi-block `## Define` document forms a definition library
//! with a `uses` dependency graph. A `## Define` chain (`widget` uses `gizmo`,
//! `gizmo` uses primitives) verifies end-to-end, and the recorded graph carries
//! the def→def and theorem→def edges that a `mathscrapes` node/edge compiles into.

use logicaffeine_compile::{theorem_dependency_graph, verify_theorem};

/// Task #7 — an English QUANTIFIED definition over a verb (event semantics).
/// `celebrity(x) :↔ every person admires x` lowers the definiens to the
/// categorical form `∀y. person(y) → admires(y, x)` (a neo-Davidsonian event,
/// abstracted to a predicate). From "Bob is a celebrity" and "Alice is a person"
/// we instantiate at Alice and derive "Alice admires Bob" — universal definiens
/// + event-structure matching, end-to-end in English.
#[test]
fn english_quantified_definition_instantiates() {
    let input = r#"
## Define
x is a celebrity if and only if every person admires x.

## Theorem: Alice_Admires_Bob
Given: Bob is a celebrity.
Given: Alice is a person.
Prove: Alice admires Bob.
Proof: Auto.
"#;

    let result = verify_theorem(input);
    assert!(
        result.is_ok(),
        "Alice admires Bob should follow from Bob being a celebrity: {:?}",
        result.err()
    );
}

/// Rung 0c — a STRUCTURE as a bundle of axioms, *inherited*. Define a preorder as
/// reflexive + transitive; then from "R is a preorder" derive an inherited
/// property ("R is transitive"). This is the "instantiate once, inherit the
/// theory" mechanism — the structure unfolds and its axioms are available by
/// ∧-elimination.
#[test]
fn structure_inherits_a_property() {
    let input = r#"
## Define
x is a preorder if and only if x is reflexive and x is transitive.

## Theorem: Preorders_Are_Transitive
Given: R is a preorder.
Prove: R is transitive.
Proof: Auto.
"#;

    let result = verify_theorem(input);
    assert!(
        result.is_ok(),
        "a preorder should inherit transitivity from its definition: {:?}",
        result.err()
    );
}

#[test]
fn definition_chain_verifies_and_records_uses_edges() {
    let input = r#"
## Define
x is a gizmo if and only if x is shiny and x is round.

## Define
x is a widget if and only if x is a gizmo and x is shiny.

## Theorem: Pat_Is_A_Widget
Given: Pat is shiny.
Given: Pat is round.
Prove: Pat is a widget.
Proof: Auto.
"#;

    // Verifies end-to-end: `widget` unfolds through `gizmo` down to primitives.
    let result = verify_theorem(input);
    assert!(
        result.is_ok(),
        "widget(Pat) should verify through the definition chain: {:?}",
        result.err()
    );

    // The dependency graph records the `uses` edges.
    let graph = theorem_dependency_graph(input).expect("dependency graph");
    let uses_of = |name: &str| -> Vec<String> {
        graph
            .def_uses
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, u)| u.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        uses_of("widget"),
        vec!["gizmo".to_string()],
        "widget uses gizmo"
    );
    assert!(
        uses_of("gizmo").is_empty(),
        "gizmo uses only primitives"
    );
    assert_eq!(
        graph.theorem_uses,
        vec!["widget".to_string()],
        "the theorem uses widget"
    );
}
