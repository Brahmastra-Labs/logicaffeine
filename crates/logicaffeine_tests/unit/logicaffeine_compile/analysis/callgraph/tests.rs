//! The functions a run can reach: named from Main, exported or native, or named by one that is.
use super::*;

fn reached(src: &str) -> Vec<String> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let mut names: Vec<String> =
            reached_functions(stmts, interner).into_iter().map(|f| interner.resolve(f).to_string()).collect();
        names.sort();
        names
    })
}

#[test]
fn a_function_main_calls_and_the_ones_it_calls_are_reached() {
    let src = "## To inner (n: Int) -> Int:\n    Return n + 1.\n\n## To outer (n: Int) -> Int:\n    Return inner(n) * 2.\n\n## To unused (n: Int) -> Int:\n    Return n.\n\n## Main\nShow outer(3).\n";
    assert_eq!(reached(src), vec!["inner", "outer"]);
}

#[test]
fn a_function_only_an_unreached_function_calls_is_not_reached() {
    let src = "## To leaf (n: Int) -> Int:\n    Return n.\n\n## To dead (n: Int) -> Int:\n    Return leaf(n).\n\n## Main\nShow 1.\n";
    assert_eq!(reached(src), Vec::<String>::new());
}

#[test]
fn a_function_passed_as_a_value_is_reached() {
    let src = "## To double (x: Int) -> Int:\n    Return x * 2.\n\n## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply(double, 21).\n";
    assert_eq!(reached(src), vec!["apply", "double"]);
}

#[test]
fn a_recursive_function_reaches_itself_only_when_called() {
    let src = "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(5).\n";
    assert_eq!(reached(src), vec!["fact"]);
}

#[test]
fn a_program_of_definitions_alone_reaches_every_function() {
    let src = "## To a (n: Int) -> Int:\n    Return n.\n\n## To b (n: Int) -> Int:\n    Return n.\n";
    assert_eq!(reached(src), vec!["a", "b"]);
}

#[test]
fn a_launched_task_reaches_its_function() {
    let src = "## To worker (n: Int):\n    Show n.\n\n## To helper (n: Int) -> Int:\n    Return n + 1.\n\n## To counted (n: Int):\n    Show helper(n).\n\n## Main\nLaunch a task to worker(1).\nLet h be Launch a task to counted(2).\n";
    assert_eq!(reached(src), vec!["counted", "helper", "worker"]);
}

/// A function the seed picks is reached with everything it names; nothing else joins.
#[test]
fn a_seeded_function_reaches_what_it_names() {
    let src = "## To leaf (n: Int) -> Int:\n    Return n.\n\n## To spare (n: Int) -> Int:\n    Return leaf(n).\n\n## To other (n: Int) -> Int:\n    Return n.\n\n## Main\nShow 1.\n";
    let seeded = crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let mut names: Vec<String> = reached_from(stmts, interner, &|f| interner.resolve(f) == "spare")
            .into_iter()
            .map(|f| interner.resolve(f).to_string())
            .collect();
        names.sort();
        names
    });
    assert_eq!(seeded, vec!["leaf", "spare"]);
    assert_eq!(reached(src), Vec::<String>::new());
}
