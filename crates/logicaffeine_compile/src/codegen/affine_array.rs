//! Affine read-only array scalarization — delete a CSR-style offset array and
//! substitute its closed form.
//!
//! An array `A` built by a counted loop whose ONLY write to `A` is one
//! unconditional `push f(i) to A`, with `f(i)` an AFFINE function of the
//! induction variable (`i*5`, `i*3 + 7`, a constant) and the IV starting at `0`
//! and stepping by `1`, holds the invariant `A[p] = f(p)` for every position
//! `p`. If `A` is then never mutated, never aliased, and read only via `item _
//! of A` / `length of A` (all of which occur lexically AFTER the build), each
//! read is the pure arithmetic `f(k-1)`. Codegen can therefore delete the array
//! and its build push and substitute the closed form at every read.
//!
//! This turns graph_bfs's `adjStarts[v]` (a random load from a 24 MB CSR offset
//! array per dequeued vertex) into C's `v * 5` shift — eliminating both the
//! array and the cache miss.
//!
//! SOUNDNESS. The rewrite reproduces the EXACT i64 arithmetic the push computed
//! (`coeff * p + offset`), so wrapping/overflow semantics are identical. The
//! pass only ever removes an array whose every value it can recompute; any shape
//! it does not recognize — an in-place write, a conditional/multiple push, a
//! non-affine value, a non-unit step, an alias or escape, or a read before the
//! build completes — leaves `A` an ordinary `Vec`.

use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use logicaffeine_base::intern::{Interner, Symbol};
use std::collections::{HashMap, HashSet};

use super::worklist::{
    bound_rust_expr, const_eval, for_each_stmt_expr, is_new_empty_int_seq, names_collection,
    ok_read_only, visit_idents,
};

/// How to emit a recognized affine array: the value at 0-based position `p` is
/// `coeff * p + offset`, and `length of A` is `trip`. (The IV is required to
/// start at 0, so position == iteration and these are the push's own constants.)
#[derive(Clone, Debug)]
pub(crate) struct AffineArrayInfo {
    pub coeff: i64,
    pub offset: i64,
    /// Rust expression for the element count (the value of `length of A`).
    pub trip: String,
}

/// Recognize the affine read-only arrays in a body. Conservative: every
/// condition in the module doc must hold, else the candidate is dropped.
pub(crate) fn detect_affine_arrays(
    body: &[Stmt],
    de_rc: &HashSet<Symbol>,
    interner: &Interner,
) -> HashMap<Symbol, AffineArrayInfo> {
    let mut out = HashMap::new();
    // Kill-switch (A/B and attribution), matching `LOGOS_NO_NARROW`.
    if std::env::var_os("LOGOS_NO_AFFINE").is_some() {
        return out;
    }
    for (di, stmt) in body.iter().enumerate() {
        let Stmt::Let { var, value, .. } = stmt else { continue };
        if !is_new_empty_int_seq(value) {
            continue;
        }
        let a = *var;
        // SOUNDNESS: only a uniquely-owned (de-Rc'd) sequence is safe to delete —
        // an aliased `LogosSeq` could be observed through the alias.
        if !de_rc.contains(&a) {
            continue;
        }
        if let Some(info) = analyze(a, di, body, interner) {
            out.insert(a, info);
        }
    }
    out
}

struct AffineFit {
    coeff: i64,
    offset: i64,
    trip: String,
}

fn analyze(a: Symbol, di: usize, body: &[Stmt], interner: &Interner) -> Option<AffineArrayInfo> {
    // Find the build loop: the first top-level `While i < N` after the decl
    // whose body affinely builds `A`.
    let mut build: Option<(usize, AffineFit)> = None;
    for bi in (di + 1)..body.len() {
        let Stmt::While { cond, body: lbody, .. } = &body[bi] else { continue };
        if let Some(fit) = match_build_loop(a, cond, lbody, body, bi, interner) {
            build = Some((bi, fit));
            break;
        }
    }
    let (bi, fit) = build?;

    // No reference to `A` before the build completes (a read of a partially-built
    // array would change behavior). The decl itself names no expression, so it is
    // exempt.
    for (idx, s) in body.iter().enumerate() {
        if idx >= bi {
            break;
        }
        if idx == di {
            continue;
        }
        if mentions_anywhere(s, a) {
            return None;
        }
    }

    // Read-only after the build: every later reference is `item _ of A` /
    // `length of A`; any write, push, alias, or escape disqualifies.
    for s in &body[bi + 1..] {
        if !read_only_stmt(s, a) {
            return None;
        }
    }

    // The payoff of this pass is turning `item k of A` reads into arithmetic
    // (graph_bfs's `adjStarts[v]` → `v*5`). An array read ONLY via `length of A`
    // (e.g. a constant fill whose count is all that's used) is not our target —
    // leave it to the fill / length-hoist passes rather than deleting it here.
    if !reads_item_of(&body[bi + 1..], a) {
        return None;
    }
    // `length of A` substitutes the trip count — but the for-range loop-bound
    // path renders it via a context-free codegen (`arr.len()`) that never sees
    // our rewrite, so deleting `A` would dangle that `.len()`. Decline whenever
    // `length of A` is used (graph_bfs reads `adjStarts` only via `item`, so it
    // is unaffected). Strictly conservative: the array simply stays a `Vec`.
    if reads_length_of(body, a) {
        return None;
    }

    Some(AffineArrayInfo { coeff: fit.coeff, offset: fit.offset, trip: fit.trip })
}

/// `while i < N` (or `i <= N`) whose body is exactly: one unconditional affine
/// `push f(i) to A`, one `Set i to i + 1`, and statements touching neither `A`
/// nor `i`. The IV must start at 0 (so position == iteration).
fn match_build_loop(
    a: Symbol,
    cond: &Expr,
    lbody: &[Stmt],
    body: &[Stmt],
    bi: usize,
    interner: &Interner,
) -> Option<AffineFit> {
    let Expr::BinaryOp { op, left, right } = cond else { return None };
    let inclusive = match op {
        BinaryOpKind::Lt => false,
        BinaryOpKind::LtEq => true,
        _ => return None,
    };
    let Expr::Identifier(iv) = left else { return None };
    let iv = *iv;
    let n_str = bound_rust_expr(right, interner)?;

    // The trip count `length of A` substitutes the bound expression verbatim, so a
    // variable bound must hold its build-time value: reject if it is ever
    // reassigned (a literal bound is always stable).
    if let Expr::Identifier(bound_sym) = right {
        if is_set_anywhere(body, *bound_sym) {
            return None;
        }
    }

    if !iv_starts_at_zero(body, bi, iv) {
        return None;
    }

    let mut push_fit: Option<(i64, i64)> = None;
    let mut iv_increments = 0;
    for s in lbody {
        match s {
            // The build push of A.
            Stmt::Push { collection, value } if names_collection(collection, a) => {
                if push_fit.is_some() {
                    return None; // more than one push to A
                }
                push_fit = Some(extract_affine(value, iv)?);
            }
            // The IV step: must be exactly `i + 1`, exactly once.
            Stmt::Set { target, value } if *target == iv => {
                if !is_increment_by_one(value, iv) {
                    return None;
                }
                iv_increments += 1;
            }
            // Anything else must touch neither A (no read/write inside the build)
            // nor the IV (a shadowing rebind would break position == iteration).
            other => {
                if mentions_anywhere(other, a) || assigns_var(other, iv) {
                    return None;
                }
            }
        }
    }

    let (coeff, offset) = push_fit?;
    // This pass exists to turn `item k of A` into the slope arithmetic `coeff*k`
    // (graph_bfs's `adjStarts[v]` → `v*5`). A constant array (coeff == 0) is not a
    // CSR offset table — leave it to the fill / with_capacity passes, which other
    // optimizations and tests expect to handle it.
    if coeff == 0 {
        return None;
    }
    if iv_increments != 1 {
        return None;
    }

    // IV starts at 0 and steps by 1, so the element count is the trip count.
    let trip = if inclusive { format!("({} + 1)", n_str) } else { n_str };
    Some(AffineFit { coeff, offset, trip })
}

/// The nearest assignment to `iv` before the build loop sets it to literal 0.
fn iv_starts_at_zero(body: &[Stmt], bi: usize, iv: Symbol) -> bool {
    for idx in (0..bi).rev() {
        match &body[idx] {
            Stmt::Let { var, value, .. } if *var == iv => return const_eval(value) == Some(0),
            Stmt::Set { target, value } if *target == iv => return const_eval(value) == Some(0),
            _ => {}
        }
    }
    false
}

/// Reify `e` as `coeff * iv + offset` with constant `coeff`/`offset`, referencing
/// only `iv` and integer constants. `None` for any non-affine or other-variable
/// term (`i*i`, `i*stride`, `i + j`).
fn extract_affine(e: &Expr, iv: Symbol) -> Option<(i64, i64)> {
    if let Some(c) = const_eval(e) {
        return Some((0, c));
    }
    match e {
        Expr::Identifier(s) if *s == iv => Some((1, 0)),
        Expr::BinaryOp { op, left, right } => match op {
            BinaryOpKind::Add => {
                let (lc, lo) = extract_affine(left, iv)?;
                let (rc, ro) = extract_affine(right, iv)?;
                Some((lc.checked_add(rc)?, lo.checked_add(ro)?))
            }
            BinaryOpKind::Subtract => {
                let (lc, lo) = extract_affine(left, iv)?;
                let (rc, ro) = extract_affine(right, iv)?;
                Some((lc.checked_sub(rc)?, lo.checked_sub(ro)?))
            }
            BinaryOpKind::Multiply => {
                let l = extract_affine(left, iv)?;
                let r = extract_affine(right, iv)?;
                // Affine × affine is affine only when one factor is a constant.
                match (l, r) {
                    ((0, k), (c, o)) | ((c, o), (0, k)) => {
                        Some((c.checked_mul(k)?, o.checked_mul(k)?))
                    }
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// `e == iv + 1` (or `1 + iv`).
fn is_increment_by_one(e: &Expr, iv: Symbol) -> bool {
    let is_iv = |x: &Expr| matches!(x, Expr::Identifier(s) if *s == iv);
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
        if (is_iv(left) && const_eval(right) == Some(1))
            || (const_eval(left) == Some(1) && is_iv(right)))
}

/// `s` is a `Let`/`Set` that assigns `v`.
fn assigns_var(s: &Stmt, v: Symbol) -> bool {
    matches!(s, Stmt::Let { var, .. } if *var == v)
        || matches!(s, Stmt::Set { target, .. } if *target == v)
}

/// `v` is reassigned by a `Set` anywhere in the body, including nested blocks.
/// (The initial `Let` definition does not count — only mutation.)
fn is_set_anywhere(body: &[Stmt], v: Symbol) -> bool {
    body.iter().any(|s| match s {
        Stmt::Set { target, .. } if *target == v => true,
        Stmt::If { then_block, else_block, .. } => {
            is_set_anywhere(then_block, v)
                || else_block.as_ref().map_or(false, |eb| is_set_anywhere(eb, v))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => is_set_anywhere(body, v),
        _ => false,
    })
}

/// `a` appears anywhere in the statement, including nested blocks.
fn mentions_anywhere(s: &Stmt, a: Symbol) -> bool {
    let mut found = false;
    for_each_stmt_expr(s, &mut |e| {
        visit_idents(e, &mut |sym| {
            if sym == a {
                found = true;
            }
        });
    });
    if found {
        return true;
    }
    match s {
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|x| mentions_anywhere(x, a))
                || else_block.as_ref().map_or(false, |eb| eb.iter().any(|x| mentions_anywhere(x, a)))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            body.iter().any(|x| mentions_anywhere(x, a))
        }
        _ => false,
    }
}

/// Does any expression in `stmts` (including nested blocks) read `item _ of a`?
fn reads_item_of(stmts: &[Stmt], a: Symbol) -> bool {
    stmts.iter().any(|s| {
        let mut hit = false;
        for_each_stmt_expr(s, &mut |e| {
            if expr_has_index_of(e, a) {
                hit = true;
            }
        });
        hit || match s {
            Stmt::If { then_block, else_block, .. } => {
                reads_item_of(then_block, a)
                    || else_block.as_ref().map_or(false, |eb| reads_item_of(eb, a))
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => reads_item_of(body, a),
            _ => false,
        }
    })
}

/// Does any expression in `stmts` (including nested blocks) read `length of a`?
fn reads_length_of(stmts: &[Stmt], a: Symbol) -> bool {
    stmts.iter().any(|s| {
        let mut hit = false;
        for_each_stmt_expr(s, &mut |e| {
            if expr_has_length_of(e, a) {
                hit = true;
            }
        });
        hit || match s {
            Stmt::If { then_block, else_block, .. } => {
                reads_length_of(then_block, a)
                    || else_block.as_ref().map_or(false, |eb| reads_length_of(eb, a))
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => reads_length_of(body, a),
            _ => false,
        }
    })
}

/// `length of a` appears anywhere in the expression tree.
fn expr_has_length_of(e: &Expr, a: Symbol) -> bool {
    match e {
        Expr::Length { collection } => names_collection(collection, a) || expr_has_length_of(collection, a),
        Expr::Index { collection, index } => expr_has_length_of(collection, a) || expr_has_length_of(index, a),
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            expr_has_length_of(left, a) || expr_has_length_of(right, a)
        }
        Expr::Not { operand } => expr_has_length_of(operand, a),
        Expr::Copy { expr } | Expr::Give { value: expr } | Expr::OptionSome { value: expr } => {
            expr_has_length_of(expr, a)
        }
        Expr::FieldAccess { object, .. } => expr_has_length_of(object, a),
        Expr::Contains { collection, value } => {
            expr_has_length_of(collection, a) || expr_has_length_of(value, a)
        }
        Expr::Slice { collection, start, end } => {
            expr_has_length_of(collection, a) || expr_has_length_of(start, a) || expr_has_length_of(end, a)
        }
        Expr::Call { args, .. } => args.iter().any(|x| expr_has_length_of(x, a)),
        Expr::CallExpr { callee, args } => {
            expr_has_length_of(callee, a) || args.iter().any(|x| expr_has_length_of(x, a))
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|x| expr_has_length_of(x, a)),
        Expr::InterpolatedString(parts) => parts.iter().any(|p| {
            matches!(p, crate::ast::stmt::StringPart::Expr { value, .. } if expr_has_length_of(value, a))
        }),
        _ => false,
    }
}

/// `item _ of a` appears anywhere in the expression tree.
fn expr_has_index_of(e: &Expr, a: Symbol) -> bool {
    match e {
        Expr::Index { collection, index } => {
            names_collection(collection, a)
                || expr_has_index_of(collection, a)
                || expr_has_index_of(index, a)
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            expr_has_index_of(left, a) || expr_has_index_of(right, a)
        }
        Expr::Not { operand } => expr_has_index_of(operand, a),
        Expr::Length { collection } => expr_has_index_of(collection, a),
        Expr::Copy { expr } | Expr::Give { value: expr } | Expr::OptionSome { value: expr } => {
            expr_has_index_of(expr, a)
        }
        Expr::FieldAccess { object, .. } => expr_has_index_of(object, a),
        Expr::Contains { collection, value } => {
            expr_has_index_of(collection, a) || expr_has_index_of(value, a)
        }
        Expr::Slice { collection, start, end } => {
            expr_has_index_of(collection, a) || expr_has_index_of(start, a) || expr_has_index_of(end, a)
        }
        Expr::Call { args, .. } => args.iter().any(|x| expr_has_index_of(x, a)),
        Expr::CallExpr { callee, args } => {
            expr_has_index_of(callee, a) || args.iter().any(|x| expr_has_index_of(x, a))
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|x| expr_has_index_of(x, a)),
        Expr::InterpolatedString(parts) => parts.iter().any(|p| {
            matches!(p, crate::ast::stmt::StringPart::Expr { value, .. } if expr_has_index_of(value, a))
        }),
        _ => false,
    }
}

/// `a` is used read-only in `s`: only via `item _ of a` / `length of a`, never
/// written (push/setindex/pop/remove/add), never bare (aliased/escaped).
fn read_only_stmt(s: &Stmt, a: Symbol) -> bool {
    match s {
        Stmt::Push { collection, value } => !names_collection(collection, a) && ok_read_only(value, a),
        Stmt::Pop { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::Add { collection, .. } => !names_collection(collection, a),
        Stmt::SetIndex { collection, index, value } => {
            !names_collection(collection, a) && ok_read_only(index, a) && ok_read_only(value, a)
        }
        // A rebind of `a` itself (`Set a to …` / a shadowing `Let a be …`) means
        // the array is no longer the affine sequence we proved — disqualify.
        Stmt::Let { var, value, .. } => *var != a && ok_read_only(value, a),
        Stmt::Set { target, value } => *target != a && ok_read_only(value, a),
        Stmt::SetField { object, value, .. } => ok_read_only(object, a) && ok_read_only(value, a),
        Stmt::If { cond, then_block, else_block } => {
            ok_read_only(cond, a)
                && then_block.iter().all(|x| read_only_stmt(x, a))
                && else_block.as_ref().map_or(true, |eb| eb.iter().all(|x| read_only_stmt(x, a)))
        }
        Stmt::While { cond, body, .. } => {
            ok_read_only(cond, a) && body.iter().all(|x| read_only_stmt(x, a))
        }
        Stmt::Repeat { body, .. } => body.iter().all(|x| read_only_stmt(x, a)),
        Stmt::Return { value: Some(v) } => ok_read_only(v, a),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            ok_read_only(object, a) && ok_read_only(recipient, a)
        }
        Stmt::Call { args, .. } => args.iter().all(|x| ok_read_only(x, a)),
        Stmt::RuntimeAssert { condition } => ok_read_only(condition, a),
        Stmt::Inspect { target, .. } => ok_read_only(target, a),
        _ => !mentions_anywhere(s, a),
    }
}
