use logicaffeine_language::compile;

#[test]
fn comparative_with_measure_phrase() {
    let output = compile("John is 2 inches taller than Mary.").unwrap();
    eprintln!("DEBUG comparative_measure: {}", output);
    assert!(
        output.contains("Taller(") && output.contains("2") && output.contains("inch"),
        "Should produce comparative with measure phrase: got '{}'",
        output
    );
}

#[test]
fn absolute_measurement() {
    let output = compile("The rope is 5 meters long.").unwrap();
    eprintln!("DEBUG absolute_measurement: {}", output);
    assert!(
        (output.contains("Long(") || output.contains("L(")) && output.contains("5") && output.contains("meter"),
        "Should produce absolute measurement: got '{}'",
        output
    );
}

#[test]
fn symbolic_cardinality() {
    let output = compile("Set A has cardinality aleph_0.").unwrap();
    eprintln!("DEBUG symbolic: {}", output);
    assert!(
        output.contains("aleph_0") || output.contains("aleph"),
        "Should produce symbolic cardinality: got '{}'",
        output
    );
}

#[test]
fn integer_count() {
    let output = compile("John has 3 children.").unwrap();
    eprintln!("DEBUG integer_count: {}", output);
    assert!(
        output.contains("3") && (output.contains("Child") || output.contains("child")),
        "Should handle integer count: got '{}'",
        output
    );
}

#[test]
fn real_measurement() {
    let output = compile("The temperature is 98.6 degrees.").unwrap();
    eprintln!("DEBUG real_measurement: {}", output);
    assert!(
        output.contains("98.6") && output.contains("degree"),
        "Should handle real measurement: got '{}'",
        output
    );
}
