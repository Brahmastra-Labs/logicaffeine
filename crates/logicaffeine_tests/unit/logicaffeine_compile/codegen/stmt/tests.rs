use super::*;
use logicaffeine_language::ast_depth::{for_each_node, Visited};

/// Each access `item i of <arr>` in the first `Repeat` of `source`, as `<arr> raw` when the fixed-array rule
/// marks it in range and `<arr> checked` when it keeps the rule, with each array's Rust type from `types`.
fn marks(source: &str, types: &[(&str, &str)]) -> Vec<String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let (counter, iterable, body) = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::Repeat {
                    pattern: Pattern::Identifier(counter),
                    iterable,
                    body,
                } => Some((*counter, *iterable, &body[..])),
                _ => None,
            })
            .expect("a counted loop");
        let var_types: HashMap<Symbol, String> = types
            .iter()
            .map(|(name, ty)| {
                (
                    interner.lookup(name).expect("the array is named"),
                    ty.to_string(),
                )
            })
            .collect();
        let _oracle = super::super::expr::install_range_oracle(Some(std::rc::Rc::new(
            crate::optimize::OracleFacts::default(),
        )));
        mark_fixed_array_counter_accesses(counter, iterable, body, &var_types);
        let mut out = Vec::new();
        for_each_node(body, |node| {
            let (collection, index) = match node {
                Visited::Expr(Expr::Index { collection, index }) => (*collection, *index),
                Visited::Stmt(Stmt::SetIndex {
                    collection, index, ..
                }) => (*collection, *index),
                _ => return,
            };
            let Expr::Identifier(arr) = collection else {
                return;
            };
            let proven = super::super::expr::range_proves_index(collection, index);
            out.push(format!(
                "{} {}",
                interner.resolve(*arr),
                if proven { "raw" } else { "checked" }
            ));
        });
        out.sort();
        out
    })
}

const FIXED: &[(&str, &str)] = &[("buf", "[i64; 8]")];

fn main_with(repeat: &str) -> String {
    format!("## Main\nLet mutable buf be a new Seq of Int.\nLet mutable total be 0.\n{repeat}Show total.\n")
}

#[test]
fn a_literal_range_inside_a_fixed_array_marks_its_accesses() {
    let source =
        main_with("Repeat for i from 1 to 8:\n    Set item i of buf to (item i of buf) + 1.\n");
    assert_eq!(marks(&source, FIXED), ["buf raw", "buf raw"]);
    let source = main_with("Repeat for i from 3 to 8:\n    Set total to total + item i of buf.\n");
    assert_eq!(marks(&source, FIXED), ["buf raw"]);
}

#[test]
fn a_range_reaching_past_the_array_keeps_the_rule() {
    let source = main_with("Repeat for i from 1 to 9:\n    Set total to total + item i of buf.\n");
    assert_eq!(marks(&source, FIXED), ["buf checked"]);
}

#[test]
fn a_range_starting_below_one_keeps_the_rule() {
    let source = main_with("Repeat for i from 0 to 7:\n    Set total to total + item i of buf.\n");
    assert_eq!(marks(&source, FIXED), ["buf checked"]);
}

#[test]
fn a_sequence_of_unknown_length_keeps_the_rule() {
    let source = main_with("Repeat for i from 1 to 8:\n    Set total to total + item i of buf.\n");
    assert_eq!(marks(&source, &[("buf", "Vec<i64>")]), ["buf checked"]);
    assert_eq!(marks(&source, &[("buf", "LogosSeq<i64>")]), ["buf checked"]);
}

#[test]
fn a_counter_bound_anew_in_the_body_keeps_the_rule() {
    let nested = main_with("Repeat for i from 1 to 8:\n    Repeat for i from 1 to 20:\n        Set total to total + item i of buf.\n");
    assert_eq!(marks(&nested, FIXED), ["buf checked"]);
}

#[test]
fn an_access_inside_a_closure_keeps_the_rule() {
    let source = "## Main\nLet mutable buf be a new Seq of Int.\nLet mutable other be a new Seq of Int.\nLet mutable total be 0.\nRepeat for i from 1 to 8:\n    Let pick be (i: Int) -> item i of other.\n    Set total to total + item i of buf + pick(i + 1).\nShow total.\n";
    assert_eq!(
        marks(source, &[("buf", "[i64; 8]"), ("other", "[i64; 8]")]),
        ["buf raw", "other checked"]
    );
}

#[test]
fn bounded_count_proof_purity_refuses_a_parameter_shadowing_a_pure_function() {
    let source = "## To chosen (n: Int) -> Int:\n    Return n + 1.\n## To apply (chosen: fn(Int) -> Int) and (x: Int) -> Int:\n    Return chosen(x).\n## Main\nShow 0.\n";
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("function-shadow fixture parses");
        let purity = crate::optimize::purity::Purity::of_program(&stmts, interner);
        assert!(purity.is_pure(interner.lookup("chosen").unwrap()));
        assert!(
            !purity.is_pure(interner.lookup("apply").unwrap()),
            "a function parameter can invoke an impure callback despite its global namesake"
        );
    });
}

#[test]
fn bounded_count_proof_purity_refuses_a_parameter_shadowing_a_pure_builtin() {
    let source = "## To apply (abs: fn(Int) -> Int) and (x: Int) -> Int:\n    Return abs(x).\n## Main\nShow 0.\n";
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("builtin-shadow fixture parses");
        let purity = crate::optimize::purity::Purity::of_program(&stmts, interner);
        assert!(
            !purity.is_pure(interner.lookup("apply").unwrap()),
            "a function parameter is not the builtin with the same spelling"
        );
    });
}

fn count_is_certified(source: &str) -> bool {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("count proof fixture parses");
        let count = interner.lookup("count").expect("count binding");
        collect_bounded_count_accumulators(&stmts, &Default::default()).contains(&count)
    })
}

#[test]
fn bounded_count_proof_certifies_one_iteration_domain() {
    assert!(count_is_certified("## Main\nLet count be 0.\nLet i be 0.\nWhile i < 100:\n    Set count to count + 1.\n    Set i to i + 1.\nShow count.\n"));
}

#[test]
fn bounded_count_proof_rejects_nested_iteration_domains() {
    let source = r#"## Main
Let count be 0.
Let i be 0.
While i < 9223372036854775807:
    Let j be 0.
    While j < 2:
        Let k be 0.
        While k < 2:
            Set count to count + 1.
            Set k to k + 1.
        Set j to j + 1.
    Set i to i + 1.
Show count.
"#;
    assert!(
        !count_is_certified(source),
        "a bound per inner loop does not bound the accumulated outer work"
    );
}

#[test]
fn bounded_count_proof_rejects_effectful_expression_calls() {
    let source = r#"## No Inline
## To raise (count: mutable Int) -> Int:
    Set count to 2 ** 100.
    Return 0.
## Main
Let count be 0.
Let i be 0.
While i < 1:
    Show raise(count).
    Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
    assert!(
        !count_is_certified(source),
        "a call may change an accumulator outside its visible increment"
    );
}

#[test]
fn bounded_count_proof_accepts_an_inclusive_word_bound_from_a_positive_start() {
    for start in [1, 2, 99] {
        let source = format!("## Main\nLet limit be 9223372036854775807.\nLet count be 0.\nLet i be {start}.\nWhile i <= limit:\n    Set count to count + 1.\n    Set i to i + 1.\nShow count.\n");
        assert!(
            count_is_certified(&source),
            "positive start {start} bounds the count by i64::MAX"
        );
    }
}

#[test]
fn bounded_count_proof_rejects_an_inclusive_word_bound_from_zero_or_negative() {
    for setup in [
        "Let i be 0.",
        "Let i be 0 - 1.",
        "Let i be 2.\nSet i to 0.",
        "Let i be 2.\nIf limit > 0:\n    Set i to 0.",
    ] {
        let source = format!("## Main\nLet limit be 9223372036854775807.\nLet count be 0.\n{setup}\nWhile i <= limit:\n    Set count to count + 1.\n    Set i to i + 1.\nShow count.\n");
        assert!(
            !count_is_certified(&source),
            "{setup}: the count may exceed i64::MAX"
        );
    }
}

#[test]
fn bounded_count_proof_rejects_an_exact_inclusive_bound() {
    let source = "## Main\nLet limit be 2 ** 100.\nLet count be 0.\nLet i be 2.\nWhile i <= limit:\n    Set count to count + 1.\n    Set i to i + 1.\nShow count.\n";
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("exact bound fixture parses");
        let limit = interner.lookup("limit").unwrap();
        let count = interner.lookup("count").unwrap();
        let exact = [limit].into_iter().collect();
        assert!(!collect_bounded_count_accumulators(&stmts, &exact).contains(&count));
    });
}
