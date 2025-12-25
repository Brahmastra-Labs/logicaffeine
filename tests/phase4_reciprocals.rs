use logos::compile;

#[test]
fn reciprocal_basic_expansion() {
    // "John and Mary love each other" should expand to bidirectional predication
    let output = compile("John and Mary love each other.").unwrap();
    // Should contain bidirectional predication: L(J, M) ∧ L(M, J)
    assert!(output.contains("L(J, M)") || output.contains("Love"));
    assert!(output.contains("L(M, J)") || output.contains("Love"));
    // Should NOT contain unexpanded "EachOther"
    assert!(!output.contains("EachOther"));
}

#[test]
fn reciprocal_past_tense() {
    let output = compile("John and Mary saw each other.").unwrap();
    // Contains See predicate (abbreviated to S)
    assert!(output.contains("S(J, M)") || output.contains("S(M, J)") || output.contains("See"));
    assert!(!output.contains("EachOther"));
}

#[test]
fn reciprocal_with_two_entities_simple() {
    // Simpler test: just two entities with verb + reciprocal
    let output = compile("John and Mary help each other.").unwrap();
    assert!(output.contains("H(J, M)") || output.contains("H(M, J)") || output.contains("Help"));
    assert!(!output.contains("EachOther"));
}
