use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use super::context::{RefinementContext, VariableCapabilities};
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

    // Emit post-loop counter value so subsequent code sees the correct value.
    // After `while (i <= limit) { ...; i++ }`, i == limit + 1.
    // After `while (i < limit) { ...; i++ }`, i == limit.
    // If the loop never executes (start >= limit), the counter must stay at start.
    // Use max(start, limit) to handle both cases correctly.
    let post_value = if is_exclusive {
        if let Expr::Literal(Literal::Number(n)) = limit_expr {
            // Both start and limit known at compile time
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

/// Peephole optimization: detect `Let vec = new Seq. Let i = 0. While i <= limit: push const to vec, i = i+1`
/// and emit `let mut vec: Vec<T> = vec![const; (limit + 1) as usize]` instead.
/// Returns (generated_code, number_of_extra_statements_consumed) or None if pattern doesn't match.
pub(crate) fn try_emit_vec_fill_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
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

    // Statement 2: Let [mutable] counter = 0 (or 1).
    // Note: mutable keyword is optional — mutability is inferred from Set in the loop body.
    let (counter_sym, counter_start) = match stmts[idx + 1] {
        Stmt::Let { var, value: Expr::Literal(Literal::Number(n)), .. } => {
            (*var, *n)
        }
        _ => return None,
    };

    // Statement 3: While counter <= limit (or counter < limit): Push const_val to vec_var. Set counter to counter + 1.
    match stmts[idx + 2] {
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
            let val_str = match push_val {
                Expr::Literal(Literal::Number(n)) => format!("{}", n),
                Expr::Literal(Literal::Float(f)) => format!("{:.1}", f),
                Expr::Literal(Literal::Boolean(b)) => format!("{}", b),
                Expr::Literal(Literal::Char(c)) => format!("'{}'", c),
                Expr::Literal(Literal::Text(s)) => format!("{}.to_string()", interner.resolve(*s)),
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

            // Calculate count based on bound type (exclusive vs inclusive) and start value
            // Inclusive (<=): count = limit - start + 1
            // Exclusive (<): count = limit - start
            let count_expr = if is_exclusive {
                // Exclusive bound: counter < limit
                if counter_start == 0 {
                    format!("{} as usize", limit_str)
                } else {
                    format!("({} - {}) as usize", limit_str, counter_start)
                }
            } else {
                // Inclusive bound: counter <= limit
                if counter_start == 0 {
                    format!("({} + 1) as usize", limit_str)
                } else if counter_start == 1 {
                    format!("{} as usize", limit_str)
                } else {
                    format!("({} - {} + 1) as usize", limit_str, counter_start)
                }
            };

            let mut output = String::new();
            writeln!(output, "{}let mut {}: Vec<{}> = vec![{}; {}];",
                indent_str, vec_name, elem_type, val_str, count_expr).unwrap();
            // Re-emit counter variable declaration (it may be reused after the fill loop)
            let names = RustNames::new(interner);
            let counter_name = names.ident(counter_sym);
            writeln!(output, "{}let mut {} = {};",
                indent_str, counter_name, counter_start).unwrap();

            Some((output, 2)) // consumed 2 extra statements (counter init + while loop)
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

/// Check if two expressions are structurally equal (for swap pattern detection).
fn exprs_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(s1), Expr::Identifier(s2)) => s1 == s2,
        (Expr::Literal(Literal::Number(n1)), Expr::Literal(Literal::Number(n2))) => n1 == n2,
        (Expr::BinaryOp { op: op1, left: l1, right: r1 }, Expr::BinaryOp { op: op2, left: l2, right: r2 }) => {
            op1 == op2 && exprs_equal(l1, l2) && exprs_equal(r1, r2)
        }
        _ => false,
    }
}

/// Peephole optimization: detect swap pattern:
///   Let a be item j of arr. Let b be item (j+1) of arr.
///   If a > b then: Set item j of arr to b. Set item (j+1) of arr to a.
/// and emit `arr.swap((j-1) as usize, ((j+1)-1) as usize)` instead.
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

    // Statement 1: Let a be item j of arr (index expression)
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

    // Statement 2: Let b be item (j+1) of arr (adjacent index)
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
