use logos::arena::Arena;
use logos::ast::{Expr, Literal, Stmt, BinaryOpKind};
use logos::codegen::{codegen_expr, codegen_stmt, codegen_program};
use logos::intern::Interner;

#[test]
fn codegen_module_exists() {
    let _ = codegen_expr;
    let _ = codegen_stmt;
    let _ = codegen_program;
}

#[test]
fn codegen_literal_number() {
    let interner = Interner::new();
    let expr = Expr::Literal(Literal::Number(42));
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "42");
}

#[test]
fn codegen_literal_boolean_true() {
    let interner = Interner::new();
    let expr = Expr::Literal(Literal::Boolean(true));
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "true");
}

#[test]
fn codegen_literal_boolean_false() {
    let interner = Interner::new();
    let expr = Expr::Literal(Literal::Boolean(false));
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "false");
}

#[test]
fn codegen_literal_text() {
    let mut interner = Interner::new();
    let text_sym = interner.intern("hello world");
    let expr = Expr::Literal(Literal::Text(text_sym));
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "\"hello world\"");
}

#[test]
fn codegen_literal_nothing() {
    let interner = Interner::new();
    let expr = Expr::Literal(Literal::Nothing);
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "()");
}

#[test]
fn codegen_identifier() {
    let mut interner = Interner::new();
    let var_sym = interner.intern("x");
    let expr = Expr::Identifier(var_sym);
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "x");
}

#[test]
fn codegen_binary_add() {
    let interner = Interner::new();
    let arena: Arena<Expr> = Arena::new();
    let left = arena.alloc(Expr::Literal(Literal::Number(1)));
    let right = arena.alloc(Expr::Literal(Literal::Number(2)));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    };
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "(1 + 2)");
}

#[test]
fn codegen_binary_eq() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let left = arena.alloc(Expr::Identifier(x));
    let right = arena.alloc(Expr::Literal(Literal::Number(5)));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Eq,
        left,
        right,
    };
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "(x == 5)");
}

#[test]
fn codegen_index_1_indexed() {
    let mut interner = Interner::new();
    let list = interner.intern("list");
    let arena: Arena<Expr> = Arena::new();
    let collection = arena.alloc(Expr::Identifier(list));
    let expr = Expr::Index {
        collection,
        index: 1,
    };
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "list[0]");
}

#[test]
fn codegen_index_5_becomes_4() {
    let mut interner = Interner::new();
    let items = interner.intern("items");
    let arena: Arena<Expr> = Arena::new();
    let collection = arena.alloc(Expr::Identifier(items));
    let expr = Expr::Index {
        collection,
        index: 5,
    };
    let result = codegen_expr(&expr, &interner);
    assert_eq!(result, "items[4]");
}

#[test]
fn codegen_let_statement() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(42)));
    let stmt = Stmt::Let {
        var: x,
        value,
        mutable: false,
    };
    let result = codegen_stmt(&stmt, &interner, 0);
    assert_eq!(result, "let x = 42;\n");
}

#[test]
fn codegen_let_mutable() {
    let mut interner = Interner::new();
    let count = interner.intern("count");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(0)));
    let stmt = Stmt::Let {
        var: count,
        value,
        mutable: true,
    };
    let result = codegen_stmt(&stmt, &interner, 0);
    assert_eq!(result, "let mut count = 0;\n");
}

#[test]
fn codegen_set_statement() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(10)));
    let stmt = Stmt::Set {
        target: x,
        value,
    };
    let result = codegen_stmt(&stmt, &interner, 0);
    assert_eq!(result, "x = 10;\n");
}

#[test]
fn codegen_return_with_value() {
    let interner = Interner::new();
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(42)));
    let stmt = Stmt::Return {
        value: Some(value),
    };
    let result = codegen_stmt(&stmt, &interner, 0);
    assert_eq!(result, "return 42;\n");
}

#[test]
fn codegen_return_without_value() {
    let interner = Interner::new();
    let stmt = Stmt::Return { value: None };
    let result = codegen_stmt(&stmt, &interner, 0);
    assert_eq!(result, "return;\n");
}

#[test]
fn codegen_if_without_else() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();

    let cond = arena.alloc(Expr::Identifier(x));

    let stmt = Stmt::If {
        cond,
        then_block: &[],
        else_block: None,
    };
    let result = codegen_stmt(&stmt, &interner, 0);
    assert!(result.contains("if x {"), "Expected 'if x {{' but got: {}", result);
    assert!(result.contains("}"), "Expected '}}' but got: {}", result);
}

#[test]
fn codegen_while_loop() {
    let mut interner = Interner::new();
    let running = interner.intern("running");
    let arena: Arena<Expr> = Arena::new();

    let cond = arena.alloc(Expr::Identifier(running));

    let stmt = Stmt::While {
        cond,
        body: &[],
    };
    let result = codegen_stmt(&stmt, &interner, 0);
    assert!(result.contains("while running {"), "Expected 'while running {{' but got: {}", result);
    assert!(result.contains("}"), "Expected '}}' but got: {}", result);
}

#[test]
fn codegen_indentation() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(5)));
    let stmt = Stmt::Let {
        var: x,
        value,
        mutable: false,
    };
    let result = codegen_stmt(&stmt, &interner, 1);
    assert_eq!(result, "    let x = 5;\n");
}

#[test]
fn codegen_program_wraps_in_main() {
    let interner = Interner::new();
    let stmts: &[Stmt] = &[];
    let result = codegen_program(stmts, &interner);
    assert!(result.contains("fn main()"), "Expected 'fn main()' but got: {}", result);
    assert!(result.contains("{"), "Expected '{{' but got: {}", result);
    assert!(result.contains("}"), "Expected '}}' but got: {}", result);
}

#[test]
fn codegen_call_statement() {
    let mut interner = Interner::new();
    let println = interner.intern("println");

    let stmt = Stmt::Call {
        function: println,
        args: vec![],
    };
    let result = codegen_stmt(&stmt, &interner, 0);
    assert_eq!(result, "println();\n");
}
