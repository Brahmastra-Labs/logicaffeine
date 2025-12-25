use logos::*;
use logos::ast::{Stmt, Expr};

fn make_parser(source: &str) -> (Interner, Vec<Token>) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    (interner, tokens)
}

#[test]
fn colon_token_generated() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("If x:", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Colon)),
        "Colon token should be generated: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn indent_token_generated() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("If x:\n    y.", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Indent)),
        "Indent token should be generated: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn dedent_token_generated() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("If x:\n    y.\nz.", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Dedent)),
        "Dedent token should be generated: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn if_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("If x equals 5:", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::If)),
        "If token should be recognized: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn if_block_parses() {
    let source = "## Main\nIf x equals 5:\n    Return true.\nReturn false.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
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
    assert!(result.is_ok(), "If block should parse: {:?}", result);

    let stmts = result.unwrap();
    assert_eq!(stmts.len(), 2, "Should have 2 statements (If + Return)");
    assert!(matches!(stmts[0], Stmt::If { .. }), "First statement should be If");
    assert!(matches!(stmts[1], Stmt::Return { .. }), "Second statement should be Return");
}

#[test]
fn nested_if_blocks_parse() {
    let source = "## Main\nIf x equals 5:\n    If y equals 10:\n        Return 1.\n    Return 0.\nReturn 99.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
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
    assert!(result.is_ok(), "Nested if blocks should parse: {:?}", result);

    let stmts = result.unwrap();
    assert_eq!(stmts.len(), 2, "Should have 2 top-level statements");
}

#[test]
fn while_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("While x equals 5:", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::While)),
        "While token should be recognized: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn while_block_parses() {
    let source = "## Main\nLet x be 0.\nWhile x equals 5:\n    Set x to 10.\nReturn x.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
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
    assert!(result.is_ok(), "While block should parse: {:?}", result);

    let stmts = result.unwrap();
    assert_eq!(stmts.len(), 3, "Should have 3 statements (Let + While + Return)");
    assert!(matches!(stmts[0], Stmt::Let { .. }), "First statement should be Let");
    assert!(matches!(stmts[1], Stmt::While { .. }), "Second statement should be While");
    assert!(matches!(stmts[2], Stmt::Return { .. }), "Third statement should be Return");
}

#[test]
fn otherwise_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("If x:\n    y.\nOtherwise:\n    z.", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Otherwise)),
        "Otherwise token should be recognized: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn if_else_block_parses() {
    let source = "## Main\nIf x equals 5:\n    Return true.\nOtherwise:\n    Return false.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
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
    assert!(result.is_ok(), "If-else block should parse: {:?}", result);

    let stmts = result.unwrap();
    assert_eq!(stmts.len(), 1, "Should have 1 statement (If with else)");

    if let Stmt::If { else_block, .. } = &stmts[0] {
        assert!(else_block.is_some(), "If statement should have else_block");
    } else {
        panic!("First statement should be If");
    }
}

#[test]
fn call_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Call process with data.", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Call)),
        "Call token should be recognized: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn call_statement_parses() {
    let source = "## Main\nCall process with data.";
    let (mut interner, tokens) = make_parser(source);

    let mut ctx = DiscourseContext::new();
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
    assert!(result.is_ok(), "Call statement should parse: {:?}", result);

    let stmts = result.unwrap();
    assert_eq!(stmts.len(), 1, "Should have 1 statement");
    assert!(matches!(stmts[0], Stmt::Call { .. }), "Statement should be Call");
}
