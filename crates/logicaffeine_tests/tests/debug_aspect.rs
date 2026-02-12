use logicaffeine_language::compile;

#[test]
fn debug_aspect_chain() {
    let output1 = compile("The apple would have been being eaten.");
    assert!(output1.is_ok(), "Failed: would have been being eaten: {:?}", output1);

    let output2 = compile("John would run.");
    assert!(output2.is_ok(), "Failed: John would run: {:?}", output2);

    let output3 = compile("John has run.");
    assert!(output3.is_ok(), "Failed: John has run: {:?}", output3);

    let output4 = compile("The apple was eaten.");
    assert!(output4.is_ok(), "Failed: was eaten: {:?}", output4);
}
