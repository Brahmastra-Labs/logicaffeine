use logicaffeine_language::compile;

#[test]
fn subsective_adjective_small_elephant() {
    let output = compile("A small elephant ran.").unwrap();
    eprintln!("DEBUG subsective: {}", output);
    // Subsective adjective: S(x, ^Elephant) - adjective abbreviated, intension uses full word
    assert!(
        output.contains("^Elephant") || output.contains("^elephant"),
        "Should produce subsective logic with ^Elephant intension: got '{}'",
        output
    );
}

#[test]
fn subsective_adjective_large_mouse() {
    let output = compile("A large mouse ran.").unwrap();
    eprintln!("DEBUG large_mouse: {}", output);
    // Subsective: uses ^Mouse intension
    assert!(
        output.contains("^Mouse") || output.contains("^mouse"),
        "Should produce subsective logic with ^Mouse intension: got '{}'",
        output
    );
}

#[test]
fn intersective_unchanged() {
    let output = compile("A red ball rolled.").unwrap();
    eprintln!("DEBUG intersective: {}", output);
    // Red->R, Ball->B - should remain as conjunction R(x) ∧ B(x)
    // NOT R(x, ^B) since red is intersective
    assert!(
        (output.contains("R(x)") || output.contains("Red(x)")) &&
        (output.contains("B(x)") || output.contains("Ball(x)")),
        "Intersective adjective should stay as conjunction: got '{}'",
        output
    );
}

#[test]
fn generalized_quantifier_many() {
    let output = compile("Many dogs bark.").unwrap();
    eprintln!("DEBUG many: {}", output);
    assert!(
        output.contains("MANY") || output.contains("Many x"),
        "Should support 'Many' quantifier: got '{}'",
        output
    );
}

#[test]
fn good_thief_subsective() {
    let output = compile("A good thief escaped.").unwrap();
    eprintln!("DEBUG good_thief: {}", output);
    // Subsective: uses ^Thief intension
    assert!(
        output.contains("^Thief") || output.contains("^thief"),
        "Should produce subsective logic with ^Thief intension: got '{}'",
        output
    );
}
