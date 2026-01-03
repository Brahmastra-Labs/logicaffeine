use logos::arena::Arena;
use logos::ast::logic::{LogicExpr, Term, NumberKind};
use logos::codegen::codegen_assertion;
use logos::intern::Interner;
use logos::token::TokenType;

#[test]
fn codegen_assertion_exists() {
    let _ = codegen_assertion;
}

#[test]
fn codegen_atom_as_boolean() {
    let mut interner = Interner::new();
    let valid = interner.intern("valid");
    let expr = LogicExpr::Atom(valid);
    let result = codegen_assertion(&expr, &interner);
    assert_eq!(result, "valid");
}

#[test]
fn codegen_identity_as_equality() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let y = interner.intern("y");
    let arena: Arena<Term> = Arena::new();
    let left = arena.alloc(Term::Variable(x));
    let right = arena.alloc(Term::Variable(y));
    let expr = LogicExpr::Identity { left, right };
    let result = codegen_assertion(&expr, &interner);
    assert_eq!(result, "(x == y)");
}

#[test]
fn codegen_predicate_greater() {
    let mut interner = Interner::new();
    let greater = interner.intern("Greater");
    let x = interner.intern("x");
    let arena: Arena<Term> = Arena::new();
    let args = arena.alloc_slice([
        Term::Variable(x),
        Term::Value { kind: NumberKind::Integer(0), unit: None, dimension: None },
    ]);
    let expr = LogicExpr::Predicate { name: greater, args, world: None };
    let result = codegen_assertion(&expr, &interner);
    assert_eq!(result, "(x > 0)");
}

#[test]
fn codegen_predicate_less() {
    let mut interner = Interner::new();
    let less = interner.intern("Less");
    let x = interner.intern("x");
    let arena: Arena<Term> = Arena::new();
    let args = arena.alloc_slice([
        Term::Variable(x),
        Term::Value { kind: NumberKind::Integer(10), unit: None, dimension: None },
    ]);
    let expr = LogicExpr::Predicate { name: less, args, world: None };
    let result = codegen_assertion(&expr, &interner);
    assert_eq!(result, "(x < 10)");
}

#[test]
fn codegen_binary_and() {
    let mut interner = Interner::new();
    let a = interner.intern("a");
    let b = interner.intern("b");
    let arena: Arena<LogicExpr> = Arena::new();
    let left = arena.alloc(LogicExpr::Atom(a));
    let right = arena.alloc(LogicExpr::Atom(b));
    let expr = LogicExpr::BinaryOp {
        left,
        op: TokenType::And,
        right,
    };
    let result = codegen_assertion(&expr, &interner);
    assert_eq!(result, "(a && b)");
}

#[test]
fn codegen_binary_or() {
    let mut interner = Interner::new();
    let a = interner.intern("a");
    let b = interner.intern("b");
    let arena: Arena<LogicExpr> = Arena::new();
    let left = arena.alloc(LogicExpr::Atom(a));
    let right = arena.alloc(LogicExpr::Atom(b));
    let expr = LogicExpr::BinaryOp {
        left,
        op: TokenType::Or,
        right,
    };
    let result = codegen_assertion(&expr, &interner);
    assert_eq!(result, "(a || b)");
}

#[test]
fn codegen_unary_not() {
    let mut interner = Interner::new();
    let valid = interner.intern("valid");
    let arena: Arena<LogicExpr> = Arena::new();
    let operand = arena.alloc(LogicExpr::Atom(valid));
    let expr = LogicExpr::UnaryOp {
        op: TokenType::Not,
        operand,
    };
    let result = codegen_assertion(&expr, &interner);
    assert_eq!(result, "(!valid)");
}

#[test]
fn codegen_term_constant() {
    let mut interner = Interner::new();
    let john = interner.intern("John");
    let term = Term::Constant(john);
    let result = logos::codegen::codegen_term(&term, &interner);
    assert_eq!(result, "John");
}

#[test]
fn codegen_term_variable() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let term = Term::Variable(x);
    let result = logos::codegen::codegen_term(&term, &interner);
    assert_eq!(result, "x");
}

#[test]
fn codegen_term_integer() {
    let interner = Interner::new();
    let term = Term::Value {
        kind: NumberKind::Integer(42),
        unit: None,
        dimension: None,
    };
    let result = logos::codegen::codegen_term(&term, &interner);
    assert_eq!(result, "42");
}
