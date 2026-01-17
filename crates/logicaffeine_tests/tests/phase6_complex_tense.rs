use logicaffeine_language::compile;

#[test]
fn simple_past_baseline() {
    let output = compile("John ran.").unwrap();
    eprintln!("DEBUG simple_past: {}", output);
    assert!(output.contains("Run") || output.contains("Ran"));
}

#[test]
fn debug_had_finished() {
    let output = compile("John had finished.").unwrap();
    eprintln!("DEBUG had_finished: {}", output);
    // Symbol registry abbreviates "Finish" to "F"
    assert!(
        output.contains("Finish") || output.contains("F("),
        "Should contain Finish predicate: got '{}'", output
    );
}

#[test]
fn past_perfect_explicit_constraints() {
    let output = compile("John had run.").unwrap();
    eprintln!("DEBUG past_perfect: {}", output);

    // Perfect: Event before Reference
    assert!(
        output.contains("Precedes(e") || output.contains("< r") || output.contains("E < R"),
        "Perfect must place E < R: got '{}'", output
    );

    // Past: Reference before Speech
    assert!(
        output.contains("Precedes(r") || output.contains("< S") || output.contains("R < S"),
        "Past must place R < S: got '{}'", output
    );
}

#[test]
fn future_perfect_constraints() {
    let output = compile("John will have run.").unwrap();
    eprintln!("DEBUG future_perfect: {}", output);

    // Future: Speech before Reference
    assert!(
        output.contains("S <") || output.contains("Precedes(S"),
        "Future must place S < R: got '{}'", output
    );

    // Perfect: Event before Reference
    assert!(
        output.contains("E <") || output.contains("Precedes(e"),
        "Perfect must place E < R: got '{}'", output
    );
}

#[test]
fn present_perfect_constraints() {
    let output = compile("John has run.").unwrap();
    eprintln!("DEBUG present_perfect: {}", output);

    // Present: R = S (or R at S)
    // Perfect: E < R
    assert!(
        output.contains("Precedes(e") || output.contains("E <"),
        "Present Perfect: E < R: got '{}'", output
    );
}

#[test]
fn simple_future_constraints() {
    let output = compile("John will run.").unwrap();
    eprintln!("DEBUG future: {}", output);

    assert!(
        output.contains("S <") || output.contains("Precedes(S") || output.contains("Future"),
        "Future: S < R: got '{}'", output
    );
}
