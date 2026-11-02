use logicaffeine_language::compile;

#[test]
fn debug_tense_structures() {
    let past = compile("John ran.");
    assert!(past.is_ok(), "Failed: John ran: {:?}", past);

    let future = compile("John will run.");
    assert!(future.is_ok(), "Failed: John will run: {:?}", future);

    let present = compile("John runs.");
    assert!(present.is_ok(), "Failed: John runs: {:?}", present);
}
