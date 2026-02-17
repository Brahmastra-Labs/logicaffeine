use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldType, TypeDef, TypeRegistry};
use crate::analysis::types::RustNames;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, ReadSource, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

use super::context::{RefinementContext, VariableCapabilities, emit_refinement_check, analyze_variable_capabilities};
use super::detection::{
    requires_async_stmt, calls_async_function, collect_mutable_vars, collect_mutable_vars_stmt,
    collect_crdt_register_fields, collect_boxed_fields, collect_expr_identifiers,
    collect_stmt_identifiers, expr_debug_prefix, get_root_identifier_for_mutability,
    is_copy_type_expr, is_hashable_type,
};
use super::expr::{
    codegen_expr, codegen_expr_with_async, codegen_expr_boxed,
    codegen_expr_boxed_with_strings, codegen_expr_boxed_with_types,
    codegen_interpolated_string, codegen_literal, codegen_assertion,
    codegen_expr_with_async_and_strings, is_definitely_string_expr_with_vars,
    is_definitely_string_expr, is_definitely_numeric_expr,
    collect_string_concat_operands,
};
use super::peephole::{
    try_emit_for_range_pattern, try_emit_vec_fill_pattern, try_emit_swap_pattern,
    body_mutates_collection,
};
use super::types::{
    codegen_type_expr, infer_rust_type_from_expr, infer_numeric_type,
    infer_variant_type_annotation,
};
use super::escape_rust_ident;

pub fn codegen_stmt<'a>(
    stmt: &Stmt<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,  // Phase 49b: MVRegister fields (no timestamp)
    synced_vars: &mut HashSet<Symbol>,  // Phase 52: Track synced variables
    var_caps: &HashMap<Symbol, VariableCapabilities>,  // Phase 56: Mount+Sync detection
    async_functions: &HashSet<Symbol>,  // Phase 54: Functions that are async
    pipe_vars: &HashSet<Symbol>,  // Phase 54: Pipe declarations (have _tx/_rx suffixes)
    boxed_fields: &HashSet<(String, String, String)>,  // Phase 102: Recursive enum fields
    registry: &TypeRegistry,  // Phase 103: For type annotations on polymorphic enums
    type_env: &crate::analysis::types::TypeEnv,
) -> String {
    let indent_str = "    ".repeat(indent);
    let mut output = String::new();
    let names = RustNames::new(interner);

    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            let var_name = names.ident(*var);

            // Register collection type for direct indexing optimization.
            // Check explicit type annotation first, then infer from Expr::New.
            if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                match base_name {
                    "Seq" | "List" | "Vec" => {
                        let rust_type = if !params.is_empty() {
                            format!("Vec<{}>", codegen_type_expr(&params[0], interner))
                        } else {
                            "Vec<()>".to_string()
                        };
                        ctx.register_variable_type(*var, rust_type);
                    }
                    "Map" | "HashMap" => {
                        let rust_type = if params.len() >= 2 {
                            format!("std::collections::HashMap<{}, {}>", codegen_type_expr(&params[0], interner), codegen_type_expr(&params[1], interner))
                        } else {
                            "std::collections::HashMap<String, String>".to_string()
                        };
                        ctx.register_variable_type(*var, rust_type);
                    }
                    _ => {}
                }
            } else if let Expr::New { type_name, type_args, .. } = value {
                let type_str = interner.resolve(*type_name);
                match type_str {
                    "Seq" | "List" | "Vec" => {
                        let rust_type = if !type_args.is_empty() {
                            format!("Vec<{}>", codegen_type_expr(&type_args[0], interner))
                        } else {
                            "Vec<()>".to_string()
                        };
                        ctx.register_variable_type(*var, rust_type);
                    }
                    "Map" | "HashMap" => {
                        let rust_type = if type_args.len() >= 2 {
                            format!("std::collections::HashMap<{}, {}>", codegen_type_expr(&type_args[0], interner), codegen_type_expr(&type_args[1], interner))
                        } else {
                            "std::collections::HashMap<String, String>".to_string()
                        };
                        ctx.register_variable_type(*var, rust_type);
                    }
                    _ => {}
                }
            } else if let Expr::List(items) = value {
                // Infer element type from first literal in the list for Copy elimination
                let elem_type = items.first()
                    .map(|e| infer_rust_type_from_expr(e, interner))
                    .unwrap_or_else(|| "_".to_string());
                ctx.register_variable_type(*var, format!("Vec<{}>", elem_type));
            }

            // Register scalar types for mixed Float*Int arithmetic coercion
            if !ctx.get_variable_types().contains_key(var) {
                let inferred = infer_rust_type_from_expr(value, interner);
                if inferred != "_" {
                    ctx.register_variable_type(*var, inferred);
                } else {
                    // Try deeper numeric type inference for expressions like `4.0 * pi * pi`
                    let numeric = infer_numeric_type(value, interner, ctx.get_variable_types());
                    if numeric != "unknown" {
                        ctx.register_variable_type(*var, numeric.to_string());
                    }
                }
            }

            // Phase 54+: Use codegen_expr_boxed with string+type tracking for proper codegen
            let value_str = codegen_expr_boxed_with_types(
                value, interner, synced_vars, boxed_fields, registry, async_functions,
                ctx.get_string_vars(), ctx.get_variable_types()
            );

            // Phase 103: Get explicit type annotation or infer for multi-param generic enums
            let type_annotation = ty.map(|t| codegen_type_expr(t, interner))
                .or_else(|| infer_variant_type_annotation(value, registry, interner));

            // Grand Challenge: Variable is mutable if explicitly marked OR if it's a Set target
            let is_mutable = *mutable || mutable_vars.contains(var);

            match (is_mutable, type_annotation) {
                (true, Some(t)) => writeln!(output, "{}let mut {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (true, None) => writeln!(output, "{}let mut {} = {};", indent_str, var_name, value_str).unwrap(),
                (false, Some(t)) => writeln!(output, "{}let {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (false, None) => writeln!(output, "{}let {} = {};", indent_str, var_name, value_str).unwrap(),
            }

            // Track string variables for proper concatenation in subsequent expressions
            if is_definitely_string_expr_with_vars(value, ctx.get_string_vars()) {
                ctx.register_string_var(*var);
            }

            // Phase 43C: Handle refinement type
            if let Some(TypeExpr::Refinement { base: _, var: bound_var, predicate }) = ty {
                emit_refinement_check(&var_name, *bound_var, predicate, interner, &indent_str, &mut output);
                ctx.register(*var, *bound_var, predicate);
            }
        }

        Stmt::Set { target, value } => {
            let target_name = names.ident(*target);
            let string_vars = ctx.get_string_vars();
            let var_types = ctx.get_variable_types();

            // Optimization: detect self-append pattern (result = result + x + y)
            // and emit write!(result, "{}{}", x, y) instead of result = format!(...).
            // This is O(n) amortized (in-place append) vs O(n²) (full copy each iteration).
            let used_write = if ctx.is_string_var(*target)
                && is_definitely_string_expr_with_vars(value, string_vars)
            {
                let mut operands = Vec::new();
                collect_string_concat_operands(value, string_vars, &mut operands);

                // Need at least 2 operands, leftmost must be the target variable
                if operands.len() >= 2 && matches!(operands[0], Expr::Identifier(sym) if *sym == *target) {
                    // Check no other operand references target (would cause borrow conflict)
                    let tail = &operands[1..];
                    let mut tail_ids = HashSet::new();
                    for op in tail {
                        collect_expr_identifiers(op, &mut tail_ids);
                    }

                    if !tail_ids.contains(target) {
                        // Safe to emit write!() — target not referenced in tail operands
                        let placeholders: String = tail.iter().map(|_| "{}").collect::<Vec<_>>().join("");
                        let values: Vec<String> = tail.iter().map(|e| {
                            // String literals can be &str inside write!() — no heap allocation needed
                            if let Expr::Literal(Literal::Text(sym)) = e {
                                format!("\"{}\"", interner.resolve(*sym))
                            } else {
                                codegen_expr_boxed_with_types(
                                    e, interner, synced_vars, boxed_fields, registry, async_functions,
                                    string_vars, var_types
                                )
                            }
                        }).collect();
                        writeln!(output, "{}write!({}, \"{}\", {}).unwrap();",
                            indent_str, target_name, placeholders, values.join(", ")).unwrap();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if !used_write {
                // Fallback: standard assignment with format!
                let value_str = codegen_expr_boxed_with_types(
                    value, interner, synced_vars, boxed_fields, registry, async_functions,
                    string_vars, var_types
                );
                writeln!(output, "{}{} = {};", indent_str, target_name, value_str).unwrap();
            }

            // Phase 43C: Check if this variable has a refinement constraint
            if let Some((bound_var, predicate)) = ctx.get_constraint(*target) {
                emit_refinement_check(&target_name, bound_var, predicate, interner, &indent_str, &mut output);
            }
        }

        Stmt::Call { function, args } => {
            let func_name = names.ident(*function);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types())).collect();
            // Add .await if calling an async function
            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
            writeln!(output, "{}{}({}){};", indent_str, func_name, args_str.join(", "), await_suffix).unwrap();
        }

        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for stmt in *then_block {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for stmt in *else_stmts {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::While { cond, body, decreasing: _ } => {
            // decreasing is compile-time only, ignored at runtime
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            // Peephole: process body statements with peephole optimizations
            let body_refs: Vec<&Stmt> = body.iter().collect();
            let mut bi = 0;
            while bi < body_refs.len() {
                if let Some((code, skip)) = try_emit_vec_fill_pattern(&body_refs, bi, interner, indent + 1) {
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
                output.push_str(&codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                bi += 1;
            }
            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Repeat { pattern, iterable, body } => {
            use crate::ast::stmt::Pattern;

            // Generate pattern string for Rust code
            let pattern_str = match pattern {
                Pattern::Identifier(sym) => interner.resolve(*sym).to_string(),
                Pattern::Tuple(syms) => {
                    let names = syms.iter()
                        .map(|s| interner.resolve(*s))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({})", names)
                }
            };

            let iter_str = codegen_expr_with_async(iterable, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Check if body contains async operations - if so, use while-let pattern
            // because standard for loops cannot contain .await
            let body_has_async = body.iter().any(|s| {
                requires_async_stmt(s) || calls_async_function(s, async_functions)
            });

            if body_has_async {
                // Use while-let with explicit iterator for async compatibility
                writeln!(output, "{}let mut __iter = ({}).into_iter();", indent_str, iter_str).unwrap();
                writeln!(output, "{}while let Some({}) = __iter.next() {{", indent_str, pattern_str).unwrap();
            } else {
                // Optimization: for known Vec<T> with Copy element type and non-mutating body,
                // use .iter().copied() instead of .clone() to avoid copying the entire collection.
                let use_iter_copied = if let Expr::Identifier(coll_sym) = iterable {
                    if let Some(coll_type) = ctx.get_variable_types().get(coll_sym) {
                        coll_type.starts_with("Vec") && has_copy_element_type(coll_type)
                            && !body_mutates_collection(body, *coll_sym)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if use_iter_copied {
                    writeln!(output, "{}for {} in {}.iter().copied() {{", indent_str, pattern_str, iter_str).unwrap();
                } else {
                    // Clone the collection before iterating to avoid moving it.
                    // This allows the collection to be reused after the loop.
                    writeln!(output, "{}for {} in {}.clone() {{", indent_str, pattern_str, iter_str).unwrap();
                }
            }
            ctx.push_scope();
            // Peephole: process body statements with swap pattern detection
            {
                let body_refs: Vec<&Stmt> = body.iter().collect();
                let mut bi = 0;
                while bi < body_refs.len() {
                    if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
                        output.push_str(&code);
                        bi += 1 + skip;
                        continue;
                    }
                    output.push_str(&codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                    bi += 1;
                }
            }
            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Return { value } => {
            if let Some(v) = value {
                let value_str = codegen_expr_with_async(v, interner, synced_vars, async_functions, ctx.get_variable_types());
                writeln!(output, "{}return {};", indent_str, value_str).unwrap();
            } else {
                writeln!(output, "{}return;", indent_str).unwrap();
            }
        }

        Stmt::Assert { proposition } => {
            let condition = codegen_assertion(proposition, interner);
            writeln!(output, "{}debug_assert!({});", indent_str, condition).unwrap();
        }

        // Phase 35: Trust with documented justification
        Stmt::Trust { proposition, justification } => {
            let reason = interner.resolve(*justification);
            // Strip quotes if present (string literals include their quotes)
            let reason_clean = reason.trim_matches('"');
            writeln!(output, "{}// TRUST: {}", indent_str, reason_clean).unwrap();
            let condition = codegen_assertion(proposition, interner);
            writeln!(output, "{}debug_assert!({});", indent_str, condition).unwrap();
        }

        Stmt::RuntimeAssert { condition } => {
            let cond_str = codegen_expr_with_async(condition, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}debug_assert!({});", indent_str, cond_str).unwrap();
        }

        // Phase 50: Security Check - mandatory runtime guard (NEVER optimized out)
        Stmt::Check { subject, predicate, is_capability, object, source_text, span } => {
            let subj_name = interner.resolve(*subject);
            let pred_name = interner.resolve(*predicate).to_lowercase();

            let call = if *is_capability {
                let obj_sym = object.expect("capability must have object");
                let obj_word = interner.resolve(obj_sym);

                // Phase 50: Type-based resolution
                // "Check that user can publish the document" -> find variable of type Document
                // First try to find a variable whose type matches the object word
                let obj_name = ctx.find_variable_by_type(obj_word, interner)
                    .unwrap_or_else(|| obj_word.to_string());

                format!("{}.can_{}(&{})", subj_name, pred_name, obj_name)
            } else {
                format!("{}.is_{}()", subj_name, pred_name)
            };

            writeln!(output, "{}if !({}) {{", indent_str, call).unwrap();
            writeln!(output, "{}    logicaffeine_system::panic_with(\"Security Check Failed at line {}: {}\");",
                     indent_str, span.start, source_text).unwrap();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        // Phase 51: P2P Networking - Listen on network address
        Stmt::Listen { address } => {
            let addr_str = codegen_expr_with_async(address, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Pass &str instead of String
            writeln!(output, "{}logicaffeine_system::network::listen(&{}).await.expect(\"Failed to listen\");",
                     indent_str, addr_str).unwrap();
        }

        // Phase 51: P2P Networking - Connect to remote peer
        Stmt::ConnectTo { address } => {
            let addr_str = codegen_expr_with_async(address, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Pass &str instead of String
            writeln!(output, "{}logicaffeine_system::network::connect(&{}).await.expect(\"Failed to connect\");",
                     indent_str, addr_str).unwrap();
        }

        // Phase 51: P2P Networking - Create PeerAgent remote handle
        Stmt::LetPeerAgent { var, address } => {
            let var_name = interner.resolve(*var);
            let addr_str = codegen_expr_with_async(address, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Pass &str instead of String
            writeln!(output, "{}let {} = logicaffeine_system::network::PeerAgent::new(&{}).expect(\"Invalid address\");",
                     indent_str, var_name, addr_str).unwrap();
        }

        // Phase 51: Sleep - supports Duration literals or milliseconds
        Stmt::Sleep { milliseconds } => {
            let expr_str = codegen_expr_with_async(milliseconds, interner, synced_vars, async_functions, ctx.get_variable_types());
            let inferred_type = infer_rust_type_from_expr(milliseconds, interner);

            if inferred_type == "std::time::Duration" {
                // Duration type: use directly (already a std::time::Duration)
                writeln!(output, "{}tokio::time::sleep({}).await;",
                         indent_str, expr_str).unwrap();
            } else {
                // Assume milliseconds (integer) - legacy behavior
                writeln!(output, "{}tokio::time::sleep(std::time::Duration::from_millis({} as u64)).await;",
                         indent_str, expr_str).unwrap();
            }
        }

        // Phase 52/56: Sync CRDT variable on topic
        Stmt::Sync { var, topic } => {
            let var_name = interner.resolve(*var);
            let topic_str = codegen_expr_with_async(topic, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Phase 56: Check if this variable is also mounted
            if let Some(caps) = var_caps.get(var) {
                if caps.mounted {
                    // Both Mount and Sync: use Distributed<T>
                    // Mount statement will handle the Distributed::mount call
                    // Here we just track it as synced
                    synced_vars.insert(*var);
                    return output;  // Skip - Mount will emit Distributed<T>
                }
            }

            // Sync-only: use Synced<T>
            writeln!(
                output,
                "{}let {} = logicaffeine_system::crdt::Synced::new({}, &{}).await;",
                indent_str, var_name, var_name, topic_str
            ).unwrap();
            synced_vars.insert(*var);
        }

        // Phase 53/56: Mount persistent CRDT from journal
        Stmt::Mount { var, path } => {
            let var_name = interner.resolve(*var);
            let path_str = codegen_expr_with_async(path, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Phase 56: Check if this variable is also synced
            if let Some(caps) = var_caps.get(var) {
                if caps.synced {
                    // Both Mount and Sync: use Distributed<T>
                    let topic_str = caps.sync_topic.as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or("\"default\"");
                    writeln!(
                        output,
                        "{}let {} = logicaffeine_system::distributed::Distributed::mount(std::sync::Arc::new(vfs.clone()), &{}, Some({}.to_string())).await.expect(\"Failed to mount\");",
                        indent_str, var_name, path_str, topic_str
                    ).unwrap();
                    synced_vars.insert(*var);
                    return output;
                }
            }

            // Mount-only: use Persistent<T>
            writeln!(
                output,
                "{}let {} = logicaffeine_system::storage::Persistent::mount(&vfs, &{}).await.expect(\"Failed to mount\");",
                indent_str, var_name, path_str
            ).unwrap();
            synced_vars.insert(*var);
        }

        // =====================================================================
        // Phase 54: Go-like Concurrency Codegen
        // =====================================================================

        Stmt::LaunchTask { function, args } => {
            let fn_name = names.ident(*function);
            // Phase 54: When passing a pipe variable, pass the sender (_tx)
            let args_str: Vec<String> = args.iter()
                .map(|a| {
                    if let Expr::Identifier(sym) = a {
                        if pipe_vars.contains(sym) {
                            return format!("{}_tx.clone()", interner.resolve(*sym));
                        }
                    }
                    codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types())
                })
                .collect();
            // Phase 54: Add .await only if the function is async
            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
            writeln!(
                output,
                "{}tokio::spawn(async move {{ {}({}){await_suffix}; }});",
                indent_str, fn_name, args_str.join(", ")
            ).unwrap();
        }

        Stmt::LaunchTaskWithHandle { handle, function, args } => {
            let handle_name = interner.resolve(*handle);
            let fn_name = names.ident(*function);
            // Phase 54: When passing a pipe variable, pass the sender (_tx)
            let args_str: Vec<String> = args.iter()
                .map(|a| {
                    if let Expr::Identifier(sym) = a {
                        if pipe_vars.contains(sym) {
                            return format!("{}_tx.clone()", interner.resolve(*sym));
                        }
                    }
                    codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types())
                })
                .collect();
            // Phase 54: Add .await only if the function is async
            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
            writeln!(
                output,
                "{}let {} = tokio::spawn(async move {{ {}({}){await_suffix} }});",
                indent_str, handle_name, fn_name, args_str.join(", ")
            ).unwrap();
        }

        Stmt::CreatePipe { var, element_type, capacity } => {
            let var_name = interner.resolve(*var);
            let type_name = interner.resolve(*element_type);
            let cap = capacity.unwrap_or(32);
            // Map LOGOS types to Rust types
            let rust_type = match type_name {
                "Int" => "i64",
                "Nat" => "u64",
                "Text" => "String",
                "Bool" => "bool",
                _ => type_name,
            };
            writeln!(
                output,
                "{}let ({}_tx, mut {}_rx) = tokio::sync::mpsc::channel::<{}>({});",
                indent_str, var_name, var_name, rust_type, cap
            ).unwrap();
        }

        Stmt::SendPipe { value, pipe } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 54: Check if pipe is a local declaration (has _tx suffix) or parameter (no suffix)
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            if is_local_pipe {
                writeln!(
                    output,
                    "{}{}_tx.send({}).await.expect(\"pipe send failed\");",
                    indent_str, pipe_str, val_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}{}.send({}).await.expect(\"pipe send failed\");",
                    indent_str, pipe_str, val_str
                ).unwrap();
            }
        }

        Stmt::ReceivePipe { var, pipe } => {
            let var_name = interner.resolve(*var);
            let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 54: Check if pipe is a local declaration (has _rx suffix) or parameter (no suffix)
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            if is_local_pipe {
                writeln!(
                    output,
                    "{}let {} = {}_rx.recv().await.expect(\"pipe closed\");",
                    indent_str, var_name, pipe_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}let {} = {}.recv().await.expect(\"pipe closed\");",
                    indent_str, var_name, pipe_str
                ).unwrap();
            }
        }

        Stmt::TrySendPipe { value, pipe, result } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 54: Check if pipe is a local declaration
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_tx" } else { "" };
            if let Some(res) = result {
                let res_name = interner.resolve(*res);
                writeln!(
                    output,
                    "{}let {} = {}{}.try_send({}).is_ok();",
                    indent_str, res_name, pipe_str, suffix, val_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}let _ = {}{}.try_send({});",
                    indent_str, pipe_str, suffix, val_str
                ).unwrap();
            }
        }

        Stmt::TryReceivePipe { var, pipe } => {
            let var_name = interner.resolve(*var);
            let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 54: Check if pipe is a local declaration
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_rx" } else { "" };
            writeln!(
                output,
                "{}let {} = {}{}.try_recv().ok();",
                indent_str, var_name, pipe_str, suffix
            ).unwrap();
        }

        Stmt::StopTask { handle } => {
            let handle_str = codegen_expr_with_async(handle, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}.abort();", indent_str, handle_str).unwrap();
        }

        Stmt::Select { branches } => {
            use crate::ast::stmt::SelectBranch;

            writeln!(output, "{}tokio::select! {{", indent_str).unwrap();
            for branch in branches {
                match branch {
                    SelectBranch::Receive { var, pipe, body } => {
                        let var_name = interner.resolve(*var);
                        let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
                        // Check if pipe is a local declaration (has _rx suffix) or a parameter (no suffix)
                        let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                            pipe_vars.contains(sym)
                        } else {
                            false
                        };
                        let suffix = if is_local_pipe { "_rx" } else { "" };
                        writeln!(
                            output,
                            "{}    {} = {}{}.recv() => {{",
                            indent_str, var_name, pipe_str, suffix
                        ).unwrap();
                        writeln!(
                            output,
                            "{}        if let Some({}) = {} {{",
                            indent_str, var_name, var_name
                        ).unwrap();
                        for stmt in *body {
                            let stmt_code = codegen_stmt(stmt, interner, indent + 3, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env);
                            write!(output, "{}", stmt_code).unwrap();
                        }
                        writeln!(output, "{}        }}", indent_str).unwrap();
                        writeln!(output, "{}    }}", indent_str).unwrap();
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        let ms_str = codegen_expr_with_async(milliseconds, interner, synced_vars, async_functions, ctx.get_variable_types());
                        // Convert seconds to milliseconds if the value looks like seconds
                        writeln!(
                            output,
                            "{}    _ = tokio::time::sleep(std::time::Duration::from_secs({} as u64)) => {{",
                            indent_str, ms_str
                        ).unwrap();
                        for stmt in *body {
                            let stmt_code = codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env);
                            write!(output, "{}", stmt_code).unwrap();
                        }
                        writeln!(output, "{}    }}", indent_str).unwrap();
                    }
                }
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Give { object, recipient } => {
            // Move semantics: pass ownership without borrowing
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            let recv_str = codegen_expr_with_async(recipient, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}({});", indent_str, recv_str, obj_str).unwrap();
        }

        Stmt::Show { object, recipient } => {
            // Optimization: Show with InterpolatedString → println! directly
            if let Expr::InterpolatedString(parts) = object {
                let recv_name = if let Expr::Identifier(sym) = recipient {
                    interner.resolve(*sym).to_string()
                } else {
                    String::new()
                };
                if recv_name == "show" {
                    // Emit println! directly — no intermediate String allocation
                    let mut fmt_str = String::new();
                    let mut args = Vec::new();
                    for part in parts {
                        match part {
                            crate::ast::stmt::StringPart::Literal(sym) => {
                                let text = interner.resolve(*sym);
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
                            crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
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
                                let arg_str = codegen_expr_with_async_and_strings(value, interner, synced_vars, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
                                if needs_float_cast {
                                    args.push(format!("{} as f64", arg_str));
                                } else {
                                    args.push(arg_str);
                                }
                            }
                        }
                    }
                    writeln!(output, "{}println!(\"{}\"{});", indent_str, fmt_str,
                        args.iter().map(|a| format!(", {}", a)).collect::<String>()).unwrap();
                } else {
                    let obj_str = codegen_expr_with_async_and_strings(object, interner, synced_vars, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
                    let recv_str = codegen_expr_with_async(recipient, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}{}(&{});", indent_str, recv_str, obj_str).unwrap();
                }
            } else {
                // Borrow semantics: pass immutable reference
                // Use string_vars for proper concatenation of string variables
                let obj_str = codegen_expr_with_async_and_strings(object, interner, synced_vars, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
                let recv_str = codegen_expr_with_async(recipient, interner, synced_vars, async_functions, ctx.get_variable_types());
                writeln!(output, "{}{}(&{});", indent_str, recv_str, obj_str).unwrap();
            }
        }

        Stmt::SetField { object, field, value } => {
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            let field_name = interner.resolve(*field);
            let value_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());

            // Phase 49: Check if this field is an LWWRegister or MVRegister
            // LWW needs .set(value, timestamp), MV needs .set(value)
            let is_lww = lww_fields.iter().any(|(_, f)| f == field_name);
            let is_mv = mv_fields.iter().any(|(_, f)| f == field_name);
            if is_lww {
                // LWWRegister needs a timestamp - use current system time in microseconds
                writeln!(output, "{}{}.{}.set({}, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as u64);", indent_str, obj_str, field_name, value_str).unwrap();
            } else if is_mv {
                // MVRegister just needs the value
                writeln!(output, "{}{}.{}.set({});", indent_str, obj_str, field_name, value_str).unwrap();
            } else {
                writeln!(output, "{}{}.{} = {};", indent_str, obj_str, field_name, value_str).unwrap();
            }
        }

        Stmt::StructDef { .. } => {
            // Struct definitions are handled in codegen_program, not here
        }

        Stmt::FunctionDef { .. } => {
            // Function definitions are handled in codegen_program, not here
        }

        Stmt::Inspect { target, arms, .. } => {
            let target_str = codegen_expr_with_async(target, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Phase 102: Track which bindings come from boxed fields for inner Inspects
            // Use NAMES (strings) not symbols, because parser may create different symbols
            // for the same identifier in different syntactic positions.
            let mut inner_boxed_binding_names: HashSet<String> = HashSet::new();

            writeln!(output, "{}match {} {{", indent_str, target_str).unwrap();

            for arm in arms {
                if let Some(variant) = arm.variant {
                    let variant_name = interner.resolve(variant);
                    // Get the enum name from the arm, or fallback to just variant name
                    let enum_name_str = arm.enum_name.map(|e| interner.resolve(e));
                    let enum_prefix = enum_name_str
                        .map(|e| format!("{}::", e))
                        .unwrap_or_default();

                    if arm.bindings.is_empty() {
                        // Unit variant pattern
                        writeln!(output, "{}    {}{} => {{", indent_str, enum_prefix, variant_name).unwrap();
                    } else {
                        // Pattern with bindings
                        // Phase 102: Check which bindings are from boxed fields
                        let bindings_str: Vec<String> = arm.bindings.iter()
                            .map(|(field, binding)| {
                                let field_name = interner.resolve(*field);
                                let binding_name = interner.resolve(*binding);

                                // Check if this field is boxed
                                if let Some(enum_name) = enum_name_str {
                                    let key = (enum_name.to_string(), variant_name.to_string(), field_name.to_string());
                                    if boxed_fields.contains(&key) {
                                        inner_boxed_binding_names.insert(binding_name.to_string());
                                    }
                                }

                                if field_name == binding_name {
                                    field_name.to_string()
                                } else {
                                    format!("{}: {}", field_name, binding_name)
                                }
                            })
                            .collect();
                        writeln!(output, "{}    {}{} {{ {} }} => {{", indent_str, enum_prefix, variant_name, bindings_str.join(", ")).unwrap();
                    }
                } else {
                    // Otherwise (wildcard) pattern
                    writeln!(output, "{}    _ => {{", indent_str).unwrap();
                }

                ctx.push_scope();

                // Generate explicit dereferences for boxed bindings at the start of the arm
                // This makes them usable as regular values in the rest of the body
                for binding_name in &inner_boxed_binding_names {
                    writeln!(output, "{}        let {} = (*{}).clone();", indent_str, binding_name, binding_name).unwrap();
                }

                for stmt in arm.body {
                    // Phase 102: Handle inner Inspect statements with boxed bindings
                    // Note: Since we now dereference boxed bindings at the start of the arm,
                    // inner matches don't need the `*` dereference operator.
                    let inner_stmt_code = if let Stmt::Inspect { target: inner_target, .. } = stmt {
                        // Check if the inner target is a boxed binding (already dereferenced above)
                        // Use name comparison since symbols may differ between binding and reference
                        if let Expr::Identifier(sym) = inner_target {
                            let target_name = interner.resolve(*sym);
                            if inner_boxed_binding_names.contains(target_name) {
                                // Generate match (binding was already dereferenced at arm start)
                                let mut inner_output = String::new();
                                writeln!(inner_output, "{}match {} {{", "    ".repeat(indent + 2), target_name).unwrap();

                                if let Stmt::Inspect { arms: inner_arms, .. } = stmt {
                                    for inner_arm in inner_arms.iter() {
                                        if let Some(v) = inner_arm.variant {
                                            let v_name = interner.resolve(v);
                                            let inner_enum_prefix = inner_arm.enum_name
                                                .map(|e| format!("{}::", interner.resolve(e)))
                                                .unwrap_or_default();

                                            if inner_arm.bindings.is_empty() {
                                                writeln!(inner_output, "{}    {}{} => {{", "    ".repeat(indent + 2), inner_enum_prefix, v_name).unwrap();
                                            } else {
                                                let bindings: Vec<String> = inner_arm.bindings.iter()
                                                    .map(|(f, b)| {
                                                        let fn_name = interner.resolve(*f);
                                                        let bn_name = interner.resolve(*b);
                                                        if fn_name == bn_name { fn_name.to_string() }
                                                        else { format!("{}: {}", fn_name, bn_name) }
                                                    })
                                                    .collect();
                                                writeln!(inner_output, "{}    {}{} {{ {} }} => {{", "    ".repeat(indent + 2), inner_enum_prefix, v_name, bindings.join(", ")).unwrap();
                                            }
                                        } else {
                                            writeln!(inner_output, "{}    _ => {{", "    ".repeat(indent + 2)).unwrap();
                                        }

                                        ctx.push_scope();
                                        for inner_stmt in inner_arm.body {
                                            inner_output.push_str(&codegen_stmt(inner_stmt, interner, indent + 4, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                                        }
                                        ctx.pop_scope();
                                        writeln!(inner_output, "{}    }}", "    ".repeat(indent + 2)).unwrap();
                                    }
                                }
                                writeln!(inner_output, "{}}}", "    ".repeat(indent + 2)).unwrap();
                                inner_output
                            } else {
                                codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env)
                            }
                        } else {
                            codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env)
                        }
                    } else {
                        codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env)
                    };
                    output.push_str(&inner_stmt_code);
                }
                ctx.pop_scope();
                writeln!(output, "{}    }}", indent_str).unwrap();
            }

            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Push { value, collection } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}.push({});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::Pop { collection, into } => {
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            match into {
                Some(var) => {
                    let var_name = names.ident(*var);
                    // Unwrap the Option returned by pop() - panics if empty
                    writeln!(output, "{}let {} = {}.pop().expect(\"Pop from empty collection\");", indent_str, var_name, coll_str).unwrap();
                }
                None => {
                    writeln!(output, "{}{}.pop();", indent_str, coll_str).unwrap();
                }
            }
        }

        Stmt::Add { value, collection } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}.insert({});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::Remove { value, collection } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}.remove(&{});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::SetIndex { collection, index, value } => {
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            let value_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());

            // Direct indexing for known collection types (avoids trait dispatch)
            let known_type = if let Expr::Identifier(sym) = collection {
                ctx.get_variable_types().get(sym).map(|s| s.as_str())
            } else {
                None
            };

            match known_type {
                Some(t) if t.starts_with("Vec") => {
                    // Peephole: simplify (x + 1) - 1 → x for 1-based indexing
                    let index_part = if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = index {
                        if matches!(right, Expr::Literal(Literal::Number(1))) {
                            let inner = codegen_expr_with_async(left, interner, synced_vars, async_functions, ctx.get_variable_types());
                            format!("({}) as usize", inner)
                        } else if matches!(left, Expr::Literal(Literal::Number(1))) {
                            let inner = codegen_expr_with_async(right, interner, synced_vars, async_functions, ctx.get_variable_types());
                            format!("({}) as usize", inner)
                        } else {
                            let index_str = codegen_expr_with_async(index, interner, synced_vars, async_functions, ctx.get_variable_types());
                            format!("({} - 1) as usize", index_str)
                        }
                    } else {
                        let index_str = codegen_expr_with_async(index, interner, synced_vars, async_functions, ctx.get_variable_types());
                        format!("({} - 1) as usize", index_str)
                    };
                    // Evaluate value first if it references the same collection (borrow safety)
                    if value_str.contains(&coll_str) {
                        writeln!(output, "{}let __set_tmp = {};", indent_str, value_str).unwrap();
                        writeln!(output, "{}{}[{}] = __set_tmp;", indent_str, coll_str, index_part).unwrap();
                    } else {
                        writeln!(output, "{}{}[{}] = {};", indent_str, coll_str, index_part, value_str).unwrap();
                    }
                }
                Some(t) if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") => {
                    let index_str = codegen_expr_with_async(index, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}{}.insert({}, {});", indent_str, coll_str, index_str, value_str).unwrap();
                }
                _ => {
                    let index_str = codegen_expr_with_async(index, interner, synced_vars, async_functions, ctx.get_variable_types());
                    // Fallback: polymorphic indexing via trait
                    if value_str.contains("logos_get") && value_str.contains(&coll_str) {
                        writeln!(output, "{}let __set_tmp = {};", indent_str, value_str).unwrap();
                        writeln!(output, "{}LogosIndexMut::logos_set(&mut {}, {}, __set_tmp);", indent_str, coll_str, index_str).unwrap();
                    } else {
                        writeln!(output, "{}LogosIndexMut::logos_set(&mut {}, {}, {});", indent_str, coll_str, index_str, value_str).unwrap();
                    }
                }
            }
        }

        // Phase 8.5: Zone (memory arena) block
        Stmt::Zone { name, capacity, source_file, body } => {
            let zone_name = interner.resolve(*name);

            // Generate zone creation based on type
            if let Some(path_sym) = source_file {
                // Memory-mapped file zone
                let path = interner.resolve(*path_sym);
                writeln!(
                    output,
                    "{}let {} = logicaffeine_system::memory::Zone::new_mapped(\"{}\").expect(\"Failed to map file\");",
                    indent_str, zone_name, path
                ).unwrap();
            } else {
                // Heap arena zone
                let cap = capacity.unwrap_or(4096); // Default 4KB
                writeln!(
                    output,
                    "{}let {} = logicaffeine_system::memory::Zone::new_heap({});",
                    indent_str, zone_name, cap
                ).unwrap();
            }

            // Open block scope
            writeln!(output, "{}{{", indent_str).unwrap();
            ctx.push_scope();

            // Generate body statements
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
            }

            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        // Phase 9: Concurrent execution block (async, I/O-bound)
        // Generates tokio::join! for concurrent task execution
        // Phase 51: Variables used across multiple tasks are cloned to avoid move issues
        Stmt::Concurrent { tasks } => {
            // Collect Let statements to generate tuple destructuring
            let let_bindings: Vec<_> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(interner.resolve(*var).to_string())
                } else {
                    None
                }
            }).collect();

            // Collect variables DEFINED in this block (to exclude from cloning)
            let defined_vars: HashSet<Symbol> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(*var)
                } else {
                    None
                }
            }).collect();

            // Check if there are intra-block dependencies (a later task uses a var from earlier task)
            // If so, fall back to sequential execution
            let mut has_intra_dependency = false;
            let mut seen_defs: HashSet<Symbol> = HashSet::new();
            for s in *tasks {
                // Check if this task uses any variable defined by previous tasks in this block
                let mut used_in_task: HashSet<Symbol> = HashSet::new();
                collect_stmt_identifiers(s, &mut used_in_task);
                for used_var in &used_in_task {
                    if seen_defs.contains(used_var) {
                        has_intra_dependency = true;
                        break;
                    }
                }
                // Track variables defined by this task
                if let Stmt::Let { var, .. } = s {
                    seen_defs.insert(*var);
                }
                if has_intra_dependency {
                    break;
                }
            }

            // Collect ALL variables used in task expressions (not just Call args)
            // Exclude variables defined within this block
            let mut used_syms: HashSet<Symbol> = HashSet::new();
            for s in *tasks {
                collect_stmt_identifiers(s, &mut used_syms);
            }
            // Remove variables that are defined in this block
            for def_var in &defined_vars {
                used_syms.remove(def_var);
            }
            let used_vars: HashSet<String> = used_syms.iter()
                .map(|sym| interner.resolve(*sym).to_string())
                .collect();

            // If there are intra-block dependencies, execute sequentially
            if has_intra_dependency {
                // Generate sequential Let bindings
                for stmt in *tasks {
                    output.push_str(&codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                }
            } else {
                // Generate concurrent execution with tokio::join!
                if !let_bindings.is_empty() {
                    // Generate tuple destructuring for concurrent Let bindings
                    writeln!(output, "{}let ({}) = tokio::join!(", indent_str, let_bindings.join(", ")).unwrap();
                } else {
                    writeln!(output, "{}tokio::join!(", indent_str).unwrap();
                }

                for (i, stmt) in tasks.iter().enumerate() {
                    // For Let statements, generate only the VALUE so the async block returns it
                    // For Call statements, generate the call with .await
                    let inner_code = match stmt {
                        Stmt::Let { value, .. } => {
                            // Return the value expression directly (not "let x = value;")
                            // Phase 54+: Use codegen_expr_with_async to handle all nested async calls
                            codegen_expr_with_async(value, interner, synced_vars, async_functions, ctx.get_variable_types())
                        }
                        Stmt::Call { function, args } => {
                            let func_name = interner.resolve(*function);
                            let args_str: Vec<String> = args.iter()
                                .map(|a| codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types()))
                                .collect();
                            // Only add .await for async functions
                            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
                            format!("{}({}){}", func_name, args_str.join(", "), await_suffix)
                        }
                        _ => {
                            // Fallback for other statement types
                            let inner = codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env);
                            inner.trim().to_string()
                        }
                    };

                    // For tasks that use shared variables, wrap in a block that clones them
                    if !used_vars.is_empty() && i < tasks.len() - 1 {
                        // Clone variables for all tasks except the last one
                        let clones: Vec<String> = used_vars.iter()
                            .map(|v| format!("let {} = {}.clone();", v, v))
                            .collect();
                        write!(output, "{}    {{ {} async move {{ {} }} }}",
                               indent_str, clones.join(" "), inner_code).unwrap();
                    } else {
                        // Last task can use original variables
                        write!(output, "{}    async {{ {} }}", indent_str, inner_code).unwrap();
                    }

                    if i < tasks.len() - 1 {
                        writeln!(output, ",").unwrap();
                    } else {
                        writeln!(output).unwrap();
                    }
                }

                writeln!(output, "{});", indent_str).unwrap();
            }
        }

        // Phase 9: Parallel execution block (CPU-bound)
        // Generates rayon::join for two tasks, or thread::spawn for 3+ tasks
        Stmt::Parallel { tasks } => {
            // Collect Let statements to generate tuple destructuring
            let let_bindings: Vec<_> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(interner.resolve(*var).to_string())
                } else {
                    None
                }
            }).collect();

            if tasks.len() == 2 {
                // Use rayon::join for exactly 2 tasks
                if !let_bindings.is_empty() {
                    writeln!(output, "{}let ({}) = rayon::join(", indent_str, let_bindings.join(", ")).unwrap();
                } else {
                    writeln!(output, "{}rayon::join(", indent_str).unwrap();
                }

                for (i, stmt) in tasks.iter().enumerate() {
                    // For Let statements, generate only the VALUE so the closure returns it
                    let inner_code = match stmt {
                        Stmt::Let { value, .. } => {
                            // Return the value expression directly (not "let x = value;")
                            codegen_expr(value, interner, synced_vars)
                        }
                        Stmt::Call { function, args } => {
                            let func_name = interner.resolve(*function);
                            let args_str: Vec<String> = args.iter()
                                .map(|a| codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types()))
                                .collect();
                            format!("{}({})", func_name, args_str.join(", "))
                        }
                        _ => {
                            // Fallback for other statement types
                            let inner = codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env);
                            inner.trim().to_string()
                        }
                    };
                    write!(output, "{}    || {{ {} }}", indent_str, inner_code).unwrap();
                    if i == 0 {
                        writeln!(output, ",").unwrap();
                    } else {
                        writeln!(output).unwrap();
                    }
                }
                writeln!(output, "{});", indent_str).unwrap();
            } else {
                // For 3+ tasks, use thread::spawn pattern
                writeln!(output, "{}{{", indent_str).unwrap();
                writeln!(output, "{}    let handles: Vec<_> = vec![", indent_str).unwrap();
                for stmt in *tasks {
                    // For Let statements, generate only the VALUE so the closure returns it
                    let inner_code = match stmt {
                        Stmt::Let { value, .. } => {
                            codegen_expr(value, interner, synced_vars)
                        }
                        Stmt::Call { function, args } => {
                            let func_name = interner.resolve(*function);
                            let args_str: Vec<String> = args.iter()
                                .map(|a| codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types()))
                                .collect();
                            format!("{}({})", func_name, args_str.join(", "))
                        }
                        _ => {
                            let inner = codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env);
                            inner.trim().to_string()
                        }
                    };
                    writeln!(output, "{}        std::thread::spawn(move || {{ {} }}),",
                             indent_str, inner_code).unwrap();
                }
                writeln!(output, "{}    ];", indent_str).unwrap();
                writeln!(output, "{}    for h in handles {{ h.join().unwrap(); }}", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
            }
        }

        // Phase 10: Read from console or file
        // Phase 53: File reads now use async VFS
        Stmt::ReadFrom { var, source } => {
            let var_name = interner.resolve(*var);
            match source {
                ReadSource::Console => {
                    writeln!(output, "{}let {} = logicaffeine_system::io::read_line();", indent_str, var_name).unwrap();
                }
                ReadSource::File(path_expr) => {
                    let path_str = codegen_expr_with_async(path_expr, interner, synced_vars, async_functions, ctx.get_variable_types());
                    // Phase 53: Use VFS with async
                    writeln!(
                        output,
                        "{}let {} = vfs.read_to_string(&{}).await.expect(\"Failed to read file\");",
                        indent_str, var_name, path_str
                    ).unwrap();
                }
            }
        }

        // Phase 10: Write to file
        // Phase 53: File writes now use async VFS
        Stmt::WriteFile { content, path } => {
            let content_str = codegen_expr_with_async(content, interner, synced_vars, async_functions, ctx.get_variable_types());
            let path_str = codegen_expr_with_async(path, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 53: Use VFS with async
            writeln!(
                output,
                "{}vfs.write(&{}, {}.as_bytes()).await.expect(\"Failed to write file\");",
                indent_str, path_str, content_str
            ).unwrap();
        }

        // Phase 46: Spawn an agent
        Stmt::Spawn { agent_type, name } => {
            let type_name = interner.resolve(*agent_type);
            let agent_name = interner.resolve(*name);
            // Generate agent spawn with tokio channel
            writeln!(
                output,
                "{}let {} = tokio::spawn(async move {{ /* {} agent loop */ }});",
                indent_str, agent_name, type_name
            ).unwrap();
        }

        // Phase 46: Send message to agent
        Stmt::SendMessage { message, destination } => {
            let msg_str = codegen_expr_with_async(message, interner, synced_vars, async_functions, ctx.get_variable_types());
            let dest_str = codegen_expr_with_async(destination, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.send({}).await.expect(\"Failed to send message\");",
                indent_str, dest_str, msg_str
            ).unwrap();
        }

        // Phase 46: Await response from agent
        Stmt::AwaitMessage { source, into } => {
            let src_str = codegen_expr_with_async(source, interner, synced_vars, async_functions, ctx.get_variable_types());
            let var_name = interner.resolve(*into);
            writeln!(
                output,
                "{}let {} = {}.recv().await.expect(\"Failed to receive message\");",
                indent_str, var_name, src_str
            ).unwrap();
        }

        // Phase 49: Merge CRDT state
        Stmt::MergeCrdt { source, target } => {
            let src_str = codegen_expr_with_async(source, interner, synced_vars, async_functions, ctx.get_variable_types());
            let tgt_str = codegen_expr_with_async(target, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.merge(&{});",
                indent_str, tgt_str, src_str
            ).unwrap();
        }

        // Phase 49: Increment GCounter
        // Phase 52: If object is synced, wrap in .mutate() for auto-publish
        Stmt::IncreaseCrdt { object, field, amount } => {
            let field_name = interner.resolve(*field);
            let amount_str = codegen_expr_with_async(amount, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Check if the root object is synced
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    // Synced: use .mutate() for auto-publish
                    let obj_name = interner.resolve(sym);
                    writeln!(
                        output,
                        "{}{}.mutate(|inner| inner.{}.increment({} as u64)).await;",
                        indent_str, obj_name, field_name, amount_str
                    ).unwrap();
                    return output;
                }
            }

            // Not synced: direct access
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.{}.increment({} as u64);",
                indent_str, obj_str, field_name, amount_str
            ).unwrap();
        }

        // Phase 49b: Decrement PNCounter
        Stmt::DecreaseCrdt { object, field, amount } => {
            let field_name = interner.resolve(*field);
            let amount_str = codegen_expr_with_async(amount, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Check if the root object is synced
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    // Synced: use .mutate() for auto-publish
                    let obj_name = interner.resolve(sym);
                    writeln!(
                        output,
                        "{}{}.mutate(|inner| inner.{}.decrement({} as u64)).await;",
                        indent_str, obj_name, field_name, amount_str
                    ).unwrap();
                    return output;
                }
            }

            // Not synced: direct access
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.{}.decrement({} as u64);",
                indent_str, obj_str, field_name, amount_str
            ).unwrap();
        }

        // Phase 49b: Append to SharedSequence (RGA)
        Stmt::AppendToSequence { sequence, value } => {
            let seq_str = codegen_expr_with_async(sequence, interner, synced_vars, async_functions, ctx.get_variable_types());
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.append({});",
                indent_str, seq_str, val_str
            ).unwrap();
        }

        // Phase 49b: Resolve MVRegister conflicts
        Stmt::ResolveConflict { object, field, value } => {
            let field_name = interner.resolve(*field);
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.{}.resolve({});",
                indent_str, obj_str, field_name, val_str
            ).unwrap();
        }

        // Escape hatch: emit raw foreign code wrapped in braces for scope isolation
        Stmt::Escape { code, .. } => {
            let raw_code = interner.resolve(*code);
            write!(output, "{}{{\n", indent_str).unwrap();
            for line in raw_code.lines() {
                write!(output, "{}    {}\n", indent_str, line).unwrap();
            }
            write!(output, "{}}}\n", indent_str).unwrap();
        }

        // Dependencies are metadata; no Rust code emitted.
        Stmt::Require { .. } => {}

        // Phase 63: Theorems are verified at compile-time, no runtime code generated
        Stmt::Theorem(_) => {
            // Theorems don't generate runtime code - they're processed separately
            // by compile_theorem() at the meta-level
        }
    }

    output
}

/// Phase 52: Extract the root identifier from an expression.
/// For `x.field.subfield`, returns `x`.
pub(crate) fn get_root_identifier(expr: &Expr) -> Option<Symbol> {
    match expr {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => get_root_identifier(object),
        _ => None,
    }
}

/// Check if a type string represents a Copy type (no .clone() needed).
/// Delegates to `LogosType::is_copy()` — single source of truth.
pub(crate) fn is_copy_type(ty: &str) -> bool {
    crate::analysis::types::LogosType::from_rust_type_str(ty).is_copy()
}

/// Check if a Vec<T> type has a Copy element type.
/// Delegates to `LogosType::element_type().is_copy()` — single source of truth.
pub(crate) fn has_copy_element_type(vec_type: &str) -> bool {
    crate::analysis::types::LogosType::from_rust_type_str(vec_type)
        .element_type()
        .map_or(false, |e| e.is_copy())
}

/// Check if a HashMap<K, V> type has a Copy value type.
/// Delegates to `LogosType::value_type().is_copy()` — single source of truth.
pub(crate) fn has_copy_value_type(map_type: &str) -> bool {
    crate::analysis::types::LogosType::from_rust_type_str(map_type)
        .value_type()
        .map_or(false, |v| v.is_copy())
}

