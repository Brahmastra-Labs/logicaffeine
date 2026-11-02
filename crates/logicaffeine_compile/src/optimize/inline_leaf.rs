//! Statement-body leaf-function inlining for the RUN PATH.
//!
//! [`inline_tiny`](super::inline_tiny) folds helpers whose body is exactly
//! `Return <expr>`. The gcd shape — an iterative helper (`Let`s, a `While`,
//! a trailing `Return`) called per inner-loop element — is one statement
//! richer, so it pays the native call boundary on every iteration (gcd is
//! ~720k calls of a ~6-statement leaf). Folding the body in turns the calling
//! loop into one call-free region the JIT compiles whole, erasing the
//! per-call frame round-trip.
//!
//! Fail-closed candidate rules (every one is load-bearing for soundness):
//! - body is `[prefix…] ++ [Return <ret>]` — a single trailing return, no
//!   early returns anywhere (an early `Return` inside an `If`/`While` would
//!   need control flow we do not synthesise);
//! - every prefix statement is `Let`/`Set`/`If`/`While` only — no `Push`,
//!   `SetIndex`, `Show`, `Call`, … so the body is pure: no effects, no
//!   recursion, no collection mutation that could alias an argument;
//! - every expression in the body (and `ret`) is built only from literals,
//!   identifiers, binary/unary operators, `Index` and `Length` — the same
//!   pure, duplication-safe fragment, with no nested calls;
//! - a call site inlines only in an UNCONDITIONAL position: never out of the
//!   right operand of a short-circuiting `and`/`or` (hoisting it would make a
//!   guarded sub-expression always evaluate), and never out of a `While`
//!   condition (re-evaluated per iteration).
//!
//! Every eligible leaf inlines — loopy (the gcd shape) and loop-free alike — so
//! a helper on a hot path never pays the call boundary. `MAX_BODY_STMTS` bounds
//! the duplicated body so many call sites cannot bloat the code.
//!
//! Parameters and locals are alpha-renamed per call instance, so inlining a
//! helper twice — or into a caller that reuses its names — never captures;
//! and a helper inlines only where none of its FREE names (the program's
//! bindings it reads) is bound in the call site's frame ([`super::capture`]),
//! and never a helper that SETS one of its free names — its effect leaves
//! its frame, and a prelude would run it before the statement's earlier reads.
//! Like every inliner (the AOT supercompiler included), this erases the
//! call-depth accounting a bytecode call would perform — the kernel's
//! resource limit, not a semantic boundary.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Stmt};
use crate::intern::{Interner, Symbol};

use super::capture;

/// Helpers larger than this stay as calls — inlining a big body at many sites
/// is code bloat with no region-tiering payoff.
const MAX_BODY_STMTS: usize = 32;

/// A confirmed inline candidate: its parameters, the statements before the
/// trailing return, and the returned expression.
struct Candidate<'a> {
    params: Vec<Symbol>,
    prefix: Block<'a>,
    ret: &'a Expr<'a>,
    /// The names the helper resolves where it is declared (see [`super::capture`]).
    free: HashSet<Symbol>,
}

/// Where a call is rewritten: the helpers that may inline, and the names bound in the frame around it,
/// which a helper's free name must not be.
struct Site<'c, 'a> {
    cands: &'c HashMap<Symbol, Candidate<'a>>,
    frame: &'c HashSet<Symbol>,
}

/// The pure, duplication-safe expression fragment (no calls, no effects).
fn expr_pure(e: &Expr) -> bool {
    match e {
        Expr::Literal(_) | Expr::Identifier(_) => true,
        Expr::BinaryOp { left, right, .. } => expr_pure(left) && expr_pure(right),
        Expr::Not { operand } => expr_pure(operand),
        Expr::Index { collection, index } => expr_pure(collection) && expr_pure(index),
        Expr::Length { collection } => expr_pure(collection),
        _ => false,
    }
}

/// A prefix statement that is safe to duplicate and alpha-rename: pure control
/// flow over pure expressions, with no `Return` anywhere inside.
fn stmt_pure_nonreturn(s: &Stmt) -> bool {
    match s {
        Stmt::Let { value, .. } => expr_pure(value),
        Stmt::Set { value, .. } => expr_pure(value),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_pure(cond)
                && then_block.iter().all(stmt_pure_nonreturn)
                && else_block.map_or(true, |b| b.iter().all(stmt_pure_nonreturn))
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            expr_pure(cond)
                && decreasing.map_or(true, |d| expr_pure(d))
                && body.iter().all(stmt_pure_nonreturn)
        }
        _ => false,
    }
}

/// Whether `block` sets any of `names` (a `Set` at any depth; the prefix admits no other write).
fn sets_any(block: &[Stmt], names: &HashSet<Symbol>) -> bool {
    let mut sets = false;
    logicaffeine_language::ast_depth::for_each_node(block, |node| {
        if let logicaffeine_language::ast_depth::Visited::Stmt(Stmt::Set { target, .. }) = node {
            sets |= names.contains(target);
        }
    });
    sets
}

/// Recognise `[prefix…] ++ [Return Some(ret)]` with an all-pure prefix.
fn as_candidate<'a>(body: Block<'a>) -> Option<(Block<'a>, &'a Expr<'a>)> {
    if body.is_empty() || body.len() > MAX_BODY_STMTS {
        return None;
    }
    let (last, prefix) = body.split_last().unwrap();
    let ret = match last {
        Stmt::Return { value: Some(e) } => *e,
        _ => return None,
    };
    if !expr_pure(ret) || !prefix.iter().all(stmt_pure_nonreturn) {
        return None;
    }
    Some((prefix, ret))
}

/// No bound name (param or `Let` local) may appear twice — alpha-renaming maps
/// each `Symbol` to one fresh name, so a shadowed name would collapse two
/// distinct bindings into one. Reject such bodies rather than miscompile them.
fn bound_names_unique(params: &[Symbol], prefix: Block) -> bool {
    fn walk(stmts: &[Stmt], seen: &mut HashSet<Symbol>) -> bool {
        for s in stmts {
            match s {
                Stmt::Let { var, .. } => {
                    if !seen.insert(*var) {
                        return false;
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    if !walk(then_block, seen) {
                        return false;
                    }
                    if let Some(b) = else_block {
                        if !walk(b, seen) {
                            return false;
                        }
                    }
                }
                Stmt::While { body, .. } => {
                    if !walk(body, seen) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }
    let mut seen = HashSet::new();
    for p in params {
        if !seen.insert(*p) {
            return false;
        }
    }
    walk(prefix, &mut seen)
}

/// Every `Let`-bound local declared anywhere in a pure prefix.
fn collect_locals(stmts: &[Stmt], out: &mut HashSet<Symbol>) {
    for s in stmts {
        match s {
            Stmt::Let { var, .. } => {
                out.insert(*var);
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_locals(then_block, out);
                if let Some(b) = else_block {
                    collect_locals(b, out);
                }
            }
            Stmt::While { body, .. } => collect_locals(body, out),
            _ => {}
        }
    }
}

fn ren(sym: Symbol, map: &HashMap<Symbol, Symbol>) -> Symbol {
    map.get(&sym).copied().unwrap_or(sym)
}

/// Alpha-rename params/locals in a body expression (no calls occur here — the
/// candidate gate forbade them).
fn rename_expr<'a>(
    e: &'a Expr<'a>,
    map: &HashMap<Symbol, Symbol>,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    match e {
        Expr::Identifier(s) => match map.get(s) {
            Some(r) => ea.alloc(Expr::Identifier(*r)),
            None => e,
        },
        Expr::Literal(_) => e,
        Expr::BinaryOp { op, left, right } => ea.alloc(Expr::BinaryOp {
            op: *op,
            left: rename_expr(left, map, ea),
            right: rename_expr(right, map, ea),
        }),
        Expr::Not { operand } => ea.alloc(Expr::Not {
            operand: rename_expr(operand, map, ea),
        }),
        Expr::Index { collection, index } => ea.alloc(Expr::Index {
            collection: rename_expr(collection, map, ea),
            index: rename_expr(index, map, ea),
        }),
        Expr::Length { collection } => ea.alloc(Expr::Length {
            collection: rename_expr(collection, map, ea),
        }),
        _ => e,
    }
}

fn rename_block<'a>(
    block: Block<'a>,
    map: &HashMap<Symbol, Symbol>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Block<'a> {
    let out: Vec<Stmt<'a>> = block.iter().map(|s| rename_stmt(s, map, ea, sa)).collect();
    sa.alloc_slice(out)
}

fn rename_stmt<'a>(
    s: &Stmt<'a>,
    map: &HashMap<Symbol, Symbol>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    match s {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => Stmt::Let {
            var: ren(*var, map),
            ty: *ty,
            value: rename_expr(value, map, ea),
            mutable: *mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: ren(*target, map),
            value: rename_expr(value, map, ea),
        },
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond: rename_expr(cond, map, ea),
            then_block: rename_block(then_block, map, ea, sa),
            else_block: else_block.map(|b| rename_block(b, map, ea, sa)),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond: rename_expr(cond, map, ea),
            body: rename_block(body, map, ea, sa),
            decreasing: decreasing.map(|d| rename_expr(d, map, ea)),
        },
        // The candidate gate admitted only the four arms above.
        other => other.clone(),
    }
}

/// Expand one candidate call: emit the param bindings + renamed body, and
/// return the fresh identifier holding the result.
fn expand<'a>(
    cand: &Candidate<'a>,
    args: &[&'a Expr<'a>],
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    counter: &mut usize,
    prelude: &mut Vec<Stmt<'a>>,
) -> &'a Expr<'a> {
    let id = *counter;
    *counter += 1;

    let mut locals = HashSet::new();
    collect_locals(cand.prefix, &mut locals);

    let mut map: HashMap<Symbol, Symbol> = HashMap::new();
    for p in &cand.params {
        let name = format!("__il{id}_{}", interner.resolve(*p));
        map.insert(*p, interner.intern(&name));
    }
    for l in &locals {
        let name = format!("__il{id}_{}", interner.resolve(*l));
        map.insert(*l, interner.intern(&name));
    }

    for (p, arg) in cand.params.iter().zip(args.iter()) {
        prelude.push(Stmt::Let {
            var: ren(*p, &map),
            ty: None,
            value: arg,
            mutable: true,
        });
    }
    for s in cand.prefix {
        prelude.push(rename_stmt(s, &map, ea, sa));
    }
    let result = interner.intern(&format!("__il{id}_result"));
    prelude.push(Stmt::Let {
        var: result,
        ty: None,
        value: rename_expr(cand.ret, &map, ea),
        mutable: false,
    });
    ea.alloc(Expr::Identifier(result))
}

/// Rewrite an expression, inlining candidate calls in unconditional positions
/// and accumulating their bodies into `prelude` (in left-to-right order).
fn rewrite_expr<'a>(
    e: &'a Expr<'a>,
    site: &Site<'_, 'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    counter: &mut usize,
    prelude: &mut Vec<Stmt<'a>>,
) -> &'a Expr<'a> {
    match e {
        Expr::Call { function, args } => {
            let new_args: Vec<&'a Expr<'a>> = args
                .iter()
                .map(|a| rewrite_expr(a, site, ea, sa, interner, counter, prelude))
                .collect();
            if let Some(c) = site.cands.get(function) {
                if c.params.len() == new_args.len() && !capture::captures(&c.free, site.frame) {
                    return expand(c, &new_args, ea, sa, interner, counter, prelude);
                }
            }
            ea.alloc(Expr::Call {
                function: *function,
                args: new_args,
            })
        }
        Expr::BinaryOp { op, left, right } => {
            let l = rewrite_expr(left, site, ea, sa, interner, counter, prelude);
            // The right operand of `and`/`or` is conditionally evaluated;
            // hoisting a call out of it would always run it. Leave it intact.
            let r = if matches!(op, BinaryOpKind::And | BinaryOpKind::Or) {
                *right
            } else {
                rewrite_expr(right, site, ea, sa, interner, counter, prelude)
            };
            ea.alloc(Expr::BinaryOp {
                op: *op,
                left: l,
                right: r,
            })
        }
        Expr::Not { operand } => {
            let o = rewrite_expr(operand, site, ea, sa, interner, counter, prelude);
            ea.alloc(Expr::Not { operand: o })
        }
        Expr::Index { collection, index } => {
            let c = rewrite_expr(collection, site, ea, sa, interner, counter, prelude);
            let i = rewrite_expr(index, site, ea, sa, interner, counter, prelude);
            ea.alloc(Expr::Index {
                collection: c,
                index: i,
            })
        }
        Expr::Length { collection } => {
            let c = rewrite_expr(collection, site, ea, sa, interner, counter, prelude);
            ea.alloc(Expr::Length { collection: c })
        }
        _ => e,
    }
}

/// Rewrite one statement, returning its prelude-prepended replacement(s).
fn rewrite_stmt<'a>(
    s: &Stmt<'a>,
    site: &Site<'_, 'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    counter: &mut usize,
    out: &mut Vec<Stmt<'a>>,
) {
    // Each arm lifts candidate calls from the statement's own expressions into
    // a prelude evaluated immediately before it — sound because the calls are
    // pure and these positions are evaluated exactly once when control arrives.
    match s {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => {
            let mut prelude = Vec::new();
            let v = rewrite_expr(value, site, ea, sa, interner, counter, &mut prelude);
            out.extend(prelude);
            out.push(Stmt::Let {
                var: *var,
                ty: *ty,
                value: v,
                mutable: *mutable,
            });
        }
        Stmt::Set { target, value } => {
            let mut prelude = Vec::new();
            let v = rewrite_expr(value, site, ea, sa, interner, counter, &mut prelude);
            out.extend(prelude);
            out.push(Stmt::Set {
                target: *target,
                value: v,
            });
        }
        Stmt::Return { value } => {
            let mut prelude = Vec::new();
            let v = value.map(|e| rewrite_expr(e, site, ea, sa, interner, counter, &mut prelude));
            out.extend(prelude);
            out.push(Stmt::Return { value: v });
        }
        Stmt::Show { object, recipient } => {
            let mut prelude = Vec::new();
            let o = rewrite_expr(object, site, ea, sa, interner, counter, &mut prelude);
            out.extend(prelude);
            out.push(Stmt::Show {
                object: o,
                recipient: *recipient,
            });
        }
        Stmt::Push { value, collection } => {
            let mut prelude = Vec::new();
            let v = rewrite_expr(value, site, ea, sa, interner, counter, &mut prelude);
            out.extend(prelude);
            out.push(Stmt::Push {
                value: v,
                collection: *collection,
            });
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            let mut prelude = Vec::new();
            let i = rewrite_expr(index, site, ea, sa, interner, counter, &mut prelude);
            let v = rewrite_expr(value, site, ea, sa, interner, counter, &mut prelude);
            out.extend(prelude);
            out.push(Stmt::SetIndex {
                collection: *collection,
                index: i,
                value: v,
            });
        }
        Stmt::RuntimeAssert {
            condition,
            hard,
            dim,
            reason,
        } => {
            let mut prelude = Vec::new();
            let c = rewrite_expr(condition, site, ea, sa, interner, counter, &mut prelude);
            out.extend(prelude);
            out.push(Stmt::RuntimeAssert {
                condition: c,
                hard: *hard,
                dim: *dim,
                reason: *reason,
            });
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            // The condition is evaluated once on arrival → safe to lift.
            let mut prelude = Vec::new();
            let c = rewrite_expr(cond, site, ea, sa, interner, counter, &mut prelude);
            out.extend(prelude);
            out.push(Stmt::If {
                cond: c,
                then_block: rewrite_block(then_block, site, ea, sa, interner, counter),
                else_block: else_block.map(|b| rewrite_block(b, site, ea, sa, interner, counter)),
            });
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            // The condition re-evaluates each iteration → do NOT lift from it.
            out.push(Stmt::While {
                cond: *cond,
                body: rewrite_block(body, site, ea, sa, interner, counter),
                decreasing: *decreasing,
            });
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            // The iterable is evaluated once → safe to lift.
            let mut prelude = Vec::new();
            let it = rewrite_expr(iterable, site, ea, sa, interner, counter, &mut prelude);
            out.extend(prelude);
            out.push(Stmt::Repeat {
                pattern: pattern.clone(),
                iterable: it,
                body: rewrite_block(body, site, ea, sa, interner, counter),
            });
        }
        Stmt::FunctionDef {
            name,
            generics,
            params,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        } => {
            let own: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
            let frame = capture::frame_names(&own, body);
            let inner = Site {
                cands: site.cands,
                frame: &frame,
            };
            out.push(Stmt::FunctionDef {
                name: *name,
                generics: generics.clone(),
                params: params.clone(),
                body: rewrite_block(body, &inner, ea, sa, interner, counter),
                return_type: *return_type,
                is_native: *is_native,
                native_path: *native_path,
                is_exported: *is_exported,
                export_target: *export_target,
                opt_flags: opt_flags.clone(),
            });
        }
        other => out.push(other.clone()),
    }
}

fn rewrite_block<'a>(
    block: Block<'a>,
    site: &Site<'_, 'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    counter: &mut usize,
) -> Block<'a> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(block.len());
    for s in block {
        rewrite_stmt(s, site, ea, sa, interner, counter, &mut out);
    }
    sa.alloc_slice(out)
}

/// Inline every pure statement-body leaf helper at its call sites.
pub fn inline_leaf_fns<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut cands: HashMap<Symbol, Candidate<'a>> = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            is_native: false,
            generics,
            is_exported: false,
            ..
        } = s
        {
            // This inliner builds a prelude of independent parameter values.
            // A mutable parameter lends a caller's place, so its writes cannot
            // be represented by that value-only prelude.
            if !generics.is_empty()
                || params
                    .iter()
                    .any(|(_, ty)| matches!(ty, crate::ast::stmt::TypeExpr::Mutable { .. }))
            {
                continue;
            }
            if let Some((prefix, ret)) = as_candidate(body) {
                let param_syms: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
                if !bound_names_unique(&param_syms, prefix) {
                    continue;
                }
                let free = capture::free_names(&param_syms, body);
                // A helper that sets one of its free names — a program binding — has an effect outside its
                // frame: hoisted into a prelude, it would run before the expression's earlier operands read
                // that binding. It stays a call.
                if sets_any(prefix, &free) {
                    continue;
                }
                cands.insert(
                    *name,
                    Candidate {
                        params: param_syms,
                        prefix,
                        ret,
                        free,
                    },
                );
            }
        }
    }
    if cands.is_empty() {
        return stmts;
    }
    let mut counter: usize = 0;
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    let frame = capture::main_frame_names(&stmts);
    let main = Site {
        cands: &cands,
        frame: &frame,
    };
    for s in &stmts {
        rewrite_stmt(
            s,
            &main,
            expr_arena,
            stmt_arena,
            interner,
            &mut counter,
            &mut out,
        );
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/inline_leaf/tests.rs"]
mod tests;
