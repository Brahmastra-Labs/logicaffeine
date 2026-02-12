use logicaffeine_language::compile;

#[test]
fn debug_garden_path() {
    let output1 = compile("The horse raced past the barn fell.");
    assert!(output1.is_ok(), "Failed: The horse raced past the barn fell: {:?}", output1);

    let output2 = compile("The horse raced past the barn.");
    assert!(output2.is_ok(), "Failed: The horse raced past the barn: {:?}", output2);

    let output3 = compile("The man pushed fell.");
    assert!(output3.is_ok(), "Failed: The man pushed fell: {:?}", output3);

    let output4 = compile("The man fell.");
    assert!(output4.is_ok(), "Failed: The man fell: {:?}", output4);
}
