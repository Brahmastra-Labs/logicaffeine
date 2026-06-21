//! Counted-loop recognition — the single source of truth for "this loop runs a
//! statically-determinable number of times with a unit-stride induction
//! variable."
//!
//! Both the AOT loop-unroller (`optimize::unroll`) and the codegen for-range
//! peephole (`codegen::peephole`) consume this. The recognizer is pure AST
//! analysis; it makes no codegen decisions and emits nothing.
//!
//! Two surface forms collapse to one [`CountedLoop`]:
//!   - the counted-`While`: `Let/Set counter = start; While counter <= / < limit:
//!     <body>; Set counter to counter + 1`, and
//!   - the counted-`Repeat`: `Repeat for v from a to b:` (an `Expr::Range`,
//!     inclusive).

use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Pattern, Stmt};
use crate::codegen::peephole::{
    body_modifies_var, body_mutates_collection, collect_expr_symbols, is_counter_increment,
    is_simple_expr,
};
use crate::intern::Symbol;

/// A loop with a unit-stride induction variable and explicit start/limit
/// expressions. `body_without_increment` excludes the trailing counter bump for
/// the `While` form (the `Repeat` form has no such statement, so it is the full
/// body). The two surface forms are distinguished by which extractor produced
/// the value, so callers handle the `While` counter's post-loop binding (a real
/// outer variable) separately from a loop-scoped `Repeat` variable.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CountedLoop<'a> {
    pub var: Symbol,
    pub start: &'a Expr<'a>,
    pub limit: &'a Expr<'a>,
    /// `true` for `<=` (and the inclusive `Range`); `false` for `<`.
    pub inclusive: bool,
    pub body_without_increment: Block<'a>,
}

/// Recognize a counted-`While` at `stmts[idx..]`: an init `Let`/`Set` of the
/// counter followed by a `While counter </<= limit` whose last body statement
/// bumps the counter by one. Returns the loop plus the number of leading
/// statements it spans (always 2 — the init and the `While`).
///
/// This mirrors the recognizer half of `codegen::peephole::try_emit_for_range_pattern`
/// exactly, so the two stay in lockstep.
pub(crate) fn extract_counted_while<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
) -> Option<(CountedLoop<'a>, usize)> {
    if idx + 1 >= stmts.len() {
        return None;
    }

    // Statement 1: `Let counter = start` or `Set counter to start`. The start
    // must be a simple expression (literal / identifier / simple arithmetic).
    let (counter_sym, counter_start_expr) = match stmts[idx] {
        Stmt::Let { var, value, .. } => {
            if is_simple_expr(value) {
                (*var, *value)
            } else {
                return None;
            }
        }
        Stmt::Set { target, value } => {
            if is_simple_expr(value) {
                (*target, *value)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 2: `While counter <= limit` (inclusive) or `While counter < limit`.
    let (body, limit_expr, is_exclusive): (Block<'a>, &'a Expr<'a>, bool) = match stmts[idx + 1] {
        Stmt::While { cond, body, .. } => match cond {
            Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => match left {
                Expr::Identifier(sym) if *sym == counter_sym => (*body, *right, false),
                _ => return None,
            },
            Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => match left {
                Expr::Identifier(sym) if *sym == counter_sym => (*body, *right, true),
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };

    if body.is_empty() {
        return None;
    }

    // The last body statement must be the unit increment `Set counter to counter + 1`.
    if !is_counter_increment(&body[body.len() - 1], counter_sym) {
        return None;
    }
    let body_without_increment = &body[..body.len() - 1];

    // The counter must not be reassigned anywhere except that final increment.
    if body_modifies_var(body_without_increment, counter_sym) {
        return None;
    }

    if !is_simple_expr(limit_expr) {
        return None;
    }

    // No symbol appearing in the limit may be mutated (value or collection) in
    // the body — otherwise the trip count is not loop-invariant.
    let mut limit_syms = Vec::new();
    collect_expr_symbols(limit_expr, &mut limit_syms);
    for sym in &limit_syms {
        if body_modifies_var(body_without_increment, *sym)
            || body_mutates_collection(body_without_increment, *sym)
        {
            return None;
        }
    }

    Some((
        CountedLoop {
            var: counter_sym,
            start: counter_start_expr,
            limit: limit_expr,
            inclusive: !is_exclusive,
            body_without_increment,
        },
        2,
    ))
}

/// Recognize a counted-`Repeat`: `Repeat for v from a to b:` lowers to an
/// `Expr::Range { start, end }` iterable (inclusive) over a single identifier.
/// `Repeat for v in <list>` is rejected — its trip count is not statically a
/// range.
pub(crate) fn extract_counted_repeat<'a>(stmt: &Stmt<'a>) -> Option<CountedLoop<'a>> {
    if let Stmt::Repeat { pattern, iterable, body } = stmt {
        if let Pattern::Identifier(var) = pattern {
            if let Expr::Range { start, end } = iterable {
                return Some(CountedLoop {
                    var: *var,
                    start: *start,
                    limit: *end,
                    inclusive: true,
                    body_without_increment: *body,
                });
            }
        }
    }
    None
}

/// Fold an expression to an `i64` when it is built purely from integer literals
/// and constant `+ - * / %` arithmetic. Used to test whether a counted loop's
/// start and limit are compile-time constants (and, after the unroller
/// substitutes a literal for an outer induction variable, whether an inner
/// bound like `i + 1` has become constant). Division/modulo by zero folds to
/// `None` rather than panicking.
pub(crate) fn const_eval_i64(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        Expr::BinaryOp { op, left, right } => {
            let l = const_eval_i64(left)?;
            let r = const_eval_i64(right)?;
            match op {
                BinaryOpKind::Add => l.checked_add(r),
                BinaryOpKind::Subtract => l.checked_sub(r),
                BinaryOpKind::Multiply => l.checked_mul(r),
                BinaryOpKind::Divide => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_div(r)
                    }
                }
                BinaryOpKind::Modulo => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_rem(r)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}
