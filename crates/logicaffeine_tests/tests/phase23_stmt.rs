use logicaffeine_language::ast::{Stmt, Expr, Literal, Block};
use logicaffeine_base::Symbol;
use logicaffeine_compile::compile::compile_to_rust;

#[test]
fn stmt_let_variant_exists() {
    fn _check<'a>(expr: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Let {
            var: Symbol::default(),
            ty: None,
            value: expr,
            mutable: false,
        }
    }
}

#[test]
fn stmt_set_variant_exists() {
    fn _check<'a>(expr: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Set {
            target: Symbol::default(),
            value: expr,
        }
    }
}

#[test]
fn stmt_call_variant_exists() {
    fn _check<'a>(args: Vec<&'a Expr<'a>>) -> Stmt<'a> {
        Stmt::Call {
            function: Symbol::default(),
            args,
        }
    }
}

#[test]
fn stmt_if_variant_exists() {
    fn _check<'a>(cond: &'a Expr<'a>, then_block: Block<'a>) -> Stmt<'a> {
        Stmt::If {
            cond,
            then_block,
            else_block: None,
        }
    }
}

#[test]
fn stmt_while_variant_exists() {
    fn _check<'a>(cond: &'a Expr<'a>, body: Block<'a>) -> Stmt<'a> {
        Stmt::While {
            cond,
            body,
            decreasing: None,
        }
    }
}

#[test]
fn stmt_return_variant_exists() {
    fn _check<'a>() -> Stmt<'a> {
        Stmt::Return { value: None }
    }
}

#[test]
fn stmt_return_with_value_variant() {
    fn _check<'a>(expr: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Return { value: Some(expr) }
    }
}

#[test]
fn expr_literal_variant_exists() {
    let _lit = Expr::Literal(Literal::Number(42));
}

#[test]
fn expr_identifier_variant_exists() {
    let _id = Expr::Identifier(Symbol::default());
}

#[test]
fn literal_variants_exist() {
    let _num = Literal::Number(42);
    let _text = Literal::Text(Symbol::default());
    let _bool = Literal::Boolean(true);
    let _nothing = Literal::Nothing;
}

// =============================================================================
// Phase 23b: Equals Assignment Syntax (x = 5)
// =============================================================================

#[test]
fn test_equals_assignment_basic() {
    let source = r#"## Main
x = 42.
Return x.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("let x = 42"), "Generated: {}", rust);
}

#[test]
fn test_equals_assignment_string() {
    let source = r#"## Main
greeting = "Hello".
Show greeting.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("let greeting"), "Generated: {}", rust);
    assert!(rust.contains("Hello"), "Generated: {}", rust);
}

#[test]
fn test_equals_assignment_expression() {
    let source = r#"## Main
result = 10 + 5.
Return result.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("let result"), "Generated: {}", rust);
}

#[test]
fn test_equals_auto_mutability() {
    let source = r#"## Main
counter = 0.
Set counter to 1.
Return counter.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("let mut counter = 0"), "Generated: {}", rust);
}

#[test]
fn test_equals_with_explicit_mut() {
    let source = r#"## Main
mut x = 5.
Set x to 10.
Return x.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("let mut x"), "Generated: {}", rust);
}

#[test]
fn test_equals_with_type_annotation() {
    let source = r#"## Main
count: Int = 42.
Return count.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("let count"), "Generated: {}", rust);
}

#[test]
fn test_let_be_still_works() {
    let source = r#"## Main
Let x be 5.
Let mutable y be 10.
Return x.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("let x = 5"), "Generated: {}", rust);
    assert!(rust.contains("let mut y = 10"), "Generated: {}", rust);
}

#[test]
fn test_mixed_syntax() {
    let source = r#"## Main
Let old_style be 1.
new_style = 2.
Let mutable old_mut be 3.
mut new_mut = 4.
Return old_style.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("let old_style = 1"), "Generated: {}", rust);
    assert!(rust.contains("let new_style = 2"), "Generated: {}", rust);
}
