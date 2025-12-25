use logos::compile;

#[test]
fn test_john_has_cardinality_5() {
    let output = compile("John has cardinality 5.").unwrap();
    eprintln!("John has cardinality 5: {}", output);
    assert!(output.contains("5"));
}

#[test]
fn test_set_a_has_cardinality_5() {
    let output = compile("Set A has cardinality 5.").unwrap();
    eprintln!("Set A has cardinality 5: {}", output);
    assert!(output.contains("5"));
}
