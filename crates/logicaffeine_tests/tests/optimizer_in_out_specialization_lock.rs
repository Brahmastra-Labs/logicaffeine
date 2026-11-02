//! Specialization retains mutable storage identity and parameter positions.
use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;

fn check(source: &str, expected: &str) {
    for (name, run) in [
        ("tree walker", tw_outcome(source)),
        ("VM", vm_outcome(source)),
        (
            "optimized VM",
            vm_outcome_tiered(source, &[], Tier::T3, None),
        ),
    ] {
        assert_eq!(run.error, None, "{name}: {run:?}");
        assert_eq!(run.output.trim_end_matches('\n'), expected, "{name}");
    }
}

#[test]
fn specializing_a_later_argument_preserves_the_first_loan() {
    check("## To bump (value: mutable Int, delta: Int) -> Int:\n    Set value to value + delta.\n    Return value.\n## Main\nLet n be 4.\nShow bump(n, 3).\nShow n.\n", "7\n7");
}
#[test]
fn specializing_an_earlier_argument_remaps_the_loan_position() {
    check("## To bump (delta: Int, value: mutable Int) -> Int:\n    Set value to value + delta.\n    Return value.\n## Main\nLet n be 4.\nShow bump(3, n).\nShow n.\n", "7\n7");
}
#[test]
fn specialization_preserves_two_mutable_outputs() {
    check("## To change (left: mutable Int, delta: Int, right: mutable Int) -> Int:\n    Set left to left + delta.\n    Set right to right + delta.\n    Return left + right.\n## Main\nLet a be 1.\nLet b be 2.\nShow change(a, 3, b).\nShow a.\nShow b.\n", "9\n4\n5");
}
#[test]
fn a_mutable_temporary_is_never_substituted_as_a_constant() {
    check("## To change (value: mutable Int, delta: Int) -> Int:\n    Set value to value + 1.\n    Return value + delta.\n## No Inline\n## To invoke (delta: Int) -> Int:\n    Return change(4, delta).\n## Main\nShow invoke(3).\n", "8");
}
#[test]
fn an_ordinary_static_argument_still_specializes() {
    let source = "## To bump (value: mutable Int, delta: Int) -> Int:\n    Set value to value + delta.\n    Return value.\n## Main\nLet n be 4.\nShow bump(n, 3).\nShow n.\n";
    logicaffeine_compile::ui_bridge::with_optimized_program_tiered(
        source,
        Tier::T3,
        |parsed, _| {
            let (stmts, _, _) = parsed.unwrap();
            assert!(stmts.iter().any(|s| matches!(s,
            logicaffeine_compile::ast::stmt::Stmt::FunctionDef { params, .. }
                if params.len() == 1 && matches!(params[0].1, logicaffeine_compile::ast::stmt::TypeExpr::Mutable { .. })
        )), "the static argument should be removed while the mutable parameter remains");
        },
    );
}
