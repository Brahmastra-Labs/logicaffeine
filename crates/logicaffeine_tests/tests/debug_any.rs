use logicaffeine_language::compile;

#[test]
fn debug_simple_any() {
    let output1 = compile("I saw dogs.");
    assert!(output1.is_ok(), "Failed: I saw dogs: {:?}", output1);

    let output2 = compile("I did see dogs.");
    assert!(output2.is_ok(), "Failed: I did see dogs: {:?}", output2);

    let output3 = compile("I did not see dogs.");
    assert!(output3.is_ok(), "Failed: I did not see dogs: {:?}", output3);

    let output4 = compile("I did not see any dogs.");
    assert!(output4.is_ok(), "Failed: I did not see any dogs: {:?}", output4);

    let output5 = compile("John did not see dogs.");
    assert!(output5.is_ok(), "Failed: John did not see dogs: {:?}", output5);
}
