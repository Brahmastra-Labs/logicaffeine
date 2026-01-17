use logicaffeine_language::compile;

#[test]
fn debug_tense_structures() {
    let past = compile("John ran.");
    println!("John ran: {:?}", past);

    let future = compile("John will run.");
    println!("John will run: {:?}", future);

    let present = compile("John runs.");
    println!("John runs: {:?}", present);
}
