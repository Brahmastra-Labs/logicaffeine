use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    Lexer, Token, Parser,
    ast::{Stmt, Expr},
    drs::WorldState,
    arena_ctx::AstContext,
    analysis::TypeRegistry,
};

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

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();

    let ast_ctx = AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "parse_program should exist and succeed");
}

#[test]
fn let_statement_parses() {
    let source = "## Main\nLet x be 5.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();

    let ast_ctx = AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, TypeRegistry::default());
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

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();

    let ast_ctx = AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, TypeRegistry::default());
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

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();

    let ast_ctx = AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Set statement should parse: {:?}", result);

    let stmts = result.unwrap();
    assert_eq!(stmts.len(), 2, "Should have two statements");
    assert!(matches!(stmts[1], Stmt::Set { .. }), "Second statement should be Set");
}
