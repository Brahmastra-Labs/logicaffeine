// =============================================================================
// PHASE 65: EVENT SEMANTICS - ALPHA-EQUIVALENCE TESTS
// =============================================================================
//
// These tests verify that the proof engine can handle event semantics,
// which require alpha-equivalence (the understanding that bound variable
// names are arbitrary).
//
// Example: ∃e(Run(e) ∧ Agent(e, John)) should unify with ∃x(Run(x) ∧ Agent(x, John))

use logos::compile_theorem;

#[test]
fn test_event_alpha_equivalence() {
    // The simplest event test: prove a premise matches itself.
    // This requires alpha-equivalence because each parse generates fresh event vars.
    //
    // Premise: "John runs" → ∃e(Run(e) ∧ Agent(e, John))
    // Goal:    "John runs" → ∃e'(Run(e') ∧ Agent(e', John))
    //
    // Without alpha-equivalence, this fails because 'e' != 'e''.

    let input = r#"
## Theorem: Event_Binding
Given: John runs.
Prove: John runs.
Proof: Auto.
"#;

    let result = compile_theorem(input);

    if let Err(e) = &result {
        println!("Error (may indicate alpha-equivalence needed): {:?}", e);
    }

    assert!(
        result.is_ok(),
        "Failed to unify events with different variable names: {:?}",
        result
    );
}

#[test]
fn test_event_chain_simple() {
    // Chain reasoning with events:
    // "Every runner sweats" + "John runs" → "John sweats"
    //
    // The universal premise creates: ∀x(Runner(x) → ∃e(Sweat(e) ∧ Agent(e, x)))
    // "John runs" creates: ∃e'(Run(e') ∧ Agent(e', John))
    // We need to prove: ∃e''(Sweat(e'') ∧ Agent(e'', John))

    let input = r#"
## Theorem: Event_Chain
Given: Every runner sweats.
Given: John is a runner.
Prove: John sweats.
Proof: Auto.
"#;

    let result = compile_theorem(input);

    if let Err(e) = &result {
        println!("Chain error: {:?}", e);
    }

    assert!(
        result.is_ok(),
        "Event chain reasoning failed: {:?}",
        result
    );
}

#[test]
fn test_copular_still_works() {
    // Sanity check: copular sentences (Phase 63/64) still work after our changes.
    // This uses adjective predicates, not events.

    let input = r#"
## Theorem: Copular_Check
Given: All cats are furry.
Given: Whiskers is a cat.
Prove: Whiskers is furry.
Proof: Auto.
"#;

    let result = compile_theorem(input);
    assert!(
        result.is_ok(),
        "Copular sentences should still work: {:?}",
        result
    );
}

#[test]
fn test_socrates_fears_death_full_semantics() {
    // THE VICTORY LAP: Full Event Semantics with Alpha-Equivalence
    //
    // This is the original Socrates theorem that motivated the entire proof engine.
    // It tests the complete pipeline:
    //   1. "fears" → Parser creates Aspectual(State, Exists(e, Fear(e)...))
    //   2. Converter → Unwraps Aspectual, produces Exists(e, Fear(e)...)
    //   3. "death" → Treated as constant "Death" (Abstract Noun)
    //   4. Prover → Needs to unify:
    //        Goal:   ∃e (Fear(e) ∧ Agent(e, Socrates) ∧ Theme(e, Death))
    //        Axiom:  ∀x (Mortal(x) → ∃e' (Fear(e') ∧ Agent(e', x) ∧ Theme(e', Death)))
    //   5. Unifier → Alpha-equivalence renames e → e' to match

    let input = r#"
## Theorem: Socrates_Real
Given: Every man is mortal.
Given: Every mortal fears death.
Given: Socrates is a man.
Prove: Socrates fears death.
Proof: Auto.
"#;

    let result = compile_theorem(input);

    if let Err(e) = &result {
        println!("Theorem Failed: {:?}", e);
    }

    assert!(
        result.is_ok(),
        "Full Event Semantics should work with alpha-equivalence: {:?}",
        result
    );
}
