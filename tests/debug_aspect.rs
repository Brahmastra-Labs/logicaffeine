use logos::compile;

#[test]
fn debug_aspect_chain() {
    let output1 = compile("The apple would have been being eaten.");
    println!("would have been being eaten: {:?}", output1);

    let output2 = compile("John would run.");
    println!("John would run: {:?}", output2);

    let output3 = compile("John has run.");
    println!("John has run: {:?}", output3);

    let output4 = compile("The apple was eaten.");
    println!("was eaten: {:?}", output4);
}
