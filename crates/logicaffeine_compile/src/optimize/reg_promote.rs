//! Aggregate-element register promotion (store-to-load forwarding across a loop).
//!
//! A loop that read-modify-writes ONE array element at a loop-INVARIANT index —
//! `Set item IDX of C to f(item IDX of C, …)` with `IDX` unchanging across the
//! loop — reloads and restores that element from the heap every iteration. The
//! element is loaded into a scalar ONCE before the loop, used and updated in a
//! register throughout, and written back ONCE after. LICM cannot do this: the
//! element is WRITTEN, so it is not loop-invariant; this is store-to-load
//! forwarding, not invariant hoisting.
//!
//! ## Soundness — the preconditions
//!
//! Promotion is sound only when the forwarded scalar can never disagree with the
//! backing element:
//!   * `IDX` is loop-invariant (`licm::is_loop_invariant`), so it names the same
//!     cell every iteration;
//!   * EVERY access to `C` in the loop is `item IDX of C` (structurally, via
//!     `licm::loads_equal`) — no read or write at a different, possibly-aliasing
//!     index, and `C` never appears "bare" (passed to a call, `Give`n, aliased by
//!     a `Let`, or `Push`/`Pop`/`Add`/`Remove`d), any of which could observe the
//!     stale in-memory value;
//!   * the loop body has no `Break`/`Return` (the write-back after the loop would
//!     be skipped, leaving the element stale);
//!   * the RMW is at the loop body's top level (unconditional), so if the loop
//!     runs at all the element is definitely touched — the pre-load can never
//!     fault where the original would not; and
//!   * the loop condition is side-effect-free, so the zero-trip `If` guard may
//!     evaluate it a second time.
//!
//! The whole load/loop/write-back is wrapped in `If <cond> { … }` so a zero-trip
//! loop neither loads nor stores — byte-identical to the untouched source.

use super::licm::{collect_loop_writes, is_loop_invariant, is_side_effect_free, loads_equal};
use super::writes::{CalleeGlobals, WriteSet};
use crate::arena::Arena;
use crate::ast::stmt::{Block, Expr, Stmt};
use crate::intern::{Interner, Symbol};

/// Apply register promotion to every qualifying loop.
pub fn reg_promote_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let mut changed = false;
    let globals = CalleeGlobals::of_program(&stmts);
    let out = rewrite_block(
        stmts,
        expr_arena,
        stmt_arena,
        interner,
        &mut changed,
        &globals,
    );
    (out, changed)
}

fn rewrite_block<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
    globals: &CalleeGlobals,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        if let Stmt::While {
            cond,
            body,
            decreasing,
        } = &stmt
        {
            if let Some(promoted) = try_promote(cond, body, *decreasing, ea, sa, it, globals) {
                out.push(promoted);
                *changed = true;
                continue;
            }
        }
        out.push(recurse_stmt(stmt, ea, sa, it, changed, globals));
    }
    out
}

fn recurse_stmt<'a>(
    stmt: Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
    globals: &CalleeGlobals,
) -> Stmt<'a> {
    let recur = |block: Block<'a>, it: &mut Interner, changed: &mut bool| -> Block<'a> {
        sa.alloc_slice(rewrite_block(block.to_vec(), ea, sa, it, changed, globals))
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

/// Try to promote an invariant-indexed RMW element in `body`. Returns the
/// guarded `If { load; While; write-back }` replacement, or `None`.
fn try_promote<'a>(
    cond: &'a Expr<'a>,
    body: Block<'a>,
    decreasing: Option<&'a Expr<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    globals: &CalleeGlobals,
) -> Option<Stmt<'a>> {
    if !is_side_effect_free(cond) || contains_break_or_return(body) {
        return None;
    }
    let writes = collect_loop_writes(body, globals);
    // Find a top-level RMW `Set item IDX of C to f(item IDX of C, …)`.
    let (coll_sym, idx) = find_rmw(body, &writes)?;
    // The collection binding must keep naming the same backing value throughout
    // the loop. Rebinding or exchanging it makes the cached element stale.
    if writes.is_opaque()
        || writes.assigned().contains(&coll_sym)
        || writes.declared().contains(&coll_sym)
    {
        return None;
    }
    // Every occurrence of `C` in the body must be `item IDX of C`.
    if !c_only_at_index_block(body, coll_sym, idx) {
        return None;
    }
    // Fresh accumulator; load, rewrite, write back.
    let cname = it.resolve(coll_sym).to_string();
    let acc = it.intern(&format!("__rp_{cname}"));
    let load_expr = ea.alloc(Expr::Index {
        collection: ea.alloc(Expr::Identifier(coll_sym)),
        index: idx,
    });
    let load = Stmt::Let {
        var: acc,
        ty: None,
        value: load_expr,
        mutable: true,
    };
    let new_body: Vec<Stmt<'a>> = body
        .iter()
        .map(|s| rewrite_stmt(s, coll_sym, idx, acc, ea, sa))
        .collect();
    let new_while = Stmt::While {
        cond,
        body: sa.alloc_slice(new_body),
        decreasing,
    };
    let writeback = Stmt::SetIndex {
        collection: ea.alloc(Expr::Identifier(coll_sym)),
        index: idx,
        value: ea.alloc(Expr::Identifier(acc)),
    };
    let guarded_body: Block<'a> = sa.alloc_slice(vec![load, new_while, writeback]);
    Some(Stmt::If {
        cond,
        then_block: guarded_body,
        else_block: None,
    })
}

/// The `(C, IDX)` of the first top-level `Set item IDX of C to <… item IDX of C …>`
/// (a read-modify-write) with `IDX` loop-invariant.
fn find_rmw<'a>(body: &[Stmt<'a>], writes: &WriteSet) -> Option<(Symbol, &'a Expr<'a>)> {
    for s in body {
        if let Stmt::SetIndex {
            collection,
            index,
            value,
        } = s
        {
            if let Expr::Identifier(c) = &**collection {
                if is_loop_invariant(index, writes) && value_reads_index(value, *c, index) {
                    return Some((*c, index));
                }
            }
        }
    }
    None
}

/// Does `value` read `item IDX of C` somewhere (making the write an RMW)?
fn value_reads_index(value: &Expr, c: Symbol, idx: &Expr) -> bool {
    match value {
        Expr::Index { collection, index } => {
            matches!(&**collection, Expr::Identifier(cc) if *cc == c) && loads_equal(index, idx)
                || value_reads_index(collection, c, idx)
                || value_reads_index(index, c, idx)
        }
        _ => expr_children(value)
            .into_iter()
            .any(|ch| value_reads_index(ch, c, idx)),
    }
}

// ---------------------------------------------------------------------------
// Precondition: `C` appears ONLY as `item IDX of C`
// ---------------------------------------------------------------------------

fn c_only_at_index_block(block: &[Stmt], c: Symbol, idx: &Expr) -> bool {
    block.iter().all(|s| c_only_at_index_stmt(s, c, idx))
}

fn c_only_at_index_stmt(s: &Stmt, c: Symbol, idx: &Expr) -> bool {
    match s {
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            let coll_ok = match &**collection {
                Expr::Identifier(cc) if *cc == c => loads_equal(index, idx),
                other => c_only_at_index_expr(other, c, idx),
            };
            coll_ok && c_only_at_index_expr(index, c, idx) && c_only_at_index_expr(value, c, idx)
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => c_only_at_index_expr(value, c, idx),
        Stmt::SetField { object, value, .. } => {
            c_only_at_index_expr(object, c, idx) && c_only_at_index_expr(value, c, idx)
        }
        Stmt::Return { value: Some(v) } => c_only_at_index_expr(v, c, idx),
        Stmt::Return { value: None } | Stmt::Break => true,
        Stmt::Show { object, recipient } => {
            c_only_at_index_expr(object, c, idx) && c_only_at_index_expr(recipient, c, idx)
        }
        Stmt::Call { args, .. } => args.iter().all(|a| c_only_at_index_expr(a, c, idx)),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            c_only_at_index_expr(cond, c, idx)
                && c_only_at_index_block(then_block, c, idx)
                && else_block.is_none_or(|eb| c_only_at_index_block(eb, c, idx))
        }
        Stmt::While { cond, body, .. } => {
            c_only_at_index_expr(cond, c, idx) && c_only_at_index_block(body, c, idx)
        }
        // Any other statement kind: conservatively require that `C` does not
        // appear in it at all (unknown positions could alias / observe stale).
        other => !stmt_mentions(other, c),
    }
}

fn c_only_at_index_expr(e: &Expr, c: Symbol, idx: &Expr) -> bool {
    match e {
        Expr::Index { collection, index } => {
            let coll_ok = match &**collection {
                Expr::Identifier(cc) if *cc == c => loads_equal(index, idx),
                other => c_only_at_index_expr(other, c, idx),
            };
            coll_ok && c_only_at_index_expr(index, c, idx)
        }
        Expr::Identifier(cc) => *cc != c, // a bare `C` anywhere else is a hazard
        _ => expr_children(e)
            .into_iter()
            .all(|ch| c_only_at_index_expr(ch, c, idx)),
    }
}

/// Conservative: does statement `s` mention `Identifier(c)` anywhere?
fn stmt_mentions(s: &Stmt, c: Symbol) -> bool {
    let mut found = false;
    visit_stmt_exprs(s, &mut |e| {
        if expr_mentions(e, c) {
            found = true;
        }
    });
    found
}

fn expr_mentions(e: &Expr, c: Symbol) -> bool {
    matches!(e, Expr::Identifier(cc) if *cc == c)
        || expr_children(e).into_iter().any(|ch| expr_mentions(ch, c))
}

// ---------------------------------------------------------------------------
// Rewrite: `item IDX of C` → acc; `Set item IDX of C to V` → `Set acc to V'`
// ---------------------------------------------------------------------------

fn rewrite_stmt<'a>(
    s: &Stmt<'a>,
    c: Symbol,
    idx: &'a Expr<'a>,
    acc: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    match s {
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            if matches!(&**collection, Expr::Identifier(cc) if *cc == c) && loads_equal(index, idx)
            {
                Stmt::Set {
                    target: acc,
                    value: rewrite_reads(value, c, idx, acc, ea),
                }
            } else {
                Stmt::SetIndex {
                    collection: *collection,
                    index: rewrite_reads(index, c, idx, acc, ea),
                    value: rewrite_reads(value, c, idx, acc, ea),
                }
            }
        }
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => Stmt::Let {
            var: *var,
            ty: *ty,
            value: rewrite_reads(value, c, idx, acc, ea),
            mutable: *mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: *target,
            value: rewrite_reads(value, c, idx, acc, ea),
        },
        Stmt::Show { object, recipient } => Stmt::Show {
            object: rewrite_reads(object, c, idx, acc, ea),
            recipient: *recipient,
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| rewrite_reads(v, c, idx, acc, ea)),
        },
        Stmt::Call { function, args } => Stmt::Call {
            function: *function,
            args: args
                .iter()
                .map(|a| rewrite_reads(a, c, idx, acc, ea))
                .collect(),
        },
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond: rewrite_reads(cond, c, idx, acc, ea),
            then_block: rewrite_block_reads(then_block, c, idx, acc, ea, sa),
            else_block: else_block.map(|b| rewrite_block_reads(b, c, idx, acc, ea, sa)),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond: rewrite_reads(cond, c, idx, acc, ea),
            body: rewrite_block_reads(body, c, idx, acc, ea, sa),
            decreasing: *decreasing,
        },
        other => other.clone(),
    }
}

fn rewrite_block_reads<'a>(
    block: Block<'a>,
    c: Symbol,
    idx: &'a Expr<'a>,
    acc: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Block<'a> {
    let v: Vec<Stmt<'a>> = block
        .iter()
        .map(|s| rewrite_stmt(s, c, idx, acc, ea, sa))
        .collect();
    sa.alloc_slice(v)
}

fn rewrite_reads<'a>(
    e: &'a Expr<'a>,
    c: Symbol,
    idx: &'a Expr<'a>,
    acc: Symbol,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    if let Expr::Index { collection, index } = e {
        if matches!(&**collection, Expr::Identifier(cc) if *cc == c) && loads_equal(index, idx) {
            return ea.alloc(Expr::Identifier(acc));
        }
    }
    rebuild_expr(e, ea, &mut |ch| rewrite_reads(ch, c, idx, acc, ea))
}

// ---------------------------------------------------------------------------
// Generic AST traversal
// ---------------------------------------------------------------------------

fn contains_break_or_return(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Break | Stmt::Return { .. } => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            contains_break_or_return(then_block) || else_block.is_some_and(contains_break_or_return)
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            contains_break_or_return(body)
        }
        _ => false,
    })
}

fn expr_children<'a>(e: &'a Expr<'a>) -> Vec<&'a Expr<'a>> {
    match e {
        Expr::BinaryOp { left, right, .. } => vec![left, right],
        Expr::Not { operand } => vec![operand],
        Expr::Index { collection, index } => vec![collection, index],
        Expr::Slice {
            collection,
            start,
            end,
        } => vec![collection, start, end],
        Expr::Length { collection } => vec![collection],
        Expr::Contains { collection, value } => vec![collection, value],
        Expr::Union { left, right } | Expr::Intersection { left, right } => vec![left, right],
        Expr::Range { start, end } => vec![start, end],
        Expr::OptionSome { value } => vec![value],
        Expr::Copy { expr } => vec![expr],
        Expr::Give { value } => vec![value],
        Expr::WithCapacity { value, capacity } => vec![value, capacity],
        Expr::FieldAccess { object, .. } => vec![object],
        Expr::Call { args, .. } => args.iter().copied().collect(),
        Expr::CallExpr { callee, args } => std::iter::once(*callee)
            .chain(args.iter().copied())
            .collect(),
        Expr::List(elems) | Expr::Tuple(elems) => elems.iter().copied().collect(),
        Expr::New { init_fields, .. } => init_fields.iter().map(|(_, v)| *v).collect(),
        Expr::NewVariant { fields, .. } => fields.iter().map(|(_, v)| *v).collect(),
        // Leaves and node kinds with no arithmetic/collection children we forward.
        _ => Vec::new(),
    }
}

fn rebuild_expr<'a>(
    e: &'a Expr<'a>,
    ea: &'a Arena<Expr<'a>>,
    f: &mut dyn FnMut(&'a Expr<'a>) -> &'a Expr<'a>,
) -> &'a Expr<'a> {
    match e {
        Expr::BinaryOp { op, left, right } => {
            let l = f(left);
            let r = f(right);
            if std::ptr::eq(l, *left) && std::ptr::eq(r, *right) {
                e
            } else {
                ea.alloc(Expr::BinaryOp {
                    op: *op,
                    left: l,
                    right: r,
                })
            }
        }
        Expr::Not { operand } => {
            let o = f(operand);
            if std::ptr::eq(o, *operand) {
                e
            } else {
                ea.alloc(Expr::Not { operand: o })
            }
        }
        Expr::Index { collection, index } => {
            let col = f(collection);
            let i = f(index);
            if std::ptr::eq(col, *collection) && std::ptr::eq(i, *index) {
                e
            } else {
                ea.alloc(Expr::Index {
                    collection: col,
                    index: i,
                })
            }
        }
        Expr::Length { collection } => {
            let col = f(collection);
            if std::ptr::eq(col, *collection) {
                e
            } else {
                ea.alloc(Expr::Length { collection: col })
            }
        }
        Expr::Contains { collection, value } => {
            let col = f(collection);
            let v = f(value);
            if std::ptr::eq(col, *collection) && std::ptr::eq(v, *value) {
                e
            } else {
                ea.alloc(Expr::Contains {
                    collection: col,
                    value: v,
                })
            }
        }
        Expr::Call { function, args } => {
            let na: Vec<&'a Expr<'a>> = args.iter().map(|a| f(a)).collect();
            if na.iter().zip(args).all(|(n, o)| std::ptr::eq(*n, *o)) {
                e
            } else {
                ea.alloc(Expr::Call {
                    function: *function,
                    args: na,
                })
            }
        }
        Expr::OptionSome { value } => {
            let v = f(value);
            if std::ptr::eq(v, *value) {
                e
            } else {
                ea.alloc(Expr::OptionSome { value: v })
            }
        }
        Expr::Copy { expr } => {
            let v = f(expr);
            if std::ptr::eq(v, *expr) {
                e
            } else {
                ea.alloc(Expr::Copy { expr: v })
            }
        }
        // Node kinds we never rewrite through (they cannot hold a promotable
        // read given the `c_only_at_index` precondition) are returned unchanged.
        _ => e,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/reg_promote/tests.rs"]
mod tests;

/// Visit each top-level value expression of a statement.
fn visit_stmt_exprs<'a>(s: &'a Stmt<'a>, f: &mut dyn FnMut(&'a Expr<'a>)) {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => f(value),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            f(collection);
            f(index);
            f(value);
        }
        Stmt::SetField { object, value, .. } => {
            f(object);
            f(value);
        }
        Stmt::Return { value: Some(v) } => f(v),
        Stmt::Show { object, recipient } => {
            f(object);
            f(recipient);
        }
        Stmt::Push { value, collection } => {
            f(value);
            f(collection);
        }
        Stmt::Pop { collection, .. } => f(collection),
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            f(value);
            f(collection);
        }
        Stmt::Give { object, recipient } => {
            f(object);
            f(recipient);
        }
        Stmt::Call { args, .. } => {
            for a in args {
                f(a);
            }
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => f(cond),
        Stmt::Repeat { iterable, .. } => f(iterable),
        _ => {}
    }
}
