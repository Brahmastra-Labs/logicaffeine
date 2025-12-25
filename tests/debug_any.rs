use logos::compile;

#[test]
fn debug_simple_any() {
    // Test pronoun subject with auxiliary negation
    let output1 = compile("I saw dogs.");
    println!("I saw dogs: {:?}", output1);

    let output2 = compile("I did see dogs.");
    println!("I did see dogs: {:?}", output2);

    let output3 = compile("I did not see dogs.");
    println!("I did not see dogs: {:?}", output3);

    let output4 = compile("I did not see any dogs.");
    println!("I did not see any dogs: {:?}", output4);

    // Also test the original working cases
    let output5 = compile("John did not see dogs.");
    println!("John did not see dogs: {:?}", output5);
}
