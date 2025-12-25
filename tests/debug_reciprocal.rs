use logos::compile;

#[test]
fn debug_reciprocal() {
    let output1 = compile("John and Mary love each other.");
    println!("John and Mary love each other: {:?}", output1);

    let output2 = compile("John and Mary run.");
    println!("John and Mary run: {:?}", output2);

    let output3 = compile("John loves Mary.");
    println!("John loves Mary: {:?}", output3);
}
