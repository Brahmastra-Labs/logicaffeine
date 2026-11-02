//! Leaf-outlining of the hot work region of a self-recursive in-place function.
//!
//! LLVM optimizes a hot loop far better as its own leaf function than embedded in
//! a *recursive* function (register allocation and loop transforms are inhibited
//! by the recursion sharing the frame). Every reference quicksort
//! (C/Rust/Go/Java/Nim/Zig) therefore factors the partition loop into a separate
//! `partition` function; LOGOS compiles the whole thing — partition loop and
//! recursion — into one `fn qs(arr: &mut [T], ..)`, and pays for it.
//!
//! This module plans the split of a self-recursive `&mut [T]` in-place function
//! into a leaf helper (the non-recursive work region) and a thin recursive
//! wrapper. The plan is pure over the AST; [`super::program`] renders it by
//! codegen'ing the region statements through the ordinary `codegen_stmt` path
//! (so bounds-check elimination and the overflow de-guard flow exactly as they
//! would inline) and wrapping them in a hand-written leaf signature. Nothing here
//! rewrites generated text — the helper body is real codegen.

use crate::ast::stmt::{Expr, Stmt};
use crate::intern::Symbol;

/// A planned outline of one self-recursive in-place function. The body splits
/// into three contiguous spans of `stmt_refs`: `[0, region_start)` base-case
/// guards (wrapper), `[region_start, region_end)` the work region (helper), and
/// `[region_end, len)` the recursive calls + trailing return (wrapper).
pub(crate) struct OutlinePlan {
    /// End of the leading base-case guards (start of the work region).
    pub region_start: usize,
    /// End of the work region (= index of the first self-recursive call).
    pub region_end: usize,
    /// The `let mut result = arr` alias binding's index (within the region),
    /// re-emitted in the wrapper so its recursive calls run on the same slice.
    pub alias_idx: Option<usize>,
    /// Scalars defined in the region and read by the trailing calls (the pivot
    /// index `i`) — the helper's return values.
    pub live_outs: Vec<Symbol>,
}

/// Does `e` (or a subexpression) call `name`?
fn calls(e: &Expr, name: Symbol) -> bool {
    match e {
        Expr::Call { function, args } => *function == name || args.iter().any(|a| calls(a, name)),
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        } => calls(left, name) || calls(right, name),
        Expr::Not { operand } => calls(operand, name),
        Expr::Index { collection, index } => calls(collection, name) || calls(index, name),
        Expr::Length { collection } => calls(collection, name),
        _ => false,
    }
}

/// A top-level statement that performs the self-recursion.
fn stmt_recurses(s: &Stmt, name: Symbol) -> bool {
    match s {
        Stmt::Set { value, .. } | Stmt::Let { value, .. } => calls(value, name),
        Stmt::Return { value: Some(e) } => calls(e, name),
        Stmt::Call { function, args } => *function == name || args.iter().any(|a| calls(a, name)),
        _ => false,
    }
}

/// A base-case guard: `if <cond> { return [x]; }` (single-return then-block, no else).
fn is_base_case_guard(s: &Stmt) -> bool {
    matches!(s, Stmt::If { then_block, else_block: None, .. }
        if then_block.len() == 1 && matches!(then_block[0], Stmt::Return { .. }))
}

/// The `let [mutable] <r> = <p>` alias of a slice parameter, returning `<r>`.
fn alias_of(s: &Stmt, slice_param: Symbol) -> Option<Symbol> {
    match s {
        Stmt::Let {
            var,
            value: Expr::Identifier(src),
            ..
        } if *src == slice_param => Some(*var),
        _ => None,
    }
}

/// Names bound by a statement (`let <v> = ..`), for live-out detection.
fn defined_var(s: &Stmt) -> Option<Symbol> {
    match s {
        Stmt::Let { var, .. } => Some(*var),
        _ => None,
    }
}

/// Whole-statement mention test for a symbol (reads in any position).
fn stmt_mentions(s: &Stmt, v: Symbol) -> bool {
    fn e(x: &Expr, v: Symbol) -> bool {
        match x {
            Expr::Identifier(s) => *s == v,
            Expr::BinaryOp { left, right, .. }
            | Expr::Union { left, right }
            | Expr::Intersection { left, right }
            | Expr::Range {
                start: left,
                end: right,
            } => e(left, v) || e(right, v),
            Expr::Not { operand } => e(operand, v),
            Expr::Call { function, args } => *function == v || args.iter().any(|a| e(a, v)),
            Expr::Index { collection, index } => e(collection, v) || e(index, v),
            Expr::Length { collection } => e(collection, v),
            _ => false,
        }
    }
    match s {
        Stmt::Set { target, value } => *target == v || e(value, v),
        Stmt::Let { value, .. } => e(value, v),
        Stmt::Return { value: Some(x) } => e(x, v),
        Stmt::Call { function, args } => *function == v || args.iter().any(|a| e(a, v)),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => e(collection, v) || e(index, v) || e(value, v),
        _ => false,
    }
}

/// Plan the split of a self-recursive in-place function, or `None` when it does
/// not match the conservative divide-and-conquer shape (a base case, a single
/// non-recursive work region containing exactly one loop, then self-recursive
/// calls). `slice_param` is the `&mut [T]` parameter (`arr`).
pub(crate) fn plan_outline(
    name: Symbol,
    stmts: &[&Stmt],
    slice_param: Symbol,
) -> Option<OutlinePlan> {
    // The self-recursive calls partition the body.
    let rec_idxs: Vec<usize> = stmts
        .iter()
        .enumerate()
        .filter(|(_, s)| stmt_recurses(s, name))
        .map(|(i, _)| i)
        .collect();
    let first_rec = *rec_idxs.first()?;
    let region_end = first_rec;

    // Everything from `first_rec` on stays in the wrapper; it must be only the
    // recursive calls and a trailing `return` — no post-recursion work.
    for i in first_rec..stmts.len() {
        let s = stmts[i];
        if !stmt_recurses(s, name) && !matches!(s, Stmt::Return { .. }) {
            return None;
        }
    }

    // The base-case guards must be a leading prefix; the work region is the
    // contiguous remainder before the recursion (so the peephole windows that
    // fuse the loop and the swaps stay intact when it is codegen'd).
    let mut region_start = 0;
    while region_start < first_rec && is_base_case_guard(stmts[region_start]) {
        region_start += 1;
    }
    if region_start >= first_rec {
        return None; // no work region
    }
    // No guard may hide inside the region (it would carry a `return` into the
    // helper, which returns a scalar).
    if (region_start..first_rec).any(|i| is_base_case_guard(stmts[i])) {
        return None;
    }

    // Exactly one loop in the region — the hot loop to leaf-ify.
    let loop_count = (region_start..region_end)
        .filter(|&i| matches!(stmts[i], Stmt::While { .. } | Stmt::Repeat { .. }))
        .count();
    if loop_count != 1 {
        return None;
    }

    // The `let mut result = arr` alias (if any).
    let alias_idx = (region_start..region_end).find(|&i| alias_of(stmts[i], slice_param).is_some());
    let alias_var = alias_idx.and_then(|i| alias_of(stmts[i], slice_param));

    // Live-outs: region-defined scalars read by the trailing calls. The alias is
    // re-created in the wrapper, so it is not a live-out.
    let mut live_outs = Vec::new();
    for i in region_start..region_end {
        if let Some(v) = defined_var(stmts[i]) {
            if Some(v) == alias_var || live_outs.contains(&v) {
                continue;
            }
            if (first_rec..stmts.len()).any(|t| stmt_mentions(stmts[t], v)) {
                live_outs.push(v);
            }
        }
    }
    if live_outs.is_empty() {
        return None;
    }

    Some(OutlinePlan {
        region_start,
        region_end,
        alias_idx,
        live_outs,
    })
}
