//! Tiny-function inlining for the RUN PATH.
//!
//! The spectral_norm shape — a one-expression arithmetic helper called per
//! inner-loop element — pays the native call boundary on every iteration.
//! Inlining the body turns the loop into straight arithmetic the region
//! tier compiles whole.
//!
//! Fail-closed candidate rules (every one is load-bearing):
//! - body is EXACTLY `Return <expr>` where `<expr>` uses only literals,
//!   identifiers, binary operators, `not` and calls of PURE builtins
//!   ([`BuiltinEffect::Pure`]: total, no effect, so neither recursion nor an
//!   action can hide in one) — no call of a function the program defines
//!   (a name the program defines is never taken for a builtin), no indexing
//!   (collection state), nothing else;
//! - a call site inlines when EVERY argument is a literal or an identifier:
//!   parameters may occur many times in the body, and duplicating a compound
//!   argument would duplicate its cost and its errors. A call that is a
//!   statement's whole value (`Let`, `Set`, `Return`) inlines with compound
//!   arguments too: each is bound once to a fresh name ahead of the
//!   statement, in argument order, which is exactly where the call would
//!   have evaluated it, and the body reads the fresh names;
//! - and only where none of the helper's free names is bound in the call
//!   site's frame ([`super::capture`]): the helper reads the program's
//!   bindings, never its caller's locals.
//!
//! Like every inliner (including the AOT supercompiler's), this erases the
//! call-depth accounting a bytecode call would perform — the kernel's
//! resource limit, not a semantic boundary.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};
use crate::semantics::builtins::{builtin_from_name, BuiltinEffect};

use super::capture;

/// Whether `function` names a pure builtin in this program: no function of the program's own has the name.
fn pure_builtin(function: Symbol, defined: &HashSet<Symbol>, interner: &Interner) -> bool {
    !defined.contains(&function)
        && builtin_from_name(interner.resolve(function))
            .is_some_and(|b| b.effect() == BuiltinEffect::Pure)
}

/// Is this expression in the inlinable fragment (pure, total-modulo-division,
/// duplicable structure)?
fn body_inlinable(expr: &Expr, defined: &HashSet<Symbol>, interner: &Interner) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) => true,
        Expr::BinaryOp { left, right, .. } => {
            body_inlinable(left, defined, interner) && body_inlinable(right, defined, interner)
        }
        Expr::Not { operand } => body_inlinable(operand, defined, interner),
        Expr::Call { function, args } => {
            pure_builtin(*function, defined, interner)
                && args.iter().all(|a| body_inlinable(a, defined, interner))
        }
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
        Expr::Not { operand } => arena.alloc(Expr::Not {
            operand: subst(operand, bind, arena),
        }),
        Expr::Call { function, args } => arena.alloc(Expr::Call {
            function: *function,
            args: args.iter().map(|a| subst(a, bind, arena)).collect(),
        }),
        _ => unreachable!("body_inlinable admitted only the fragment above"),
    }
}

/// A helper to inline: its parameters, its returned expression, and its free names.
struct Tiny<'a> {
    params: Vec<Symbol>,
    body: &'a Expr<'a>,
    free: HashSet<Symbol>,
}

struct Inliner<'h, 'a> {
    bodies: &'h HashMap<Symbol, Tiny<'a>>,
    /// The names bound in the frame being rewritten, which a helper's free name must not be.
    frame: HashSet<Symbol>,
}

impl<'h, 'a> Inliner<'h, 'a> {
    fn rewrite_expr(&self, expr: &'a Expr<'a>, arena: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
        match expr {
            Expr::Call { function, args } => {
                let new_args: Vec<&'a Expr<'a>> =
                    args.iter().map(|a| self.rewrite_expr(a, arena)).collect();
                if let Some(tiny) = self.bodies.get(function) {
                    if tiny.params.len() == new_args.len()
                        && new_args.iter().all(|a| arg_atomic(a))
                        && !capture::captures(&tiny.free, &self.frame)
                    {
                        let bind: HashMap<Symbol, &'a Expr<'a>> = tiny
                            .params
                            .iter()
                            .copied()
                            .zip(new_args.iter().copied())
                            .collect();
                        return subst(tiny.body, &bind, arena);
                    }
                }
                if new_args
                    .iter()
                    .zip(args.iter())
                    .all(|(n, o)| std::ptr::eq(*n, *o))
                {
                    expr
                } else {
                    arena.alloc(Expr::Call {
                        function: *function,
                        args: new_args,
                    })
                }
            }
            Expr::BinaryOp { op, left, right } => {
                let l = self.rewrite_expr(left, arena);
                let r = self.rewrite_expr(right, arena);
                if std::ptr::eq(l, *left) && std::ptr::eq(r, *right) {
                    expr
                } else {
                    arena.alloc(Expr::BinaryOp {
                        op: *op,
                        left: l,
                        right: r,
                    })
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
                    arena.alloc(Expr::Index {
                        collection: c,
                        index: i,
                    })
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
        fresh: &mut Fresh<'_>,
    ) -> &'a [Stmt<'a>] {
        let mut out: Vec<Stmt<'a>> = Vec::with_capacity(block.len());
        for s in block.iter().cloned() {
            let s = self.bind_compound_arguments(s, expr_arena, &mut out, fresh);
            out.push(self.rewrite_stmt(s, expr_arena, stmt_arena, fresh));
        }
        stmt_arena.alloc_slice(out)
    }

    /// When `stmt`'s whole value is a call of a tiny helper with a compound argument, bind each compound argument
    /// to a fresh name in `out`, in argument order, and answer the statement calling with those names: the call then
    /// has only atomic arguments, and inlines.
    fn bind_compound_arguments(
        &self,
        stmt: Stmt<'a>,
        expr_arena: &'a Arena<Expr<'a>>,
        out: &mut Vec<Stmt<'a>>,
        fresh: &mut Fresh<'_>,
    ) -> Stmt<'a> {
        let value = match &stmt {
            Stmt::Let { value, .. }
            | Stmt::Set { value, .. }
            | Stmt::Return { value: Some(value) } => *value,
            _ => return stmt,
        };
        let Expr::Call { function, args } = value else {
            return stmt;
        };
        let Some(tiny) = self.bodies.get(function) else {
            return stmt;
        };
        if tiny.params.len() != args.len()
            || args.iter().all(|a| arg_atomic(a))
            || capture::captures(&tiny.free, &self.frame)
        {
            return stmt;
        }
        let mut named: Vec<&'a Expr<'a>> = Vec::with_capacity(args.len());
        for arg in args {
            if arg_atomic(arg) {
                named.push(arg);
                continue;
            }
            let var = fresh.name();
            out.push(Stmt::Let {
                var,
                ty: None,
                value: self.rewrite_expr(arg, expr_arena),
                mutable: false,
            });
            named.push(expr_arena.alloc(Expr::Identifier(var)));
        }
        let call = expr_arena.alloc(Expr::Call {
            function: *function,
            args: named,
        });
        match stmt {
            Stmt::Let {
                var, ty, mutable, ..
            } => Stmt::Let {
                var,
                ty,
                value: call,
                mutable,
            },
            Stmt::Set { target, .. } => Stmt::Set {
                target,
                value: call,
            },
            _ => Stmt::Return { value: Some(call) },
        }
    }

    fn rewrite_stmt(
        &self,
        stmt: Stmt<'a>,
        expr_arena: &'a Arena<Expr<'a>>,
        stmt_arena: &'a Arena<Stmt<'a>>,
        fresh: &mut Fresh<'_>,
    ) -> Stmt<'a> {
        let re = |e: &'a Expr<'a>| self.rewrite_expr(e, expr_arena);
        match stmt {
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => Stmt::Let {
                var,
                ty,
                value: re(value),
                mutable,
            },
            Stmt::Set { target, value } => Stmt::Set {
                target,
                value: re(value),
            },
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => Stmt::If {
                cond: re(cond),
                then_block: self.rewrite_block(then_block, expr_arena, stmt_arena, fresh),
                else_block: else_block
                    .map(|b| self.rewrite_block(b, expr_arena, stmt_arena, fresh)),
            },
            Stmt::While {
                cond,
                body,
                decreasing,
            } => Stmt::While {
                cond: re(cond),
                body: self.rewrite_block(body, expr_arena, stmt_arena, fresh),
                decreasing,
            },
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => Stmt::Repeat {
                pattern,
                iterable: re(iterable),
                body: self.rewrite_block(body, expr_arena, stmt_arena, fresh),
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
                generics,
                body: {
                    let own: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
                    let inner = Inliner {
                        bodies: self.bodies,
                        frame: capture::frame_names(&own, body),
                    };
                    inner.rewrite_block(body, expr_arena, stmt_arena, fresh)
                },
                params,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            },
            Stmt::Show { object, recipient } => Stmt::Show {
                object: re(object),
                recipient,
            },
            Stmt::Return { value } => Stmt::Return {
                value: value.map(re),
            },
            Stmt::RuntimeAssert {
                condition,
                hard,
                dim,
                reason,
            } => Stmt::RuntimeAssert {
                condition: re(condition),
                hard,
                dim,
                reason: reason.map(|r| re(r)),
            },
            Stmt::Push { value, collection } => Stmt::Push {
                value: re(value),
                collection,
            },
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => Stmt::SetIndex {
                collection,
                index: re(index),
                value: re(value),
            },
            Stmt::Call { function, args } => Stmt::Call {
                function,
                args: args.into_iter().map(re).collect(),
            },
            other => other,
        }
    }
}

/// Fresh names for the compound arguments the inliner binds, unique in the program it rewrites.
struct Fresh<'i> {
    interner: &'i mut Interner,
    next: u32,
}

impl Fresh<'_> {
    fn name(&mut self) -> Symbol {
        loop {
            let candidate = format!("__inline_arg_{}", self.next);
            self.next += 1;
            if self.interner.lookup(&candidate).is_none() {
                return self.interner.intern(&candidate);
            }
        }
    }
}

/// Inline every tiny pure helper at its atomic-argument call sites, and at a statement's whole-value call through
/// fresh bindings of its compound arguments.
pub fn inline_tiny_fns<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let defined: HashSet<Symbol> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef { name, .. } => Some(*name),
            _ => None,
        })
        .collect();
    let mut bodies: HashMap<Symbol, Tiny<'a>> = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            is_native: false,
            generics,
            ..
        } = s
        {
            if !generics.is_empty() {
                continue;
            }
            if let [Stmt::Return { value: Some(expr) }] = body {
                if body_inlinable(expr, &defined, interner) {
                    let params: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
                    let free = capture::free_names(&params, body);
                    bodies.insert(
                        *name,
                        Tiny {
                            params,
                            body: *expr,
                            free,
                        },
                    );
                }
            }
        }
    }
    if bodies.is_empty() {
        return stmts;
    }
    let inliner = Inliner {
        bodies: &bodies,
        frame: capture::main_frame_names(&stmts),
    };
    let mut fresh = Fresh { interner, next: 0 };
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    for s in stmts {
        let s = inliner.bind_compound_arguments(s, expr_arena, &mut out, &mut fresh);
        out.push(inliner.rewrite_stmt(s, expr_arena, stmt_arena, &mut fresh));
    }
    out
}
