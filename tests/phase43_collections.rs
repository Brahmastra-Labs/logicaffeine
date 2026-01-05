// Phase 43D: Collection Operations Tests
//
// Tests for LOGOS collection syntax:
// - Push x to items
// - Pop from items
// - length of items
// - copy of slice
// - items at i (index access)
// - items 1 through 3 (slice access)

use logos::*;
use logos::ast::{Stmt, Expr};

fn make_parser(source: &str) -> (Interner, Vec<Token>) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    (interner, tokens)
}

fn parse_and_check<F>(source: &str, checker: F) -> bool
where F: Fn(&Vec<Stmt>) -> bool {
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

    match parser.parse_program() {
        Ok(stmts) => checker(&stmts),
        Err(e) => {
            println!("Parse error: {:?}", e);
            false
        }
    }
}

// Push statement tests
#[test]
fn push_statement_parses() {
    let source = r#"## Main
Let items be [1, 2, 3].
Push 4 to items.
"#;
    let result = parse_and_check(source, |stmts| {
        stmts.iter().any(|s| matches!(s, Stmt::Push { .. }))
    });
    assert!(result, "Expected Push statement in parsed output");
}

#[test]
fn push_with_expression_parses() {
    let source = r#"## Main
Let items be [1, 2, 3].
Let x be 10.
Push x to items.
"#;
    let result = parse_and_check(source, |stmts| {
        stmts.iter().any(|s| matches!(s, Stmt::Push { .. }))
    });
    assert!(result, "Expected Push statement with variable");
}

// Pop statement tests
#[test]
fn pop_statement_parses() {
    let source = r#"## Main
Let items be [1, 2, 3].
Pop from items.
"#;
    let result = parse_and_check(source, |stmts| {
        stmts.iter().any(|s| matches!(s, Stmt::Pop { into: None, .. }))
    });
    assert!(result, "Expected Pop statement without binding");
}

#[test]
fn pop_into_variable_parses() {
    let source = r#"## Main
Let items be [1, 2, 3].
Pop from items into last.
"#;
    let result = parse_and_check(source, |stmts| {
        stmts.iter().any(|s| matches!(s, Stmt::Pop { into: Some(_), .. }))
    });
    assert!(result, "Expected Pop statement with 'into' binding");
}

// Length expression tests
#[test]
fn length_expression_parses() {
    let source = r#"## Main
Let items be [1, 2, 3].
Let n be length of items.
"#;
    let result = parse_and_check(source, |stmts| {
        stmts.iter().any(|s| {
            if let Stmt::Let { value, .. } = s {
                matches!(**value, Expr::Length { .. })
            } else {
                false
            }
        })
    });
    assert!(result, "Expected Length expression in Let statement");
}

// Copy expression tests
#[test]
fn copy_expression_parses() {
    let source = r#"## Main
Let items be [1, 2, 3].
Let cloned be copy of items.
"#;
    let result = parse_and_check(source, |stmts| {
        stmts.iter().any(|s| {
            if let Stmt::Let { value, .. } = s {
                matches!(**value, Expr::Copy { .. })
            } else {
                false
            }
        })
    });
    assert!(result, "Expected Copy expression in Let statement");
}

// Index expression tests (using existing "item N of" syntax)
#[test]
fn index_expression_parses() {
    let source = r#"## Main
Let items be [1, 2, 3].
Let first be item 1 of items.
"#;
    let result = parse_and_check(source, |stmts| {
        stmts.iter().any(|s| {
            if let Stmt::Let { value, .. } = s {
                matches!(**value, Expr::Index { .. })
            } else {
                false
            }
        })
    });
    assert!(result, "Expected Index expression in Let statement");
}

// Slice expression tests (using existing "items N through M of" syntax)
#[test]
fn slice_expression_parses() {
    let source = r#"## Main
Let items be [1, 2, 3, 4, 5].
Let middle be items 2 through 4 of items.
"#;
    let result = parse_and_check(source, |stmts| {
        stmts.iter().any(|s| {
            if let Stmt::Let { value, .. } = s {
                matches!(**value, Expr::Slice { .. })
            } else {
                false
            }
        })
    });
    assert!(result, "Expected Slice expression in Let statement");
}

// Integration tests
#[test]
fn collection_operations_in_sequence() {
    let source = r#"## Main
Let items be [].
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let n be length of items.
Pop from items into last.
"#;
    let result = parse_and_check(source, |stmts| {
        let push_count = stmts.iter().filter(|s| matches!(s, Stmt::Push { .. })).count();
        let has_length = stmts.iter().any(|s| {
            if let Stmt::Let { value, .. } = s {
                matches!(**value, Expr::Length { .. })
            } else {
                false
            }
        });
        let has_pop = stmts.iter().any(|s| matches!(s, Stmt::Pop { into: Some(_), .. }));

        push_count == 3 && has_length && has_pop
    });
    assert!(result, "Expected 3 Push, Length, and Pop with binding");
}
