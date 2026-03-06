//! Loop-Invariant Code Motion (LICM).
//!
//! Hoists immutable Let statements from loop bodies when their value expressions
//! only read variables that are not written in the loop body. This avoids
//! recomputing invariant expressions on every iteration.

use std::collections::HashSet;

use crate::arena::Arena;
use crate::ast::stmt::{Block, Expr, Stmt};
use crate::intern::{Interner, Symbol};

/// Collect all symbols written in a block of statements (recursively).
fn collect_loop_writes(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut writes = HashSet::new();
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, .. } => {
                writes.insert(*target);
            }
            Stmt::Push { collection, .. }
            | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. }
            | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    writes.insert(*sym);
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
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
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
fn is_loop_invariant(expr: &Expr, loop_writes: &HashSet<Symbol>) -> bool {
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
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
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
    let remaining_processed = licm_stmts(remaining_body, stmt_arena, interner);

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
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
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
    let then_processed = licm_stmts(then_body_stmts, stmt_arena, interner);
    let else_processed = licm_stmts(else_body_stmts, stmt_arena, interner);

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

/// Process a block of statements, hoisting loop-invariant Lets from While/Repeat bodies.
pub fn licm_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    _interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::While { cond, body, decreasing } => {
                let loop_writes = collect_loop_writes(body);

                // Loop unswitching: if the body contains a top-level If with an
                // invariant condition (and else branch), hoist the If outside.
                if let Some(unswitched) = try_unswitch(body, &loop_writes, cond, decreasing, stmt_arena, _interner) {
                    result.push(unswitched);
                    continue;
                }

                // Loop peeling: extract first iteration when body has If counter==start
                if let Some(peeled) = try_peel(body, cond, decreasing, stmt_arena, _interner) {
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
                let processed_remaining = licm_stmts(remaining, stmt_arena, _interner);
                result.extend(hoisted);
                result.push(Stmt::While {
                    cond,
                    body: stmt_arena.alloc_slice(processed_remaining),
                    decreasing,
                });
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

                let processed_remaining = licm_stmts(remaining, stmt_arena, _interner);
                result.extend(hoisted);
                result.push(Stmt::Repeat {
                    pattern,
                    iterable,
                    body: stmt_arena.alloc_slice(processed_remaining),
                });
            }
            Stmt::FunctionDef { name, generics, params, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
                let new_body = licm_stmts(body.to_vec(), stmt_arena, _interner);
                result.push(Stmt::FunctionDef {
                    name, generics, params,
                    body: stmt_arena.alloc_slice(new_body),
                    return_type, is_native, native_path, is_exported, export_target, opt_flags,
                });
            }
            Stmt::If { cond, then_block, else_block } => {
                let new_then = licm_stmts(then_block.to_vec(), stmt_arena, _interner);
                let new_else = else_block.map(|eb| {
                    let processed = licm_stmts(eb.to_vec(), stmt_arena, _interner);
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
                let new_body = licm_stmts(body.to_vec(), stmt_arena, _interner);
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
