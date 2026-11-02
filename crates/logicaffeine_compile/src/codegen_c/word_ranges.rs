//! The range each integer name of one body can hold, so an integer expression over them that provably stays
//! within a machine word is computed in machine words and every other one is checked.
//!
//! The analysis is flow-insensitive: a name's range is the join of the ranges of every value the body binds it
//! to, and a value's range is its expression's interval over the ranges of the names it reads, to a fixpoint. A
//! name still growing after a few rounds (a counter, an accumulator) has no bound. A parameter, a program binding,
//! an item read out of a collection and a call's result have none either; a length lies within what memory can
//! hold, and an operator that bounds its result (`x & c`, `x % c`, see `types::bounded_int`) bounds it whatever
//! its operands are. A range is kept in `i128`, so an interval step past `i64` is seen, not wrapped.

use std::collections::HashMap;

use super::types::{bounded_int, for_each_binding, Binding};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt};
use crate::intern::Symbol;

/// An inclusive integer interval; `None` bounds nothing.
pub(super) type Range = Option<(i128, i128)>;

/// The most elements a collection can hold, which bounds its length.
const LENGTH_CEILING: i128 = 1 << 62;

/// Rounds of the fixpoint before a name still growing loses its bound.
const ROUNDS: usize = 4;

/// Whether `range` lies within `i64`.
pub(super) fn fits_word(range: Range) -> bool {
    range.is_some_and(|(lo, hi)| lo >= i64::MIN as i128 && hi <= i64::MAX as i128)
}

/// The ranges of one body's integer names.
#[derive(Default)]
pub(super) struct WordRanges {
    ranges: HashMap<Symbol, Range>,
}

impl WordRanges {
    /// The ranges of the names `body` binds; `shared` are names other bodies bind too (the program bindings
    /// functions reach), which have no bound here.
    pub(super) fn of(body: &[Stmt], shared: &std::collections::HashSet<Symbol>) -> WordRanges {
        let mut bindings: Vec<(Symbol, &Expr)> = Vec::new();
        let mut unbounded: Vec<Symbol> = shared.iter().copied().collect();
        for_each_binding(body, |target, binding| match binding {
            Binding::Value(value, _) => bindings.push((target, value)),
            Binding::Field { .. } => unbounded.push(target),
        });
        let mut counted: Vec<(Symbol, &Expr, &Expr)> = Vec::new();
        let mut raw_code = false;
        logicaffeine_language::ast_depth::walk_nodes(body, |node| {
            use logicaffeine_language::ast_depth::{Visited, Walk};
            match node {
                Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                    Walk::Past
                }
                Visited::Stmt(Stmt::Repeat {
                    pattern, iterable, ..
                }) => {
                    match (pattern, iterable) {
                        (Pattern::Identifier(i), Expr::Range { start, end }) => {
                            counted.push((*i, start, end))
                        }
                        (Pattern::Identifier(i), _) => unbounded.push(*i),
                        (Pattern::Tuple(names), _) => unbounded.extend(names.iter().copied()),
                    }
                    Walk::Into
                }
                Visited::Stmt(Stmt::Escape { .. }) | Visited::Expr(Expr::Escape { .. }) => {
                    raw_code = true;
                    Walk::Into
                }
                Visited::Stmt(s) => {
                    unbounded.extend(bound_otherwise(s));
                    Walk::Into
                }
                _ => Walk::Into,
            }
        });
        if raw_code {
            return WordRanges::default();
        }
        let mut ranges: HashMap<Symbol, Range> = HashMap::new();
        for s in &unbounded {
            ranges.insert(*s, None);
        }
        bindings.retain(|(target, _)| !shared.contains(target));
        let mut settled = false;
        for _ in 0..ROUNDS {
            let before = ranges.clone();
            apply(&bindings, &counted, &mut ranges, join_into);
            if ranges == before {
                settled = true;
                break;
            }
        }
        if !settled {
            // A name whose range still moves grows with the loop that binds it: from here on any growth loses
            // the bound, so every round either leaves the ranges as they are or unbounds a name, and the
            // rounds end.
            loop {
                let before = ranges.clone();
                apply(&bindings, &counted, &mut ranges, widen_into);
                if ranges == before {
                    break;
                }
            }
        }
        WordRanges { ranges }
    }

    /// The range of `expr`'s value, where the body's names hold their ranges and `oracle` bounds whatever this
    /// analysis leaves unbounded (an item read out of a list whose elements it proved, a parameter).
    pub(super) fn of_expr(
        &self,
        expr: &Expr,
        oracle: Option<&crate::optimize::OracleFacts>,
    ) -> Range {
        eval_with(expr, &self.ranges, oracle)
    }
}

/// One round: every binding's value, and every counted loop's bounds, joined into its name by `into`.
fn apply(
    bindings: &[(Symbol, &Expr)],
    counted: &[(Symbol, &Expr, &Expr)],
    ranges: &mut HashMap<Symbol, Range>,
    into: fn(&mut HashMap<Symbol, Range>, Symbol, Range),
) {
    for (target, value) in bindings {
        let r = eval(value, ranges);
        into(ranges, *target, r);
    }
    for (i, start, end) in counted {
        let r = match (eval(start, ranges), eval(end, ranges)) {
            (Some((lo, _)), Some((_, hi))) => Some((lo, hi.max(lo))),
            _ => None,
        };
        into(ranges, *i, r);
    }
}

/// The names `stmt` binds or changes other than by `Let`, `Set`, a loop or a match arm: each holds whatever
/// arrives, with no bound.
fn bound_otherwise(stmt: &Stmt) -> Vec<Symbol> {
    match stmt {
        Stmt::Swap { a, b } => vec![*a, *b],
        Stmt::Pop { into: Some(v), .. }
        | Stmt::AwaitMessage { into: v, .. }
        | Stmt::ReceivePipe { var: v, .. }
        | Stmt::TryReceivePipe { var: v, .. }
        | Stmt::ReadFrom { var: v, .. }
        | Stmt::Mount { var: v, .. }
        | Stmt::Sync { var: v, .. }
        | Stmt::LetPeerAgent { var: v, .. }
        | Stmt::CreatePipe { var: v, .. }
        | Stmt::LaunchTaskWithHandle { handle: v, .. }
        | Stmt::Spawn { name: v, .. }
        | Stmt::Zone { name: v, .. } => vec![*v],
        Stmt::TrySendPipe {
            result: Some(v), ..
        }
        | Stmt::AwaitTask {
            result: Some(v), ..
        } => vec![*v],
        Stmt::Select { branches, .. } => branches
            .iter()
            .filter_map(|b| match b {
                crate::ast::stmt::SelectBranch::Receive { var, .. } => Some(*var),
                crate::ast::stmt::SelectBranch::Timeout { .. } => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn join_into(ranges: &mut HashMap<Symbol, Range>, name: Symbol, r: Range) {
    let joined = match ranges.get(&name) {
        None => r,
        Some(None) => None,
        Some(Some((lo, hi))) => r.map(|(a, b)| (a.min(*lo), b.max(*hi))),
    };
    ranges.insert(name, joined);
}

/// [`join_into`], losing the bound of a name whose range grows at all.
fn widen_into(ranges: &mut HashMap<Symbol, Range>, name: Symbol, r: Range) {
    let widened = match (ranges.get(&name), r) {
        (None, r) => r,
        (Some(Some((lo, hi))), Some((a, b))) if a >= *lo && b <= *hi => Some((*lo, *hi)),
        _ => None,
    };
    ranges.insert(name, widened);
}

/// [`eval`], an expression it leaves unbounded taking the range `oracle` proves for it.
fn eval_with(
    expr: &Expr,
    ranges: &HashMap<Symbol, Range>,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Range {
    let proven = || {
        oracle
            .and_then(|o| o.expr_int_range(expr))
            .map(|(lo, hi)| (lo as i128, hi as i128))
    };
    match expr {
        Expr::BinaryOp {
            op: op @ (BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply),
            left,
            right,
        } if bounded_int(expr).is_none() => {
            let combined = (|| {
                let (a, b) = (
                    eval_with(left, ranges, oracle)?,
                    eval_with(right, ranges, oracle)?,
                );
                let r = match op {
                    BinaryOpKind::Add => (a.0.checked_add(b.0)?, a.1.checked_add(b.1)?),
                    BinaryOpKind::Subtract => (a.0.checked_sub(b.1)?, a.1.checked_sub(b.0)?),
                    _ => {
                        let c = [
                            a.0.checked_mul(b.0)?,
                            a.0.checked_mul(b.1)?,
                            a.1.checked_mul(b.0)?,
                            a.1.checked_mul(b.1)?,
                        ];
                        (*c.iter().min()?, *c.iter().max()?)
                    }
                };
                fits_word(Some(r)).then_some(r)
            })();
            combined.or_else(proven)
        }
        _ => eval(expr, ranges).or_else(proven),
    }
}

/// The interval of `expr` over `ranges`; a name the body never binds has no bound.
fn eval(expr: &Expr, ranges: &HashMap<Symbol, Range>) -> Range {
    if let Some((lo, hi)) = bounded_int(expr) {
        return Some((lo as i128, hi as i128));
    }
    match expr {
        Expr::Literal(Literal::Number(n)) => Some((*n as i128, *n as i128)),
        Expr::Identifier(s) => match ranges.get(s) {
            Some(r) => *r,
            None => None,
        },
        Expr::Length { .. } => Some((0, LENGTH_CEILING)),
        Expr::BinaryOp { op, left, right } => {
            let (a, b) = (eval(left, ranges)?, eval(right, ranges)?);
            let r = match op {
                BinaryOpKind::Add => (a.0.checked_add(b.0)?, a.1.checked_add(b.1)?),
                BinaryOpKind::Subtract => (a.0.checked_sub(b.1)?, a.1.checked_sub(b.0)?),
                BinaryOpKind::Multiply => {
                    let corners = [
                        a.0.checked_mul(b.0)?,
                        a.0.checked_mul(b.1)?,
                        a.1.checked_mul(b.0)?,
                        a.1.checked_mul(b.1)?,
                    ];
                    (*corners.iter().min()?, *corners.iter().max()?)
                }
                _ => return None,
            };
            fits_word(Some(r)).then_some(r)
        }
        _ => None,
    }
}
