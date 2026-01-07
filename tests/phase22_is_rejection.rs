use logos::*;
use logos::drs::{Gender, Number, WorldState};
use logos::analysis::TypeRegistry;

#[test]
fn x_is_5_errors_in_imperative() {
    let source = "## Main\nx is 5.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mut world_state = WorldState::new();

    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, registry);
    parser.process_block_headers();

    let result = parser.parse();
    assert!(result.is_err(), "x is 5 should error in imperative mode");

    let err = result.unwrap_err();
    assert!(
        matches!(err.kind, ParseErrorKind::IsValueEquality { .. }),
        "Should be IsValueEquality error, got {:?}",
        err.kind
    );
}

#[test]
fn let_x_is_5_accepted() {
    let source = "## Main\nLet x is 5.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mut world_state = WorldState::new();

    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, registry);
    parser.process_block_headers();

    let result = parser.parse();
    assert!(result.is_ok(), "Let x is 5 should parse: {:?}", result.err());
}

#[test]
fn x_equals_5_accepted() {
    let source = "## Main\nLet x = 5.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mut world_state = WorldState::new();

    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, registry);
    parser.process_block_headers();

    let result = parser.parse();
    assert!(result.is_ok(), "Let x = 5 should parse: {:?}", result.err());
}

#[test]
fn x_is_true_accepted() {
    let source = "## Main\nLet x is true.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mut world_state = WorldState::new();

    let expr_arena = logos::arena::Arena::new();
    let term_arena = logos::arena::Arena::new();
    let np_arena = logos::arena::Arena::new();
    let sym_arena = logos::arena::Arena::new();
    let role_arena = logos::arena::Arena::new();
    let pp_arena = logos::arena::Arena::new();

    let ast_ctx = logos::arena_ctx::AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, registry);
    parser.process_block_headers();

    let result = parser.parse();
    assert!(result.is_ok(), "Let x is true should parse: {:?}", result.err());
}
