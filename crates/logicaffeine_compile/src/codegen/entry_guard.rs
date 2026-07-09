//! Function-entry precondition guard for recursive 1-based partitions (the
//! quicksort bounds-check-elimination lever).
//!
//! A recursive partition indexes its slice parameter by `item j of arr` with
//! `j ∈ [lo, hi]` (1-based → `arr[(j-1)]`). LLVM keeps the per-access bounds
//! checks because it cannot prove `lo >= 1` (so `(j-1) as usize` does not wrap)
//! nor `hi <= len`. Those two facts ARE the function's precondition; emitting
//! them as one entry assert lets LLVM drop the checks across the hot loop.
//!
//! The guard is emitted as `if lo < hi { assert!(lo >= 1 && hi <= arr.len()) }`,
//! so it runs only when the function falls through its base case into the
//! indexing path. For the **pure** (no-I/O) functions this targets, that assert
//! either never fires (a valid sort) or aborts exactly where the out-of-range
//! access would have — observationally identical — so it never changes behavior.

use crate::ast::stmt::{BinaryOpKind, Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

pub(crate) struct EntryGuard {
    /// The slice/Vec parameter whose length bounds the accesses.
    pub arr: Symbol,
    /// The lower-bound parameter (must be `>= 1`).
    pub lo: Symbol,
    /// The upper-bound parameter (must be `<= arr.len()`).
    pub hi: Symbol,
}

/// Detect the recursive-1-based-partition shape and the `(arr, lo, hi)` it
/// constrains, or `None` when the function does not qualify.
pub(crate) fn detect_entry_guard(
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    interner: &Interner,
) -> Option<EntryGuard> {
    // The array parameter: the first Seq/List/Vec-typed parameter.
    let arr = params
        .iter()
        .find_map(|(s, ty)| is_seq_type(ty, interner).then_some(*s))?;
    // The bound parameters from a `while c < hi` loop whose counter `c` is
    // initialized `let c = lo`, with `lo` and `hi` both parameters.
    let param_is = |s: Symbol| params.iter().any(|(p, _)| *p == s);
    let (lo, hi) = find_partition_bounds(body, &param_is)?;
    if lo == hi {
        return None;
    }
    // There must be 1-based indexing to protect, and no observable I/O before
    // it (so an entry-path abort is equivalent to an out-of-range access abort).
    if !body_indexes(body) || !is_pure(body) {
        return None;
    }
    Some(EntryGuard { arr, lo, hi })
}

fn is_seq_type(ty: &TypeExpr, interner: &Interner) -> bool {
    matches!(ty, TypeExpr::Generic { base, .. } if {
        matches!(interner.resolve(*base), "Seq" | "List" | "Vec")
    })
}

/// Find `(lo, hi)`: the loop `while <c> < <hi>` (or `<= <hi>`) whose counter
/// `<c>` is bound by an earlier `let <c> = <lo>`, with both `lo` and `hi`
/// parameters. Scans the whole body (the loop may sit under the base-case `if`).
fn find_partition_bounds(
    body: &[Stmt],
    param_is: &dyn Fn(Symbol) -> bool,
) -> Option<(Symbol, Symbol)> {
    // Counter -> its initializing parameter, gathered from `let c = <param>`.
    let mut counter_init: Vec<(Symbol, Symbol)> = Vec::new();
    collect_counter_inits(body, param_is, &mut counter_init);
    find_loop_bound(body, &counter_init, param_is)
}

fn collect_counter_inits(
    stmts: &[Stmt],
    param_is: &dyn Fn(Symbol) -> bool,
    out: &mut Vec<(Symbol, Symbol)>,
) {
    for s in stmts {
        match s {
            Stmt::Let { var, value: Expr::Identifier(src), .. } if param_is(*src) => {
                out.push((*var, *src));
            }
            Stmt::If { then_block, else_block, .. } => {
                collect_counter_inits(then_block, param_is, out);
                if let Some(e) = else_block {
                    collect_counter_inits(e, param_is, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_counter_inits(body, param_is, out)
            }
            _ => {}
        }
    }
}

fn find_loop_bound(
    stmts: &[Stmt],
    counter_init: &[(Symbol, Symbol)],
    param_is: &dyn Fn(Symbol) -> bool,
) -> Option<(Symbol, Symbol)> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                if let Expr::BinaryOp {
                    op: BinaryOpKind::Lt | BinaryOpKind::LtEq,
                    left: Expr::Identifier(c),
                    right: Expr::Identifier(hi),
                } = cond
                {
                    if param_is(*hi) {
                        if let Some((_, lo)) = counter_init.iter().find(|(cc, _)| cc == c) {
                            return Some((*lo, *hi));
                        }
                    }
                }
                if let Some(r) = find_loop_bound(body, counter_init, param_is) {
                    return Some(r);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if let Some(r) = find_loop_bound(then_block, counter_init, param_is) {
                    return Some(r);
                }
                if let Some(e) = else_block {
                    if let Some(r) = find_loop_bound(e, counter_init, param_is) {
                        return Some(r);
                    }
                }
            }
            Stmt::Repeat { body, .. } => {
                if let Some(r) = find_loop_bound(body, counter_init, param_is) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

/// True if any statement performs a 1-based index (`item E of x`) read or write.
fn body_indexes(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_indexes)
}

fn stmt_indexes(s: &Stmt) -> bool {
    match s {
        Stmt::SetIndex { .. } => true,
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_indexes(value),
        Stmt::Return { value: Some(e) } => expr_indexes(e),
        Stmt::If { cond, then_block, else_block, .. } => {
            expr_indexes(cond)
                || body_indexes(then_block)
                || else_block.as_ref().is_some_and(|e| body_indexes(e))
        }
        Stmt::While { cond, body, .. } => expr_indexes(cond) || body_indexes(body),
        Stmt::Repeat { body, .. } => body_indexes(body),
        Stmt::Push { value, .. } | Stmt::Add { value, .. } => expr_indexes(value),
        _ => false,
    }
}

fn expr_indexes(e: &Expr) -> bool {
    match e {
        Expr::Index { .. } => true,
        Expr::BinaryOp { left, right, .. } => expr_indexes(left) || expr_indexes(right),
        Expr::Not { operand } => expr_indexes(operand),
        Expr::Call { args, .. } => args.iter().any(|a| expr_indexes(a)),
        Expr::Length { collection } => expr_indexes(collection),
        _ => false,
    }
}

/// A function is "pure enough" for the entry guard when it has no observable
/// side effect (no I/O, messaging, spawning, or sleeping) before its accesses —
/// only then is an entry-path abort indistinguishable from an access-path abort.
fn is_pure(stmts: &[Stmt]) -> bool {
    stmts.iter().all(stmt_is_pure)
}

fn stmt_is_pure(s: &Stmt) -> bool {
    match s {
        Stmt::Show { .. }
        | Stmt::Give { .. }
        | Stmt::WriteFile { .. }
        | Stmt::ReadFrom { .. }
        | Stmt::Sleep { .. }
        | Stmt::Listen { .. }
        | Stmt::ConnectTo { .. }
        | Stmt::SendMessage { .. }
        | Stmt::StreamMessage { .. }
        | Stmt::AwaitMessage { .. }
        | Stmt::Spawn { .. }
        | Stmt::Inspect { .. }
        | Stmt::Mount { .. }
        | Stmt::LaunchTask { .. }
        | Stmt::LaunchTaskWithHandle { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::SendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::TryReceivePipe { .. } => false,
        Stmt::If { then_block, else_block, .. } => {
            is_pure(then_block) && else_block.as_ref().map_or(true, |e| is_pure(e))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => is_pure(body),
        _ => true,
    }
}
