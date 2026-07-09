//! Loop unrolling for small, compile-time-constant trip-count loops that are
//! nested inside a hot outer loop and index a FIXED-SIZE SCALARIZABLE array.
//!
//! Motivation (nbody): LLVM already fully unrolls and vectorizes *top-level*
//! constant loops (nbody's one-time `energy` block emits packed `vsqrtpd`), but
//! it declines to unroll a constant-trip loop *nested* inside a runtime loop
//! (nbody's per-step `advance`). That leaves the inner body scalar and prevents
//! SROA of the `[f64; N]` arrays, so every body value round-trips the stack.
//!
//! This pass closes exactly that gap, but stays narrowly scoped so it never
//! steals loop shapes other passes own:
//!   - **Nested** in an enclosing loop (top-level constant loops are LLVM's job).
//!   - Bounded by integer constants, small trip count.
//!   - Every collection it indexes is a fixed-size **scalarizable** array
//!     (`[T; N]`). A loop indexing a reference-semantics `LogosSeq`, or one that
//!     grows a buffer via `Push`, belongs to the de-Rc buffer-reuse and
//!     borrow-hoist passes — unrolling it would destroy the loop shape they
//!     pattern-match on AND yields no SROA benefit (only scalarized arrays
//!     promote to registers).
//!   - No `Break` and only substitution-safe statements.
//!
//! **Value-preserving.** Unrolling is pure sequencing: iteration `v`'s body
//! emits before `v+1`'s in identical statement order, so loop-carried
//! reductions keep their exact order and grouping. Only integer index
//! arithmetic is folded; float operand structure is untouched. No reassociation,
//! no FMA — numeric output is bit-identical.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Literal, MatchArm, Stmt};
use crate::intern::{Interner, Symbol};
use crate::loop_shape::{
    const_eval_i64, extract_counted_repeat, extract_counted_while, CountedLoop,
};

use super::partial_eval::substitute_block;

/// Maximum trip count of a single loop we will fully unroll.
const TRIP_THRESHOLD: usize = 16;
/// Global ceiling on total unrolled iterations across the whole program — a
/// backstop against code explosion / register pressure.
const TOTAL_EXPANSION_BUDGET: usize = 4096;

/// Fully unroll qualifying nested constant loops in `stmts`. Returns the
/// (possibly) rewritten statements and whether anything was actually unrolled.
/// When nothing unrolls, the ORIGINAL statements are returned untouched — the
/// pass is a guaranteed no-op so it never perturbs unrelated programs.
///
/// AOT entry: top-level constant loops are left rolled (LLVM unrolls and SROAs
/// them itself); only the loops nested in a runtime loop are unrolled.
pub fn unroll_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    unroll_entry(stmts, false, expr_arena, stmt_arena, interner)
}

/// RUN-path entry: also unroll TOP-LEVEL constant loops over scalarizable
/// arrays. The interpreter has no LLVM to unroll/SROA them, so leaving them
/// rolled keeps their variable-index reads — which block the paired
/// `scalarize` pass (any non-constant index disqualifies the whole array). On
/// the run path every constant-bound loop over a scalarizable array should
/// collapse so the array can become scalar locals.
pub fn unroll_stmts_run<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    unroll_entry(stmts, true, expr_arena, stmt_arena, interner)
}

fn unroll_entry<'a>(
    stmts: Vec<Stmt<'a>>,
    entry_in_loop: bool,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    // The arrays worth unrolling for — fixed-size, scalarizable to `[T; N]`.
    // Any loop indexing something NOT in this set is left rolled so the de-Rc
    // buffer-reuse and borrow-hoist passes keep their loop shapes.
    let scalarizable = crate::codegen::detection::scalarizable_seq_symbols(&stmts, interner);
    // The rotate builtins, resolved to their existing symbols (idempotent, non-inserting). A loop whose
    // induction variable feeds a `rotl`/`rotr` shift amount unrolls even at the top level — see
    // `plan_unroll`. Absent from the program ⇒ empty set ⇒ the crypto path never fires.
    let rotate_syms: HashSet<Symbol> = ["rotl", "rotr"].iter().filter_map(|n| interner.lookup(n)).collect();
    // Nothing to do when there are neither scalarizable arrays (SROA path) nor rotate builtins (crypto path).
    if scalarizable.is_empty() && rotate_syms.is_empty() {
        return (stmts, false);
    }
    let mut budget = TOTAL_EXPANSION_BUDGET;
    let out = unroll_block(
        &stmts,
        entry_in_loop,
        &mut budget,
        &scalarizable,
        &rotate_syms,
        expr_arena,
        stmt_arena,
    );
    if budget == TOTAL_EXPANSION_BUDGET {
        // Budget untouched ⇒ no loop was unrolled ⇒ return the input verbatim.
        (stmts, false)
    } else {
        (out, true)
    }
}

struct UnrollPlan {
    /// The induction-variable values, in iteration order.
    values: Vec<i64>,
    /// The counter's value after the loop exits (for the `While` form).
    final_val: i64,
}

/// Decide whether `cl` should be unrolled and, if so, enumerate its iterations.
fn plan_unroll(
    cl: &CountedLoop,
    in_loop: bool,
    budget: usize,
    scalarizable: &HashSet<Symbol>,
    rotate_syms: &HashSet<Symbol>,
) -> Option<UnrollPlan> {
    let start = const_eval_i64(cl.start)?;
    let limit = const_eval_i64(cl.limit)?;
    let span = if cl.inclusive {
        limit.checked_sub(start)?.checked_add(1)?
    } else {
        limit.checked_sub(start)?
    };
    let tc = span.max(0);
    let tcu = tc as usize;
    if tcu > TRIP_THRESHOLD || tcu > budget {
        return None;
    }
    // A `Push` in the body means the loop grows a buffer — a buffer-reuse shape,
    // not a fixed-array kernel. Never unroll it.
    if body_has_push(cl.body_without_increment) {
        return None;
    }
    // Every statement must be one the substitution engine fully rewrites, and
    // the loop must not own a `Break` (there is no loop to break out of once
    // unrolled).
    if !body_substitutable(cl.body_without_increment) || owns_break(cl.body_without_increment) {
        return None;
    }

    // The crypto rotate path: the induction variable feeds a `rotl`/`rotr` shift amount, so unrolling
    // makes every data-dependent rotate a CONSTANT `rol` (a runtime table-load + variable-count rotate
    // otherwise) — the reason hand-written block ciphers/hashes fully unroll. LLVM keeps these rolled
    // (its unroller can't see the win), so we do it regardless of nesting or of what the loop indexes.
    // The no-Push / no-Break / fully-substitutable guards above already make it value-safe.
    let crypto = loop_rotates_by_ivar(cl.body_without_increment, cl.var, rotate_syms);

    if !crypto {
        // The SROA path: only NESTED loops (top-level constant loops are LLVM's job) indexing exclusively
        // fixed-size scalarizable `[T; N]` arrays. A loop over a reference-semantics `LogosSeq` (a
        // push-grown buffer or DP row) is left rolled — it belongs to de-Rc/borrow-hoist, and unrolling
        // both breaks those passes and buys no SROA.
        if !in_loop {
            return None;
        }
        let mut roots = HashSet::new();
        collect_indexed_roots(cl.body_without_increment, &mut roots);
        if roots.is_empty() || !roots.iter().all(|r| scalarizable.contains(r)) {
            return None;
        }
    }
    let final_val = start.checked_add(tc)?;
    let values: Vec<i64> = (0..tcu).map(|k| start + k as i64).collect();
    Some(UnrollPlan { values, final_val })
}

fn unroll_block<'a>(
    block: &[Stmt<'a>],
    in_loop: bool,
    budget: &mut usize,
    scalarizable: &HashSet<Symbol>,
    rotate_syms: &HashSet<Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let refs: Vec<&Stmt<'a>> = block.iter().collect();
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(block.len());
    let mut idx = 0;
    while idx < block.len() {
        // Extraction is attempted regardless of nesting: `plan_unroll` enforces the nested-only rule for
        // the SROA path and lifts it for the crypto rotate path.
        {
            // Counted-`While`: an init `Let`/`Set` at idx then `While` at idx+1.
            if let Some((cl, _consumed)) = extract_counted_while(&refs, idx) {
                if let Some(plan) = plan_unroll(&cl, in_loop, *budget, scalarizable, rotate_syms) {
                    *budget -= plan.values.len();
                    // Keep the init binding so the counter stays declared and
                    // mutable for the trailing reset (and any later reuse).
                    out.push(block[idx].clone());
                    emit_unrolled(&cl, &plan, &mut out, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
                    let fin = expr_arena.alloc(Expr::Literal(Literal::Number(plan.final_val)));
                    out.push(Stmt::Set { target: cl.var, value: fin });
                    idx += 2;
                    continue;
                }
            }
            // Counted-`Repeat` over an inclusive constant range.
            if let Some(cl) = extract_counted_repeat(&block[idx]) {
                if let Some(plan) = plan_unroll(&cl, in_loop, *budget, scalarizable, rotate_syms) {
                    *budget -= plan.values.len();
                    emit_unrolled(&cl, &plan, &mut out, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
                    idx += 1;
                    continue;
                }
            }
        }
        out.push(descend_stmt(&block[idx], in_loop, budget, scalarizable, rotate_syms, expr_arena, stmt_arena));
        idx += 1;
    }
    out
}

/// Emit the unrolled body: for each iteration value, substitute the induction
/// variable with that literal and recursively unroll the result (so inner
/// triangular loops, whose bounds become constant once the outer variable is a
/// literal, unroll on the same pass).
fn emit_unrolled<'a>(
    cl: &CountedLoop<'a>,
    plan: &UnrollPlan,
    out: &mut Vec<Stmt<'a>>,
    budget: &mut usize,
    scalarizable: &HashSet<Symbol>,
    rotate_syms: &HashSet<Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) {
    for &v in &plan.values {
        let lit: &'a Expr<'a> = expr_arena.alloc(Expr::Literal(Literal::Number(v)));
        let mut subs: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
        subs.insert(cl.var, lit);
        let sub_body = substitute_block(cl.body_without_increment, &subs, expr_arena, stmt_arena);
        let unrolled = unroll_block(&sub_body, true, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
        out.extend(unrolled);
    }
}

/// Rebuild a statement, recursively unrolling its child blocks. Entering any
/// loop body sets `in_loop = true`; a function body resets it to `false`
/// (a constant loop at the top of a function is, again, LLVM's job).
fn descend_stmt<'a>(
    s: &Stmt<'a>,
    in_loop: bool,
    budget: &mut usize,
    scalarizable: &HashSet<Symbol>,
    rotate_syms: &HashSet<Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    match s {
        Stmt::While { cond, body, decreasing } => {
            let nb = unroll_block(body, true, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
            Stmt::While { cond: *cond, body: stmt_arena.alloc_slice(nb), decreasing: *decreasing }
        }
        Stmt::Repeat { pattern, iterable, body } => {
            let nb = unroll_block(body, true, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
            Stmt::Repeat {
                pattern: pattern.clone(),
                iterable: *iterable,
                body: stmt_arena.alloc_slice(nb),
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            let nt = unroll_block(then_block, in_loop, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
            let ne = match else_block {
                Some(eb) => {
                    let neb = unroll_block(eb, in_loop, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
                    Some(stmt_arena.alloc_slice(neb) as &[Stmt<'a>])
                }
                None => None,
            };
            Stmt::If { cond: *cond, then_block: stmt_arena.alloc_slice(nt), else_block: ne }
        }
        Stmt::Inspect { target, arms, has_otherwise } => {
            let na = arms
                .iter()
                .map(|a| MatchArm {
                    enum_name: a.enum_name,
                    variant: a.variant,
                    bindings: a.bindings.clone(),
                    body: stmt_arena.alloc_slice(unroll_block(
                        a.body, in_loop, budget, scalarizable, rotate_syms, expr_arena, stmt_arena,
                    )),
                })
                .collect();
            Stmt::Inspect { target: *target, arms: na, has_otherwise: *has_otherwise }
        }
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
        } => {
            let nb = unroll_block(body, false, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
            Stmt::FunctionDef {
                name: *name,
                generics: generics.clone(),
                params: params.clone(),
                body: stmt_arena.alloc_slice(nb),
                return_type: *return_type,
                is_native: *is_native,
                native_path: *native_path,
                is_exported: *is_exported,
                export_target: *export_target,
                opt_flags: opt_flags.clone(),
            }
        }
        Stmt::Zone { name, capacity, source_file, body } => {
            let nb = unroll_block(body, in_loop, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
            Stmt::Zone {
                name: *name,
                capacity: *capacity,
                source_file: *source_file,
                body: stmt_arena.alloc_slice(nb),
            }
        }
        Stmt::Concurrent { tasks } => {
            let nb = unroll_block(tasks, in_loop, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
            Stmt::Concurrent { tasks: stmt_arena.alloc_slice(nb) }
        }
        Stmt::Parallel { tasks } => {
            let nb = unroll_block(tasks, in_loop, budget, scalarizable, rotate_syms, expr_arena, stmt_arena);
            Stmt::Parallel { tasks: stmt_arena.alloc_slice(nb) }
        }
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// Every statement in `stmts` (and their nested blocks) must be one that
/// `partial_eval::substitute_stmt` fully rewrites — otherwise its catch-all
/// `clone()` would copy a statement WITHOUT substituting the induction variable
/// inside it. Fail-closed: any unrecognized statement kind aborts unrolling.
fn body_substitutable(stmts: &[Stmt]) -> bool {
    stmts.iter().all(stmt_substitutable)
}

fn stmt_substitutable(s: &Stmt) -> bool {
    match s {
        Stmt::Let { .. }
        | Stmt::Set { .. }
        | Stmt::Return { .. }
        | Stmt::Show { .. }
        | Stmt::Call { .. }
        | Stmt::SetIndex { .. }
        | Stmt::Push { .. }
        | Stmt::SetField { .. }
        | Stmt::Give { .. }
        | Stmt::Add { .. }
        | Stmt::Remove { .. }
        | Stmt::RuntimeAssert { .. }
        | Stmt::Break => true,
        Stmt::If { then_block, else_block, .. } => {
            body_substitutable(then_block)
                && match else_block {
                    Some(eb) => body_substitutable(eb),
                    None => true,
                }
        }
        Stmt::While { body, .. } => body_substitutable(body),
        Stmt::Repeat { body, .. } => body_substitutable(body),
        Stmt::Inspect { arms, .. } => arms.iter().all(|a| body_substitutable(a.body)),
        _ => false,
    }
}

/// True if `stmts` contains a `Break` that belongs to the loop being unrolled —
/// i.e. one not enclosed in a nested `While`/`Repeat` (which capture their own).
fn owns_break(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Break => true,
        Stmt::If { then_block, else_block, .. } => {
            owns_break(then_block) || matches!(else_block, Some(eb) if owns_break(eb))
        }
        Stmt::Inspect { arms, .. } => arms.iter().any(|a| owns_break(a.body)),
        _ => false,
    })
}

/// True if any statement (recursively) pushes onto a collection — the signature
/// of a buffer-reuse / fill loop, which must keep its loop shape.
fn body_has_push(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Push { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            body_has_push(then_block) || matches!(else_block, Some(eb) if body_has_push(eb))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => body_has_push(body),
        Stmt::Inspect { arms, .. } => arms.iter().any(|a| body_has_push(a.body)),
        _ => false,
    })
}

/// The crypto-round signature: some statement in `body` (recursively) contains a rotate (`rotl`/`rotr`)
/// whose shift-amount argument mentions the induction variable `ivar`. Unrolling then makes `ivar` a
/// literal in every copy, so each formerly data-dependent rotate folds to a constant `rol` — the payoff
/// hand-written block ciphers/hashes chase by fully unrolling. A rotate by an ALREADY-constant amount is
/// excluded: it needs no unrolling to be constant.
fn loop_rotates_by_ivar(body: &[Stmt], ivar: Symbol, rotate_syms: &HashSet<Symbol>) -> bool {
    body.iter().any(|s| stmt_rotates_by_ivar(s, ivar, rotate_syms))
}

fn stmt_rotates_by_ivar(s: &Stmt, ivar: Symbol, rot: &HashSet<Symbol>) -> bool {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } | Stmt::Show { object: value, .. } => {
            expr_rotates_by_ivar(value, ivar, rot)
        }
        Stmt::SetIndex { collection, index, value } => {
            expr_rotates_by_ivar(collection, ivar, rot)
                || expr_rotates_by_ivar(index, ivar, rot)
                || expr_rotates_by_ivar(value, ivar, rot)
        }
        Stmt::SetField { object, value, .. } => {
            expr_rotates_by_ivar(object, ivar, rot) || expr_rotates_by_ivar(value, ivar, rot)
        }
        Stmt::Push { value, collection } => {
            expr_rotates_by_ivar(value, ivar, rot) || expr_rotates_by_ivar(collection, ivar, rot)
        }
        Stmt::Return { value } => matches!(value, Some(v) if expr_rotates_by_ivar(v, ivar, rot)),
        Stmt::Call { args, .. } => args.iter().any(|a| expr_rotates_by_ivar(a, ivar, rot)),
        Stmt::RuntimeAssert { condition, .. } => expr_rotates_by_ivar(condition, ivar, rot),
        Stmt::If { cond, then_block, else_block } => {
            expr_rotates_by_ivar(cond, ivar, rot)
                || loop_rotates_by_ivar(then_block, ivar, rot)
                || matches!(else_block, Some(eb) if loop_rotates_by_ivar(eb, ivar, rot))
        }
        Stmt::While { cond, body, .. } => {
            expr_rotates_by_ivar(cond, ivar, rot) || loop_rotates_by_ivar(body, ivar, rot)
        }
        Stmt::Repeat { iterable, body, .. } => {
            expr_rotates_by_ivar(iterable, ivar, rot) || loop_rotates_by_ivar(body, ivar, rot)
        }
        Stmt::Inspect { target, arms, .. } => {
            expr_rotates_by_ivar(target, ivar, rot)
                || arms.iter().any(|a| loop_rotates_by_ivar(a.body, ivar, rot))
        }
        _ => false,
    }
}

fn expr_rotates_by_ivar(e: &Expr, ivar: Symbol, rot: &HashSet<Symbol>) -> bool {
    match e {
        Expr::Call { function, args } => {
            if rot.contains(function) {
                if let Some(shift) = args.get(1) {
                    if expr_mentions(shift, ivar) {
                        return true;
                    }
                }
            }
            args.iter().any(|a| expr_rotates_by_ivar(a, ivar, rot))
        }
        Expr::CallExpr { callee, args } => {
            expr_rotates_by_ivar(callee, ivar, rot) || args.iter().any(|a| expr_rotates_by_ivar(a, ivar, rot))
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            expr_rotates_by_ivar(left, ivar, rot) || expr_rotates_by_ivar(right, ivar, rot)
        }
        Expr::Index { collection, index } => {
            expr_rotates_by_ivar(collection, ivar, rot) || expr_rotates_by_ivar(index, ivar, rot)
        }
        Expr::Slice { collection, start, end } => {
            expr_rotates_by_ivar(collection, ivar, rot)
                || expr_rotates_by_ivar(start, ivar, rot)
                || expr_rotates_by_ivar(end, ivar, rot)
        }
        Expr::Not { operand } => expr_rotates_by_ivar(operand, ivar, rot),
        Expr::Length { collection } => expr_rotates_by_ivar(collection, ivar, rot),
        Expr::Contains { collection, value } => {
            expr_rotates_by_ivar(collection, ivar, rot) || expr_rotates_by_ivar(value, ivar, rot)
        }
        Expr::FieldAccess { object, .. } => expr_rotates_by_ivar(object, ivar, rot),
        Expr::Copy { expr } => expr_rotates_by_ivar(expr, ivar, rot),
        Expr::Give { value } | Expr::OptionSome { value } => expr_rotates_by_ivar(value, ivar, rot),
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| expr_rotates_by_ivar(i, ivar, rot)),
        _ => false,
    }
}

/// `sym` appears anywhere in `e`. Conservative under partial coverage: an unrecognized node returns
/// `false`, so a missed mention only DECLINES a rotate (never unrolls a non-crypto loop unsoundly).
fn expr_mentions(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == sym,
        Expr::Call { args, .. } => args.iter().any(|a| expr_mentions(a, sym)),
        Expr::CallExpr { callee, args } => {
            expr_mentions(callee, sym) || args.iter().any(|a| expr_mentions(a, sym))
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            expr_mentions(left, sym) || expr_mentions(right, sym)
        }
        Expr::Index { collection, index } => expr_mentions(collection, sym) || expr_mentions(index, sym),
        Expr::Slice { collection, start, end } => {
            expr_mentions(collection, sym) || expr_mentions(start, sym) || expr_mentions(end, sym)
        }
        Expr::Not { operand } => expr_mentions(operand, sym),
        Expr::Length { collection } => expr_mentions(collection, sym),
        Expr::Contains { collection, value } => expr_mentions(collection, sym) || expr_mentions(value, sym),
        Expr::FieldAccess { object, .. } => expr_mentions(object, sym),
        Expr::Copy { expr } => expr_mentions(expr, sym),
        Expr::Give { value } | Expr::OptionSome { value } => expr_mentions(value, sym),
        Expr::WithCapacity { value, capacity } => expr_mentions(value, sym) || expr_mentions(capacity, sym),
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| expr_mentions(i, sym)),
        Expr::InterpolatedString(parts) => parts.iter().any(|p| {
            matches!(p, crate::ast::stmt::StringPart::Expr { value, .. } if expr_mentions(value, sym))
        }),
        _ => false,
    }
}

/// The root collection symbol an index expression ultimately addresses, peeling
/// nested `Index`/`Slice`/`FieldAccess` (e.g. `item 1 of (item w of curr)` → `curr`).
fn root_symbol(e: &Expr) -> Option<Symbol> {
    match e {
        Expr::Identifier(s) => Some(*s),
        Expr::Index { collection, .. } => root_symbol(collection),
        Expr::Slice { collection, .. } => root_symbol(collection),
        Expr::FieldAccess { object, .. } => root_symbol(object),
        _ => None,
    }
}

/// Collect the root collection symbols of every `Index`/`Slice`/`SetIndex` in
/// `stmts` (recursively). Empty ⇒ the loop indexes nothing.
fn collect_indexed_roots(stmts: &[Stmt], out: &mut HashSet<Symbol>) {
    for s in stmts {
        stmt_indexed_roots(s, out);
    }
}

fn stmt_indexed_roots(s: &Stmt, out: &mut HashSet<Symbol>) {
    match s {
        Stmt::SetIndex { collection, index, value } => {
            if let Some(r) = root_symbol(collection) {
                out.insert(r);
            }
            expr_indexed_roots(collection, out);
            expr_indexed_roots(index, out);
            expr_indexed_roots(value, out);
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_indexed_roots(value, out),
        Stmt::Push { value, collection } => {
            expr_indexed_roots(value, out);
            expr_indexed_roots(collection, out);
        }
        Stmt::Show { object, .. } => expr_indexed_roots(object, out),
        Stmt::SetField { object, value, .. } => {
            expr_indexed_roots(object, out);
            expr_indexed_roots(value, out);
        }
        Stmt::Give { object, recipient } => {
            expr_indexed_roots(object, out);
            expr_indexed_roots(recipient, out);
        }
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            expr_indexed_roots(value, out);
            expr_indexed_roots(collection, out);
        }
        Stmt::RuntimeAssert { condition, .. } => expr_indexed_roots(condition, out),
        Stmt::Return { value } => {
            if let Some(v) = value {
                expr_indexed_roots(v, out);
            }
        }
        Stmt::Call { args, .. } => {
            for a in args {
                expr_indexed_roots(a, out);
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_indexed_roots(cond, out);
            collect_indexed_roots(then_block, out);
            if let Some(eb) = else_block {
                collect_indexed_roots(eb, out);
            }
        }
        Stmt::While { cond, body, .. } => {
            expr_indexed_roots(cond, out);
            collect_indexed_roots(body, out);
        }
        Stmt::Repeat { iterable, body, .. } => {
            expr_indexed_roots(iterable, out);
            collect_indexed_roots(body, out);
        }
        Stmt::Inspect { target, arms, .. } => {
            expr_indexed_roots(target, out);
            for a in arms {
                collect_indexed_roots(a.body, out);
            }
        }
        _ => {}
    }
}

fn expr_indexed_roots(e: &Expr, out: &mut HashSet<Symbol>) {
    match e {
        Expr::Index { collection, index } => {
            if let Some(r) = root_symbol(collection) {
                out.insert(r);
            }
            expr_indexed_roots(collection, out);
            expr_indexed_roots(index, out);
        }
        Expr::Slice { collection, start, end } => {
            if let Some(r) = root_symbol(collection) {
                out.insert(r);
            }
            expr_indexed_roots(collection, out);
            expr_indexed_roots(start, out);
            expr_indexed_roots(end, out);
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_indexed_roots(left, out);
            expr_indexed_roots(right, out);
        }
        Expr::Not { operand } => expr_indexed_roots(operand, out),
        Expr::Length { collection } => expr_indexed_roots(collection, out),
        Expr::Contains { collection, value } => {
            expr_indexed_roots(collection, out);
            expr_indexed_roots(value, out);
        }
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            expr_indexed_roots(left, out);
            expr_indexed_roots(right, out);
        }
        Expr::Call { args, .. } | Expr::CallExpr { args, .. } => {
            for a in args {
                expr_indexed_roots(a, out);
            }
        }
        Expr::FieldAccess { object, .. } => expr_indexed_roots(object, out),
        Expr::Copy { expr } => expr_indexed_roots(expr, out),
        Expr::Give { value } | Expr::OptionSome { value } => expr_indexed_roots(value, out),
        Expr::WithCapacity { value, capacity } => {
            expr_indexed_roots(value, out);
            expr_indexed_roots(capacity, out);
        }
        Expr::Range { start, end } => {
            expr_indexed_roots(start, out);
            expr_indexed_roots(end, out);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                expr_indexed_roots(i, out);
            }
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    expr_indexed_roots(value, out);
                }
            }
        }
        _ => {}
    }
}
