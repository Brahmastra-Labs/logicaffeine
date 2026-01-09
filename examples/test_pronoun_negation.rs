fn main() {
    let source = r#"## Theorem: Modus_Tollens_Chain
Given: If the butler did it, he was seen.
Given: If he was seen, he was caught.
Given: He was not caught.
Prove: The butler did not do it.
Proof: Auto.
"#;
    let result = logos::compile_theorem_for_ui(source);
    println!("Name: {}", result.name);
    println!("Premises:");
    for (i, p) in result.premises.iter().enumerate() {
        println!("  {}: {}", i, p);
    }
    if let Some(g) = &result.goal {
        println!("Goal: {}", g);
    } else {
        println!("Goal: None");
    }
    println!("Derivation: {:?}", result.derivation.is_some());
    println!("Error: {:?}", result.error);
}
