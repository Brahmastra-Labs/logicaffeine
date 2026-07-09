use logicaffeine_language::compile;

#[test]
fn goal_sentence_has_all_four() {
    let s = "Determine each trip's activity, state and year, as well as the friend Simon went with.";
    let out = compile(s).expect("goal should compile");
    println!("GOAL OUT:\n{out}");
    for needle in ["Activity", "State", "Year", "Friend"] {
        assert!(out.contains(needle), "goal lost {needle}; got: {out}");
    }
}
