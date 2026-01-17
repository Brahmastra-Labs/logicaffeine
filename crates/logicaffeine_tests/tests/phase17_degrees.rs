use logicaffeine_language::compile;

#[test]
fn test_comparative_with_measure() {
    let input = "John is 2 inches taller than Mary.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("Taller"), "Should contain comparative predicate");
    assert!(result.contains(", 2 inches)"), "Should contain measure as 3rd arg");
}

#[test]
fn test_clausal_comparative_ellipsis() {
    let input = "John is taller than Bill is.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("Taller"), "Should parse clausal comparative");
    assert!(result.contains("B") || result.contains("Bill"), "Should bind comparison target");
    assert!(!result.contains("is"), "Should reduce clausal 'is'");
}

#[test]
fn test_superlative_relative_scope() {
    let input = "John climbed the highest mountain.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("∀") || result.contains("All"), "Superlative implies Universal");
    assert!(result.contains("Mountain"), "Should restrict to Mountain");
    assert!(result.contains("Higher") || result.contains("Taller") || result.contains("High"), "Superlative expands to comparative");
}
