//! Phase D — Jones optimality (PE_IMPROVE §5).
//!
//! For every program, the PE residual must contain ZERO surviving interpreter dispatch
//! (`env`/`funcs` lookups, `peExpr`/`peBlock`/`coreEval` names, un-reduced Core-variant
//! `Inspect`) — the interpreter has fully dissolved — AND it must still compute the right
//! answer (optimality never at the cost of correctness).
//!
//! NOTE: programs use `"\` + real newlines + real indentation (NOT `\n\` continuation).

mod pe_support;

use pe_support::*;

/// (program, expected output) — diverse coverage of the folded operation surface.
fn jones_corpus() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("int_arith", "## Main\nShow 2 + 3 * 4.", "14"),
        ("text_interp", "## Main\nLet n be 7.\nShow \"v={n}\".", "v=7"),
        ("struct_access", "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet b be a new Box with base 3 and flex 7.\nShow b's base.", "3"),
        ("list_index", "## Main\nLet xs be [10, 20, 30].\nShow item 2 of xs.", "20"),
        ("tuple_index", "## Main\nLet t be (5, 6, 7).\nShow item 3 of t.", "7"),
        ("while_unroll", "## Main\nLet mutable i be 3.\nLet mutable s be 0.\nWhile i is greater than 0:\n    Set s to s + i.\n    Set i to i - 1.\nShow s.", "6"),
        ("repeat_range", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 5:\n    Set s to s + i.\nShow s.", "15"),
        ("nested_loops", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 3:\n    Repeat for j from 1 to 3:\n        Set s to s + 1.\nShow s.", "9"),
        ("factorial", "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(5).", "120"),
        ("map_get", "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to 2.\nShow item \"b\" of m.", "2"),
        ("set_contains", "## Main\nLet s be a new Set of Int.\nAdd 3 to s.\nIf s contains 3:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".", "yes"),
        ("text_len", "## Main\nShow length of \"hello\".", "5"),
        ("closure_hof", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 21).", "42"),
        ("guard_prune", "## Main\nLet x be 3.\nIf x equals 3:\n    Show \"hit\".\nOtherwise:\n    Show \"miss\".", "hit"),
    ]
}

/// Every residual is free of surviving interpreter dispatch (Jones-optimal).
#[test]
fn p1_no_dispatch_full_corpus() {
    let mut failures = Vec::new();
    for (name, program, _expected) in jones_corpus() {
        let residual = match decompile(program) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("[{}] PE failed: {}", name, e));
                continue;
            }
        };
        let d = count_dispatch(&residual);
        if d != 0 {
            failures.push(format!(
                "[{}] residual has {} units of interpreter dispatch:\n{}",
                name, d, residual
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "Jones-optimality violations ({}):\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

/// Optimality never at the cost of correctness: every residual still runs to the right answer.
#[test]
fn p1_residual_run_equals_source() {
    for (name, program, expected) in jones_corpus() {
        let _ = name;
        assert_run_equals(program, expected);
    }
}
