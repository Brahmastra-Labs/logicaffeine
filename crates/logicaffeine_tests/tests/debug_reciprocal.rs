use logicaffeine_language::compile;

#[test]
fn debug_reciprocal() {
    let output1 = compile("John and Mary love each other.");
    assert!(output1.is_ok(), "Failed: John and Mary love each other: {:?}", output1);

    let output2 = compile("John and Mary run.");
    assert!(output2.is_ok(), "Failed: John and Mary run: {:?}", output2);

    let output3 = compile("John loves Mary.");
    assert!(output3.is_ok(), "Failed: John loves Mary: {:?}", output3);
}
