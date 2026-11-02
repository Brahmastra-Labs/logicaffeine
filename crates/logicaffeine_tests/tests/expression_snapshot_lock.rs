//! Values already read stay stable while later operands replace their original bindings.
use logicaffeine_compile::compile::{tw_lines, vm_outcome};

fn holds(source: &str, expected: &str) {
    let reference = tw_lines(source);
    assert_eq!(reference.error, None, "tree-walker: {source}");
    let output: String = reference
        .lines
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(output, expected, "tree-walker: {source}");
    let vm = vm_outcome(source);
    assert_eq!(vm.error, None, "VM: {source}");
    assert_eq!(vm.output, expected, "VM: {source}");
}

#[test]
fn index_preserves_the_collection_before_the_index_mutates_it() {
    holds("## To choose (xs: mutable Seq of Int) -> Int:\n    Set xs to [9].\n    Return 1.\n\n## Main\nLet mutable outer be [[7, 8]].\nPop from outer into values.\nShow item (choose(values)) of values.\nShow values.\n", "7\n[9]\n");
}

#[test]
fn contains_preserves_the_collection_before_the_needle_mutates_it() {
    holds("## To choose (xs: mutable Seq of Int) -> Int:\n    Set xs to [9].\n    Return 7.\n\n## Main\nLet mutable outer be [[7, 8]].\nPop from outer into values.\nShow values contains choose(values).\nShow values.\n", "true\n[9]\n");
}

#[test]
fn slice_preserves_the_collection_before_a_bound_mutates_it() {
    holds("## To lower (xs: mutable Seq of Int) -> Int:\n    Set xs to [9].\n    Return 1.\n\n## Main\nLet mutable outer be [[7, 8]].\nPop from outer into values.\nShow items (lower(values)) through 1 of values.\nShow values.\n", "[7]\n[9]\n");
}

#[test]
fn scalar_left_operand() {
    holds("## To replace (n: mutable Int) -> Int:\n    Set n to 9.\n    Return 1.\n\n## Main\nLet mutable value be 7.\nShow value + replace(value).\nShow value.\n", "8\n9\n");
}

#[test]
fn slice_start_before_end() {
    holds("## To replace (n: mutable Int) -> Int:\n    Set n to 2.\n    Return 2.\n\n## Main\nLet values be [7, 8].\nLet mutable first be 1.\nShow items first through (replace(first)) of values.\nShow first.\n", "[7, 8]\n2\n");
}

#[test]
fn pure_collection_control() {
    holds("## Main\nLet values be [7, 8].\nShow item 1 of values.\nShow values contains 7.\nShow items 1 through 1 of values.\n", "7\ntrue\n[7]\n");
}

#[test]
fn union_left_operand() {
    holds("## To replace (values: mutable Set of Int) -> Set of Int:\n    Set values to a new Set of Int.\n    Add 9 to values.\n    Let result be a new Set of Int.\n    Add 7 to result.\n    Return result.\n\n## Main\nLet mutable values be a new Set of Int.\nAdd 7 to values.\nAdd 8 to values.\nLet result be values union replace(values).\nShow length of result.\nShow result contains 7.\nShow result contains 8.\nShow result contains 9.\n", "2\ntrue\ntrue\nfalse\n");
}

#[test]
fn intersection_left_operand() {
    holds("## To replace (values: mutable Set of Int) -> Set of Int:\n    Set values to a new Set of Int.\n    Add 9 to values.\n    Let result be a new Set of Int.\n    Add 7 to result.\n    Return result.\n\n## Main\nLet mutable values be a new Set of Int.\nAdd 7 to values.\nAdd 8 to values.\nLet result be values intersection replace(values).\nShow length of result.\nShow result contains 7.\nShow result contains 8.\nShow result contains 9.\n", "1\ntrue\nfalse\nfalse\n");
}

#[test]
fn map_key_before_value() {
    holds("## To replace (key: mutable Int) -> Int:\n    Set key to 9.\n    Return 1.\n\n## Main\nLet mutable key be 7.\nLet values be {key: replace(key)}.\nShow item 7 of values.\nShow key.\n", "1\n9\n");
}

#[test]
fn range_start_before_end() {
    holds("## To replace (first: mutable Int) -> Int:\n    Set first to 2.\n    Return 2.\n\n## Main\nLet mutable first be 1.\nLet mutable values be a new Seq of Int.\nRepeat for n from first to (replace(first)):\n    Push n to values.\nShow values.\nShow first.\n", "[1, 2]\n2\n");
}

#[test]
fn named_callee_before_argument() {
    holds("## To replace (f: mutable fn(Int) -> Int) -> Int:\n    Set f to (n: Int) -> n + 100.\n    Return 1.\n\n## Main\nLet mutable callback be (n: Int) -> n + 10.\nShow callback(replace(callback)).\nShow callback(1).\n", "11\n101\n");
}

#[test]
fn expression_callee_before_argument() {
    use logicaffeine_compile::{
        ast::stmt::{Expr, Stmt},
        ui_bridge, vm,
    };
    // The source grammar names callees; lower that call to the general AST form.
    let source = "## To replace (f: mutable fn(Int) -> Int) -> Int:\n    Set f to (n: Int) -> n + 100.\n    Return 1.\n\n## Main\nLet mutable callback be (n: Int) -> n + 10.\nShow callback(replace(callback)).\nShow callback(1).\n";
    let expected = "11\n101\n";
    let reference = tw_lines(source);
    assert_eq!(reference.error, None);
    let output: String = reference
        .lines
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(output, expected);
    ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("valid named-call source");
        let index = stmts
            .iter()
            .position(|stmt| {
                matches!(stmt,
                    Stmt::Show { object: Expr::Call { function, .. }, .. }
                        if interner.resolve(*function) == "callback"
                )
            })
            .expect("the first callback invocation");
        let Stmt::Show {
            object: Expr::Call { function, args },
            recipient,
        } = &stmts[index]
        else {
            unreachable!()
        };
        let callee = Expr::Identifier(*function);
        let call = Expr::CallExpr {
            callee: &callee,
            args: args.clone(),
        };
        let mut program = stmts.to_vec();
        program[index] = Stmt::Show {
            object: &call,
            recipient,
        };
        let (output, error) = vm::run_to_outcome(&program, interner, Some(types), Some(&policies));
        assert_eq!(error, None);
        assert_eq!(output, expected);
    });
}

#[test]
fn a_nested_call_preserves_the_outer_left_operand() {
    holds("## To replace (n: mutable Int) -> Int:\n    Set n to 9.\n    Return 1.\n\n## Main\nLet mutable value be 7.\nShow value + (2 * replace(value)).\nShow value.\n", "9\n9\n");
}
