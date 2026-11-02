//! Run-path memoization: bring the AOT memoizer's win to the interpreter/VM tier.
//!
//! The AOT backend caches a pure self-recursive function in a `thread_local!`
//! `FxHashMap` emitted into the generated Rust ([`codegen::detection::should_memoize`]);
//! the interpreter and VM have no such construct. This pass expresses the SAME
//! optimization as a source-to-source transform: a module-level `Map` cache the
//! function reads before recursing and writes before returning. The VM promotes
//! a Main top-level binding referenced from a function to a global, so the
//! function can reach the cache.
//!
//! ## Scope (sound subset)
//!
//! A function is memoized only when it is:
//!   * non-native with a declared return type,
//!   * EXACTLY ONE parameter of a hashable type (the cache key),
//!   * pure ([`super::purity::Purity`]: its whole effect is its answer, from its
//!     argument alone), in the shapes the supercompiler's evaluator runs
//!     ([`supercompile::evaluable_body`]), with SELF-calls its only calls, and
//!   * genuinely recursive (≥ 2 self-calls, so caching pays).
//!
//! `fib` is the canonical case: turning its exponential recursion into O(n).
//! A function `closed_form` already solved has no self-calls left when this pass
//! runs (after ClosedForm), so it declines — the natural preemption.

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Stmt, StringPart, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::optimization::Opt;

use super::purity::Purity;
use super::supercompile::evaluable_body;

/// Memoize every eligible run-path function. Returns the rewritten program and
/// whether anything was memoized.
pub fn memoize_run_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    // 1. Find the eligible functions and their (param, key-type, ret-type). A function the exact extension
    //    reaches computes on the extension's values, which a cache typed by its declaration cannot key or hold.
    let purity = Purity::of_program(&stmts, interner);
    let tower = crate::analysis::tower::TowerTypes::of_program(&stmts, interner);
    let mut targets: Vec<Memoizable<'a>> = Vec::new();
    for s in &stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            return_type,
            is_native,
            opt_flags,
            ..
        } = s
        {
            if !opt_flags.is_on(Opt::Memo) {
                continue;
            }
            let reached = tower.as_ref().is_some_and(|t| {
                t.returns(*name) || (0..params.len()).any(|i| t.param_at(*name, i))
            });
            if reached {
                continue;
            }
            if let Some(m) = eligible(
                *name,
                params,
                body,
                *return_type,
                *is_native,
                &purity,
                interner,
            ) {
                targets.push(m);
            }
        }
    }
    if targets.is_empty() {
        return (stmts, false);
    }

    // 2. Rewrite each eligible function's body to check/populate its cache, and
    //    build the cache `Let`s to hoist to the first executable statement.
    let cache_of: HashMap<Symbol, Memoizable<'a>> =
        targets.iter().map(|m| (m.name, m.clone())).collect();
    let mut counter = 0u32;
    let rewritten: Vec<Stmt<'a>> = stmts
        .into_iter()
        .map(|s| match s {
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
            } if cache_of.contains_key(&name) => {
                let m = &cache_of[&name];
                let new_body =
                    memoize_body(m, body, &mut counter, expr_arena, stmt_arena, interner);
                Stmt::FunctionDef {
                    name,
                    generics,
                    params,
                    body: new_body,
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                }
            }
            other => other,
        })
        .collect();

    // 3. Insert the cache initializers before the first executable statement.
    let cache_lets: Vec<Stmt<'a>> = targets
        .iter()
        .map(|m| cache_init(m, expr_arena, interner))
        .collect();
    let out = insert_before_first_executable(rewritten, cache_lets);
    (out, true)
}

#[derive(Clone)]
struct Memoizable<'a> {
    name: Symbol,
    param: Symbol,
    key_ty: &'a TypeExpr<'a>,
    ret_ty: &'a TypeExpr<'a>,
    cache: Symbol,
}

fn eligible<'a>(
    name: Symbol,
    params: &[(Symbol, &'a TypeExpr<'a>)],
    body: Block<'a>,
    return_type: Option<&'a TypeExpr<'a>>,
    is_native: bool,
    purity: &Purity,
    interner: &mut Interner,
) -> Option<Memoizable<'a>> {
    if is_native {
        return None;
    }
    let ret_ty = return_type?;
    if params.len() != 1 {
        return None;
    }
    let (param, key_ty) = params[0];
    if !is_hashable(key_ty, interner) {
        return None;
    }
    if !purity.is_pure(name) || !evaluable_body(body) {
        return None;
    }
    let (self_calls, other_calls) = count_calls(name, body);
    if self_calls < 2 || other_calls > 0 {
        return None;
    }
    let cache = interner.intern(&format!("__memo_{}", interner.resolve(name)));
    Some(Memoizable {
        name,
        param,
        key_ty,
        ret_ty,
        cache,
    })
}

/// `Map of KEY to RET` initializer: `Let mutable __memo_f be a new Map of K to R`.
fn cache_init<'a>(m: &Memoizable<'a>, ea: &'a Arena<Expr<'a>>, it: &mut Interner) -> Stmt<'a> {
    let map_sym = it.intern("Map");
    let value = ea.alloc(Expr::New {
        type_name: map_sym,
        type_args: vec![m.key_ty.clone(), m.ret_ty.clone()],
        init_fields: Vec::new(),
    });
    Stmt::Let {
        var: m.cache,
        ty: None,
        value,
        mutable: true,
    }
}

/// Prepend the cache lookup and rewrite every `Return e` to populate the cache.
fn memoize_body<'a>(
    m: &Memoizable<'a>,
    body: Block<'a>,
    counter: &mut u32,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
) -> Block<'a> {
    let key = ea.alloc(Expr::Identifier(m.param));
    let contains = ea.alloc(Expr::Contains {
        collection: ea.alloc(Expr::Identifier(m.cache)),
        value: key,
    });
    let get = ea.alloc(Expr::Index {
        collection: ea.alloc(Expr::Identifier(m.cache)),
        index: key,
    });
    let hit = Stmt::If {
        cond: contains,
        then_block: sa.alloc_slice(vec![Stmt::Return { value: Some(get) }]),
        else_block: None,
    };
    let mut out = vec![hit];
    out.extend(rewrite_returns(m, body, counter, ea, sa, it));
    sa.alloc_slice(out)
}

/// Rewrite `Return e` → `Let __r be e. Set __memo at key to __r. Return __r.`
/// throughout the body (recursing into nested control flow).
fn rewrite_returns<'a>(
    m: &Memoizable<'a>,
    block: Block<'a>,
    counter: &mut u32,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut out = Vec::with_capacity(block.len());
    for s in block {
        match s {
            Stmt::Return { value: Some(e) } => {
                // A memoized self-call reads and writes the function's cache. C does not fix the
                // order of sibling operands, so make every eagerly-evaluated recursive call in a
                // return value an explicit statement, from left to right. Besides avoiding C's
                // unsequenced-side-effect refusal, this gives bytecode the same deterministic
                // residual. Short-circuit RHSs and delayed closures stay in place.
                let mut calls = Vec::new();
                let e = sequence_self_calls(m.name, e, counter, &mut calls, ea, it);
                out.extend(calls);
                let r = it.intern(&format!("__memo_r_{counter}"));
                *counter += 1;
                out.push(Stmt::Let {
                    var: r,
                    ty: None,
                    value: e,
                    mutable: false,
                });
                out.push(Stmt::SetIndex {
                    collection: ea.alloc(Expr::Identifier(m.cache)),
                    index: ea.alloc(Expr::Identifier(m.param)),
                    value: ea.alloc(Expr::Identifier(r)),
                });
                out.push(Stmt::Return {
                    value: Some(ea.alloc(Expr::Identifier(r))),
                });
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => out.push(Stmt::If {
                cond,
                then_block: sa.alloc_slice(rewrite_returns(m, then_block, counter, ea, sa, it)),
                else_block: else_block
                    .map(|b| sa.alloc_slice(rewrite_returns(m, b, counter, ea, sa, it))),
            }),
            Stmt::While {
                cond,
                body,
                decreasing,
            } => out.push(Stmt::While {
                cond: *cond,
                body: sa.alloc_slice(rewrite_returns(m, body, counter, ea, sa, it)),
                decreasing: *decreasing,
            }),
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => out.push(Stmt::Repeat {
                pattern: pattern.clone(),
                iterable,
                body: sa.alloc_slice(rewrite_returns(m, body, counter, ea, sa, it)),
            }),
            other => out.push(other.clone()),
        }
    }
    out
}

/// Hoist eagerly evaluated calls to `function` from an expression into `prelude`.
///
/// The recursion is deliberately exhaustive for static-backend expression forms. A closure is a
/// delayed computation and a short-circuit RHS is conditional, so neither may be hoisted into the
/// surrounding statement. Browser expressions have already been refused before this pass.
fn sequence_self_calls<'a>(
    function: Symbol,
    e: &'a Expr<'a>,
    counter: &mut u32,
    prelude: &mut Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    it: &mut Interner,
) -> &'a Expr<'a> {
    let mut map = |x| sequence_self_calls(function, x, counter, prelude, ea, it);
    let rebuilt = match e {
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::Escape { .. }
        | Expr::OptionNone
        | Expr::Closure { .. }
        | Expr::View(_)
        | Expr::Dom(_) => return e,
        Expr::Call {
            function: called,
            args,
        } => {
            let args = args.iter().map(|a| map(a)).collect();
            let call = ea.alloc(Expr::Call {
                function: *called,
                args,
            });
            if *called != function {
                return call;
            }
            let tmp = it.intern(&format!("__memo_call_{counter}"));
            *counter += 1;
            prelude.push(Stmt::Let {
                var: tmp,
                ty: None,
                value: call,
                mutable: false,
            });
            return ea.alloc(Expr::Identifier(tmp));
        }
        Expr::BinaryOp { op, left, right } => {
            let left = map(left);
            let right = if matches!(op, BinaryOpKind::And | BinaryOpKind::Or) {
                *right
            } else {
                map(right)
            };
            Expr::BinaryOp {
                op: *op,
                left,
                right,
            }
        }
        Expr::Not { operand } => Expr::Not {
            operand: map(operand),
        },
        Expr::Index { collection, index } => Expr::Index {
            collection: map(collection),
            index: map(index),
        },
        Expr::Slice {
            collection,
            start,
            end,
        } => Expr::Slice {
            collection: map(collection),
            start: map(start),
            end: map(end),
        },
        Expr::Copy { expr } => Expr::Copy { expr: map(expr) },
        Expr::Give { value } => Expr::Give { value: map(value) },
        Expr::Length { collection } => Expr::Length {
            collection: map(collection),
        },
        Expr::Contains { collection, value } => Expr::Contains {
            collection: map(collection),
            value: map(value),
        },
        Expr::Union { left, right } => Expr::Union {
            left: map(left),
            right: map(right),
        },
        Expr::Intersection { left, right } => Expr::Intersection {
            left: map(left),
            right: map(right),
        },
        Expr::ManifestOf { zone } => Expr::ManifestOf { zone: map(zone) },
        Expr::ChunkAt { index, zone } => Expr::ChunkAt {
            index: map(index),
            zone: map(zone),
        },
        Expr::List(items) => Expr::List(items.iter().map(|x| map(x)).collect()),
        Expr::Tuple(items) => Expr::Tuple(items.iter().map(|x| map(x)).collect()),
        Expr::Range { start, end } => Expr::Range {
            start: map(start),
            end: map(end),
        },
        Expr::FieldAccess { object, field } => Expr::FieldAccess {
            object: map(object),
            field: *field,
        },
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => Expr::New {
            type_name: *type_name,
            type_args: type_args.clone(),
            init_fields: init_fields
                .iter()
                .map(|(name, value)| (*name, map(value)))
                .collect(),
        },
        Expr::NewVariant {
            enum_name,
            variant,
            fields,
        } => Expr::NewVariant {
            enum_name: *enum_name,
            variant: *variant,
            fields: fields
                .iter()
                .map(|(name, value)| (*name, map(value)))
                .collect(),
        },
        Expr::OptionSome { value } => Expr::OptionSome { value: map(value) },
        Expr::WithCapacity { value, capacity } => Expr::WithCapacity {
            value: map(value),
            capacity: map(capacity),
        },
        Expr::CallExpr { callee, args } => Expr::CallExpr {
            callee: map(callee),
            args: args.iter().map(|x| map(x)).collect(),
        },
        Expr::InterpolatedString(parts) => Expr::InterpolatedString(
            parts
                .iter()
                .map(|part| match part {
                    StringPart::Literal(s) => StringPart::Literal(*s),
                    StringPart::Expr {
                        value,
                        format_spec,
                        debug,
                    } => StringPart::Expr {
                        value: map(value),
                        format_spec: *format_spec,
                        debug: *debug,
                    },
                })
                .collect(),
        ),
    };
    ea.alloc(rebuilt)
}

/// Insert `lets` immediately before the first statement that is not a top-level
/// DEFINITION (function/struct/test/theorem/…), so the caches initialize at the
/// start of the executable Main flow — before any memoized function is called.
fn insert_before_first_executable<'a>(stmts: Vec<Stmt<'a>>, lets: Vec<Stmt<'a>>) -> Vec<Stmt<'a>> {
    let pos = stmts
        .iter()
        .position(|s| !is_definition(s))
        .unwrap_or(stmts.len());
    let mut out = Vec::with_capacity(stmts.len() + lets.len());
    out.extend(stmts[..pos].iter().cloned());
    out.extend(lets);
    out.extend(stmts[pos..].iter().cloned());
    out
}

fn is_definition(s: &Stmt) -> bool {
    matches!(
        s,
        Stmt::FunctionDef { .. }
            | Stmt::StructDef { .. }
            | Stmt::TestDef { .. }
            | Stmt::Theorem(_)
            | Stmt::Definition(_)
            | Stmt::Axiom(_)
            | Stmt::Theory(_)
            | Stmt::Require { .. }
    )
}

fn is_hashable(ty: &TypeExpr, interner: &Interner) -> bool {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(
                name,
                "Int"
                    | "Nat"
                    | "Bool"
                    | "Char"
                    | "Byte"
                    | "Text"
                    | "i64"
                    | "u64"
                    | "bool"
                    | "char"
                    | "u8"
                    | "String"
            )
        }
        TypeExpr::Refinement { base, .. } => is_hashable(base, interner),
        _ => false,
    }
}

/// `(self_calls, other_calls)` in a body — counts `Call`/`CallExpr` to `name`
/// vs to any other function (in both statement and expression position).
fn count_calls(name: Symbol, body: &[Stmt]) -> (usize, usize) {
    let mut acc = (0usize, 0usize);
    for s in body {
        count_calls_stmt(name, s, &mut acc);
    }
    acc
}

fn count_calls_stmt(name: Symbol, s: &Stmt, acc: &mut (usize, usize)) {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => count_calls_expr(name, value, acc),
        Stmt::Return { value: Some(e) } => count_calls_expr(name, e, acc),
        Stmt::Call { function, args } => {
            if *function == name {
                acc.0 += 1;
            } else {
                acc.1 += 1;
            }
            for a in args {
                count_calls_expr(name, a, acc);
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            count_calls_expr(name, cond, acc);
            for s in *then_block {
                count_calls_stmt(name, s, acc);
            }
            if let Some(eb) = else_block {
                for s in *eb {
                    count_calls_stmt(name, s, acc);
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            count_calls_expr(name, cond, acc);
            for s in *body {
                count_calls_stmt(name, s, acc);
            }
        }
        Stmt::Repeat { iterable, body, .. } => {
            count_calls_expr(name, iterable, acc);
            for s in *body {
                count_calls_stmt(name, s, acc);
            }
        }
        Stmt::Show { object, .. } => count_calls_expr(name, object, acc),
        _ => {}
    }
}

fn count_calls_expr(name: Symbol, e: &Expr, acc: &mut (usize, usize)) {
    match e {
        Expr::Call { function, args } => {
            if *function == name {
                acc.0 += 1;
            } else {
                acc.1 += 1;
            }
            for a in args {
                count_calls_expr(name, a, acc);
            }
        }
        Expr::CallExpr { callee, args } => {
            // A dynamic call target is not a named self-call — count as "other".
            acc.1 += 1;
            count_calls_expr(name, callee, acc);
            for a in args {
                count_calls_expr(name, a, acc);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            count_calls_expr(name, left, acc);
            count_calls_expr(name, right, acc);
        }
        Expr::Not { operand } => count_calls_expr(name, operand, acc),
        Expr::Index { collection, index } => {
            count_calls_expr(name, collection, acc);
            count_calls_expr(name, index, acc);
        }
        Expr::Length { collection } => count_calls_expr(name, collection, acc),
        Expr::FieldAccess { object, .. } => count_calls_expr(name, object, acc),
        _ => {}
    }
}
