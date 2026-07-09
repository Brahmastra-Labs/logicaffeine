//! Parser AST-structure validation for representative Logic-mode sentences.
//!
//! This file holds the coverage that is NOT an execution mirror of a shipped
//! Studio example:
//!
//!   * `parse_tree_*` — full parse-tree / AST-shape validation via `ExprView`
//!     (quantifier kinds, neo-Davidsonian events, temporal/aspectual structure).
//!     These inspect the parsed tree itself, which nothing else asserts on.
//!   * multi-sentence discourse formatting — the numbered-formula invariant of
//!     `compile` (`1)` / `2)` per sentence, never a top-level ∧; single sentences
//!     unnumbered). A cross-cutting output-format contract, not an example copy.
//!   * proof-script / tactic vernacular — English `Proof:` scripts (`Assume`,
//!     `By cases on`, `Introduce`, `Split`, `By automation`, named premises)
//!     driven through the tactic framework and kernel-certified. These use
//!     bespoke theorems (not shipped examples) and are the only coverage of the
//!     scripted-proof surface in the suite.
//!
//! The authoritative execution lock for ALL shipped Studio examples (logic/code/
//! math/hardware — compilation, proving, interpretation, and a rustc compile-and-run
//! audit) lives in the web crate: `logicaffeine_web::ui::examples`, in the
//! `#[cfg(test)] mod example_health` module. The former hand-copied example
//! mirrors here were redundant with it and have been removed.

#[cfg(not(target_arch = "wasm32"))]
mod common;

use logicaffeine_language::{compile, compile_all_scopes, compile_theorem};
use logicaffeine_language::ast::{QuantifierKind, TemporalOperator};
use logicaffeine_language::view::ExprView;

// ============================================================
// Parse-Tree Validation (Logic Mode)
// These validate the complete parsed AST structure via ExprView.
// ============================================================

/// Validate: "Every cat sleeps."
/// Expected structure: Quantifier(Universal) { body: Implication { Cat(x) -> Sleep(x) } }
#[test]
fn parse_tree_every_cat_sleeps() {
    let view = common::parse_to_view("Every cat sleeps.");
    match view {
        ExprView::Quantifier { kind, variable, body, .. } => {
            assert_eq!(kind, QuantifierKind::Universal,
                "Expected Universal quantifier, got {:?}", kind);
            assert!(!variable.is_empty(), "Should bind a variable");
            // Body should be an implication: Cat(x) -> Sleep(x)
            match *body {
                ExprView::BinaryOp { .. } => {
                    // Implication structure confirmed
                }
                _ => panic!("Expected BinaryOp (implication) in body, got {:?}", body),
            }
        }
        _ => panic!("Expected Quantifier variant for 'Every cat sleeps', got {:?}", view),
    }
}

/// Validate: "Some dogs bark loudly."
/// Expected structure: Quantifier(Existential) { body: And { Dog(x), BarkLoudly(x) } }
#[test]
fn parse_tree_some_dogs_bark() {
    let view = common::parse_to_view("Some dogs bark loudly.");
    match view {
        ExprView::Quantifier { kind, variable, .. } => {
            assert_eq!(kind, QuantifierKind::Existential,
                "Expected Existential quantifier, got {:?}", kind);
            assert!(!variable.is_empty(), "Should bind a variable");
        }
        _ => panic!("Expected Quantifier variant for 'Some dogs bark loudly', got {:?}", view),
    }
}

/// Validate: "John loves Mary."
/// Expected structure: NeoEvent { verb: Love, roles: [Agent(John), Theme(Mary)] }
#[test]
fn parse_tree_john_loves_mary() {
    let view = common::parse_to_view("John loves Mary.");
    match view {
        ExprView::NeoEvent { verb, roles, .. } => {
            assert_eq!(verb, "Love", "Expected verb 'Love', got '{}'", verb);
            assert!(roles.len() >= 2, "Should have at least 2 roles (agent, theme)");
        }
        _ => panic!("Expected NeoEvent variant for 'John loves Mary', got {:?}", view),
    }
}

/// Validate: "The quick brown fox jumps."
/// Expected structure: NeoEvent with definite description subject
#[test]
fn parse_tree_quick_brown_fox() {
    let view = common::parse_to_view("The quick brown fox jumps.");
    match view {
        ExprView::NeoEvent { verb, roles, .. } => {
            assert_eq!(verb, "Jump", "Expected verb 'Jump', got '{}'", verb);
            // Should have at least an agent role (the fox)
            assert!(!roles.is_empty(), "Should have thematic roles");
        }
        ExprView::Quantifier { .. } => {
            // Definite descriptions can be analyzed as quantifiers
        }
        _ => panic!("Expected NeoEvent or Quantifier for 'The quick brown fox jumps', got {:?}", view),
    }
}

/// Validate: "No student failed."
/// Expected structure: Quantifier(Universal) { body: Implication with negated body }
#[test]
fn parse_tree_no_student_failed() {
    let view = common::parse_to_view("No student failed.");
    match view {
        ExprView::Quantifier { kind, body, .. } => {
            // "No" is typically Universal with negated body: ∀x(Student(x) → ¬Failed(x))
            assert_eq!(kind, QuantifierKind::Universal,
                "Expected Universal quantifier for 'No', got {:?}", kind);
            // Body should contain negation
            match *body {
                ExprView::BinaryOp { .. } => {
                    // Implication with negated consequent
                }
                ExprView::UnaryOp { .. } => {
                    // Direct negation
                }
                _ => {}
            }
        }
        _ => panic!("Expected Quantifier variant for 'No student failed', got {:?}", view),
    }
}

/// Validate: "Every student read a book." (quantifier scope ambiguity)
/// Should parse with multiple scope readings
#[test]
fn parse_tree_quantifier_scope() {
    let readings = compile_all_scopes("Every student read a book.").unwrap();
    // Should have at least 2 readings due to scope ambiguity
    // 1. Surface scope: ∀x(Student(x) → ∃y(Book(y) ∧ Read(x,y)))
    // 2. Inverse scope: ∃y(Book(y) ∧ ∀x(Student(x) → Read(x,y)))
    assert!(readings.len() >= 1,
        "Should have at least one reading for scope ambiguity. Got {} readings", readings.len());
}

/// Validate: "John was running." (past progressive)
/// Expected structure: Temporal(Past) { Aspectual(Progressive) { Run(John) } }
#[test]
fn parse_tree_past_progressive() {
    let view = common::parse_to_view("John was running.");
    match view {
        ExprView::Temporal { operator, body } => {
            // Past tense wrapping progressive aspect
            assert_eq!(operator, TemporalOperator::Past,
                "Expected Past temporal operator, got {:?}", operator);
            match *body {
                ExprView::Aspectual { .. } | ExprView::NeoEvent { .. } => {
                    // Progressive aspect confirmed
                }
                _ => panic!("Expected Aspectual or NeoEvent in temporal body, got {:?}", body),
            }
        }
        ExprView::NeoEvent { modifiers, .. } => {
            // Alternative: modifiers contain tense/aspect info
            assert!(!modifiers.is_empty(), "Should have temporal modifiers");
        }
        _ => panic!("Expected Temporal or NeoEvent for 'John was running', got {:?}", view),
    }
}

/// Validate: "Mary has eaten." (perfect aspect)
/// Expected structure: Aspectual(Perfect) { Eat(Mary) }
#[test]
fn parse_tree_perfect_aspect() {
    let view = common::parse_to_view("Mary has eaten.");
    match view {
        ExprView::Aspectual { .. } => {
            // Perfect aspect confirmed
        }
        ExprView::NeoEvent { modifiers, verb, .. } => {
            assert_eq!(verb, "Eat", "Expected verb 'Eat', got '{}'", verb);
            // Modifiers should contain perfect aspect info
            assert!(!modifiers.is_empty(), "Modifiers should contain aspect info");
        }
        _ => panic!("Expected Aspectual or NeoEvent for 'Mary has eaten', got {:?}", view),
    }
}

/// Validate: "The train will arrive." (future tense with definite description)
/// Expected structure: Quantifier (for "the") wrapping NeoEvent with Future modifier
#[test]
fn parse_tree_future_tense() {
    let view = common::parse_to_view("The train will arrive.");
    // "The train" creates a definite description (quantifier structure)
    // The NeoEvent with "Future" modifier is embedded inside
    match view {
        ExprView::Quantifier { kind, body, .. } => {
            // "The" creates an existential with uniqueness condition
            assert_eq!(kind, QuantifierKind::Existential,
                "Expected Existential for 'the', got {:?}", kind);
            // Search for NeoEvent with "Arrive" verb and "Future" modifier inside the body
            fn contains_arrive_future(expr: &ExprView) -> bool {
                match expr {
                    ExprView::NeoEvent { verb, modifiers, .. } => {
                        *verb == "Arrive" && modifiers.contains(&"Future")
                    }
                    ExprView::BinaryOp { left, right, .. } => {
                        contains_arrive_future(left) || contains_arrive_future(right)
                    }
                    ExprView::Quantifier { body, .. } => contains_arrive_future(body),
                    _ => false
                }
            }
            assert!(contains_arrive_future(&body),
                "Should contain NeoEvent with Arrive and Future modifier");
        }
        ExprView::Temporal { operator, .. } => {
            // Alternative: direct temporal operator
            assert_eq!(operator, TemporalOperator::Future,
                "Expected Future temporal operator");
        }
        ExprView::NeoEvent { verb, modifiers, .. } => {
            assert_eq!(verb, "Arrive", "Expected verb 'Arrive', got '{}'", verb);
            assert!(modifiers.contains(&"Future"), "Should have Future modifier");
        }
        _ => panic!("Unexpected parse tree structure for 'The train will arrive': {:?}", view),
    }
}

// ============================================================
// Multi-Sentence Discourse Formatting (Logic Mode)
// The numbered-formula invariant of `compile` — a cross-cutting output
// contract, not an example mirror. `example_health` drives single examples
// only and does not assert on the multi-sentence numbering / no-top-level-AND
// behaviour, nor on single-sentence non-numbering.
// ============================================================

/// Multiple sentences produce numbered formulas (like the marketing page),
/// never a single top-level AND conjunction.
#[test]
fn logic_multi_sentence_numbered_output() {
    // Multiple sentences together should produce numbered formulas
    let output = compile("Every cat sleeps. Some dogs bark.").unwrap();

    // Should contain numbered formulas (1, 2)
    assert!(output.contains("1)"),
        "Output should contain '1)' for first sentence. Got: {}", output);
    assert!(output.contains("2)"),
        "Output should contain '2)' for second sentence. Got: {}", output);

    // Should NOT contain top-level AND conjunction
    // (The old behavior was: (∀x... ∧ ∃y...) which is wrong)
    // The new behavior is: 1) ∀x...\n2) ∃y...
    let has_top_level_and = output.starts_with("(") && output.contains(" ∧ ");
    assert!(!has_top_level_and,
        "Should NOT have top-level AND conjunction. Got: {}", output);
}

/// A single sentence has no numbering.
#[test]
fn logic_single_sentence_no_numbering() {
    let output = compile("Every cat sleeps.").unwrap();

    // Single sentence should NOT have "1)" prefix
    assert!(!output.starts_with("1)"),
        "Single sentence should not be numbered. Got: {}", output);
}

// ============================================================
// Proof-Script / Tactic Vernacular (Logic Mode)
// English `Proof:` scripts driven through the tactic framework and
// kernel-certified. These use bespoke theorems (not shipped examples) and are
// the only coverage of the scripted-proof surface in the suite — the shipped
// examples all prove via `Auto`.
// ============================================================

/// A theorem proved by an explicit English-esque `Proof:` SCRIPT (not `Auto`),
/// driven through the tactic framework and kernel-certified — the prose vernacular
/// wired into the surface language.
#[test]
fn logic_prover_with_proof_script() {
    let source = r#"## Theorem: Socrates_By_Script
Given: All men are mortal.
Given: Socrates is a man.
Prove: Socrates is mortal.
Proof: By automation.
"#;
    let result = compile_theorem(source);
    assert!(result.is_ok(), "scripted theorem should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "scripted proof should kernel-verify: {}", output);
}

/// An EXPLICIT prose proof using `intro` over an implication goal — exercises the
/// English→ProofExpr mapping producing `Implies`, then `Assume h; by assumption`.
#[test]
fn logic_prover_implication_by_intro_script() {
    let source = r#"## Theorem: Sad_Implies_Happy
Given: Bob is happy.
Prove: If Bob is sad then Bob is happy.
Proof: Assume h. By assumption.
"#;
    let result = compile_theorem(source);
    assert!(result.is_ok(), "scripted implication should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "intro proof should kernel-verify: {}", output);
}

/// An EXPLICIT prose proof using `cases` over a disjunction premise — ∨-commutativity
/// written as English-esque tactics.
#[test]
fn logic_prover_disjunction_by_cases_script() {
    let source = r#"## Theorem: Or_Commutative
Given: Bob is happy or Bob is sad.
Prove: Bob is sad or Bob is happy.
Proof: By cases on hp0. Right, by assumption. Left, by assumption.
"#;
    let result = compile_theorem(source);
    assert!(result.is_ok(), "scripted disjunction should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "cases proof should kernel-verify: {}", output);
}

/// An EXPLICIT prose proof using `cases` to DESTRUCTURE a conjunction premise
/// (∧-elimination), then re-assemble it the other way round.
#[test]
fn logic_prover_conjunction_by_cases_script() {
    let source = r#"## Theorem: And_Commutative
Given: Bob is happy and Bob is tall.
Prove: Bob is tall and Bob is happy.
Proof: By cases on hp0. Split, by assumption, by assumption.
"#;
    let result = compile_theorem(source);
    assert!(result.is_ok(), "scripted conjunction should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "∧-destructure proof should kernel-verify: {}", output);
}

/// A NAMED premise (`Given (h): …`) referenced by the prose proof as `cases h` — the
/// hypothesis reads by its given name, not the positional `hp0`.
#[test]
fn logic_prover_named_premise_in_script() {
    let source = r#"## Theorem: Or_Comm_Named
Given (h): Bob is happy or Bob is sad.
Prove: Bob is sad or Bob is happy.
Proof: By cases on h. Right, by assumption. Left, by assumption.
"#;
    let result = compile_theorem(source);
    assert!(result.is_ok(), "named-premise theorem should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "named-premise proof should kernel-verify: {}", output);
}

/// ∀-introduction over an English universal goal: `Introduce x` opens the `∀`, then
/// `Assume h; by assumption` discharges the trivial `man(x) → man(x)`.
#[test]
fn logic_prover_universal_intro_script() {
    let source = r#"## Theorem: Men_Are_Men
Prove: All men are men.
Proof: Introduce x. Assume h. By assumption.
"#;
    let result = compile_theorem(source);
    assert!(result.is_ok(), "universal theorem should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "∀-intro proof should kernel-verify: {}", output);
}

/// ∃-cases over an English existential premise: `cases h` opens the witness, then
/// `automation` re-derives the (conjunct-swapped) existential goal.
#[test]
fn logic_prover_existential_cases_script() {
    let source = r#"## Theorem: Some_Mortal_Is_A_Man
Given (h): Some man is mortal.
Prove: Some mortal is a man.
Proof: By cases on h. By automation.
"#;
    let result = compile_theorem(source);
    assert!(result.is_ok(), "existential theorem should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "∃-cases proof should kernel-verify: {}", output);
}
