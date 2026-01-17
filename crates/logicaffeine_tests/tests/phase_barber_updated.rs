//! Test the updated barber paradox formulation

#[test]
fn test_barber_parsing_only() {
    // Just check the parsing structure - no proof attempt
    let result = logicaffeine_compile::compile_theorem_for_ui(r#"## Theorem: Barber_Paradox
Given: The barber is a man.
Given: The barber shaves all men who do not shave themselves.
Given: The barber does not shave any man who shaves himself.
Prove: The barber does not exist.
Proof: Auto.
"#);

    println!("\n=== BARBER PARSING STRUCTURE ===");
    println!("Name: {}", result.name);
    println!("\nPremises (Debug):");
    for (i, p) in result.premises.iter().enumerate() {
        println!("  {}: {:?}", i, p);
    }
    if let Some(g) = &result.goal {
        println!("\nGoal (Debug): {:?}", g);
    }
    if let Some(ref err) = result.error {
        println!("\nError: {}", err);
    }

    // Just check that parsing succeeded
    assert!(result.error.is_none() || !result.error.as_ref().unwrap().contains("Parse error"),
        "Barber paradox should parse successfully: {:?}", result.error);
}

#[test]
fn test_simple_man_premise() {
    // First, test if "Socrates is a man" works (it should)
    let result = logicaffeine_compile::compile_theorem_for_ui(r#"## Theorem: Simple_Man
Given: Socrates is a man.
Prove: Socrates is a man.
Proof: Auto.
"#);

    println!("\n=== SIMPLE MAN TEST ===");
    println!("Name: {}", result.name);
    println!("Premises: {:?}", result.premises);
    println!("Goal: {:?}", result.goal);
    println!("Derivation: {}", result.derivation.is_some());
    println!("Error: {:?}", result.error);

    assert!(result.error.is_none() || !result.error.as_ref().unwrap().contains("Parse error"),
        "Simple man should parse: {:?}", result.error);
}

#[test]
fn test_definite_man_premise() {
    // Now test if "The barber is a man" works
    let result = logicaffeine_compile::compile_theorem_for_ui(r#"## Theorem: Definite_Man
Given: The barber is a man.
Prove: The barber is a man.
Proof: Auto.
"#);

    println!("\n=== DEFINITE MAN TEST ===");
    println!("Name: {}", result.name);
    println!("Premises: {:?}", result.premises);
    println!("Goal: {:?}", result.goal);
    println!("Derivation: {}", result.derivation.is_some());
    println!("Error: {:?}", result.error);

    assert!(result.error.is_none() || !result.error.as_ref().unwrap().contains("Parse error"),
        "Definite man should parse: {:?}", result.error);
}

#[test]
fn test_barber_paradox_updated_formulation() {
    let result = logicaffeine_compile::compile_theorem_for_ui(r#"## Theorem: Barber_Paradox
Given: The barber is a man.
Given: The barber shaves all men who do not shave themselves.
Given: The barber does not shave any man who shaves himself.
Prove: The barber does not exist.
Proof: Auto.
"#);

    println!("\n=== UPDATED BARBER PARADOX ===");
    println!("Name: {}", result.name);
    println!("\nPremises:");
    for (i, p) in result.premises.iter().enumerate() {
        println!("  {}: {}", i, p);
    }
    if let Some(g) = &result.goal {
        println!("\nGoal: {}", g);
    }
    println!("\nDerivation exists: {}", result.derivation.is_some());
    if let Some(ref deriv) = result.derivation {
        println!("Derivation tree:\n{}", deriv.display_tree());
    }
    if let Some(ref err) = result.error {
        println!("Error: {}", err);
    }

    // Debug: Show what the KB looks like after preprocessing
    use logicaffeine_proof::{BackwardChainer, ProofExpr};
    let mut engine = BackwardChainer::new();
    for p in &result.premises {
        engine.add_axiom(p.clone());
    }
    // Run prove to trigger preprocessing
    let _ = engine.prove(ProofExpr::Atom("dummy".into()));
    println!("\nKB after preprocessing:");
    for (i, expr) in engine.knowledge_base().iter().enumerate() {
        println!("  {}: {}", i, expr);
    }

    // Now we expect the proof to succeed
    assert!(result.derivation.is_some(),
        "Barber paradox should prove successfully, got error: {:?}",
        result.error);
}
