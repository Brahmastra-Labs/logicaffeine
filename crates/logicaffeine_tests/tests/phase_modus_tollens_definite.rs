//! Modus tollens over a DEFINITE DESCRIPTION subject.
//!
//! "If the butler did it, he was seen." threads one discourse referent through
//! a definite description ("the butler") and a co-referring pronoun ("he").
//! Both must denote the SAME term or the kernel certifier cannot match the
//! hypothesis and the theorem finds a derivation but fails certification.
//!
//! This is the exact `LOGIC_MODUS_TOLLENS` Studio example.

const LOGIC_MODUS_TOLLENS_SOURCE: &str = r#"## Theorem: Modus_Tollens_Chain
Given: If the butler did it, he was seen.
Given: If he was seen, he was caught.
Given: He was not caught.
Prove: The butler did not do it.
Proof: Auto.
"#;

const LOGIC_BARBER_SOURCE: &str = r#"## Theorem: Barber_Paradox
Given: The barber is a man.
Given: The barber shaves all men who do not shave themselves.
Given: The barber does not shave any man who shaves himself.
Prove: The barber does not exist.
Proof: Auto.
"#;

#[test]
fn modus_tollens_definite_description_certifies() {
    let result = logicaffeine_compile::compile_theorem_for_ui(LOGIC_MODUS_TOLLENS_SOURCE);

    println!("\n=== MODUS TOLLENS (definite) ===");
    println!("Name: {}", result.name);
    println!("\nPremises:");
    for (i, p) in result.premises.iter().enumerate() {
        println!("  {}: {:?}", i, p);
    }
    if let Some(g) = &result.goal {
        println!("\nGoal: {:?}", g);
    }
    println!("\nDerivation exists: {}", result.derivation.is_some());
    println!("Verified: {}", result.verified);
    if let Some(ref err) = result.verification_error {
        println!("Verification error: {}", err);
    }
    if let Some(ref err) = result.error {
        println!("Error: {}", err);
    }

    assert!(
        result.derivation.is_some(),
        "modus tollens should find a derivation: {:?}",
        result.verification_error
    );
    assert!(
        result.verified,
        "modus tollens over a definite description MUST certify: {:?}",
        result.verification_error
    );
}

#[test]
fn report_barber_status() {
    let result = logicaffeine_compile::compile_theorem_for_ui(LOGIC_BARBER_SOURCE);
    println!("\n=== BARBER STATUS ===");
    println!("Derivation exists: {}", result.derivation.is_some());
    println!("Verified: {}", result.verified);
    if let Some(ref err) = result.verification_error {
        println!("Verification error: {}", err);
    }
}
