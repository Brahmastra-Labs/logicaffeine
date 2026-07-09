//! Loop-carried common-subexpression elimination for the RUN path.
//!
//! Motivation (mandelbrot): the escape loop computes the squared terms of the
//! complex iterate twice per orbit step. With `zr`/`zi` the real/imaginary
//! parts:
//!
//! ```text
//! While iter < 50:
//!     Let zr2 be zr * zr - zi * zi + cr.   # uses the OLD zr, zi
//!     Let zi2 be 2.0 * zr * zi + ci.
//!     Set zr to zr2.
//!     Set zi to zi2.
//!     If zr * zr + zi * zi > 4.0:          # uses the NEW zr, zi
//!         ...
//!     Set iter to iter + 1.
//! ```
//!
//! The products `zr * zr` and `zi * zi` are evaluated in the EXIT GUARD over the
//! freshly assigned (new) `zr`/`zi`, and then RE-evaluated at the top of the very
//! next iteration over the same values — because the next iteration's OLD `zr` is
//! this iteration's NEW `zr`. They are a loop-carried CSE across the back-edge.
//!
//! This pass introduces one fresh loop-carried temp per recomputed square: it is
//! seeded before the loop with the square of the operand's entry value, assigned
//! `o * o` ONCE inside the body right after the last `Set` of `o`, and substituted
//! for the product in BOTH the exit guard and the body's earlier use. The square
//! is then computed once per iteration instead of twice (the hand-validated form
//! is mandelbrot ~1.19× faster, bit-identical).
//!
//! **The exact pattern matched (narrow and sound).** For a top-level `While`
//! whose guard is an `If <E> {is greater/less than} <bound>` *inside the body*
//! (the loop's escape test), this pass fires on every sub-expression `E = o * o`
//! where:
//!
//! 1. `o` is a single loop-carried scalar `Identifier` (the body holds exactly
//!    one `Set o to …`, and that `Set`'s right-hand side does not itself read `o`
//!    — `o`'s new value is independent of its old value, so the guard's `o * o`
//!    over the new value equals the next iteration's `o * o` over the same value);
//! 2. `o * o` (syntactically: a `Multiply` of `Identifier(o)` with itself) appears
//!    in that exit guard's compared expression, AND also appears in the loop body
//!    BEFORE the `Set o to …` (the recomputation we elide);
//! 3. between `Set o to …` and the end of the iteration, `o` is not reassigned
//!    again (no second `Set o` / shadowing `Let o`) — so the value the temp
//!    captures is exactly what both uses observe;
//! 4. immediately before the loop, `o` is introduced by `Let [mutable] o be <lit>`
//!    with a numeric (Int/Float) literal, so the temp's pre-loop seed `o * o`
//!    is a known constant — if the entry value cannot be determined this way the
//!    square is SKIPPED.
//!
//! **Value-preserving.** `o * o` is pure (no calls, indexing, or division). The
//! temp is assigned exactly the product the guard would have computed, at the same
//! point in the iteration, and read in place of every recomputation; no operation
//! is reordered relative to a write of `o`. The pre-loop seed reifies the same
//! product over the entry literal. Every condition is conservative: any shape not
//! matched leaves the loop untouched.

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, MatchArm, Stmt};
use crate::intern::{Interner, Symbol};

/// A recognized loop-carried square `o * o`: the operand, the fresh temp it is
/// hoisted into, and the temp's pre-loop seed value (the square of `o`'s entry
/// literal).
#[derive(Clone, Debug)]
struct CarriedSquare {
    operand: Symbol,
    temp: Symbol,
    seed: Literal,
}

/// Apply loop-carried CSE to every qualifying top-level `While` in `stmts`.
/// Returns the (possibly) rewritten statements and whether anything changed. On a
/// program with no qualifying loop the ORIGINAL statements are returned untouched.
pub fn loop_carried_cse_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let mut changed = false;
    let out = rewrite_block(stmts, expr_arena, stmt_arena, interner, &mut changed);
    (out, changed)
}

/// Rewrite a statement list: recurse into every nested block, and at each level
/// try to fire the transform on any `While` it contains.
fn rewrite_block<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    let mut idx = 0;
    while idx < stmts.len() {
        // Peek the upcoming statements so a `While` can see the `Let o be 0.0`
        // declarations that precede it (its operands' entry values).
        let prefix: &[Stmt<'a>] = &stmts[..idx];
        let stmt = &stmts[idx];
        if let Stmt::While { cond, body, decreasing } = stmt {
            if let Some(squares) = detect_carried_squares(body, &out, prefix, it) {
                let new_body =
                    build_body_with_temps(body, &squares, ea, sa, it, changed);
                let new_cond = substitute_squares_expr(cond, &squares, ea);
                // Seed each temp immediately before the loop.
                for sq in &squares {
                    out.push(Stmt::Let {
                        var: sq.temp,
                        ty: None,
                        value: ea.alloc(Expr::Literal(sq.seed.clone())),
                        mutable: true,
                    });
                }
                out.push(Stmt::While {
                    cond: new_cond,
                    body: new_body,
                    decreasing: *decreasing,
                });
                *changed = true;
                idx += 1;
                continue;
            }
        }
        // Not a firing `While`: recurse into nested blocks and copy through.
        let owned = stmts[idx].clone();
        out.push(recurse_stmt(owned, ea, sa, it, changed));
        idx += 1;
    }
    out
}

/// Recurse the transform into a statement's nested blocks without firing on the
/// statement itself.
fn recurse_stmt<'a>(
    stmt: Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
) -> Stmt<'a> {
    let recur = |block: &'a [Stmt<'a>], it: &mut Interner, changed: &mut bool| -> &'a [Stmt<'a>] {
        let v = rewrite_block(block.to_vec(), ea, sa, it, changed);
        sa.alloc_slice(v)
    };
    match stmt {
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond,
            then_block: recur(then_block, it, changed),
            else_block: else_block.map(|b| recur(b, it, changed)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond,
            body: recur(body, it, changed),
            decreasing,
        },
        Stmt::Repeat { pattern, iterable, body } => Stmt::Repeat {
            pattern,
            iterable,
            body: recur(body, it, changed),
        },
        Stmt::Inspect { target, arms, has_otherwise } => {
            let arms = arms
                .into_iter()
                .map(|a| MatchArm {
                    enum_name: a.enum_name,
                    variant: a.variant,
                    bindings: a.bindings,
                    body: recur(a.body, it, changed),
                })
                .collect();
            Stmt::Inspect { target, arms, has_otherwise }
        }
        Stmt::Zone { name, capacity, source_file, body } => Stmt::Zone {
            name,
            capacity,
            source_file,
            body: recur(body, it, changed),
        },
        Stmt::Concurrent { tasks } => Stmt::Concurrent { tasks: recur(tasks, it, changed) },
        Stmt::Parallel { tasks } => Stmt::Parallel { tasks: recur(tasks, it, changed) },
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Recognize every loop-carried square `o * o` for the `While` whose body is
/// `body` and whose preceding statements are `before` (already-emitted output of
/// this level) and `prefix` (the not-yet-emitted remainder of the source level).
/// Returns `None` if the loop is not a qualifying escape loop or no square fires.
fn detect_carried_squares(
    body: &[Stmt],
    before: &[Stmt],
    prefix: &[Stmt],
    it: &mut Interner,
) -> Option<Vec<CarriedSquare>> {
    // Find the loop's exit guard: a top-level `If <E> {Lt|Gt|LtEq|GtEq} <bound>`
    // in the body. The compared expression `E` is where the guard squares live.
    let guard_expr = body.iter().find_map(|s| match s {
        Stmt::If { cond, .. } => exit_guard_compared(cond),
        _ => None,
    })?;

    // Candidate operands: every `o` for which `o * o` appears in the guard.
    let mut candidates: Vec<Symbol> = Vec::new();
    collect_self_squares(guard_expr, &mut candidates);
    candidates.dedup();
    if candidates.is_empty() {
        return None;
    }

    let mut squares: Vec<CarriedSquare> = Vec::new();
    for o in candidates {
        if let Some(sq) = qualify_square(o, body, before, prefix, it) {
            squares.push(sq);
        }
    }
    if squares.is_empty() {
        None
    } else {
        Some(squares)
    }
}

/// The compared expression of a loop escape guard: the left operand of a
/// `<E> {Lt|Gt|LtEq|GtEq} <bound>` comparison (mandelbrot's `zr*zr + zi*zi > 4.0`).
fn exit_guard_compared<'e>(cond: &'e Expr<'e>) -> Option<&'e Expr<'e>> {
    match cond {
        Expr::BinaryOp { op, left, .. }
            if matches!(
                op,
                BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq
            ) =>
        {
            Some(left)
        }
        _ => None,
    }
}

/// Collect every operand `o` such that the pure sub-expression `o * o` (a
/// `Multiply` of `Identifier(o)` with the identical identifier) occurs in `e`.
/// Only descends through pure `+ - *` arithmetic and `Not`; a call/index/division
/// anywhere in a branch makes that branch non-pure and it is not searched.
fn collect_self_squares(e: &Expr, out: &mut Vec<Symbol>) {
    if let Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right } = e {
        if let (Expr::Identifier(a), Expr::Identifier(b)) = (left, right) {
            if a == b {
                out.push(*a);
            }
        }
    }
    match e {
        Expr::BinaryOp { op, left, right }
            if matches!(op, BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply) =>
        {
            collect_self_squares(left, out);
            collect_self_squares(right, out);
        }
        Expr::Not { operand } => collect_self_squares(operand, out),
        _ => {}
    }
}

/// Verify operand `o` carries a sound square and recover its temp + seed.
fn qualify_square(
    o: Symbol,
    body: &[Stmt],
    before: &[Stmt],
    prefix: &[Stmt],
    it: &mut Interner,
) -> Option<CarriedSquare> {
    // (1) `o` is loop-carried: exactly one `Set o to …` in the body, and its
    // right-hand side does not read `o` (the new value is independent of the old
    // — so the guard's `o*o` over the new value equals the next iteration's).
    let mut set_positions: Vec<usize> = Vec::new();
    for (i, s) in body.iter().enumerate() {
        match s {
            Stmt::Set { target, value } if *target == o => {
                if expr_reads(value, o) {
                    return None;
                }
                set_positions.push(i);
            }
            // A shadowing `Let o` inside the body breaks the carried identity.
            Stmt::Let { var, .. } if *var == o => return None,
            _ => {}
        }
    }
    if set_positions.len() != 1 {
        return None;
    }
    let set_pos = set_positions[0];

    // (3) After the `Set o`, `o` is never reassigned again before the end of the
    // iteration — guaranteed by `set_positions.len() == 1`, but a nested
    // reassignment (inside an `If`/`While`) would also break the identity.
    if body.iter().any(|s| nested_assigns_var(s, o)) {
        return None;
    }

    // (2) `o * o` is recomputed in the body BEFORE the `Set o`. We require the
    // square to appear in a statement strictly before `set_pos` (the old-value
    // recomputation this pass elides). If it appears only in the guard there is no
    // CSE to capture, so we skip — a hoist with no second use is pure overhead.
    let recomputed_before = body[..set_pos]
        .iter()
        .any(|s| stmt_has_self_square(s, o));
    if !recomputed_before {
        return None;
    }

    // (4) Entry value: the `Let [mutable] o be <numeric literal>` that
    // immediately introduces `o` before the loop (searched across both halves of
    // the surrounding level). The seed is `o_entry * o_entry`.
    let entry = entry_literal(o, before, prefix)?;
    let seed = square_literal(&entry)?;

    let temp = it.intern(&format!("__lcse_{}_sq", it.resolve(o)));
    // A defensive guard against the temp colliding with the operand or an
    // existing program variable that already happens to be live in the loop.
    if temp == o || body.iter().any(|s| stmt_assigns(s, temp)) {
        return None;
    }

    Some(CarriedSquare { operand: o, temp, seed })
}

/// The most recent `Let o be <lit>` for `o` among the statements preceding the
/// loop, with a numeric (Int/Float) literal right-hand side.
fn entry_literal(o: Symbol, before: &[Stmt], prefix: &[Stmt]) -> Option<Literal> {
    let scan = |stmts: &[Stmt]| -> Option<Literal> {
        let mut found = None;
        for s in stmts {
            if let Stmt::Let { var, value, .. } = s {
                if *var == o {
                    found = match value {
                        Expr::Literal(l @ (Literal::Number(_) | Literal::Float(_))) => {
                            Some(l.clone())
                        }
                        _ => None,
                    };
                }
            } else if stmt_assigns(s, o) {
                // A `Set o` before the loop without a literal `Let` makes the
                // entry value unknown — conservatively unknown.
                found = None;
            }
        }
        found
    };
    // `before` (already-emitted output) holds the seeds the pass just inserted
    // plus original statements; `prefix` is the source remainder. The operand's
    // declaration is in one of them; prefer the latest match across both.
    scan(prefix).or_else(|| scan(before))
}

/// The square of a numeric literal, preserving its type (Int → Int, Float → Float).
fn square_literal(l: &Literal) -> Option<Literal> {
    match l {
        Literal::Number(n) => n.checked_mul(*n).map(Literal::Number),
        Literal::Float(f) => Some(Literal::Float(f * f)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Body / expression rewriting
// ---------------------------------------------------------------------------

/// Rebuild the loop body: substitute `o * o` → `t` everywhere, and insert
/// `Set t to o * o` immediately after the (sole) `Set o to …` for each carried
/// square. Nested blocks are also recursed for the inner transform.
fn build_body_with_temps<'a>(
    body: &'a [Stmt<'a>],
    squares: &[CarriedSquare],
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
) -> &'a [Stmt<'a>] {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(body.len() + squares.len());
    for s in body {
        // Recurse the outer transform into nested blocks first, then substitute
        // squares in this statement's own expressions.
        let recursed = recurse_stmt(s.clone(), ea, sa, it, changed);
        let substituted = substitute_squares_stmt(recursed, squares, ea, sa);
        out.push(substituted);
        if let Stmt::Set { target, .. } = s {
            for sq in squares.iter().filter(|sq| sq.operand == *target) {
                let prod = ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Multiply,
                    left: ea.alloc(Expr::Identifier(sq.operand)),
                    right: ea.alloc(Expr::Identifier(sq.operand)),
                });
                out.push(Stmt::Set { target: sq.temp, value: prod });
            }
        }
    }
    sa.alloc_slice(out)
}

/// Substitute `o * o` → `Identifier(t)` in every expression of a statement.
fn substitute_squares_stmt<'a>(
    stmt: Stmt<'a>,
    squares: &[CarriedSquare],
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    let sub = |e: &'a Expr<'a>| substitute_squares_expr(e, squares, ea);
    let sub_block = |block: &'a [Stmt<'a>]| -> &'a [Stmt<'a>] {
        let v: Vec<Stmt<'a>> = block
            .iter()
            .map(|s| substitute_squares_stmt(s.clone(), squares, ea, sa))
            .collect();
        sa.alloc_slice(v)
    };
    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            Stmt::Let { var, ty, value: sub(value), mutable }
        }
        Stmt::Set { target, value } => Stmt::Set { target, value: sub(value) },
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond: sub(cond),
            then_block: sub_block(then_block),
            else_block: else_block.map(sub_block),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond: sub(cond),
            body: sub_block(body),
            decreasing: decreasing.map(sub),
        },
        Stmt::Return { value } => Stmt::Return { value: value.map(sub) },
        Stmt::RuntimeAssert { condition, hard } => Stmt::RuntimeAssert { condition: sub(condition) , hard },
        Stmt::Show { object, recipient } => {
            Stmt::Show { object: sub(object), recipient: sub(recipient) }
        }
        // Statements whose own expressions cannot contain a carried square (the
        // guard/recomputation live in Let/Set/If) are passed through unchanged.
        other => other,
    }
}

/// Substitute every `o * o` sub-expression (for a carried operand) with its temp
/// identifier, descending through pure `+ - *` arithmetic and comparisons.
fn substitute_squares_expr<'a>(
    e: &'a Expr<'a>,
    squares: &[CarriedSquare],
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    if let Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right } = e {
        if let (Expr::Identifier(a), Expr::Identifier(b)) = (left, right) {
            if a == b {
                if let Some(sq) = squares.iter().find(|sq| sq.operand == *a) {
                    return ea.alloc(Expr::Identifier(sq.temp));
                }
            }
        }
    }
    match e {
        Expr::BinaryOp { op, left, right } => ea.alloc(Expr::BinaryOp {
            op: *op,
            left: substitute_squares_expr(left, squares, ea),
            right: substitute_squares_expr(right, squares, ea),
        }),
        Expr::Not { operand } => {
            ea.alloc(Expr::Not { operand: substitute_squares_expr(operand, squares, ea) })
        }
        _ => e,
    }
}

// ---------------------------------------------------------------------------
// Small scans
// ---------------------------------------------------------------------------

/// `o * o` (a self-multiply of an identifier) occurs anywhere in `s`'s own
/// expressions (top level only — nested-block recomputations are not the pattern).
fn stmt_has_self_square(s: &Stmt, o: Symbol) -> bool {
    let mut found = false;
    let mut check = |e: &Expr| {
        if expr_has_self_square(e, o) {
            found = true;
        }
    };
    match s {
        Stmt::Let { value, .. } => check(value),
        Stmt::Set { value, .. } => check(value),
        Stmt::If { cond, .. } => check(cond),
        Stmt::While { cond, .. } => check(cond),
        Stmt::Return { value: Some(v) } => check(v),
        Stmt::RuntimeAssert { condition, .. } => check(condition),
        _ => {}
    }
    found
}

/// `o * o` occurs anywhere in the expression tree.
fn expr_has_self_square(e: &Expr, o: Symbol) -> bool {
    if let Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right } = e {
        if let (Expr::Identifier(a), Expr::Identifier(b)) = (left, right) {
            if *a == o && *b == o {
                return true;
            }
        }
    }
    match e {
        Expr::BinaryOp { left, right, .. } => {
            expr_has_self_square(left, o) || expr_has_self_square(right, o)
        }
        Expr::Not { operand } => expr_has_self_square(operand, o),
        _ => false,
    }
}

/// `o` is read anywhere in `e` (as a bare identifier).
fn expr_reads(e: &Expr, o: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == o,
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => expr_reads(left, o) || expr_reads(right, o),
        Expr::Not { operand } => expr_reads(operand, o),
        Expr::Index { collection, index } => expr_reads(collection, o) || expr_reads(index, o),
        Expr::Slice { collection, start, end } => {
            expr_reads(collection, o) || expr_reads(start, o) || expr_reads(end, o)
        }
        Expr::Length { collection } => expr_reads(collection, o),
        Expr::Copy { expr } | Expr::Give { value: expr } | Expr::OptionSome { value: expr } => {
            expr_reads(expr, o)
        }
        Expr::Contains { collection, value } => {
            expr_reads(collection, o) || expr_reads(value, o)
        }
        Expr::FieldAccess { object, .. } => expr_reads(object, o),
        Expr::Call { args, .. } => args.iter().any(|a| expr_reads(a, o)),
        Expr::CallExpr { callee, args } => {
            expr_reads(callee, o) || args.iter().any(|a| expr_reads(a, o))
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|a| expr_reads(a, o)),
        Expr::WithCapacity { value, capacity } => {
            expr_reads(value, o) || expr_reads(capacity, o)
        }
        _ => false,
    }
}

/// `s` is a top-level `Let`/`Set` that assigns `v`.
fn stmt_assigns(s: &Stmt, v: Symbol) -> bool {
    matches!(s, Stmt::Let { var, .. } if *var == v)
        || matches!(s, Stmt::Set { target, .. } if *target == v)
}

/// `v` is assigned inside a nested block (an `If`/`While`/`Repeat`/… body) of `s`.
/// Top-level assignments are not nested — they are handled by `set_positions`.
fn nested_assigns_var(s: &Stmt, v: Symbol) -> bool {
    fn block_assigns(block: &[Stmt], v: Symbol) -> bool {
        block.iter().any(|s| stmt_assigns(s, v) || nested_assigns_var(s, v))
    }
    match s {
        Stmt::If { then_block, else_block, .. } => {
            block_assigns(then_block, v)
                || else_block.as_ref().map_or(false, |b| block_assigns(b, v))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => block_assigns(body, v),
        Stmt::Inspect { arms, .. } => arms.iter().any(|a| block_assigns(a.body, v)),
        Stmt::Zone { body, .. }
        | Stmt::Concurrent { tasks: body }
        | Stmt::Parallel { tasks: body } => block_assigns(body, v),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct B<'a> {
        ea: &'a Arena<Expr<'a>>,
    }
    impl<'a> B<'a> {
        fn id(&self, s: Symbol) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Identifier(s))
        }
        fn fl(&self, f: f64) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Literal(Literal::Float(f)))
        }
        fn num(&self, n: i64) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Literal(Literal::Number(n)))
        }
        fn bin(&self, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
            self.ea.alloc(Expr::BinaryOp { op, left: l, right: r })
        }
        fn sq(&self, s: Symbol) -> &'a Expr<'a> {
            self.bin(BinaryOpKind::Multiply, self.id(s), self.id(s))
        }
    }

    /// Count `Set <temp> to <op> * <op>` statements in a block.
    fn count_square_assigns(block: &[Stmt], temp: Symbol) -> usize {
        block
            .iter()
            .filter(|s| matches!(s, Stmt::Set { target, value }
                if *target == temp
                && matches!(value, Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right }
                    if matches!((left, right), (Expr::Identifier(a), Expr::Identifier(b)) if a == b))))
            .count()
    }

    /// The ONLY surviving `o * o` in the block is the single `Set <temp> to o*o`
    /// the pass inserts — no other statement recomputes the square.
    fn only_square_is_temp_assign(block: &[Stmt], o: Symbol, temp: Symbol) -> bool {
        block.iter().all(|s| {
            if matches!(s, Stmt::Set { target, .. } if *target == temp) {
                true
            } else {
                !stmt_has_self_square(s, o)
            }
        })
    }

    /// Build mandelbrot's escape loop and assert the temp is introduced once per
    /// iteration and every `zr*zr` / `zi*zi` recomputation is replaced.
    #[test]
    fn mandelbrot_escape_loop_hoists_squares() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let zr = it.intern("zr");
        let zi = it.intern("zi");
        let cr = it.intern("cr");
        let ci = it.intern("ci");
        let zr2 = it.intern("zr2");
        let zi2 = it.intern("zi2");
        let iter = it.intern("iter");
        let is_inside = it.intern("isInside");
        let b = B { ea: &ea };

        // Pre-loop: Let mutable zr be 0.0.  Let mutable zi be 0.0.
        let decl_zr = Stmt::Let { var: zr, ty: None, value: b.fl(0.0), mutable: true };
        let decl_zi = Stmt::Let { var: zi, ty: None, value: b.fl(0.0), mutable: true };
        let decl_iter = Stmt::Let { var: iter, ty: None, value: b.num(0), mutable: true };

        // Body.
        // Let zr2 be zr*zr - zi*zi + cr.
        let let_zr2 = Stmt::Let {
            var: zr2,
            ty: None,
            value: b.bin(
                BinaryOpKind::Add,
                b.bin(BinaryOpKind::Subtract, b.sq(zr), b.sq(zi)),
                b.id(cr),
            ),
            mutable: false,
        };
        // Let zi2 be 2.0 * zr * zi + ci.
        let let_zi2 = Stmt::Let {
            var: zi2,
            ty: None,
            value: b.bin(
                BinaryOpKind::Add,
                b.bin(
                    BinaryOpKind::Multiply,
                    b.bin(BinaryOpKind::Multiply, b.fl(2.0), b.id(zr)),
                    b.id(zi),
                ),
                b.id(ci),
            ),
            mutable: false,
        };
        let set_zr = Stmt::Set { target: zr, value: b.id(zr2) };
        let set_zi = Stmt::Set { target: zi, value: b.id(zi2) };
        // If zr*zr + zi*zi > 4.0: Set isInside to 0. Set iter to 50.
        let guard = Stmt::If {
            cond: b.bin(
                BinaryOpKind::Gt,
                b.bin(BinaryOpKind::Add, b.sq(zr), b.sq(zi)),
                b.fl(4.0),
            ),
            then_block: sa.alloc_slice(vec![
                Stmt::Set { target: is_inside, value: b.num(0) },
                Stmt::Set { target: iter, value: b.num(50) },
            ]),
            else_block: None,
        };
        let step = Stmt::Set {
            target: iter,
            value: b.bin(BinaryOpKind::Add, b.id(iter), b.num(1)),
        };
        let body = sa.alloc_slice(vec![
            let_zr2, let_zi2, set_zr, set_zi, guard, step,
        ]);
        let loop_stmt = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(iter), b.num(50)),
            body,
            decreasing: None,
        };

        let input = vec![decl_zr, decl_zi, decl_iter, loop_stmt];
        let (out, changed) = loop_carried_cse_stmts(input, &ea, &sa, &mut it);
        assert!(changed, "the transform must fire on mandelbrot's escape loop");

        let zr_sq = it.intern("__lcse_zr_sq");
        let zi_sq = it.intern("__lcse_zi_sq");

        // Two seeds (zrSq=0.0, ziSq=0.0) precede the loop.
        let seeds: Vec<&Stmt> = out
            .iter()
            .filter(|s| matches!(s, Stmt::Let { var, value, .. }
                if (*var == zr_sq || *var == zi_sq)
                && matches!(value, Expr::Literal(Literal::Float(f)) if *f == 0.0)))
            .collect();
        assert_eq!(seeds.len(), 2, "both squared-temp seeds must precede the loop");

        // The loop body: each temp is assigned exactly once, and no `o*o`
        // recomputation survives anywhere in the body.
        let Stmt::While { body, cond, .. } = out.last().unwrap() else {
            panic!("the last statement must remain the While loop");
        };
        assert_eq!(count_square_assigns(body, zr_sq), 1, "zr*zr computed once per iter");
        assert_eq!(count_square_assigns(body, zi_sq), 1, "zi*zi computed once per iter");
        assert!(
            only_square_is_temp_assign(body, zr, zr_sq),
            "the only zr*zr left is the single temp assignment"
        );
        assert!(
            only_square_is_temp_assign(body, zi, zi_sq),
            "the only zi*zi left is the single temp assignment"
        );

        // The exit guard must now read the temps, not recompute the products.
        // (The `cond` is the loop guard `iter < 50`; the escape `If` is inside the
        // body — assert both the body's escape If reads the temps.)
        let _ = cond;
        let escape = body
            .iter()
            .find_map(|s| match s {
                Stmt::If { cond, .. } => Some(*cond),
                _ => None,
            })
            .expect("escape guard remains");
        // The compared expression is now `__lcse_zr_sq + __lcse_zi_sq`.
        assert!(
            matches!(escape, Expr::BinaryOp { op: BinaryOpKind::Gt, left, .. }
                if matches!(left, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                    if matches!((left, right),
                        (Expr::Identifier(a), Expr::Identifier(c)) if *a == zr_sq && *c == zi_sq))),
            "the escape guard must compare the hoisted temps"
        );
    }

    /// A loop whose carried operand's new value READS its old value (`Set o to
    /// o + 1`) must NOT fire — the guard's `o*o` over the new value differs from
    /// the next iteration's recomputation point in general, and the narrow square
    /// identity is only sound when the new value is independent of the old.
    #[test]
    fn self_referential_update_does_not_fire() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let o = it.intern("o");
        let acc = it.intern("acc");
        let b = B { ea: &ea };

        let decl = Stmt::Let { var: o, ty: None, value: b.num(0), mutable: true };
        // Let acc be o*o.   Set o to o + 1.   If o*o > 100: ...
        let recompute = Stmt::Let {
            var: acc,
            ty: None,
            value: b.sq(o),
            mutable: false,
        };
        let step = Stmt::Set {
            target: o,
            value: b.bin(BinaryOpKind::Add, b.id(o), b.num(1)),
        };
        let guard = Stmt::If {
            cond: b.bin(BinaryOpKind::Gt, b.sq(o), b.num(100)),
            then_block: sa.alloc_slice(vec![Stmt::Break]),
            else_block: None,
        };
        let body = sa.alloc_slice(vec![recompute, step, guard]);
        let loop_stmt = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(o), b.num(1000)),
            body,
            decreasing: None,
        };
        let (_, changed) =
            loop_carried_cse_stmts(vec![decl, loop_stmt], &ea, &sa, &mut it);
        assert!(!changed, "must not fire when the operand's update reads itself");
    }

    /// When the operand's entry value is not a known literal, the seed cannot be
    /// formed and the square is skipped (conservative).
    #[test]
    fn unknown_entry_value_skips() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let o = it.intern("o");
        let src = it.intern("src");
        let nv = it.intern("nv");
        let acc = it.intern("acc");
        let b = B { ea: &ea };

        // Let o be src.   (entry value unknown)
        let decl = Stmt::Let { var: o, ty: None, value: b.id(src), mutable: true };
        let recompute = Stmt::Let { var: acc, ty: None, value: b.sq(o), mutable: false };
        let set = Stmt::Set { target: o, value: b.id(nv) };
        let guard = Stmt::If {
            cond: b.bin(BinaryOpKind::Gt, b.sq(o), b.num(4)),
            then_block: sa.alloc_slice(vec![Stmt::Break]),
            else_block: None,
        };
        let body = sa.alloc_slice(vec![recompute, set, guard]);
        let loop_stmt = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(o), b.num(10)),
            body,
            decreasing: None,
        };
        let (_, changed) =
            loop_carried_cse_stmts(vec![decl, loop_stmt], &ea, &sa, &mut it);
        assert!(!changed, "must not fire when the entry value is not a known literal");
    }
}
