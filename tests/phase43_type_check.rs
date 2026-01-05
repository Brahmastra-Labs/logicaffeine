// Phase 43B: Type Checking Tests

use logos::*;
use logos::ast::Stmt;
use logos::error::ParseErrorKind;

fn make_parser(source: &str) -> (Interner, Vec<Token>) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    (interner, tokens)
}

// =============================================================================
// Type Mismatch Detection Tests
// =============================================================================

#[test]
fn type_mismatch_int_with_text_literal() {
    let source = "## Main\nLet x: Int be \"hello\".";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<logos::ast::Expr> = logos::arena::Arena::new();
    let type_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();

    assert!(result.is_err(), "Should error on type mismatch");
    let err = result.unwrap_err();
    match &err.kind {
        ParseErrorKind::TypeMismatch { expected, found } => {
            assert_eq!(expected, "Int", "Expected type should be Int");
            assert_eq!(found, "Text", "Found type should be Text");
        }
        other => panic!("Expected TypeMismatch error, got {:?}", other),
    }
}

#[test]
fn type_mismatch_text_with_int_literal() {
    let source = "## Main\nLet name: Text be 42.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<logos::ast::Expr> = logos::arena::Arena::new();
    let type_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();

    assert!(result.is_err(), "Should error on type mismatch");
    let err = result.unwrap_err();
    match &err.kind {
        ParseErrorKind::TypeMismatch { expected, found } => {
            assert_eq!(expected, "Text", "Expected type should be Text");
            assert_eq!(found, "Int", "Found type should be Int");
        }
        other => panic!("Expected TypeMismatch error, got {:?}", other),
    }
}

#[test]
fn type_mismatch_bool_with_int_literal() {
    let source = "## Main\nLet flag: Bool be 1.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<logos::ast::Expr> = logos::arena::Arena::new();
    let type_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();

    assert!(result.is_err(), "Should error on type mismatch");
    let err = result.unwrap_err();
    match &err.kind {
        ParseErrorKind::TypeMismatch { expected, found } => {
            assert_eq!(expected, "Bool");
            assert_eq!(found, "Int");
        }
        other => panic!("Expected TypeMismatch error, got {:?}", other),
    }
}

// =============================================================================
// Correct Type Annotation Tests
// =============================================================================

#[test]
fn correct_int_type_annotation() {
    let source = "## Main\nLet x: Int be 42.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<logos::ast::Expr> = logos::arena::Arena::new();
    let type_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Correct type annotation should pass: {:?}", result);
}

#[test]
fn correct_text_type_annotation() {
    let source = "## Main\nLet name: Text be \"Alice\".";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<logos::ast::Expr> = logos::arena::Arena::new();
    let type_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Correct type annotation should pass: {:?}", result);
}

#[test]
fn correct_bool_type_annotation() {
    let source = "## Main\nLet flag: Bool be true.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<logos::ast::Expr> = logos::arena::Arena::new();
    let type_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Correct type annotation should pass: {:?}", result);
}

// =============================================================================
// No Type Annotation (Inference) Tests
// =============================================================================

#[test]
fn no_type_annotation_allowed() {
    let source = "## Main\nLet x be 42.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<logos::ast::Expr> = logos::arena::Arena::new();
    let type_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "No type annotation should be allowed: {:?}", result);
}

// =============================================================================
// Nat accepts Int literals (special compatibility)
// =============================================================================

#[test]
fn nat_accepts_int_literal() {
    let source = "## Main\nLet n: Nat be 5.";
    let (mut interner, tokens) = make_parser(source);

    let mut world_state = logos::drs::WorldState::new();
    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();
    let stmt_arena: logos::arena::Arena<Stmt> = logos::arena::Arena::new();
    let imperative_expr_arena: logos::arena::Arena<logos::ast::Expr> = logos::arena::Arena::new();
    let type_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, logos::analysis::TypeRegistry::default());
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Nat should accept Int literals: {:?}", result);
}
