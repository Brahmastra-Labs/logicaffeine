use logos::compile;

#[test]
fn metaphor_juliet_sun() {
    let output = compile("Juliet is the sun.").unwrap();
    assert!(output.contains("Metaphor") || output.contains("≈"),
        "Sort mismatch (Human/Celestial) should trigger metaphor. Output: {}", output);
    assert!(output.contains("Juliet") || output.contains("J"),
        "Tenor should be present. Output: {}", output);
    assert!(output.contains("Sun") || output.contains("S"),
        "Vehicle should be present. Output: {}", output);
}

#[test]
fn metaphor_time_money() {
    let output = compile("Time is money.").unwrap();
    assert!(output.contains("Metaphor") || output.contains("≈"),
        "Sort mismatch (Abstract/Value) should trigger metaphor. Output: {}", output);
}

#[test]
fn literal_king_bald() {
    let output = compile("The king is bald.").unwrap();
    assert!(!output.contains("Metaphor"),
        "Adjective predicate should NOT be a metaphor. Output: {}", output);
}

#[test]
fn literal_john_man() {
    let output = compile("John is a man.").unwrap();
    assert!(!output.contains("Metaphor"),
        "Human/Human should NOT be a metaphor. Output: {}", output);
    assert!(output.contains("Man") || output.contains("M("),
        "Should be standard predication. Output: {}", output);
}
