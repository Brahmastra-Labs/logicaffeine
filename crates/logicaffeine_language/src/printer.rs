//! English rendering of imperative expressions — the shared surface behind
//! power-assert failure messages, property counterexamples, and mutation
//! survivor reports.
//!
//! [`render_expr`] turns an [`Expr`] back into source-like English (`a plus b`,
//! `the total of xs`, `x is at least 10`). [`subexpressions`] collects the
//! parts worth showing a runtime value for — named things and computations,
//! never literals (already visible in the rendered text) — so a failed
//! `Expect` can annotate each with its actual value.

use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
use crate::intern::Interner;

/// The English word(s) a binary operator reads as, in infix position.
fn op_word(op: BinaryOpKind) -> &'static str {
    use BinaryOpKind::*;
    match op {
        Add => "plus",
        Subtract => "minus",
        Multiply => "times",
        Divide | ExactDivide => "divided by",
        FloorDivide => "floor-divided by",
        Pow => "to the power of",
        Modulo => "modulo",
        Eq => "is",
        NotEq => "is not",
        Lt => "is less than",
        Gt => "is greater than",
        LtEq => "is at most",
        GtEq => "is at least",
        And => "and",
        Or => "or",
        Concat => "combined with",
        SeqConcat => "followed by",
        ApproxEq => "is approximately",
        BitXor => "xor",
        BitAnd => "bitand",
        BitOr => "bitor",
        Shl => "shifted left by",
        Shr => "shifted right by",
    }
}

/// Render a literal as it reads in source.
fn render_literal(lit: &Literal, interner: &Interner) -> String {
    match lit {
        Literal::Number(n) => n.to_string(),
        Literal::Float(f) => f.to_string(),
        Literal::Text(s) => format!("{:?}", interner.resolve(*s)),
        Literal::Boolean(b) => b.to_string(),
        Literal::Nothing => "nothing".to_string(),
        Literal::Char(c) => format!("'{c}'"),
        Literal::Duration(ns) => format!("{ns}ns"),
        _ => "…".to_string(),
    }
}

/// Render an expression as source-like English. Exotic shapes fall back to an
/// ellipsis rather than panicking — a failure message must never crash.
pub fn render_expr(expr: &Expr<'_>, interner: &Interner) -> String {
    match expr {
        Expr::Literal(lit) => render_literal(lit, interner),
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            format!(
                "{} {} {}",
                render_expr(left, interner),
                op_word(*op),
                render_expr(right, interner)
            )
        }
        Expr::Not { operand } => format!("not {}", render_expr(operand, interner)),
        Expr::Call { function, args } => {
            let rendered: Vec<String> = args.iter().map(|a| render_expr(a, interner)).collect();
            format!("{}({})", interner.resolve(*function), rendered.join(", "))
        }
        Expr::Index { collection, index } => {
            format!(
                "item {} of {}",
                render_expr(index, interner),
                render_expr(collection, interner)
            )
        }
        Expr::Length { collection } => {
            format!("the length of {}", render_expr(collection, interner))
        }
        Expr::Contains { collection, value } => {
            format!(
                "{} contains {}",
                render_expr(collection, interner),
                render_expr(value, interner)
            )
        }
        Expr::FieldAccess { object, field } => {
            format!(
                "the {} of {}",
                interner.resolve(*field),
                render_expr(object, interner)
            )
        }
        Expr::List(items) => {
            let rendered: Vec<String> = items.iter().map(|i| render_expr(i, interner)).collect();
            format!("[{}]", rendered.join(", "))
        }
        _ => "…".to_string(),
    }
}

/// The subexpressions worth showing a runtime value for, in a stable order:
/// the whole expression's interesting operands and their operands, depth-first,
/// EXCLUDING bare literals (already visible in the rendered text). Used to
/// build a failed expectation's power-assert annotations.
pub fn subexpressions<'a, 'e>(expr: &'e Expr<'a>, out: &mut Vec<&'e Expr<'a>>) {
    match expr {
        Expr::Literal(_) => {}
        Expr::Identifier(_) => out.push(expr),
        Expr::BinaryOp { left, right, .. } => {
            subexpressions(left, out);
            subexpressions(right, out);
        }
        Expr::Not { operand } => subexpressions(operand, out),
        Expr::Call { args, .. } => {
            out.push(expr);
            for a in args {
                subexpressions(a, out);
            }
        }
        Expr::Index { collection, index } => {
            out.push(expr);
            subexpressions(collection, out);
            subexpressions(index, out);
        }
        Expr::Length { collection } => {
            out.push(expr);
            subexpressions(collection, out);
        }
        Expr::FieldAccess { object, .. } => {
            out.push(expr);
            subexpressions(object, out);
        }
        Expr::Contains { collection, value } => {
            subexpressions(collection, out);
            subexpressions(value, out);
        }
        Expr::List(items) => {
            for i in items {
                subexpressions(i, out);
            }
        }
        _ => {}
    }
}
