use crate::analysis::known_calls::harvest_calls;
use crate::ast::stmt::Stmt;
use crate::ui_bridge::with_parsed_program;

/// The names of every function the program, as the seam hands it on, calls.
fn calls(source: &str) -> Vec<String> {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        harvest_calls(stmts)
            .calls
            .iter()
            .map(|c| interner.resolve(c.function).to_string())
            .collect()
    })
}

/// How many `While` loops the program as handed on holds, at any depth.
fn whiles(source: &str) -> usize {
    fn count(block: &[Stmt<'_>]) -> usize {
        block
            .iter()
            .map(|s| match s {
                Stmt::While { body, .. } => 1 + count(body),
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => count(then_block) + else_block.map_or(0, count),
                Stmt::Repeat { body, .. } | Stmt::Splice { body } => count(body),
                Stmt::FunctionDef { body, .. } => count(body),
                _ => 0,
            })
            .sum()
    }
    with_parsed_program(source, |parsed, _| count(parsed.expect("the program parses").0))
}

const COUNTING: &str = "Let i be 0.\nLet s be 0.\nWhile i < 10:\n    Set s to s + i.\n    Set i to i + 1.\nShow s.";

#[test]
fn a_program_without_a_supertask_is_handed_on_as_it_was_parsed() {
    let plain = format!("## Main\n{COUNTING}");
    assert!(!calls(&plain).iter().any(|c| c == "tripCount"), "{:?}", calls(&plain));
    assert_eq!(whiles(&plain), 1);
}

#[test]
fn a_constant_step_is_left_as_written_even_beside_an_infinitesimal() {
    let program = format!("## Main\nLet dt be an infinitesimal.\n{COUNTING}");
    assert!(!calls(&program).iter().any(|c| c == "tripCount"), "{:?}", calls(&program));
    assert_eq!(whiles(&program), 1);
}

/// Imamura's Lemma 1: a counter stepped by a constant toward an exact value.
const FLOOR: &str = "## To floorOf (x: Hyperreal) -> Hyperreal:
    Let n be 0.
    While not ((n <= x and x < n + 1) or (0 - n <= x and x < 0 - n + 1)):
        Set n to n + 1.
    Return n.

## Main
Let dt be an infinitesimal.
Show floorOf(1 / dt).";

#[test]
fn a_guard_linear_in_its_counter_that_reads_an_exact_value_is_counted_by_its_first_exit() {
    let called = calls(FLOOR);
    assert!(called.iter().any(|c| c == "firstExit"), "{called:?}");
    assert!(called.iter().any(|c| c == "affinePower"), "{called:?}");
    assert!(!called.iter().any(|c| c == "tripCount"), "{called:?}");
    assert_eq!(whiles(FLOOR), 1, "the loop as written survives as the finite branch");
}

#[test]
fn a_constant_step_toward_an_exact_bound_is_a_clock() {
    let program = "## To countTo (x: Hyperreal) -> Hyperreal:
    Let n be 0.
    While n < x:
        Set n to n + 1.
    Return n.

## Main
Let dt be an infinitesimal.
Show countTo(1 / dt).";
    let called = calls(program);
    assert!(called.iter().any(|c| c == "tripCount"), "{called:?}");
    assert!(!called.iter().any(|c| c == "firstExit"), "{called:?}");
}

#[test]
fn a_guard_that_reads_no_exact_value_keeps_its_constant_step_as_written() {
    let program = "## Main
Let dt be an infinitesimal.
Let x be 7.
Let n be 0.
While not (n <= x and x < n + 1):
    Set n to n + 1.
Show n.";
    let called = calls(program);
    assert!(!called.iter().any(|c| c == "firstExit" || c == "tripCount"), "{called:?}");
}

#[test]
fn a_guard_that_is_not_linear_in_its_counter_is_left_as_written() {
    for guard in ["n * n < x", "n < x and n < m", "(n + 1) % n = 0 or n < x"] {
        let program = format!(
            "## To f (x: Hyperreal) -> Hyperreal:
    Let n be 1.
    Let m be 1.
    While {guard}:
        Set n to n + 1.
        Set m to m + 1.
    Return n.

## Main
Let dt be an infinitesimal.
Show f(1 / dt)."
        );
        let called = calls(&program);
        assert!(!called.iter().any(|c| c == "firstExit" || c == "tripCount"), "{guard}: {called:?}");
    }
}

#[test]
fn a_clock_stepped_by_a_variable_keeps_the_loop_for_its_finite_case() {
    let program = "## Main
Let dt be an infinitesimal.
Let t be 0.
Let n be 0.
While t < 1:
    Set t to t + dt.
    Set n to n + 1.
Show n.";
    let called = calls(program);
    assert!(called.iter().any(|c| c == "tripCount"), "{called:?}");
    assert!(called.iter().any(|c| c == "affinePower"), "{called:?}");
    assert_eq!(whiles(program), 1, "the loop as written survives as the finite branch");
}

#[test]
fn a_search_with_a_linear_predicate_takes_the_exact_reading_first() {
    let program = "## Main
Let found be false.
Repeat for every natural n:
    If n * 3 = 12:
        Set found to true.
Show found.";
    let called = calls(program);
    assert!(called.iter().any(|c| c == "searchCandidates"), "{called:?}");
    assert!(called.iter().any(|c| c == "provisional"), "the budgeted run stays for coefficients the reading cannot decide: {called:?}");
    assert!(!called.iter().any(|c| c == "everyNatural"), "no search reaches a tier unresolved: {called:?}");
}

#[test]
fn a_search_beyond_the_linear_reading_runs_its_budget() {
    let program = "## Main
Let found be false.
Repeat for every natural n, checking up to 50 cases:
    If n * n = 49:
        Set found to true.
Show found.";
    let called = calls(program);
    assert!(!called.iter().any(|c| c == "searchCandidates"), "{called:?}");
    assert!(called.iter().any(|c| c == "provisional"), "{called:?}");
}

#[test]
fn a_search_with_no_stated_budget_checks_ten_million_cases() {
    let program = "## Main
Let found be false.
Repeat for every natural n:
    If n * n = 2:
        Set found to true.
Show found.";
    let budgets: Vec<String> = with_parsed_program(program, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        harvest_calls(stmts)
            .calls
            .iter()
            .filter(|c| interner.resolve(c.function) == "provisional")
            .map(|c| format!("{:?}", c.args[1]))
            .collect()
    });
    assert!(!budgets.is_empty());
    assert!(budgets.iter().all(|b| b.contains("10000000")), "{budgets:?}");
}

#[test]
fn a_polynomial_euler_step_resolves_to_a_flow() {
    let program = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1 / 2.
Let t be 0.
While t < 1:
    Set x to x + x * (1 - x) * dt.
    Set t to t + dt.
Show x.";
    let called = calls(program);
    assert!(called.iter().any(|c| c == "polynomialFlow"), "{called:?}");
    assert!(!called.iter().any(|c| c == "affinePower"), "{called:?}");
    assert!(!called.iter().any(|c| c == "cycleAt"), "{called:?}");
}

#[test]
fn a_polynomial_update_without_the_infinitesimal_is_left_to_the_cycle() {
    let program = "## Main
Let dt be an infinitesimal.
Let x be 2.
Let t be 0.
While t < 1:
    Set x to x * x.
    Set t to t + dt.
Show x.";
    let called = calls(program);
    assert!(!called.iter().any(|c| c == "polynomialFlow"), "{called:?}");
    assert!(called.iter().any(|c| c == "cycleAt"), "{called:?}");
}

#[test]
fn an_affine_update_keeps_its_closed_form_beside_polynomial_flows() {
    let program = "## Main
Let dt be an infinitesimal.
Let x be 1.
Let t be 0.
While t < 1:
    Set x to x + x * dt.
    Set t to t + dt.
Show x.";
    let called = calls(program);
    assert!(called.iter().any(|c| c == "affinePower"), "{called:?}");
    assert!(!called.iter().any(|c| c == "polynomialFlow"), "{called:?}");
}

#[test]
fn a_polynomial_map_whose_monomials_close_keeps_the_closed_form() {
    let program = "## Main
Let dt be an infinitesimal.
Let total be 0.
Let t be 0.
While t < 1:
    Set total to total + t * t * dt.
    Set t to t + dt.
Show total.";
    let called = calls(program);
    assert!(called.iter().any(|c| c == "affinePower"), "{called:?}");
    assert!(!called.iter().any(|c| c == "polynomialFlow"), "{called:?}");
}

const BALL: &str = "## Main
Let dt be an infinitesimal.
Let h: Hyperreal be 0.
Let v: Hyperreal be 10.
Let time: Hyperreal be 0.
While time < 5:
    Set h to h + v * dt.
    Set v to v - 10 * dt.
    If h <= 0 and v < 0:
        Set h to 0.
        Set v to 0 - v / 2.
    Set time to time + dt.
Show h.";

#[test]
fn a_flow_with_a_jump_resolves_to_a_hybrid_flow_and_keeps_its_finite_case() {
    let called = calls(BALL);
    assert!(called.iter().any(|c| c == "hybridFlow"), "{called:?}");
    for other in ["affinePower", "polynomialFlow", "cycleAt"] {
        assert!(!called.iter().any(|c| c == other), "{other} in {called:?}");
    }
    assert_eq!(whiles(BALL), 1, "the loop as written stays for a standard step");
}

#[test]
fn a_jump_that_reads_only_the_clock_is_a_hybrid_flow() {
    let program = "## Main
Let dt be an infinitesimal.
Let fired be false.
Let time: Hyperreal be 0.
While time < 2:
    If time >= 1 and not fired:
        Set fired to true.
    Set time to time + dt.
Show fired.";
    let called = calls(program);
    assert!(called.iter().any(|c| c == "hybridFlow"), "{called:?}");
}

#[test]
fn a_toggle_that_neither_flows_nor_reads_the_clock_is_left_to_the_cycle() {
    let program = "## Main
Let dt be an infinitesimal.
Let lamp be 0.
Let t be 0.
While t < 1:
    If lamp = 0:
        Set lamp to 1.
    Otherwise:
        Set lamp to 0.
    Set t to t + dt.
Show lamp.";
    let called = calls(program);
    assert!(called.iter().any(|c| c == "cycleAt"), "{called:?}");
    assert!(!called.iter().any(|c| c == "hybridFlow"), "{called:?}");
}

#[test]
fn a_flow_beside_a_jump_with_an_otherwise_is_refused_before_any_builtin() {
    let program = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 0.
Let t be 0.
While t < 1:
    Set x to x + dt.
    If x > 1 / 2:
        Set x to 0.
    Otherwise:
        Set x to x + dt.
    Set t to t + dt.
Show x.";
    let called = calls(program);
    for name in ["hybridFlow", "cycleAt", "affinePower", "polynomialFlow"] {
        assert!(!called.iter().any(|c| c == name), "{name} in {called:?}");
    }
    assert!(called.iter().any(|c| c == "tripCount"), "{called:?}");
}
