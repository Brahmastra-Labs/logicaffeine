//! Dead-code elimination, pinned at the unit level: a constant nothing reads is not bound; one a later statement, a
//! closure or a function's global read names is.
use super::*;

/// The names the `Let`s of Main bind after dead-code elimination, and those of each function's body.
fn bound_after(src: &str) -> (Vec<String>, Vec<String>) {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let (sa, ea) = (crate::arena::Arena::new(), crate::arena::Arena::new());
        let out = eliminate_dead_code(stmts.to_vec(), &sa, &ea);
        let lets = |block: &[Stmt]| -> Vec<String> {
            block
                .iter()
                .filter_map(|s| match s {
                    Stmt::Let { var, .. } => Some(interner.resolve(*var).to_string()),
                    _ => None,
                })
                .collect()
        };
        let in_functions = out
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { body, .. } => Some(lets(body)),
                _ => None,
            })
            .flatten()
            .collect();
        (lets(&out), in_functions)
    })
}

#[test]
fn a_constant_nothing_reads_is_not_bound() {
    let (main, _) = bound_after("## Main\nLet pi be 3.5.\nLet n be 2.\nShow n.\n");
    assert_eq!(main, vec!["n"], "`pi` is read by nothing");
}

#[test]
fn a_constant_a_later_statement_reads_is_bound() {
    let (main, _) = bound_after("## Main\nLet n be 2.\nIf true:\n    Show n.\n");
    assert_eq!(main, vec!["n"]);
}

#[test]
fn a_binding_only_stored_to_goes_with_its_stores() {
    let kinds = kinds_after("## Main\nLet mutable n be 2.\nSet n to 3.\nShow 1.\n");
    assert_eq!(kinds, vec!["Show"], "no store outlives its binding");
    let unrolled = "## Main\nLet mutable j be 2.\nShow 7.\nSet j to j + 1.\nIf 3 > 1:\n    Set j to j * 2.\nWhile false:\n    Set j to 0.\nShow 8.\n";
    let kinds = kinds_after(unrolled);
    assert_eq!(kinds.iter().filter(|k| ["Let", "Set"].contains(k)).count(), 0, "every store of `j` goes: {kinds:?}");
    assert_eq!(arms_after(unrolled), vec![(vec![], None)], "the store under the guard goes too");
    let (main, _) = bound_after(unrolled);
    assert!(main.is_empty(), "`j` is read by nothing but its own stores: {main:?}");
}

#[test]
fn a_store_that_may_fail_keeps_its_binding() {
    let (main, _) = bound_after("## Main\nLet d be 0.\nLet mutable n be 2.\nSet n to 6 / d.\nShow d.\n");
    assert!(main.contains(&"n".to_string()), "the division may fail, so its store stays and needs `n`: {main:?}");
}

#[test]
fn a_binding_read_after_its_stores_is_bound() {
    let (main, _) = bound_after("## Main\nLet mutable n be 2.\nSet n to n + 1.\nShow n.\n");
    assert_eq!(main, vec!["n"]);
    let (main, _) = bound_after("## Main\nLet mutable n be 2.\nWhile n is less than 5:\n    Set n to n + 1.\nShow 0.\n");
    assert_eq!(main, vec!["n"], "the loop's guard reads `n`");
}

#[test]
fn a_constant_a_closure_captures_is_bound() {
    let (main, _) = bound_after("## Main\nLet step be 3.\nLet add be (x: Int) -> x + step.\nShow add(1).\n");
    assert!(main.contains(&"step".to_string()), "the closure reads `step`: {main:?}");
}

#[test]
fn a_constant_of_main_a_function_reads_as_a_global_is_bound() {
    let (main, _) = bound_after("## To peek -> Int:\n    Return g + 1.\n\n## Main\nLet g be 5.\nShow peek().\n");
    assert_eq!(main, vec!["g"], "`peek` reads Main's `g`");
}

#[test]
fn a_constant_in_a_function_no_later_statement_reads_is_not_bound_whatever_other_functions_name() {
    let src = "## To peek -> Int:\n    Return g + 1.\n\n## To outer -> Int:\n    Let g be 7.\n    Let k be 1.\n    Return k.\n\n## Main\nShow outer().\n";
    let (_, in_functions) = bound_after(src);
    assert_eq!(in_functions, vec!["k"], "a call never reaches `outer`'s own `g`");
}

#[test]
fn a_refinement_typed_constant_keeps_its_check() {
    let (main, _) = bound_after("## Main\nLet x: Int where x > 0 be 5.\nShow 1.\n");
    assert_eq!(main, vec!["x"], "the refinement is checked when `x` is bound");
}

#[test]
fn a_binding_of_a_value_that_is_not_a_literal_is_kept() {
    let (main, _) = bound_after("## Main\nLet a be 4.\nLet b be a * a.\nShow a.\n");
    assert!(main.contains(&"b".to_string()), "only a literal is known to do nothing: {main:?}");
}

/// The statements of Main after dead-code elimination, by kind: `Set`, `If`, `Show`, `Let`, `While`.
fn kinds_after(src: &str) -> Vec<&'static str> {
    crate::ui_bridge::with_parsed_program(src, |parsed, _| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let (sa, ea) = (crate::arena::Arena::new(), crate::arena::Arena::new());
        eliminate_dead_code(stmts.to_vec(), &sa, &ea).iter().map(kind).collect()
    })
}

fn kind(s: &Stmt) -> &'static str {
    match s {
        Stmt::Set { .. } => "Set",
        Stmt::If { .. } => "If",
        Stmt::Show { .. } => "Show",
        Stmt::Let { .. } => "Let",
        Stmt::While { .. } => "While",
        _ => "other",
    }
}

/// The arms of every `If` of Main after dead-code elimination, as the kinds of their statements.
fn arms_after(src: &str) -> Vec<(Vec<&'static str>, Option<Vec<&'static str>>)> {
    crate::ui_bridge::with_parsed_program(src, |parsed, _| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let (sa, ea) = (crate::arena::Arena::new(), crate::arena::Arena::new());
        eliminate_dead_code(stmts.to_vec(), &sa, &ea)
            .iter()
            .filter_map(|s| match s {
                Stmt::If { then_block, else_block, .. } => Some((
                    then_block.iter().map(kind).collect(),
                    else_block.map(|b| b.iter().map(kind).collect()),
                )),
                _ => None,
            })
            .collect()
    })
}

#[test]
fn consecutive_stores_keep_the_last() {
    let src = "## Main\nLet mutable x be 5.\nSet x to 10.\nSet x to 20.\nShow x.\n";
    assert_eq!(kinds_after(src), vec!["Let", "Set", "Show"]);
}

#[test]
fn a_store_read_before_the_next_stays() {
    let src = "## Main\nLet mutable x be 5.\nSet x to 10.\nShow x.\nSet x to 20.\nShow x.\n";
    assert_eq!(kinds_after(src), vec!["Let", "Set", "Show", "Set", "Show"]);
}

#[test]
fn a_guarded_store_overwritten_before_any_read_is_dead() {
    let src = "## Main\nLet n be 5.\nLet mutable i be 0.\nShow i.\nIf i is less than n - 1:\n    Set i to n - 1.\nSet i to 1.\nShow i.\n";
    assert_eq!(kinds_after(src), vec!["Let", "Show", "Set", "Show"], "the guarded store and then `n` go");
}

#[test]
fn a_guarded_store_read_before_the_next_store_stays() {
    let src = "## Main\nLet n be 5.\nLet mutable i be 0.\nShow i.\nIf i is less than n - 1:\n    Set i to n - 1.\nShow i.\nSet i to 1.\nShow i.\n";
    assert_eq!(kinds_after(src), vec!["Let", "Let", "Show", "If", "Show", "Set", "Show"]);
}

#[test]
fn a_guarded_store_under_a_guard_that_could_fail_stays() {
    let src = "## Main\nLet n be 0.\nLet mutable i be 0.\nShow i.\nIf i is less than 10 / n:\n    Set i to 2.\nSet i to 1.\nShow i.\n";
    assert_eq!(kinds_after(src), vec!["Let", "Let", "Show", "If", "Set", "Show"]);
}

#[test]
fn a_store_two_guards_deep_is_dead() {
    let src = "## Main\nLet mutable i be 0.\nLet mutable k be 2.\nShow i.\nShow k.\nIf k is greater than 1:\n    If k is greater than 0:\n        Set i to 7.\nSet i to 1.\nShow i.\n";
    assert_eq!(kinds_after(src), vec!["Let", "Let", "Show", "Show", "Set", "Show"]);
}

#[test]
fn a_dead_store_leaves_its_arm_and_the_rest_stays() {
    let src = "## Main\nLet mutable i be 0.\nLet mutable k be 2.\nShow i.\nIf k is greater than 1:\n    Show k.\n    Set i to 7.\nOtherwise:\n    Set i to 8.\nSet i to 1.\nShow i.\n";
    assert_eq!(arms_after(src), vec![(vec!["Show"], None)], "both stores go; the second arm with its only store");
}

#[test]
fn a_store_a_loop_after_the_if_reads_stays() {
    let src = "## Main\nLet mutable i be 0.\nShow i.\nIf i is less than 5:\n    Set i to 4.\nWhile i is less than 10:\n    Set i to i + 1.\nShow i.\n";
    assert_eq!(arms_after(src), vec![(vec!["Set"], None)]);
}

#[test]
fn a_store_a_call_may_read_stays() {
    let src = "## To f (x: Int) -> Int:\n    If x is at most 0:\n        Return g.\n    Return f(x - 1) + 1.\n\n## Main\nLet mutable g be 1.\nSet g to 5.\nShow f(3).\nSet g to 6.\nShow g.\n";
    let kinds = kinds_after(src);
    assert_eq!(kinds.iter().filter(|k| **k == "Set").count(), 2, "`f` reads `g`: {kinds:?}");
}

#[test]
fn a_store_whose_value_calls_out_stays() {
    let src = "## To f (x: Int) -> Int:\n    Show x.\n    Return x.\n\n## Main\nLet mutable i be 0.\nShow i.\nSet i to f(1).\nSet i to 2.\nShow i.\n";
    let kinds = kinds_after(src);
    assert_eq!(kinds.iter().filter(|k| **k == "Set").count(), 2, "dropping the store would drop the call: {kinds:?}");
}

#[test]
fn a_store_only_a_dropped_guard_reads_is_dead_too() {
    let src = "## Main\nLet mutable i be 0.\nShow i.\nSet i to 3.\nIf i is greater than 2:\n    Set i to 9.\nSet i to 1.\nShow i.\n";
    assert_eq!(kinds_after(src), vec!["Let", "Show", "Set", "Show"], "the guarded store goes, then the `If`, then the store its guard read");
}

#[test]
fn a_store_a_kept_guard_reads_stays() {
    let src = "## Main\nLet mutable i be 0.\nShow i.\nSet i to 3.\nIf i is greater than 2:\n    Show i.\nSet i to 1.\nShow i.\n";
    assert_eq!(kinds_after(src), vec!["Let", "Show", "Set", "If", "Set", "Show"]);
}

#[test]
fn a_loop_that_only_counts_is_the_value_it_leaves() {
    let src = "## Main\nLet mutable i be 0.\nWhile i is less than 5:\n    Set i to i + 1.\nShow i.\n";
    assert_eq!(kinds_after(src), vec!["Let", "If", "Show"], "the loop is the store of its end");
    assert_eq!(arms_after(src), vec![(vec!["Set"], None)]);
    let inclusive = "## Main\nLet mutable i be 1.\nWhile i is at most 5:\n    Set i to i + 1.\nShow i.\n";
    assert_eq!(kinds_after(inclusive), vec!["Let", "If", "Show"]);
}

#[test]
fn a_counting_loop_that_does_work_or_may_fail_stays() {
    let working = "## Main\nLet mutable i be 0.\nWhile i is less than 5:\n    Show i.\n    Set i to i + 1.\n";
    assert_eq!(kinds_after(working), vec!["Let", "While"]);
    let failing = "## Main\nLet d be 0.\nLet mutable i be 0.\nWhile i is less than 10 / d:\n    Set i to i + 1.\nShow i.\n";
    assert!(kinds_after(failing).contains(&"While"), "a bound that may fail is evaluated as written");
}
