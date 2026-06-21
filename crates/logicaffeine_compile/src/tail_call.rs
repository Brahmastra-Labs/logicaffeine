//! Tail-call recognition shared by the three execution tiers.
//!
//! "A self-tail-call runs in constant stack" is a LANGUAGE semantic, so the
//! tree-walker interpreter, the bytecode VM, and the AOT codegen must agree on
//! exactly which call shapes qualify. Keeping the detectors here — one
//! definition consumed by all three — is what stops them drifting (a pattern
//! one tier loops while another recurses would diverge only on recursion deep
//! enough to hit the call-depth limit, the worst kind of silent gap).
//!
//! Two shapes qualify as a self-tail-call of `func` with `pc` parameters:
//!
//!  * **direct** — `Return self(args)`. The call's value is the function's
//!    value, so it is in tail position. A nested self-call inside an argument
//!    (`Return self(self(x)))`) stays ordinary recursion: only the OUTER call
//!    becomes the loop-back, and the args are evaluated first at every tier.
//!  * **pair** — `Set/Let x to self(args)` immediately followed by `Return x`.
//!    The binding flows straight into the return with nothing in between, so it
//!    is tail position too (the quicksort second-recursion shape).
//!
//! Both return the argument expressions UN-evaluated; each tier evaluates them
//! in its own way (the tree-walker into `RuntimeValue`s, the VM into registers,
//! the AOT into Rust temporaries) before reassigning the parameters and looping.

use std::collections::HashSet;

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, OptFlag, Stmt};
use crate::intern::{Interner, Symbol};
use crate::Arena;

/// The argument expressions of a direct self-tail-call `Return self(args)`:
/// `Some` when `expr` calls `func` with exactly `pc` arguments, else `None`.
pub(crate) fn direct_self_tail_args<'a>(
    expr: &'a Expr<'a>,
    func: Symbol,
    pc: usize,
) -> Option<&'a [&'a Expr<'a>]> {
    let Expr::Call { function, args } = expr else {
        return None;
    };
    if *function != func || args.len() != pc {
        return None;
    }
    Some(args.as_slice())
}

/// The argument expressions of the `Set/Let x to self(args); Return x` pair:
/// `Some` when `s1` binds `x` to a call of `func` with `pc` arguments and `s2`
/// is `Return x` for that same `x`, else `None`.
pub(crate) fn tail_pair_args<'a>(
    s1: &Stmt<'a>,
    s2: &Stmt<'a>,
    func: Symbol,
    pc: usize,
) -> Option<&'a [&'a Expr<'a>]> {
    let (bound, call) = match s1 {
        Stmt::Set { target, value } => (*target, *value),
        Stmt::Let { var, value, .. } => (*var, *value),
        _ => return None,
    };
    let Expr::Call { function, args } = call else {
        return None;
    };
    if *function != func || args.len() != pc {
        return None;
    }
    let Stmt::Return { value: Some(rv) } = s2 else {
        return None;
    };
    let Expr::Identifier(rvar) = rv else {
        return None;
    };
    if *rvar != bound {
        return None;
    }
    Some(args.as_slice())
}

// =============================================================================
// Accumulator introduction
// =============================================================================
//
// `Return n + f(n-1)` is NOT a tail call (the `+` runs after the call returns),
// but single-linear recursion folding into an associative+commutative operator
// strength-reduces to a constant-stack loop carrying an accumulator. The AOT
// already does this at codegen; this module rewrites the SAME pattern at the AST
// level (to a `while`-loop) so the bytecode VM and tree-walker — which already
// run `while`-loops natively/JIT-compiled — get it too, with no new bytecode and
// no interpreter changes. Detection is the single source of truth shared with
// the AOT (`codegen/tce.rs` imports it from here).

/// Which operand of the folding binary op is the NON-recursive one.
#[derive(Debug, Clone, Copy)]
pub(crate) enum NonRecSide {
    Left,
    Right,
}

/// A recognized accumulator shape: the folding op, its identity element, and
/// which side carries the non-recursive contribution.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AccumulatorInfo {
    pub(crate) op: BinaryOpKind,
    pub(crate) identity: i64,
    pub(crate) non_recursive_side: NonRecSide,
}

fn expr_is_self_call(func: Symbol, e: &Expr) -> bool {
    matches!(e, Expr::Call { function, .. } if *function == func)
}

/// Does `e` contain ANY call to `func` (at any depth)?
fn expr_contains_self_call(func: Symbol, e: &Expr) -> bool {
    match e {
        Expr::Call { function, args } => {
            *function == func || args.iter().any(|a| expr_contains_self_call(func, a))
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_contains_self_call(func, left) || expr_contains_self_call(func, right)
        }
        Expr::Index { collection, index } => {
            expr_contains_self_call(func, collection) || expr_contains_self_call(func, index)
        }
        Expr::FieldAccess { object, .. } => expr_contains_self_call(func, object),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|i| expr_contains_self_call(func, i))
        }
        Expr::Not { operand } => expr_contains_self_call(func, operand),
        Expr::Length { collection } => expr_contains_self_call(func, collection),
        _ => false,
    }
}

/// A self-call anywhere OTHER than a `Return` expression disqualifies the
/// accumulator transform (the fold would not capture it).
fn has_non_return_self_calls(func: Symbol, body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Return { .. } => false,
        Stmt::If { cond, then_block, else_block } => {
            expr_contains_self_call(func, cond)
                || has_non_return_self_calls(func, then_block)
                || else_block.is_some_and(|e| has_non_return_self_calls(func, e))
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_contains_self_call(func, value),
        Stmt::Show { object, .. } => expr_contains_self_call(func, object),
        Stmt::While { cond, body, .. } => {
            expr_contains_self_call(func, cond) || has_non_return_self_calls(func, body)
        }
        Stmt::Repeat { body, .. } => has_non_return_self_calls(func, body),
        Stmt::Call { function, args } => {
            *function == func || args.iter().any(|a| expr_contains_self_call(func, a))
        }
        _ => false,
    })
}

/// `(base_returns, recursive_returns)` — counts `Return` statements (recursing
/// into `If` branches) split by whether they contain a self-call.
fn count_recursive_returns(func: Symbol, body: &[Stmt]) -> (usize, usize) {
    let mut base = 0;
    let mut recursive = 0;
    for s in body {
        match s {
            Stmt::Return { value: Some(e) } => {
                if expr_contains_self_call(func, e) {
                    recursive += 1;
                } else {
                    base += 1;
                }
            }
            Stmt::Return { value: None } => base += 1,
            Stmt::If { then_block, else_block, .. } => {
                let (tb, tr) = count_recursive_returns(func, then_block);
                base += tb;
                recursive += tr;
                if let Some(e) = else_block {
                    let (eb, er) = count_recursive_returns(func, e);
                    base += eb;
                    recursive += er;
                }
            }
            _ => {}
        }
    }
    (base, recursive)
}

/// Find the folding op/side of the single recursive return.
fn find_accumulator_return(func: Symbol, body: &[Stmt]) -> Option<AccumulatorInfo> {
    for s in body {
        match s {
            Stmt::Return { value: Some(Expr::BinaryOp { op, left, right }) } => {
                if !matches!(op, BinaryOpKind::Add | BinaryOpKind::Multiply) {
                    continue;
                }
                let identity = if matches!(op, BinaryOpKind::Add) { 0 } else { 1 };
                let left_call = expr_is_self_call(func, left);
                let right_call = expr_is_self_call(func, right);
                if left_call && !expr_contains_self_call(func, right) {
                    return Some(AccumulatorInfo { op: *op, identity, non_recursive_side: NonRecSide::Right });
                }
                if right_call && !expr_contains_self_call(func, left) {
                    return Some(AccumulatorInfo { op: *op, identity, non_recursive_side: NonRecSide::Left });
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if let Some(info) = find_accumulator_return(func, then_block) {
                    return Some(info);
                }
                if let Some(e) = else_block {
                    if let Some(info) = find_accumulator_return(func, e) {
                        return Some(info);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Recognize the accumulator shape for `func`: at least one base return, exactly
/// one recursive return of the form `Return self(args) OP nonRec` (or swapped)
/// with `OP ∈ {+, *}`, and no self-call outside a `Return`.
pub(crate) fn detect_accumulator_pattern(func: Symbol, body: &[Stmt]) -> Option<AccumulatorInfo> {
    if has_non_return_self_calls(func, body) {
        return None;
    }
    let (base, recursive) = count_recursive_returns(func, body);
    if recursive != 1 || base == 0 {
        return None;
    }
    find_accumulator_return(func, body)
}

// =============================================================================
// Accumulator → `while`-loop AST rewrite (consumed by the VM + tree-walker)
// =============================================================================

/// Rewrite every accumulator-shaped function in `stmts` into an equivalent
/// constant-stack `while`-loop, so the bytecode VM and tree-walker run it
/// iteratively (and JIT it) instead of recursing to the call-depth limit. Pure
/// pass-through when nothing qualifies (no allocation). Native/exported functions
/// and those annotated `## No TCO` / `## No Optimize` are left untouched, as are
/// functions already constant-stack via direct/pair tail recursion.
/// Returns `Some(rewritten)` if any function was strength-reduced, else `None`
/// (the caller keeps the original `stmts` — no clone).
pub(crate) fn rewrite_accumulators<'a>(
    stmts: &[Stmt<'a>],
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> Option<&'a [Stmt<'a>]> {
    let mut out: Option<Vec<Stmt<'a>>> = None;
    for (i, s) in stmts.iter().enumerate() {
        let rewritten = accumulator_rewrite_of(s, stmt_arena, expr_arena, interner);
        match (&mut out, rewritten) {
            (Some(v), Some(fd)) => v.push(fd),
            (Some(v), None) => v.push(s.clone()),
            (None, Some(fd)) => {
                let mut v: Vec<Stmt<'a>> = stmts[..i].to_vec();
                v.push(fd);
                out = Some(v);
            }
            (None, None) => {}
        }
    }
    out.map(|v| stmt_arena.alloc_slice(v))
}

/// If `s` is an accumulator-eligible function, return the rewritten `FunctionDef`
/// (a clone with its body replaced by the loop); else `None`.
fn accumulator_rewrite_of<'a>(
    s: &Stmt<'a>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> Option<Stmt<'a>> {
    let Stmt::FunctionDef { name, params, body, is_native, is_exported, opt_flags, .. } = s else {
        return None;
    };
    let body: &'a [Stmt<'a>] = *body;
    if *is_native || *is_exported {
        return None;
    }
    if opt_flags.contains(&OptFlag::NoTCO) || opt_flags.contains(&OptFlag::NoOptimize) {
        return None;
    }
    // `detect_accumulator_pattern` already excludes direct-tail recursion (a bare
    // `Return self(args)` is not a folding binop) and the `Set/Let x; Return x`
    // pair (that is a non-return self-call), so those keep their own constant-
    // stack lowering and never reach here.
    let acc = detect_accumulator_pattern(*name, body)?;
    let param_syms: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
    let new_body = build_accumulator_loop(body, *name, &param_syms, acc, stmt_arena, expr_arena, interner);
    let mut fd = s.clone();
    if let Stmt::FunctionDef { body: b, .. } = &mut fd {
        *b = new_body;
    }
    Some(fd)
}

/// `Let mutable __acc be <identity>. While true: <body with returns folded>.`
fn build_accumulator_loop<'a>(
    body: &'a [Stmt<'a>],
    func: Symbol,
    params: &[Symbol],
    acc: AccumulatorInfo,
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> &'a [Stmt<'a>] {
    let acc_sym = interner.intern("__acc");
    let identity = expr_arena.alloc(Expr::Literal(Literal::Number(acc.identity)));
    let let_acc = Stmt::Let { var: acc_sym, ty: None, value: identity, mutable: true };
    let inner = rewrite_accumulator_body(body, func, params, acc, acc_sym, stmt_arena, expr_arena, interner);
    let cond = expr_arena.alloc(Expr::Literal(Literal::Boolean(true)));
    let while_stmt = Stmt::While { cond, body: inner, decreasing: None };
    stmt_arena.alloc_slice(vec![let_acc, while_stmt])
}

/// Fold `Return`s: a base `Return X` becomes `Return __acc OP X`; the recursive
/// `Return self(args) OP nonRec` becomes `Set __acc to __acc OP nonRec` plus a
/// temp-buffered parameter reassignment (so the `while` re-runs the body). `If`
/// branches recurse.
fn rewrite_accumulator_body<'a>(
    body: &'a [Stmt<'a>],
    func: Symbol,
    params: &[Symbol],
    acc: AccumulatorInfo,
    acc_sym: Symbol,
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> &'a [Stmt<'a>] {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(body.len() + params.len());
    for s in body {
        match s {
            Stmt::Return { value: Some(e) } if expr_contains_self_call(func, e) => {
                let Expr::BinaryOp { op, left, right } = *e else {
                    out.push(s.clone());
                    continue;
                };
                let (call_expr, nonrec): (&'a Expr<'a>, &'a Expr<'a>) = match acc.non_recursive_side {
                    NonRecSide::Left => (*right, *left),
                    NonRecSide::Right => (*left, *right),
                };
                let acc_id = expr_arena.alloc(Expr::Identifier(acc_sym));
                let folded = expr_arena.alloc(Expr::BinaryOp { op: *op, left: acc_id, right: nonrec });
                out.push(Stmt::Set { target: acc_sym, value: folded });
                if let Expr::Call { args, .. } = call_expr {
                    let temps: Vec<Symbol> = (0..args.len())
                        .map(|i| interner.intern(&format!("__acc_arg{}", i)))
                        .collect();
                    for (t, arg) in temps.iter().zip(args.iter()) {
                        out.push(Stmt::Let { var: *t, ty: None, value: *arg, mutable: false });
                    }
                    for (p, t) in params.iter().zip(temps.iter()) {
                        let t_id = expr_arena.alloc(Expr::Identifier(*t));
                        out.push(Stmt::Set { target: *p, value: t_id });
                    }
                }
                // No `Return` — control falls through to the `while` head.
            }
            Stmt::Return { value: Some(e) } => {
                let acc_id = expr_arena.alloc(Expr::Identifier(acc_sym));
                let ret = expr_arena.alloc(Expr::BinaryOp { op: acc.op, left: acc_id, right: *e });
                out.push(Stmt::Return { value: Some(ret) });
            }
            Stmt::Return { value: None } => {
                let acc_id = expr_arena.alloc(Expr::Identifier(acc_sym));
                out.push(Stmt::Return { value: Some(acc_id) });
            }
            Stmt::If { cond, then_block, else_block } => {
                let then_b = rewrite_accumulator_body(then_block, func, params, acc, acc_sym, stmt_arena, expr_arena, interner);
                let else_b = (*else_block)
                    .map(|e| rewrite_accumulator_body(e, func, params, acc, acc_sym, stmt_arena, expr_arena, interner));
                out.push(Stmt::If { cond: *cond, then_block: then_b, else_block: else_b });
            }
            other => out.push(other.clone()),
        }
    }
    stmt_arena.alloc_slice(out)
}
