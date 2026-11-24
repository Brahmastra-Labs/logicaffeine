//! What a versioning guard decides before the program runs.
//!
//! A versioner guards its fast clone with comparisons of the region's names against bounds it solved. Where an
//! earlier pass folded a name to its literal, a part of the guard compares two literals, and the program would
//! evaluate it on every run to the same answer. Deciding it here keeps it out of the program: a guard that always
//! holds is the clone alone, a guard that never holds is no versioning, and a guard that holds at run time keeps
//! only the parts that can go either way. Every versioner builds its guard from comparisons of names, lengths and
//! literals joined by `and`, so a part left unevaluated changes nothing but the cost.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
use crate::intern::Symbol;

/// What a guard decides.
pub(crate) enum Decided<'a> {
    /// It holds on every run: the fast clone is the program.
    Always,
    /// It holds on no run: the region is not versioned.
    Never,
    /// It holds or not at run time; the expression is the guard without the parts that always hold.
    AtRunTime(&'a Expr<'a>),
}

/// Decide `guard`.
pub(crate) fn decide<'a>(guard: &'a Expr<'a>, ea: &'a Arena<Expr<'a>>) -> Decided<'a> {
    match guard {
        Expr::Literal(Literal::Boolean(b)) => {
            if *b {
                Decided::Always
            } else {
                Decided::Never
            }
        }
        Expr::BinaryOp {
            op: BinaryOpKind::And,
            left,
            right,
        } => match (decide(left, ea), decide(right, ea)) {
            (Decided::Never, _) | (_, Decided::Never) => Decided::Never,
            (Decided::Always, rest) | (rest, Decided::Always) => rest,
            (Decided::AtRunTime(l), Decided::AtRunTime(r)) => {
                if std::ptr::eq(l, *left) && std::ptr::eq(r, *right) {
                    Decided::AtRunTime(guard)
                } else {
                    Decided::AtRunTime(ea.alloc(Expr::BinaryOp {
                        op: BinaryOpKind::And,
                        left: l,
                        right: r,
                    }))
                }
            }
        },
        Expr::BinaryOp { op, left, right } => {
            match (literal_number(left), literal_number(right)) {
                (Some(l), Some(r)) => match compare(*op, l, r) {
                    Some(true) => Decided::Always,
                    Some(false) => Decided::Never,
                    None => Decided::AtRunTime(guard),
                },
                _ => Decided::AtRunTime(guard),
            }
        }
        _ => Decided::AtRunTime(guard),
    }
}

/// Which way a part of a guard bounds a name.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    /// `name <= bound`.
    Upper,
    /// `name >= bound`.
    Lower,
}

/// The name a comparison bounds by a literal, which way, and the bound, as an inclusive one (`n < 8` is `n <= 7`).
fn bound_of(part: &Expr) -> Option<(Symbol, Side, i64)> {
    let Expr::BinaryOp { op, left, right } = part else {
        return None;
    };
    let (name, op, bound) = match (&**left, &**right) {
        (Expr::Identifier(x), r) => (*x, *op, literal_number(r)?),
        (l, Expr::Identifier(x)) => {
            let flipped = match op {
                BinaryOpKind::Lt => BinaryOpKind::Gt,
                BinaryOpKind::LtEq => BinaryOpKind::GtEq,
                BinaryOpKind::Gt => BinaryOpKind::Lt,
                BinaryOpKind::GtEq => BinaryOpKind::LtEq,
                _ => return None,
            };
            (*x, flipped, literal_number(l)?)
        }
        _ => return None,
    };
    match op {
        BinaryOpKind::LtEq => Some((name, Side::Upper, bound)),
        BinaryOpKind::Lt => Some((name, Side::Upper, bound.checked_sub(1)?)),
        BinaryOpKind::GtEq => Some((name, Side::Lower, bound)),
        BinaryOpKind::Gt => Some((name, Side::Lower, bound.checked_add(1)?)),
        _ => None,
    }
}

/// The parts of a guard joined by `and`, with each bound of a name by a literal stated once, at its first place, as
/// the tightest any part states the same way: `n <= 2^30 and n <= 2^28` is `n <= 2^28`. A dropped part's bound is
/// implied by the kept one, and the name is still bounded no later than before, ahead of any part that computes with
/// it.
pub(crate) fn tightened<'a>(parts: &[&'a Expr<'a>], ea: &'a Arena<Expr<'a>>) -> Vec<&'a Expr<'a>> {
    let mut tightest: HashMap<(Symbol, Side), i64> = HashMap::new();
    for part in parts {
        if let Some((name, side, bound)) = bound_of(part) {
            tightest
                .entry((name, side))
                .and_modify(|t| {
                    *t = match side {
                        Side::Upper => (*t).min(bound),
                        Side::Lower => (*t).max(bound),
                    }
                })
                .or_insert(bound);
        }
    }
    let mut stated: HashSet<(Symbol, Side)> = HashSet::new();
    parts
        .iter()
        .filter_map(|part| match bound_of(part) {
            None => Some(*part),
            Some((name, side, bound)) => {
                if !stated.insert((name, side)) {
                    return None;
                }
                let t = tightest[&(name, side)];
                if t == bound {
                    return Some(*part);
                }
                let op = match side {
                    Side::Upper => BinaryOpKind::LtEq,
                    Side::Lower => BinaryOpKind::GtEq,
                };
                Some(&*ea.alloc(Expr::BinaryOp {
                    op,
                    left: ea.alloc(Expr::Identifier(name)),
                    right: ea.alloc(Expr::Literal(Literal::Number(t))),
                }))
            }
        })
        .collect()
}

/// An integer literal, possibly negated.
fn literal_number(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => match (&**left, &**right) {
            (Expr::Literal(Literal::Number(0)), Expr::Literal(Literal::Number(n))) => n.checked_neg(),
            _ => None,
        },
        _ => None,
    }
}

/// `l op r` for a comparison `op`; `None` for any other operator.
fn compare(op: BinaryOpKind, l: i64, r: i64) -> Option<bool> {
    Some(match op {
        BinaryOpKind::Lt => l < r,
        BinaryOpKind::LtEq => l <= r,
        BinaryOpKind::Gt => l > r,
        BinaryOpKind::GtEq => l >= r,
        BinaryOpKind::Eq => l == r,
        BinaryOpKind::NotEq => l != r,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/guard/tests.rs"]
mod tests;
