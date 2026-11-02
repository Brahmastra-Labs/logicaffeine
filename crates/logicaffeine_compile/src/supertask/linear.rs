//! Terms linear in one variable, the way the engine reads a condition: the search over every
//! natural finds where its predicate can change, and a loop's counter where its guard can fail.
//! A term `a·n + b` keeps its coefficients as invariant expressions, so the runtime computes them
//! in whatever kind of number the program's values are.

use logicaffeine_base::Symbol;

use super::build::{add, scale, Build};
use super::clock::reads;
use crate::ast::stmt::{BinaryOpKind, Expr};

/// A term `a·n + b` in one variable; an absent coefficient is zero.
#[derive(Clone, Copy)]
pub(super) struct Linear<'a> {
    pub(super) a: Option<&'a Expr<'a>>,
    pub(super) b: Option<&'a Expr<'a>>,
}

/// Whether `e` is simple and reads neither `n` nor a name `assigned` holds.
pub(super) fn invariant_in(e: &Expr<'_>, n: Symbol, assigned: &[Symbol]) -> bool {
    let mut names = Vec::new();
    reads(e, &mut names).is_some() && names.iter().all(|s| *s != n && !assigned.contains(s))
}

/// `e` as a term `a·n + b`, or `None` when it is not linear in `n` with invariant coefficients.
pub(super) fn linear<'a>(e: &'a Expr<'a>, n: Symbol, assigned: &[Symbol], b: &Build<'a, '_>) -> Option<Linear<'a>> {
    if invariant_in(e, n, assigned) {
        return Some(Linear { a: None, b: Some(e) });
    }
    match e {
        Expr::Identifier(s) if *s == n => Some(Linear {
            a: Some(b.number(1)),
            b: None,
        }),
        Expr::BinaryOp { op, left, right } => {
            let l = linear(left, n, assigned, b)?;
            let r = linear(right, n, assigned, b)?;
            match op {
                BinaryOpKind::Add | BinaryOpKind::Subtract => {
                    let negate = *op == BinaryOpKind::Subtract;
                    Some(Linear {
                        a: add(b, l.a, r.a, negate),
                        b: add(b, l.b, r.b, negate),
                    })
                }
                BinaryOpKind::Multiply if r.a.is_none() => {
                    let by = r.b.unwrap_or_else(|| b.number(0));
                    Some(Linear {
                        a: scale(b, l.a, by, BinaryOpKind::Multiply),
                        b: scale(b, l.b, by, BinaryOpKind::Multiply),
                    })
                }
                BinaryOpKind::Multiply if l.a.is_none() => {
                    let by = l.b.unwrap_or_else(|| b.number(0));
                    Some(Linear {
                        a: scale(b, r.a, by, BinaryOpKind::Multiply),
                        b: scale(b, r.b, by, BinaryOpKind::Multiply),
                    })
                }
                BinaryOpKind::ExactDivide if r.a.is_none() => {
                    let by = r.b?;
                    Some(Linear {
                        a: scale(b, l.a, by, BinaryOpKind::ExactDivide),
                        b: scale(b, l.b, by, BinaryOpKind::ExactDivide),
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// `(a·n + b) % m` with `m` invariant: the term and `m`.
pub(super) fn remainder<'a>(
    e: &'a Expr<'a>,
    n: Symbol,
    assigned: &[Symbol],
    b: &Build<'a, '_>,
) -> Option<(Linear<'a>, &'a Expr<'a>)> {
    let Expr::BinaryOp {
        op: BinaryOpKind::Modulo,
        left,
        right,
    } = e
    else {
        return None;
    };
    if !invariant_in(right, n, assigned) {
        return None;
    }
    Some((linear(left, n, assigned, b)?, *right))
}
