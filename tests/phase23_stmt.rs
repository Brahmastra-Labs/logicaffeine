use logos::ast::{Stmt, Expr, Literal, Block};
use logos::intern::Symbol;

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
