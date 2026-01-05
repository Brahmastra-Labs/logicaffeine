use logos::*;
use logos::ast::{Stmt, Expr};

fn make_parser(source: &str) -> (Interner, Vec<Token>) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    (interner, tokens)
}

#[test]
fn parse_program_method_exists() {
    let source = "## Main\nLet x be 5.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<Expr> = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "parse_program should exist and succeed");
}

#[test]
fn let_statement_parses() {
    let source = "## Main\nLet x be 5.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<Expr> = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Let statement should parse: {:?}", result);

    let stmts = result.unwrap();
    assert!(!stmts.is_empty(), "Should have at least one statement");
    assert!(matches!(stmts[0], Stmt::Let { .. }), "First statement should be Let");
}

#[test]
fn return_statement_parses() {
    let source = "## Main\nReturn 42.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<Expr> = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Return statement should parse: {:?}", result);

    let stmts = result.unwrap();
    assert!(matches!(stmts[0], Stmt::Return { .. }), "First statement should be Return");
}

#[test]
fn set_statement_parses() {
    let source = "## Main\nLet x be 5.\nSet x to 10.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<Expr> = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Set statement should parse: {:?}", result);

    let stmts = result.unwrap();
    assert_eq!(stmts.len(), 2, "Should have two statements");
    assert!(matches!(stmts[1], Stmt::Set { .. }), "Second statement should be Set");
}
