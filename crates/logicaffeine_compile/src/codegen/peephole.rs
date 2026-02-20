use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use super::context::{RefinementContext, VariableCapabilities};
use super::detection::symbol_appears_in_stmts;
use super::types::codegen_type_expr;

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

    // Statement 1: Let counter = start_literal (integer)
    // Note: mutable flag may be false in AST even when counter is mutated via Set.
    // The counter's mutability is proven by the while body's increment statement.
    let (counter_sym, counter_start) = match stmts[idx] {
        Stmt::Let { var, value: Expr::Literal(Literal::Number(n)), .. } => {
            (*var, *n)
        }
        _ => return None,
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

    // Pattern matched! Emit for-range loop.
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let counter_name = names.ident(counter_sym);
    let limit_str = codegen_expr_simple(limit_expr, interner);

    // Always use exclusive ranges (Range) instead of inclusive (RangeInclusive).
    // RangeInclusive has a known performance overhead in Rust due to internal
    // bookkeeping for edge cases, which compounds in hot inner loops.
    // Convert `i <= limit` to `i < (limit + 1)`.
    let range_str = if is_exclusive {
        format!("{}..{}", counter_start, limit_str)
    } else {
        // For literal limits, compute limit+1 at compile time
        if let Expr::Literal(Literal::Number(n)) = limit_expr {
            format!("{}..{}", counter_start, n + 1)
        } else {
            format!("{}..({} + 1)", counter_start, limit_str)
        }
    };

    let mut output = String::new();
    writeln!(output, "{}for {} in {} {{", indent_str, counter_name, range_str).unwrap();

    // Emit body statements (excluding the final counter increment)
    ctx.push_scope();
    let body_refs: Vec<&Stmt> = body_without_increment.iter().collect();
    let mut bi = 0;
    while bi < body_refs.len() {
        if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        output.push_str(&super::codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
        bi += 1;
    }
    ctx.pop_scope();
    writeln!(output, "{}}}", indent_str).unwrap();

    // Emit post-loop counter value only if the counter is used after the loop.
    // This eliminates dead `let mut i = max(start, limit);` bindings.
    let remaining_stmts = &stmts[idx + 2..];
    if symbol_appears_in_stmts(counter_sym, remaining_stmts) {
        // After `while (i <= limit) { ...; i++ }`, i == limit + 1.
        // After `while (i < limit) { ...; i++ }`, i == limit.
        // If the loop never executes (start >= limit), the counter must stay at start.
        // Use max(start, limit) to handle both cases correctly.
        let post_value = if is_exclusive {
            if let Expr::Literal(Literal::Number(n)) = limit_expr {
                format!("{}", std::cmp::max(counter_start, *n))
            } else {
                format!("({}_i64).max({})", counter_start, limit_str)
            }
        } else {
            if let Expr::Literal(Literal::Number(n)) = limit_expr {
                format!("{}", std::cmp::max(counter_start, n + 1))
            } else {
                format!("({}_i64).max({} + 1)", counter_start, limit_str)
            }
        };
        writeln!(output, "{}let mut {} = {};", indent_str, counter_name, post_value).unwrap();
    }

    Some((output, 1)) // consumed 1 extra statement (the While)
}

/// Check if a slice of statements modifies a specific variable (used for for-range validity).
/// Recursively walks into nested If/While/Repeat blocks.
fn body_modifies_var(stmts: &[Stmt], sym: Symbol) -> bool {
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
        _ => "_".to_string(),
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

    // Only optimize for known Vec types (direct indexing)
    if let Some(t) = variable_types.get(&arr_sym_1) {
        if !t.starts_with("Vec") {
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

    // Pattern A: conditional swap with adjacent indices
    // Statement 2: Let b be item (j+1) of arr
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

    // idx_expr_2 must be idx_expr_1 + 1
    let is_adjacent = match idx_expr_2 {
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            (exprs_equal(left, idx_expr_1) && matches!(right, Expr::Literal(Literal::Number(1))))
            || (matches!(left, Expr::Literal(Literal::Number(1))) && exprs_equal(right, idx_expr_1))
        }
        _ => false,
    };
    if !is_adjacent {
        return None;
    }

    // Statement 3: If a > b (or a < b, etc.) then: SetIndex arr j b, SetIndex arr j+1 a
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
            let idx1_str = codegen_expr_simple(idx_expr_1, interner);
            let idx2_str = codegen_expr_simple(idx_expr_2, interner);

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
            writeln!(output, "{}if {}[({} - 1) as usize] {} {}[({} - 1) as usize] {{",
                indent_str, arr_name, idx1_str, op_str, arr_name, idx2_str,
            ).unwrap();
            writeln!(output, "{}    {}.swap(({} - 1) as usize, ({} - 1) as usize);",
                indent_str, arr_name, idx1_str, idx2_str).unwrap();
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

    // Pattern matched! Emit arr.swap()
    let indent_str = "    ".repeat(indent);
    let arr_name = interner.resolve(arr_sym);
    let idx1_str = codegen_expr_simple(idx_expr_1, interner);
    let idx2_str = codegen_expr_simple(idx_expr_2, interner);

    let mut output = String::new();
    writeln!(output, "{}{}.swap(({} - 1) as usize, ({} - 1) as usize);",
        indent_str, arr_name, idx1_str, idx2_str).unwrap();

    Some((output, 2)) // consumed 2 extra statements (SetIndex + SetIndex)
}
