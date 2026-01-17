use logicaffeine_language::compile;

#[test]
fn aspect_chain_would_have_been_being() {
    // "The apple would have been being eaten."
    // Modal + Perfect + Passive + Progressive + Verb
    let output = compile("The apple would have been being eaten.").unwrap();
    // Should contain modal (would), perfect (have), and verb
    assert!(output.contains("Would") || output.contains("□") || output.contains("◇"));
    assert!(output.contains("Perf") || output.contains("Perfect") || output.contains("have"));
}

#[test]
fn aspect_simple_passive() {
    // "The apple was eaten."
    let output = compile("The apple was eaten.").unwrap();
    // Should contain passive structure - agent not specified
    assert!(output.contains("Eat") || output.contains("eat") || output.contains("E("));
}

#[test]
fn aspect_passive_with_agent() {
    // "The apple was eaten by John."
    let output = compile("The apple was eaten by John.").unwrap();
    // Should contain both eat and John
    assert!(output.contains("Eat") || output.contains("E("));
    assert!(output.contains("J") || output.contains("John"));
}
