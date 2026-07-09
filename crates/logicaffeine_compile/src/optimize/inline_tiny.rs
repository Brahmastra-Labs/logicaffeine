//! Tiny-function inlining for the RUN PATH.
//!
//! The spectral_norm shape — a one-expression arithmetic helper called per
//! inner-loop element — pays the native call boundary on every iteration.
//! Inlining the body turns the loop into straight arithmetic the region
//! tier compiles whole.
//!
//! Fail-closed candidate rules (every one is load-bearing):
//! - body is EXACTLY `Return <expr>` where `<expr>` uses only literals,
//!   identifiers, binary operators and `not` — no calls (kills recursion
//!   and effects in one stroke), no indexing (collection state), nothing
//!   else;
//! - a call site inlines only when EVERY argument is a literal or an
//!   identifier: parameters may occur many times in the body, and
//!   duplicating a compound argument would duplicate its cost and its
//!   errors.
//!
//! Like every inliner (including the AOT supercompiler's), this erases the
//! call-depth accounting a bytecode call would perform — the kernel's
//! resource limit, not a semantic boundary.

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};

/// Is this expression in the inlinable fragment (pure, total-modulo-division,
/// duplicable structure)?
fn body_inlinable(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) => true,
        Expr::BinaryOp { left, right, .. } => body_inlinable(left) && body_inlinable(right),
        Expr::Not { operand } => body_inlinable(operand),
        _ => false,
    }
}

fn arg_atomic(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(_) | Expr::Identifier(_))
}

/// Substitute parameters with argument expressions inside an inlinable body.
fn subst<'a>(
    expr: &'a Expr<'a>,
    bind: &HashMap<Symbol, &'a Expr<'a>>,
    arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    match expr {
        Expr::Identifier(sym) => bind.get(sym).copied().unwrap_or(expr),
        Expr::Literal(_) => expr,
        Expr::BinaryOp { op, left, right } => arena.alloc(Expr::BinaryOp {
            op: *op,
            left: subst(left, bind, arena),
            right: subst(right, bind, arena),
        }),
        Expr::Not { operand } => arena.alloc(Expr::Not { operand: subst(operand, bind, arena) }),
        _ => unreachable!("body_inlinable admitted only the fragment above"),
    }
}

struct Inliner<'a> {
    bodies: HashMap<Symbol, (Vec<Symbol>, &'a Expr<'a>)>,
}

impl<'a> Inliner<'a> {
    fn rewrite_expr(&self, expr: &'a Expr<'a>, arena: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
        match expr {
            Expr::Call { function, args } => {
                let new_args: Vec<&'a Expr<'a>> =
                    args.iter().map(|a| self.rewrite_expr(a, arena)).collect();
                if let Some((params, body)) = self.bodies.get(function) {
                    if params.len() == new_args.len() && new_args.iter().all(|a| arg_atomic(a)) {
                        let bind: HashMap<Symbol, &'a Expr<'a>> =
                            params.iter().copied().zip(new_args.iter().copied()).collect();
                        return subst(body, &bind, arena);
                    }
                }
                if new_args
                    .iter()
                    .zip(args.iter())
                    .all(|(n, o)| std::ptr::eq(*n, *o))
                {
                    expr
                } else {
                    arena.alloc(Expr::Call { function: *function, args: new_args })
                }
            }
            Expr::BinaryOp { op, left, right } => {
                let l = self.rewrite_expr(left, arena);
                let r = self.rewrite_expr(right, arena);
                if std::ptr::eq(l, *left) && std::ptr::eq(r, *right) {
                    expr
                } else {
                    arena.alloc(Expr::BinaryOp { op: *op, left: l, right: r })
                }
            }
            Expr::Not { operand } => {
                let o = self.rewrite_expr(operand, arena);
                if std::ptr::eq(o, *operand) {
                    expr
                } else {
                    arena.alloc(Expr::Not { operand: o })
                }
            }
            Expr::Index { collection, index } => {
                let c = self.rewrite_expr(collection, arena);
                let i = self.rewrite_expr(index, arena);
                if std::ptr::eq(c, *collection) && std::ptr::eq(i, *index) {
                    expr
                } else {
                    arena.alloc(Expr::Index { collection: c, index: i })
                }
            }
            Expr::Length { collection } => {
                let c = self.rewrite_expr(collection, arena);
                if std::ptr::eq(c, *collection) {
                    expr
                } else {
                    arena.alloc(Expr::Length { collection: c })
                }
            }
            _ => expr,
        }
    }

    fn rewrite_block(
        &self,
        block: &'a [Stmt<'a>],
        expr_arena: &'a Arena<Expr<'a>>,
        stmt_arena: &'a Arena<Stmt<'a>>,
    ) -> &'a [Stmt<'a>] {
        let out: Vec<Stmt<'a>> = block
            .iter()
            .cloned()
            .map(|s| self.rewrite_stmt(s, expr_arena, stmt_arena))
            .collect();
        stmt_arena.alloc_slice(out)
    }

    fn rewrite_stmt(
        &self,
        stmt: Stmt<'a>,
        expr_arena: &'a Arena<Expr<'a>>,
        stmt_arena: &'a Arena<Stmt<'a>>,
    ) -> Stmt<'a> {
        let re = |e: &'a Expr<'a>| self.rewrite_expr(e, expr_arena);
        match stmt {
            Stmt::Let { var, ty, value, mutable } => {
                Stmt::Let { var, ty, value: re(value), mutable }
            }
            Stmt::Set { target, value } => Stmt::Set { target, value: re(value) },
            Stmt::If { cond, then_block, else_block } => Stmt::If {
                cond: re(cond),
                then_block: self.rewrite_block(then_block, expr_arena, stmt_arena),
                else_block: else_block.map(|b| self.rewrite_block(b, expr_arena, stmt_arena)),
            },
            Stmt::While { cond, body, decreasing } => Stmt::While {
                cond: re(cond),
                body: self.rewrite_block(body, expr_arena, stmt_arena),
                decreasing,
            },
            Stmt::Repeat { pattern, iterable, body } => Stmt::Repeat {
                pattern,
                iterable: re(iterable),
                body: self.rewrite_block(body, expr_arena, stmt_arena),
            },
            Stmt::FunctionDef {
                name,
                params,
                generics,
                body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            } => Stmt::FunctionDef {
                name,
                params,
                generics,
                body: self.rewrite_block(body, expr_arena, stmt_arena),
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            },
            Stmt::Show { object, recipient } => Stmt::Show { object: re(object), recipient },
            Stmt::Return { value } => Stmt::Return { value: value.map(re) },
            Stmt::RuntimeAssert { condition, hard } => Stmt::RuntimeAssert { condition: re(condition) , hard },
            Stmt::Push { value, collection } => Stmt::Push { value: re(value), collection },
            Stmt::SetIndex { collection, index, value } => {
                Stmt::SetIndex { collection, index: re(index), value: re(value) }
            }
            Stmt::Call { function, args } => Stmt::Call {
                function,
                args: args.into_iter().map(re).collect(),
            },
            other => other,
        }
    }
}

/// Inline every tiny pure helper at its atomic-argument call sites.
pub fn inline_tiny_fns<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    _interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut bodies: HashMap<Symbol, (Vec<Symbol>, &'a Expr<'a>)> = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef { name, params, body, is_native: false, generics, .. } = s {
            if !generics.is_empty() {
                continue;
            }
            if let [Stmt::Return { value: Some(expr) }] = body {
                if body_inlinable(expr) {
                    bodies.insert(*name, (params.iter().map(|(p, _)| *p).collect(), *expr));
                }
            }
        }
    }
    if bodies.is_empty() {
        return stmts;
    }
    let inliner = Inliner { bodies };
    stmts
        .into_iter()
        .map(|s| inliner.rewrite_stmt(s, expr_arena, stmt_arena))
        .collect()
}
