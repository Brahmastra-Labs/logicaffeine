//! Affine read-only `Seq` scalarization for the RUN path — delete a CSR-style
//! offset array and substitute its closed form.
//!
//! Motivation (graph_bfs): the kernel builds `adjStarts` with one counted loop
//! whose only write is `Push i * 5 to adjStarts`, then reads it only as
//! `item (v + 1) of adjStarts` — a data-dependent random load from a multi-MB
//! offset table, once per dequeued vertex. Because the array is built by an
//! AFFINE function of a unit-stride induction variable starting at 0, element
//! `p` (0-based) is exactly `i * 5` at iteration `p`, so the invariant
//! `adjStarts[p] = 5 * p` holds for every position. Every 1-based
//! `item k of adjStarts` read is therefore the pure arithmetic `5 * (k - 1)`,
//! and the array — together with its build push — can be deleted entirely. The
//! random heap load collapses to C's `v * 5` shift.
//!
//! **Conservative by construction.** A candidate qualifies only when EVERY
//! condition holds: it is declared `new Seq of Int`/`Float`; its ONLY write is
//! one unconditional `Push f(i) to arr` inside ONE top-level counted
//! `while i < N` (or `i <= N`) loop whose IV starts at 0 and steps by 1; the
//! pushed value `f(i)` is affine in `i` (`a*i + b`); it is never referenced
//! before that loop completes; and after the loop it appears only as
//! `item _ of arr` reads — never written by `SetIndex`/`Push`/`Pop`/…, never
//! length-queried, never aliased or escaped to a call. Any shape it does not
//! recognize leaves the Seq an ordinary `Vec`, untouched.
//!
//! **Value-preserving.** The substituted closed form `a * (k - 1) + b` reifies
//! the EXACT i64/f64 arithmetic the push computed at iteration `k - 1`, so its
//! wrapping/overflow semantics are identical. No read or write order is
//! reassociated; the array's other co-built sibling arrays (graph_bfs's
//! `adjCounts`/`adj`) keep their pushes in place.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, MatchArm, Stmt};
use crate::intern::{Interner, Symbol};

/// The closed form for a recognized affine array: element `p` (0-based) is
/// `coeff * p + offset`, so 1-based `item k of arr` is `coeff * (k - 1) + offset`.
#[derive(Clone, Copy, Debug)]
struct AffineInfo {
    coeff: i64,
    offset: i64,
}

/// Replace affine read-only `Seq` locals with their closed form, deleting the
/// array and its build push. Returns the (possibly) rewritten statements and
/// whether anything changed. When nothing qualifies the ORIGINAL statements are
/// returned untouched — a guaranteed no-op on programs without such an array.
pub fn affine_scalarize_seqs<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let _ = interner;
    let qualified = detect_affine_arrays(&stmts);
    if qualified.is_empty() {
        return (stmts, false);
    }

    let mut rw = Rewriter { qualified: &qualified, expr_arena, stmt_arena };
    let out = rw.rewrite_stmts(stmts);
    (out, true)
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Recognize every affine read-only array at the top level of `stmts`.
fn detect_affine_arrays(stmts: &[Stmt]) -> HashMap<Symbol, AffineInfo> {
    let mut out = HashMap::new();
    for (di, stmt) in stmts.iter().enumerate() {
        let Stmt::Let { var, value, .. } = stmt else { continue };
        if !is_new_empty_numeric_seq(value) {
            continue;
        }
        let a = *var;
        if let Some(info) = analyze(a, di, stmts) {
            out.insert(a, info);
        }
    }
    out
}

/// `a new Seq of Int`/`a new Seq of Float` (an empty, numeric-element sequence).
fn is_new_empty_numeric_seq(value: &Expr) -> bool {
    let Expr::New { type_args, init_fields, .. } = value else { return false };
    init_fields.is_empty() && type_args.len() == 1
}

/// Full analysis for the array declared at `body[di]`: find its build loop,
/// prove it read-only afterward, and recover the closed form.
fn analyze(a: Symbol, di: usize, body: &[Stmt]) -> Option<AffineInfo> {
    // The build loop: the first top-level `while i < N` after the decl whose
    // body holds exactly one affine `push f(i) to a`.
    let mut build: Option<(usize, AffineInfo)> = None;
    for bi in (di + 1)..body.len() {
        let Stmt::While { cond, body: lbody, .. } = &body[bi] else { continue };
        if let Some(info) = match_build_loop(a, cond, lbody, body, bi) {
            build = Some((bi, info));
            break;
        }
    }
    let (bi, info) = build?;

    // No reference to `a` before the build completes — a read of a partially
    // built array would change behavior. The decl itself names no expression of
    // `a`, so it is exempt.
    for (idx, s) in body.iter().enumerate() {
        if idx >= bi {
            break;
        }
        if idx == di {
            continue;
        }
        if stmt_mentions(s, a) {
            return None;
        }
    }

    // Read-only after the build: every later reference is `item _ of a`; any
    // write, push, length-query, alias, or escape disqualifies.
    for s in &body[bi + 1..] {
        if !read_only_stmt(s, a) {
            return None;
        }
    }

    // The payoff is turning `item k of a` reads into arithmetic. An array read
    // ONLY via `length of a` (already barred above) or never read at all is not
    // our target — require at least one `item _ of a` read to fire.
    if !reads_item_of(&body[bi + 1..], a) {
        return None;
    }

    Some(info)
}

/// `while i < N` (or `i <= N`) whose body has exactly one unconditional affine
/// `push f(i) to a`, an `i := i + 1` step, and statements touching neither `a`
/// nor `i`. The IV must start at 0 (so position == iteration). Returns the
/// closed form `f(p) = coeff*p + offset`.
fn match_build_loop(
    a: Symbol,
    cond: &Expr,
    lbody: &[Stmt],
    body: &[Stmt],
    bi: usize,
) -> Option<AffineInfo> {
    let Expr::BinaryOp { op, left, right: _ } = cond else { return None };
    match op {
        BinaryOpKind::Lt | BinaryOpKind::LtEq => {}
        _ => return None,
    }
    let Expr::Identifier(iv) = left else { return None };
    let iv = *iv;

    if !iv_starts_at_zero(body, bi, iv) {
        return None;
    }

    let mut push_fit: Option<AffineInfo> = None;
    let mut iv_increments = 0;
    for s in lbody {
        match s {
            // The build push of `a`.
            Stmt::Push { collection, value } if names_collection(collection, a) => {
                if push_fit.is_some() {
                    return None; // more than one push to a
                }
                let (coeff, offset) = extract_affine(value, iv)?;
                push_fit = Some(AffineInfo { coeff, offset });
            }
            // The IV step: must be exactly `i := i + 1`, exactly once.
            Stmt::Set { target, value } if *target == iv => {
                if !is_increment_by_one(value, iv) {
                    return None;
                }
                iv_increments += 1;
            }
            // Anything else must touch neither `a` (no read/write inside the
            // build) nor the IV (a shadowing rebind would break position ==
            // iteration). A nested control-flow statement is rejected outright:
            // it would make the push conditional or the count non-affine.
            other => {
                if stmt_mentions(other, a) || assigns_var(other, iv) || is_control_flow(other) {
                    return None;
                }
            }
        }
    }

    if iv_increments != 1 {
        return None;
    }
    push_fit
}

/// `e == iv + 1` (or `1 + iv`).
fn is_increment_by_one(e: &Expr, iv: Symbol) -> bool {
    let is_iv = |x: &Expr| matches!(x, Expr::Identifier(s) if *s == iv);
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
        if (is_iv(left) && const_eval(right) == Some(1))
            || (const_eval(left) == Some(1) && is_iv(right)))
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

/// Fold a constant integer expression (`-1` is `0 - 1` in the AST, so a plain
/// `Literal` match is not enough).
fn const_eval(e: &Expr) -> Option<i64> {
    crate::loop_shape::const_eval_i64(e)
}

fn names_collection(e: &Expr, a: Symbol) -> bool {
    matches!(e, Expr::Identifier(s) if *s == a)
}

/// `s` is a `Let`/`Set` that assigns `v`.
fn assigns_var(s: &Stmt, v: Symbol) -> bool {
    matches!(s, Stmt::Let { var, .. } if *var == v)
        || matches!(s, Stmt::Set { target, .. } if *target == v)
}

/// `s` opens a nested control-flow scope (so a push inside the build loop could
/// be conditional / repeated). Plain straight-line statements are not.
fn is_control_flow(s: &Stmt) -> bool {
    matches!(
        s,
        Stmt::If { .. }
            | Stmt::While { .. }
            | Stmt::Repeat { .. }
            | Stmt::Inspect { .. }
            | Stmt::Zone { .. }
            | Stmt::Concurrent { .. }
            | Stmt::Parallel { .. }
    )
}

// ---------------------------------------------------------------------------
// Use classification (read-only / mention scans)
// ---------------------------------------------------------------------------

/// `a` is used read-only in `s`: only via `item _ of a`, never written
/// (push/setindex/pop/remove/add), never length-queried, never bare (an alias
/// or escape).
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
        Stmt::Repeat { iterable, body, .. } => {
            ok_read_only(iterable, a) && body.iter().all(|x| read_only_stmt(x, a))
        }
        Stmt::Inspect { target, arms, .. } => {
            ok_read_only(target, a) && arms.iter().all(|arm| arm.body.iter().all(|x| read_only_stmt(x, a)))
        }
        Stmt::Zone { body, .. }
        | Stmt::Concurrent { tasks: body }
        | Stmt::Parallel { tasks: body } => body.iter().all(|x| read_only_stmt(x, a)),
        Stmt::Return { value: Some(v) } => ok_read_only(v, a),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            ok_read_only(object, a) && ok_read_only(recipient, a)
        }
        Stmt::Call { args, .. } => args.iter().all(|x| ok_read_only(x, a)),
        Stmt::RuntimeAssert { condition, .. } => ok_read_only(condition, a),
        _ => !stmt_mentions(s, a),
    }
}

/// `a` appears in `e` only inside `item _ of a` reads — never bare (which would
/// alias or escape it), never length-queried, never as another op's collection.
fn ok_read_only(e: &Expr, a: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s != a, // a bare reference to `a` escapes it
        Expr::Index { collection, index } => {
            let coll_ok = match collection {
                Expr::Identifier(s) if *s == a => true,
                other => ok_read_only(other, a),
            };
            coll_ok && ok_read_only(index, a)
        }
        // `length of a` cannot be honored once the array is deleted — disqualify.
        Expr::Length { collection } => match collection {
            Expr::Identifier(s) if *s == a => false,
            other => ok_read_only(other, a),
        },
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => ok_read_only(left, a) && ok_read_only(right, a),
        Expr::Not { operand } => ok_read_only(operand, a),
        Expr::Call { args, .. } => args.iter().all(|x| ok_read_only(x, a)),
        Expr::CallExpr { callee, args } => {
            ok_read_only(callee, a) && args.iter().all(|x| ok_read_only(x, a))
        }
        Expr::Copy { expr } | Expr::Give { value: expr } | Expr::OptionSome { value: expr } => {
            ok_read_only(expr, a)
        }
        Expr::Contains { collection, value } => {
            !names_collection(collection, a) && ok_read_only(value, a)
        }
        Expr::Slice { collection, start, end } => {
            !names_collection(collection, a) && ok_read_only(start, a) && ok_read_only(end, a)
        }
        Expr::FieldAccess { object, .. } => ok_read_only(object, a),
        Expr::List(items) | Expr::Tuple(items) => items.iter().all(|i| ok_read_only(i, a)),
        Expr::WithCapacity { value, capacity } => ok_read_only(value, a) && ok_read_only(capacity, a),
        Expr::InterpolatedString(parts) => parts.iter().all(|p| match p {
            crate::ast::stmt::StringPart::Expr { value, .. } => ok_read_only(value, a),
            _ => true,
        }),
        _ => !expr_mentions(e, a),
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
            Stmt::Inspect { arms, .. } => arms.iter().any(|arm| reads_item_of(arm.body, a)),
            Stmt::Zone { body, .. }
            | Stmt::Concurrent { tasks: body }
            | Stmt::Parallel { tasks: body } => reads_item_of(body, a),
            _ => false,
        }
    })
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
            expr_has_index_of(collection, a)
                || expr_has_index_of(start, a)
                || expr_has_index_of(end, a)
        }
        Expr::WithCapacity { value, capacity } => {
            expr_has_index_of(value, a) || expr_has_index_of(capacity, a)
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

/// `a` appears anywhere in the statement's own expressions or nested blocks.
fn stmt_mentions(s: &Stmt, a: Symbol) -> bool {
    let mut found = false;
    for_each_stmt_expr(s, &mut |e| {
        if expr_mentions(e, a) {
            found = true;
        }
    });
    if found {
        return true;
    }
    match s {
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|x| stmt_mentions(x, a))
                || else_block.as_ref().map_or(false, |eb| eb.iter().any(|x| stmt_mentions(x, a)))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => body.iter().any(|x| stmt_mentions(x, a)),
        Stmt::Inspect { arms, .. } => arms.iter().any(|arm| arm.body.iter().any(|x| stmt_mentions(x, a))),
        Stmt::Zone { body, .. }
        | Stmt::Concurrent { tasks: body }
        | Stmt::Parallel { tasks: body } => body.iter().any(|x| stmt_mentions(x, a)),
        _ => false,
    }
}

/// `a` appears anywhere in the expression tree (any position).
fn expr_mentions(e: &Expr, a: Symbol) -> bool {
    let mut found = false;
    visit_idents(e, &mut |s| {
        if s == a {
            found = true;
        }
    });
    found
}

/// Visit every `Identifier` in an expression tree.
fn visit_idents(e: &Expr, f: &mut impl FnMut(Symbol)) {
    match e {
        Expr::Identifier(s) => f(*s),
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            visit_idents(left, f);
            visit_idents(right, f);
        }
        Expr::Not { operand } => visit_idents(operand, f),
        Expr::Index { collection, index } => {
            visit_idents(collection, f);
            visit_idents(index, f);
        }
        Expr::Length { collection }
        | Expr::Copy { expr: collection }
        | Expr::Give { value: collection }
        | Expr::OptionSome { value: collection }
        | Expr::FieldAccess { object: collection, .. } => visit_idents(collection, f),
        Expr::Contains { collection, value } => {
            visit_idents(collection, f);
            visit_idents(value, f);
        }
        Expr::Slice { collection, start, end } => {
            visit_idents(collection, f);
            visit_idents(start, f);
            visit_idents(end, f);
        }
        Expr::Call { args, .. } => args.iter().for_each(|a| visit_idents(a, f)),
        Expr::CallExpr { callee, args } => {
            visit_idents(callee, f);
            args.iter().for_each(|a| visit_idents(a, f));
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().for_each(|i| visit_idents(i, f)),
        Expr::New { init_fields, .. } => init_fields.iter().for_each(|(_, e)| visit_idents(e, f)),
        Expr::NewVariant { fields, .. } => fields.iter().for_each(|(_, e)| visit_idents(e, f)),
        Expr::WithCapacity { value, capacity } => {
            visit_idents(value, f);
            visit_idents(capacity, f);
        }
        Expr::InterpolatedString(parts) => parts.iter().for_each(|p| {
            if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                visit_idents(value, f);
            }
        }),
        _ => {}
    }
}

/// Visit every top-level expression of a statement (not recursing into nested
/// blocks — callers walk those separately).
fn for_each_stmt_expr(s: &Stmt, f: &mut impl FnMut(&Expr)) {
    match s {
        Stmt::Let { value, .. }
        | Stmt::Set { value, .. }
        | Stmt::Return { value: Some(value) }
        | Stmt::Inspect { target: value, .. } => f(value),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            f(object);
            f(recipient);
        }
        Stmt::Push { collection, value } | Stmt::Add { collection, value } => {
            f(collection);
            f(value);
        }
        Stmt::Pop { collection, .. } | Stmt::Remove { collection, .. } => f(collection),
        Stmt::SetIndex { collection, index, value } => {
            f(collection);
            f(index);
            f(value);
        }
        Stmt::SetField { object, value, .. } => {
            f(object);
            f(value);
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => f(cond),
        Stmt::Repeat { iterable, .. } => f(iterable),
        Stmt::Call { args, .. } => args.iter().for_each(|a| f(a)),
        Stmt::RuntimeAssert { condition, .. } => f(condition),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Rewrite
// ---------------------------------------------------------------------------

struct Rewriter<'a, 'q> {
    qualified: &'q HashMap<Symbol, AffineInfo>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
}

impl<'a, 'q> Rewriter<'a, 'q> {
    fn rewrite_stmts(&mut self, stmts: Vec<Stmt<'a>>) -> Vec<Stmt<'a>> {
        let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            self.rewrite_stmt(stmt, &mut out);
        }
        out
    }

    fn rewrite_block_ref(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        let v = self.rewrite_stmts(block.to_vec());
        self.stmt_arena.alloc_slice(v)
    }

    fn rewrite_stmt(&mut self, stmt: Stmt<'a>, out: &mut Vec<Stmt<'a>>) {
        match stmt {
            // The `new Seq` declaration of a qualified array is deleted — the
            // array no longer exists; its reads are substituted below.
            Stmt::Let { var, .. } if self.qualified.contains_key(&var) => {}
            // The build `Push f(i) to arr` of a qualified array is deleted; any
            // sibling pushes in the same loop are left in place by the block
            // walk (this arm fires only for the affine array's own push).
            Stmt::Push { collection, .. }
                if matches!(collection, Expr::Identifier(s) if self.qualified.contains_key(s)) => {}
            Stmt::Let { var, ty, value, mutable } => {
                out.push(Stmt::Let { var, ty, value: self.rewrite_expr(value), mutable });
            }
            Stmt::Set { target, value } => {
                out.push(Stmt::Set { target, value: self.rewrite_expr(value) });
            }
            Stmt::SetIndex { collection, index, value } => {
                out.push(Stmt::SetIndex {
                    collection: self.rewrite_expr(collection),
                    index: self.rewrite_expr(index),
                    value: self.rewrite_expr(value),
                });
            }
            Stmt::Push { value, collection } => {
                out.push(Stmt::Push {
                    value: self.rewrite_expr(value),
                    collection: self.rewrite_expr(collection),
                });
            }
            Stmt::Show { object, recipient } => {
                out.push(Stmt::Show {
                    object: self.rewrite_expr(object),
                    recipient: self.rewrite_expr(recipient),
                });
            }
            Stmt::Give { object, recipient } => {
                out.push(Stmt::Give {
                    object: self.rewrite_expr(object),
                    recipient: self.rewrite_expr(recipient),
                });
            }
            Stmt::Return { value } => {
                out.push(Stmt::Return { value: value.map(|v| self.rewrite_expr(v)) });
            }
            Stmt::RuntimeAssert { condition, hard } => {
                out.push(Stmt::RuntimeAssert { condition: self.rewrite_expr(condition) , hard });
            }
            Stmt::Call { function, args } => {
                let args = args.into_iter().map(|a| self.rewrite_expr(a)).collect();
                out.push(Stmt::Call { function, args });
            }
            Stmt::SetField { object, field, value } => {
                out.push(Stmt::SetField {
                    object: self.rewrite_expr(object),
                    field,
                    value: self.rewrite_expr(value),
                });
            }
            Stmt::Add { value, collection } => {
                out.push(Stmt::Add {
                    value: self.rewrite_expr(value),
                    collection: self.rewrite_expr(collection),
                });
            }
            Stmt::Remove { value, collection } => {
                out.push(Stmt::Remove {
                    value: self.rewrite_expr(value),
                    collection: self.rewrite_expr(collection),
                });
            }
            Stmt::If { cond, then_block, else_block } => {
                out.push(Stmt::If {
                    cond: self.rewrite_expr(cond),
                    then_block: self.rewrite_block_ref(then_block),
                    else_block: else_block.map(|b| self.rewrite_block_ref(b)),
                });
            }
            Stmt::While { cond, body, decreasing } => {
                out.push(Stmt::While {
                    cond: self.rewrite_expr(cond),
                    body: self.rewrite_block_ref(body),
                    decreasing: decreasing.map(|d| self.rewrite_expr(d)),
                });
            }
            Stmt::Repeat { pattern, iterable, body } => {
                out.push(Stmt::Repeat {
                    pattern,
                    iterable: self.rewrite_expr(iterable),
                    body: self.rewrite_block_ref(body),
                });
            }
            Stmt::Inspect { target, arms, has_otherwise } => {
                let arms = arms
                    .into_iter()
                    .map(|a| MatchArm {
                        enum_name: a.enum_name,
                        variant: a.variant,
                        bindings: a.bindings,
                        body: self.rewrite_block_ref(a.body),
                    })
                    .collect();
                out.push(Stmt::Inspect { target: self.rewrite_expr(target), arms, has_otherwise });
            }
            Stmt::Zone { name, capacity, source_file, body } => {
                out.push(Stmt::Zone {
                    name,
                    capacity,
                    source_file,
                    body: self.rewrite_block_ref(body),
                });
            }
            other => out.push(other),
        }
    }

    fn rewrite_expr(&self, expr: &'a Expr<'a>) -> &'a Expr<'a> {
        match expr {
            // `item idx of arr` (qualified arr) → `coeff * (idx - 1) + offset`.
            Expr::Index { collection, index } => {
                if let Expr::Identifier(s) = collection {
                    if let Some(info) = self.qualified.get(s) {
                        let idx = self.rewrite_expr(index);
                        return self.closed_form(*info, idx);
                    }
                }
                self.expr_arena.alloc(Expr::Index {
                    collection: self.rewrite_expr(collection),
                    index: self.rewrite_expr(index),
                })
            }
            Expr::BinaryOp { op, left, right } => self.expr_arena.alloc(Expr::BinaryOp {
                op: *op,
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Not { operand } => {
                self.expr_arena.alloc(Expr::Not { operand: self.rewrite_expr(operand) })
            }
            Expr::Call { function, args } => self.expr_arena.alloc(Expr::Call {
                function: *function,
                args: args.iter().map(|a| self.rewrite_expr(a)).collect(),
            }),
            Expr::CallExpr { callee, args } => self.expr_arena.alloc(Expr::CallExpr {
                callee: self.rewrite_expr(callee),
                args: args.iter().map(|a| self.rewrite_expr(a)).collect(),
            }),
            Expr::Slice { collection, start, end } => self.expr_arena.alloc(Expr::Slice {
                collection: self.rewrite_expr(collection),
                start: self.rewrite_expr(start),
                end: self.rewrite_expr(end),
            }),
            Expr::Length { collection } => self
                .expr_arena
                .alloc(Expr::Length { collection: self.rewrite_expr(collection) }),
            Expr::Copy { expr } => {
                self.expr_arena.alloc(Expr::Copy { expr: self.rewrite_expr(expr) })
            }
            Expr::Give { value } => {
                self.expr_arena.alloc(Expr::Give { value: self.rewrite_expr(value) })
            }
            Expr::Contains { collection, value } => self.expr_arena.alloc(Expr::Contains {
                collection: self.rewrite_expr(collection),
                value: self.rewrite_expr(value),
            }),
            Expr::Union { left, right } => self.expr_arena.alloc(Expr::Union {
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Intersection { left, right } => self.expr_arena.alloc(Expr::Intersection {
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Range { start, end } => self.expr_arena.alloc(Expr::Range {
                start: self.rewrite_expr(start),
                end: self.rewrite_expr(end),
            }),
            Expr::FieldAccess { object, field } => self.expr_arena.alloc(Expr::FieldAccess {
                object: self.rewrite_expr(object),
                field: *field,
            }),
            Expr::List(items) => self
                .expr_arena
                .alloc(Expr::List(items.iter().map(|i| self.rewrite_expr(i)).collect())),
            Expr::Tuple(items) => self
                .expr_arena
                .alloc(Expr::Tuple(items.iter().map(|i| self.rewrite_expr(i)).collect())),
            Expr::OptionSome { value } => {
                self.expr_arena.alloc(Expr::OptionSome { value: self.rewrite_expr(value) })
            }
            Expr::WithCapacity { value, capacity } => self.expr_arena.alloc(Expr::WithCapacity {
                value: self.rewrite_expr(value),
                capacity: self.rewrite_expr(capacity),
            }),
            Expr::InterpolatedString(parts) => {
                let parts = parts
                    .iter()
                    .map(|p| match p {
                        crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
                            crate::ast::stmt::StringPart::Expr {
                                value: self.rewrite_expr(value),
                                format_spec: *format_spec,
                                debug: *debug,
                            }
                        }
                        crate::ast::stmt::StringPart::Literal(s) => {
                            crate::ast::stmt::StringPart::Literal(*s)
                        }
                    })
                    .collect();
                self.expr_arena.alloc(Expr::InterpolatedString(parts))
            }
            other => other,
        }
    }

    /// Build `coeff * (idx - 1) + offset`, folding the trivial shapes so the
    /// emitted expression is as small as the hand-rewrite (`v * 5`). The full
    /// form is value-identical; the simplifications only drop `* 1`, `+ 0`,
    /// `- 0`, and collapse a constant index outright.
    fn closed_form(&self, info: AffineInfo, idx: &'a Expr<'a>) -> &'a Expr<'a> {
        let AffineInfo { coeff, offset } = info;
        // Constant index: fold the whole closed form to one literal.
        if let Some(k) = const_eval(idx) {
            let v = coeff.wrapping_mul(k.wrapping_sub(1)).wrapping_add(offset);
            return self.num(v);
        }
        // `idx - 1` (0-based position). Fold when idx is itself `e + 1`.
        let pos = self.minus_one(idx);
        // `coeff * pos`.
        let scaled = if coeff == 1 {
            pos
        } else {
            self.bin(BinaryOpKind::Multiply, pos, self.num(coeff))
        };
        // `… + offset`.
        if offset == 0 {
            scaled
        } else {
            self.bin(BinaryOpKind::Add, scaled, self.num(offset))
        }
    }

    /// Build `idx - 1`, collapsing `(e + 1) - 1` → `e` and a constant `k` → `k-1`.
    fn minus_one(&self, idx: &'a Expr<'a>) -> &'a Expr<'a> {
        if let Some(k) = const_eval(idx) {
            return self.num(k - 1);
        }
        if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = idx {
            if const_eval(right) == Some(1) {
                return left;
            }
            if const_eval(left) == Some(1) {
                return right;
            }
        }
        self.bin(BinaryOpKind::Subtract, idx, self.num(1))
    }

    fn num(&self, n: i64) -> &'a Expr<'a> {
        self.expr_arena.alloc(Expr::Literal(Literal::Number(n)))
    }

    fn bin(&self, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
        self.expr_arena.alloc(Expr::BinaryOp { op, left: l, right: r })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::TypeExpr;

    struct B<'a> {
        ea: &'a Arena<Expr<'a>>,
    }
    impl<'a> B<'a> {
        fn id(&self, s: Symbol) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Identifier(s))
        }
        fn num(&self, n: i64) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Literal(Literal::Number(n)))
        }
        fn bin(&self, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
            self.ea.alloc(Expr::BinaryOp { op, left: l, right: r })
        }
        fn index(&self, coll: &'a Expr<'a>, idx: &'a Expr<'a>) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Index { collection: coll, index: idx })
        }
        fn new_seq(&self, seq: Symbol, elem: Symbol) -> &'a Expr<'a> {
            self.ea.alloc(Expr::New {
                type_name: seq,
                type_args: vec![TypeExpr::Primitive(elem)],
                init_fields: vec![],
            })
        }
    }

    fn run<'a>(
        input: Vec<Stmt<'a>>,
        ea: &'a Arena<Expr<'a>>,
        sa: &'a Arena<Stmt<'a>>,
        it: &mut Interner,
    ) -> (Vec<Stmt<'a>>, bool) {
        affine_scalarize_seqs(input, ea, sa, it)
    }

    /// Recursively evaluate a constant-foldable closed-form expression so a test
    /// can assert the EXACT i64 value the substitution produces.
    fn eval(e: &Expr) -> i64 {
        const_eval(e).expect("closed form must be constant-foldable")
    }

    /// Build the graph_bfs shape: an `adjStarts` populated only by `Push i*5`
    /// inside a `while i < n` loop that ALSO pushes to a sibling array, then read
    /// twice as `item (v+1) of adjStarts`. The array and its push must vanish,
    /// the sibling push must survive, and the reads must become `v * 5`.
    #[test]
    fn affine_csr_array_scalarizes_to_closed_form() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let int = it.intern("Int");
        let adj_starts = it.intern("adjStarts");
        let sibling = it.intern("adjCounts");
        let i = it.intern("i");
        let n = it.intern("n");
        let v = it.intern("v");
        let out_var = it.intern("start");
        let b = B { ea: &ea };

        let decl_sibling = Stmt::Let {
            var: sibling,
            ty: None,
            value: b.new_seq(seq, int),
            mutable: true,
        };
        let decl = Stmt::Let {
            var: adj_starts,
            ty: None,
            value: b.new_seq(seq, int),
            mutable: true,
        };
        let init_i = Stmt::Let { var: i, ty: None, value: b.num(0), mutable: true };
        // while i < n: Push i*5 to adjStarts; Push 0 to adjCounts; Set i to i+1
        let push_affine = Stmt::Push {
            value: b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
            collection: b.id(adj_starts),
        };
        let push_sibling = Stmt::Push { value: b.num(0), collection: b.id(sibling) };
        let step = Stmt::Set {
            target: i,
            value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)),
        };
        let body = sa.alloc_slice(vec![push_affine, push_sibling, step]);
        let build = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
            body,
            decreasing: None,
        };
        // Let start be item (v + 1) of adjStarts.
        let read = Stmt::Let {
            var: out_var,
            ty: None,
            value: b.index(b.id(adj_starts), b.bin(BinaryOpKind::Add, b.id(v), b.num(1))),
            mutable: true,
        };

        let input = vec![decl_sibling, decl, init_i, build, read];
        let (out, changed) = run(input, &ea, &sa, &mut it);
        assert!(changed, "affine scalarization should fire");

        // adjStarts' declaration is gone; adjCounts' survives.
        assert!(
            !out.iter().any(|s| matches!(s, Stmt::Let { var, .. } if *var == adj_starts)),
            "adjStarts decl must be deleted"
        );
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let { var, .. } if *var == sibling)),
            "sibling adjCounts decl must survive"
        );

        // The build loop keeps the sibling push and the IV step; the affine push
        // is gone.
        let while_stmt = out.iter().find(|s| matches!(s, Stmt::While { .. })).unwrap();
        let Stmt::While { body, .. } = while_stmt else { unreachable!() };
        assert_eq!(body.len(), 2, "only the affine push is removed");
        assert!(
            !body.iter().any(|s| matches!(s, Stmt::Push { collection, .. }
                if matches!(collection, Expr::Identifier(sy) if *sy == adj_starts))),
            "the affine push must be gone"
        );
        assert!(
            body.iter().any(|s| matches!(s, Stmt::Push { collection, .. }
                if matches!(collection, Expr::Identifier(sy) if *sy == sibling))),
            "the sibling push must survive"
        );

        // The read `item (v+1) of adjStarts` became `v * 5` (coeff 5, offset 0,
        // `(v+1)-1 == v`): a Multiply of `v` by 5, no Index left.
        let read_out = out.last().unwrap();
        let Stmt::Let { value, .. } = read_out else { panic!("expected Let") };
        match value {
            Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right } => {
                assert!(matches!(left, Expr::Identifier(s) if *s == v), "lhs is v");
                assert!(matches!(right, Expr::Literal(Literal::Number(5))), "rhs is 5");
            }
            other => panic!("expected `v * 5`, got {other:?}"),
        }
    }

    /// The closed form must EXACTLY reproduce the pushed values for every
    /// in-range 1-based index — the off-by-one is the whole correctness story.
    /// `Push (i*5 + 3)` makes 0-based element p equal `5p + 3`, so 1-based
    /// `item k` must equal `5*(k-1) + 3`.
    #[test]
    fn closed_form_matches_pushed_values_exactly() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let int = it.intern("Int");
        let arr = it.intern("arr");
        let i = it.intern("i");
        let n = it.intern("n");
        let sink = it.intern("sink");
        let b = B { ea: &ea };

        let decl = Stmt::Let { var: arr, ty: None, value: b.new_seq(seq, int), mutable: true };
        let init_i = Stmt::Let { var: i, ty: None, value: b.num(0), mutable: true };
        // Push (i * 5 + 3) to arr
        let affine = b.bin(
            BinaryOpKind::Add,
            b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
            b.num(3),
        );
        let push = Stmt::Push { value: affine, collection: b.id(arr) };
        let step = Stmt::Set { target: i, value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)) };
        let body = sa.alloc_slice(vec![push, step]);
        let build = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
            body,
            decreasing: None,
        };
        // Read every constant index 1..=4; assert the substituted value equals
        // the value the k-th push computed (element p = k-1 ⟹ 5*(k-1)+3).
        let mut reads = vec![decl, init_i, build];
        for k in 1..=4i64 {
            reads.push(Stmt::Let {
                var: sink,
                ty: None,
                value: b.index(b.id(arr), b.num(k)),
                mutable: true,
            });
        }
        let (out, changed) = run(reads, &ea, &sa, &mut it);
        assert!(changed, "affine scalarization should fire");

        let read_stmts: Vec<_> = out
            .iter()
            .filter(|s| matches!(s, Stmt::Let { var, .. } if *var == sink))
            .collect();
        assert_eq!(read_stmts.len(), 4);
        for (j, s) in read_stmts.iter().enumerate() {
            let k = (j + 1) as i64;
            let Stmt::Let { value, .. } = s else { unreachable!() };
            let expected = 5 * (k - 1) + 3;
            assert_eq!(
                eval(value),
                expected,
                "item {k} of arr must equal 5*({k}-1)+3 = {expected}"
            );
        }
    }

    /// A `SetIndex` write on the array disqualifies it — left untouched.
    #[test]
    fn in_place_write_blocks() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let int = it.intern("Int");
        let arr = it.intern("arr");
        let i = it.intern("i");
        let n = it.intern("n");
        let b = B { ea: &ea };

        let decl = Stmt::Let { var: arr, ty: None, value: b.new_seq(seq, int), mutable: true };
        let init_i = Stmt::Let { var: i, ty: None, value: b.num(0), mutable: true };
        let push = Stmt::Push {
            value: b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
            collection: b.id(arr),
        };
        let step = Stmt::Set { target: i, value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)) };
        let body = sa.alloc_slice(vec![push, step]);
        let build = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
            body,
            decreasing: None,
        };
        // Set item 1 of arr to 99 — an in-place write after the build.
        let write = Stmt::SetIndex { collection: b.id(arr), index: b.num(1), value: b.num(99) };
        let read = Stmt::Let {
            var: it.intern("sink"),
            ty: None,
            value: b.index(b.id(arr), b.num(1)),
            mutable: true,
        };
        let input = vec![decl, init_i, build, write, read];
        let (out, changed) = run(input, &ea, &sa, &mut it);
        assert!(!changed, "an in-place write must block scalarization");
        assert!(out.iter().any(|s| matches!(s, Stmt::Let { var, .. } if *var == arr)));
    }

    /// A `length of arr` query disqualifies — the closed form has no length.
    #[test]
    fn length_query_blocks() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let int = it.intern("Int");
        let arr = it.intern("arr");
        let i = it.intern("i");
        let n = it.intern("n");
        let b = B { ea: &ea };

        let decl = Stmt::Let { var: arr, ty: None, value: b.new_seq(seq, int), mutable: true };
        let init_i = Stmt::Let { var: i, ty: None, value: b.num(0), mutable: true };
        let push = Stmt::Push {
            value: b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
            collection: b.id(arr),
        };
        let step = Stmt::Set { target: i, value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)) };
        let body = sa.alloc_slice(vec![push, step]);
        let build = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
            body,
            decreasing: None,
        };
        let read = Stmt::Let {
            var: it.intern("len"),
            ty: None,
            value: ea.alloc(Expr::Length { collection: b.id(arr) }),
            mutable: true,
        };
        let input = vec![decl, init_i, build, read];
        let (_out, changed) = run(input, &ea, &sa, &mut it);
        assert!(!changed, "a length query must block scalarization");
    }

    /// A non-affine pushed value (`i * i`) disqualifies the array.
    #[test]
    fn non_affine_push_blocks() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let int = it.intern("Int");
        let arr = it.intern("arr");
        let i = it.intern("i");
        let n = it.intern("n");
        let b = B { ea: &ea };

        let decl = Stmt::Let { var: arr, ty: None, value: b.new_seq(seq, int), mutable: true };
        let init_i = Stmt::Let { var: i, ty: None, value: b.num(0), mutable: true };
        let push = Stmt::Push {
            value: b.bin(BinaryOpKind::Multiply, b.id(i), b.id(i)),
            collection: b.id(arr),
        };
        let step = Stmt::Set { target: i, value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)) };
        let body = sa.alloc_slice(vec![push, step]);
        let build = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
            body,
            decreasing: None,
        };
        let read = Stmt::Let {
            var: it.intern("sink"),
            ty: None,
            value: b.index(b.id(arr), b.num(1)),
            mutable: true,
        };
        let input = vec![decl, init_i, build, read];
        let (_out, changed) = run(input, &ea, &sa, &mut it);
        assert!(!changed, "a non-affine push (i*i) must block scalarization");
    }

    /// A bare reference to the array (an alias/escape) disqualifies it.
    #[test]
    fn alias_blocks() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let int = it.intern("Int");
        let arr = it.intern("arr");
        let alias = it.intern("alias");
        let i = it.intern("i");
        let n = it.intern("n");
        let b = B { ea: &ea };

        let decl = Stmt::Let { var: arr, ty: None, value: b.new_seq(seq, int), mutable: true };
        let init_i = Stmt::Let { var: i, ty: None, value: b.num(0), mutable: true };
        let push = Stmt::Push {
            value: b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
            collection: b.id(arr),
        };
        let step = Stmt::Set { target: i, value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)) };
        let body = sa.alloc_slice(vec![push, step]);
        let build = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
            body,
            decreasing: None,
        };
        // Let alias be arr — a bare escape.
        let escape = Stmt::Let { var: alias, ty: None, value: b.id(arr), mutable: true };
        let input = vec![decl, init_i, build, escape];
        let (_out, changed) = run(input, &ea, &sa, &mut it);
        assert!(!changed, "a bare alias must block scalarization");
    }
}
