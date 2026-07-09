use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::ast::stmt::{BinaryOpKind, Expr, Stmt, TypeExpr};
use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::intern::{Interner, Symbol};

use super::context::{RefinementContext, VariableCapabilities};
use super::{codegen_stmt, codegen_expr_with_async};
use super::types::codegen_type_expr;
use super::detection::{collect_mutable_vars, expr_contains_self_call};
use crate::tail_call::{detect_accumulator_pattern, AccumulatorInfo, NonRecSide};

// =============================================================================
// Tail Call Elimination (TCE) Detection
// =============================================================================

pub(super) fn expr_is_self_call(func_name: Symbol, expr: &Expr) -> bool {
    matches!(expr, Expr::Call { function, .. } if *function == func_name)
}

pub(super) fn has_tail_call_in_stmt(func_name: Symbol, stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { value: Some(expr) } => {
            if expr_is_self_call(func_name, expr) {
                return true;
            }
            // Check for nested self-call pattern: f(a, f(b, c))
            // The outer call is in tail position even if an arg is also a self-call
            if let Expr::Call { function, args } = expr {
                if *function == func_name {
                    return true;
                }
                // The outer is a self-call with a nested self-call arg — still tail position
                let _ = args;
            }
            false
        }
        Stmt::If { then_block, else_block, .. } => {
            let then_tail = then_block.last()
                .map_or(false, |s| has_tail_call_in_stmt(func_name, s));
            let else_tail = else_block
                .and_then(|block| block.last())
                .map_or(false, |s| has_tail_call_in_stmt(func_name, s));
            then_tail || else_tail
        }
        _ => false,
    }
}

pub(crate) fn is_tail_recursive(func_name: Symbol, body: &[Stmt]) -> bool {
    body.iter().any(|s| has_tail_call_in_stmt(func_name, s))
}

/// Does the body have a top-level `Set/Let x to self(args); Return x` pair — a
/// self-tail-call spelled across two statements? Detected only at the function
/// body's top level (never nested in an `If`), matching the bytecode VM and the
/// tree-walker exactly so all three tiers loop the same shapes.
pub(super) fn body_has_top_level_tail_pair(
    func_name: Symbol,
    body: &[Stmt],
    param_count: usize,
) -> bool {
    body.windows(2).any(|w| {
        crate::tail_call::tail_pair_args(&w[0], &w[1], func_name, param_count).is_some()
    })
}

/// Emit the TCE loop-back for a self-tail-call: evaluate the call's args into
/// temporaries (so reassigning one param can't perturb another's source), assign
/// them to the parameters, and `continue` the function's wrapping `loop`. Shared
/// by the direct-`Return` case and the `Set/Let x; Return x` pair so they can't
/// drift.
pub(super) fn codegen_tce_loopback<'a>(
    args: &[&'a Expr<'a>],
    param_names: &[Symbol],
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
) -> String {
    let indent_str = "    ".repeat(indent);
    let mut output = String::new();
    writeln!(output, "{}{{", indent_str).unwrap();
    for (i, arg) in args.iter().enumerate() {
        let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
        writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
    }
    for (i, param_sym) in param_names.iter().enumerate() {
        let param_name = interner.resolve(*param_sym);
        writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
    }
    writeln!(output, "{}    continue;", indent_str).unwrap();
    writeln!(output, "{}}}", indent_str).unwrap();
    output
}

// Accumulator-introduction DETECTION (`detect_accumulator_pattern`,
// `AccumulatorInfo`, `NonRecSide`) lives in `crate::tail_call` — one definition
// shared with the VM/tree-walker accumulator→loop rewrite so the AOT and the
// interpreters can't disagree on which functions strength-reduce. The AOT's
// codegen-time emitter (`codegen_stmt_acc` below) consumes that `AccumulatorInfo`.

// =============================================================================
// Accumulator Introduction — Statement Emitter
// =============================================================================

pub(super) fn codegen_stmt_acc<'a>(
    stmt: &Stmt<'a>,
    func_name: Symbol,
    param_names: &[Symbol],
    acc_info: &AccumulatorInfo,
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
) -> String {
    let indent_str = "    ".repeat(indent);
    let op_str = match acc_info.op {
        BinaryOpKind::Add => "+",
        BinaryOpKind::Multiply => "*",
        _ => unreachable!(),
    };

    match stmt {
        // Recursive return: BinaryOp(op, self_call, non_rec) or swapped
        Stmt::Return { value: Some(expr) } if expr_contains_self_call(func_name, expr) => {
            if let Expr::BinaryOp { left, right, .. } = expr {
                let (call_expr, non_rec_expr) = match acc_info.non_recursive_side {
                    NonRecSide::Left => (right, left),
                    NonRecSide::Right => (left, right),
                };
                // Extract args from the self-call
                if let Expr::Call { args, .. } = call_expr {
                    let mut output = String::new();
                    writeln!(output, "{}{{", indent_str).unwrap();
                    let non_rec_str = codegen_expr_with_async(non_rec_expr, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}    let __acc_expr = {};", indent_str, non_rec_str).unwrap();
                    writeln!(output, "{}    __acc = __acc {} __acc_expr;", indent_str, op_str).unwrap();
                    // Evaluate args into temporaries
                    for (i, arg) in args.iter().enumerate() {
                        let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                        writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                    }
                    // Assign temporaries to params
                    for (i, param_sym) in param_names.iter().enumerate() {
                        let param_name = interner.resolve(*param_sym);
                        writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                    }
                    writeln!(output, "{}    continue;", indent_str).unwrap();
                    writeln!(output, "{}}}", indent_str).unwrap();
                    return output;
                }
            }
            // Fallback
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env)
        }

        // Base return: no self-call
        Stmt::Return { value: Some(expr) } => {
            let val_str = codegen_expr_with_async(expr, interner, synced_vars, async_functions, ctx.get_variable_types());
            format!("{}return __acc {} {};\n", indent_str, op_str, val_str)
        }

        Stmt::Return { value: None } => {
            format!("{}return __acc;\n", indent_str)
        }

        // If: recurse into branches
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            let mut output = String::new();
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for s in *then_block {
                output.push_str(&codegen_stmt_acc(s, func_name, param_names, acc_info, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for s in *else_stmts {
                    output.push_str(&codegen_stmt_acc(s, func_name, param_names, acc_info, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
            output
        }

        // Everything else: delegate
        _ => codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env),
    }
}

// =============================================================================
// Mutual Tail Call Optimization — Detection
// =============================================================================

pub(super) fn find_tail_call_targets(func_name: Symbol, body: &[Stmt]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    for stmt in body {
        collect_tail_targets(func_name, stmt, &mut targets);
    }
    targets
}

pub(super) fn collect_tail_targets(func_name: Symbol, stmt: &Stmt, targets: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Return { value: Some(Expr::Call { function, .. }) } => {
            if *function != func_name {
                targets.insert(*function);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            if let Some(last) = then_block.last() {
                collect_tail_targets(func_name, last, targets);
            }
            if let Some(else_stmts) = else_block {
                if let Some(last) = else_stmts.last() {
                    collect_tail_targets(func_name, last, targets);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn detect_mutual_tce_pairs<'a>(stmts: &'a [Stmt<'a>], interner: &Interner) -> Vec<(Symbol, Symbol)> {
    // Collect function definitions
    let mut func_defs: HashMap<Symbol, (&[(Symbol, &TypeExpr)], &[Stmt], Option<&TypeExpr>, bool, bool, bool)> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, is_native, is_exported, .. } = stmt {
            let is_async_fn = false; // Will be checked properly later
            func_defs.insert(*name, (params, body, return_type.as_ref().copied(), *is_native, *is_exported, is_async_fn));
        }
    }

    // Build tail-call graph
    let mut tail_targets: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
    for (name, (_, body, _, _, _, _)) in &func_defs {
        tail_targets.insert(*name, find_tail_call_targets(*name, body));
    }

    // Find mutually tail-calling pairs
    let mut pairs = Vec::new();
    let mut used = HashSet::new();
    let names: Vec<Symbol> = func_defs.keys().copied().collect();

    for i in 0..names.len() {
        for j in (i + 1)..names.len() {
            let a = names[i];
            let b = names[j];
            if used.contains(&a) || used.contains(&b) {
                continue;
            }

            let a_targets = tail_targets.get(&a).cloned().unwrap_or_default();
            let b_targets = tail_targets.get(&b).cloned().unwrap_or_default();

            // Both must tail-call each other
            if !a_targets.contains(&b) || !b_targets.contains(&a) {
                continue;
            }

            let (a_params, _, a_ret, a_native, a_exported, _) = func_defs[&a];
            let (b_params, _, b_ret, b_native, b_exported, _) = func_defs[&b];

            // Neither can be native or exported
            if a_native || b_native || a_exported || b_exported {
                continue;
            }

            // Same number of params
            if a_params.len() != b_params.len() {
                continue;
            }

            // Same param types
            let same_params = a_params.iter().zip(b_params.iter()).all(|((_, t1), (_, t2))| {
                codegen_type_expr(t1, interner) == codegen_type_expr(t2, interner)
            });
            if !same_params {
                continue;
            }

            // Same return type
            let a_ret_str = a_ret.map(|t| codegen_type_expr(t, interner));
            let b_ret_str = b_ret.map(|t| codegen_type_expr(t, interner));
            if a_ret_str != b_ret_str {
                continue;
            }

            // Verify that the mutual calls are actually in tail position
            // (the targets above only collect Return { Call } patterns, so they are)
            pairs.push((a, b));
            used.insert(a);
            used.insert(b);
        }
    }

    pairs
}

// =============================================================================
// Mutual Tail Call Optimization — Code Generation
// =============================================================================

pub(super) fn codegen_mutual_tce_pair<'a>(
    func_a: Symbol,
    func_b: Symbol,
    stmts: &'a [Stmt<'a>],
    interner: &Interner,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) -> String {
    // Extract function defs
    let mut a_def = None;
    let mut b_def = None;
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, .. } = stmt {
            if *name == func_a {
                a_def = Some((params.as_slice(), *body, return_type.as_ref().copied()));
            } else if *name == func_b {
                b_def = Some((params.as_slice(), *body, return_type.as_ref().copied()));
            }
        }
    }
    let (a_params, a_body, a_ret) = a_def.expect("mutual TCE: func_a not found");
    let (b_params, b_body, _b_ret) = b_def.expect("mutual TCE: func_b not found");
    let names = RustNames::new(interner);

    let a_name = names.ident(func_a);
    let b_name = names.ident(func_b);
    let merged_name = format!("__mutual_{}_{}", a_name, b_name);

    // Build param list (using func_a's param names, since types match)
    let params_str: Vec<String> = a_params.iter()
        .map(|(p, t)| format!("mut {}: {}", interner.resolve(*p), codegen_type_expr(t, interner)))
        .collect();

    let ret_str = a_ret.map(|t| codegen_type_expr(t, interner));

    let mut output = String::new();

    // Merged function
    let sig = if let Some(ref r) = ret_str {
        if r != "()" {
            format!("fn {}(mut __tag: u8, {}) -> {}", merged_name, params_str.join(", "), r)
        } else {
            format!("fn {}(mut __tag: u8, {})", merged_name, params_str.join(", "))
        }
    } else {
        format!("fn {}(mut __tag: u8, {})", merged_name, params_str.join(", "))
    };

    writeln!(output, "{} {{", sig).unwrap();
    writeln!(output, "    loop {{").unwrap();
    writeln!(output, "        match __tag {{").unwrap();

    // Tag 0: func_a body
    writeln!(output, "            0 => {{").unwrap();
    let a_mutable = collect_mutable_vars(a_body);
    let mut a_ctx = RefinementContext::new();
    let mut a_synced = HashSet::new();
    let a_caps = HashMap::new();
    let a_pipes = HashSet::new();
    let a_param_syms: Vec<Symbol> = a_params.iter().map(|(s, _)| *s).collect();
    for s in a_body {
        output.push_str(&codegen_stmt_mutual_tce(s, func_a, func_b, &a_param_syms, 0, 1, interner, 4, &a_mutable, &mut a_ctx, lww_fields, mv_fields, &mut a_synced, &a_caps, async_functions, &a_pipes, boxed_fields, registry, type_env));
    }
    writeln!(output, "            }}").unwrap();

    // Tag 1: func_b body
    writeln!(output, "            1 => {{").unwrap();
    let b_mutable = collect_mutable_vars(b_body);
    let mut b_ctx = RefinementContext::new();
    let mut b_synced = HashSet::new();
    let b_caps = HashMap::new();
    let b_pipes = HashSet::new();
    let b_param_syms: Vec<Symbol> = b_params.iter().map(|(s, _)| *s).collect();
    // Map b's param names to a's param names for assignment
    for s in b_body {
        output.push_str(&codegen_stmt_mutual_tce(s, func_b, func_a, &b_param_syms, 1, 0, interner, 4, &b_mutable, &mut b_ctx, lww_fields, mv_fields, &mut b_synced, &b_caps, async_functions, &b_pipes, boxed_fields, registry, type_env));
    }
    writeln!(output, "            }}").unwrap();

    writeln!(output, "            _ => unreachable!()").unwrap();
    writeln!(output, "        }}").unwrap();
    writeln!(output, "    }}").unwrap();
    writeln!(output, "}}\n").unwrap();

    // Wrapper for func_a
    let wrapper_params_a: Vec<String> = a_params.iter()
        .map(|(p, t)| format!("{}: {}", interner.resolve(*p), codegen_type_expr(t, interner)))
        .collect();
    let wrapper_args_a: Vec<String> = a_params.iter()
        .map(|(p, _)| interner.resolve(*p).to_string())
        .collect();
    writeln!(output, "#[inline]").unwrap();
    if let Some(ref r) = ret_str {
        if r != "()" {
            writeln!(output, "fn {}({}) -> {} {{ {}(0, {}) }}\n", a_name, wrapper_params_a.join(", "), r, merged_name, wrapper_args_a.join(", ")).unwrap();
        } else {
            writeln!(output, "fn {}({}) {{ {}(0, {}) }}\n", a_name, wrapper_params_a.join(", "), merged_name, wrapper_args_a.join(", ")).unwrap();
        }
    } else {
        writeln!(output, "fn {}({}) {{ {}(0, {}) }}\n", a_name, wrapper_params_a.join(", "), merged_name, wrapper_args_a.join(", ")).unwrap();
    }

    // Wrapper for func_b
    let wrapper_params_b: Vec<String> = b_params.iter()
        .map(|(p, t)| format!("{}: {}", interner.resolve(*p), codegen_type_expr(t, interner)))
        .collect();
    let wrapper_args_b: Vec<String> = b_params.iter()
        .map(|(p, _)| interner.resolve(*p).to_string())
        .collect();
    writeln!(output, "#[inline]").unwrap();
    if let Some(ref r) = ret_str {
        if r != "()" {
            writeln!(output, "fn {}({}) -> {} {{ {}(1, {}) }}\n", b_name, wrapper_params_b.join(", "), r, merged_name, wrapper_args_b.join(", ")).unwrap();
        } else {
            writeln!(output, "fn {}({}) {{ {}(1, {}) }}\n", b_name, wrapper_params_b.join(", "), merged_name, wrapper_args_b.join(", ")).unwrap();
        }
    } else {
        writeln!(output, "fn {}({}) {{ {}(1, {}) }}\n", b_name, wrapper_params_b.join(", "), merged_name, wrapper_args_b.join(", ")).unwrap();
    }

    output
}

pub(super) fn codegen_stmt_mutual_tce<'a>(
    stmt: &Stmt<'a>,
    self_name: Symbol,
    partner_name: Symbol,
    param_names: &[Symbol],
    self_tag: u8,
    partner_tag: u8,
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
) -> String {
    let indent_str = "    ".repeat(indent);

    match stmt {
        // Return with a call to partner → switch tag + continue
        Stmt::Return { value: Some(expr) } if expr_is_call_to(partner_name, expr) => {
            if let Expr::Call { args, .. } = expr {
                let mut output = String::new();
                writeln!(output, "{}{{", indent_str).unwrap();
                for (i, arg) in args.iter().enumerate() {
                    let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                }
                for (i, param_sym) in param_names.iter().enumerate() {
                    let param_name = interner.resolve(*param_sym);
                    writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                }
                writeln!(output, "{}    __tag = {};", indent_str, partner_tag).unwrap();
                writeln!(output, "{}    continue;", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
                return output;
            }
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env)
        }

        // Return with a call to self → standard self-TCE
        Stmt::Return { value: Some(expr) } if expr_is_call_to(self_name, expr) => {
            if let Expr::Call { args, .. } = expr {
                let mut output = String::new();
                writeln!(output, "{}{{", indent_str).unwrap();
                for (i, arg) in args.iter().enumerate() {
                    let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                }
                for (i, param_sym) in param_names.iter().enumerate() {
                    let param_name = interner.resolve(*param_sym);
                    writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                }
                writeln!(output, "{}    continue;", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
                return output;
            }
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env)
        }

        // If: recurse into branches
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            let mut output = String::new();
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for s in *then_block {
                output.push_str(&codegen_stmt_mutual_tce(s, self_name, partner_name, param_names, self_tag, partner_tag, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for s in *else_stmts {
                    output.push_str(&codegen_stmt_mutual_tce(s, self_name, partner_name, param_names, self_tag, partner_tag, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
            output
        }

        // Everything else: delegate
        _ => codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env),
    }
}

pub(super) fn expr_is_call_to(target: Symbol, expr: &Expr) -> bool {
    matches!(expr, Expr::Call { function, .. } if *function == target)
}

// =============================================================================
// Tail Call Elimination (TCE) Statement Emitter
// =============================================================================

pub(super) fn codegen_stmt_tce<'a>(
    stmt: &Stmt<'a>,
    func_name: Symbol,
    param_names: &[Symbol],
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
) -> String {
    let indent_str = "    ".repeat(indent);

    match stmt {
        // Case 1 & 2: Return with a self-call in tail position
        Stmt::Return { value: Some(expr) } if expr_is_self_call(func_name, expr) => {
            if let Expr::Call { args, .. } = expr {
                return codegen_tce_loopback(args, param_names, interner, indent, ctx, synced_vars, async_functions);
            }
            // Shouldn't reach here, but fall through to default
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env)
        }

        // Case 2: Return with outer self-call that has a nested self-call arg (Ackermann pattern)
        Stmt::Return { value: Some(expr) } => {
            if let Expr::Call { function, args } = expr {
                if *function == func_name {
                    let mut output = String::new();
                    writeln!(output, "{}{{", indent_str).unwrap();
                    // Evaluate args — nested self-calls remain as normal recursion,
                    // but the outer call becomes a loop iteration
                    for (i, arg) in args.iter().enumerate() {
                        if expr_is_self_call(func_name, arg) {
                            // Inner self-call: evaluate as normal recursive call
                            let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                            writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                        } else {
                            let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                            writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                        }
                    }
                    // Assign temporaries to params
                    for (i, param_sym) in param_names.iter().enumerate() {
                        let param_name = interner.resolve(*param_sym);
                        writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                    }
                    writeln!(output, "{}    continue;", indent_str).unwrap();
                    writeln!(output, "{}}}", indent_str).unwrap();
                    return output;
                }
            }
            // Not a self-call — delegate to normal codegen
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env)
        }

        // Case 3: If statement — recurse into branches
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            let mut output = String::new();
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for s in *then_block {
                output.push_str(&codegen_stmt_tce(s, func_name, param_names, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for s in *else_stmts {
                    output.push_str(&codegen_stmt_tce(s, func_name, param_names, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
            output
        }

        // Case 4: Everything else — delegate
        _ => codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env),
    }
}
