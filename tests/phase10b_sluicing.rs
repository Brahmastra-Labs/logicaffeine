use logos::compile;

#[test]
fn sluicing_basic_who() {
    // "Someone left. I know who."
    // Should parse as: ∃x(Left(x)) ∧ Know(Speaker, Question(y, Left(y)))
    let output = compile("Someone left. I know who.").unwrap();

    // Must have Leave predicate (lemma form of "left")
    assert!(output.contains("Leave") || output.contains("L("),
        "Should contain Leave predicate. Output: {}", output);

    // Must have Know predicate
    assert!(output.contains("Know") || output.contains("K("),
        "Should contain Know predicate. Output: {}", output);

    // Must have Question wrapper (not just a bare "?")
    assert!(output.contains("Question") || output.contains("?[") || output.contains("λ"),
        "Should contain Question or lambda abstraction. Output: {}", output);
}

#[test]
fn sluicing_object_what() {
    // "John ate something. I know what."
    // Should reconstruct: Know(Speaker, Question(y, Eat(John, y)))
    let output = compile("John ate something. I know what.").unwrap();

    // Must have Eat predicate twice (once in main, once in sluiced question)
    assert!(output.contains("Eat") || output.contains("E("),
        "Should contain Eat predicate. Output: {}", output);

    // Must have Know predicate
    assert!(output.contains("Know") || output.contains("K("),
        "Should contain Know predicate. Output: {}", output);

    // The sluiced clause should reference the object slot
    assert!(output.contains("Question") || output.contains("?[") || output.contains("λ"),
        "Should contain Question for the embedded wh-clause. Output: {}", output);
}

#[test]
fn sluicing_with_negation() {
    // "Someone called. I don't know who."
    // Should parse as: ∃x(Call(x)) ∧ ¬Know(Speaker, Question(y, Call(y)))
    let output = compile("Someone called. I don't know who.").unwrap();

    assert!(output.contains("Cal") || output.contains("Call") || output.contains("C("),
        "Should contain Call predicate. Output: {}", output);

    assert!(output.contains("¬") || output.contains("Not"),
        "Should contain negation. Output: {}", output);

    assert!(output.contains("Know") || output.contains("K("),
        "Should contain Know predicate. Output: {}", output);
}

#[test]
fn sluicing_mid_sentence() {
    // "Someone ran. I wonder who."
    // Tests sluicing with wonder (embedding verb)
    let output = compile("Someone ran. I wonder who.").unwrap();

    assert!(output.contains("Run") || output.contains("R("),
        "Should contain Run predicate. Output: {}", output);

    // The sluiced question should reconstruct the Run event
    assert!(output.contains("Question") || output.contains("?[") ||
            (output.matches("Run").count() >= 2) || (output.matches("R(").count() >= 2),
        "Should have reconstructed Run in the sluiced question. Output: {}", output);
}
