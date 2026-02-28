use std::collections::{HashMap, HashSet};

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::logic::{LogicExpr, NumberKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::formatter::RustFormatter;
use crate::intern::{Interner, Symbol};
use crate::registry::SymbolRegistry;

use super::context::RefinementContext;
use super::detection::{collect_mutable_vars, expr_debug_prefix};
use super::types::{codegen_type_expr, infer_numeric_type};
use super::{
    codegen_stmt, get_root_identifier, has_copy_element_type, has_copy_value_type, is_copy_type,
};

pub fn codegen_expr(expr: &Expr, interner: &Interner, synced_vars: &HashSet<Symbol>) -> String {
    // Use empty registry, boxed_fields, and async_functions for simple expression codegen
    let empty_registry = TypeRegistry::new();
    let empty_async = HashSet::new();
    codegen_expr_boxed(expr, interner, synced_vars, &HashSet::new(), &empty_registry, &empty_async)
}

/// Phase 54+: Codegen expression with async function tracking.
/// Adds .await to async function calls at the expression level, handling nested calls.
pub(crate) fn codegen_expr_with_async(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let empty_registry = TypeRegistry::new();
    let empty_strings = HashSet::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, &HashSet::new(), &empty_registry, async_functions, &HashSet::new(), &empty_strings, variable_types)
}

/// Codegen expression with async support and string variable tracking.
pub(crate) fn codegen_expr_with_async_and_strings(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let empty_registry = TypeRegistry::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, &HashSet::new(), &empty_registry, async_functions, &HashSet::new(), string_vars, variable_types)
}

/// Check if an expression is definitely numeric (safe to use + operator).
/// This is conservative for Add operations - treats it as string concat only
/// when clearly dealing with strings (string literals).
pub(crate) fn is_definitely_numeric_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(Literal::Number(_)) => true,
        Expr::Literal(Literal::Float(_)) => true,
        Expr::Literal(Literal::Duration(_)) => true,
        // Identifiers might be strings, but without a string literal nearby,
        // assume numeric (Rust will catch type errors)
        Expr::Identifier(_) => true,
        // Arithmetic operations are numeric
        Expr::BinaryOp { op: BinaryOpKind::Subtract, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Multiply, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Divide, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Modulo, .. } => true,
        // Length always returns a number
        Expr::Length { .. } => true,
        // Add is numeric if both operands seem numeric
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            is_definitely_numeric_expr(left) && is_definitely_numeric_expr(right)
        }
        // Function calls - assume numeric (Rust type checker will validate)
        Expr::Call { .. } => true,
        // Index expressions - assume numeric
        Expr::Index { .. } => true,
        _ => true,
    }
}

/// Check if an expression is definitely a string (needs format! for concatenation).
/// Takes a set of known string variable symbols for identifier lookup.
pub(crate) fn is_definitely_string_expr_with_vars(expr: &Expr, string_vars: &HashSet<Symbol>) -> bool {
    match expr {
        // String literals are definitely strings
        Expr::Literal(Literal::Text(_)) => true,
        // Variables known to be strings
        Expr::Identifier(sym) => string_vars.contains(sym),
        // Concat always produces strings
        Expr::BinaryOp { op: BinaryOpKind::Concat, .. } => true,
        // Add with a string operand produces a string
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            is_definitely_string_expr_with_vars(left, string_vars)
                || is_definitely_string_expr_with_vars(right, string_vars)
        }
        // WithCapacity wrapping a string value is a string
        Expr::WithCapacity { value, .. } => is_definitely_string_expr_with_vars(value, string_vars),
        // Interpolated strings always produce strings
        Expr::InterpolatedString(_) => true,
        _ => false,
    }
}

/// Check if an expression is definitely a string (without variable tracking).
/// This is a fallback for contexts where string_vars isn't available.
pub(crate) fn is_definitely_string_expr(expr: &Expr) -> bool {
    let empty = HashSet::new();
    is_definitely_string_expr_with_vars(expr, &empty)
}

/// Collect leaf operands from a chain of string Add/Concat operations.
///
/// Walks left-leaning trees of `+` (on strings) and `Concat` operations,
/// collecting all leaf expressions into a flat Vec. This enables emitting
/// a single `format!("{}{}{}", a, b, c)` instead of nested
/// `format!("{}{}", format!("{}{}", a, b), c)`, avoiding O(n^2) allocation.
pub(crate) fn collect_string_concat_operands<'a, 'b>(
    expr: &'b Expr<'a>,
    string_vars: &HashSet<Symbol>,
    operands: &mut Vec<&'b Expr<'a>>,
) {
    match expr {
        Expr::BinaryOp { op: BinaryOpKind::Concat, left, right } => {
            collect_string_concat_operands(left, string_vars, operands);
            collect_string_concat_operands(right, string_vars, operands);
        }
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            let has_string = is_definitely_string_expr_with_vars(left, string_vars)
                || is_definitely_string_expr_with_vars(right, string_vars);
            if has_string {
                collect_string_concat_operands(left, string_vars, operands);
                collect_string_concat_operands(right, string_vars, operands);
            } else {
                operands.push(expr);
            }
        }
        _ => {
            operands.push(expr);
        }
    }
}

/// Phase 102: Codegen with boxed field support for recursive enums.
/// Phase 103: Added registry for polymorphic enum type inference.
/// Phase 54+: Added async_functions for proper .await on nested async calls.
pub(crate) fn codegen_expr_boxed(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,  // (EnumName, VariantName, FieldName)
    registry: &TypeRegistry,  // Phase 103: For type annotations on polymorphic enums
    async_functions: &HashSet<Symbol>,  // Phase 54+: Functions that are async
) -> String {
    // Delegate to codegen_expr_full with empty context for boxed bindings and string vars
    let empty_boxed = HashSet::new();
    let empty_strings = HashSet::new();
    let empty_types = HashMap::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, boxed_fields, registry, async_functions, &empty_boxed, &empty_strings, &empty_types)
}

/// Codegen with string variable tracking for proper string concatenation.
pub(crate) fn codegen_expr_boxed_with_strings(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
) -> String {
    let empty_boxed = HashSet::new();
    let empty_types = HashMap::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, boxed_fields, registry, async_functions, &empty_boxed, string_vars, &empty_types)
}

/// Codegen with variable type tracking for direct collection indexing optimization.
pub(crate) fn codegen_expr_boxed_with_types(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let empty_boxed = HashSet::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, boxed_fields, registry, async_functions, &empty_boxed, string_vars, variable_types)
}

/// Internal implementation of codegen_expr_boxed that can handle extra context.
fn codegen_expr_boxed_internal(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    boxed_bindings: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let names = RustNames::new(interner);
    // Helper macro for recursive calls with all context
    macro_rules! recurse {
        ($e:expr) => {
            codegen_expr_boxed_internal($e, interner, synced_vars, boxed_fields, registry, async_functions, boxed_bindings, string_vars, variable_types)
        };
    }

    match expr {
        Expr::Literal(lit) => codegen_literal(lit, interner),

        Expr::Identifier(sym) => {
            let name = names.ident(*sym);
            // Dereference boxed bindings from enum destructuring
            if boxed_bindings.contains(sym) {
                format!("(*{})", name)
            } else {
                name
            }
        }

        Expr::BinaryOp { op, left, right } => {
            // Flatten chained string concat/add into a single format! call.
            // Turns O(n^2) nested format! into O(n) single-allocation.
            let is_string_concat = matches!(op, BinaryOpKind::Concat)
                || (matches!(op, BinaryOpKind::Add)
                    && (is_definitely_string_expr_with_vars(left, string_vars)
                        || is_definitely_string_expr_with_vars(right, string_vars)));

            if is_string_concat {
                let mut operands = Vec::new();
                collect_string_concat_operands(expr, string_vars, &mut operands);
                let placeholders: String = operands.iter().map(|_| "{}").collect::<Vec<_>>().join("");
                let values: Vec<String> = operands.iter().map(|e| {
                    // String literals can be &str inside format!() — no heap allocation needed
                    if let Expr::Literal(Literal::Text(sym)) = e {
                        format!("\"{}\"", interner.resolve(*sym))
                    } else {
                        recurse!(e)
                    }
                }).collect();
                return format!("format!(\"{}\", {})", placeholders, values.join(", "));
            }

            // Optimize HashMap .get() for equality comparisons to avoid cloning
            if matches!(op, BinaryOpKind::Eq | BinaryOpKind::NotEq) {
                let neg = matches!(op, BinaryOpKind::NotEq);
                // Check if left side is a HashMap index
                if let Expr::Index { collection, index } = left {
                    if let Expr::Identifier(sym) = collection {
                        if let Some(t) = variable_types.get(sym) {
                            if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") || t.starts_with("rustc_hash::FxHashMap") || t.starts_with("FxHashMap") {
                                let coll_str = recurse!(collection);
                                let key_str = recurse!(index);
                                let val_str = recurse!(right);
                                let cmp = if neg { "!=" } else { "==" };
                                if has_copy_value_type(t) {
                                    return format!("({}.get(&({})).copied() {} Some({}))", coll_str, key_str, cmp, val_str);
                                } else {
                                    return format!("({}.get(&({})) {} Some(&({})))", coll_str, key_str, cmp, val_str);
                                }
                            }
                        }
                    }
                }
                // Check if right side is a HashMap index
                if let Expr::Index { collection, index } = right {
                    if let Expr::Identifier(sym) = collection {
                        if let Some(t) = variable_types.get(sym) {
                            if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") || t.starts_with("rustc_hash::FxHashMap") || t.starts_with("FxHashMap") {
                                let coll_str = recurse!(collection);
                                let key_str = recurse!(index);
                                let val_str = recurse!(left);
                                let cmp = if neg { "!=" } else { "==" };
                                if has_copy_value_type(t) {
                                    return format!("(Some({}) {} {}.get(&({})).copied())", val_str, cmp, coll_str, key_str);
                                } else {
                                    return format!("(Some(&({})) {} {}.get(&({})))", val_str, cmp, coll_str, key_str);
                                }
                            }
                        }
                    }
                }

                // Optimize string-index-vs-string-index comparison to use direct
                // byte comparison via as_bytes() instead of logos_get_char().
                // Byte equality is correct for UTF-8: two characters are equal
                // iff their byte representations are equal, and the logos_get_char
                // function already uses byte-level indexing for the ASCII fast path.
                if let (Expr::Index { collection: left_coll, index: left_idx },
                        Expr::Index { collection: right_coll, index: right_idx }) = (left, right) {
                    let left_is_string = if let Expr::Identifier(sym) = left_coll {
                        string_vars.contains(sym) || variable_types.get(sym).map_or(false, |t| t == "String")
                    } else { false };
                    let right_is_string = if let Expr::Identifier(sym) = right_coll {
                        string_vars.contains(sym) || variable_types.get(sym).map_or(false, |t| t == "String")
                    } else { false };
                    if left_is_string && right_is_string {
                        let cmp = if neg { "!=" } else { "==" };
                        let left_coll_str = recurse!(left_coll);
                        let right_coll_str = recurse!(right_coll);
                        let left_idx_simplified = super::peephole::simplify_1based_index(left_idx, interner, true);
                        let right_idx_simplified = super::peephole::simplify_1based_index(right_idx, interner, true);
                        return format!("({}.as_bytes()[{}] {} {}.as_bytes()[{}])",
                            left_coll_str, left_idx_simplified, cmp, right_coll_str, right_idx_simplified);
                    }
                }

                // Optimize string-index-vs-single-char-literal comparison to use
                // logos_get_char() == 'c' instead of LogosIndex::logos_get() == String::from("c").
                // Avoids two heap allocations per comparison in hot loops.
                let is_string_index = |expr: &Expr| -> bool {
                    if let Expr::Index { collection, .. } = expr {
                        if let Expr::Identifier(sym) = collection {
                            return string_vars.contains(sym) || variable_types.get(sym).map_or(false, |t| t == "String");
                        }
                    }
                    false
                };
                let single_char_literal = |expr: &Expr| -> Option<char> {
                    if let Expr::Literal(Literal::Text(sym)) = expr {
                        let s = interner.resolve(*sym);
                        let mut chars = s.chars();
                        if let Some(c) = chars.next() {
                            if chars.next().is_none() {
                                return Some(c);
                            }
                        }
                    }
                    None
                };

                // Left is string index, right is single-char literal
                if is_string_index(left) {
                    if let Some(ch) = single_char_literal(right) {
                        if let Expr::Index { collection, index } = left {
                            let coll_str = recurse!(collection);
                            let idx_str = recurse!(index);
                            let cmp = if neg { "!=" } else { "==" };
                            let ch_escaped = match ch {
                                '\'' => "\\'".to_string(),
                                '\\' => "\\\\".to_string(),
                                '\n' => "\\n".to_string(),
                                '\t' => "\\t".to_string(),
                                '\r' => "\\r".to_string(),
                                _ => ch.to_string(),
                            };
                            return format!("({}.logos_get_char({}) {} '{}')",
                                coll_str, idx_str, cmp, ch_escaped);
                        }
                    }
                }
                // Right is string index, left is single-char literal
                if is_string_index(right) {
                    if let Some(ch) = single_char_literal(left) {
                        if let Expr::Index { collection, index } = right {
                            let coll_str = recurse!(collection);
                            let idx_str = recurse!(index);
                            let cmp = if neg { "!=" } else { "==" };
                            let ch_escaped = match ch {
                                '\'' => "\\'".to_string(),
                                '\\' => "\\\\".to_string(),
                                '\n' => "\\n".to_string(),
                                '\t' => "\\t".to_string(),
                                '\r' => "\\r".to_string(),
                                _ => ch.to_string(),
                            };
                            return format!("('{}' {} {}.logos_get_char({}))",
                                ch_escaped, cmp, coll_str, idx_str);
                        }
                    }
                }
            }

            // OPT-8b: Zero-based counter in comparison.
            // When a __zero_based_i64 counter appears as a bare operand in a comparison,
            // emit (counter + 1) to compensate for the 0-based range shift.
            // E.g., `If i > 3` with 0-based `i` becomes `if (i + 1) > 3`.
            if matches!(op, BinaryOpKind::Lt | BinaryOpKind::LtEq | BinaryOpKind::Gt
                | BinaryOpKind::GtEq | BinaryOpKind::Eq | BinaryOpKind::NotEq)
            {
                let left_zb = if let Expr::Identifier(sym) = left {
                    variable_types.get(sym).map_or(false, |t| t == "__zero_based_i64")
                } else { false };
                let right_zb = if let Expr::Identifier(sym) = right {
                    variable_types.get(sym).map_or(false, |t| t == "__zero_based_i64")
                } else { false };
                if left_zb || right_zb {
                    let left_str = if left_zb {
                        format!("({} + 1)", recurse!(left))
                    } else { recurse!(left) };
                    let right_str = if right_zb {
                        format!("({} + 1)", recurse!(right))
                    } else { recurse!(right) };
                    let op_str = match op {
                        BinaryOpKind::Lt => "<", BinaryOpKind::LtEq => "<=",
                        BinaryOpKind::Gt => ">", BinaryOpKind::GtEq => ">=",
                        BinaryOpKind::Eq => "==", BinaryOpKind::NotEq => "!=",
                        _ => unreachable!(),
                    };
                    return format!("({} {} {})", left_str, op_str, right_str);
                }
            }

            let left_str = recurse!(left);
            let right_str = recurse!(right);

            // And/Or are type-aware: integers → bitwise (&/|), booleans → logical (&&/||)
            if matches!(op, BinaryOpKind::And | BinaryOpKind::Or) {
                let left_type = infer_numeric_type(left, interner, variable_types);
                let op_str = match op {
                    BinaryOpKind::And => if left_type == "i64" { "&" } else { "&&" },
                    BinaryOpKind::Or  => if left_type == "i64" { "|" } else { "||" },
                    _ => unreachable!(),
                };
                return format!("({} {} {})", left_str, op_str, right_str);
            }

            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And | BinaryOpKind::Or => unreachable!(), // handled above
                BinaryOpKind::Concat => unreachable!(), // handled above
                BinaryOpKind::BitXor => "^",
                BinaryOpKind::Shl => "<<",
                BinaryOpKind::Shr => ">>",
            };

            // Mixed Float*Int arithmetic coercion: if one side is f64,
            // cast the other side to f64 so Rust compiles mixed operations.
            if !matches!(op, BinaryOpKind::And | BinaryOpKind::Or) {
                let left_type = infer_numeric_type(left, interner, variable_types);
                let right_type = infer_numeric_type(right, interner, variable_types);
                if left_type == "f64" && right_type != "f64" {
                    return format!("({} {} (({}) as f64))", left_str, op_str, right_str);
                } else if right_type == "f64" && left_type != "f64" {
                    return format!("((({}) as f64) {} {})", left_str, op_str, right_str);
                }
            }

            format!("({} {} {})", left_str, op_str, right_str)
        }

        Expr::Call { function, args } => {
            let func_name = names.ident(*function);
            let raw_name = names.raw(*function);
            // Check if callee has borrow params (encoded as "fn_borrow:0,1" in variable_types)
            let callee_borrow_indices: HashSet<usize> = variable_types.get(function)
                .and_then(|t| t.strip_prefix("fn_borrow:"))
                .map(|s| s.split(',').filter_map(|i| i.parse().ok()).collect())
                .unwrap_or_default();
            // Recursively codegen args with full context.
            // Borrow params: pass &name (or pass through if already a slice).
            // Non-borrow params: clone non-Copy identifiers to avoid move-after-use.
            let args_str: Vec<String> = args.iter()
                .enumerate()
                .map(|(i, a)| {
                    let s = recurse!(a);
                    if callee_borrow_indices.contains(&i) {
                        // Borrow param: pass reference instead of cloning
                        if let Expr::Identifier(sym) = a {
                            if let Some(ty) = variable_types.get(sym) {
                                if ty.starts_with("&[") {
                                    return s; // Already a slice — pass through
                                }
                            }
                        }
                        format!("&{}", s)
                    } else {
                        // Regular param: clone non-Copy identifiers
                        if let Expr::Identifier(sym) = a {
                            if let Some(ty) = variable_types.get(sym) {
                                if !is_copy_type(ty) {
                                    return format!("{}.clone()", s);
                                }
                            }
                        }
                        s
                    }
                })
                .collect();
            // Builtin math functions → Rust method call syntax
            match raw_name {
                "sqrt" if args_str.len() == 1 => {
                    format!("(({}) as f64).sqrt()", args_str[0])
                }
                "abs" if args_str.len() == 1 => {
                    let arg_type = infer_numeric_type(&args[0], interner, variable_types);
                    if arg_type == "f64" {
                        format!("(({}) as f64).abs()", args_str[0])
                    } else {
                        format!("(({}) as i64).abs()", args_str[0])
                    }
                }
                "floor" if args_str.len() == 1 => {
                    format!("((({}) as f64).floor() as i64)", args_str[0])
                }
                "ceil" if args_str.len() == 1 => {
                    format!("((({}) as f64).ceil() as i64)", args_str[0])
                }
                "round" if args_str.len() == 1 => {
                    format!("((({}) as f64).round() as i64)", args_str[0])
                }
                "pow" if args_str.len() == 2 => {
                    format!("((({}) as f64).powf(({}) as f64))", args_str[0], args_str[1])
                }
                "min" if args_str.len() == 2 => {
                    format!("({}).min({})", args_str[0], args_str[1])
                }
                "max" if args_str.len() == 2 => {
                    format!("({}).max({})", args_str[0], args_str[1])
                }
                _ => {
                    // Add .await if this function is async
                    if async_functions.contains(function) {
                        format!("{}({}).await", func_name, args_str.join(", "))
                    } else {
                        format!("{}({})", func_name, args_str.join(", "))
                    }
                }
            }
        }

        Expr::Index { collection, index } => {
            let coll_str = recurse!(collection);
            // Direct indexing for known collection types (avoids trait dispatch)
            // Strip |__hl: hoisting suffix so type parsing (strip_suffix, etc.) works correctly.
            let known_type = if let Expr::Identifier(sym) = collection {
                variable_types.get(sym).map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
            } else {
                None
            };
            match known_type {
                Some(t) if t.starts_with("Vec") => {
                    let suffix = if has_copy_element_type(t) { "" } else { ".clone()" };
                    // OPT-8: Check if index is a zero-based counter (already 0-based, no -1 needed)
                    let is_zero_based_counter = if let Expr::Identifier(idx_sym) = index {
                        variable_types.get(idx_sym).map_or(false, |t| t == "__zero_based_i64")
                    } else {
                        false
                    };
                    let index_part = if is_zero_based_counter {
                        let idx_name = recurse!(index);
                        format!("{} as usize", idx_name)
                    } else { match index {
                        // Literal(1) → 0
                        Expr::Literal(Literal::Number(1)) => "0".to_string(),
                        // Literal(N) → N-1
                        Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
                        // (X + K) patterns: +1 cancels the -1 from 1-based indexing
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            match (left, right) {
                                (_, Expr::Literal(Literal::Number(1))) => {
                                    let left_str = recurse!(left);
                                    if matches!(left, Expr::Identifier(_)) {
                                        format!("{} as usize", left_str)
                                    } else {
                                        format!("({}) as usize", left_str)
                                    }
                                }
                                (Expr::Literal(Literal::Number(1)), _) => {
                                    let right_str = recurse!(right);
                                    if matches!(right, Expr::Identifier(_)) {
                                        format!("{} as usize", right_str)
                                    } else {
                                        format!("({}) as usize", right_str)
                                    }
                                }
                                (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                                    format!("({} + {}) as usize", recurse!(left), k - 1)
                                }
                                (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                                    format!("({} + {}) as usize", recurse!(right), k - 1)
                                }
                                _ => {
                                    format!("({} - 1) as usize", recurse!(index))
                                }
                            }
                        }
                        _ => {
                            format!("({} - 1) as usize", recurse!(index))
                        }
                    } };
                    format!("{}[{}]{}", coll_str, index_part, suffix)
                }
                Some(t) if t.starts_with("&[") || t.starts_with("&mut [") => {
                    let elem = t.strip_prefix("&mut [")
                        .or_else(|| t.strip_prefix("&["))
                        .and_then(|s| s.strip_suffix(']'))
                        .unwrap_or("_");
                    let suffix = if is_copy_type(elem) { "" } else { ".clone()" };
                    // OPT-8: Check if index is a zero-based counter
                    let is_zero_based_counter = if let Expr::Identifier(idx_sym) = index {
                        variable_types.get(idx_sym).map_or(false, |t| t == "__zero_based_i64")
                    } else {
                        false
                    };
                    let index_part = if is_zero_based_counter {
                        let idx_name = recurse!(index);
                        format!("{} as usize", idx_name)
                    } else { match index {
                        Expr::Literal(Literal::Number(1)) => "0".to_string(),
                        Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            match (left, right) {
                                (_, Expr::Literal(Literal::Number(1))) => {
                                    let left_str = recurse!(left);
                                    if matches!(left, Expr::Identifier(_)) {
                                        format!("{} as usize", left_str)
                                    } else {
                                        format!("({}) as usize", left_str)
                                    }
                                }
                                (Expr::Literal(Literal::Number(1)), _) => {
                                    let right_str = recurse!(right);
                                    if matches!(right, Expr::Identifier(_)) {
                                        format!("{} as usize", right_str)
                                    } else {
                                        format!("({}) as usize", right_str)
                                    }
                                }
                                (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                                    format!("({} + {}) as usize", recurse!(left), k - 1)
                                }
                                (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                                    format!("({} + {}) as usize", recurse!(right), k - 1)
                                }
                                _ => {
                                    format!("({} - 1) as usize", recurse!(index))
                                }
                            }
                        }
                        _ => {
                            format!("({} - 1) as usize", recurse!(index))
                        }
                    } };
                    format!("{}[{}]{}", coll_str, index_part, suffix)
                }
                Some(t) if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") || t.starts_with("rustc_hash::FxHashMap") || t.starts_with("FxHashMap") => {
                    let index_str = recurse!(index);
                    let suffix = if has_copy_value_type(t) { "" } else { ".clone()" };
                    format!("{}[&({})]{}", coll_str, index_str, suffix)
                }
                Some("String") => {
                    let index_str = recurse!(index);
                    format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
                }
                _ => {
                    let index_str = recurse!(index);
                    format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
                }
            }
        }

        Expr::Slice { collection, start, end } => {
            let coll_str = recurse!(collection);
            let start_str = recurse!(start);
            let end_str = recurse!(end);
            // Phase 43D: 1-indexed inclusive to 0-indexed exclusive
            // "items 1 through 3" → &items[0..3] (elements at indices 0, 1, 2)
            format!("&{}[({} - 1) as usize..{} as usize]", coll_str, start_str, end_str)
        }

        Expr::Copy { expr: inner } => {
            // Special case: Copy of Slice → emit arr[range].to_vec() without &
            // Otherwise the & from Slice codegen captures .to_owned() giving &Vec<T>
            if let Expr::Slice { collection, start, end } = inner {
                let coll_str = recurse!(collection);
                let start_str = recurse!(start);
                let end_str = recurse!(end);
                format!("{}[({} - 1) as usize..{} as usize].to_vec()", coll_str, start_str, end_str)
            } else {
                let expr_str = recurse!(inner);
                // Phase 43D: Explicit owned copy — .to_owned() is universal:
                // - &[T] (slices) → Vec<T> via [T]: ToOwned<Owned=Vec<T>>
                // - Vec<T>, HashMap<K,V>, HashSet<T> → Self via Clone blanket impl
                format!("{}.to_owned()", expr_str)
            }
        }

        Expr::Give { value } => {
            // Ownership transfer: emit value without .clone()
            // The move semantics are implicit in Rust - no special syntax needed
            recurse!(value)
        }

        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                if let Some(t) = variable_types.get(sym) {
                    if let Some(pos) = t.find("|__hl:") {
                        return t[pos + "|__hl:".len()..].to_string();
                    }
                }
            }
            let coll_str = recurse!(collection);
            // Phase 43D: Collection length - cast to i64 for LOGOS integer semantics
            format!("({}.len() as i64)", coll_str)
        }

        Expr::Contains { collection, value } => {
            let coll_str = recurse!(collection);
            let val_str = recurse!(value);
            // Use LogosContains trait for unified contains across List, Set, Map, Text
            format!("{}.logos_contains(&{})", coll_str, val_str)
        }

        Expr::Union { left, right } => {
            let left_str = recurse!(left);
            let right_str = recurse!(right);
            format!("{}.union(&{}).cloned().collect::<FxHashSet<_>>()", left_str, right_str)
        }

        Expr::Intersection { left, right } => {
            let left_str = recurse!(left);
            let right_str = recurse!(right);
            format!("{}.intersection(&{}).cloned().collect::<FxHashSet<_>>()", left_str, right_str)
        }

        // Phase 48: Sipping Protocol expressions
        Expr::ManifestOf { zone } => {
            let zone_str = recurse!(zone);
            format!("logicaffeine_system::network::FileSipper::from_zone(&{}).manifest()", zone_str)
        }

        Expr::ChunkAt { index, zone } => {
            let zone_str = recurse!(zone);
            let index_str = recurse!(index);
            // LOGOS uses 1-indexed, Rust uses 0-indexed
            format!("logicaffeine_system::network::FileSipper::from_zone(&{}).get_chunk(({} - 1) as usize)", zone_str, index_str)
        }

        Expr::List(ref items) => {
            let item_strs: Vec<String> = items.iter()
                .map(|i| recurse!(i))
                .collect();
            format!("vec![{}]", item_strs.join(", "))
        }

        Expr::Tuple(ref items) => {
            let item_strs: Vec<String> = items.iter()
                .map(|i| format!("Value::from({})", recurse!(i)))
                .collect();
            // Tuples as Vec<Value> for heterogeneous support
            format!("vec![{}]", item_strs.join(", "))
        }

        Expr::Range { start, end } => {
            let start_str = recurse!(start);
            let end_str = recurse!(end);
            format!("({}..={})", start_str, end_str)
        }

        Expr::FieldAccess { object, field } => {
            let field_name = interner.resolve(*field);

            // Phase 52: Check if root object is synced - use .get().await
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    let obj_name = interner.resolve(sym);
                    return format!("{}.get().await.{}", obj_name, field_name);
                }
            }

            let obj_str = recurse!(object);
            format!("{}.{}", obj_str, field_name)
        }

        Expr::New { type_name, type_args, init_fields } => {
            let type_str = interner.resolve(*type_name);
            if !init_fields.is_empty() {
                // Struct initialization with fields: Point { x: 10, y: 20, ..Default::default() }
                // Always add ..Default::default() to handle partial initialization (e.g., CRDT fields)
                let fields_str = init_fields.iter()
                    .map(|(name, value)| {
                        let field_name = interner.resolve(*name);
                        let value_str = recurse!(value);
                        format!("{}: {}", field_name, value_str)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} {{ {}, ..Default::default() }}", type_str, fields_str)
            } else if type_args.is_empty() {
                format!("{}::default()", type_str)
            } else {
                // Phase 34: Turbofish syntax for generic instantiation
                // Bug fix: Use codegen_type_expr to support nested types like Seq of (Seq of Int)
                let args_str = type_args.iter()
                    .map(|t| codegen_type_expr(t, interner))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}::<{}>::default()", type_str, args_str)
            }
        }

        Expr::NewVariant { enum_name, variant, fields } => {
            let enum_str = interner.resolve(*enum_name);
            let variant_str = interner.resolve(*variant);
            if fields.is_empty() {
                // Unit variant: Shape::Point
                format!("{}::{}", enum_str, variant_str)
            } else {
                // Phase 103: Count identifier usage to handle cloning for reused values
                // We need to clone on all uses except the last one
                let mut identifier_counts: HashMap<Symbol, usize> = HashMap::new();
                for (_, value) in fields.iter() {
                    if let Expr::Identifier(sym) = value {
                        *identifier_counts.entry(*sym).or_insert(0) += 1;
                    }
                }

                // Track remaining uses for each identifier
                let mut remaining_uses: HashMap<Symbol, usize> = identifier_counts.clone();

                // Struct variant: Shape::Circle { radius: 10 }
                // Phase 102: Check if any field is recursive and needs Box::new()
                let fields_str: Vec<String> = fields.iter()
                    .map(|(field_name, value)| {
                        let name = interner.resolve(*field_name);

                        // Phase 103: Clone identifiers that are used multiple times
                        // Clone on all uses except the last one (to allow move on final use)
                        let val = if let Expr::Identifier(sym) = value {
                            let total = identifier_counts.get(sym).copied().unwrap_or(0);
                            let remaining = remaining_uses.get_mut(sym);
                            let base_name = if boxed_bindings.contains(sym) {
                                format!("(*{})", interner.resolve(*sym))
                            } else {
                                interner.resolve(*sym).to_string()
                            };
                            if total > 1 {
                                if let Some(r) = remaining {
                                    *r -= 1;
                                    if *r > 0 {
                                        // Not the last use, need to clone
                                        format!("{}.clone()", base_name)
                                    } else {
                                        // Last use, can move
                                        base_name
                                    }
                                } else {
                                    base_name
                                }
                            } else {
                                base_name
                            }
                        } else {
                            recurse!(value)
                        };

                        // Check if this field needs to be boxed (recursive type)
                        let key = (enum_str.to_string(), variant_str.to_string(), name.to_string());
                        if boxed_fields.contains(&key) {
                            format!("{}: Box::new({})", name, val)
                        } else {
                            format!("{}: {}", name, val)
                        }
                    })
                    .collect();
                format!("{}::{} {{ {} }}", enum_str, variant_str, fields_str.join(", "))
            }
        }

        Expr::OptionSome { value } => {
            format!("Some({})", recurse!(value))
        }

        Expr::OptionNone => {
            "None".to_string()
        }

        Expr::Escape { code, .. } => {
            let raw_code = interner.resolve(*code);
            let mut block = String::from("{\n");
            for line in raw_code.lines() {
                block.push_str("    ");
                block.push_str(line);
                block.push('\n');
            }
            block.push('}');
            block
        }

        Expr::WithCapacity { value, capacity } => {
            let cap_str = recurse!(capacity);
            match value {
                // Empty string → String::with_capacity(cap)
                Expr::Literal(Literal::Text(sym)) if interner.resolve(*sym).is_empty() => {
                    format!("String::with_capacity(({}) as usize)", cap_str)
                }
                // Non-empty string → { let mut __s = String::with_capacity(cap); __s.push_str("..."); __s }
                Expr::Literal(Literal::Text(sym)) => {
                    let text = interner.resolve(*sym);
                    format!("{{ let mut __s = String::with_capacity(({}) as usize); __s.push_str(\"{}\"); __s }}", cap_str, text)
                }
                // Collection Expr::New → Type::with_capacity(cap)
                Expr::New { type_name, type_args, .. } => {
                    let type_str = interner.resolve(*type_name);
                    match type_str {
                        "Seq" | "List" | "Vec" => {
                            let elem = if !type_args.is_empty() {
                                codegen_type_expr(&type_args[0], interner)
                            } else { "()".to_string() };
                            format!("{{ let __v: Vec<{}> = Vec::with_capacity(({}) as usize); __v }}", elem, cap_str)
                        }
                        "Map" | "HashMap" => {
                            let (k, v) = if type_args.len() >= 2 {
                                (codegen_type_expr(&type_args[0], interner),
                                 codegen_type_expr(&type_args[1], interner))
                            } else { ("String".to_string(), "String".to_string()) };
                            format!("{{ let __m: FxHashMap<{}, {}> = FxHashMap::with_capacity_and_hasher(({}) as usize, Default::default()); __m }}", k, v, cap_str)
                        }
                        "Set" | "HashSet" => {
                            let elem = if !type_args.is_empty() {
                                codegen_type_expr(&type_args[0], interner)
                            } else { "()".to_string() };
                            format!("{{ let __s: FxHashSet<{}> = FxHashSet::with_capacity_and_hasher(({}) as usize, Default::default()); __s }}", elem, cap_str)
                        }
                        _ => recurse!(value) // Unknown type — ignore capacity
                    }
                }
                // Other expressions — ignore capacity hint
                _ => recurse!(value)
            }
        }

        Expr::Closure { params, body, .. } => {
            use crate::ast::stmt::ClosureBody;
            let params_str: Vec<String> = params.iter()
                .map(|(name, ty)| {
                    let param_name = names.ident(*name);
                    let param_type = codegen_type_expr(ty, interner);
                    format!("{}: {}", param_name, param_type)
                })
                .collect();

            match body {
                ClosureBody::Expression(expr) => {
                    let body_str = recurse!(expr);
                    format!("move |{}| {{ {} }}", params_str.join(", "), body_str)
                }
                ClosureBody::Block(stmts) => {
                    let mut body_str = String::new();
                    let mut ctx = RefinementContext::new();
                    let empty_mutable = collect_mutable_vars(stmts);
                    let empty_lww = HashSet::new();
                    let empty_mv = HashSet::new();
                    let mut empty_synced = HashSet::new();
                    let empty_caps = HashMap::new();
                    let empty_pipes = HashSet::new();
                    let empty_boxed = HashSet::new();
                    let empty_registry = TypeRegistry::new();
                    let type_env = crate::analysis::types::TypeEnv::new();
                    for stmt in stmts.iter() {
                        body_str.push_str(&codegen_stmt(
                            stmt, interner, 2, &empty_mutable, &mut ctx,
                            &empty_lww, &empty_mv, &mut empty_synced, &empty_caps,
                            async_functions, &empty_pipes, &empty_boxed, &empty_registry,
                            &type_env,
                        ));
                    }
                    format!("move |{}| {{\n{}{}}}", params_str.join(", "), body_str, "    ")
                }
            }
        }

        Expr::CallExpr { callee, args } => {
            let callee_str = recurse!(callee);
            let args_str: Vec<String> = args.iter().map(|a| recurse!(a)).collect();
            format!("({})({})", callee_str, args_str.join(", "))
        }

        Expr::InterpolatedString(parts) => {
            codegen_interpolated_string(parts, interner, synced_vars, boxed_fields, registry, async_functions, boxed_bindings, string_vars, variable_types)
        }

        Expr::Not { operand } => {
            let operand_str = recurse!(operand);
            format!("!({})", operand_str)
        }
    }
}

pub(crate) fn codegen_interpolated_string(
    parts: &[crate::ast::stmt::StringPart],
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    boxed_bindings: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    use crate::ast::stmt::StringPart;

    let mut fmt_str = String::new();
    let mut args = Vec::new();

    for part in parts {
        match part {
            StringPart::Literal(sym) => {
                let text = interner.resolve(*sym);
                // Escape braces and special chars in the format string
                for ch in text.chars() {
                    match ch {
                        '{' => fmt_str.push_str("{{"),
                        '}' => fmt_str.push_str("}}"),
                        '\n' => fmt_str.push_str("\\n"),
                        '\t' => fmt_str.push_str("\\t"),
                        '\r' => fmt_str.push_str("\\r"),
                        _ => fmt_str.push(ch),
                    }
                }
            }
            StringPart::Expr { value, format_spec, debug } => {
                if *debug {
                    let debug_prefix = expr_debug_prefix(value, interner);
                    for ch in debug_prefix.chars() {
                        match ch {
                            '{' => fmt_str.push_str("{{"),
                            '}' => fmt_str.push_str("}}"),
                            _ => fmt_str.push(ch),
                        }
                    }
                    fmt_str.push('=');
                }
                let needs_float_cast = if let Some(spec) = format_spec {
                    let spec_str = interner.resolve(*spec);
                    if spec_str == "$" {
                        fmt_str.push('$');
                        fmt_str.push_str("{:.2}");
                        true
                    } else if spec_str.starts_with('.') {
                        fmt_str.push_str(&format!("{{:{}}}", spec_str));
                        true
                    } else {
                        fmt_str.push_str(&format!("{{:{}}}", spec_str));
                        false
                    }
                } else {
                    fmt_str.push_str("{}");
                    false
                };
                let arg_str = codegen_expr_boxed_internal(
                    value, interner, synced_vars, boxed_fields, registry,
                    async_functions, boxed_bindings, string_vars, variable_types,
                );
                if needs_float_cast {
                    args.push(format!("{} as f64", arg_str));
                } else {
                    args.push(arg_str);
                }
            }
        }
    }

    if args.is_empty() {
        // No holes — emit raw String::from (no format! escaping needed).
        // Reconstruct the raw text from parts without brace escaping.
        let mut raw = String::new();
        for part in parts {
            if let StringPart::Literal(sym) = part {
                let text = interner.resolve(*sym);
                for ch in text.chars() {
                    match ch {
                        '\n' => raw.push_str("\\n"),
                        '\t' => raw.push_str("\\t"),
                        '\r' => raw.push_str("\\r"),
                        '"' => raw.push_str("\\\""),
                        '\\' => raw.push_str("\\\\"),
                        _ => raw.push(ch),
                    }
                }
            }
        }
        format!("String::from(\"{}\")", raw)
    } else {
        format!("format!(\"{}\"{})", fmt_str, args.iter().map(|a| format!(", {}", a)).collect::<String>())
    }
}

pub(crate) fn codegen_literal(lit: &Literal, interner: &Interner) -> String {
    match lit {
        Literal::Number(n) => n.to_string(),
        Literal::Float(f) => format!("{}f64", f),
        // String literals are converted to String for consistent Text type handling
        Literal::Text(sym) => {
            let raw = interner.resolve(*sym);
            let escaped: String = raw.chars().map(|c| match c {
                '\n' => "\\n".to_string(),
                '\r' => "\\r".to_string(),
                '\t' => "\\t".to_string(),
                '\\' => "\\\\".to_string(),
                '"' => "\\\"".to_string(),
                other => other.to_string(),
            }).collect();
            format!("String::from(\"{}\")", escaped)
        }
        Literal::Boolean(b) => b.to_string(),
        Literal::Nothing => "()".to_string(),
        // Character literals
        Literal::Char(c) => {
            // Handle escape sequences for special characters
            match c {
                '\n' => "'\\n'".to_string(),
                '\t' => "'\\t'".to_string(),
                '\r' => "'\\r'".to_string(),
                '\\' => "'\\\\'".to_string(),
                '\'' => "'\\''".to_string(),
                '\0' => "'\\0'".to_string(),
                c => format!("'{}'", c),
            }
        }
        // Temporal literals: Duration stored as nanoseconds (i64)
        Literal::Duration(nanos) => format!("std::time::Duration::from_nanos({}u64)", nanos),
        // Date stored as days since Unix epoch (i32)
        Literal::Date(days) => format!("LogosDate({})", days),
        // Moment stored as nanoseconds since Unix epoch (i64)
        Literal::Moment(nanos) => format!("LogosMoment({})", nanos),
        // Span stored as (months, days) - separate because they're incommensurable
        Literal::Span { months, days } => format!("LogosSpan::new({}, {})", months, days),
        // Time-of-day stored as nanoseconds from midnight
        Literal::Time(nanos) => format!("LogosTime({})", nanos),
    }
}

/// Converts a LogicExpr to a Rust boolean expression for debug_assert!().
/// Uses RustFormatter to unify all logic-to-Rust translation.
pub fn codegen_assertion(expr: &LogicExpr, interner: &Interner) -> String {
    let mut registry = SymbolRegistry::new();
    let formatter = RustFormatter;
    let mut buf = String::new();

    match expr.write_logic(&mut buf, &mut registry, interner, &formatter) {
        Ok(_) => buf,
        Err(_) => "/* error generating assertion */ false".to_string(),
    }
}

pub fn codegen_term(term: &Term, interner: &Interner) -> String {
    match term {
        Term::Constant(sym) => interner.resolve(*sym).to_string(),
        Term::Variable(sym) => interner.resolve(*sym).to_string(),
        Term::Value { kind, .. } => match kind {
            NumberKind::Integer(n) => n.to_string(),
            NumberKind::Real(f) => f.to_string(),
            NumberKind::Symbolic(sym) => interner.resolve(*sym).to_string(),
        },
        Term::Function(name, args) => {
            let args_str: Vec<String> = args.iter()
                .map(|a| codegen_term(a, interner))
                .collect();
            format!("{}({})", interner.resolve(*name), args_str.join(", "))
        }
        Term::Possessed { possessor, possessed } => {
            let poss_str = codegen_term(possessor, interner);
            format!("{}.{}", poss_str, interner.resolve(*possessed))
        }
        Term::Group(members) => {
            let members_str: Vec<String> = members.iter()
                .map(|m| codegen_term(m, interner))
                .collect();
            format!("({})", members_str.join(", "))
        }
        _ => "/* unsupported Term */".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_number() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        let expr = Expr::Literal(Literal::Number(42));
        assert_eq!(codegen_expr(&expr, &interner, &synced_vars), "42");
    }

    #[test]
    fn test_literal_boolean() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(true)), &interner, &synced_vars), "true");
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(false)), &interner, &synced_vars), "false");
    }

    #[test]
    fn test_literal_nothing() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Nothing), &interner, &synced_vars), "()");
    }
}
