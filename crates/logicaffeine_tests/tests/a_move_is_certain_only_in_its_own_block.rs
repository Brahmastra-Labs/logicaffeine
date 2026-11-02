//! The parser refuses a use after `Give` at parse time — but only when the move
//! is certain: a `Give` moves its value for the rest of the block it is written
//! in. Once a branch, a loop body or a function ends, whether the value is still
//! there is flow analysis, which the ownership checker does (and reports as a
//! maybe-move, not a definite one). A parse-time refusal there would name a
//! move that may never happen.

use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{Expr, Stmt, TypeExpr},
    drs::WorldState,
    error::ParseErrorKind,
    Lexer, Parser,
};

/// `Ok(())` when `source` parses, else the error kind.
fn parse(source: &str) -> Result<(), ParseErrorKind> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let mwe_trie = logicaffeine_language::mwe::build_mwe_trie();
    let tokens = logicaffeine_language::mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);
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
    parser.parse_program().map(|_| ()).map_err(|e| e.kind)
}

fn refuses_use_after_move(source: &str) -> bool {
    matches!(parse(source), Err(ParseErrorKind::UseAfterMove { .. }))
}

/// `Ok(())` when `source` parses, else the error, spelled out for the failure message.
fn outcome(source: &str) -> Result<(), String> {
    parse(source).map_err(|kind| format!("{kind:?}"))
}

#[test]
fn a_use_after_a_move_in_the_same_block_is_refused() {
    assert!(refuses_use_after_move(
        "## Main\nLet s be a new Seq of Int.\nGive s to keeper.\nShow s.\n"
    ));
}

#[test]
fn a_use_after_a_move_in_the_same_branch_is_refused() {
    assert!(refuses_use_after_move(
        "## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c:\n    Give s to keeper.\n    Show s.\n"
    ));
}

#[test]
fn a_use_after_a_move_in_the_same_function_body_is_refused() {
    assert!(refuses_use_after_move(
        "## To f () -> Int:\n    Let s be a new Seq of Int.\n    Give s to keeper.\n    Show s.\n    Return 0.\n\n## Main\nShow f().\n"
    ));
}

#[test]
fn a_move_in_a_branch_is_not_certain_after_it() {
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c:\n    Give s to keeper.\nShow s.\n"),
        Ok(()),
    );
}

#[test]
fn a_move_in_an_inline_branch_is_not_certain_after_it() {
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c: Give s to keeper.\nShow s.\n"),
        Ok(()),
    );
}

#[test]
fn a_move_in_an_otherwise_branch_is_not_certain_after_it() {
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c:\n    Show 1.\nOtherwise:\n    Give s to keeper.\nShow s.\n"),
        Ok(()),
    );
}

#[test]
fn a_move_in_a_loop_body_is_not_certain_after_it() {
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nRepeat for i from 1 to 2:\n    Give s to keeper.\nShow s.\n"),
        Ok(()),
    );
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet n be 0.\nWhile n < 1:\n    Give s to keeper.\n    Set n to n + 1.\nShow s.\n"),
        Ok(()),
    );
}

#[test]
fn a_move_in_a_function_does_not_reach_another_block() {
    assert_eq!(
        outcome("## To f () -> Int:\n    Let s be a new Seq of Int.\n    Give s to keeper.\n    Return 0.\n\n## Main\nLet s be a new Seq of Int.\nShow s.\n"),
        Ok(()),
    );
}

#[test]
fn a_move_before_a_branch_is_still_certain_inside_and_after_it() {
    assert!(refuses_use_after_move(
        "## Main\nLet s be a new Seq of Int.\nLet c be true.\nGive s to keeper.\nIf c:\n    Show s.\n"
    ));
    assert!(refuses_use_after_move(
        "## Main\nLet s be a new Seq of Int.\nLet c be true.\nGive s to keeper.\nIf c:\n    Show 1.\nShow s.\n"
    ));
}

/// A block ends the moves made in it — so a move in one branch never reaches its sibling: the
/// `Otherwise` branch runs only when the then-branch did not.
#[test]
fn a_move_in_the_then_branch_does_not_reach_the_otherwise_branch() {
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c:\n    Give s to keeper.\nOtherwise:\n    Show s.\n"),
        Ok(()),
    );
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c:\n    Give s to keeper.\nOtherwise:\n    Give s to other.\n"),
        Ok(()),
    );
}

#[test]
fn a_move_in_an_inline_then_branch_does_not_reach_the_otherwise_branch() {
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c: Give s to keeper.\nOtherwise: Show s.\n"),
        Ok(()),
    );
}

#[test]
fn a_move_in_one_link_of_an_otherwise_if_chain_does_not_reach_the_next() {
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet c be true.\nLet d be false.\nIf c:\n    Give s to keeper.\nOtherwise If d:\n    Show s.\nOtherwise:\n    Show s.\n"),
        Ok(()),
    );
    assert_eq!(
        outcome("## Main\nLet s be a new Seq of Int.\nLet c be true.\nLet d be false.\nIf c:\n    Show 1.\nOtherwise If d:\n    Give s to keeper.\nOtherwise:\n    Show s.\n"),
        Ok(()),
    );
}

#[test]
fn a_move_in_one_inspect_arm_does_not_reach_the_next() {
    assert_eq!(
        outcome("## Definition\nA Color is either:\n    A Red.\n    A Blue.\n\n## Main\nLet s be a new Seq of Int.\nLet c be a new Red.\nInspect c:\n    If it is a Red:\n        Give s to keeper.\n    If it is a Blue:\n        Show s.\n"),
        Ok(()),
    );
    assert_eq!(
        outcome("## Definition\nA Color is either:\n    A Red.\n    A Blue.\n\n## Main\nLet s be a new Seq of Int.\nLet c be a new Red.\nInspect c:\n    If it is a Red:\n        Give s to keeper.\n    Otherwise:\n        Show s.\n"),
        Ok(()),
    );
}

/// Ending a block's moves never ends a move made BEFORE the block: that one is certain in the
/// enclosing block, so every branch after it — and each sibling — still refuses the use.
#[test]
fn a_move_before_a_branch_still_reaches_every_sibling_branch() {
    assert!(refuses_use_after_move(
        "## Main\nLet s be a new Seq of Int.\nLet c be true.\nGive s to keeper.\nIf c:\n    Show 1.\nOtherwise:\n    Show s.\n"
    ));
    assert!(refuses_use_after_move(
        "## Main\nLet s be a new Seq of Int.\nLet c be true.\nGive s to keeper.\nIf c: Show 1.\nOtherwise: Show s.\n"
    ));
}
