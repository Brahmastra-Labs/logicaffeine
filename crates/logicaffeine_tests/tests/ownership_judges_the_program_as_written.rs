//! Ownership and zone escapes are properties of the program its author wrote, so every compile
//! path judges the freshly-parsed program — never what the optimizer made of it.
//!
//! A `Give` inside a branch or a loop body may or may not run, so a later use of the value is a
//! maybe-move: the flow checker's to refuse, since the parser refuses only a move that is certain
//! in its own block (`a_move_is_certain_only_in_its_own_block.rs`). Constant propagation rewrites
//! `Give x to p.` into `Give 5 to p.` and `Show x.` into `Show 5.` once `x` is a known constant,
//! and folding flattens a constant `If` into its body. A checker fed the optimized program sees no
//! `x` at all, or sees a certain move where the author wrote a possible one. The editor (the LSP)
//! always judged the parse, so the two disagreed. Each shape below pins the verdict on the program
//! as written, whatever the value, the construct, or the optimization config.

use std::path::Path;

use logicaffeine_base::{Arena, Interner};
use logicaffeine_compile::analysis::{EscapeChecker, OwnershipChecker};
use logicaffeine_compile::compile::{
    compile_and_run, compile_project, compile_to_rust, compile_to_rust_checked, CompileError,
};
use logicaffeine_compile::optimization::{decorate_source, REGISTRY};
use logicaffeine_language::{
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{Expr, Stmt, TypeExpr},
    drs::WorldState,
    error::ParseErrorKind,
    Lexer, Parser,
};

// ---------------------------------------------------------------------------------------------
// The programs
// ---------------------------------------------------------------------------------------------

const CONSTANT_GIVEN_IN_A_BRANCH: &str = "\
## Main
Let x be 5.
If x > 0:
    Give x to processor.
Show x.
";

const CONSTANT_GIVEN_IN_AN_INLINE_BRANCH: &str = "\
## Main
Let x be 5.
If x > 0: Give x to processor.
Show x.
";

const CONSTANT_GIVEN_IN_THE_OTHERWISE_BRANCH: &str = "\
## Main
Let x be 5.
If x > 9:
    Show 1.
Otherwise:
    Give x to processor.
Show x.
";

const TEXT_CONSTANT_GIVEN_IN_A_BRANCH: &str = "\
## Main
Let x be \"hello\".
Let c be true.
If c:
    Give x to processor.
Show x.
";

const FOLDED_SUM_GIVEN_IN_A_BRANCH: &str = "\
## Main
Let x be 2 + 3.
If x > 4:
    Give x to processor.
Show x.
";

const FUNCTION_RESULT_GIVEN_IN_A_BRANCH: &str = "\
## To five () -> Int:
    Return 5.

## Main
Let x be five().
If x > 4:
    Give x to processor.
Show x.
";

const CONSTANT_GIVEN_IN_A_WHILE_BODY: &str = "\
## Main
Let x be 5.
Let mutable n be 0.
While n < 1:
    Give x to processor.
    Set n to n + 1.
Show x.
";

const CONSTANT_GIVEN_IN_A_REPEAT_BODY: &str = "\
## Main
Let x be 5.
Repeat for i from 1 to 2:
    Give x to processor.
Show x.
";

const CONSTANT_GIVEN_DEEP_IN_NESTED_BLOCKS: &str = "\
## Main
Let x be 5.
Let c be true.
If c:
    Repeat for i from 1 to 2:
        If i = 2:
            Give x to processor.
Show x.
";

const CONSTANT_GIVEN_AS_A_CALL_ARGUMENT_IN_A_BRANCH: &str = "\
## To keep (n: Int):
    Show n.

## Main
Let x be 5.
If x > 0:
    Call keep with give x.
Show x.
";

const CONSTANT_GIVEN_IN_A_BRANCH_OF_A_FUNCTION: &str = "\
## To f (c: Bool) -> Int:
    Let x be 5.
    If c:
        Give x to processor.
    Show x.
    Return 0.

## Main
Show f(true).
";

const CONSTANT_GIVEN_IN_ONE_INSPECT_ARM: &str = "\
## Definition
A Color is either:
    A Red.
    A Blue.

## Main
Let x be 5.
Let c be a new Red.
Inspect c:
    If it is a Red:
        Give x to processor.
    If it is a Blue:
        Show 1.
Show x.
";

const POSSIBLY_GIVEN_THEN_BOUND: &str = "\
## Main
Let x be 5.
Let c be true.
If c:
    Give x to processor.
Let y be x + 1.
Show y.
";

const POSSIBLY_GIVEN_THEN_ASSIGNED: &str = "\
## Main
Let x be 5.
Let mutable y be 0.
Let c be true.
If c:
    Give x to processor.
Set y to x.
Show y.
";

const POSSIBLY_GIVEN_THEN_RETURNED: &str = "\
## To f (c: Bool) -> Int:
    Let x be 5.
    If c:
        Give x to processor.
    Return x.

## Main
Show f(true).
";

const POSSIBLY_GIVEN_THEN_PASSED: &str = "\
## To keep (n: Int):
    Show n.

## Main
Let x be 5.
Let c be true.
If c:
    Give x to processor.
Call keep with x.
";

const POSSIBLY_GIVEN_THEN_GIVEN_AGAIN: &str = "\
## Main
Let x be 5.
Let c be true.
If c:
    Give x to processor.
Give x to other.
";

const POSSIBLY_GIVEN_THEN_TESTED: &str = "\
## Main
Let x be 5.
Let c be true.
If c:
    Give x to processor.
If x > 3:
    Show 1.
";

const POSSIBLY_GIVEN_THEN_STEERING_A_LOOP: &str = "\
## Main
Let x be 5.
Let c be true.
If c:
    Give x to processor.
While x > 3:
    Show 1.
";

const POSSIBLY_GIVEN_THEN_WALKED: &str = "\
## Main
Let xs be [1, 2].
Let c be true.
If c:
    Give xs to processor.
Repeat for v in xs:
    Show v.
";

const POSSIBLY_GIVEN_THEN_INSPECTED: &str = "\
## Definition
A Color is either:
    A Red.
    A Blue.

## Main
Let c be a new Red.
Let d be true.
If d:
    Give c to keeper.
Inspect c:
    If it is a Red:
        Show 1.
    Otherwise:
        Show 2.
";

const GIVEN_IN_BOTH_BRANCHES: &str = "\
## Main
Let x be 5.
Let c be true.
If c:
    Give x to processor1.
Otherwise:
    Give x to processor2.
Show x.
";

const GIVEN_IN_ONE_BRANCH_SHOWN_IN_THE_OTHER: &str = "\
## Main
Let x be 5.
Let c be true.
If c:
    Give x to processor.
Otherwise:
    Show x.
";

const GIVEN_IN_ONE_INLINE_BRANCH_SHOWN_IN_THE_OTHER: &str = "\
## Main
Let x be 5.
Let c be true.
If c: Give x to processor.
Otherwise: Show x.
";

const GIVEN_IN_ONE_INSPECT_ARM_SHOWN_IN_ANOTHER: &str = "\
## Definition
A Color is either:
    A Red.
    A Blue.

## Main
Let x be 5.
Let c be a new Red.
Inspect c:
    If it is a Red:
        Give x to processor.
    If it is a Blue:
        Show x.
";

const GIVEN_IN_A_BRANCH_NEVER_USED_AGAIN: &str = "\
## Main
Let x be 5.
Let c be true.
If c:
    Give x to processor.
Show 1.
";

const BOUND_AFRESH_AND_GIVEN_ON_EVERY_PASS: &str = "\
## Main
Repeat for i from 1 to 3:
    Let y be [i].
    Give y to keeper.
";

const SHOWN_BEFORE_IT_IS_GIVEN_IN_A_BRANCH: &str = "\
## Main
Let x be 5.
Let c be true.
Show x.
If c:
    Give x to processor.
";

const SEQUENCE_GIVEN_IN_A_CONSTANT_TRUE_BRANCH: &str = "\
## Main
Let x be [1, 2].
Let c be true.
If c:
    Give x to processor.
Show x.
";

const NEW_SEQUENCE_GIVEN_IN_A_REPEAT_BODY: &str = "\
## Main
Let x be a new Seq of Int.
Repeat for i from 1 to 2:
    Give x to processor.
Show x.
";

const GIVEN_TWICE_IN_A_ROW: &str = "\
## Main
Let x be 5.
Give x to processor1.
Give x to processor2.
";

const ZONE_ESCAPE_IN_A_BRANCH_THAT_NEVER_RUNS: &str = "\
## Main
Let outer be 0.
If 1 > 2:
    Inside a new zone called \"Scratch\":
        Let x be 5.
        Set outer to x.
Show outer.
";

const ZONE_ESCAPE: &str = "\
## Main
Let outer be 0.
Inside a new zone called \"Scratch\":
    Let x be 5.
    Set outer to x.
Show outer.
";

const ZONE_ESCAPE_INSIDE_A_DURING_SPAN: &str = "\
## Main
Let outer be 0.
During scratching:
    Inside a new zone called \"Scratch\":
        Let x be 5.
        Set outer to x.
Show outer.
";

const CONSTANT_GIVEN_IN_A_BRANCH_INSIDE_A_DURING_SPAN: &str = "\
## Main
Let x be 5.
During handing:
    If x > 0:
        Give x to processor.
Show x.
";

/// Every program above, for the invariants that must hold for all of them.
const EVERY_SHAPE: &[(&str, &str)] = &[
    ("CONSTANT_GIVEN_IN_A_BRANCH", CONSTANT_GIVEN_IN_A_BRANCH),
    ("CONSTANT_GIVEN_IN_AN_INLINE_BRANCH", CONSTANT_GIVEN_IN_AN_INLINE_BRANCH),
    ("CONSTANT_GIVEN_IN_THE_OTHERWISE_BRANCH", CONSTANT_GIVEN_IN_THE_OTHERWISE_BRANCH),
    ("TEXT_CONSTANT_GIVEN_IN_A_BRANCH", TEXT_CONSTANT_GIVEN_IN_A_BRANCH),
    ("FOLDED_SUM_GIVEN_IN_A_BRANCH", FOLDED_SUM_GIVEN_IN_A_BRANCH),
    ("FUNCTION_RESULT_GIVEN_IN_A_BRANCH", FUNCTION_RESULT_GIVEN_IN_A_BRANCH),
    ("CONSTANT_GIVEN_IN_A_WHILE_BODY", CONSTANT_GIVEN_IN_A_WHILE_BODY),
    ("CONSTANT_GIVEN_IN_A_REPEAT_BODY", CONSTANT_GIVEN_IN_A_REPEAT_BODY),
    ("CONSTANT_GIVEN_DEEP_IN_NESTED_BLOCKS", CONSTANT_GIVEN_DEEP_IN_NESTED_BLOCKS),
    ("CONSTANT_GIVEN_AS_A_CALL_ARGUMENT_IN_A_BRANCH", CONSTANT_GIVEN_AS_A_CALL_ARGUMENT_IN_A_BRANCH),
    ("CONSTANT_GIVEN_IN_A_BRANCH_OF_A_FUNCTION", CONSTANT_GIVEN_IN_A_BRANCH_OF_A_FUNCTION),
    ("CONSTANT_GIVEN_IN_ONE_INSPECT_ARM", CONSTANT_GIVEN_IN_ONE_INSPECT_ARM),
    ("POSSIBLY_GIVEN_THEN_BOUND", POSSIBLY_GIVEN_THEN_BOUND),
    ("POSSIBLY_GIVEN_THEN_ASSIGNED", POSSIBLY_GIVEN_THEN_ASSIGNED),
    ("POSSIBLY_GIVEN_THEN_RETURNED", POSSIBLY_GIVEN_THEN_RETURNED),
    ("POSSIBLY_GIVEN_THEN_PASSED", POSSIBLY_GIVEN_THEN_PASSED),
    ("POSSIBLY_GIVEN_THEN_GIVEN_AGAIN", POSSIBLY_GIVEN_THEN_GIVEN_AGAIN),
    ("POSSIBLY_GIVEN_THEN_TESTED", POSSIBLY_GIVEN_THEN_TESTED),
    ("POSSIBLY_GIVEN_THEN_STEERING_A_LOOP", POSSIBLY_GIVEN_THEN_STEERING_A_LOOP),
    ("POSSIBLY_GIVEN_THEN_WALKED", POSSIBLY_GIVEN_THEN_WALKED),
    ("POSSIBLY_GIVEN_THEN_INSPECTED", POSSIBLY_GIVEN_THEN_INSPECTED),
    ("GIVEN_IN_BOTH_BRANCHES", GIVEN_IN_BOTH_BRANCHES),
    ("GIVEN_IN_ONE_BRANCH_SHOWN_IN_THE_OTHER", GIVEN_IN_ONE_BRANCH_SHOWN_IN_THE_OTHER),
    ("GIVEN_IN_ONE_INLINE_BRANCH_SHOWN_IN_THE_OTHER", GIVEN_IN_ONE_INLINE_BRANCH_SHOWN_IN_THE_OTHER),
    ("GIVEN_IN_ONE_INSPECT_ARM_SHOWN_IN_ANOTHER", GIVEN_IN_ONE_INSPECT_ARM_SHOWN_IN_ANOTHER),
    ("GIVEN_IN_A_BRANCH_NEVER_USED_AGAIN", GIVEN_IN_A_BRANCH_NEVER_USED_AGAIN),
    ("BOUND_AFRESH_AND_GIVEN_ON_EVERY_PASS", BOUND_AFRESH_AND_GIVEN_ON_EVERY_PASS),
    ("SHOWN_BEFORE_IT_IS_GIVEN_IN_A_BRANCH", SHOWN_BEFORE_IT_IS_GIVEN_IN_A_BRANCH),
    ("SEQUENCE_GIVEN_IN_A_CONSTANT_TRUE_BRANCH", SEQUENCE_GIVEN_IN_A_CONSTANT_TRUE_BRANCH),
    ("NEW_SEQUENCE_GIVEN_IN_A_REPEAT_BODY", NEW_SEQUENCE_GIVEN_IN_A_REPEAT_BODY),
    ("GIVEN_TWICE_IN_A_ROW", GIVEN_TWICE_IN_A_ROW),
    ("ZONE_ESCAPE_IN_A_BRANCH_THAT_NEVER_RUNS", ZONE_ESCAPE_IN_A_BRANCH_THAT_NEVER_RUNS),
    ("ZONE_ESCAPE", ZONE_ESCAPE),
    ("ZONE_ESCAPE_INSIDE_A_DURING_SPAN", ZONE_ESCAPE_INSIDE_A_DURING_SPAN),
    ("CONSTANT_GIVEN_IN_A_BRANCH_INSIDE_A_DURING_SPAN", CONSTANT_GIVEN_IN_A_BRANCH_INSIDE_A_DURING_SPAN),
];

// ---------------------------------------------------------------------------------------------
// Verdicts
// ---------------------------------------------------------------------------------------------

/// The flow checker's refusal of a use after a possible move in a branch.
const MAYBE_IN_A_BRANCH: &str = "might have been given away in a conditional branch.";
/// The flow checker's refusal of a use after a possible move in a loop.
const MAYBE_IN_A_LOOP: &str = "might have been given away in a pass through the loop.";
/// Either maybe-move.
const MAYBE: &str = "might have been given away in ";
/// The flow checker's refusal of a use after a certain move.
const CERTAIN: &str = "after giving it away.";
/// The escape checker's refusal.
const ESCAPES: &str = "cannot escape zone 'Scratch'";

/// A compile's verdict, spelled out: `Ok(())`, or the refusal's message (a checker's own text,
/// any other error kind as its debug form).
fn spelled(result: Result<(), ParseErrorKind>) -> Result<(), String> {
    result.map_err(|kind| match kind {
        ParseErrorKind::Custom(message) => message,
        other => format!("{other:?}"),
    })
}

fn checked(source: &str) -> Result<(), String> {
    spelled(compile_to_rust_checked(source).map(|_| ()).map_err(|e| e.kind))
}

/// Every optimization switched off with a `## No` decorator.
fn with_every_optimization_off(source: &str) -> String {
    let keywords: Vec<&str> = REGISTRY.iter().map(|m| m.keyword).collect();
    decorate_source(source, &keywords)
}

/// The checkers' verdict on the program exactly as parsed, in the order the checked compile runs
/// them: zone escapes, then ownership. A parse refusal is spelled as the compile spells it.
fn as_written(source: &str) -> Result<(), String> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run_full().types
    };
    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();
    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = match parser.parse_program() {
        Ok(stmts) => stmts,
        Err(e) => return spelled(Err(e.kind)),
    };
    EscapeChecker::new(&interner).check_program(&stmts).map_err(|e| e.to_string())?;
    OwnershipChecker::new(&interner).check_program(&stmts).map_err(|e| e.to_string())
}

#[track_caller]
fn refused_by_the_checker(source: &str, phrase: &str) {
    match compile_to_rust_checked(source) {
        Ok(_) => panic!("the checked compile accepted a use after a possible move:\n{source}"),
        Err(e) => match &e.kind {
            ParseErrorKind::Custom(message) => assert!(
                message.contains(phrase),
                "refused, but not as {phrase:?}:\n{source}\n{message}"
            ),
            other => panic!(
                "refused by {other:?}, not by the ownership checker — the checker decides a move \
                 across blocks:\n{source}"
            ),
        },
    }
}

#[track_caller]
fn accepted(source: &str) {
    if let Err(e) = checked(source) {
        panic!("the checked compile refused a sound program:\n{source}\n{e}");
    }
}

// ---------------------------------------------------------------------------------------------
// A value the optimizer can fold is still given away where the author gave it
// ---------------------------------------------------------------------------------------------

#[test]
fn a_constant_given_in_a_branch_may_be_gone_after_it() {
    refused_by_the_checker(CONSTANT_GIVEN_IN_A_BRANCH, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_constant_given_in_an_inline_branch_may_be_gone_after_it() {
    refused_by_the_checker(CONSTANT_GIVEN_IN_AN_INLINE_BRANCH, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_constant_given_in_the_otherwise_branch_may_be_gone_after_it() {
    refused_by_the_checker(CONSTANT_GIVEN_IN_THE_OTHERWISE_BRANCH, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_text_constant_given_in_a_branch_may_be_gone_after_it() {
    refused_by_the_checker(TEXT_CONSTANT_GIVEN_IN_A_BRANCH, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_value_folded_at_compile_time_given_in_a_branch_may_be_gone_after_it() {
    refused_by_the_checker(FOLDED_SUM_GIVEN_IN_A_BRANCH, MAYBE_IN_A_BRANCH);
    refused_by_the_checker(FUNCTION_RESULT_GIVEN_IN_A_BRANCH, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_constant_given_in_a_while_body_may_be_gone_after_the_loop() {
    refused_by_the_checker(CONSTANT_GIVEN_IN_A_WHILE_BODY, MAYBE_IN_A_LOOP);
}

#[test]
fn a_constant_given_in_a_repeat_body_may_be_gone_after_the_loop() {
    refused_by_the_checker(CONSTANT_GIVEN_IN_A_REPEAT_BODY, MAYBE_IN_A_LOOP);
}

#[test]
fn a_constant_given_deep_in_nested_blocks_may_be_gone_after_them() {
    refused_by_the_checker(CONSTANT_GIVEN_DEEP_IN_NESTED_BLOCKS, MAYBE);
}

#[test]
fn a_constant_given_as_a_call_argument_in_a_branch_may_be_gone_after_it() {
    refused_by_the_checker(CONSTANT_GIVEN_AS_A_CALL_ARGUMENT_IN_A_BRANCH, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_constant_given_in_a_branch_of_a_function_may_be_gone_after_it() {
    refused_by_the_checker(CONSTANT_GIVEN_IN_A_BRANCH_OF_A_FUNCTION, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_constant_given_in_one_inspect_arm_may_be_gone_after_the_inspect() {
    refused_by_the_checker(CONSTANT_GIVEN_IN_ONE_INSPECT_ARM, MAYBE_IN_A_BRANCH);
}

/// A `During` span only names the work inside it: the body runs in place, so what it gives away
/// is judged as if the span were not there.
#[test]
fn a_constant_given_in_a_branch_inside_a_during_span_may_be_gone_after_it() {
    refused_by_the_checker(CONSTANT_GIVEN_IN_A_BRANCH_INSIDE_A_DURING_SPAN, MAYBE_IN_A_BRANCH);
}

// ---------------------------------------------------------------------------------------------
// Any read of a value that may be gone is refused
// ---------------------------------------------------------------------------------------------

#[test]
fn a_value_that_may_be_gone_cannot_be_bound() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_BOUND, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_value_that_may_be_gone_cannot_be_assigned() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_ASSIGNED, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_value_that_may_be_gone_cannot_be_returned() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_RETURNED, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_value_that_may_be_gone_cannot_be_passed_to_a_function() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_PASSED, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_value_that_may_be_gone_cannot_be_given_again() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_GIVEN_AGAIN, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_value_that_may_be_gone_cannot_be_tested_by_a_condition() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_TESTED, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_value_that_may_be_gone_cannot_steer_a_loop() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_STEERING_A_LOOP, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_collection_that_may_be_gone_cannot_be_walked() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_WALKED, MAYBE_IN_A_BRANCH);
}

#[test]
fn a_value_that_may_be_gone_cannot_be_inspected() {
    refused_by_the_checker(POSSIBLY_GIVEN_THEN_INSPECTED, MAYBE_IN_A_BRANCH);
}

// ---------------------------------------------------------------------------------------------
// Certain moves, and the sound programs around them
// ---------------------------------------------------------------------------------------------

#[test]
fn a_value_given_in_both_branches_is_gone_after_them() {
    refused_by_the_checker(GIVEN_IN_BOTH_BRANCHES, CERTAIN);
}

#[test]
fn a_value_given_in_one_branch_may_be_shown_in_the_other() {
    accepted(GIVEN_IN_ONE_BRANCH_SHOWN_IN_THE_OTHER);
}

#[test]
fn a_value_given_in_one_inline_branch_may_be_shown_in_the_other() {
    accepted(GIVEN_IN_ONE_INLINE_BRANCH_SHOWN_IN_THE_OTHER);
}

#[test]
fn a_value_given_in_one_inspect_arm_may_be_shown_in_another() {
    accepted(GIVEN_IN_ONE_INSPECT_ARM_SHOWN_IN_ANOTHER);
}

#[test]
fn a_value_given_in_a_branch_and_never_used_again_is_fine() {
    accepted(GIVEN_IN_A_BRANCH_NEVER_USED_AGAIN);
}

#[test]
fn a_value_bound_afresh_on_every_pass_may_be_given_on_every_pass() {
    accepted(BOUND_AFRESH_AND_GIVEN_ON_EVERY_PASS);
}

#[test]
fn a_value_shown_before_it_is_given_in_a_branch_is_fine() {
    accepted(SHOWN_BEFORE_IT_IS_GIVEN_IN_A_BRANCH);
}

// ---------------------------------------------------------------------------------------------
// A value the optimizer cannot fold keeps the verdict the author's program earns
// ---------------------------------------------------------------------------------------------

/// `If c:` with `c` a known `true` folds away, but the author wrote a branch: the value may still
/// be there, and the refusal says so rather than claiming a certain move.
#[test]
fn a_sequence_given_in_a_constant_true_branch_is_only_maybe_gone() {
    refused_by_the_checker(SEQUENCE_GIVEN_IN_A_CONSTANT_TRUE_BRANCH, MAYBE_IN_A_BRANCH);
}

/// A loop may run no pass at all, so a value given in its body may still be there after it.
#[test]
fn a_sequence_given_in_a_repeat_body_is_only_maybe_gone_after_it() {
    refused_by_the_checker(NEW_SEQUENCE_GIVEN_IN_A_REPEAT_BODY, MAYBE_IN_A_LOOP);
}

// ---------------------------------------------------------------------------------------------
// The refusal names where the value may have gone, in plain words
// ---------------------------------------------------------------------------------------------

#[test]
fn a_maybe_move_says_where_the_value_may_have_gone() {
    let branch = checked(CONSTANT_GIVEN_IN_A_BRANCH).expect_err("a possible move in a branch");
    assert!(
        branch.starts_with("Cannot use 'x' - it might have been given away in a conditional branch.\n\n"),
        "{branch}"
    );
    assert!(!branch.contains("the a conditional branch branch"), "{branch}");
    assert!(branch.contains("Tip: Move the usage inside the branch"), "{branch}");

    let pass = checked(NEW_SEQUENCE_GIVEN_IN_A_REPEAT_BODY).expect_err("a possible move in a loop");
    assert!(
        pass.starts_with("Cannot use 'x' - it might have been given away in a pass through the loop.\n\n"),
        "{pass}"
    );
    assert!(!pass.contains("branch"), "a loop is not a branch: {pass}");
}

// ---------------------------------------------------------------------------------------------
// Invariants over every shape
// ---------------------------------------------------------------------------------------------

/// The checked compile's verdict IS the checkers' verdict on the parse — same acceptance, same
/// words — for every program above.
#[test]
fn the_checked_compile_gives_the_checkers_verdict_on_the_program_as_written() {
    let disagreements: Vec<String> = EVERY_SHAPE
        .iter()
        .filter_map(|(name, source)| {
            let written = as_written(source);
            let compiled = checked(source);
            (written != compiled)
                .then(|| format!("{name}:\n  as written: {written:?}\n  compiled:   {compiled:?}"))
        })
        .collect();
    assert!(disagreements.is_empty(), "{}", disagreements.join("\n"));
}

/// The optimization config decides how fast the program runs, never whether it is sound.
#[test]
fn the_verdict_does_not_depend_on_the_optimization_config() {
    let disagreements: Vec<String> = EVERY_SHAPE
        .iter()
        .filter_map(|(name, source)| {
            let default = checked(source);
            let all_off = checked(&with_every_optimization_off(source));
            (default != all_off)
                .then(|| format!("{name}:\n  default: {default:?}\n  all off: {all_off:?}"))
        })
        .collect();
    assert!(disagreements.is_empty(), "{}", disagreements.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// Every entry point
// ---------------------------------------------------------------------------------------------

/// `compile_and_run` checks ownership before it writes a project. The output path sits under a
/// plain file, so reaching the write fails at once with `Io` and no cargo build ever starts.
#[test]
fn compile_and_run_refuses_a_possible_move_before_writing_anything() {
    let dir = tempfile::tempdir().expect("temp dir");
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, "").expect("blocker file");
    let output = blocker.join("project");
    match compile_and_run(CONSTANT_GIVEN_IN_A_BRANCH, &output) {
        Err(CompileError::Parse(e)) => match &e.kind {
            ParseErrorKind::Custom(message) => {
                assert!(message.contains(MAYBE_IN_A_BRANCH), "{message}")
            }
            other => panic!("refused by {other:?}, not by the ownership checker"),
        },
        other => panic!("compile_and_run went past its ownership check: {other:?}"),
    }
    assert!(!output.exists());
}

fn compiled_through_the_project_path(source: &str) -> Result<(), String> {
    let dir = tempfile::tempdir().expect("temp dir");
    let main = dir.path().join("main.lg");
    std::fs::write(&main, source).expect("main.lg");
    compile_project(Path::new(&main)).map(|_| ()).map_err(|e| format!("{e:?}"))
}

/// Each compile path's verdict on `source` — the plain compile, the checked compile and `largo`'s
/// project path — with the default config and with every optimization off.
fn every_compile_path(source: &str) -> Vec<(&'static str, Result<(), String>)> {
    let all_off = with_every_optimization_off(source);
    vec![
        ("compile_to_rust", spelled(compile_to_rust(source).map(|_| ()).map_err(|e| e.kind))),
        ("compile_to_rust, every optimization off", spelled(compile_to_rust(&all_off).map(|_| ()).map_err(|e| e.kind))),
        ("compile_to_rust_checked", checked(source)),
        ("compile_to_rust_checked, every optimization off", checked(&all_off)),
        ("compile_project", compiled_through_the_project_path(source)),
        ("compile_project, every optimization off", compiled_through_the_project_path(&all_off)),
    ]
}

#[track_caller]
fn every_compile_path_refuses_the_escape(source: &str) {
    for (path, verdict) in every_compile_path(source) {
        match verdict {
            Ok(()) => panic!("{path} accepted a zone escape:\n{source}"),
            Err(message) => assert!(message.contains(ESCAPES), "{path}: {message}"),
        }
    }
}

/// A zone escape is refused on every compile path, even inside a branch the optimizer can prove
/// never runs, and whatever the optimization config.
#[test]
fn every_compile_path_refuses_a_zone_escape_in_a_branch_that_never_runs() {
    every_compile_path_refuses_the_escape(ZONE_ESCAPE_IN_A_BRANCH_THAT_NEVER_RUNS);
}

/// The release profile erases `During` spans before the optimizer; the checks judge the span as
/// written, and look inside it.
#[test]
fn every_compile_path_refuses_a_zone_escape_inside_a_during_span() {
    every_compile_path_refuses_the_escape(ZONE_ESCAPE_INSIDE_A_DURING_SPAN);
}

#[test]
fn every_compile_path_refuses_a_plain_zone_escape() {
    every_compile_path_refuses_the_escape(ZONE_ESCAPE);
}
