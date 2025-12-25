use logos::compile;

#[test]
fn garden_path_reduced_relative() {
    // "The horse raced past the barn fell."
    // Standard parse fails: "The horse raced past the barn" is complete, but "fell" remains
    // Correct parse: "The horse [that was] raced past the barn" + "fell"
    let output = compile("The horse raced past the barn fell.").unwrap();
    // Should contain both Race and Fall predicates
    assert!(output.contains("Race") || output.contains("R(") || output.contains("Raced"));
    assert!(output.contains("Fall") || output.contains("F(") || output.contains("Fell"));
}

#[test]
fn garden_path_simple_reduced_relative() {
    // "The man pushed fell."
    // "The man [who was] pushed" + "fell"
    let output = compile("The man pushed fell.").unwrap();
    assert!(output.contains("Push") || output.contains("P("));
    assert!(output.contains("Fall") || output.contains("F("));
}
