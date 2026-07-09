//! Loop-Invariant Code Motion (LICM).
//!
//! Hoists immutable Let statements from loop bodies when their value expressions
//! only read variables that are not written in the loop body. This avoids
//! recomputing invariant expressions on every iteration.

use std::collections::HashSet;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Stmt};
use crate::intern::{Interner, Symbol};

/// Collect all symbols written in a block of statements (recursively).
pub(crate) fn collect_loop_writes(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut writes = HashSet::new();
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, .. } => {
                writes.insert(*target);
            }
            // A Let introduces an ITERATION-LOCAL binding: any expression
            // reading it is body-dependent, never invariant. (Treating it
            // as a "write" is exactly the right approximation — hoisting a
            // reader above the loop would read a name that does not exist
            // there: the nbody `mag`/`dist` miscompilation.)
            Stmt::Let { var, .. } => {
                writes.insert(*var);
            }
            Stmt::ReadFrom { var, .. } => {
                writes.insert(*var);
            }
            Stmt::Push { collection, .. }
            | Stmt::Add { collection, .. }
            | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    writes.insert(*sym);
                }
            }
            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(sym) = collection {
                    writes.insert(*sym);
                }
                if let Some(v) = into {
                    writes.insert(*v);
                }
            }
            Stmt::SetIndex { collection, .. } | Stmt::SetField { object: collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    writes.insert(*sym);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                writes.extend(collect_loop_writes(then_block));
                if let Some(eb) = else_block {
                    writes.extend(collect_loop_writes(eb));
                }
            }
            Stmt::While { body, .. } => {
                writes.extend(collect_loop_writes(body));
            }
            Stmt::Repeat { body, pattern, .. } => {
                use crate::ast::stmt::Pattern;
                match pattern {
                    Pattern::Identifier(sym) => {
                        writes.insert(*sym);
                    }
                    Pattern::Tuple(syms) => {
                        writes.extend(syms.iter().copied());
                    }
                }
                writes.extend(collect_loop_writes(body));
            }
            Stmt::Zone { body, .. } => {
                writes.extend(collect_loop_writes(body));
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    writes.extend(collect_loop_writes(arm.body));
                }
            }
            _ => {}
        }
    }
    writes
}

/// Check if an expression is loop-invariant: only reads symbols NOT in loop_writes.
/// Returns false for expressions with potential side effects (calls, allocs, escape).
pub(crate) fn is_loop_invariant(expr: &Expr, loop_writes: &HashSet<Symbol>) -> bool {
    match expr {
        Expr::Literal(_) => true,
        Expr::Identifier(sym) => !loop_writes.contains(sym),
        Expr::BinaryOp { left, right, .. } => {
            is_loop_invariant(left, loop_writes) && is_loop_invariant(right, loop_writes)
        }
        Expr::Length { collection } => is_loop_invariant(collection, loop_writes),
        Expr::Not { operand } => is_loop_invariant(operand, loop_writes),
        Expr::FieldAccess { object, .. } => is_loop_invariant(object, loop_writes),
        Expr::Index { collection, index } => {
            is_loop_invariant(collection, loop_writes) && is_loop_invariant(index, loop_writes)
        }
        Expr::Contains { collection, value } => {
            is_loop_invariant(collection, loop_writes) && is_loop_invariant(value, loop_writes)
        }
        // Don't hoist: function calls, constructors, escape blocks, allocs, closures
        _ => false,
    }
}

/// Returns true if the expression is non-trivial (worth hoisting).
fn is_worth_hoisting(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::BinaryOp { .. }
        | Expr::Length { .. }
        | Expr::Not { .. }
        | Expr::Index { .. }
        | Expr::Contains { .. }
        | Expr::FieldAccess { .. }
    )
}

/// True when evaluating `expr` has NO side effects and cannot panic on its own
/// — so it is safe to evaluate it an EXTRA time (the guarded-hoist duplicates
/// the loop condition). Identifiers, literals, and arithmetic/relational/logical
/// combinations of safe sub-expressions qualify; calls, indexing (OOB panic),
/// length (the collection might not exist yet — conservative), and everything
/// else fail closed.
fn is_side_effect_free(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone => true,
        Expr::BinaryOp { op, left, right } => {
            // Division/modulo can panic on a zero divisor — not safe to double.
            !matches!(op, BinaryOpKind::Divide | BinaryOpKind::Modulo)
                && is_side_effect_free(left)
                && is_side_effect_free(right)
        }
        Expr::Not { operand } => is_side_effect_free(operand),
        _ => false,
    }
}

/// True when `expr` is a loop-invariant indexed READ worth hoisting:
/// `item <idx> of <coll>` where both `coll` and `idx` only read names that the
/// loop never writes, AND `coll` is a bare identifier the loop never mutates
/// (the `collect_loop_writes` set already records `SetIndex`/`Push`/`Pop`/
/// `Add`/`Remove`/`Set`/`Let` on an identifier collection, so "not in
/// loop_writes" proves the backing collection is stable across the loop).
fn is_hoistable_indexed_load(expr: &Expr, loop_writes: &HashSet<Symbol>) -> bool {
    match expr {
        Expr::Index { collection, index } => {
            matches!(&**collection, Expr::Identifier(sym) if !loop_writes.contains(sym))
                && is_loop_invariant(index, loop_writes)
        }
        _ => false,
    }
}

/// Structural equality for the indexed-load expressions we hoist. Two reads
/// `item i of arr` are "the same load" when collection and index match
/// structurally — so every occurrence in the body folds onto one hoisted Let.
fn loads_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Literal(x), Expr::Literal(y)) => literals_equal(x, y),
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        (
            Expr::BinaryOp { op: o1, left: l1, right: r1 },
            Expr::BinaryOp { op: o2, left: l2, right: r2 },
        ) => o1 == o2 && loads_equal(l1, l2) && loads_equal(r1, r2),
        (Expr::Not { operand: o1 }, Expr::Not { operand: o2 }) => loads_equal(o1, o2),
        (
            Expr::Index { collection: c1, index: i1 },
            Expr::Index { collection: c2, index: i2 },
        ) => loads_equal(c1, c2) && loads_equal(i1, i2),
        _ => false,
    }
}

fn literals_equal(a: &crate::ast::stmt::Literal, b: &crate::ast::stmt::Literal) -> bool {
    use crate::ast::stmt::Literal::*;
    match (a, b) {
        (Number(x), Number(y)) => x == y,
        (Float(x), Float(y)) => x.to_bits() == y.to_bits(),
        (Text(x), Text(y)) => x == y,
        (Boolean(x), Boolean(y)) => x == y,
        (Char(x), Char(y)) => x == y,
        (Nothing, Nothing) => true,
        (Duration(x), Duration(y)) => x == y,
        (Date(x), Date(y)) => x == y,
        (Moment(x), Moment(y)) => x == y,
        (Span { months: m1, days: d1 }, Span { months: m2, days: d2 }) => m1 == m2 && d1 == d2,
        (Time(x), Time(y)) => x == y,
        _ => false,
    }
}

/// Walk `expr`, collecting every distinct hoistable invariant indexed load.
fn collect_indexed_loads<'a>(
    expr: &'a Expr<'a>,
    loop_writes: &HashSet<Symbol>,
    out: &mut Vec<&'a Expr<'a>>,
) {
    if is_hoistable_indexed_load(expr, loop_writes) {
        if !out.iter().any(|e| loads_equal(e, expr)) {
            out.push(expr);
        }
        return; // index sub-tree is invariant scalars; no nested array loads to find
    }
    match expr {
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            collect_indexed_loads(left, loop_writes, out);
            collect_indexed_loads(right, loop_writes, out);
        }
        Expr::Not { operand } => collect_indexed_loads(operand, loop_writes, out),
        Expr::Index { collection, index } => {
            collect_indexed_loads(collection, loop_writes, out);
            collect_indexed_loads(index, loop_writes, out);
        }
        Expr::Slice { collection, start, end } => {
            collect_indexed_loads(collection, loop_writes, out);
            collect_indexed_loads(start, loop_writes, out);
            collect_indexed_loads(end, loop_writes, out);
        }
        Expr::Length { collection } => collect_indexed_loads(collection, loop_writes, out),
        Expr::Contains { collection, value } => {
            collect_indexed_loads(collection, loop_writes, out);
            collect_indexed_loads(value, loop_writes, out);
        }
        Expr::Call { args, .. } => {
            for a in args {
                collect_indexed_loads(a, loop_writes, out);
            }
        }
        Expr::CallExpr { callee, args } => {
            collect_indexed_loads(callee, loop_writes, out);
            for a in args {
                collect_indexed_loads(a, loop_writes, out);
            }
        }
        Expr::Copy { expr } | Expr::Give { value: expr } => {
            collect_indexed_loads(expr, loop_writes, out)
        }
        Expr::FieldAccess { object, .. } => collect_indexed_loads(object, loop_writes, out),
        _ => {}
    }
}

/// Rewrite every occurrence of `target` (one specific hoistable load) in `expr`
/// to `Identifier(name)`, allocating new nodes only along the changed spine.
fn replace_load<'a>(
    expr: &'a Expr<'a>,
    target: &Expr<'a>,
    name: Symbol,
    expr_arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    if loads_equal(expr, target) {
        return expr_arena.alloc(Expr::Identifier(name));
    }
    match expr {
        Expr::BinaryOp { op, left, right } => {
            let l = replace_load(left, target, name, expr_arena);
            let r = replace_load(right, target, name, expr_arena);
            expr_arena.alloc(Expr::BinaryOp { op: *op, left: l, right: r })
        }
        Expr::Not { operand } => {
            let o = replace_load(operand, target, name, expr_arena);
            expr_arena.alloc(Expr::Not { operand: o })
        }
        Expr::Index { collection, index } => {
            let c = replace_load(collection, target, name, expr_arena);
            let i = replace_load(index, target, name, expr_arena);
            expr_arena.alloc(Expr::Index { collection: c, index: i })
        }
        Expr::Slice { collection, start, end } => {
            let c = replace_load(collection, target, name, expr_arena);
            let s = replace_load(start, target, name, expr_arena);
            let e = replace_load(end, target, name, expr_arena);
            expr_arena.alloc(Expr::Slice { collection: c, start: s, end: e })
        }
        Expr::Length { collection } => {
            let c = replace_load(collection, target, name, expr_arena);
            expr_arena.alloc(Expr::Length { collection: c })
        }
        Expr::Contains { collection, value } => {
            let c = replace_load(collection, target, name, expr_arena);
            let v = replace_load(value, target, name, expr_arena);
            expr_arena.alloc(Expr::Contains { collection: c, value: v })
        }
        Expr::Copy { expr: inner } => {
            let e = replace_load(inner, target, name, expr_arena);
            expr_arena.alloc(Expr::Copy { expr: e })
        }
        Expr::Give { value } => {
            let v = replace_load(value, target, name, expr_arena);
            expr_arena.alloc(Expr::Give { value: v })
        }
        Expr::Call { function, args } => {
            let new_args: Vec<&Expr> =
                args.iter().map(|a| replace_load(a, target, name, expr_arena)).collect();
            expr_arena.alloc(Expr::Call { function: *function, args: new_args })
        }
        Expr::CallExpr { callee, args } => {
            let c = replace_load(callee, target, name, expr_arena);
            let new_args: Vec<&Expr> =
                args.iter().map(|a| replace_load(a, target, name, expr_arena)).collect();
            expr_arena.alloc(Expr::CallExpr { callee: c, args: new_args })
        }
        _ => expr,
    }
}

/// Rewrite every occurrence of `target` across a whole statement subtree.
/// Only the statement forms whose value expressions can carry a hoistable
/// load are rebuilt; the loop body we feed this is already a flat region.
fn replace_load_in_stmt<'a>(
    stmt: Stmt<'a>,
    target: &Expr<'a>,
    name: Symbol,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    let r = |e: &'a Expr<'a>| replace_load(e, target, name, expr_arena);
    let rb = |b: Block<'a>, sa: &'a Arena<Stmt<'a>>| -> Block<'a> {
        let v: Vec<Stmt> = b
            .iter()
            .cloned()
            .map(|s| replace_load_in_stmt(s, target, name, expr_arena, sa))
            .collect();
        sa.alloc_slice(v)
    };
    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            Stmt::Let { var, ty, value: r(value), mutable }
        }
        Stmt::Set { target: t, value } => Stmt::Set { target: t, value: r(value) },
        Stmt::SetIndex { collection, index, value } => Stmt::SetIndex {
            collection: r(collection),
            index: r(index),
            value: r(value),
        },
        Stmt::SetField { object, field, value } => {
            Stmt::SetField { object: r(object), field, value: r(value) }
        }
        Stmt::Push { value, collection } => {
            Stmt::Push { value: r(value), collection: r(collection) }
        }
        Stmt::Add { value, collection } => {
            Stmt::Add { value: r(value), collection: r(collection) }
        }
        Stmt::Remove { value, collection } => {
            Stmt::Remove { value: r(value), collection: r(collection) }
        }
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond: r(cond),
            then_block: rb(then_block, stmt_arena),
            else_block: else_block.map(|eb| rb(eb, stmt_arena)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond: r(cond),
            body: rb(body, stmt_arena),
            decreasing: decreasing.map(r),
        },
        other => other,
    }
}

/// Hoist loop-invariant indexed loads out of `loop_stmt` (a `While`) into its
/// preheader, returning `If cond { <hoisted Lets…> <rewritten loop> }` when at
/// least one load is hoisted, or `None` otherwise. The `If cond` guard makes
/// the hoist semantics-preserving on a zero-trip loop (the loads never run, as
/// in the source), and is only applied when `cond` is side-effect-free so the
/// duplicated test is observationally identical.
fn hoist_invariant_loads<'a>(
    loop_stmt: Stmt<'a>,
    cond: &'a Expr<'a>,
    loop_writes: &HashSet<Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Option<Stmt<'a>> {
    if !is_side_effect_free(cond) {
        return None;
    }
    let body = match &loop_stmt {
        Stmt::While { body, .. } => *body,
        _ => return None,
    };

    // Gather distinct hoistable loads ONLY from the prefix of top-level body
    // statements that is GUARANTEED to execute when the loop body runs once —
    // i.e. up to (and excluding) the first statement that can divert control
    // (Break/Return/Fail) or that hides one in a nested block. A load there
    // executes on the first iteration whenever the loop runs, so guarding the
    // hoist with `If cond` reproduces the source's evaluation EXACTLY: it ran
    // (≥1 trip) or it didn't (0 trip), and an OOB panic is preserved on both
    // sides. Loads beyond that prefix, or inside conditionals, are left in
    // place (they might never execute in the source).
    let mut loads: Vec<&Expr> = Vec::new();
    for s in body {
        if !stmt_runs_unconditionally(s) {
            break;
        }
        for e in stmt_value_exprs(s) {
            collect_indexed_loads(e, loop_writes, &mut loads);
        }
    }
    if loads.is_empty() {
        return None;
    }

    // For each load, mint a fresh Let in the preheader and rewrite the loop.
    let mut hoisted: Vec<Stmt<'a>> = Vec::with_capacity(loads.len());
    let mut current = loop_stmt;
    for (k, load) in loads.iter().enumerate() {
        let name = interner.intern(&format!("__licm_load_{}", fresh_id(interner, k)));
        hoisted.push(Stmt::Let {
            var: name,
            ty: None,
            value: load,
            mutable: false,
        });
        current = replace_load_in_stmt(current, load, name, expr_arena, stmt_arena);
    }

    let mut guarded_body = hoisted;
    guarded_body.push(current);
    Some(Stmt::If {
        cond,
        then_block: stmt_arena.alloc_slice(guarded_body),
        else_block: None,
    })
}

/// A monotonically unique suffix for hoisted-load names — interning the same
/// string twice returns the same Symbol, so we vary it by a global counter and
/// the per-call index to guarantee distinct bindings.
fn fresh_id(_interner: &Interner, k: usize) -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{n}_{k}")
}

/// The UNCONDITIONALLY-evaluated value expressions of a single statement (so
/// we can scan them for hoist candidates). Nested If/While BODIES are NOT
/// scanned — a load inside a conditional/inner loop may never run in the
/// source, so hoisting it (and risking an OOB panic) would be unsound. Only
/// the statement's own straight-line value expressions are collected.
fn stmt_value_exprs<'a>(stmt: &Stmt<'a>) -> Vec<&'a Expr<'a>> {
    let mut out: Vec<&Expr> = Vec::new();
    match stmt {
        Stmt::Let { value, .. } => out.push(value),
        Stmt::Set { value, .. } => out.push(value),
        Stmt::SetIndex { collection, index, value } => {
            out.push(collection);
            out.push(index);
            out.push(value);
        }
        Stmt::SetField { object, value, .. } => {
            out.push(object);
            out.push(value);
        }
        Stmt::Push { value, collection } | Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            out.push(value);
            out.push(collection);
        }
        _ => {}
    }
    out
}

/// True when reaching this statement (at the top level of a loop body) does NOT
/// itself divert control before its successors — so the next top-level
/// statement still runs on the first iteration. Straight-line value statements
/// qualify; anything that can `Break`/`Return`/`Fail` (directly or nested),
/// and any unrecognized form, fails closed so the unconditional-prefix scan
/// stops there.
fn stmt_runs_unconditionally(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::Let { .. }
            | Stmt::Set { .. }
            | Stmt::SetIndex { .. }
            | Stmt::SetField { .. }
            | Stmt::Push { .. }
            | Stmt::Add { .. }
            | Stmt::Remove { .. }
    )
}

/// Check if an expression is an equality comparison of a symbol with a literal.
/// Returns (symbol, literal_value) if matched.
fn extract_eq_boundary(expr: &Expr) -> Option<(Symbol, i64)> {
    match expr {
        Expr::BinaryOp { op: crate::ast::stmt::BinaryOpKind::Eq, left, right } => {
            // sym == literal
            if let (Expr::Identifier(sym), Expr::Literal(crate::ast::stmt::Literal::Number(n))) = (&**left, &**right) {
                return Some((*sym, *n));
            }
            // literal == sym
            if let (Expr::Literal(crate::ast::stmt::Literal::Number(n)), Expr::Identifier(sym)) = (&**left, &**right) {
                return Some((*sym, *n));
            }
            None
        }
        _ => None,
    }
}

/// Find the start value of a counter variable from the enclosing context.
/// Looks for the pattern: counter starts at some value based on the While condition.
/// For `While i < n` with `i` starting at 0, the start value is 0.
fn find_counter_start(counter: Symbol, body: &[Stmt]) -> Option<i64> {
    // Check if the counter is incremented in the body (Set counter to counter + 1)
    for stmt in body {
        if let Stmt::Set { target, value } = stmt {
            if *target == counter {
                if let Expr::BinaryOp { op: crate::ast::stmt::BinaryOpKind::Add, left, right } = &**value {
                    if let Expr::Identifier(sym) = &**left {
                        if *sym == counter {
                            if let Expr::Literal(crate::ast::stmt::Literal::Number(1)) = &**right {
                                return Some(0);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Try to peel the first iteration of a While loop.
/// Detects `If counter == start: ... Otherwise: ...` in the body and extracts
/// the first iteration, simplifying the remaining loop body.
fn try_peel<'a>(
    body: &'a [Stmt<'a>],
    while_cond: &'a Expr<'a>,
    decreasing: Option<&'a Expr<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    hoist_indexed_loads: bool,
) -> Option<Vec<Stmt<'a>>> {
    // Find an If with an equality boundary check
    let if_idx = body.iter().position(|s| {
        if let Stmt::If { cond, else_block: Some(_), .. } = s {
            extract_eq_boundary(cond).is_some()
        } else {
            false
        }
    })?;

    let (if_cond, then_block, else_block) = match &body[if_idx] {
        Stmt::If { cond, then_block, else_block: Some(eb) } => (cond, then_block, eb),
        _ => return None,
    };

    let (counter_sym, boundary_value) = extract_eq_boundary(if_cond)?;

    // Only peel first iteration (boundary == 0 or start value)
    let start_value = find_counter_start(counter_sym, body)?;
    if boundary_value != start_value {
        return None;
    }

    // Verify the counter is used in the While condition
    let counter_in_cond = match while_cond {
        Expr::BinaryOp { left, .. } => matches!(&**left, Expr::Identifier(sym) if *sym == counter_sym),
        _ => false,
    };
    if !counter_in_cond {
        return None;
    }

    // Build peeled first iteration:
    // [stmts_before_if..., then_block stmts..., stmts_after_if...]
    let before = &body[..if_idx];
    let after = &body[if_idx + 1..];

    let mut peeled_stmts: Vec<Stmt<'a>> = before.iter().cloned().collect();
    peeled_stmts.extend(then_block.iter().cloned());
    peeled_stmts.extend(after.iter().cloned());

    // Build remaining loop body (else branch replaces the If):
    // [stmts_before_if..., else_block stmts..., stmts_after_if...]
    let mut remaining_body: Vec<Stmt<'a>> = before.iter().cloned().collect();
    remaining_body.extend(else_block.iter().cloned());
    remaining_body.extend(after.iter().cloned());

    // Wrap peeled iteration in a guard: If while_cond { peeled } (handles zero-trip case)
    let remaining_processed = licm_stmts_with(remaining_body, expr_arena, stmt_arena, interner, hoist_indexed_loads);

    let remaining_while = Stmt::While {
        cond: while_cond,
        body: stmt_arena.alloc_slice(remaining_processed),
        decreasing,
    };

    // Result: If while_cond: { peeled_first_iteration; remaining_while }
    let mut guarded_body: Vec<Stmt<'a>> = peeled_stmts;
    guarded_body.push(remaining_while);

    let guarded = Stmt::If {
        cond: while_cond,
        then_block: stmt_arena.alloc_slice(guarded_body),
        else_block: None,
    };

    Some(vec![guarded])
}

/// Try to unswitch a While loop: if the body is exactly [If{invariant_cond, then, else}, ...rest],
/// transform into If{cond, While{..., then+rest}, While{..., else+rest}}.
/// Only fires when the If condition is invariant AND there's an else branch.
fn try_unswitch<'a>(
    body: &'a [Stmt<'a>],
    loop_writes: &HashSet<Symbol>,
    while_cond: &'a Expr<'a>,
    decreasing: Option<&'a Expr<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    hoist_indexed_loads: bool,
) -> Option<Stmt<'a>> {
    // Find a top-level If with invariant condition and else branch
    let if_idx = body.iter().position(|s| {
        matches!(s, Stmt::If { else_block: Some(_), .. })
    })?;

    let (if_cond, then_block, else_block) = match &body[if_idx] {
        Stmt::If { cond, then_block, else_block: Some(eb) } => (cond, then_block, eb),
        _ => return None,
    };

    // Condition must be loop-invariant. Variables defined via Let inside the
    // loop body are re-created each iteration, so they are NOT invariant.
    let mut effective_writes = loop_writes.clone();
    for s in body {
        if let Stmt::Let { var, .. } = s {
            effective_writes.insert(*var);
        }
    }
    if !is_loop_invariant(if_cond, &effective_writes) {
        return None;
    }

    // Build the two loop bodies:
    // then_body = [stmts_before_if..., then_block stmts..., stmts_after_if...]
    // else_body = [stmts_before_if..., else_block stmts..., stmts_after_if...]
    let before = &body[..if_idx];
    let after = &body[if_idx + 1..];

    let mut then_body_stmts: Vec<Stmt<'a>> = before.iter().cloned().collect();
    then_body_stmts.extend(then_block.iter().cloned());
    then_body_stmts.extend(after.iter().cloned());

    let mut else_body_stmts: Vec<Stmt<'a>> = before.iter().cloned().collect();
    else_body_stmts.extend(else_block.iter().cloned());
    else_body_stmts.extend(after.iter().cloned());

    // Recursively process both bodies
    let then_processed = licm_stmts_with(then_body_stmts, expr_arena, stmt_arena, interner, hoist_indexed_loads);
    let else_processed = licm_stmts_with(else_body_stmts, expr_arena, stmt_arena, interner, hoist_indexed_loads);

    let then_while = Stmt::While {
        cond: while_cond,
        body: stmt_arena.alloc_slice(then_processed),
        decreasing,
    };
    let else_while = Stmt::While {
        cond: while_cond,
        body: stmt_arena.alloc_slice(else_processed),
        decreasing,
    };

    Some(Stmt::If {
        cond: if_cond,
        then_block: stmt_arena.alloc_slice(vec![then_while]),
        else_block: Some(stmt_arena.alloc_slice(vec![else_while])),
    })
}

/// Process a block of statements, hoisting loop-invariant Lets from While/Repeat
/// bodies. The AOT entry point keeps `hoist_indexed_loads` OFF — its later
/// passes (tiling / loop-split / BCE-hoist) want the bare nested-loop shape, and
/// wrapping an inner loop in the hoist's `If` guard suppresses them. The
/// run-path entry turns it ON: the bytecode VM has no such downstream pass, and
/// lifting an invariant `item i of arr` read out of an inner loop is a direct
/// per-iteration win (nbody's force loop).
pub fn licm_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    licm_stmts_with(stmts, expr_arena, stmt_arena, interner, false)
}

/// [`licm_stmts`] with the indexed-load hoist enabled — the run-path entry.
pub fn licm_stmts_run<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    licm_stmts_with(stmts, expr_arena, stmt_arena, interner, true)
}

fn licm_stmts_with<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    hoist_indexed_loads: bool,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::While { cond, body, decreasing } => {
                let loop_writes = collect_loop_writes(body);

                // Loop unswitching: if the body contains a top-level If with an
                // invariant condition (and else branch), hoist the If outside.
                if let Some(unswitched) = try_unswitch(body, &loop_writes, cond, decreasing, expr_arena, stmt_arena, interner, hoist_indexed_loads) {
                    result.push(unswitched);
                    continue;
                }

                // Loop peeling: extract first iteration when body has If counter==start
                if let Some(peeled) = try_peel(body, cond, decreasing, expr_arena, stmt_arena, interner, hoist_indexed_loads) {
                    result.extend(peeled);
                    continue;
                }

                let mut hoisted = Vec::new();
                let mut remaining = Vec::new();

                for s in body.iter().cloned() {
                    if let Stmt::Let { var: _, ty: _, value, mutable: false } = &s {
                        if is_loop_invariant(value, &loop_writes) && is_worth_hoisting(value) {
                            hoisted.push(s);
                            continue;
                        }
                    }
                    remaining.push(s);
                }

                // Recursively process the remaining loop body
                let processed_remaining = licm_stmts_with(remaining, expr_arena, stmt_arena, interner, hoist_indexed_loads);
                let inner = Stmt::While {
                    cond,
                    body: stmt_arena.alloc_slice(processed_remaining),
                    decreasing,
                };
                result.extend(hoisted);
                // Indexed-load hoisting (run path only): pull loop-invariant
                // `item i of arr` reads (collection + index both invariant,
                // collection never mutated in the loop) into the preheader.
                // Unlike the whole-Let hoist above, an indexed read can PANIC
                // (OOB), so moving it above a zero-trip loop would manufacture a
                // panic the source never had — the hoisted reads are wrapped in
                // `If cond { … }` so they run iff the loop body runs at least
                // once (only when `cond` is side-effect-free, so the duplicated
                // test is observationally identical).
                let guarded = if hoist_indexed_loads {
                    hoist_invariant_loads(
                        inner.clone(), cond, &loop_writes, expr_arena, stmt_arena, interner,
                    )
                } else {
                    None
                };
                match guarded {
                    Some(g) => result.push(g),
                    None => result.push(inner),
                }
            }
            Stmt::Repeat { pattern, iterable, body } => {
                let mut loop_writes = collect_loop_writes(body);
                // The pattern variable varies each iteration — expressions
                // reading it are NOT loop-invariant.
                if let crate::ast::stmt::Pattern::Identifier(sym) = &pattern {
                    loop_writes.insert(*sym);
                }
                let mut hoisted = Vec::new();
                let mut remaining = Vec::new();

                for s in body.iter().cloned() {
                    if let Stmt::Let { var: _, ty: _, value, mutable: false } = &s {
                        if is_loop_invariant(value, &loop_writes) && is_worth_hoisting(value) {
                            hoisted.push(s);
                            continue;
                        }
                    }
                    remaining.push(s);
                }

                let processed_remaining = licm_stmts_with(remaining, expr_arena, stmt_arena, interner, hoist_indexed_loads);
                result.extend(hoisted);
                result.push(Stmt::Repeat {
                    pattern,
                    iterable,
                    body: stmt_arena.alloc_slice(processed_remaining),
                });
            }
            Stmt::FunctionDef { name, generics, params, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
                let new_body = licm_stmts_with(body.to_vec(), expr_arena, stmt_arena, interner, hoist_indexed_loads);
                result.push(Stmt::FunctionDef {
                    name, generics, params,
                    body: stmt_arena.alloc_slice(new_body),
                    return_type, is_native, native_path, is_exported, export_target, opt_flags,
                });
            }
            Stmt::If { cond, then_block, else_block } => {
                let new_then = licm_stmts_with(then_block.to_vec(), expr_arena, stmt_arena, interner, hoist_indexed_loads);
                let new_else = else_block.map(|eb| {
                    let processed = licm_stmts_with(eb.to_vec(), expr_arena, stmt_arena, interner, hoist_indexed_loads);
                    let b: Block = stmt_arena.alloc_slice(processed);
                    b
                });
                result.push(Stmt::If {
                    cond,
                    then_block: stmt_arena.alloc_slice(new_then),
                    else_block: new_else,
                });
            }
            Stmt::Zone { name, capacity, source_file, body } => {
                let new_body = licm_stmts_with(body.to_vec(), expr_arena, stmt_arena, interner, hoist_indexed_loads);
                result.push(Stmt::Zone {
                    name, capacity, source_file,
                    body: stmt_arena.alloc_slice(new_body),
                });
            }
            other => result.push(other),
        }
    }

    result
}

#[cfg(test)]
mod indexed_load_hoist_tests {
    //! Task C: hoisting loop-invariant `item i of arr` reads out of an inner
    //! loop. The structural proofs build a nbody-shaped inner `While j` and
    //! assert the invariant load is lifted into a guarded preheader, plus the
    //! soundness guards: a MUTATED collection is never hoisted, a variant index
    //! is never hoisted, and a side-effecting loop condition disables the lift.
    use super::*;
    use crate::ast::stmt::{BinaryOpKind, Literal};

    struct B<'a> {
        ea: &'a Arena<Expr<'a>>,
        sa: &'a Arena<Stmt<'a>>,
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
        fn block(&self, v: Vec<Stmt<'a>>) -> Block<'a> {
            self.sa.alloc_slice(v)
        }
    }

    struct Names {
        i: Symbol,
        j: Symbol,
        bx: Symbol,
        bvx: Symbol,
        acc: Symbol,
        dx: Symbol,
    }
    fn names(it: &mut Interner) -> Names {
        Names {
            i: it.intern("i"),
            j: it.intern("j"),
            bx: it.intern("bx"),
            bvx: it.intern("bvx"),
            acc: it.intern("acc"),
            dx: it.intern("dx"),
        }
    }

    /// Count `Let … be item _ of <coll>` bindings (a bare indexed-load Let,
    /// which is exactly the shape the hoist mints) in a statement tree.
    fn count_hoisted(stmts: &[Stmt], coll: Symbol) -> usize {
        stmts
            .iter()
            .map(|s| match s {
                Stmt::Let { value, .. } => matches!(
                    value,
                    Expr::Index { collection, .. }
                        if matches!(&**collection, Expr::Identifier(c) if *c == coll)
                ) as usize,
                Stmt::If { then_block, else_block, .. } => {
                    count_hoisted(then_block, coll)
                        + else_block.map_or(0, |eb| count_hoisted(eb, coll))
                }
                Stmt::While { body, .. } => count_hoisted(body, coll),
                _ => 0,
            })
            .sum()
    }

    /// Count every `item _ of <coll>` indexed read across the statement tree
    /// (including nested Let/Set/If/While bodies and identifiers used as
    /// collections), so a hoist that DUPLICATES a read is caught.
    fn count_item_reads(stmts: &[Stmt], coll: Symbol) -> usize {
        fn in_expr(e: &Expr, coll: Symbol) -> usize {
            match e {
                Expr::Index { collection, index } => {
                    let here = matches!(&**collection, Expr::Identifier(c) if *c == coll) as usize;
                    here + in_expr(collection, coll) + in_expr(index, coll)
                }
                Expr::BinaryOp { left, right, .. } => in_expr(left, coll) + in_expr(right, coll),
                Expr::Not { operand } => in_expr(operand, coll),
                _ => 0,
            }
        }
        stmts
            .iter()
            .map(|s| match s {
                Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, coll),
                Stmt::SetIndex { collection, index, value } => {
                    in_expr(collection, coll) + in_expr(index, coll) + in_expr(value, coll)
                }
                Stmt::If { cond, then_block, else_block } => {
                    in_expr(cond, coll)
                        + count_item_reads(then_block, coll)
                        + else_block.map_or(0, |eb| count_item_reads(eb, coll))
                }
                Stmt::While { cond, body, .. } => in_expr(cond, coll) + count_item_reads(body, coll),
                _ => 0,
            })
            .sum()
    }

    /// Does any `item _ of <coll>` indexed read remain anywhere in the tree?
    fn has_index_of(stmts: &[Stmt], coll: Symbol) -> bool {
        fn in_expr(e: &Expr, coll: Symbol) -> bool {
            match e {
                Expr::Index { collection, index } => {
                    matches!(&**collection, Expr::Identifier(c) if *c == coll)
                        || in_expr(collection, coll)
                        || in_expr(index, coll)
                }
                Expr::BinaryOp { left, right, .. } => in_expr(left, coll) || in_expr(right, coll),
                Expr::Not { operand } => in_expr(operand, coll),
                _ => false,
            }
        }
        stmts.iter().any(|s| match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, coll),
            Stmt::SetIndex { collection, index, value } => {
                in_expr(collection, coll) || in_expr(index, coll) || in_expr(value, coll)
            }
            Stmt::If { cond, then_block, else_block } => {
                in_expr(cond, coll)
                    || has_index_of(then_block, coll)
                    || else_block.map_or(false, |eb| has_index_of(eb, coll))
            }
            Stmt::While { cond, body, .. } => in_expr(cond, coll) || has_index_of(body, coll),
            _ => false,
        })
    }

    /// The nbody inner-loop shape: `While j <= 5: Let dx be item i of bx - item
    /// j of bx; Set acc to acc + dx.` `item i of bx` is invariant w.r.t. j and
    /// bx is never mutated → it must hoist; `item j of bx` must stay.
    fn nbody_inner_loop<'a>(b: &B<'a>, n: &Names) -> Vec<Stmt<'a>> {
        use BinaryOpKind::*;
        let dx_val = b.bin(
            Subtract,
            b.index(b.id(n.bx), b.id(n.i)), // invariant: item i of bx
            b.index(b.id(n.bx), b.id(n.j)), // variant: item j of bx
        );
        let let_dx = Stmt::Let { var: n.dx, ty: None, value: dx_val, mutable: false };
        let set_acc = Stmt::Set {
            target: n.acc,
            value: b.bin(Add, b.id(n.acc), b.id(n.dx)),
        };
        let inc_j = Stmt::Set { target: n.j, value: b.bin(Add, b.id(n.j), b.num(1)) };
        let while_j = Stmt::While {
            cond: b.bin(LtEq, b.id(n.j), b.num(5)),
            body: b.block(vec![let_dx, set_acc, inc_j]),
            decreasing: None,
        };
        vec![while_j]
    }

    #[test]
    fn hoists_invariant_indexed_load_into_guarded_preheader() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let n = names(&mut it);
        let b = B { ea: &ea, sa: &sa };
        let input = nbody_inner_loop(&b, &n);
        let out = licm_stmts_run(input, &ea, &sa, &mut it);

        // The While is now wrapped in `If cond { Let __licm_load be item i of bx; While … }`.
        let guarded = match &out[..] {
            [Stmt::If { then_block, else_block: None, .. }] => *then_block,
            other => panic!("expected a single guarded If, got {other:?}"),
        };
        // Exactly one invariant load (`item i of bx`) hoisted as a fresh Let.
        assert_eq!(
            count_hoisted(guarded, n.bx),
            1,
            "the invariant `item i of bx` must be hoisted into the preheader.\nGot: {guarded:?}"
        );
        // The hoisted Let precedes the (rewritten) While.
        assert!(matches!(guarded[0], Stmt::Let { .. }), "hoisted load comes first");
        assert!(
            matches!(guarded.last(), Some(Stmt::While { .. })),
            "the loop follows the hoisted loads"
        );
        // The variant `item j of bx` is STILL read inside the loop body —
        // exactly one `item _ of bx` read should remain (the j one); the i one
        // is now an identifier. (Verify the loop still indexes bx for j.)
        if let Some(Stmt::While { body, .. }) = guarded.last() {
            assert!(
                has_index_of(body, n.bx),
                "the variant `item j of bx` must remain in the loop body"
            );
        }
    }

    #[test]
    fn does_not_hoist_when_collection_is_mutated_in_loop() {
        // `Set item i of bvx to …` in the body makes bvx a loop write → its
        // `item i of bvx` read is NOT invariant and must NOT hoist.
        use BinaryOpKind::*;
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let n = names(&mut it);
        let b = B { ea: &ea, sa: &sa };
        let read = b.index(b.id(n.bvx), b.id(n.i)); // item i of bvx
        let let_dx = Stmt::Let {
            var: n.dx,
            ty: None,
            value: b.bin(Add, read, b.num(1)),
            mutable: false,
        };
        // Mutate item i of bvx in the SAME loop.
        let store = Stmt::SetIndex {
            collection: b.id(n.bvx),
            index: b.id(n.i),
            value: b.id(n.dx),
        };
        let inc_j = Stmt::Set { target: n.j, value: b.bin(Add, b.id(n.j), b.num(1)) };
        let while_j = Stmt::While {
            cond: b.bin(LtEq, b.id(n.j), b.num(5)),
            body: b.block(vec![let_dx, store, inc_j]),
            decreasing: None,
        };
        let out = licm_stmts_run(vec![while_j], &ea, &sa, &mut it);
        // No guard wrapper — the load was not hoistable, so the loop is
        // returned unchanged (still a bare While).
        assert!(
            matches!(&out[..], [Stmt::While { .. }]),
            "a mutated collection must NOT be hoisted (no guarded If).\nGot: {out:?}"
        );
    }

    #[test]
    fn does_not_hoist_variant_index() {
        // `item j of bx` (index is the loop variable) is variant → no hoist.
        use BinaryOpKind::*;
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let n = names(&mut it);
        let b = B { ea: &ea, sa: &sa };
        let let_dx = Stmt::Let {
            var: n.dx,
            ty: None,
            value: b.index(b.id(n.bx), b.id(n.j)), // only the variant read
            mutable: false,
        };
        let inc_j = Stmt::Set { target: n.j, value: b.bin(Add, b.id(n.j), b.num(1)) };
        let while_j = Stmt::While {
            cond: b.bin(LtEq, b.id(n.j), b.num(5)),
            body: b.block(vec![let_dx, inc_j]),
            decreasing: None,
        };
        let out = licm_stmts_run(vec![while_j], &ea, &sa, &mut it);
        assert!(
            matches!(&out[..], [Stmt::While { .. }]),
            "a variant index must NOT be hoisted.\nGot: {out:?}"
        );
    }

    /// A wholly-invariant indexed Let (`Let dx be item i of bx`, no variant
    /// co-index) is already lifted by the EXISTING whole-Let hoist; the
    /// indexed-load pass must add NO second copy of `item i of bx` — exactly one
    /// read survives, never a duplicate.
    #[test]
    fn indexed_load_hoist_does_not_duplicate_whole_let_invariant_read() {
        use BinaryOpKind::*;
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let n = names(&mut it);
        let b = B { ea: &ea, sa: &sa };
        let let_dx = Stmt::Let {
            var: n.dx,
            ty: None,
            value: b.index(b.id(n.bx), b.id(n.i)), // item i of bx — wholly invariant
            mutable: false,
        };
        let inc_j = Stmt::Set { target: n.j, value: b.bin(Add, b.id(n.j), b.num(1)) };
        let while_j = Stmt::While {
            cond: b.bin(LtEq, b.id(n.j), b.num(5)),
            body: b.block(vec![let_dx, inc_j]),
            decreasing: None,
        };
        let out = licm_stmts_run(vec![while_j], &ea, &sa, &mut it);
        assert_eq!(
            count_item_reads(&out, n.bx),
            1,
            "a wholly-invariant indexed Let must not be duplicated by the load hoist.\nGot: {out:?}"
        );
    }
}
