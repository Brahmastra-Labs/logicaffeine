//! Loop interchange — reorder two adjacent nested counted loops for locality.
//!
//! Reordering loops is something LLVM will not do through the LOGOS `Seq`
//! abstraction. It pays off when it moves the stride-1 (contiguous) access
//! innermost — the classic `ijk`→`ikj` matmul recipe. This pass fires ONLY when
//! the reordering is PROVABLY bit-identical AND improves locality:
//!
//!   * SAFETY — the inner body is a single INTEGER COMMUTATIVE REDUCTION
//!     `Set item IDX of C to (item IDX of C + E) [% M]`, `C` is accessed at NO
//!     other index, and no other collection is written. Such a reduction sums a
//!     value over the two loops' combined iteration space; integer `+` is
//!     commutative and associative (and modular reduction preserves this), so
//!     ANY permutation of the two loops yields the bit-identical result. (A float
//!     reduction would re-associate and is intentionally NOT matched — that is
//!     the deferred fast-math case.)
//!   * PROFIT — a stride-1 cost model: swap only when putting the OUTER variable
//!     innermost yields strictly MORE unit-stride (coefficient-1) array accesses
//!     than the current inner variable. So an already-optimal `ikj` is left
//!     alone; only a suboptimal `ijk` is reordered.
//!
//! Correctness is pinned by a synthetic integer `ijk` matmul whose modular
//! checksum is byte-identical before and after (`loop_interchange_lock`).

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use crate::loop_shape::{extract_counted_while, CountedLoop};

/// Interchange every qualifying adjacent counted-loop pair.
pub fn interchange_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let mut changed = false;
    let out = rewrite_block(stmts, expr_arena, stmt_arena, interner, &mut changed);
    (out, changed)
}

fn rewrite_block<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
) -> Vec<Stmt<'a>> {
    let refs: Vec<&Stmt<'a>> = stmts.iter().collect();
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    let mut idx = 0;
    while idx < refs.len() {
        if let Some((outer, span)) = extract_counted_while(&refs, idx) {
            if let Some(swapped) = try_interchange(&outer, ea, sa) {
                out.push(swapped.0);
                out.push(swapped.1);
                *changed = true;
                idx += span;
                continue;
            }
            // Not interchangeable here: recurse into the loop body and re-emit.
            out.push(refs[idx].clone());
            out.push(recurse_while(refs[idx + 1], ea, sa, it, changed));
            idx += span;
            continue;
        }
        out.push(recurse_stmt(refs[idx].clone(), ea, sa, it, changed));
        idx += 1;
    }
    out
}

fn recurse_stmt<'a>(
    stmt: Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
) -> Stmt<'a> {
    let recur = |b: Block<'a>, it: &mut Interner, c: &mut bool| -> Block<'a> {
        sa.alloc_slice(rewrite_block(b.to_vec(), ea, sa, it, c))
    };
    match stmt {
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond,
            then_block: recur(then_block, it, changed),
            else_block: else_block.map(|b| recur(b, it, changed)),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond,
            body: recur(body, it, changed),
            decreasing,
        },
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => Stmt::Repeat {
            pattern,
            iterable,
            body: recur(body, it, changed),
        },
        other => other,
    }
}

fn recurse_while<'a>(
    while_stmt: &Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
) -> Stmt<'a> {
    if let Stmt::While {
        cond,
        body,
        decreasing,
    } = while_stmt
    {
        Stmt::While {
            cond,
            body: sa.alloc_slice(rewrite_block(body.to_vec(), ea, sa, it, changed)),
            decreasing: *decreasing,
        }
    } else {
        while_stmt.clone()
    }
}

/// Try to interchange the outer counted loop with an inner counted loop nested
/// as its sole body. Returns the swapped `(init, while)` pair.
fn try_interchange<'a>(
    outer: &CountedLoop<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Option<(Stmt<'a>, Stmt<'a>)> {
    // The outer body (minus its increment) must be exactly an inner counted loop.
    let inner_refs: Vec<&Stmt<'a>> = outer.body_without_increment.iter().collect();
    let (inner, inner_span) = extract_counted_while(&inner_refs, 0)?;
    if inner_span != outer.body_without_increment.len() {
        return None; // extra statements around the inner loop — not perfectly nested
    }
    let p = outer.var;
    let q = inner.var;
    // The inner body must be a safe integer commutative reduction.
    let red = safe_reduction(inner.body_without_increment, p, q)?;
    // Cost model: swap only if putting `p` innermost gives strictly more
    // unit-stride accesses than the current inner `q`.
    if stride1_count(&red.accesses, p) <= stride1_count(&red.accesses, q) {
        return None;
    }
    // Emit the swapped nest: `Let q ...; While q { Let p ...; While p { body; p++ }; q++ }`.
    let p_incr = incr(p, ea);
    let q_incr = incr(q, ea);
    let mut inner_body: Vec<Stmt<'a>> = inner.body_without_increment.to_vec();
    inner_body.push(p_incr);
    let new_p_while = Stmt::While {
        cond: cmp(p, outer.limit, outer.inclusive, ea),
        body: sa.alloc_slice(inner_body),
        decreasing: None,
    };
    let p_init = Stmt::Let {
        var: p,
        ty: None,
        value: outer.start,
        mutable: true,
    };
    let new_q_while = Stmt::While {
        cond: cmp(q, inner.limit, inner.inclusive, ea),
        body: sa.alloc_slice(vec![p_init, new_p_while, q_incr]),
        decreasing: None,
    };
    let q_init = Stmt::Let {
        var: q,
        ty: None,
        value: inner.start,
        mutable: true,
    };
    Some((q_init, new_q_while))
}

struct Reduction<'a> {
    /// Every array-index expression accessed in the body (write + reads).
    accesses: Vec<&'a Expr<'a>>,
}

/// Verify the body is a single `Set item IDX of C to (item IDX of C + E) [% M]`
/// integer reduction with `C` accessed only at `IDX` and no other collection
/// written, and collect every index expression for the cost model. Both loop
/// vars must actually appear (else interchange is pointless / ill-defined).
fn safe_reduction<'a>(body: Block<'a>, p: Symbol, q: Symbol) -> Option<Reduction<'a>> {
    if body.len() != 1 {
        return None;
    }
    let Stmt::SetIndex {
        collection,
        index,
        value,
    } = &body[0]
    else {
        return None;
    };
    let Expr::Identifier(c) = &**collection else {
        return None;
    };
    // The value must be `(item IDX of C + E) [% LIT]` — a `+`-reduction of the
    // same cell. Peel an optional `% literal`.
    let core = match value {
        Expr::BinaryOp {
            op: BinaryOpKind::Modulo,
            left,
            right,
        } if is_int_literal(right) => left,
        other => other,
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = &**core
    else {
        return None;
    };
    // One side must be exactly `item IDX of C`; the other is the added value.
    let reads_self = |e: &Expr<'a>| {
        matches!(e, Expr::Index { collection: cc, index: ii }
        if matches!(&**cc, Expr::Identifier(cn) if *cn == *c) && expr_eq(ii, index))
    };
    if !reads_self(left) && !reads_self(right) {
        return None;
    }
    // Collect all index expressions and enforce: `C` appears only as `item IDX
    // of C` (no aliasing read/write), and `C` is the only WRITTEN collection.
    let mut accesses = vec![*index];
    let mut c_only_at_idx = true;
    collect_accesses(value, *c, index, &mut accesses, &mut c_only_at_idx);
    if !c_only_at_idx {
        return None;
    }
    // Both loop variables must occur in some index (a genuine 2-D nest).
    let touches = |v: Symbol| accesses.iter().any(|e| contains(e, v));
    if !touches(p) || !touches(q) {
        return None;
    }
    // No affine-ness requirement: a non-literal coefficient (e.g. `k*n`, the row
    // stride) is fine — `coeff_of` returns `None` there and the cost model simply
    // treats it as NOT unit-stride. Interchange safety rests on the integer
    // commutative reduction, not on affine indices.
    Some(Reduction { accesses })
}

/// Walk `value`, gathering every `item _ of _` index and checking that any
/// access to `c` uses `idx` (structurally). Sets `ok = false` on a violating
/// access or a bare mention of `c`.
fn collect_accesses<'a>(
    e: &'a Expr<'a>,
    c: Symbol,
    idx: &Expr<'a>,
    out: &mut Vec<&'a Expr<'a>>,
    ok: &mut bool,
) {
    match e {
        Expr::Index { collection, index } => {
            out.push(index);
            match &**collection {
                // A bare array NAME in collection position is fine; only flag
                // `c` accessed at a DIFFERENT index (a genuine alias hazard). Do
                // NOT recurse into the name itself — that is not a bare-`c` use.
                Expr::Identifier(cn) => {
                    if *cn == c && !expr_eq(index, idx) {
                        *ok = false;
                    }
                }
                // A NESTED collection (`item x of (item y of g)`) — recurse.
                other => collect_accesses(other, c, idx, out, ok),
            }
            collect_accesses(index, c, idx, out, ok);
        }
        Expr::Identifier(cn) if *cn == c => {
            // A bare `c` OUTSIDE a collection position is an alias hazard.
            *ok = false;
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_accesses(left, c, idx, out, ok);
            collect_accesses(right, c, idx, out, ok);
        }
        Expr::Not { operand } => collect_accesses(operand, c, idx, out, ok),
        _ => {}
    }
}

/// The number of index expressions in which `var` has coefficient exactly 1
/// (unit stride in the flat backing array).
fn stride1_count(accesses: &[&Expr], var: Symbol) -> usize {
    accesses
        .iter()
        .filter(|e| coeff_of(e, var) == Some(1))
        .count()
}

/// The integer coefficient of `var` in an affine index expression, or `None` if
/// the expression is not affine / `var` appears non-linearly.
fn coeff_of(e: &Expr, var: Symbol) -> Option<i64> {
    if !contains(e, var) {
        return Some(0);
    }
    match e {
        Expr::Identifier(s) if *s == var => Some(1),
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => Some(coeff_of(left, var)?.checked_add(coeff_of(right, var)?)?),
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => Some(coeff_of(left, var)?.checked_sub(coeff_of(right, var)?)?),
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left,
            right,
        } => match (contains(left, var), contains(right, var)) {
            (true, false) => Some(coeff_of(left, var)?.checked_mul(int_literal(right)?)?),
            (false, true) => Some(int_literal(left)?.checked_mul(coeff_of(right, var)?)?),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Small builders / helpers
// ---------------------------------------------------------------------------

fn incr<'a>(v: Symbol, ea: &'a Arena<Expr<'a>>) -> Stmt<'a> {
    Stmt::Set {
        target: v,
        value: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: ea.alloc(Expr::Identifier(v)),
            right: ea.alloc(Expr::Literal(Literal::Number(1))),
        }),
    }
}

fn cmp<'a>(
    v: Symbol,
    limit: &'a Expr<'a>,
    inclusive: bool,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    ea.alloc(Expr::BinaryOp {
        op: if inclusive {
            BinaryOpKind::LtEq
        } else {
            BinaryOpKind::Lt
        },
        left: ea.alloc(Expr::Identifier(v)),
        right: limit,
    })
}

fn contains(e: &Expr, var: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == var,
        Expr::BinaryOp { left, right, .. } => contains(left, var) || contains(right, var),
        Expr::Not { operand } => contains(operand, var),
        Expr::Index { collection, index } => contains(collection, var) || contains(index, var),
        Expr::Length { collection } => contains(collection, var),
        _ => false,
    }
}

fn int_literal(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/loop_interchange/tests.rs"]
mod tests;
fn is_int_literal(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::Number(_)))
}

fn expr_eq(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Literal(Literal::Number(x)), Expr::Literal(Literal::Number(y))) => x == y,
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        (
            Expr::BinaryOp {
                op: o1,
                left: l1,
                right: r1,
            },
            Expr::BinaryOp {
                op: o2,
                left: l2,
                right: r2,
            },
        ) => o1 == o2 && expr_eq(l1, l2) && expr_eq(r1, r2),
        (
            Expr::Index {
                collection: c1,
                index: i1,
            },
            Expr::Index {
                collection: c2,
                index: i2,
            },
        ) => expr_eq(c1, c2) && expr_eq(i1, i2),
        _ => false,
    }
}
