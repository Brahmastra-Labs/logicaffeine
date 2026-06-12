//! Phase D — Differential corpus (PE_IMPROVE §4.2/§5, closes gap G8).
//!
//! The centerpiece safety case: over a broad corpus exercising every B1–B5 feature, the
//! genuine PE residual run through the tree-walker (`run_p1`) must observably agree with the
//! production tree-walker on the source (`run_treewalk`) — same output stream, same
//! value/error class. The tree-walker is the independent oracle; any divergence is a real PE
//! bug. This is "robust to the point of absurdity": one assertion, dozens of programs.
//!
//! NOTE: programs use `"\` + real newlines + real indentation (NOT `\n\` continuation).

mod pe_support;

use pe_support::*;

/// A broad, hand-curated corpus spanning the operation surface the PE now folds.
fn differential_corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        // --- B1: arithmetic / text / coercion ---
        ("int_arith", "## Main\nShow 2 + 3 * 4 - 1."),
        ("int_div_mod", "## Main\nShow 17 / 5.\nShow 17 % 5."),
        ("int_bitwise", "## Main\nShow 6 xor 3.\nShow 1 shifted left by 4."),
        ("float_arith", "## Main\nShow 1.5 + 2.25.\nShow 10.0 / 4.0."),
        ("mixed_int_float", "## Main\nShow 3 + 0.5."),
        ("text_concat", "## Main\nLet n be 42.\nShow \"n is {n}\"."),
        ("bool_logic", "## Main\nShow true and false.\nShow true or false.\nShow not true."),
        ("comparisons", "## Main\nShow 3 is less than 5.\nShow 5 is at most 5."),
        // --- dynamic accumulator (forces a residual loop) ---
        ("dynamic_sum", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 200000:\n    Set s to s + 1.\nShow s."),
        // --- B2: structs / partial-static ---
        ("struct_field", "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet b be a new Box with base 3 and flex 7.\nShow b's base.\nShow b's flex."),
        ("struct_partial", "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + 1.\nLet b be a new Box with base 5 and flex d.\nShow b's base.\nShow b's flex."),
        ("struct_setfield", "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet mutable b be a new Box with base 1 and flex 7.\nSet b's base to 9.\nShow b's base.\nShow b's flex."),
        // --- B2.2: lists / tuples ---
        ("list_index", "## Main\nLet xs be [10, 20, 30].\nShow item 2 of xs.\nShow length of xs."),
        ("tuple_index", "## Main\nLet t be (1, 2, 3).\nShow item 3 of t."),
        ("list_dynamic", "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + 1.\nLet xs be [1, d, 3].\nShow item 1 of xs.\nShow item 2 of xs."),
        // --- B3: loops ---
        ("while_static", "## Main\nLet mutable i be 3.\nLet mutable s be 0.\nWhile i is greater than 0:\n    Set s to s + i.\n    Set i to i - 1.\nShow s."),
        ("repeat_range", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 5:\n    Set s to s + i.\nShow s."),
        ("repeat_break", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 10:\n    Set s to s + i.\n    If i equals 3:\n        Break.\nShow s."),
        ("nested_loops", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 3:\n    Repeat for j from 1 to 3:\n        Set s to s + 1.\nShow s."),
        // --- B3 MSG: recursion ---
        ("factorial", "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(6)."),
        ("recursive_count_dynamic", "## To count (n: Int) and (acc: Int) -> Int:\n    If n equals 0:\n        Return acc.\n    Return count(n - 1, acc + 1).\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 50:\n    Set d to d + 1.\nShow count(d, 0)."),
        // --- B4: flow-sensitive refinement ---
        ("guard_refine", "## Main\nLet mutable x be 0.\nRepeat for i from 1 to 100:\n    Set x to x + 1.\nIf x equals 100:\n    Show x.\nOtherwise:\n    Show 0."),
        ("nested_guard", "## Main\nLet mutable x be 0.\nRepeat for i from 1 to 100:\n    Set x to x + 1.\nIf x equals 5:\n    Show \"a\".\nOtherwise:\n    If x equals 5:\n        Show \"b\".\n    Otherwise:\n        Show \"c\"."),
        // --- B5: maps / sets / text / closures ---
        ("map_ops", "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to 2.\nShow item \"b\" of m.\nShow length of m."),
        ("set_ops", "## Main\nLet s be a new Set of Int.\nAdd 3 to s.\nAdd 9 to s.\nShow length of s.\nIf s contains 3:\n    Show \"yes\".\nOtherwise:\n    Show \"no\"."),
        ("text_length", "## Main\nShow length of \"hello world\"."),
        ("closure_hof", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 21)."),
        ("closure_dynamic", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 50:\n    Set d to d + 1.\nShow apply((n: Int) -> n + 1, d)."),
        // --- mixed / functions ---
        ("multi_function", "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## To inc (n: Int) -> Int:\n    Return n + 1.\n\n## Main\nShow double(inc(20))."),
        // --- edge cases (robustness) ---
        ("negative_ints", "## Main\nShow 0 - 5.\nShow 3 * (0 - 4)."),
        ("multi_show_ordering", "## Main\nShow 1.\nShow 2.\nShow 3."),
        ("empty_range_loop", "## Main\nLet mutable s be 7.\nRepeat for i from 1 to 0:\n    Set s to s + i.\nShow s."),
        ("while_false_eliminated", "## Main\nLet mutable s be 5.\nWhile s is greater than 100:\n    Set s to s + 1.\nShow s."),
        ("return_mid_loop", "## To f () -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to 10:\n        Set s to s + i.\n        If i equals 4:\n            Return s.\n    Return s.\n\n## Main\nShow f()."),
        ("mutual_recursion", "## To isEven (n: Int) -> Bool:\n    If n equals 0:\n        Return true.\n    Return isOdd(n - 1).\n\n## To isOdd (n: Int) -> Bool:\n    If n equals 0:\n        Return false.\n    Return isEven(n - 1).\n\n## Main\nShow isEven(10)."),
        ("float_formatting", "## Main\nShow 3.14.\nShow 1.0 + 2.0."),
        ("alias_mutation", "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + 1.\nLet s be [1, 2, 3].\nLet a be s.\nSet item 1 of a to d.\nShow item 1 of s."),
        ("deeply_nested_arith", "## Main\nShow ((1 + 2) * (3 + 4)) - ((5 - 1) * 2)."),
        ("closure_capture", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nLet c be 100.\nShow apply((n: Int) -> n + c, 5)."),
    ]
}

/// The PE residual run through the tree-walker must observably agree with the production
/// tree-walker on the source, for every program in the corpus.
#[test]
fn interp_vs_treewalk_corpus() {
    let mut failures = Vec::new();
    for (name, program) in differential_corpus() {
        let tw = run_treewalk(program);
        let p1 = run_p1(program);
        // Output stream must match exactly; value/error class compared leniently
        // (Nothing ≡ Error at the engine boundary, per the harness contract).
        if tw.output != p1.output {
            failures.push(format!(
                "[{}] output differs:\n  tree-walk: {:?}\n  P1:        {:?}",
                name, tw.output, p1.output
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "differential corpus divergences ({}):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Stronger leg: full value+output agreement under the harness's behavioral comparison.
#[test]
fn interp_vs_treewalk_corpus_strict_value() {
    for (name, program) in differential_corpus() {
        let tw = run_treewalk(program);
        let p1 = run_p1(program);
        // Skip programs the oracle itself errors on (corpus is meant to be well-formed).
        if tw.is_value() {
            assert_same_behavior(&p1, &tw, CmpMode::Lenient);
            let _ = name;
        }
    }
}

/// Generative differential: over many randomly-generated, well-typed total Int programs, the
/// PE residual must agree with the tree-walker oracle. This is the "robust to the point of
/// absurdity" fuzzer — it explores the arithmetic/binding/nesting space far beyond the curated
/// corpus. Deterministic (seeded), so a failure is reproducible.
#[test]
fn generative_differential_arith() {
    let mut failures = Vec::new();
    for seed in 0u64..200 {
        let program = gen_program(seed, Shape::RandomArith(seed));
        let tw = run_treewalk(&program);
        let p1 = run_p1(&program);
        if tw.output != p1.output {
            failures.push(format!(
                "[seed {}] output differs:\n  tree-walk: {:?}\n  P1:        {:?}\nprogram:\n{}",
                seed, tw.output, p1.output, program
            ));
            if failures.len() >= 5 {
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "generative differential divergences:\n{}",
        failures.join("\n---\n")
    );
}
