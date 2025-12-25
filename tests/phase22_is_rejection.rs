use logos::*;
use logos::context::{Entity, Gender, Number};

#[test]
fn x_is_5_errors_in_imperative() {
    let source = "## Main\nx is 5.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "x".to_string(),
        ownership: OwnershipState::Owned,
    });

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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
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
fn x_is_predicate_still_works_in_imperative() {
    let source = "## Main\nx is mortal.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "x".to_string(),
        ownership: OwnershipState::Owned,
    });

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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let result = parser.parse();
    if let Err(ref e) = result {
        assert!(
            !matches!(e.kind, ParseErrorKind::IsValueEquality { .. }),
            "x is mortal should NOT trigger IsValueEquality, got {:?}",
            e.kind
        );
    }
}

#[test]
fn count_is_10_errors_in_imperative() {
    let source = "## Main\ncount is 10.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "count".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "count".to_string(),
        ownership: OwnershipState::Owned,
    });

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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let result = parser.parse();
    assert!(result.is_err(), "count is 10 should error in imperative mode");

    let err = result.unwrap_err();
    assert!(
        matches!(err.kind, ParseErrorKind::IsValueEquality { .. }),
        "Should be IsValueEquality error, got {:?}",
        err.kind
    );
}
