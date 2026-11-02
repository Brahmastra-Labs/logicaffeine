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
    /// The upper-bound parameter (must be `<= arr.len() + hi_len_adjust`).
    pub hi: Symbol,
    /// Extra headroom on the `hi <= len` precondition. `0` for the general case
    /// (`hi` IS a 1-based index bound). `1` when the bounding loop is EXCLUSIVE
    /// (`while c < hi`) AND every access of `arr` is `item <c> of arr` (the bare
    /// counter) — then the counter reaches only `hi-1`, so the largest 1-based
    /// index is `hi-1` and `hi <= len+1` is the tight, sound precondition. A slice
    /// copy `while j < hi { item j of arr }` is exactly this shape and is called
    /// with `hi = len+1` (one past the end), which the un-adjusted guard wrongly
    /// rejected. SOUNDNESS: adjust is `1` only when the sole index into `arr` is
    /// the exclusive counter, so no `item (c+k)` (k>0) access can reach `len+1`.
    pub hi_len_adjust: i64,
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
    let (lo, hi, counter, exclusive) = find_partition_bounds(body, &param_is)?;
    if lo == hi {
        return None;
    }
    // There must be 1-based indexing to protect, and no observable I/O before
    // it (so an entry-path abort is equivalent to an out-of-range access abort).
    if !body_indexes(body) || !is_pure(body) {
        return None;
    }
    // Tight upper-bound headroom: an EXCLUSIVE loop (`while counter < hi`) whose
    // only index into `arr` is the bare counter reaches index `hi-1`, so `hi` may
    // be `len+1` (one past the end); the precondition is `hi <= len+1`. Any other
    // shape keeps the conservative `hi <= len`.
    let hi_len_adjust = if exclusive && arr_only_indexed_by_counter(body, arr, counter) {
        1
    } else {
        0
    };
    Some(EntryGuard {
        arr,
        lo,
        hi,
        hi_len_adjust,
    })
}

fn is_seq_type(ty: &TypeExpr, interner: &Interner) -> bool {
    matches!(ty, TypeExpr::Generic { base, .. } if {
        matches!(interner.resolve(*base), "Seq" | "List" | "Vec")
    })
}

/// The `(arr, start, end, counter)` an affine-index descent constrains: every access
/// index is in `[0, end]`, so `start >= 0` and `end < arr.len()` is its precondition,
/// and the loop `counter` (`root`) is provably `>= 0` (seeded by the oracle).
pub(crate) struct HeapSiftGuard {
    pub arr: Symbol,
    pub start: Symbol,
    pub end: Symbol,
    pub counter: Symbol,
}

fn hs_expr_contains(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => {
            hs_expr_contains(left, sym) || hs_expr_contains(right, sym)
        }
        Expr::Index { collection, index } => {
            hs_expr_contains(collection, sym) || hs_expr_contains(index, sym)
        }
        Expr::Length { collection } => hs_expr_contains(collection, sym),
        _ => false,
    }
}

/// A `while <affine expr in `c`> < / <= <end-param>` loop — the descent guard
/// `while 2*root+1 <= end`, whose LHS is affine (not a bare counter, which
/// [`find_loop_bound`] handles). Returns `(counter, end)`.
fn find_heap_loop(
    stmts: &[Stmt],
    counter_init: &[(Symbol, Symbol)],
    param_is: &dyn Fn(Symbol) -> bool,
) -> Option<(Symbol, Symbol)> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                if let Expr::BinaryOp {
                    op: BinaryOpKind::Lt | BinaryOpKind::LtEq,
                    left,
                    right: Expr::Identifier(end),
                } = cond
                {
                    if param_is(*end) && !matches!(left, Expr::Identifier(_)) {
                        for (c, _) in counter_init {
                            if hs_expr_contains(left, *c) {
                                return Some((*c, *end));
                            }
                        }
                    }
                }
                if let Some(r) = find_heap_loop(body, counter_init, param_is) {
                    return Some(r);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if let Some(r) = find_heap_loop(then_block, counter_init, param_is) {
                    return Some(r);
                }
                if let Some(e) = else_block {
                    if let Some(r) = find_heap_loop(e, counter_init, param_is) {
                        return Some(r);
                    }
                }
            }
            Stmt::Repeat { body, .. } => {
                if let Some(r) = find_heap_loop(body, counter_init, param_is) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

/// Detect an affine-index descent (a heap `siftDown`, but the shape is general): a
/// `Seq` parameter indexed inside a `while <affine(counter)> <= end` loop whose
/// `counter` starts at a parameter `start`. Pure and index-bearing, exactly like
/// [`detect_entry_guard`].
pub(crate) fn detect_heap_sift_guard(
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    interner: &Interner,
) -> Option<HeapSiftGuard> {
    let arr = params
        .iter()
        .find_map(|(s, ty)| is_seq_type(ty, interner).then_some(*s))?;
    let param_is = |s: Symbol| params.iter().any(|(p, _)| *p == s);
    let mut counter_init: Vec<(Symbol, Symbol)> = Vec::new();
    collect_counter_inits(body, &param_is, &mut counter_init);
    let (counter, end) = find_heap_loop(body, &counter_init, &param_is)?;
    let start = counter_init
        .iter()
        .find(|(c, _)| *c == counter)
        .map(|(_, s)| *s)?;
    if start == end || !body_indexes(body) || !is_pure(body) {
        return None;
    }
    Some(HeapSiftGuard {
        arr,
        start,
        end,
        counter,
    })
}

/// The `Seq of T` element is at least 2 bytes wide — the soundness precondition
/// for de-guarding a length-bounded counter (see [`mark_deguardable_counters`]).
/// A `Vec<T>` holds at most `isize::MAX / size_of::<T>()` elements; only for
/// `size_of::<T>() == 1` (`Word8`/byte/bool) can that reach `i64::MAX`, where
/// `counter + 1` could overflow. `Int` (i64/i32), the `WordN`, floats — all wide.
fn seq_element_is_wide(ty: &TypeExpr, interner: &Interner) -> bool {
    let TypeExpr::Generic { base, params } = ty else {
        return false;
    };
    if !matches!(interner.resolve(*base), "Seq" | "List" | "Vec") {
        return false;
    }
    match params.first() {
        Some(TypeExpr::Primitive(e)) | Some(TypeExpr::Named(e)) => {
            !matches!(interner.resolve(*e), "Word8" | "Byte" | "Bool")
        }
        // A non-scalar element (nested Seq, struct handle, …) is a pointer/handle
        // — comfortably wide.
        Some(_) => true,
        None => false,
    }
}

/// Mark the `counter ± <small literal>` arithmetic of the entry guard's
/// length-bounded partition counters as provably in i64, so codegen lowers it as
/// raw `c + 1` rather than the checked `logos_add_i64`.
///
/// **Soundness.** A counter `c` here is initialized to the guard's `lo` and is
/// only ever advanced by a single `+ 1` per iteration of the `while _ < hi` loop,
/// which itself runs at most `hi - lo` times — so `c <= lo + (hi - lo) = hi`. The
/// guard proves `hi <= arr.len()`, and a `Seq of T` with `size_of::<T>() >= 2`
/// holds at most `isize::MAX / 2 < i64::MAX / 2` elements. Hence
/// `c <= hi <= len < i64::MAX / 2`, and `c ± k` for any `|k| <= 2^40` stays in
/// range. The mark is pointer-keyed to these exact op nodes; a mismatch can only
/// *fail* to fire (a missed optimization), never mislower.
pub(crate) fn mark_deguardable_counters(
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    guard: &EntryGuard,
    interner: &Interner,
) {
    // Element-width soundness gate.
    let wide = params
        .iter()
        .find(|(s, _)| *s == guard.arr)
        .is_some_and(|(_, ty)| seq_element_is_wide(ty, interner));
    if !wide {
        return;
    }
    let l_body = match find_hi_loop_body(body, guard.hi) {
        Some(b) => b,
        None => return,
    };
    // A counter qualifies when: it is initialized `let c = lo`, and every one of
    // its reassignments in the whole function is `Set c to c + 1`, with at least
    // one such increment sitting inside the `hi`-bounded loop (never in a loop
    // nested within it — that could advance `c` more than once per outer trip).
    let mut counters: Vec<Symbol> = Vec::new();
    for (v, src) in collect_lo_inits(body, guard.lo) {
        let _ = src;
        if reassignments_all_plus_one(body, v)
            && counter_incremented_in_loop_body(l_body, v)
            && !counter_incremented_in_nested_loop(l_body, v)
        {
            counters.push(v);
        }
    }
    for c in counters {
        mark_counter_arith(body, c);
    }
}

/// `let <c> = <guard-lo>` bindings (the counter seeds).
fn collect_lo_inits(stmts: &[Stmt], lo: Symbol) -> Vec<(Symbol, Symbol)> {
    let mut out = Vec::new();
    fn walk(stmts: &[Stmt], lo: Symbol, out: &mut Vec<(Symbol, Symbol)>) {
        for s in stmts {
            match s {
                Stmt::Let {
                    var,
                    value: Expr::Identifier(src),
                    ..
                } if *src == lo => {
                    out.push((*var, *src));
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, lo, out);
                    if let Some(e) = else_block {
                        walk(e, lo, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => walk(body, lo, out),
                _ => {}
            }
        }
    }
    walk(stmts, lo, &mut out);
    out
}

/// The body of the `while <c> < <hi>` (or `<=`) loop, or `None`.
fn find_hi_loop_body<'a>(stmts: &'a [Stmt<'a>], hi: Symbol) -> Option<&'a [Stmt<'a>]> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                if let Expr::BinaryOp {
                    op: BinaryOpKind::Lt | BinaryOpKind::LtEq,
                    right: Expr::Identifier(h),
                    ..
                } = cond
                {
                    if *h == hi {
                        return Some(body);
                    }
                }
                if let Some(r) = find_hi_loop_body(body, hi) {
                    return Some(r);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if let Some(r) = find_hi_loop_body(then_block, hi) {
                    return Some(r);
                }
                if let Some(e) = else_block {
                    if let Some(r) = find_hi_loop_body(e, hi) {
                        return Some(r);
                    }
                }
            }
            Stmt::Repeat { body, .. } => {
                if let Some(r) = find_hi_loop_body(body, hi) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

/// Is `v` a `Set v to v + 1`? (the sole shape allowed for a length-bounded counter)
fn is_plus_one(target: Symbol, value: &Expr) -> bool {
    matches!(
        value,
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: Expr::Identifier(l),
            right: Expr::Literal(crate::ast::stmt::Literal::Number(n)),
        } if *l == target && *n == 1
    )
}

/// Every reassignment of `v` in the function is `Set v to v + 1`.
fn reassignments_all_plus_one(stmts: &[Stmt], v: Symbol) -> bool {
    fn walk(stmts: &[Stmt], v: Symbol, ok: &mut bool) {
        for s in stmts {
            match s {
                Stmt::Set { target, value } => {
                    if *target == v && !is_plus_one(v, value) {
                        *ok = false;
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, v, ok);
                    if let Some(e) = else_block {
                        walk(e, v, ok);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => walk(body, v, ok),
                _ => {}
            }
        }
    }
    let mut ok = true;
    walk(stmts, v, &mut ok);
    ok
}

/// `Set v to v + 1` appears directly in `l_body` (top level or under an `if`,
/// but not inside a nested loop).
fn counter_incremented_in_loop_body(l_body: &[Stmt], v: Symbol) -> bool {
    fn walk(stmts: &[Stmt], v: Symbol) -> bool {
        for s in stmts {
            match s {
                Stmt::Set { target, value } if *target == v && is_plus_one(v, value) => {
                    return true;
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    if walk(then_block, v) || else_block.as_ref().is_some_and(|e| walk(e, v)) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    walk(l_body, v)
}

/// `Set v to v + 1` appears inside a loop nested within `l_body` — disqualifying,
/// since that could advance `v` more than once per outer-loop trip.
fn counter_incremented_in_nested_loop(l_body: &[Stmt], v: Symbol) -> bool {
    fn any_incr(stmts: &[Stmt], v: Symbol) -> bool {
        stmts.iter().any(|s| match s {
            Stmt::Set { target, value } => *target == v && is_plus_one(v, value),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => any_incr(then_block, v) || else_block.as_ref().is_some_and(|e| any_incr(e, v)),
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => any_incr(body, v),
            _ => false,
        })
    }
    fn walk(stmts: &[Stmt], v: Symbol) -> bool {
        stmts.iter().any(|s| match s {
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => any_incr(body, v),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => walk(then_block, v) || else_block.as_ref().is_some_and(|e| walk(e, v)),
            _ => false,
        })
    }
    walk(l_body, v)
}

/// The element byte-width of a `Seq of T` for a known scalar `T`, or `None`.
/// Drives the overflow-safety margin for [`mark_heap_sift_index_arith`]: a
/// `Vec<T>` holds at most `isize::MAX / size_of::<T>()` elements, so a wider
/// element shrinks the maximum index and widens the headroom before an affine
/// index expression can overflow `i64`.
fn seq_element_bytes(ty: &TypeExpr, interner: &Interner) -> Option<u32> {
    let TypeExpr::Generic { base, params } = ty else {
        return None;
    };
    if !matches!(interner.resolve(*base), "Seq" | "List" | "Vec") {
        return None;
    }
    match params.first() {
        Some(TypeExpr::Primitive(e)) | Some(TypeExpr::Named(e)) => match interner.resolve(*e) {
            "Int" | "Word64" | "Float" => Some(8),
            "Word32" => Some(4),
            "Word16" => Some(2),
            "Word8" | "Byte" | "Bool" => Some(1),
            _ => None,
        },
        _ => None,
    }
}

/// `Some(c)` when `e` is `v`, `c*v`, or `v*c` for a bounded index `v` and a small
/// coefficient `|c| <= 4` — the affine-index shapes a heap descent produces.
fn heap_coeff_var(e: &Expr, bounded: &std::collections::HashSet<Symbol>) -> Option<i64> {
    match e {
        Expr::Identifier(s) if bounded.contains(s) => Some(1),
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left,
            right,
        } => match (left, right) {
            (Expr::Literal(crate::ast::stmt::Literal::Number(c)), Expr::Identifier(s))
            | (Expr::Identifier(s), Expr::Literal(crate::ast::stmt::Literal::Number(c)))
                if bounded.contains(s) && c.abs() <= 4 =>
            {
                Some(*c)
            }
            _ => None,
        },
        _ => None,
    }
}

/// A single-variable affine op `c*v (+/- k)` on a bounded index (`|c| <= 4`,
/// `|k| <= 2^40`) — provably in `i64` when `v <= end < len` and the element is
/// `>= 8` bytes (so `len < 2^60`, hence `c*v + k < 2^62`).
fn is_heap_affine_op(e: &Expr, bounded: &std::collections::HashSet<Symbol>) -> bool {
    let small = |x: &Expr| matches!(x, Expr::Literal(crate::ast::stmt::Literal::Number(n)) if n.abs() <= (1i64 << 40));
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            ..
        } => heap_coeff_var(e, bounded).is_some(),
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract,
            left,
            right,
        } => {
            (heap_coeff_var(left, bounded).is_some() && small(right))
                || (small(left) && heap_coeff_var(right, bounded).is_some())
        }
        _ => false,
    }
}

/// Grow `bounded` with any variable a loop-body `Let`/`Set` assigns an affine
/// function of already-bounded indices (`child = 2*root+1`, `swapIdx = child+1`).
/// Iterated to a fixpoint by the caller.
fn collect_heap_derived(stmts: &[Stmt], bounded: &mut std::collections::HashSet<Symbol>) {
    let is_bounded_val = |value: &Expr, b: &std::collections::HashSet<Symbol>| {
        matches!(value, Expr::Identifier(s) if b.contains(s)) || is_heap_affine_op(value, b)
    };
    for s in stmts {
        match s {
            Stmt::Let { var, value, .. } => {
                if is_bounded_val(value, bounded) {
                    bounded.insert(*var);
                }
            }
            Stmt::Set { target, value } => {
                if is_bounded_val(value, bounded) {
                    bounded.insert(*target);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_heap_derived(then_block, bounded);
                if let Some(e) = else_block {
                    collect_heap_derived(e, bounded);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_heap_derived(body, bounded)
            }
            _ => {}
        }
    }
}

/// Mark every bounded-index affine op node in a statement tree as raw-i64.
fn mark_heap_affine_block(stmts: &[Stmt], bounded: &std::collections::HashSet<Symbol>) {
    for s in stmts {
        match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => {
                mark_heap_affine_expr(value, bounded)
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                mark_heap_affine_expr(collection, bounded);
                mark_heap_affine_expr(index, bounded);
                mark_heap_affine_expr(value, bounded);
            }
            Stmt::Return { value: Some(e) } => mark_heap_affine_expr(e, bounded),
            Stmt::Push {
                value, collection, ..
            }
            | Stmt::Add {
                value, collection, ..
            } => {
                mark_heap_affine_expr(value, bounded);
                mark_heap_affine_expr(collection, bounded);
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                mark_heap_affine_expr(cond, bounded);
                mark_heap_affine_block(then_block, bounded);
                if let Some(e) = else_block {
                    mark_heap_affine_block(e, bounded);
                }
            }
            Stmt::While { cond, body, .. } => {
                mark_heap_affine_expr(cond, bounded);
                mark_heap_affine_block(body, bounded);
            }
            Stmt::Repeat { body, .. } => mark_heap_affine_block(body, bounded),
            _ => {}
        }
    }
}

fn mark_heap_affine_expr(e: &Expr, bounded: &std::collections::HashSet<Symbol>) {
    if is_heap_affine_op(e, bounded) {
        crate::optimize::facts::mark_expr(crate::optimize::facts::Fact::RawIntOp, e);
    }
    match e {
        Expr::BinaryOp { left, right, .. } => {
            mark_heap_affine_expr(left, bounded);
            mark_heap_affine_expr(right, bounded);
        }
        Expr::Not { operand } => mark_heap_affine_expr(operand, bounded),
        Expr::Call { args, .. } => args.iter().for_each(|a| mark_heap_affine_expr(a, bounded)),
        Expr::Index { collection, index } => {
            mark_heap_affine_expr(collection, bounded);
            mark_heap_affine_expr(index, bounded);
        }
        Expr::Length { collection } => mark_heap_affine_expr(collection, bounded),
        _ => {}
    }
}

/// Mark the affine heap-index arithmetic (`2*root+1`, `child+1`, `swapIdx+1`, …)
/// inside an affine-descent sift's loop body as provably-raw-`i64`, so it lowers
/// to plain `2*root + 1` rather than the `logos_mul_exact`/`logos_add_exact`
/// overflow-promote form.
///
/// **Soundness.** The marking is scoped to the descent loop's BODY, where the loop
/// guard `<affine(counter)> <= end` has already held — so `counter <= end` and every
/// index derived from it (`child = 2*counter+1 <= end`, `swapIdx <= end`) is bounded
/// by `end`, which the entry guard proves `< arr.len()`. For an element of
/// `>= 8` bytes a `Vec<T>` holds at most `isize::MAX / 8 < 2^60` elements, so any
/// index is `< 2^60` and an affine `c*idx + k` (`|c| <= 4`, `|k| <= 2^40`) stays
/// `< 2^62 < i64::MAX` — it cannot overflow. The loop GUARD's own affine (evaluated
/// once with the unconstrained entry `start`) is left checked. Pointer-keyed to
/// these exact op nodes; a mismatch can only fail to fire, never mislower.
pub(crate) fn mark_heap_sift_index_arith(
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    guard: &HeapSiftGuard,
    interner: &Interner,
) {
    let Some(bounded) = heap_sift_word_vars(params, body, guard, interner) else {
        return;
    };
    mark_heap_affine_block(body, &bounded);
}

/// Every integer name whose value is proven to stay in one machine word by a
/// heap-sift entry guard. Backends may use this to shadow an otherwise exact
/// `Int` representation inside a guarded fast path instead of merely lowering
/// the affine operations themselves as raw arithmetic.
///
/// The returned set includes the two bound parameters, the descent counter,
/// and every local derived from those indices. `None` retains the exact path
/// when the sequence element is too narrow to provide the required overflow
/// headroom or the guarded loop cannot be recovered.
pub(crate) fn heap_sift_word_vars(
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    guard: &HeapSiftGuard,
    interner: &Interner,
) -> Option<std::collections::HashSet<Symbol>> {
    let wide8 = params
        .iter()
        .find(|(s, _)| *s == guard.arr)
        .and_then(|(_, ty)| seq_element_bytes(ty, interner))
        .is_some_and(|b| b >= 8);
    if !wide8 {
        return None;
    }
    let l_body = find_hi_loop_body(body, guard.end)?;
    let mut bounded: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
    bounded.insert(guard.start);
    bounded.insert(guard.end);
    bounded.insert(guard.counter);
    // Fixpoint over the loop body: `child = 2*root+1`, then `swapIdx = child`, then
    // `swapIdx = child + 1` — three levels, so a few passes converge.
    for _ in 0..4 {
        collect_heap_derived(l_body, &mut bounded);
    }
    Some(bounded)
}

/// Register every `v + k` / `v - k` (small literal `k`) op node in the function
/// as provably-raw-i64. Recurses through statements and expressions.
fn mark_counter_arith(stmts: &[Stmt], v: Symbol) {
    for s in stmts {
        stmt_mark(s, v);
    }
}

fn stmt_mark(s: &Stmt, v: Symbol) {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_mark(value, v),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            expr_mark(collection, v);
            expr_mark(index, v);
            expr_mark(value, v);
        }
        Stmt::Return { value: Some(e) } => expr_mark(e, v),
        Stmt::Push {
            value, collection, ..
        }
        | Stmt::Add {
            value, collection, ..
        } => {
            expr_mark(value, v);
            expr_mark(collection, v);
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            expr_mark(cond, v);
            mark_counter_arith(then_block, v);
            if let Some(e) = else_block {
                mark_counter_arith(e, v);
            }
        }
        Stmt::While { cond, body, .. } => {
            expr_mark(cond, v);
            mark_counter_arith(body, v);
        }
        Stmt::Repeat { body, .. } => mark_counter_arith(body, v),
        _ => {}
    }
}

fn expr_mark(e: &Expr, v: Symbol) {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add | BinaryOpKind::Subtract,
        left,
        right,
    } = e
    {
        let small_lit = |x: &Expr| matches!(x, Expr::Literal(crate::ast::stmt::Literal::Number(n)) if n.abs() <= (1i64 << 40));
        let is_v = |x: &Expr| matches!(x, Expr::Identifier(s) if *s == v);
        if (is_v(left) && small_lit(right)) || (is_v(right) && small_lit(left)) {
            crate::optimize::facts::mark_expr(crate::optimize::facts::Fact::RawIntOp, e);
        }
    }
    match e {
        Expr::BinaryOp { left, right, .. } => {
            expr_mark(left, v);
            expr_mark(right, v);
        }
        Expr::Not { operand } => expr_mark(operand, v),
        Expr::Call { args, .. } => args.iter().for_each(|a| expr_mark(a, v)),
        Expr::Index { collection, index } => {
            expr_mark(collection, v);
            expr_mark(index, v);
        }
        Expr::Length { collection } => expr_mark(collection, v),
        _ => {}
    }
}

/// Find `(lo, hi)`: the loop `while <c> < <hi>` (or `<= <hi>`) whose counter
/// `<c>` is bound by an earlier `let <c> = <lo>`, with both `lo` and `hi`
/// parameters. Scans the whole body (the loop may sit under the base-case `if`).
fn find_partition_bounds(
    body: &[Stmt],
    param_is: &dyn Fn(Symbol) -> bool,
) -> Option<(Symbol, Symbol, Symbol, bool)> {
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
            Stmt::Let {
                var,
                value: Expr::Identifier(src),
                ..
            } if param_is(*src) => {
                out.push((*var, *src));
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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

/// Returns `(lo, hi, counter, exclusive)` — `exclusive` is `true` for `while c < hi`
/// (the counter reaches only `hi-1`), `false` for `while c <= hi`.
fn find_loop_bound(
    stmts: &[Stmt],
    counter_init: &[(Symbol, Symbol)],
    param_is: &dyn Fn(Symbol) -> bool,
) -> Option<(Symbol, Symbol, Symbol, bool)> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                if let Expr::BinaryOp {
                    op: op @ (BinaryOpKind::Lt | BinaryOpKind::LtEq),
                    left: Expr::Identifier(c),
                    right: Expr::Identifier(hi),
                } = cond
                {
                    if param_is(*hi) {
                        if let Some((_, lo)) = counter_init.iter().find(|(cc, _)| cc == c) {
                            return Some((*lo, *hi, *c, matches!(op, BinaryOpKind::Lt)));
                        }
                    }
                }
                if let Some(r) = find_loop_bound(body, counter_init, param_is) {
                    return Some(r);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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

/// True iff every `item E of arr` (an index whose collection is `arr`) uses `E == counter`
/// exactly — the exclusive loop counter itself. Then the largest 1-based index into `arr` is
/// `counter_max = hi-1`, so the tight precondition is `hi <= len+1`. Any `item (counter+k) of arr`
/// (k>0) or `item <other> of arr` makes it `false`, keeping the conservative `hi <= len`. Accesses
/// of OTHER collections are ignored (they have their own bounds). SOUNDNESS-CRITICAL: a `true` here
/// is the sole license for the `+1` headroom in the entry guard.
fn arr_only_indexed_by_counter(stmts: &[Stmt], arr: Symbol, counter: Symbol) -> bool {
    fn idx_ok(collection: &Expr, index: &Expr, arr: Symbol, counter: Symbol) -> bool {
        if matches!(collection, Expr::Identifier(s) if *s == arr) {
            matches!(index, Expr::Identifier(s) if *s == counter)
        } else {
            true
        }
    }
    fn expr_ok(e: &Expr, arr: Symbol, counter: Symbol) -> bool {
        match e {
            Expr::Index { collection, index } => {
                idx_ok(collection, index, arr, counter)
                    && expr_ok(collection, arr, counter)
                    && expr_ok(index, arr, counter)
            }
            Expr::BinaryOp { left, right, .. } => {
                expr_ok(left, arr, counter) && expr_ok(right, arr, counter)
            }
            Expr::Not { operand } => expr_ok(operand, arr, counter),
            Expr::Call { args, .. } => args.iter().all(|a| expr_ok(a, arr, counter)),
            Expr::Length { collection } => expr_ok(collection, arr, counter),
            _ => true,
        }
    }
    fn stmt_ok(s: &Stmt, arr: Symbol, counter: Symbol) -> bool {
        match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_ok(value, arr, counter),
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                idx_ok(collection, index, arr, counter)
                    && expr_ok(collection, arr, counter)
                    && expr_ok(index, arr, counter)
                    && expr_ok(value, arr, counter)
            }
            Stmt::Return { value: Some(e) } => expr_ok(e, arr, counter),
            Stmt::Push { value, .. } | Stmt::Add { value, .. } => expr_ok(value, arr, counter),
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                expr_ok(cond, arr, counter)
                    && then_block.iter().all(|s| stmt_ok(s, arr, counter))
                    && else_block
                        .as_ref()
                        .map_or(true, |e| e.iter().all(|s| stmt_ok(s, arr, counter)))
            }
            Stmt::While { cond, body, .. } => {
                expr_ok(cond, arr, counter) && body.iter().all(|s| stmt_ok(s, arr, counter))
            }
            Stmt::Repeat { body, .. } => body.iter().all(|s| stmt_ok(s, arr, counter)),
            _ => true,
        }
    }
    stmts.iter().all(|s| stmt_ok(s, arr, counter))
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
        Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
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
        Stmt::If {
            then_block,
            else_block,
            ..
        } => is_pure(then_block) && else_block.as_ref().map_or(true, |e| is_pure(e)),
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => is_pure(body),
        _ => true,
    }
}
