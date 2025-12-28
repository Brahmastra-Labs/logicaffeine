use logos::{DiscourseContext, OwnershipState};
use logos::context::{Entity, Gender, Number};

#[test]
fn entity_starts_owned() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "thing".to_string(),
        ownership: OwnershipState::Owned,
    });

    let entity = ctx.resolve_definite("thing").unwrap();
    assert_eq!(entity.ownership, OwnershipState::Owned);
}

#[test]
fn ownership_state_can_be_moved() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "book".to_string(),
        ownership: OwnershipState::Owned,
    });

    ctx.set_ownership("book", OwnershipState::Moved);

    let entity = ctx.resolve_definite("book").unwrap();
    assert_eq!(entity.ownership, OwnershipState::Moved);
}

#[test]
fn ownership_state_can_be_borrowed() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "item".to_string(),
        ownership: OwnershipState::Owned,
    });

    ctx.set_ownership("item", OwnershipState::Borrowed);

    let entity = ctx.resolve_definite("item").unwrap();
    assert_eq!(entity.ownership, OwnershipState::Borrowed);
}

#[test]
fn get_ownership_returns_current_state() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "y".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "value".to_string(),
        ownership: OwnershipState::Owned,
    });

    assert_eq!(ctx.get_ownership("value"), Some(OwnershipState::Owned));
    assert_eq!(ctx.get_ownership("unknown"), None);
}

// Step 1.5: Use-After-Move Detection

#[test]
fn moved_variable_detected() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "book".to_string(),
        ownership: OwnershipState::Owned,
    });

    ctx.set_ownership("book", OwnershipState::Moved);

    // After move, get_ownership should return Moved
    assert_eq!(ctx.get_ownership("book"), Some(OwnershipState::Moved));
}

#[test]
fn borrowed_variable_still_accessible() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "item".to_string(),
        ownership: OwnershipState::Owned,
    });

    ctx.set_ownership("item", OwnershipState::Borrowed);

    // After borrow, get_ownership should return Borrowed (not Moved)
    assert_eq!(ctx.get_ownership("item"), Some(OwnershipState::Borrowed));
}

// =============================================================================
// Step 2: Give/Show Statement Parsing and Ownership Tracking
// =============================================================================

use logos::*;
use logos::ast::{Stmt, Expr};
use logos::error::ParseErrorKind;

fn make_parser(source: &str) -> (Interner, Vec<Token>) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    (interner, tokens)
}

#[test]
fn give_token_recognized() {
    // Give is only a keyword in imperative mode (after ## Main)
    let source = "## Main\nGive";
    let (_, tokens) = make_parser(source);

    // Token 0 is BlockHeader, Token 1 is Give
    assert!(tokens.len() >= 2, "Should have at least two tokens");
    assert!(
        matches!(tokens[1].kind, TokenType::Give),
        "Expected Give token after ## Main, got {:?}",
        tokens[1].kind
    );
}

#[test]
fn show_token_recognized() {
    // Show is only a keyword in imperative mode (after ## Main)
    let source = "## Main\nShow";
    let (_, tokens) = make_parser(source);

    assert!(tokens.len() >= 2, "Should have at least two tokens");
    assert!(
        matches!(tokens[1].kind, TokenType::Show),
        "Expected Show token after ## Main, got {:?}",
        tokens[1].kind
    );
}

#[test]
fn give_statement_parses() {
    let source = "## Main\nLet x be 5.\nGive x to processor.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
    // Register recipient so strict verification passes
    ctx.register(Entity {
        symbol: "processor".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "processor".to_string(),
        ownership: OwnershipState::Owned,
    });
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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Give statement should parse: {:?}", result);

    let stmts = result.unwrap();
    assert!(stmts.len() >= 2, "Should have at least two statements");
    assert!(
        matches!(stmts[1], Stmt::Give { .. }),
        "Second statement should be Give, got {:?}",
        stmts[1]
    );
}

#[test]
fn give_statement_marks_variable_as_moved() {
    let source = "## Main\nLet x be 5.\nGive x to processor.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "processor".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "processor".to_string(),
        ownership: OwnershipState::Owned,
    });
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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Give statement should parse: {:?}", result);

    // After parsing Give, the variable should be marked as Moved
    assert_eq!(
        ctx.get_ownership("x"),
        Some(OwnershipState::Moved),
        "Variable 'x' should be marked as Moved after Give"
    );
}

#[test]
fn use_after_give_produces_error() {
    let source = "## Main\nLet x be 5.\nGive x to processor.\nReturn x.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "processor".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "processor".to_string(),
        ownership: OwnershipState::Owned,
    });
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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let result = parser.parse_program();

    // Should fail with UseAfterMove error
    assert!(result.is_err(), "Should error when using variable after Give");
    let err = result.unwrap_err();
    assert!(
        matches!(err.kind, ParseErrorKind::UseAfterMove { .. }),
        "Expected UseAfterMove error, got {:?}",
        err.kind
    );
}

#[test]
fn show_statement_parses() {
    let source = "## Main\nLet x be 5.\nShow x to console.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "console".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "console".to_string(),
        ownership: OwnershipState::Owned,
    });
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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Show statement should parse: {:?}", result);

    let stmts = result.unwrap();
    assert!(stmts.len() >= 2, "Should have at least two statements");
    assert!(
        matches!(stmts[1], Stmt::Show { .. }),
        "Second statement should be Show, got {:?}",
        stmts[1]
    );
}

#[test]
fn show_statement_marks_variable_as_borrowed() {
    let source = "## Main\nLet x be 5.\nShow x to console.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "console".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "console".to_string(),
        ownership: OwnershipState::Owned,
    });
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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let result = parser.parse_program();
    assert!(result.is_ok(), "Show statement should parse: {:?}", result);

    // After parsing Show, the variable should be marked as Borrowed
    assert_eq!(
        ctx.get_ownership("x"),
        Some(OwnershipState::Borrowed),
        "Variable 'x' should be marked as Borrowed after Show"
    );
}

#[test]
fn variable_accessible_after_show() {
    let source = "## Main\nLet x be 5.\nShow x to console.\nReturn x.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "console".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "console".to_string(),
        ownership: OwnershipState::Owned,
    });
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

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let result = parser.parse_program();

    // Should succeed - variable is borrowed, not moved
    assert!(
        result.is_ok(),
        "Variable should still be accessible after Show: {:?}",
        result
    );
}
