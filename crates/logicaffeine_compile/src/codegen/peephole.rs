use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use super::context::{RefinementContext, VariableCapabilities};
use super::detection::symbol_appears_in_stmts;
use super::types::codegen_type_expr;

/// Collection type information for with_capacity pattern detection.
enum CollInfo { Vec(String), Map(String, String) }

/// Peephole optimization: detect `Let counter = start. While counter <= limit: body; Set counter to counter + 1`
/// and emit `for counter in start..=limit { body } let mut counter = limit + 1;` instead.
/// The for-range form enables LLVM trip count analysis, unrolling, and vectorization.
/// Returns (generated_code, number_of_extra_statements_consumed) or None if pattern doesn't match.
pub(crate) fn try_emit_for_range_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) -> Option<(String, usize)> {
    if idx + 1 >= stmts.len() {
        return None;
    }

    // Statement 1: Let counter = start OR Set counter to start
    // Accepts:
    //   - Let counter = literal (original)
    //   - Set counter to literal (OPT-1a: reused counter variables)
    //   - Let counter = expr (OPT-1b: variable/expression start values)
    //   - Set counter to expr (OPT-1a + OPT-1b combined)
    // The counter's mutability is proven by the while body's increment statement.
    let (counter_sym, counter_start_expr, is_new_binding) = match stmts[idx] {
        Stmt::Let { var, value, .. } => {
            if is_simple_expr(value) {
                (*var, *value, true)
            } else {
                return None;
            }
        }
        Stmt::Set { target, value } => {
            if is_simple_expr(value) {
                (*target, *value, false)
            } else {
                return None;
            }
        }
        _ => return None,
    };
    // Extract literal start for compile-time range optimization
    let counter_start_literal = match counter_start_expr {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    };

    // Statement 2: While (counter <= limit) or (counter < limit)
    let (body, limit_expr, is_exclusive) = match stmts[idx + 1] {
        Stmt::While { cond, body, .. } => {
            match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (body, *right, false)
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (body, *right, true)
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        _ => return None,
    };

    // Body must have at least 1 statement (the counter increment)
    if body.is_empty() {
        return None;
    }

    // Last body statement must be: Set counter to counter + 1
    let last = &body[body.len() - 1];
    match last {
        Stmt::Set { target, value, .. } => {
            if *target != counter_sym {
                return None;
            }
            match value {
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                    let is_counter_plus_1 = match (left, right) {
                        (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym => true,
                        (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym => true,
                        _ => false,
                    };
                    if !is_counter_plus_1 {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        _ => return None,
    }

    // Validity: counter must NOT be modified anywhere in the body EXCEPT the last statement.
    // Walk all body statements (excluding the last) and check for Set { target: counter_sym }.
    let body_without_increment = &body[..body.len() - 1];
    if body_modifies_var(body_without_increment, counter_sym) {
        return None;
    }

    // Bail out if the limit expression is too complex for codegen_expr_simple
    // (it returns "_" for unhandled expressions like `length of`).
    if !is_simple_expr(limit_expr) {
        return None;
    }

    // Bail out if any variable in limit_expr is modified in the loop body.
    // This prevents converting loops like `While front <= length of queue`
    // where queue grows via Push inside the body.
    let mut limit_syms = Vec::new();
    collect_expr_symbols(limit_expr, &mut limit_syms);
    for sym in &limit_syms {
        if body_modifies_var(body_without_increment, *sym)
            || body_mutates_collection(body_without_increment, *sym)
        {
            return None;
        }
    }

    // Detect buffer reuse: inner buffer allocated each iteration, transferred to outer var.
    let buffer_reuse = detect_buffer_reuse_in_body(body_without_increment, interner, ctx);

    // Detect double-buffer swap: pre-allocated buffers with SetIndex + transfer.
    // Only fires when buffer_reuse doesn't (they're mutually exclusive patterns).
    let double_buffer = if buffer_reuse.is_none() {
        detect_double_buffer_swap(body_without_increment, interner, ctx)
    } else {
        None
    };

    // Pattern matched! Emit for-range loop.
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let counter_name = names.ident(counter_sym);
    let limit_str = codegen_expr_simple(limit_expr, interner);
    let start_str = codegen_expr_simple(counter_start_expr, interner);

    // OPT-8: Zero-based counter normalization.
    // When the counter starts at 1 and is ONLY used for direct array indexing
    // in the body, shift the range to 0-based to eliminate (i - 1) subtracts.
    // The counter is registered as "__zero_based_i64" so index codegen skips -1.
    let use_zero_based = counter_start_literal == Some(1)
        && !is_exclusive
        && counter_has_index_uses(body_without_increment, counter_sym)
        && counter_only_used_for_indexing(body_without_increment, counter_sym)
        && counter_indexes_only_vec_types(body_without_increment, counter_sym, ctx);

    // Always use exclusive ranges (Range) instead of inclusive (RangeInclusive).
    // RangeInclusive has a known performance overhead in Rust due to internal
    // bookkeeping for edge cases, which compounds in hot inner loops.
    // Convert `i <= limit` to `i < (limit + 1)`.
    let range_str = if use_zero_based {
        // 0-based: for i in 0..limit (instead of 1..(limit+1))
        if let Expr::Literal(Literal::Number(n)) = limit_expr {
            format!("0..{}", n)
        } else {
            format!("0..{}", limit_str)
        }
    } else if is_exclusive {
        format!("{}..{}", start_str, limit_str)
    } else {
        // For literal limits, compute limit+1 at compile time
        if let Expr::Literal(Literal::Number(n)) = limit_expr {
            format!("{}..{}", start_str, n + 1)
        } else {
            format!("{}..({} + 1)", start_str, limit_str)
        }
    };

    let mut output = String::new();

    // OPT-4: Emit assert_unchecked hints for arrays indexed by the counter.
    // This helps LLVM elide bounds checks and enables vectorization.
    // The assertion `(limit as usize) <= arr.len()` holds for both 1-based and
    // zero-based loops:
    //   1-based: range start..limit+1, index arr[(i-1) as usize], max index = limit-1
    //   0-based: range 0..limit, index arr[i as usize], max index = limit-1
    // In both cases, limit <= arr.len() guarantees all accesses are in-bounds.
    // This bridges the i64→usize cast gap that prevents LLVM from proving safety.
    {
        let indexed_arrays = collect_indexed_arrays(body_without_increment, counter_sym);
        // Exclude Map-typed collections — Maps use .get(&key) which returns None for
        // missing keys, so bounds assertions are invalid (and cause panics).
        let indexed_arrays: Vec<Symbol> = indexed_arrays.into_iter().filter(|arr_sym| {
            match ctx.get_variable_types().get(arr_sym) {
                Some(t) if t.contains("HashMap") => false,
                _ => true,
            }
        }).collect();
        for arr_sym in &indexed_arrays {
            let arr_name = interner.resolve(*arr_sym);
            writeln!(output, "{}unsafe {{ std::hint::assert_unchecked(({} as usize) <= {}.len()); }}",
                indent_str, limit_str, arr_name).unwrap();
        }
    }

    // Hoist inner buffer if buffer reuse detected.
    if let Some(ref reuse) = buffer_reuse {
        let reuse_inner = names.ident(reuse.inner_sym);
        writeln!(output, "{}let mut {}: Vec<{}> = Vec::new();", indent_str, reuse_inner, reuse.inner_elem_type).unwrap();
    }

    writeln!(output, "{}for {} in {} {{", indent_str, counter_name, range_str).unwrap();

    // Emit body statements (excluding the final counter increment)
    // Apply full peephole suite to enable nested loop optimization.
    ctx.push_scope();

    // Register zero-based counter so index codegen skips the (i - 1) subtraction
    if use_zero_based {
        ctx.register_variable_type(counter_sym, "__zero_based_i64".to_string());
    }
    let body_refs: Vec<&Stmt> = body_without_increment.iter().collect();
    let mut bi = 0;
    while bi < body_refs.len() {
        // Buffer reuse interception: replace allocation with clear, transfer with swap.
        if let Some(ref reuse) = buffer_reuse {
            if bi == reuse.inner_let_idx {
                let reuse_inner = names.ident(reuse.inner_sym);
                writeln!(output, "{}{}.clear();", "    ".repeat(indent + 1), reuse_inner).unwrap();
                ctx.register_variable_type(reuse.inner_sym, format!("Vec<{}>", reuse.inner_elem_type));
                bi += 1;
                continue;
            }
            if bi == reuse.set_idx {
                let reuse_inner = names.ident(reuse.inner_sym);
                let reuse_outer = names.ident(reuse.outer_sym);
                writeln!(output, "{}std::mem::swap(&mut {}, &mut {});", "    ".repeat(indent + 1), reuse_outer, reuse_inner).unwrap();
                bi += 1;
                continue;
            }
        }
        // Double-buffer swap interception: replace Set X to Y with mem::swap.
        if let Some((x_sym, y_sym, db_set_idx)) = double_buffer {
            if bi == db_set_idx {
                let x_name = names.ident(x_sym);
                let y_name = names.ident(y_sym);
                writeln!(output, "{}std::mem::swap(&mut {}, &mut {});", "    ".repeat(indent + 1), x_name, y_name).unwrap();
                bi += 1;
                continue;
            }
        }
        if let Some((code, skip)) = try_emit_vec_fill_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_for_range_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_seq_copy_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_rotate_left_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        output.push_str(&super::codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
        bi += 1;
    }
    ctx.pop_scope();
    // Clear the zero-based marker since the counter reverts to 1-based after the loop.
    // variable_types is a flat HashMap (not scoped), so push/pop_scope doesn't clean it.
    if use_zero_based {
        ctx.register_variable_type(counter_sym, "i64".to_string());
    }
    writeln!(output, "{}}}", indent_str).unwrap();

    // Emit post-loop counter value only if the counter is used after the loop.
    let remaining_stmts = &stmts[idx + 2..];
    if symbol_appears_in_stmts(counter_sym, remaining_stmts) {
        // OPT-7: If the next statement immediately overwrites the counter,
        // skip the max() computation and just declare the variable (if needed).
        let next_stmt_overwrites_counter = remaining_stmts.first().map_or(false, |s| {
            matches!(s, Stmt::Set { target, .. } if *target == counter_sym)
        });

        if next_stmt_overwrites_counter {
            // For Let-based counters, we still need to declare the variable
            // so it exists in scope for the overwriting Set statement.
            if is_new_binding {
                writeln!(output, "{}let mut {} = 0;", indent_str, counter_name).unwrap();
            }
        } else {
            // After `while (i <= limit) { ...; i++ }`, i == limit + 1.
            // After `while (i < limit) { ...; i++ }`, i == limit.
            // If the loop never executes (start >= limit), counter stays at start.
            let post_value = if is_exclusive {
                match (counter_start_literal, limit_expr) {
                    (Some(s), Expr::Literal(Literal::Number(n))) => {
                        format!("{}", std::cmp::max(s, *n))
                    }
                    (Some(s), _) => {
                        format!("({}_i64).max({})", s, limit_str)
                    }
                    (None, _) => {
                        format!("({}).max({})", start_str, limit_str)
                    }
                }
            } else {
                match (counter_start_literal, limit_expr) {
                    (Some(s), Expr::Literal(Literal::Number(n))) => {
                        format!("{}", std::cmp::max(s, n + 1))
                    }
                    (Some(s), _) => {
                        format!("({}_i64).max({} + 1)", s, limit_str)
                    }
                    (None, _) => {
                        format!("({}).max({} + 1)", start_str, limit_str)
                    }
                }
            };
            if is_new_binding {
                writeln!(output, "{}let mut {} = {};", indent_str, counter_name, post_value).unwrap();
            } else {
                writeln!(output, "{}{} = {};", indent_str, counter_name, post_value).unwrap();
            }
        }
    }

    Some((output, 1)) // consumed 1 extra statement (the While)
}

/// Collect all identifier symbols referenced in an expression.
/// Used to check whether a loop bound depends on variables modified in the body.
fn collect_expr_symbols(expr: &Expr, out: &mut Vec<Symbol>) {
    match expr {
        Expr::Identifier(sym) => out.push(*sym),
        Expr::Length { collection } => collect_expr_symbols(collection, out),
        Expr::BinaryOp { left, right, .. } => {
            collect_expr_symbols(left, out);
            collect_expr_symbols(right, out);
        }
        _ => {}
    }
}

/// Check if a slice of statements modifies a specific variable (used for for-range validity).
/// Recursively walks into nested If/While/Repeat blocks.
pub(crate) fn body_modifies_var(stmts: &[Stmt], sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, .. } if *target == sym => return true,
            Stmt::If { then_block, else_block, .. } => {
                if body_modifies_var(then_block, sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_modifies_var(else_stmts, sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            Stmt::Repeat { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if a loop body mutates a specific collection (used for iterator optimization).
/// Scans for Push, Pop, SetIndex, Remove, Set, and Add targeting the collection.
/// Recursively walks into nested If/While/Repeat/Zone blocks.
pub(crate) fn body_mutates_collection(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Push { collection, .. } | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::SetIndex { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::Set { target, .. } if *target == coll_sym => return true,
            Stmt::If { then_block, else_block, .. } => {
                if body_mutates_collection(then_block, coll_sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_mutates_collection(else_stmts, coll_sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if body_mutates_collection(body, coll_sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_mutates_collection(body, coll_sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if a loop body resizes a specific collection via Push, Pop, Add, or Remove.
/// Unlike `body_mutates_collection`, this does NOT flag SetIndex (element-level writes),
/// making it suitable for detecting double-buffer patterns where SetIndex is expected.
/// Recursively walks into nested If/While/Repeat/Zone blocks.
pub(crate) fn body_resizes_collection(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Push { collection, .. } | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::Set { target, .. } if *target == coll_sym => return true,
            Stmt::If { then_block, else_block, .. } => {
                if body_resizes_collection(then_block, coll_sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_resizes_collection(else_stmts, coll_sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if body_resizes_collection(body, coll_sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_resizes_collection(body, coll_sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if every execution path through `stmts` includes at least one `Push` targeting
/// `coll_sym`. Returns true only when the push count is deterministic (every branch pushes),
/// which makes `with_capacity(loop_count)` a valid pre-allocation.
/// For filter patterns (push inside If without Otherwise), returns false.
fn all_paths_push_to(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Push { collection, .. } => {
            matches!(collection, Expr::Identifier(sym) if *sym == coll_sym)
        }
        Stmt::If { then_block, else_block, .. } => {
            if let Some(else_stmts) = else_block {
                all_paths_push_to(then_block, coll_sym)
                    && all_paths_push_to(else_stmts, coll_sym)
            } else {
                false
            }
        }
        _ => false,
    })
}

/// Check if every execution path through `stmts` includes at least one `SetIndex` targeting
/// `coll_sym`. Same logic as `all_paths_push_to` but for map insertion.
fn all_paths_set_index_to(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::SetIndex { collection, .. } => {
            matches!(collection, Expr::Identifier(sym) if *sym == coll_sym)
        }
        Stmt::If { then_block, else_block, .. } => {
            if let Some(else_stmts) = else_block {
                all_paths_set_index_to(then_block, coll_sym)
                    && all_paths_set_index_to(else_stmts, coll_sym)
            } else {
                false
            }
        }
        _ => false,
    })
}

/// Check that the counter is used in at least one Expr::Index or Stmt::SetIndex
/// within the body. If there are zero index uses, zero-based normalization
/// provides no benefit (no -1 subtracts to eliminate).
fn counter_has_index_uses(stmts: &[Stmt], counter_sym: Symbol) -> bool {
    stmts.iter().any(|s| stmt_has_counter_index_use(s, counter_sym))
}

fn stmt_has_counter_index_use(stmt: &Stmt, counter_sym: Symbol) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::Set { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::Show { object, .. } => expr_has_counter_index_use(object, counter_sym),
        Stmt::Push { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::SetIndex { index, value, .. } => {
            matches!(index, Expr::Identifier(sym) if *sym == counter_sym)
                || expr_has_counter_index_use(value, counter_sym)
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_has_counter_index_use(cond, counter_sym)
                || counter_has_index_uses(then_block, counter_sym)
                || else_block.as_ref().map_or(false, |eb| counter_has_index_uses(eb, counter_sym))
        }
        Stmt::While { body, .. } => counter_has_index_uses(body, counter_sym),
        Stmt::Return { value } => value.map_or(false, |v| expr_has_counter_index_use(v, counter_sym)),
        Stmt::Call { args, .. } => args.iter().any(|a| expr_has_counter_index_use(a, counter_sym)),
        Stmt::Repeat { body, .. } => counter_has_index_uses(body, counter_sym),
        _ => false,
    }
}

fn expr_has_counter_index_use(expr: &Expr, counter_sym: Symbol) -> bool {
    match expr {
        Expr::Index { collection, index } => {
            matches!(index, Expr::Identifier(sym) if *sym == counter_sym)
                || expr_has_counter_index_use(collection, counter_sym)
                || expr_has_counter_index_use(index, counter_sym)
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_has_counter_index_use(left, counter_sym) || expr_has_counter_index_use(right, counter_sym)
        }
        Expr::Call { args, .. } => args.iter().any(|a| expr_has_counter_index_use(a, counter_sym)),
        Expr::Length { collection } => expr_has_counter_index_use(collection, counter_sym),
        Expr::List(items) => items.iter().any(|e| expr_has_counter_index_use(e, counter_sym)),
        Expr::Not { operand } => expr_has_counter_index_use(operand, counter_sym),
        Expr::Copy { expr: inner } => expr_has_counter_index_use(inner, counter_sym),
        Expr::Slice { collection, start, end } => {
            expr_has_counter_index_use(collection, counter_sym)
                || expr_has_counter_index_use(start, counter_sym)
                || expr_has_counter_index_use(end, counter_sym)
        }
        _ => false,
    }
}

/// Verify that every collection indexed by the counter is a Vec or slice type.
/// Zero-based normalization only works for direct `arr[i as usize]` codegen —
/// string indexing goes through `LogosIndex::logos_get()` which does internal
/// 1-based conversion, so passing a 0-based counter would be incorrect.
fn counter_indexes_only_vec_types(stmts: &[Stmt], counter_sym: Symbol, ctx: &RefinementContext) -> bool {
    for stmt in stmts {
        if !stmt_counter_indexes_vec_types(stmt, counter_sym, ctx) {
            return false;
        }
    }
    true
}

fn stmt_counter_indexes_vec_types(stmt: &Stmt, counter_sym: Symbol, ctx: &RefinementContext) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::Set { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::Show { object, .. } => expr_counter_indexes_vec_types(object, counter_sym, ctx),
        Stmt::Push { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::SetIndex { collection, index, value } => {
            let idx_uses_counter = matches!(index, Expr::Identifier(sym) if *sym == counter_sym);
            if idx_uses_counter {
                if let Expr::Identifier(coll_sym) = collection {
                    let is_vec = ctx.get_variable_types().get(coll_sym)
                        .map_or(false, |t| t.starts_with("Vec") || t.starts_with("&[") || t.starts_with("&mut ["));
                    if !is_vec { return false; }
                } else {
                    return false;
                }
            }
            expr_counter_indexes_vec_types(value, counter_sym, ctx)
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_counter_indexes_vec_types(cond, counter_sym, ctx)
                && counter_indexes_only_vec_types(then_block, counter_sym, ctx)
                && else_block.as_ref().map_or(true, |eb| counter_indexes_only_vec_types(eb, counter_sym, ctx))
        }
        Stmt::While { body, .. } => counter_indexes_only_vec_types(body, counter_sym, ctx),
        Stmt::Repeat { body, .. } => counter_indexes_only_vec_types(body, counter_sym, ctx),
        Stmt::Return { value } => value.map_or(true, |v| expr_counter_indexes_vec_types(v, counter_sym, ctx)),
        Stmt::Call { args, .. } => args.iter().all(|a| expr_counter_indexes_vec_types(a, counter_sym, ctx)),
        _ => true,
    }
}

fn expr_counter_indexes_vec_types(expr: &Expr, counter_sym: Symbol, ctx: &RefinementContext) -> bool {
    match expr {
        Expr::Index { collection, index } => {
            let idx_uses_counter = matches!(index, Expr::Identifier(sym) if *sym == counter_sym);
            if idx_uses_counter {
                if let Expr::Identifier(coll_sym) = collection {
                    let is_vec = ctx.get_variable_types().get(coll_sym)
                        .map_or(false, |t| t.starts_with("Vec") || t.starts_with("&[") || t.starts_with("&mut ["));
                    if !is_vec { return false; }
                } else {
                    return false;
                }
            }
            expr_counter_indexes_vec_types(collection, counter_sym, ctx)
                && expr_counter_indexes_vec_types(index, counter_sym, ctx)
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_counter_indexes_vec_types(left, counter_sym, ctx)
                && expr_counter_indexes_vec_types(right, counter_sym, ctx)
        }
        Expr::Call { args, .. } => args.iter().all(|a| expr_counter_indexes_vec_types(a, counter_sym, ctx)),
        Expr::Not { operand } => expr_counter_indexes_vec_types(operand, counter_sym, ctx),
        Expr::Copy { expr: inner } => expr_counter_indexes_vec_types(inner, counter_sym, ctx),
        Expr::Length { collection } => expr_counter_indexes_vec_types(collection, counter_sym, ctx),
        Expr::List(items) => items.iter().all(|e| expr_counter_indexes_vec_types(e, counter_sym, ctx)),
        Expr::Slice { collection, start, end } => {
            expr_counter_indexes_vec_types(collection, counter_sym, ctx)
                && expr_counter_indexes_vec_types(start, counter_sym, ctx)
                && expr_counter_indexes_vec_types(end, counter_sym, ctx)
        }
        _ => true,
    }
}

/// Check if a counter symbol is used ONLY as a direct array index in the body.
/// Returns true when every occurrence of `counter_sym` in the body is either:
/// - The index in `Expr::Index { collection, index: Identifier(counter) }`
/// - The index in `Stmt::SetIndex { collection, index: Identifier(counter), .. }`
///
/// Returns false if the counter appears in any other context (arithmetic,
/// comparison, Show, function arguments, etc.), since shifting the counter
/// to 0-based would change the computation.
fn counter_only_used_for_indexing(stmts: &[Stmt], counter_sym: Symbol) -> bool {
    for stmt in stmts {
        if !check_counter_stmt_indexing_only(stmt, counter_sym) {
            return false;
        }
    }
    true
}

/// Check a single statement for non-index uses of counter.
/// Returns false if the counter is used in a non-index context.
fn check_counter_stmt_indexing_only(stmt: &Stmt, counter_sym: Symbol) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_uses_counter_only_in_index(value, counter_sym),
        Stmt::Set { value, .. } => expr_uses_counter_only_in_index(value, counter_sym),
        Stmt::Show { object, .. } => expr_uses_counter_only_in_index(object, counter_sym),
        Stmt::Push { value, .. } => expr_uses_counter_only_in_index(value, counter_sym),
        Stmt::SetIndex { collection: _, index, value } => {
            // The index position of SetIndex IS a valid index use.
            // Check: is the index expression either Identifier(counter) or does it not
            // reference counter at all? For simplicity, allow only direct Identifier(counter).
            let index_ok = match index {
                Expr::Identifier(sym) if *sym == counter_sym => true,
                _ => !expr_contains_symbol(index, counter_sym),
            };
            let value_ok = expr_uses_counter_only_in_index(value, counter_sym);
            index_ok && value_ok
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_uses_counter_only_in_index(cond, counter_sym)
                && counter_only_used_for_indexing(then_block, counter_sym)
                && else_block.as_ref().map_or(true, |eb| counter_only_used_for_indexing(eb, counter_sym))
        }
        Stmt::While { cond, body, .. } => {
            expr_uses_counter_only_in_index(cond, counter_sym)
                && counter_only_used_for_indexing(body, counter_sym)
        }
        Stmt::Repeat { body, .. } => counter_only_used_for_indexing(body, counter_sym),
        Stmt::Call { args, .. } => {
            args.iter().all(|a| expr_uses_counter_only_in_index(a, counter_sym))
        }
        Stmt::Return { value } => {
            value.map_or(true, |v| expr_uses_counter_only_in_index(v, counter_sym))
        }
        _ => {
            // For other statements, conservatively check they don't reference counter
            true
        }
    }
}

/// Check if an expression uses the counter symbol ONLY inside Expr::Index positions.
/// Returns false if the counter appears in any non-index context.
fn expr_uses_counter_only_in_index(expr: &Expr, counter_sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(sym) => {
            // Bare identifier use of counter = non-index use
            *sym != counter_sym
        }
        Expr::Index { collection, index } => {
            // The index position IS a valid use for the counter
            let collection_ok = expr_uses_counter_only_in_index(collection, counter_sym);
            let index_ok = match index {
                Expr::Identifier(sym) if *sym == counter_sym => true,
                _ => expr_uses_counter_only_in_index(index, counter_sym),
            };
            collection_ok && index_ok
        }
        Expr::BinaryOp { op, left, right } => {
            // For comparison operators, also allow the counter as a bare operand.
            // This enables zero-based normalization for loops like:
            //   While i is at most n: If i is greater than k: ... item i of arr ...
            // The counter `i` appears in both an index and a comparison. In codegen,
            // the comparison operand gets `(i + 1)` to compensate for the 0-based shift.
            match op {
                BinaryOpKind::Lt | BinaryOpKind::LtEq | BinaryOpKind::Gt
                | BinaryOpKind::GtEq | BinaryOpKind::Eq | BinaryOpKind::NotEq => {
                    let left_is_counter = matches!(left, Expr::Identifier(s) if *s == counter_sym);
                    let right_is_counter = matches!(right, Expr::Identifier(s) if *s == counter_sym);
                    if left_is_counter && !expr_contains_symbol(right, counter_sym) {
                        return true;
                    }
                    if right_is_counter && !expr_contains_symbol(left, counter_sym) {
                        return true;
                    }
                    // Counter in both sides or in sub-expressions — fall through
                    expr_uses_counter_only_in_index(left, counter_sym)
                        && expr_uses_counter_only_in_index(right, counter_sym)
                }
                _ => {
                    expr_uses_counter_only_in_index(left, counter_sym)
                        && expr_uses_counter_only_in_index(right, counter_sym)
                }
            }
        }
        Expr::Not { operand } => expr_uses_counter_only_in_index(operand, counter_sym),
        Expr::Call { args, .. } => {
            args.iter().all(|a| expr_uses_counter_only_in_index(a, counter_sym))
        }
        Expr::Length { collection } => expr_uses_counter_only_in_index(collection, counter_sym),
        Expr::Literal(_) => true,
        Expr::List(items) => items.iter().all(|e| expr_uses_counter_only_in_index(e, counter_sym)),
        Expr::Slice { collection, start, end } => {
            expr_uses_counter_only_in_index(collection, counter_sym)
                && expr_uses_counter_only_in_index(start, counter_sym)
                && expr_uses_counter_only_in_index(end, counter_sym)
        }
        Expr::Copy { expr: inner } => expr_uses_counter_only_in_index(inner, counter_sym),
        _ => !expr_contains_symbol(expr, counter_sym),
    }
}

/// Check if an expression contains a specific symbol anywhere.
fn expr_contains_symbol(expr: &Expr, sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => {
            expr_contains_symbol(left, sym) || expr_contains_symbol(right, sym)
        }
        Expr::Not { operand } => expr_contains_symbol(operand, sym),
        Expr::Call { args, .. } => args.iter().any(|a| expr_contains_symbol(a, sym)),
        Expr::Index { collection, index } => {
            expr_contains_symbol(collection, sym) || expr_contains_symbol(index, sym)
        }
        Expr::Length { collection } => expr_contains_symbol(collection, sym),
        Expr::List(items) => items.iter().any(|e| expr_contains_symbol(e, sym)),
        Expr::Slice { collection, start, end } => {
            expr_contains_symbol(collection, sym)
                || expr_contains_symbol(start, sym)
                || expr_contains_symbol(end, sym)
        }
        Expr::Copy { expr: inner } => expr_contains_symbol(inner, sym),
        Expr::Literal(_) => false,
        _ => false,
    }
}

/// Peephole optimization: detect `Let vec = new Seq. [Push val to vec.]* Let i = 0. While i <= limit: push const to vec, i = i+1`
/// and emit `let mut vec: Vec<T> = vec![const; total_count]` with prefix overrides.
///
/// Handles two patterns:
/// - Basic: `new Seq` → counter init → push-loop (existing)
/// - Extended: `new Seq` → 1+ prefix pushes → counter init → push-loop (coins DP pattern)
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None if pattern doesn't match.
pub(crate) fn try_emit_vec_fill_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let [mutable] vec_var be a new Seq of T.
    // Note: mutable keyword is optional — mutability is inferred from Push in the loop body.
    let (vec_sym, elem_type) = match stmts[idx] {
        Stmt::Let { var, value, ty, .. } => {
            // Check for explicit type annotation like `: Seq of Bool`
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(codegen_type_expr(&params[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            // Check for `a new Seq of T`
            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() {
                    if !type_args.is_empty() {
                        Some(codegen_type_expr(&type_args[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(t) => (*var, t),
                None => return None,
            }
        }
        _ => return None,
    };

    // Scan for optional prefix Push statements: `Push const to vec`
    // These are elements pushed before the fill loop (e.g., `Push 1 to dp` in coins).
    let mut prefix_values: Vec<String> = Vec::new();
    let mut cursor = idx + 1;
    while cursor < stmts.len() {
        if let Stmt::Push { value, collection } = stmts[cursor] {
            if let Expr::Identifier(sym) = collection {
                if *sym == vec_sym {
                    let val_str = match value {
                        Expr::Literal(Literal::Number(n)) => Some(format!("{}", n)),
                        Expr::Literal(Literal::Float(f)) => Some(format!("{:.1}", f)),
                        Expr::Literal(Literal::Boolean(b)) => Some(format!("{}", b)),
                        Expr::Literal(Literal::Char(c)) => Some(format!("'{}'", c)),
                        Expr::Literal(Literal::Text(s)) => {
                            Some(format!("String::from(\"{}\")", interner.resolve(*s)))
                        }
                        _ => None,
                    };
                    if let Some(vs) = val_str {
                        prefix_values.push(vs);
                        cursor += 1;
                        continue;
                    }
                }
            }
        }
        break;
    }

    // Need at least 2 more statements: counter init + while loop
    if cursor + 1 >= stmts.len() {
        return None;
    }

    // Counter init: Let [mutable] counter = start_literal  OR  Set counter to start_literal
    let counter_is_new_binding = matches!(stmts[cursor], Stmt::Let { .. });
    let (counter_sym, counter_start) = match stmts[cursor] {
        Stmt::Let { var, value: Expr::Literal(Literal::Number(n)), .. } => {
            (*var, *n)
        }
        Stmt::Set { target, value: Expr::Literal(Literal::Number(n)) } => {
            (*target, *n)
        }
        _ => return None,
    };

    // While loop: counter <= limit (or counter < limit): Push const_val to vec_var. Set counter to counter + 1.
    match stmts[cursor + 1] {
        Stmt::While { cond, body, .. } => {
            // Check condition: counter <= limit OR counter < limit
            let (limit_expr, is_exclusive) = match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (Some(*right), false)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                }
                Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (Some(*right), true)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                }
                _ => (None, false),
            };
            let limit_expr = limit_expr?;

            // Body must have exactly 2 statements: Push and Set
            if body.len() != 2 {
                return None;
            }

            // First body stmt: Push const_val to vec_var
            let push_val = match &body[0] {
                Stmt::Push { value, collection } => {
                    if let Expr::Identifier(sym) = collection {
                        if *sym == vec_sym {
                            Some(*value)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            }?;

            // Push value must be a constant literal
            let fill_val_str = match push_val {
                Expr::Literal(Literal::Number(n)) => format!("{}", n),
                Expr::Literal(Literal::Float(f)) => format!("{:.1}", f),
                Expr::Literal(Literal::Boolean(b)) => format!("{}", b),
                Expr::Literal(Literal::Char(c)) => format!("'{}'", c),
                Expr::Literal(Literal::Text(s)) => {
                    format!("String::from(\"{}\")", interner.resolve(*s))
                }
                _ => return None,
            };

            // Second body stmt: Set counter to counter + 1
            match &body[1] {
                Stmt::Set { target, value, .. } => {
                    if *target != counter_sym {
                        return None;
                    }
                    // Value must be counter + 1
                    match value {
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            let is_counter_plus_1 = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym => true,
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym => true,
                                _ => false,
                            };
                            if !is_counter_plus_1 {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }

            // Pattern matched! Emit optimized code.
            let indent_str = "    ".repeat(indent);
            let vec_name = interner.resolve(vec_sym);
            let limit_str = codegen_expr_simple(limit_expr, interner);
            let prefix_count = prefix_values.len();

            // Calculate loop iteration count (without prefix)
            // Inclusive (<=): loop_count = limit - start + 1
            // Exclusive (<):  loop_count = limit - start
            let raw_loop_count = if is_exclusive {
                if counter_start == 0 {
                    limit_str.clone()
                } else {
                    format!("({} - {})", limit_str, counter_start)
                }
            } else {
                if counter_start == 0 {
                    format!("({} + 1)", limit_str)
                } else if counter_start == 1 {
                    limit_str.clone()
                } else {
                    format!("({} - {} + 1)", limit_str, counter_start)
                }
            };

            // Total count = prefix elements + loop iterations
            let count_expr = if prefix_count == 0 {
                format!("{} as usize", raw_loop_count)
            } else {
                format!("({} + {}) as usize", prefix_count, raw_loop_count)
            };

            let mut output = String::new();
            writeln!(output, "{}let mut {}: Vec<{}> = vec![{}; {}];",
                indent_str, vec_name, elem_type, fill_val_str, count_expr).unwrap();

            ctx.register_variable_type(vec_sym, format!("Vec<{}>", elem_type));

            // Emit prefix element overrides (only for values different from fill)
            for (i, prefix_val) in prefix_values.iter().enumerate() {
                if *prefix_val != fill_val_str {
                    writeln!(output, "{}{}[{}] = {};",
                        indent_str, vec_name, i, prefix_val).unwrap();
                }
            }

            // Re-emit counter variable (it may be reused after the fill loop)
            let names = RustNames::new(interner);
            let counter_name = names.ident(counter_sym);
            if counter_is_new_binding {
                writeln!(output, "{}let mut {} = {};",
                    indent_str, counter_name, counter_start).unwrap();
            } else {
                writeln!(output, "{}{} = {};",
                    indent_str, counter_name, counter_start).unwrap();
            }

            // Extra consumed: prefix pushes + counter init + while loop
            let extra_consumed = (cursor - idx) + 1;
            Some((output, extra_consumed))
        }
        _ => None,
    }
}

/// Check if an expression can be handled by codegen_expr_simple without fallback.
fn is_simple_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(Literal::Number(_))
        | Expr::Literal(Literal::Float(_))
        | Expr::Literal(Literal::Boolean(_))
        | Expr::Identifier(_) => true,
        Expr::BinaryOp { op, left, right } => {
            matches!(op,
                BinaryOpKind::Add | BinaryOpKind::Subtract |
                BinaryOpKind::Multiply | BinaryOpKind::Divide | BinaryOpKind::Modulo
            ) && is_simple_expr(left) && is_simple_expr(right)
        }
        Expr::Length { collection } => {
            matches!(collection, Expr::Identifier(_))
        }
        _ => false,
    }
}

/// Simple expression codegen for peephole patterns (no async/context needed).
fn codegen_expr_simple(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Literal(Literal::Number(n)) => format!("{}", n),
        Expr::Literal(Literal::Float(f)) => format!("{:.1}", f),
        Expr::Literal(Literal::Boolean(b)) => format!("{}", b),
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            let l = codegen_expr_simple(left, interner);
            let r = codegen_expr_simple(right, interner);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                _ => return format!("({})", l),
            };
            format!("({} {} {})", l, op_str, r)
        }
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                format!("({}.len() as i64)", interner.resolve(*sym))
            } else {
                "_".to_string()
            }
        }
        _ => "_".to_string(),
    }
}

/// Peephole optimization: detect `Let mutable text be ""` followed by a counted
/// loop that self-appends to `text`, and emit `String::with_capacity(n as usize)`
/// instead of `String::from("")`.
///
/// Pattern:
///   Let mutable text be "".
///   Let counter be start.
///   While counter < limit: ... Set text to text + ...; counter++
///
/// The limit expression gives us the capacity hint. For exclusive (<), capacity = limit - start.
/// For inclusive (<=), capacity = limit - start + 1.
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_string_with_capacity_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let mutable text be "".
    let str_sym = match stmts[idx] {
        Stmt::Let { var, value, mutable, .. } => {
            if !*mutable && !mutable_vars.contains(var) {
                return None;
            }
            if let Expr::Literal(Literal::Text(sym)) = value {
                if interner.resolve(*sym).is_empty() {
                    *var
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Scan forward for a counter-init + While pair where the loop body appends to str_sym.
    // Skip intervening statements that don't reference str_sym.
    for scan in (idx + 1)..stmts.len() {
        let stmt = stmts[scan];

        // Try: is this a counter init?
        let is_counter_init = match stmt {
            Stmt::Let { value, .. } if is_simple_expr(value) => true,
            Stmt::Set { value, .. } if is_simple_expr(value) => true,
            _ => false,
        };

        if is_counter_init && scan + 1 < stmts.len() {
            let (counter_sym, start_expr) = match stmt {
                Stmt::Let { var, value, .. } => (*var, *value),
                Stmt::Set { target, value } => (*target, *value),
                _ => unreachable!(),
            };

            if let Stmt::While { cond, body, .. } = stmts[scan + 1] {
                // Check condition shape: counter < limit or counter <= limit
                let (limit_expr, is_exclusive) = match cond {
                    Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
                        if matches!(left, Expr::Identifier(sym) if *sym == counter_sym) {
                            (Some(*right), true)
                        } else {
                            (None, false)
                        }
                    }
                    Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                        if matches!(left, Expr::Identifier(sym) if *sym == counter_sym) {
                            (Some(*right), false)
                        } else {
                            (None, false)
                        }
                    }
                    _ => (None, false),
                };

                if let Some(limit_expr) = limit_expr {
                    if !is_simple_expr(limit_expr) {
                        continue;
                    }

                    // Check body ends with counter increment
                    if body.len() >= 2 {
                        let last_is_increment = match &body[body.len() - 1] {
                            Stmt::Set { target, value } if *target == counter_sym => {
                                matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                                    if (matches!(left, Expr::Identifier(s) if *s == counter_sym) && matches!(right, Expr::Literal(Literal::Number(1))))
                                    || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == counter_sym))
                                )
                            }
                            _ => false,
                        };

                        if last_is_increment {
                            // Check that loop body appends to str_sym (via Set self-append)
                            let body_appends = body_has_string_self_append(body, str_sym);
                            if body_appends {
                                // Pattern matched! Emit with_capacity.
                                let indent_str = "    ".repeat(indent);
                                let var_name = interner.resolve(str_sym);
                                let limit_str = codegen_expr_simple(limit_expr, interner);
                                let start_str = codegen_expr_simple(start_expr, interner);

                                let capacity_expr = if is_exclusive {
                                    match start_expr {
                                        Expr::Literal(Literal::Number(0)) => limit_str.clone(),
                                        Expr::Literal(Literal::Number(s)) => {
                                            if let Expr::Literal(Literal::Number(n)) = limit_expr {
                                                format!("{}", n - s)
                                            } else {
                                                format!("({} - {})", limit_str, s)
                                            }
                                        }
                                        _ => format!("({} - {})", limit_str, start_str),
                                    }
                                } else {
                                    match start_expr {
                                        Expr::Literal(Literal::Number(0)) => {
                                            if let Expr::Literal(Literal::Number(n)) = limit_expr {
                                                format!("{}", n + 1)
                                            } else {
                                                format!("({} + 1)", limit_str)
                                            }
                                        }
                                        Expr::Literal(Literal::Number(1)) => limit_str.clone(),
                                        Expr::Literal(Literal::Number(s)) => {
                                            if let Expr::Literal(Literal::Number(n)) = limit_expr {
                                                format!("{}", n - s + 1)
                                            } else {
                                                format!("({} - {} + 1)", limit_str, s)
                                            }
                                        }
                                        _ => format!("({} - {} + 1)", limit_str, start_str),
                                    }
                                };

                                let mut output = String::new();
                                writeln!(output, "{}let mut {} = String::with_capacity({} as usize);",
                                    indent_str, var_name, capacity_expr).unwrap();

                                // Register as string var
                                ctx.register_string_var(str_sym);

                                // Now emit the remaining statements normally (counter init + while loop)
                                // via for-range pattern or fallback.
                                // We consumed 0 extra statements — only replaced the Let.
                                // The counter init + while will be processed by subsequent peephole passes.
                                return Some((output, 0));
                            }
                        }
                    }
                }
            }
        }

        // If this statement references str_sym, bail out.
        if symbol_appears_in_stmts(str_sym, &[stmt]) {
            return None;
        }
    }

    None
}

/// Check if a loop body contains a self-append to the string variable.
/// Looks for `Set str_sym to str_sym + ...` pattern anywhere in the body.
fn body_has_string_self_append(stmts: &[Stmt], str_sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, value } if *target == str_sym => {
                // Check for self-append: target + something
                if let Expr::BinaryOp { op: BinaryOpKind::Add, left, .. } = value {
                    if matches!(left, Expr::Identifier(sym) if *sym == str_sym) {
                        return true;
                    }
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if body_has_string_self_append(then_block, str_sym) {
                    return true;
                }
                if let Some(eb) = else_block {
                    if body_has_string_self_append(eb, str_sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if body_has_string_self_append(body, str_sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Convert a LOGOS 1-based index expression to a Rust 0-based index string.
///
/// Algebraic simplifications:
///   Literal(1)      → "0"
///   Literal(N)      → "N-1"  (compile-time constant)
///   (X + 1)         → "X as usize"   (or just "X" if raw=true)
///   (1 + X)         → "X as usize"
///   (X + K)         → "(X + K-1) as usize"  where K is literal > 1
///   fallback        → "(expr - 1) as usize"
///
/// When `include_as_usize` is false, the result omits " as usize" (for use in `.swap()` calls
/// where the caller adds it).
pub(crate) fn simplify_1based_index(expr: &Expr, interner: &Interner, include_as_usize: bool) -> String {
    let cast = if include_as_usize { " as usize" } else { "" };

    match expr {
        // Literal(1) → 0
        Expr::Literal(Literal::Number(1)) => "0".to_string(),
        // Literal(N) → N-1 (compile-time constant, no cast needed)
        Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
        // (X + K) where K is a literal
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            match (left, right) {
                // (X + 1) → X
                (_, Expr::Literal(Literal::Number(1))) => {
                    let inner = codegen_expr_simple(left, interner);
                    if include_as_usize {
                        format!("({}){}", inner, cast)
                    } else {
                        inner
                    }
                }
                // (1 + X) → X
                (Expr::Literal(Literal::Number(1)), _) => {
                    let inner = codegen_expr_simple(right, interner);
                    if include_as_usize {
                        format!("({}){}", inner, cast)
                    } else {
                        inner
                    }
                }
                // (X + K) where K > 1 → (X + K-1)
                (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                    let inner = codegen_expr_simple(left, interner);
                    format!("({} + {}){}", inner, k - 1, cast)
                }
                // (K + X) where K > 1 → (X + K-1)
                (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                    let inner = codegen_expr_simple(right, interner);
                    format!("({} + {}){}", inner, k - 1, cast)
                }
                // Fallback: (expr - 1)
                _ => {
                    let full = codegen_expr_simple(expr, interner);
                    format!("({} - 1){}", full, cast)
                }
            }
        }
        // Fallback: (expr - 1)
        _ => {
            let full = codegen_expr_simple(expr, interner);
            format!("({} - 1){}", full, cast)
        }
    }
}

/// Collect all arrays that are indexed by expressions involving `counter_sym` in the body.
/// Used for OPT-4 and OPT-9 bounds check elision hints.
pub(crate) fn collect_indexed_arrays(stmts: &[Stmt], counter_sym: Symbol) -> Vec<Symbol> {
    let mut arrays = Vec::new();
    let mut seen = HashSet::new();
    for stmt in stmts {
        collect_indexed_arrays_from_stmt(stmt, counter_sym, &mut arrays, &mut seen);
    }
    arrays
}

fn collect_indexed_arrays_from_stmt(stmt: &Stmt, counter_sym: Symbol, arrays: &mut Vec<Symbol>, seen: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Set { value, .. } => collect_indexed_arrays_from_expr(value, counter_sym, arrays, seen),
        Stmt::Let { value, .. } => collect_indexed_arrays_from_expr(value, counter_sym, arrays, seen),
        Stmt::Show { object, .. } => collect_indexed_arrays_from_expr(object, counter_sym, arrays, seen),
        Stmt::Push { value, .. } => collect_indexed_arrays_from_expr(value, counter_sym, arrays, seen),
        Stmt::SetIndex { value, index, .. } => {
            collect_indexed_arrays_from_expr(value, counter_sym, arrays, seen);
            collect_indexed_arrays_from_expr(index, counter_sym, arrays, seen);
        }
        Stmt::If { cond, then_block, else_block } => {
            collect_indexed_arrays_from_expr(cond, counter_sym, arrays, seen);
            for s in then_block.iter() { collect_indexed_arrays_from_stmt(s, counter_sym, arrays, seen); }
            if let Some(el) = else_block {
                for s in el.iter() { collect_indexed_arrays_from_stmt(s, counter_sym, arrays, seen); }
            }
        }
        _ => {}
    }
}

fn collect_indexed_arrays_from_expr(expr: &Expr, counter_sym: Symbol, arrays: &mut Vec<Symbol>, seen: &mut HashSet<Symbol>) {
    match expr {
        Expr::Index { collection, index } => {
            if expr_involves_symbol(index, counter_sym) {
                if let Expr::Identifier(sym) = collection {
                    if seen.insert(*sym) {
                        arrays.push(*sym);
                    }
                }
            }
            collect_indexed_arrays_from_expr(collection, counter_sym, arrays, seen);
            collect_indexed_arrays_from_expr(index, counter_sym, arrays, seen);
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_indexed_arrays_from_expr(left, counter_sym, arrays, seen);
            collect_indexed_arrays_from_expr(right, counter_sym, arrays, seen);
        }
        Expr::Not { operand } => collect_indexed_arrays_from_expr(operand, counter_sym, arrays, seen),
        Expr::Length { collection } => collect_indexed_arrays_from_expr(collection, counter_sym, arrays, seen),
        _ => {}
    }
}

fn expr_involves_symbol(expr: &Expr, sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => {
            expr_involves_symbol(left, sym) || expr_involves_symbol(right, sym)
        }
        _ => false,
    }
}

/// Check if two expressions are structurally equal (for swap pattern detection, sentinel detection).
pub(crate) fn exprs_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(s1), Expr::Identifier(s2)) => s1 == s2,
        (Expr::Literal(Literal::Number(n1)), Expr::Literal(Literal::Number(n2))) => n1 == n2,
        (Expr::BinaryOp { op: op1, left: l1, right: r1 }, Expr::BinaryOp { op: op2, left: l2, right: r2 }) => {
            op1 == op2 && exprs_equal(l1, l2) && exprs_equal(r1, r2)
        }
        _ => false,
    }
}

/// Peephole optimization: detect swap patterns and emit `arr.swap()` instead.
///
/// Pattern A (conditional, adjacent indices — bubble sort):
///   Let a be item j of arr. Let b be item (j+1) of arr.
///   If a > b then: Set item j of arr to b. Set item (j+1) of arr to a.
/// → `if arr[j-1] > arr[j] { arr.swap(j-1, j); }`
///
/// Pattern B (unconditional, any indices — quicksort/heapsort):
///   Let tmp be item I of arr.
///   Set item I of arr to item J of arr.
///   Set item J of arr to tmp.
/// → `arr.swap((I-1) as usize, (J-1) as usize);`
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_swap_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let tmp be item I of arr (index expression)
    let (a_sym, arr_sym_1, idx_expr_1) = match stmts[idx] {
        Stmt::Let { var, value: Expr::Index { collection, index }, mutable: false, .. } => {
            if let Expr::Identifier(coll_sym) = collection {
                (*var, *coll_sym, *index)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Only optimize for known Vec or &mut [T] types (direct indexing)
    if let Some(t) = variable_types.get(&arr_sym_1) {
        if !t.starts_with("Vec") && !t.starts_with("&mut [") {
            return None;
        }
    } else {
        return None;
    }

    // Try Pattern B first: unconditional 3-statement swap (more general)
    // Statement 2: Set item I of arr to item J of arr.
    // Statement 3: Set item J of arr to tmp.
    if let Some(result) = try_emit_unconditional_swap(stmts, idx, a_sym, arr_sym_1, idx_expr_1, interner, indent) {
        return Some(result);
    }

    // Pattern A: conditional swap with any two indices from the same array.
    // Statement 2: Let b be item J of arr (J can be any index, not just I+1)
    let (b_sym, arr_sym_2, idx_expr_2) = match stmts[idx + 1] {
        Stmt::Let { var, value: Expr::Index { collection, index }, mutable: false, .. } => {
            if let Expr::Identifier(coll_sym) = collection {
                (*var, *coll_sym, *index)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Must be the same array
    if arr_sym_1 != arr_sym_2 {
        return None;
    }

    // Both index expressions must be simple enough for codegen_expr_simple
    if !is_simple_expr(idx_expr_1) || !is_simple_expr(idx_expr_2) {
        return None;
    }

    // Statement 3: If a OP b: SetIndex arr I b, SetIndex arr J a (cross-swap)
    match stmts[idx + 2] {
        Stmt::If { cond, then_block, else_block } => {
            // Condition must compare a and b
            let compares_a_b = match cond {
                Expr::BinaryOp { op, left, right } => {
                    matches!(op, BinaryOpKind::Gt | BinaryOpKind::Lt | BinaryOpKind::GtEq | BinaryOpKind::LtEq | BinaryOpKind::Eq | BinaryOpKind::NotEq) &&
                    ((matches!(left, Expr::Identifier(s) if *s == a_sym) && matches!(right, Expr::Identifier(s) if *s == b_sym)) ||
                     (matches!(left, Expr::Identifier(s) if *s == b_sym) && matches!(right, Expr::Identifier(s) if *s == a_sym)))
                }
                _ => false,
            };
            if !compares_a_b {
                return None;
            }

            // Must have no else block
            if else_block.is_some() {
                return None;
            }

            // Then block must have exactly 2 SetIndex statements forming a cross-swap
            if then_block.len() != 2 {
                return None;
            }

            // Check: SetIndex arr idx1 b, SetIndex arr idx2 a (cross pattern)
            let swap_ok = match (&then_block[0], &then_block[1]) {
                (
                    Stmt::SetIndex { collection: c1, index: i1, value: v1 },
                    Stmt::SetIndex { collection: c2, index: i2, value: v2 },
                ) => {
                    // c1 and c2 must be the same array
                    let same_arr = matches!((c1, c2), (Expr::Identifier(s1), Expr::Identifier(s2)) if *s1 == arr_sym_1 && *s2 == arr_sym_1);
                    // Cross pattern: set idx1 to b, set idx2 to a
                    let cross = exprs_equal(i1, idx_expr_1) && exprs_equal(i2, idx_expr_2) &&
                        matches!(v1, Expr::Identifier(s) if *s == b_sym) &&
                        matches!(v2, Expr::Identifier(s) if *s == a_sym);
                    // Also check reverse: set idx1 to b via idx2/a pattern
                    let cross_rev = exprs_equal(i1, idx_expr_2) && exprs_equal(i2, idx_expr_1) &&
                        matches!(v1, Expr::Identifier(s) if *s == a_sym) &&
                        matches!(v2, Expr::Identifier(s) if *s == b_sym);
                    same_arr && (cross || cross_rev)
                }
                _ => false,
            };

            if !swap_ok {
                return None;
            }

            // Pattern matched! Emit optimized swap
            let indent_str = "    ".repeat(indent);
            let arr_name = interner.resolve(arr_sym_1);
            let idx1_simplified = simplify_1based_index(idx_expr_1, interner, true);
            let idx2_simplified = simplify_1based_index(idx_expr_2, interner, true);

            let op_str = match cond {
                Expr::BinaryOp { op, .. } => match op {
                    BinaryOpKind::Gt => ">", BinaryOpKind::Lt => "<",
                    BinaryOpKind::GtEq => ">=", BinaryOpKind::LtEq => "<=",
                    BinaryOpKind::Eq => "==", BinaryOpKind::NotEq => "!=",
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };

            let mut output = String::new();
            writeln!(output, "{}if {}[{}] {} {}[{}] {{",
                indent_str, arr_name, idx1_simplified, op_str, arr_name, idx2_simplified,
            ).unwrap();
            writeln!(output, "{}    let __swap_tmp = {}[{}];",
                indent_str, arr_name, idx1_simplified).unwrap();
            writeln!(output, "{}    {}[{}] = {}[{}];",
                indent_str, arr_name, idx1_simplified, arr_name, idx2_simplified).unwrap();
            writeln!(output, "{}    {}[{}] = __swap_tmp;",
                indent_str, arr_name, idx2_simplified).unwrap();
            writeln!(output, "{}}}", indent_str).unwrap();

            Some((output, 2)) // consumed 2 extra statements
        }
        _ => None,
    }
}

/// Peephole optimization: detect full-array copy via push loop and emit `.to_vec()`.
///
/// Pattern:
///   Let [mutable] dst be a new Seq of T.
///   Set counter to 1.                           (counter already declared)
///   While counter <= length of src:
///       Push item counter of src to dst.
///       Set counter to counter + 1.
/// → `let mut dst: Vec<T> = src.to_vec();`
///
/// If the counter appears in subsequent statements, the post-loop value
/// `src.len() as i64 + 1` is emitted so callers can rely on it.
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_seq_copy_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let mutable dst be a new Seq of T.
    let (dst_sym, elem_type) = match stmts[idx] {
        Stmt::Let { var, value, mutable: true, .. } => {
            if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
                    (*var, codegen_type_expr(&type_args[0], interner))
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 2: Set counter to 1  (counter must already be declared — this is a reset)
    let counter_sym = match stmts[idx + 1] {
        Stmt::Set { target, value: Expr::Literal(Literal::Number(1)) } => *target,
        _ => return None,
    };

    // Statement 3: While counter <= length of src: Push item counter of src to dst; Set counter++
    match stmts[idx + 2] {
        Stmt::While { cond, body, .. } => {
            // Condition: counter <= length of src
            let src_sym = match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym {
                            if let Expr::Length { collection } = right {
                                if let Expr::Identifier(s) = collection {
                                    *s
                                } else {
                                    return None;
                                }
                            } else {
                                return None;
                            }
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };

            // Body must have exactly 2 statements
            if body.len() != 2 {
                return None;
            }

            // Body[0]: Push item counter of src to dst
            match &body[0] {
                Stmt::Push { value, collection } => {
                    if !matches!(collection, Expr::Identifier(s) if *s == dst_sym) {
                        return None;
                    }
                    if let Expr::Index { collection: idx_coll, index: idx_expr } = value {
                        if !matches!(idx_coll, Expr::Identifier(s) if *s == src_sym) {
                            return None;
                        }
                        if !matches!(idx_expr, Expr::Identifier(s) if *s == counter_sym) {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }

            // Body[1]: Set counter to counter + 1
            match &body[1] {
                Stmt::Set { target, value } => {
                    if *target != counter_sym {
                        return None;
                    }
                    match value {
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            let ok = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) => *s == counter_sym,
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) => *s == counter_sym,
                                _ => false,
                            };
                            if !ok {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }

            // Pattern matched! Emit: let mut dst: Vec<T> = src.to_vec();
            let indent_str = "    ".repeat(indent);
            let dst_name = interner.resolve(dst_sym);
            let src_name = interner.resolve(src_sym);
            let names = RustNames::new(interner);
            let counter_name = names.ident(counter_sym);

            let mut output = String::new();
            writeln!(output, "{}let mut {}: Vec<{}> = {}.to_vec();",
                indent_str, dst_name, elem_type, src_name).unwrap();
            ctx.register_variable_type(dst_sym, format!("Vec<{}>", elem_type));

            // If the counter appears after the loop, emit its post-loop value.
            let remaining = &stmts[idx + 3..];
            if symbol_appears_in_stmts(counter_sym, remaining) {
                writeln!(output, "{}{} = {}.len() as i64 + 1;",
                    indent_str, counter_name, src_name).unwrap();
            }

            Some((output, 2)) // consumed: counter-reset + While = 2 extra
        }
        _ => None,
    }
}

/// Peephole optimization: detect element-by-element array copying via push loop
/// and emit slice operations instead.
///
/// Pattern:
///   Let [mutable] dst be a new Seq of T.
///   [intervening statements that don't reference dst]
///   Let/Set counter = start.
///   While counter <= end:
///       Push item counter of source to dst.
///       Set counter to counter + 1.
///
/// Relaxed pattern: allows arbitrary intervening statements between the Seq creation
/// and the counter-init + While pair, as long as they don't reference the Seq variable.
///
/// Full copy (start=1, end=length of source): `let mut dst: Vec<T> = source.to_vec();`
/// Partial slice: `let mut dst: Vec<T> = source[(start-1) as usize..end as usize].to_vec();`
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_seq_from_slice_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let mutable dst be a new Seq of T.
    let (dst_sym, elem_type) = match stmts[idx] {
        Stmt::Let { var, value, ty, mutable: true, .. } => {
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(codegen_type_expr(&params[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() {
                    if !type_args.is_empty() {
                        Some(codegen_type_expr(&type_args[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(t) => (*var, t),
                None => return None,
            }
        }
        _ => return None,
    };

    // Scan forward from idx+1 for a counter-init + While pair.
    // Skip intervening statements that don't reference dst_sym.
    let mut counter_init_idx: Option<usize> = None;

    for scan in (idx + 1)..stmts.len() {
        let stmt = stmts[scan];

        let is_counter_init = match stmt {
            Stmt::Let { value, .. } if is_simple_expr(value) => true,
            Stmt::Set { value, .. } if is_simple_expr(value) => true,
            _ => false,
        };

        if is_counter_init && scan + 1 < stmts.len() {
            let c_sym = match stmt {
                Stmt::Let { var, .. } => *var,
                Stmt::Set { target, .. } => *target,
                _ => unreachable!(),
            };

            if let Stmt::While { cond, body, .. } = stmts[scan + 1] {
                let cond_ok = match cond {
                    Expr::BinaryOp { op: BinaryOpKind::LtEq | BinaryOpKind::Lt, left, .. } => {
                        matches!(left, Expr::Identifier(sym) if *sym == c_sym)
                    }
                    _ => false,
                };

                if cond_ok && body.len() == 2 {
                    let push_to_dst = match &body[0] {
                        Stmt::Push { collection, value } => {
                            if !matches!(collection, Expr::Identifier(s) if *s == dst_sym) {
                                false
                            } else if let Expr::Index { index, .. } = value {
                                matches!(index, Expr::Identifier(s) if *s == c_sym)
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };

                    let inc_ok = match &body[1] {
                        Stmt::Set { target, value } if *target == c_sym => {
                            matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                                if (matches!(left, Expr::Identifier(s) if *s == c_sym) && matches!(right, Expr::Literal(Literal::Number(1))))
                                || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == c_sym))
                            )
                        }
                        _ => false,
                    };

                    if push_to_dst && inc_ok {
                        counter_init_idx = Some(scan);
                        break;
                    }
                }
            }
        }

        // Continuation slice: bare While with no counter re-init.
        // The counter was already initialized by a prior loop and carries over.
        if let Stmt::While { cond, body, .. } = stmt {
            if body.len() == 2 {
                if let Some((c_sym, c_end_expr, c_is_exclusive)) = extract_while_cond(cond) {
                    if is_simple_expr(c_end_expr) {
                        if let Some((c_src_sym, c_dst_check)) = extract_push_index_body(body, c_sym) {
                            if c_dst_check == dst_sym {
                                // Continuation slice matched! The counter's current runtime
                                // value is the slice start.
                                let indent_str = "    ".repeat(indent);
                                let names = RustNames::new(interner);
                                let dst_name = interner.resolve(dst_sym);
                                let src_name = interner.resolve(c_src_sym);
                                let counter_name = names.ident(c_sym);
                                let end_str = codegen_expr_simple(c_end_expr, interner);

                                let mut cont_output = String::new();

                                if c_is_exclusive {
                                    writeln!(cont_output, "{}let mut {}: Vec<{}> = {}[({} - 1) as usize..({} - 1) as usize].to_vec();",
                                        indent_str, dst_name, elem_type, src_name, counter_name, end_str).unwrap();
                                } else {
                                    writeln!(cont_output, "{}let mut {}: Vec<{}> = {}[({} - 1) as usize..{} as usize].to_vec();",
                                        indent_str, dst_name, elem_type, src_name, counter_name, end_str).unwrap();
                                }

                                ctx.register_variable_type(dst_sym, format!("Vec<{}>", elem_type));

                                // Emit intervening statements between Seq creation and the While
                                for si in (idx + 1)..scan {
                                    use super::codegen_stmt;
                                    cont_output.push_str(&codegen_stmt(stmts[si], interner, indent, mutable_vars, ctx,
                                        lww_fields, mv_fields, synced_vars, var_caps, async_functions,
                                        pipe_vars, boxed_fields, registry, type_env));
                                }

                                // Re-emit counter post-loop value if used after the While
                                let remaining = &stmts[scan + 1..];
                                if symbol_appears_in_stmts(c_sym, remaining) {
                                    let post_val = if c_is_exclusive {
                                        end_str.to_string()
                                    } else {
                                        if let Expr::Literal(Literal::Number(n)) = c_end_expr {
                                            format!("{}", n + 1)
                                        } else {
                                            format!("{} + 1", end_str)
                                        }
                                    };
                                    writeln!(cont_output, "{}{} = {};", indent_str, counter_name, post_val).unwrap();
                                }

                                let extra_consumed = scan - idx;
                                return Some((cont_output, extra_consumed));
                            }
                        }
                    }
                }
            }
        }

        // This statement doesn't start the pattern. If it references dst_sym, bail.
        if symbol_appears_in_stmts(dst_sym, &[stmt]) {
            return None;
        }
    }

    let counter_idx = counter_init_idx?;
    let while_idx = counter_idx + 1;

    // Extract counter details
    let (counter_sym, start_expr, counter_is_new_binding) = match stmts[counter_idx] {
        Stmt::Let { var, value, .. } => {
            if is_simple_expr(value) {
                (*var, *value, true)
            } else {
                return None;
            }
        }
        Stmt::Set { target, value } => {
            if is_simple_expr(value) {
                (*target, *value, false)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Match the While loop for detailed extraction
    match stmts[while_idx] {
        Stmt::While { cond, body, .. } => {
            let (end_expr, is_exclusive) = match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym { (Some(*right), false) } else { (None, false) }
                    } else { (None, false) }
                }
                Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym { (Some(*right), true) } else { (None, false) }
                    } else { (None, false) }
                }
                _ => (None, false),
            };
            let end_expr = end_expr?;

            if body.len() != 2 {
                return None;
            }

            let src_sym = match &body[0] {
                Stmt::Push { value, collection } => {
                    if !matches!(collection, Expr::Identifier(s) if *s == dst_sym) {
                        return None;
                    }
                    if let Expr::Index { collection: idx_coll, index: idx_expr } = value {
                        if !matches!(idx_expr, Expr::Identifier(s) if *s == counter_sym) {
                            return None;
                        }
                        if let Expr::Identifier(s) = idx_coll {
                            *s
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };

            match &body[1] {
                Stmt::Set { target, value } => {
                    if *target != counter_sym { return None; }
                    match value {
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            let ok = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) => *s == counter_sym,
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) => *s == counter_sym,
                                _ => false,
                            };
                            if !ok { return None; }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }

            // Pattern matched! Determine if it's a full copy or partial slice.
            let indent_str = "    ".repeat(indent);
            let names = RustNames::new(interner);
            let dst_name = interner.resolve(dst_sym);
            let src_name = interner.resolve(src_sym);
            let counter_name = names.ident(counter_sym);

            let is_start_one = matches!(start_expr, Expr::Literal(Literal::Number(1)));
            let is_end_length_of_src = if !is_exclusive {
                matches!(end_expr, Expr::Length { collection } if matches!(collection, Expr::Identifier(s) if *s == src_sym))
            } else {
                false
            };

            let mut output = String::new();

            if is_start_one && is_end_length_of_src {
                writeln!(output, "{}let mut {}: Vec<{}> = {}.to_vec();",
                    indent_str, dst_name, elem_type, src_name).unwrap();
            } else {
                let start_str = codegen_expr_simple(start_expr, interner);
                let end_str = codegen_expr_simple(end_expr, interner);

                if is_exclusive {
                    if matches!(start_expr, Expr::Literal(Literal::Number(1))) {
                        writeln!(output, "{}let mut {}: Vec<{}> = {}[..({} - 1) as usize].to_vec();",
                            indent_str, dst_name, elem_type, src_name, end_str).unwrap();
                    } else {
                        writeln!(output, "{}let mut {}: Vec<{}> = {}[({} - 1) as usize..({} - 1) as usize].to_vec();",
                            indent_str, dst_name, elem_type, src_name, start_str, end_str).unwrap();
                    }
                } else {
                    if matches!(start_expr, Expr::Literal(Literal::Number(1))) {
                        writeln!(output, "{}let mut {}: Vec<{}> = {}[..{} as usize].to_vec();",
                            indent_str, dst_name, elem_type, src_name, end_str).unwrap();
                    } else {
                        writeln!(output, "{}let mut {}: Vec<{}> = {}[({} - 1) as usize..{} as usize].to_vec();",
                            indent_str, dst_name, elem_type, src_name, start_str, end_str).unwrap();
                    }
                }
            }

            ctx.register_variable_type(dst_sym, format!("Vec<{}>", elem_type));

            // Emit intervening statements between Seq creation and counter init
            for si in (idx + 1)..counter_idx {
                use super::codegen_stmt;
                output.push_str(&codegen_stmt(stmts[si], interner, indent, mutable_vars, ctx,
                    lww_fields, mv_fields, synced_vars, var_caps, async_functions,
                    pipe_vars, boxed_fields, registry, type_env));
            }

            // Re-emit counter if it's used after the While
            let remaining = &stmts[while_idx + 1..];
            if symbol_appears_in_stmts(counter_sym, remaining) {
                let end_str = codegen_expr_simple(end_expr, interner);
                let post_val = if is_exclusive {
                    end_str.to_string()
                } else {
                    format!("{} + 1", end_str)
                };
                if counter_is_new_binding {
                    writeln!(output, "{}let mut {} = {};", indent_str, counter_name, post_val).unwrap();
                } else {
                    writeln!(output, "{}{} = {};", indent_str, counter_name, post_val).unwrap();
                }
            }

            let extra_consumed = while_idx - idx;
            Some((output, extra_consumed))
        }
        _ => None,
    }
}

/// Peephole optimization: detect `new Seq` followed by a counted push loop and emit
/// `Vec::with_capacity(count)` or `vec![value; N]` instead of `Seq::default()`.
///
/// Relaxed pattern: allows arbitrary intervening statements between the Seq creation
/// and the counter-init + While pair, as long as they don't reference the Seq variable.
/// Scans forward until the pattern is found or a statement touching vec is encountered.
///
/// For Copy types with constant push values, emits `vec![value; N]` which enables
/// LLVM to know the length at the declaration site and elide bounds checks.
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_vec_with_capacity_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let [mutable] vec_var = new Seq of T (or Map of K to V)
    let (vec_sym, collection_info) = match stmts[idx] {
        Stmt::Let { var, value, ty, .. } => {
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(CollInfo::Vec(codegen_type_expr(&params[0], interner)))
                } else if matches!(base_name, "Map" | "HashMap") && params.len() >= 2 {
                    Some(CollInfo::Map(
                        codegen_type_expr(&params[0], interner),
                        codegen_type_expr(&params[1], interner),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
                    Some(CollInfo::Vec(codegen_type_expr(&type_args[0], interner)))
                } else if matches!(tn, "Map" | "HashMap") && init_fields.is_empty() && type_args.len() >= 2 {
                    Some(CollInfo::Map(
                        codegen_type_expr(&type_args[0], interner),
                        codegen_type_expr(&type_args[1], interner),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(info) => (*var, info),
                None => return None,
            }
        }
        _ => return None,
    };

    // Scan forward from idx+1 for a counter-init + While pair.
    // Skip intervening statements that don't reference vec_sym.
    // Stop scanning when we hit a statement that touches vec_sym or find the pattern.
    let mut counter_init_idx: Option<usize> = None;

    for scan in (idx + 1)..stmts.len() {
        let stmt = stmts[scan];

        // Try: is this a counter init (Let/Set with simple expr)?
        let is_counter_init = match stmt {
            Stmt::Let { value, .. } if is_simple_expr(value) => true,
            Stmt::Set { value, .. } if is_simple_expr(value) => true,
            _ => false,
        };

        if is_counter_init && scan + 1 < stmts.len() {
            let counter_sym = match stmt {
                Stmt::Let { var, .. } => *var,
                Stmt::Set { target, .. } => *target,
                _ => unreachable!(),
            };

            if let Stmt::While { cond, body, .. } = stmts[scan + 1] {
                let loop_matches = match cond {
                    Expr::BinaryOp { op: BinaryOpKind::LtEq | BinaryOpKind::Lt, left, .. } => {
                        matches!(left, Expr::Identifier(sym) if *sym == counter_sym)
                    }
                    _ => false,
                };

                if loop_matches && body.len() >= 2 {
                    // Check last statement is counter++
                    let last_is_increment = match &body[body.len() - 1] {
                        Stmt::Set { target, value } if *target == counter_sym => {
                            matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                                if (matches!(left, Expr::Identifier(s) if *s == counter_sym) && matches!(right, Expr::Literal(Literal::Number(1))))
                                || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == counter_sym))
                            )
                        }
                        _ => false,
                    };

                    if last_is_increment {
                        let body_without_increment = &body[..body.len() - 1];
                        // Check for pushes/inserts. For top-level pushes, any push suffices.
                        // For nested pushes (inside If/Otherwise), require ALL paths to push
                        // so the count is deterministic (filter patterns excluded).
                        let has_push = match &collection_info {
                            CollInfo::Vec(_) => {
                                body_without_increment.iter().any(|s| {
                                    matches!(s, Stmt::Push { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == vec_sym))
                                }) || all_paths_push_to(body_without_increment, vec_sym)
                            }
                            CollInfo::Map(_, _) => {
                                body_without_increment.iter().any(|s| {
                                    matches!(s, Stmt::SetIndex { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == vec_sym))
                                }) || all_paths_set_index_to(body_without_increment, vec_sym)
                            }
                        };

                        if has_push {
                            counter_init_idx = Some(scan);
                            break;
                        }
                    }
                }
            }
        }

        // This statement doesn't start the pattern. If it references vec_sym, bail.
        if symbol_appears_in_stmts(vec_sym, &[stmt]) {
            return None;
        }
    }

    let counter_idx = counter_init_idx?;
    let while_idx = counter_idx + 1;

    let body = match stmts[while_idx] {
        Stmt::While { body, .. } => body,
        _ => return None,
    };

    // Delegate counter-init + While to for-range pattern
    let remaining = &stmts[counter_idx..];
    let remaining_refs: Vec<&Stmt> = remaining.iter().copied().collect();
    let loop_result = try_emit_for_range_pattern(
        &remaining_refs, 0, interner, indent, mutable_vars, ctx,
        lww_fields, mv_fields, synced_vars, var_caps, async_functions,
        pipe_vars, boxed_fields, registry, type_env,
    );

    let (loop_code, _) = loop_result?;

    // Compute capacity from the for-range bounds
    let start_str = codegen_expr_simple(match stmts[counter_idx] {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => value,
        _ => return None,
    }, interner);

    let limit_expr = match stmts[while_idx] {
        Stmt::While { cond, .. } => match cond {
            Expr::BinaryOp { right, .. } => *right,
            _ => return None,
        },
        _ => return None,
    };
    let is_exclusive = match stmts[while_idx] {
        Stmt::While { cond, .. } => matches!(cond, Expr::BinaryOp { op: BinaryOpKind::Lt, .. }),
        _ => false,
    };
    let limit_str = codegen_expr_simple(limit_expr, interner);

    let start_lit = match stmts[counter_idx] {
        Stmt::Let { value: Expr::Literal(Literal::Number(n)), .. }
        | Stmt::Set { value: Expr::Literal(Literal::Number(n)), .. } => Some(*n),
        _ => None,
    };
    let limit_lit = match limit_expr {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    };

    let capacity_expr = match (start_lit, limit_lit) {
        (Some(s), Some(l)) => {
            let count = if is_exclusive { l - s } else { l - s + 1 };
            format!("{}", std::cmp::max(0, count))
        }
        _ => {
            if is_exclusive {
                if start_str == "0" {
                    format!("{} as usize", limit_str)
                } else {
                    format!("({} - {}) as usize", limit_str, start_str)
                }
            } else {
                if start_str == "1" {
                    format!("{} as usize", limit_str)
                } else {
                    format!("({} - {} + 1) as usize", limit_str, start_str)
                }
            }
        }
    };

    let indent_str = "    ".repeat(indent);
    let vec_name = interner.resolve(vec_sym);

    // Detect vec![value; N] opportunity for Copy types with constant push values.
    let vec_fill_literal = if let CollInfo::Vec(ref elem_type) = collection_info {
        let is_copy = matches!(elem_type.as_str(), "i64" | "f64" | "bool");
        if is_copy {
            let body_without_increment = &body[..body.len() - 1];
            if body_without_increment.len() == 1 {
                match &body_without_increment[0] {
                    Stmt::Push { collection, value } => {
                        let is_target = matches!(collection, Expr::Identifier(sym) if *sym == vec_sym);
                        if is_target {
                            match value {
                                Expr::Literal(Literal::Number(n)) => Some(format!("{}", n)),
                                Expr::Literal(Literal::Float(f)) => Some(format!("{:.1}", f)),
                                Expr::Literal(Literal::Boolean(b)) => Some(format!("{}", b)),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut output = String::new();

    match &collection_info {
        CollInfo::Vec(elem_type) => {
            if let Some(fill_value) = &vec_fill_literal {
                writeln!(output, "{}let mut {}: Vec<{}> = vec![{}; {}];",
                    indent_str, vec_name, elem_type, fill_value, capacity_expr).unwrap();
            } else {
                writeln!(output, "{}let mut {}: Vec<{}> = Vec::with_capacity({});",
                    indent_str, vec_name, elem_type, capacity_expr).unwrap();
            }
            ctx.register_variable_type(vec_sym, format!("Vec<{}>", elem_type));
        }
        CollInfo::Map(key_type, val_type) => {
            writeln!(output, "{}let mut {}: FxHashMap<{}, {}> = FxHashMap::with_capacity_and_hasher({}, Default::default());",
                indent_str, vec_name, key_type, val_type, capacity_expr).unwrap();
            ctx.register_variable_type(vec_sym, format!("FxHashMap<{}, {}>", key_type, val_type));
        }
    }

    // Emit intervening statements between Seq creation and counter init.
    // Check each intervening Let for sibling Seq/Map creations that are also
    // pushed to in the same While loop — give them with_capacity too.
    let intervening = &stmts[(idx + 1)..counter_idx];
    let body_without_increment = &body[..body.len() - 1];
    for stmt in intervening {
        let sibling_cap = detect_sibling_collection(stmt, body_without_increment, interner);
        if let Some((sib_sym, sib_info)) = sibling_cap {
            let sib_name = interner.resolve(sib_sym);
            // Sibling collections always use with_capacity, never vec![val; N].
            // The loop still runs (for the primary collection), so vec![val; N]
            // would double the elements — the fill creates N items AND the loop pushes N more.
            match &sib_info {
                CollInfo::Vec(elem_type) => {
                    writeln!(output, "{}let mut {}: Vec<{}> = Vec::with_capacity({});",
                        indent_str, sib_name, elem_type, capacity_expr).unwrap();
                    ctx.register_variable_type(sib_sym, format!("Vec<{}>", elem_type));
                }
                CollInfo::Map(key_type, val_type) => {
                    writeln!(output, "{}let mut {}: FxHashMap<{}, {}> = FxHashMap::with_capacity_and_hasher({}, Default::default());",
                        indent_str, sib_name, key_type, val_type, capacity_expr).unwrap();
                    ctx.register_variable_type(sib_sym, format!("FxHashMap<{}, {}>", key_type, val_type));
                }
            }
        } else {
            use super::codegen_stmt;
            output.push_str(&codegen_stmt(stmt, interner, indent, mutable_vars, ctx,
                lww_fields, mv_fields, synced_vars, var_caps, async_functions,
                pipe_vars, boxed_fields, registry, type_env));
        }
    }

    // For vec![value; N], the fill is done by the declaration — skip the loop.
    // Only emit the post-loop counter binding if needed.
    if vec_fill_literal.is_some() {
        // Extract the post-loop counter assignment from the for-range code.
        // The for-range code looks like: `for counter in start..limit {\n  ...\n}\nlet mut counter = ...;\n`
        // We only need the trailing `let mut counter = ...;` part.
        if let Some(closing_pos) = loop_code.rfind("\n    }") {
            let after_loop = &loop_code[closing_pos + 6..]; // skip "}\n"
            let trimmed = after_loop.trim_start_matches('\n');
            if !trimmed.trim().is_empty() {
                output.push_str(trimmed);
            }
        }
    } else {
        output.push_str(&loop_code);
    }

    // Consumed: all statements from idx+1 through while_idx (inclusive)
    let extra_consumed = while_idx - idx;
    Some((output, extra_consumed))
}

/// Detect if an intervening statement is a `Let` creating a new Seq/Map that is
/// pushed to in the given loop body. Returns the symbol and collection info if so.
fn detect_sibling_collection<'a>(
    stmt: &Stmt<'a>,
    body_without_increment: &[Stmt<'a>],
    interner: &Interner,
) -> Option<(Symbol, CollInfo)> {
    let (var, value, ty) = match stmt {
        Stmt::Let { var, value, ty, .. } => (*var, *value, ty.as_ref()),
        _ => return None,
    };

    // Extract collection type info from annotation or `new` expression
    let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
        let base_name = interner.resolve(*base);
        if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
            Some(CollInfo::Vec(codegen_type_expr(&params[0], interner)))
        } else if matches!(base_name, "Map" | "HashMap") && params.len() >= 2 {
            Some(CollInfo::Map(
                codegen_type_expr(&params[0], interner),
                codegen_type_expr(&params[1], interner),
            ))
        } else {
            None
        }
    } else {
        None
    };

    let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
        let tn = interner.resolve(*type_name);
        if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
            Some(CollInfo::Vec(codegen_type_expr(&type_args[0], interner)))
        } else if matches!(tn, "Map" | "HashMap") && init_fields.is_empty() && type_args.len() >= 2 {
            Some(CollInfo::Map(
                codegen_type_expr(&type_args[0], interner),
                codegen_type_expr(&type_args[1], interner),
            ))
        } else {
            None
        }
    } else {
        None
    };

    let info = type_from_annotation.or(type_from_new)?;

    // Check if this variable is pushed to in the loop body
    let has_push = match &info {
        CollInfo::Vec(_) => {
            body_without_increment.iter().any(|s| {
                matches!(s, Stmt::Push { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == var))
            }) || all_paths_push_to(body_without_increment, var)
        }
        CollInfo::Map(_, _) => {
            body_without_increment.iter().any(|s| {
                matches!(s, Stmt::SetIndex { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == var))
            }) || all_paths_set_index_to(body_without_increment, var)
        }
    };

    if has_push { Some((var, info)) } else { None }
}

/// Peephole optimization for merge-style Vec construction.
///
/// Detects a `Let mutable X = new Seq of T` followed by one or more While loops
/// that all push to X from known source Vecs, and emits
/// `Vec::with_capacity((src1.len() + src2.len()) as usize)` instead of `Vec::default()`.
///
/// This pattern is distinct from `try_emit_vec_with_capacity_pattern` which computes
/// capacity from loop iteration count. This pattern computes capacity from the total
/// length of all source collections being merged.
///
/// Pattern:
///   Let mutable result = new Seq of T.
///   ... (counter inits, etc.)
///   While ...: Push item X of SOURCE to result. ...
///   While ...: Push item Y of SOURCE2 to result. ...
///   Return result.
///
/// → `let mut result: Vec<T> = Vec::with_capacity((src1.len() + src2.len()) as usize);`
///
/// Only fires when:
/// - Every While loop between the decl and the first non-While/non-counter-init reference
///   to the target has ALL execution paths pushing to the target
/// - All pushed values come from indexed reads on known source Vecs
/// - The target Vec is not used in any other way between declaration and the While loops
pub(crate) fn try_emit_merge_capacity_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if idx + 1 >= stmts.len() {
        return None;
    }

    // Statement at idx: Let mutable vec_var = new Seq of T
    let (vec_sym, elem_type) = match stmts[idx] {
        Stmt::Let { var, value, ty, mutable: true, .. } => {
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(codegen_type_expr(&params[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
                    Some(codegen_type_expr(&type_args[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(elem) => (*var, elem),
                None => return None,
            }
        }
        _ => return None,
    };

    // Scan forward: collect While loops that push to vec_sym.
    // Allow intervening Let/Set statements that don't reference vec_sym (counter inits).
    // Stop at Return or any non-Let/Set/While that references vec_sym.
    let mut source_syms: HashSet<Symbol> = HashSet::new();
    let mut last_while_idx = idx;
    let mut found_any_while = false;

    for scan in (idx + 1)..stmts.len() {
        let stmt = stmts[scan];

        match stmt {
            Stmt::While { body, .. } => {
                // Check that all paths push to vec_sym
                if all_paths_push_to(body, vec_sym) {
                    // Collect push sources from this While body
                    if let Some(sources) = collect_push_sources(body, vec_sym) {
                        source_syms.extend(sources);
                        last_while_idx = scan;
                        found_any_while = true;
                    } else {
                        // While pushes to target but sources are not simple indexed reads
                        break;
                    }
                } else {
                    // This While doesn't push to our target on all paths — stop scanning
                    break;
                }
            }
            Stmt::Let { .. } | Stmt::Set { .. } => {
                // Allow intervening counter init / other Let/Set if they don't reference vec_sym
                if symbol_appears_in_stmts(vec_sym, &[stmt]) {
                    break;
                }
            }
            Stmt::Return { .. } => {
                // Return ends the block — stop scanning
                break;
            }
            _ => {
                // Any other statement type — stop scanning
                break;
            }
        }
    }

    if !found_any_while || source_syms.is_empty() {
        return None;
    }

    // Build capacity expression: sum of all source .len() values
    let names = RustNames::new(interner);
    let vec_name = names.ident(vec_sym);
    let indent_str = "    ".repeat(indent);

    let capacity_parts: Vec<String> = source_syms.iter().map(|sym| {
        format!("{}.len()", names.ident(*sym))
    }).collect();
    let capacity_expr = capacity_parts.join(" + ");

    let mut output = String::new();
    writeln!(output, "{}let mut {}: Vec<{}> = Vec::with_capacity(({}) as usize);",
        indent_str, vec_name, elem_type, capacity_expr).unwrap();
    ctx.register_variable_type(vec_sym, format!("Vec<{}>", elem_type));

    // Emit intervening statements between the declaration and the first While,
    // then the While loops themselves (all handled by regular codegen).
    // We only consumed statement idx (the Let). The rest are emitted by the caller.
    // Return 0 extra consumed since we only replaced the Let declaration.
    Some((output, 0))
}

/// Collect all source collection symbols from Push statements targeting `coll_sym` in a statement block.
/// Returns None if any push to the target doesn't read from a simple indexed collection.
/// Recurses into If/Else branches.
fn collect_push_sources(stmts: &[Stmt], coll_sym: Symbol) -> Option<HashSet<Symbol>> {
    let mut sources = HashSet::new();
    for stmt in stmts {
        match stmt {
            Stmt::Push { value, collection } => {
                if matches!(collection, Expr::Identifier(sym) if *sym == coll_sym) {
                    collect_index_sources_from_expr(value, &mut sources);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if let Some(then_sources) = collect_push_sources(then_block, coll_sym) {
                    sources.extend(then_sources);
                }
                if let Some(else_stmts) = else_block {
                    if let Some(else_sources) = collect_push_sources(else_stmts, coll_sym) {
                        sources.extend(else_sources);
                    }
                }
            }
            Stmt::While { body, .. } => {
                if let Some(body_sources) = collect_push_sources(body, coll_sym) {
                    sources.extend(body_sources);
                }
            }
            _ => {}
        }
    }
    if sources.is_empty() {
        None
    } else {
        Some(sources)
    }
}

/// Extract source collection identifiers from Index expressions.
/// `Push item i of left to result` → extracts `left`.
fn collect_index_sources_from_expr(expr: &Expr, sources: &mut HashSet<Symbol>) {
    match expr {
        Expr::Index { collection, .. } => {
            if let Expr::Identifier(sym) = collection {
                sources.insert(*sym);
            }
        }
        Expr::Identifier(sym) => {
            sources.insert(*sym);
        }
        _ => {}
    }
}

/// Pre-scan a statement block to identify variables that can use `char` instead of `String`.
///
/// Peephole optimization: detect left-rotation via shift loop and emit `.rotate_left(1)`.
///
/// Pattern (4 statements):
///   Let tmp be item 1 of arr.
///   Set counter to 1.
///   While counter <= limit:
///       Set item counter of arr to item (counter + 1) of arr.
///       Set counter to counter + 1.
///   Set item (limit + 1) of arr to tmp.
/// → `arr[0..=(limit as usize)].rotate_left(1);`
///
/// `arr` must be registered as a `Vec<T>` in variable_types (requires mutable slice).
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_rotate_left_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    if idx + 3 >= stmts.len() {
        return None;
    }

    // Statement 1: Let tmp be item 1 of arr.  (saves first element)
    let (tmp_sym, arr_sym) = match stmts[idx] {
        Stmt::Let { var, mutable: false, value, .. } => {
            if let Expr::Index { collection, index } = value {
                if let Expr::Identifier(a) = collection {
                    if matches!(index, Expr::Literal(Literal::Number(1))) {
                        (*var, *a)
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // arr must be a Vec type (rotate_left requires &mut [T])
    match variable_types.get(&arr_sym) {
        Some(t) if t.starts_with("Vec") => {}
        _ => return None,
    }

    // Statement 2: Set counter to 1.
    let counter_sym = match stmts[idx + 1] {
        Stmt::Set { target, value: Expr::Literal(Literal::Number(1)) } => *target,
        _ => return None,
    };

    // Statement 3: While counter <= limit: SetIndex arr[counter] = arr[counter+1]; Set counter++
    let limit_expr = match stmts[idx + 2] {
        Stmt::While { cond, body, .. } => {
            // Condition: counter <= limit
            let limit = match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym { Some(*right) } else { None }
                    } else {
                        None
                    }
                }
                _ => None,
            }?;

            if body.len() != 2 {
                return None;
            }

            // Body[0]: Set item counter of arr to item (counter + 1) of arr.
            match &body[0] {
                Stmt::SetIndex { collection, index: idx_expr, value } => {
                    if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                        return None;
                    }
                    if !matches!(idx_expr, Expr::Identifier(s) if *s == counter_sym) {
                        return None;
                    }
                    if let Expr::Index { collection: v_coll, index: v_idx } = value {
                        if !matches!(v_coll, Expr::Identifier(s) if *s == arr_sym) {
                            return None;
                        }
                        match v_idx {
                            Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                                let ok = (matches!(left, Expr::Identifier(s) if *s == counter_sym)
                                    && matches!(right, Expr::Literal(Literal::Number(1))))
                                    || (matches!(left, Expr::Literal(Literal::Number(1)))
                                    && matches!(right, Expr::Identifier(s) if *s == counter_sym));
                                if !ok {
                                    return None;
                                }
                            }
                            _ => return None,
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }

            // Body[1]: Set counter to counter + 1.
            match &body[1] {
                Stmt::Set { target, value } => {
                    if *target != counter_sym {
                        return None;
                    }
                    match value {
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            let ok = (matches!(left, Expr::Identifier(s) if *s == counter_sym)
                                && matches!(right, Expr::Literal(Literal::Number(1))))
                                || (matches!(left, Expr::Literal(Literal::Number(1)))
                                && matches!(right, Expr::Identifier(s) if *s == counter_sym));
                            if !ok {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }

            limit
        }
        _ => return None,
    };

    if !is_simple_expr(limit_expr) {
        return None;
    }

    // Statement 4: Set item (limit + 1) of arr to tmp.  (wrap-around)
    match stmts[idx + 3] {
        Stmt::SetIndex { collection, index, value } => {
            if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                return None;
            }
            if !matches!(value, Expr::Identifier(s) if *s == tmp_sym) {
                return None;
            }
            match index {
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                    let ok = (exprs_equal(left, limit_expr)
                        && matches!(right, Expr::Literal(Literal::Number(1))))
                        || (matches!(left, Expr::Literal(Literal::Number(1)))
                        && exprs_equal(right, limit_expr));
                    if !ok {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        _ => return None,
    }

    // Pattern matched! Emit rotate_left.
    let indent_str = "    ".repeat(indent);
    let arr_name = interner.resolve(arr_sym);
    let tmp_name = interner.resolve(tmp_sym);
    let limit_str = codegen_expr_simple(limit_expr, interner);

    let mut output = String::new();

    // Only emit the tmp binding if it's used after the rotation.
    let remaining = &stmts[idx + 4..];
    if symbol_appears_in_stmts(tmp_sym, remaining) {
        writeln!(output, "{}let {} = {}[0];", indent_str, tmp_name, arr_name).unwrap();
    }
    writeln!(output, "{}{}[0..=({} as usize)].rotate_left(1);",
        indent_str, arr_name, limit_str).unwrap();

    Some((output, 3)) // consumed: Set counter + While + SetIndex = 3 extra
}

/// Pattern B: Unconditional 3-statement swap with arbitrary indices.
///   Let tmp be item I of arr.
///   Set item I of arr to item J of arr.
///   Set item J of arr to tmp.
/// → `arr.swap((I-1) as usize, (J-1) as usize);`
fn try_emit_unconditional_swap<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    tmp_sym: Symbol,
    arr_sym: Symbol,
    idx_expr_1: &Expr,
    interner: &Interner,
    indent: usize,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 2: Set item I of arr to item J of arr.
    let idx_expr_2 = match stmts[idx + 1] {
        Stmt::SetIndex { collection, index, value } => {
            // collection must be the same array
            if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                return None;
            }
            // index must match idx_expr_1
            if !exprs_equal(index, idx_expr_1) {
                return None;
            }
            // value must be an Index into the same array at a different index
            if let Expr::Index { collection: v_coll, index: v_idx } = value {
                if !matches!(v_coll, Expr::Identifier(s) if *s == arr_sym) {
                    return None;
                }
                *v_idx
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 3: Set item J of arr to tmp.
    match stmts[idx + 2] {
        Stmt::SetIndex { collection, index, value } => {
            // collection must be the same array
            if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                return None;
            }
            // index must match idx_expr_2
            if !exprs_equal(index, idx_expr_2) {
                return None;
            }
            // value must be tmp
            if !matches!(value, Expr::Identifier(s) if *s == tmp_sym) {
                return None;
            }
        }
        _ => return None,
    }

    // Both index expressions must be simple enough for codegen_expr_simple
    if !is_simple_expr(idx_expr_1) || !is_simple_expr(idx_expr_2) {
        return None;
    }

    // Pattern matched! Emit manual swap with direct indexing
    let indent_str = "    ".repeat(indent);
    let arr_name = interner.resolve(arr_sym);
    let idx1_simplified = simplify_1based_index(idx_expr_1, interner, true);
    let idx2_simplified = simplify_1based_index(idx_expr_2, interner, true);

    let mut output = String::new();
    writeln!(output, "{}let __swap_tmp = {}[{}];",
        indent_str, arr_name, idx1_simplified).unwrap();
    writeln!(output, "{}{}[{}] = {}[{}];",
        indent_str, arr_name, idx1_simplified, arr_name, idx2_simplified).unwrap();
    writeln!(output, "{}{}[{}] = __swap_tmp;",
        indent_str, arr_name, idx2_simplified).unwrap();

    Some((output, 2)) // consumed 2 extra statements (SetIndex + SetIndex)
}

/// Peephole optimization: detect a contiguous push-copy loop from one array to another
/// and emit `dst.extend_from_slice(...)` instead of individual pushes.
///
/// Matches two forms:
/// 1. **Counter init + While**: `Let/Set counter = start. While counter <= end: Push item counter of src to dst. Set counter to counter + 1.`
/// 2. **Bare While**: `While counter <= end: Push item counter of src to dst. Set counter to counter + 1.`
///    (counter already exists from a previous pattern's post-loop value emission)
///
/// Unlike `try_emit_seq_from_slice_pattern`, this does NOT require a preceding `Let mutable dst = new Seq`
/// anchor. This handles the second-half copy in divide-and-conquer splits where the first half
/// was already consumed by `try_emit_seq_from_slice_pattern`.
///
/// Must be positioned before `try_emit_for_range_pattern` in the peephole chain since both
/// match `Let counter + While` — this pattern is more specific (body must be push + increment).
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_bare_slice_push_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    // Try form 1: counter init + While (2-statement pattern)
    if let Some(result) = try_bare_slice_push_with_init(stmts, idx, interner, indent, variable_types) {
        return Some(result);
    }
    // Try form 2: bare While (1-statement pattern, counter already exists)
    try_bare_slice_push_bare_while(stmts, idx, interner, indent, variable_types)
}

fn try_bare_slice_push_with_init<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    if idx + 1 >= stmts.len() {
        return None;
    }

    // Statement 1: Let counter = start OR Set counter to start
    let (counter_sym, start_expr, is_new_binding) = match stmts[idx] {
        Stmt::Let { var, value, .. } => {
            if is_simple_expr(value) {
                (*var, *value, true)
            } else {
                return None;
            }
        }
        Stmt::Set { target, value } => {
            if is_simple_expr(value) {
                (*target, *value, false)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 2: While counter <= end OR While counter < end
    let while_info = extract_push_copy_while(stmts[idx + 1], counter_sym)?;

    // Validate types
    validate_slice_push_types(while_info.src_sym, while_info.dst_sym, variable_types)?;

    // Emit the optimized code
    let remaining = &stmts[idx + 2..];
    let output = emit_extend_from_slice(
        interner, indent, while_info.dst_sym, while_info.src_sym, counter_sym,
        start_expr, while_info.end_expr, while_info.is_exclusive,
        remaining, Some(is_new_binding),
    );

    Some((output, 1)) // consumed 1 extra statement (the While)
}

fn try_bare_slice_push_bare_while<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    // Statement at idx must be a While loop
    let while_stmt = stmts[idx];
    let (counter_sym, end_expr, is_exclusive, src_sym, dst_sym) = match while_stmt {
        Stmt::While { cond, body, .. } => {
            let (counter_sym, end_expr, is_exclusive) = extract_while_cond(cond)?;
            if !is_simple_expr(end_expr) { return None; }
            if body.len() != 2 { return None; }
            let (src, dst) = extract_push_index_body(body, counter_sym)?;
            (counter_sym, end_expr, is_exclusive, src, dst)
        }
        _ => return None,
    };

    // Validate types
    validate_slice_push_types(src_sym, dst_sym, variable_types)?;

    // For bare While, the counter already has a value from a previous statement.
    // We use the counter identifier as the start expression.
    let start_expr = Expr::Identifier(counter_sym);

    let remaining = &stmts[idx + 1..];
    let output = emit_extend_from_slice(
        interner, indent, dst_sym, src_sym, counter_sym,
        &start_expr, end_expr, is_exclusive,
        remaining, None, // no binding info — counter already exists
    );

    Some((output, 0)) // no extra statements consumed
}

struct WhileInfo<'a> {
    end_expr: &'a Expr<'a>,
    is_exclusive: bool,
    src_sym: Symbol,
    dst_sym: Symbol,
}

fn extract_push_copy_while<'a>(
    stmt: &'a Stmt<'a>,
    counter_sym: Symbol,
) -> Option<WhileInfo<'a>> {
    let (cond, body) = match stmt {
        Stmt::While { cond, body, .. } => (cond, body),
        _ => return None,
    };

    let (cond_counter, end_expr, is_exclusive) = extract_while_cond(cond)?;
    if cond_counter != counter_sym { return None; }
    if !is_simple_expr(end_expr) { return None; }
    if body.len() != 2 { return None; }

    let (src_sym, dst_sym) = extract_push_index_body(body, counter_sym)?;
    validate_increment(&body[1], counter_sym)?;

    Some(WhileInfo { end_expr, is_exclusive, src_sym, dst_sym })
}

fn extract_while_cond<'a>(cond: &'a Expr<'a>) -> Option<(Symbol, &'a Expr<'a>, bool)> {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                Some((*sym, *right, false))
            } else {
                None
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
            if let Expr::Identifier(sym) = left {
                Some((*sym, *right, true))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn extract_push_index_body<'a>(body: &[Stmt<'a>], counter_sym: Symbol) -> Option<(Symbol, Symbol)> {
    // body[0]: Push item <counter> of <source> to <target>
    let (src, dst) = match &body[0] {
        Stmt::Push { value, collection } => {
            let dst = if let Expr::Identifier(s) = collection { *s } else { return None; };
            if let Expr::Index { collection: src_coll, index } = value {
                if !matches!(index, Expr::Identifier(s) if *s == counter_sym) {
                    return None;
                }
                if let Expr::Identifier(s) = src_coll { (*s, dst) } else { return None; }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // body[1]: Set counter to counter + 1
    validate_increment(&body[1], counter_sym)?;

    Some((src, dst))
}

fn validate_increment(stmt: &Stmt, counter_sym: Symbol) -> Option<()> {
    match stmt {
        Stmt::Set { target, value } if *target == counter_sym => {
            match value {
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                    let ok = matches!((left, right),
                        (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym
                    ) || matches!((left, right),
                        (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym
                    );
                    if ok { Some(()) } else { None }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn validate_slice_push_types(
    src_sym: Symbol,
    dst_sym: Symbol,
    variable_types: &HashMap<Symbol, String>,
) -> Option<()> {
    if src_sym == dst_sym { return None; }

    let dst_type = variable_types.get(&dst_sym)?;
    if !dst_type.starts_with("Vec<") { return None; }

    let src_type = variable_types.get(&src_sym)?;
    if !src_type.starts_with("Vec<") && !src_type.starts_with("&[") && !src_type.starts_with("&mut [") {
        return None;
    }

    Some(())
}

fn emit_extend_from_slice(
    interner: &Interner,
    indent: usize,
    dst_sym: Symbol,
    src_sym: Symbol,
    counter_sym: Symbol,
    start_expr: &Expr,
    end_expr: &Expr,
    is_exclusive: bool,
    remaining: &[&Stmt],
    binding_info: Option<bool>, // Some(true) = new let, Some(false) = reassignment, None = already exists
) -> String {
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let dst_name = names.ident(dst_sym);
    let src_name = names.ident(src_sym);
    let counter_name = names.ident(counter_sym);
    let start_str = codegen_expr_simple(start_expr, interner);
    let end_str = codegen_expr_simple(end_expr, interner);

    let mut output = String::new();

    if is_exclusive {
        writeln!(output, "{}if {} < {} {{", indent_str, start_str, end_str).unwrap();
        writeln!(output, "{}    {}.extend_from_slice(&{}[({} - 1) as usize..({} - 1) as usize]);",
            indent_str, dst_name, src_name, start_str, end_str).unwrap();
        writeln!(output, "{}}}", indent_str).unwrap();
    } else {
        writeln!(output, "{}if {} <= {} {{", indent_str, start_str, end_str).unwrap();
        writeln!(output, "{}    {}.extend_from_slice(&{}[({} - 1) as usize..{} as usize]);",
            indent_str, dst_name, src_name, start_str, end_str).unwrap();
        writeln!(output, "{}}}", indent_str).unwrap();
    }

    if symbol_appears_in_stmts(counter_sym, remaining) {
        let post_val = if is_exclusive {
            end_str.to_string()
        } else {
            if let Expr::Literal(Literal::Number(n)) = end_expr {
                format!("{}", n + 1)
            } else {
                format!("{} + 1", end_str)
            }
        };
        match binding_info {
            Some(true) => {
                writeln!(output, "{}let mut {} = {};", indent_str, counter_name, post_val).unwrap();
            }
            Some(false) | None => {
                writeln!(output, "{}{} = {};", indent_str, counter_name, post_val).unwrap();
            }
        }
    }

    output
}

/// Detect drain-tail pattern in a while loop body.
///
/// When a while loop body starts with `If cond: Push item counter of array to target;
/// Set counter to counter + 1. Otherwise: ...`, and the If condition is loop-invariant
/// within the then-branch (no variable in the condition is modified by the then-branch),
/// we know that once the condition becomes true, ALL remaining iterations will take the
/// then-branch. This is equivalent to `target.extend_from_slice(&array[counter-1..])` + `break`.
///
/// Returns the optimized If code if the pattern matches, or None.
pub(crate) fn try_emit_drain_tail_in_while<'a>(
    stmt: &Stmt<'a>,
    while_cond: &Expr<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) -> Option<String> {
    // The while condition must be `counter <= bound` (LtEq only — Lt doesn't apply
    // since drain means "until exhausted").
    let counter_sym = match while_cond {
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, .. } => {
            if let Expr::Identifier(sym) = left {
                *sym
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // The statement must be an If/Otherwise.
    let (if_cond, then_block, else_block) = match stmt {
        Stmt::If { cond, then_block, else_block: Some(else_block) } => {
            (*cond, then_block, else_block)
        }
        _ => return None,
    };

    // The then-block must have exactly 2 statements.
    if then_block.len() != 2 {
        return None;
    }

    // Statement 1: Push item <counter> of <array> to <target>
    let (target_sym, array_sym, push_counter_sym) = match &then_block[0] {
        Stmt::Push { value, collection } => {
            let tgt = if let Expr::Identifier(s) = collection { *s } else { return None; };
            if let Expr::Index { collection: arr_expr, index: idx_expr } = value {
                let arr_sym = if let Expr::Identifier(s) = arr_expr { *s } else { return None; };
                let idx_sym = if let Expr::Identifier(s) = idx_expr { *s } else { return None; };
                (tgt, arr_sym, idx_sym)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 2: Set counter to counter + 1
    validate_increment(&then_block[1], push_counter_sym)?;

    // The push counter must match the while loop counter.
    if push_counter_sym != counter_sym {
        return None;
    }

    // Validate types: both array and target must be Vec<T>.
    validate_slice_push_types(array_sym, target_sym, ctx.get_variable_types())?;

    // The If condition must be loop-invariant within the then-branch:
    // no variable in the condition is modified by the then-branch.
    let mut cond_syms = Vec::new();
    collect_expr_symbols(if_cond, &mut cond_syms);
    for sym in &cond_syms {
        if body_modifies_var(then_block, *sym) || body_mutates_collection(then_block, *sym) {
            return None;
        }
    }

    // Pattern matched! Emit optimized code.
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let target_name = names.ident(target_sym);
    let array_name = names.ident(array_sym);
    let counter_name = names.ident(push_counter_sym);
    use super::expr::codegen_expr_with_async;
    let cond_str = codegen_expr_with_async(if_cond, interner, synced_vars, async_functions, ctx.get_variable_types());

    let mut output = String::new();

    // Emit the If with extend_from_slice + break for the then-branch.
    writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
    writeln!(output, "{}    {}.extend_from_slice(&{}[({} - 1) as usize..]);",
        indent_str, target_name, array_name, counter_name).unwrap();
    writeln!(output, "{}    break;", indent_str).unwrap();
    writeln!(output, "{}}} else {{", indent_str).unwrap();

    // Emit the else block normally.
    for else_stmt in else_block.iter() {
        output.push_str(&super::codegen_stmt(else_stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
    }

    writeln!(output, "{}}}", indent_str).unwrap();

    Some(output)
}

/// Detect double-buffer swap pattern in a for-range body.
///
/// Pattern: Last statement is `Set X to Y` where both are same-type `Vec<T>`,
/// Y is not declared inside the body, and the body has a sub-loop whose first
/// unconditional action on Y is a SetIndex (write-before-read at each position).
/// This makes `std::mem::swap(&mut X, &mut Y)` semantically equivalent to
/// `X = Y.clone()` — both variables remain valid, Y is fully overwritten
/// before being read in the next iteration.
pub(crate) fn detect_double_buffer_swap<'a>(
    body: &[Stmt<'a>],
    interner: &Interner,
    ctx: &RefinementContext<'a>,
) -> Option<(Symbol, Symbol, usize)> {
    if body.len() < 2 {
        return None;
    }

    // Last statement must be `Set X to Y` where Y is an Identifier.
    let (x_sym, y_sym, set_idx) = {
        let last_idx = body.len() - 1;
        match &body[last_idx] {
            Stmt::Set { target, value } => {
                if let Expr::Identifier(src) = value {
                    if *src != *target {
                        (*target, *src, last_idx)
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    };

    // Both must be same-type Vec<T>.
    let x_type = ctx.get_variable_types().get(&x_sym)?;
    let y_type = ctx.get_variable_types().get(&y_sym)?;
    if !x_type.starts_with("Vec<") || x_type != y_type {
        return None;
    }

    // Y must NOT be declared inside the body (it's a pre-existing variable).
    for stmt in body.iter() {
        if let Stmt::Let { var, .. } = stmt {
            if *var == y_sym {
                return None;
            }
        }
    }

    // Y must not be resized via Push/Pop in the body (only SetIndex is allowed).
    if body_resizes_collection(&body[..set_idx], y_sym) {
        return None;
    }

    // The body must contain a sub-loop (While) whose first body statement is
    // an unconditional SetIndex on Y where the value does NOT reference Y.
    // This ensures Y is written-before-read at each index position.
    let mut has_overwrite_loop = false;
    for stmt in body[..set_idx].iter() {
        if let Stmt::While { body: inner_body, .. } = stmt {
            if !inner_body.is_empty() {
                if let Stmt::SetIndex { collection, .. } = &inner_body[0] {
                    if let Expr::Identifier(coll_sym) = collection {
                        if *coll_sym == y_sym {
                            // Check the value expression doesn't read from Y
                            if let Stmt::SetIndex { value, .. } = &inner_body[0] {
                                let mut syms = Vec::new();
                                collect_expr_symbols(value, &mut syms);
                                if !syms.contains(&y_sym) {
                                    has_overwrite_loop = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if !has_overwrite_loop {
        return None;
    }

    Some((x_sym, y_sym, set_idx))
}

/// Result of detecting a buffer-reuse pattern in a loop body.
pub(crate) struct BufferReuseInfo {
    pub inner_sym: Symbol,
    pub outer_sym: Symbol,
    pub inner_elem_type: String,
    pub inner_let_idx: usize,
    pub set_idx: usize,
}

/// Detect buffer reuse pattern in a loop body: a mutable inner buffer is created
/// fresh each iteration, filled, then transferred to an outer variable via `Set`.
/// Returns the detected symbols and indices needed to apply the optimization:
/// hoist the buffer before the loop, emit `.clear()` instead of allocation,
/// and `std::mem::swap()` instead of ownership transfer.
pub(crate) fn detect_buffer_reuse_in_body<'a>(
    body: &[Stmt<'a>],
    interner: &Interner,
    ctx: &RefinementContext<'a>,
) -> Option<BufferReuseInfo> {
    if body.len() < 2 {
        return None;
    }

    // Find Let mutable INNER = new Seq of T in the first two body statements.
    let (inner_sym, inner_elem_type, inner_let_idx) = {
        let mut found = None;
        for (bi, stmt) in body.iter().enumerate() {
            if bi > 1 { break; }
            if let Stmt::Let { var, value, ty, mutable: true } = stmt {
                let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                    let base_name = interner.resolve(*base);
                    if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                        Some(codegen_type_expr(&params[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                    let tn = interner.resolve(*type_name);
                    if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
                        Some(codegen_type_expr(&type_args[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(t) = type_from_annotation.or(type_from_new) {
                    found = Some((*var, t, bi));
                    break;
                }
            }
        }
        found?
    };

    // Find Set OUTER to Expr::Identifier(INNER) in the body, scanning from the end.
    let (outer_sym, set_idx) = {
        let mut found = None;
        for (bi, stmt) in body.iter().enumerate().rev() {
            if let Stmt::Set { target, value } = stmt {
                if let Expr::Identifier(src) = value {
                    if *src == inner_sym && *target != inner_sym {
                        found = Some((*target, bi));
                        break;
                    }
                }
            }
        }
        found?
    };

    // The Set must be the last or second-to-last body statement.
    // If second-to-last, the last must be a counter increment.
    if set_idx == body.len().wrapping_sub(2) && body.len() >= 2 {
        match &body[body.len() - 1] {
            Stmt::Set { target, value } => {
                let is_increment = matches!(value,
                    Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                    if (matches!(left, Expr::Identifier(s) if *s == *target) && matches!(right, Expr::Literal(Literal::Number(1))))
                    || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == *target))
                );
                if !is_increment {
                    return None;
                }
            }
            _ => return None,
        }
    } else if set_idx != body.len() - 1 {
        return None;
    }

    // Verify OUTER is a Vec of the same element type as INNER.
    let outer_type = ctx.get_variable_types().get(&outer_sym)?;
    let expected_prefix = format!("Vec<{}>", inner_elem_type);
    if !outer_type.starts_with(&expected_prefix) {
        return None;
    }

    // Verify INNER is not referenced after the Set.
    for bi in (set_idx + 1)..body.len() {
        let stmt_ref: &Stmt = &body[bi];
        if symbol_appears_in_stmts(inner_sym, &[stmt_ref]) {
            return None;
        }
    }

    Some(BufferReuseInfo {
        inner_sym,
        outer_sym,
        inner_elem_type,
        inner_let_idx,
        set_idx,
    })
}

/// Peephole optimization: detect a While whose body creates a fresh Seq each iteration,
/// fills it via an inner loop, and transfers it to an outer variable. Replace with:
/// - Hoisted inner buffer declaration (before the while)
/// - `.clear()` instead of `new Seq` each iteration
/// - `std::mem::swap(&mut outer, &mut inner)` instead of ownership transfer
///
/// This eliminates N allocations + drops for N iterations of the outer loop.
///
/// AST shape:
/// ```text
/// While COND:
///     Let mutable INNER = new Seq of T.   ← replaced with INNER.clear()
///     ... (fill INNER via inner loop) ...
///     Set OUTER to INNER.                 ← replaced with mem::swap
///     [Set counter to counter + 1.]       ← optional counter increment
/// ```
///
/// Conditions:
/// - OUTER must be a Vec<T> with the same element type as INNER
/// - Set OUTER to INNER must be the last meaningful statement (before optional counter increment)
/// - INNER must not appear after the Set in the body
pub(crate) fn try_emit_buffer_reuse_while<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) -> Option<(String, usize)> {
    let (cond, body) = match stmts[idx] {
        Stmt::While { cond, body, .. } => (cond, body),
        _ => return None,
    };

    if body.len() < 3 {
        return None;
    }

    let reuse = detect_buffer_reuse_in_body(body, interner, ctx)?;

    // Pattern matched! Generate the transformed While.
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let inner_name = names.ident(reuse.inner_sym);
    let outer_name = names.ident(reuse.outer_sym);

    let mut output = String::new();

    // Hoist inner buffer declaration before the while loop.
    writeln!(output, "{}let mut {}: Vec<{}> = Vec::new();", indent_str, inner_name, reuse.inner_elem_type).unwrap();

    // Loop bounds hoisting (replicate from stmt.rs While handler).
    use super::stmt::{extract_length_expr_syms, collect_length_syms_from_stmts};
    let mut all_length_syms_raw = extract_length_expr_syms(cond);
    collect_length_syms_from_stmts(body, &mut all_length_syms_raw);
    let mut seen = HashSet::new();
    let all_length_syms: Vec<Symbol> = all_length_syms_raw
        .into_iter()
        .filter(|s| seen.insert(*s))
        .collect();

    let mut hoisted_syms: Vec<(Symbol, Option<String>)> = Vec::new();
    for len_sym in &all_length_syms {
        if !body_mutates_collection(body, *len_sym) && !body_modifies_var(body, *len_sym) {
            let name = interner.resolve(*len_sym);
            let hoisted_name = format!("{}_len", name);
            writeln!(output, "{}let {} = ({}.len() as i64);", indent_str, hoisted_name, name).unwrap();
            let old_type = ctx.get_variable_types().get(len_sym).cloned();
            let new_type = match &old_type {
                Some(existing) => format!("{}|__hl:{}", existing, hoisted_name),
                None => format!("|__hl:{}", hoisted_name),
            };
            ctx.register_variable_type(*len_sym, new_type);
            hoisted_syms.push((*len_sym, old_type));
        }
    }

    // Emit the while condition.
    use super::expr::codegen_expr_with_async;
    let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
    writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
    ctx.push_scope();

    // Process body statements with peephole chain, intercepting transformed statements.
    let body_refs: Vec<&Stmt> = body.iter().collect();
    let mut bi = 0;
    while bi < body_refs.len() {
        if bi == reuse.inner_let_idx {
            // Replace Let inner = new Seq with .clear()
            writeln!(output, "{}    {}.clear();", indent_str, inner_name).unwrap();
            ctx.register_variable_type(reuse.inner_sym, format!("Vec<{}>", reuse.inner_elem_type));
            bi += 1;
            continue;
        }
        if bi == reuse.set_idx {
            // Replace Set outer to inner with mem::swap
            writeln!(output, "{}    std::mem::swap(&mut {}, &mut {});", indent_str, outer_name, inner_name).unwrap();
            bi += 1;
            continue;
        }

        // Standard peephole chain for body statements.
        if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_vec_fill_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_vec_with_capacity_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_merge_capacity_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_for_range_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_seq_copy_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_rotate_left_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }

        // Fallback: normal codegen.
        use super::codegen_stmt;
        output.push_str(&codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx,
            lww_fields, mv_fields, synced_vars, var_caps, async_functions,
            pipe_vars, boxed_fields, registry, type_env));
        bi += 1;
    }

    ctx.pop_scope();

    // Restore hoisted length symbols.
    for (sym, old_type) in hoisted_syms {
        if let Some(old) = old_type {
            ctx.register_variable_type(sym, old);
        } else {
            ctx.get_variable_types_mut().remove(&sym);
        }
    }

    writeln!(output, "{}}}", indent_str).unwrap();

    Some((output, 0))
}
