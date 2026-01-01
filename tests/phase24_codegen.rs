use std::collections::HashSet;
use logos::arena::Arena;
use logos::ast::{Expr, Literal, Stmt, BinaryOpKind};
use logos::codegen::{codegen_expr, codegen_stmt, codegen_program, RefinementContext};
use logos::intern::{Interner, Symbol};
use logos::analysis::{TypeRegistry, PolicyRegistry};

// Empty LWW fields set for tests that don't involve CRDTs
fn empty_lww_fields() -> HashSet<(String, String)> {
    HashSet::new()
}

#[test]
fn codegen_module_exists() {
    let _ = codegen_expr;
    let _ = codegen_stmt;
    let _ = codegen_program;
}

#[test]
fn codegen_literal_number() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let expr = Expr::Literal(Literal::Number(42));
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "42");
}

#[test]
fn codegen_literal_boolean_true() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let expr = Expr::Literal(Literal::Boolean(true));
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "true");
}

#[test]
fn codegen_literal_boolean_false() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let expr = Expr::Literal(Literal::Boolean(false));
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "false");
}

#[test]
fn codegen_literal_text() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let text_sym = interner.intern("hello world");
    let expr = Expr::Literal(Literal::Text(text_sym));
    let result = codegen_expr(&expr, &interner, &synced_vars);
    // String::from() ensures we get String type, not &str
    assert_eq!(result, "String::from(\"hello world\")");
}

#[test]
fn codegen_literal_nothing() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let expr = Expr::Literal(Literal::Nothing);
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "()");
}

#[test]
fn codegen_identifier() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let var_sym = interner.intern("x");
    let expr = Expr::Identifier(var_sym);
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "x");
}

#[test]
fn codegen_binary_add() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let arena: Arena<Expr> = Arena::new();
    let left = arena.alloc(Expr::Literal(Literal::Number(1)));
    let right = arena.alloc(Expr::Literal(Literal::Number(2)));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    };
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "(1 + 2)");
}

#[test]
fn codegen_binary_eq() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let left = arena.alloc(Expr::Identifier(x));
    let right = arena.alloc(Expr::Literal(Literal::Number(5)));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Eq,
        left,
        right,
    };
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "(x == 5)");
}

#[test]
fn codegen_index_1_indexed() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let list = interner.intern("list");
    let arena: Arena<Expr> = Arena::new();
    let collection = arena.alloc(Expr::Identifier(list));
    // Phase 43D: Index now takes an expression
    let index = arena.alloc(Expr::Literal(Literal::Number(1)));
    let expr = Expr::Index {
        collection,
        index,
    };
    let result = codegen_expr(&expr, &interner, &synced_vars);
    // Phase 43D: Now uses logos_index helper for 1-based indexing
    assert_eq!(result, "logos_index(&list, 1)");
}

#[test]
fn codegen_index_5_becomes_4() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let items = interner.intern("items");
    let arena: Arena<Expr> = Arena::new();
    let collection = arena.alloc(Expr::Identifier(items));
    // Phase 43D: Index now takes an expression
    let index = arena.alloc(Expr::Literal(Literal::Number(5)));
    let expr = Expr::Index {
        collection,
        index,
    };
    let result = codegen_expr(&expr, &interner, &synced_vars);
    // Phase 43D: Now uses logos_index helper for 1-based indexing
    assert_eq!(result, "logos_index(&items, 5)");
}

#[test]
fn codegen_let_statement() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(42)));
    let stmt = Stmt::Let {
        var: x,
        ty: None,
        value,
        mutable: false,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
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
        ty: None,
        value,
        mutable: true,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
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
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
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
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
    assert_eq!(result, "return 42;\n");
}

#[test]
fn codegen_return_without_value() {
    let interner = Interner::new();
    let stmt = Stmt::Return { value: None };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
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
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
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
        decreasing: None,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
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
        ty: None,
        value,
        mutable: false,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 1, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
    assert_eq!(result, "    let x = 5;\n");
}

#[test]
fn codegen_program_wraps_in_main() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let policies = PolicyRegistry::new();
    let stmts: &[Stmt] = &[];
    let result = codegen_program(stmts, &registry, &policies, &interner);
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
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &mut synced_vars);
    assert_eq!(result, "println();\n");
}
