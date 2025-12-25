use logos::compile;

#[test]
fn debug_garden_path() {
    let output1 = compile("The horse raced past the barn fell.");
    println!("The horse raced past the barn fell: {:?}", output1);

    let output2 = compile("The horse raced past the barn.");
    println!("The horse raced past the barn: {:?}", output2);

    let output3 = compile("The man pushed fell.");
    println!("The man pushed fell: {:?}", output3);

    let output4 = compile("The man fell.");
    println!("The man fell: {:?}", output4);
}
