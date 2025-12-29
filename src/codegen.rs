use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::ast::logic::{LogicExpr, NumberKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, ReadSource, Stmt, TypeExpr};
use crate::formatter::RustFormatter;
use crate::intern::{Interner, Symbol};
use crate::registry::SymbolRegistry;

// =============================================================================
// Phase 43C: Refinement Type Enforcement
// =============================================================================

/// Tracks refinement type constraints across scopes for mutation enforcement.
/// When a variable with a refinement type is defined, we register its constraint.
/// When that variable is mutated via `Set`, we re-emit the assertion.
pub struct RefinementContext<'a> {
    /// Stack of scopes. Each scope maps variable Symbol to (bound_var, predicate).
    scopes: Vec<HashMap<Symbol, (Symbol, &'a LogicExpr<'a>)>>,
}

impl<'a> RefinementContext<'a> {
    pub fn new() -> Self {
        Self { scopes: vec![HashMap::new()] }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn register(&mut self, var: Symbol, bound_var: Symbol, predicate: &'a LogicExpr<'a>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(var, (bound_var, predicate));
        }
    }

    fn get_constraint(&self, var: Symbol) -> Option<(Symbol, &'a LogicExpr<'a>)> {
        for scope in self.scopes.iter().rev() {
            if let Some(entry) = scope.get(&var) {
                return Some(*entry);
            }
        }
        None
    }
}

/// Emits a debug_assert for a refinement predicate, substituting the bound variable.
fn emit_refinement_check(
    var_name: &str,
    bound_var: Symbol,
    predicate: &LogicExpr,
    interner: &Interner,
    indent_str: &str,
    output: &mut String,
) {
    let assertion = codegen_assertion(predicate, interner);
    let bound = interner.resolve(bound_var);
    let check = if bound == var_name {
        assertion
    } else {
        replace_word(&assertion, bound, var_name)
    };
    writeln!(output, "{}debug_assert!({});", indent_str, check).unwrap();
}

/// Word-boundary replacement to substitute bound variable with actual variable.
fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            if !word.is_empty() {
                result.push_str(if word == from { to } else { &word });
                word.clear();
            }
            result.push(c);
        }
    }
    if !word.is_empty() {
        result.push_str(if word == from { to } else { &word });
    }
    result
}

/// Grand Challenge: Collect all variables that need `let mut` in Rust.
/// This includes:
/// - Variables that are targets of `Set` statements (reassignment)
/// - Variables that are targets of `Push` statements (mutation via push)
/// - Variables that are targets of `Pop` statements (mutation via pop)
fn collect_mutable_vars(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    for stmt in stmts {
        collect_mutable_vars_stmt(stmt, &mut targets);
    }
    targets
}

fn collect_mutable_vars_stmt(stmt: &Stmt, targets: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => {
            targets.insert(*target);
        }
        Stmt::Push { collection, .. } => {
            // If collection is an identifier, it needs to be mutable
            if let Expr::Identifier(sym) = collection {
                targets.insert(*sym);
            }
        }
        Stmt::Pop { collection, .. } => {
            // If collection is an identifier, it needs to be mutable
            if let Expr::Identifier(sym) = collection {
                targets.insert(*sym);
            }
        }
        Stmt::SetIndex { collection, .. } => {
            // If collection is an identifier, it needs to be mutable
            if let Expr::Identifier(sym) = collection {
                targets.insert(*sym);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_mutable_vars_stmt(s, targets);
            }
            if let Some(else_stmts) = else_block {
                for s in *else_stmts {
                    collect_mutable_vars_stmt(s, targets);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        Stmt::Repeat { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        Stmt::Zone { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        // Phase 9: Structured Concurrency blocks
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        _ => {}
    }
}

/// Generate complete Rust program with struct definitions and main function.
///
/// Phase 31: Structs are wrapped in `mod user_types` to enforce visibility.
/// Phase 32: Function definitions are emitted before main.
pub fn codegen_program(stmts: &[Stmt], registry: &TypeRegistry, interner: &Interner) -> String {
    let mut output = String::new();

    // Prelude
    writeln!(output, "use logos_core::prelude::*;\n").unwrap();

    // Collect user-defined structs from registry (Phase 34: now with generics)
    let structs: Vec<_> = registry.iter_types()
        .filter_map(|(name, def)| {
            if let TypeDef::Struct { fields, generics } = def {
                if !fields.is_empty() || !generics.is_empty() {
                    Some((*name, fields.clone(), generics.clone()))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Phase 33/34: Collect user-defined enums from registry (now with generics)
    let enums: Vec<_> = registry.iter_types()
        .filter_map(|(name, def)| {
            if let TypeDef::Enum { variants, generics } = def {
                if !variants.is_empty() || !generics.is_empty() {
                    Some((*name, variants.clone(), generics.clone()))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Emit struct and enum definitions in user_types module if any exist
    if !structs.is_empty() || !enums.is_empty() {
        writeln!(output, "pub mod user_types {{").unwrap();
        writeln!(output, "    use super::*;\n").unwrap();

        for (name, fields, generics) in &structs {
            output.push_str(&codegen_struct_def(*name, fields, generics, interner, 4));
        }

        for (name, variants, generics) in &enums {
            output.push_str(&codegen_enum_def(*name, variants, generics, interner, 4));
        }

        writeln!(output, "}}\n").unwrap();
        writeln!(output, "use user_types::*;\n").unwrap();
    }

    // Phase 32/38: Emit function definitions before main
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, is_native } = stmt {
            output.push_str(&codegen_function_def(*name, params, body, return_type.as_ref().copied(), *is_native, interner));
        }
    }

    // Grand Challenge: Collect variables that need to be mutable
    let main_stmts: Vec<&Stmt> = stmts.iter()
        .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
        .collect();
    let mut main_mutable_vars = HashSet::new();
    for stmt in &main_stmts {
        collect_mutable_vars_stmt(stmt, &mut main_mutable_vars);
    }

    // Main function
    writeln!(output, "fn main() {{").unwrap();
    let mut main_ctx = RefinementContext::new();
    for stmt in stmts {
        // Skip function definitions - they're already emitted above
        if matches!(stmt, Stmt::FunctionDef { .. }) {
            continue;
        }
        output.push_str(&codegen_stmt(stmt, interner, 1, &main_mutable_vars, &mut main_ctx));
    }
    writeln!(output, "}}").unwrap();
    output
}

/// Phase 32/38: Generate a function definition.
/// Phase 38: Updated for native functions and TypeExpr types.
fn codegen_function_def(
    name: Symbol,
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    return_type: Option<&TypeExpr>,
    is_native: bool,
    interner: &Interner,
) -> String {
    let mut output = String::new();
    let func_name = interner.resolve(name);

    // Build parameter list using TypeExpr
    let params_str: Vec<String> = params.iter()
        .map(|(param_name, param_type)| {
            let name = interner.resolve(*param_name);
            let ty = codegen_type_expr(param_type, interner);
            format!("{}: {}", name, ty)
        })
        .collect();

    // Get return type string from TypeExpr or infer from body
    let return_type_str = return_type
        .map(|t| codegen_type_expr(t, interner))
        .or_else(|| infer_return_type_from_body(body, interner));

    // Build function signature
    let signature = if let Some(ref ret_ty) = return_type_str {
        if ret_ty != "()" {
            format!("fn {}({}) -> {}", func_name, params_str.join(", "), ret_ty)
        } else {
            format!("fn {}({})", func_name, params_str.join(", "))
        }
    } else {
        format!("fn {}({})", func_name, params_str.join(", "))
    };

    // Phase 38: Handle native functions
    if is_native {
        let (module, core_fn) = map_native_function(func_name);
        writeln!(output, "{} {{", signature).unwrap();

        // Generate call to logos_core
        let arg_names: Vec<&str> = params.iter()
            .map(|(n, _)| interner.resolve(*n))
            .collect();

        writeln!(output, "    logos_core::{}::{}({})", module, core_fn, arg_names.join(", ")).unwrap();
        writeln!(output, "}}\n").unwrap();
    } else {
        // Non-native: emit body
        // Grand Challenge: Collect mutable vars for this function
        let func_mutable_vars = collect_mutable_vars(body);
        writeln!(output, "{} {{", signature).unwrap();
        let mut func_ctx = RefinementContext::new();
        for stmt in body {
            output.push_str(&codegen_stmt(stmt, interner, 1, &func_mutable_vars, &mut func_ctx));
        }
        writeln!(output, "}}\n").unwrap();
    }

    output
}

/// Phase 38: Map native function names to logos_core module paths.
fn map_native_function(name: &str) -> (&'static str, &'static str) {
    match name {
        "read" => ("file", "read"),
        "write" => ("file", "write"),
        "now" => ("time", "now"),
        "sleep" => ("time", "sleep"),
        "randomInt" => ("random", "randomInt"),
        "randomFloat" => ("random", "randomFloat"),
        "get" => ("env", "get"),
        "args" => ("env", "args"),
        _ => panic!("Unknown native function: {}. Add mapping to map_native_function().", name),
    }
}

/// Phase 38: Convert TypeExpr to Rust type string.
fn codegen_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) => {
            map_type_to_rust(interner.resolve(*sym))
        }
        TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            // Check for common mappings
            map_type_to_rust(name)
        }
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let params_str: Vec<String> = params.iter()
                .map(|p| codegen_type_expr(p, interner))
                .collect();

            match base_name {
                "Result" => {
                    if params_str.len() == 2 {
                        format!("Result<{}, {}>", params_str[0], params_str[1])
                    } else if params_str.len() == 1 {
                        format!("Result<{}, String>", params_str[0])
                    } else {
                        "Result<(), String>".to_string()
                    }
                }
                "Option" => {
                    if !params_str.is_empty() {
                        format!("Option<{}>", params_str[0])
                    } else {
                        "Option<()>".to_string()
                    }
                }
                "Seq" | "List" | "Vec" => {
                    if !params_str.is_empty() {
                        format!("Vec<{}>", params_str[0])
                    } else {
                        "Vec<()>".to_string()
                    }
                }
                "Map" | "HashMap" => {
                    if params_str.len() >= 2 {
                        format!("std::collections::HashMap<{}, {}>", params_str[0], params_str[1])
                    } else {
                        "std::collections::HashMap<String, String>".to_string()
                    }
                }
                other => {
                    if params_str.is_empty() {
                        other.to_string()
                    } else {
                        format!("{}<{}>", other, params_str.join(", "))
                    }
                }
            }
        }
        TypeExpr::Function { inputs, output } => {
            let inputs_str: Vec<String> = inputs.iter()
                .map(|i| codegen_type_expr(i, interner))
                .collect();
            let output_str = codegen_type_expr(output, interner);
            format!("fn({}) -> {}", inputs_str.join(", "), output_str)
        }
        // Phase 43C: Refinement types use the base type for Rust type annotation
        // The constraint predicate is handled separately via debug_assert!
        TypeExpr::Refinement { base, .. } => {
            codegen_type_expr(base, interner)
        }
    }
}

/// Infer return type from function body by looking at Return statements.
fn infer_return_type_from_body(body: &[Stmt], _interner: &Interner) -> Option<String> {
    for stmt in body {
        if let Stmt::Return { value: Some(_) } = stmt {
            // For now, assume i64 for any expression return
            // TODO: Implement proper type inference
            return Some("i64".to_string());
        }
    }
    None
}

/// Map LOGOS type names to Rust types.
fn map_type_to_rust(ty: &str) -> String {
    match ty {
        "Int" => "i64".to_string(),
        "Nat" => "u64".to_string(),
        "Text" => "String".to_string(),
        "Bool" | "Boolean" => "bool".to_string(),
        "Real" => "f64".to_string(),
        "Unit" | "()" => "()".to_string(),
        other => other.to_string(),
    }
}

/// Generate a single struct definition with derives and visibility.
/// Phase 34: Now supports generic type parameters.
fn codegen_struct_def(name: Symbol, fields: &[FieldDef], generics: &[Symbol], interner: &Interner, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter()
            .map(|g| interner.resolve(*g))
            .collect();
        format!("<{}>", params.join(", "))
    };

    writeln!(output, "{}#[derive(Default, Debug, Clone)]", ind).unwrap();
    writeln!(output, "{}pub struct {}{} {{", ind, interner.resolve(name), generic_str).unwrap();

    for field in fields {
        let vis = if field.is_public { "pub " } else { "" };
        let rust_type = codegen_field_type(&field.ty, interner);
        writeln!(output, "{}    {}{}: {},", ind, vis, interner.resolve(field.name), rust_type).unwrap();
    }

    writeln!(output, "{}}}\n", ind).unwrap();
    output
}

/// Phase 33/34: Generate enum definition with optional generic parameters.
fn codegen_enum_def(name: Symbol, variants: &[VariantDef], generics: &[Symbol], interner: &Interner, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter()
            .map(|g| interner.resolve(*g))
            .collect();
        format!("<{}>", params.join(", "))
    };

    writeln!(output, "{}#[derive(Debug, Clone)]", ind).unwrap();
    writeln!(output, "{}pub enum {}{} {{", ind, interner.resolve(name), generic_str).unwrap();

    for variant in variants {
        let variant_name = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            // Unit variant
            writeln!(output, "{}    {},", ind, variant_name).unwrap();
        } else {
            // Struct variant with named fields
            let fields_str: Vec<String> = variant.fields.iter()
                .map(|f| {
                    let rust_type = codegen_field_type(&f.ty, interner);
                    format!("{}: {}", interner.resolve(f.name), rust_type)
                })
                .collect();
            writeln!(output, "{}    {} {{ {} }},", ind, variant_name, fields_str.join(", ")).unwrap();
        }
    }

    writeln!(output, "{}}}\n", ind).unwrap();
    output
}

/// Convert FieldType to Rust type string.
fn codegen_field_type(ty: &FieldType, interner: &Interner) -> String {
    match ty {
        FieldType::Primitive(sym) => {
            match interner.resolve(*sym) {
                "Int" => "i64".to_string(),
                "Nat" => "u64".to_string(),
                "Text" => "String".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Real" => "f64".to_string(),
                "Unit" => "()".to_string(),
                other => other.to_string(),
            }
        }
        FieldType::Named(sym) => interner.resolve(*sym).to_string(),
        FieldType::Generic { base, params } => {
            let base_str = match interner.resolve(*base) {
                "List" | "Seq" => "Vec",
                "Option" => "Option",
                "Result" => "Result",
                other => other,
            };
            let param_strs: Vec<String> = params.iter()
                .map(|p| codegen_field_type(p, interner))
                .collect();
            format!("{}<{}>", base_str, param_strs.join(", "))
        }
        // Phase 34: Type parameter reference (T, U, etc.)
        FieldType::TypeParam(sym) => interner.resolve(*sym).to_string(),
    }
}

pub fn codegen_stmt<'a>(
    stmt: &Stmt<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
) -> String {
    let indent_str = "    ".repeat(indent);
    let mut output = String::new();

    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            let var_name = interner.resolve(*var);
            let value_str = codegen_expr(value, interner);
            let type_annotation = ty.map(|t| codegen_type_expr(t, interner));

            // Grand Challenge: Variable is mutable if explicitly marked OR if it's a Set target
            let is_mutable = *mutable || mutable_vars.contains(var);

            match (is_mutable, type_annotation) {
                (true, Some(t)) => writeln!(output, "{}let mut {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (true, None) => writeln!(output, "{}let mut {} = {};", indent_str, var_name, value_str).unwrap(),
                (false, Some(t)) => writeln!(output, "{}let {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (false, None) => writeln!(output, "{}let {} = {};", indent_str, var_name, value_str).unwrap(),
            }

            // Phase 43C: Handle refinement type
            if let Some(TypeExpr::Refinement { base: _, var: bound_var, predicate }) = ty {
                emit_refinement_check(var_name, *bound_var, predicate, interner, &indent_str, &mut output);
                ctx.register(*var, *bound_var, predicate);
            }
        }

        Stmt::Set { target, value } => {
            let target_name = interner.resolve(*target);
            let value_str = codegen_expr(value, interner);
            writeln!(output, "{}{} = {};", indent_str, target_name, value_str).unwrap();

            // Phase 43C: Check if this variable has a refinement constraint
            if let Some((bound_var, predicate)) = ctx.get_constraint(*target) {
                emit_refinement_check(target_name, bound_var, predicate, interner, &indent_str, &mut output);
            }
        }

        Stmt::Call { function, args } => {
            let func_name = interner.resolve(*function);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, interner)).collect();
            writeln!(output, "{}{}({});", indent_str, func_name, args_str.join(", ")).unwrap();
        }

        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr(cond, interner);
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for stmt in *then_block {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for stmt in *else_stmts {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::While { cond, body, decreasing: _ } => {
            // decreasing is compile-time only, ignored at runtime
            let cond_str = codegen_expr(cond, interner);
            writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx));
            }
            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Repeat { var, iterable, body } => {
            let var_name = interner.resolve(*var);
            let iter_str = codegen_expr(iterable, interner);
            writeln!(output, "{}for {} in {} {{", indent_str, var_name, iter_str).unwrap();
            ctx.push_scope();
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx));
            }
            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Return { value } => {
            if let Some(v) = value {
                let value_str = codegen_expr(v, interner);
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

        Stmt::Give { object, recipient } => {
            // Move semantics: pass ownership without borrowing
            let obj_str = codegen_expr(object, interner);
            let recv_str = codegen_expr(recipient, interner);
            writeln!(output, "{}{}({});", indent_str, recv_str, obj_str).unwrap();
        }

        Stmt::Show { object, recipient } => {
            // Borrow semantics: pass immutable reference
            let obj_str = codegen_expr(object, interner);
            let recv_str = codegen_expr(recipient, interner);
            writeln!(output, "{}{}(&{});", indent_str, recv_str, obj_str).unwrap();
        }

        Stmt::SetField { object, field, value } => {
            let obj_str = codegen_expr(object, interner);
            let field_name = interner.resolve(*field);
            let value_str = codegen_expr(value, interner);
            writeln!(output, "{}{}.{} = {};", indent_str, obj_str, field_name, value_str).unwrap();
        }

        Stmt::StructDef { .. } => {
            // Struct definitions are handled in codegen_program, not here
        }

        Stmt::FunctionDef { .. } => {
            // Function definitions are handled in codegen_program, not here
        }

        Stmt::Inspect { target, arms, .. } => {
            let target_str = codegen_expr(target, interner);
            writeln!(output, "{}match {} {{", indent_str, target_str).unwrap();

            for arm in arms {
                if let Some(variant) = arm.variant {
                    let variant_name = interner.resolve(variant);
                    // Get the enum name from the arm, or fallback to just variant name
                    let enum_prefix = arm.enum_name
                        .map(|e| format!("{}::", interner.resolve(e)))
                        .unwrap_or_default();

                    if arm.bindings.is_empty() {
                        // Unit variant pattern
                        writeln!(output, "{}    {}{} => {{", indent_str, enum_prefix, variant_name).unwrap();
                    } else {
                        // Pattern with bindings
                        let bindings_str: Vec<String> = arm.bindings.iter()
                            .map(|(field, binding)| {
                                let field_name = interner.resolve(*field);
                                let binding_name = interner.resolve(*binding);
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
                for stmt in arm.body {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx));
                }
                ctx.pop_scope();
                writeln!(output, "{}    }}", indent_str).unwrap();
            }

            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Push { value, collection } => {
            let val_str = codegen_expr(value, interner);
            let coll_str = codegen_expr(collection, interner);
            writeln!(output, "{}{}.push({});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::Pop { collection, into } => {
            let coll_str = codegen_expr(collection, interner);
            match into {
                Some(var) => {
                    let var_name = interner.resolve(*var);
                    // Unwrap the Option returned by pop() - panics if empty
                    writeln!(output, "{}let {} = {}.pop().expect(\"Pop from empty collection\");", indent_str, var_name, coll_str).unwrap();
                }
                None => {
                    writeln!(output, "{}{}.pop();", indent_str, coll_str).unwrap();
                }
            }
        }

        Stmt::SetIndex { collection, index, value } => {
            let coll_str = codegen_expr(collection, interner);
            let index_str = codegen_expr(index, interner);
            let value_str = codegen_expr(value, interner);
            // 1-based indexing: item 2 of items → items[1]
            writeln!(output, "{}{}[({} - 1) as usize] = {};", indent_str, coll_str, index_str, value_str).unwrap();
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
                    "{}let {} = logos_core::memory::Zone::new_mapped(\"{}\").expect(\"Failed to map file\");",
                    indent_str, zone_name, path
                ).unwrap();
            } else {
                // Heap arena zone
                let cap = capacity.unwrap_or(4096); // Default 4KB
                writeln!(
                    output,
                    "{}let {} = logos_core::memory::Zone::new_heap({});",
                    indent_str, zone_name, cap
                ).unwrap();
            }

            // Open block scope
            writeln!(output, "{}{{", indent_str).unwrap();
            ctx.push_scope();

            // Generate body statements
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx));
            }

            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        // Phase 9: Concurrent execution block (async, I/O-bound)
        // Generates tokio::join! for concurrent task execution
        Stmt::Concurrent { tasks } => {
            // Collect Let statements to generate tuple destructuring
            let let_bindings: Vec<_> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(interner.resolve(*var).to_string())
                } else {
                    None
                }
            }).collect();

            if !let_bindings.is_empty() {
                // Generate tuple destructuring for concurrent Let bindings
                writeln!(output, "{}let ({}) = tokio::join!(", indent_str, let_bindings.join(", ")).unwrap();
            } else {
                writeln!(output, "{}tokio::join!(", indent_str).unwrap();
            }

            for (i, stmt) in tasks.iter().enumerate() {
                let inner = codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx);
                // Wrap each statement in an async block
                write!(output, "{}    async {{ {} }}", indent_str, inner.trim()).unwrap();
                if i < tasks.len() - 1 {
                    writeln!(output, ",").unwrap();
                } else {
                    writeln!(output).unwrap();
                }
            }

            writeln!(output, "{});", indent_str).unwrap();
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
                    let inner = codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx);
                    write!(output, "{}    || {{ {} }}", indent_str, inner.trim()).unwrap();
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
                    let inner = codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx);
                    writeln!(output, "{}        std::thread::spawn(move || {{ {} }}),",
                             indent_str, inner.trim()).unwrap();
                }
                writeln!(output, "{}    ];", indent_str).unwrap();
                writeln!(output, "{}    for h in handles {{ h.join().unwrap(); }}", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
            }
        }

        // Phase 10: Read from console or file
        Stmt::ReadFrom { var, source } => {
            let var_name = interner.resolve(*var);
            match source {
                ReadSource::Console => {
                    writeln!(output, "{}let {} = logos_core::io::read_line();", indent_str, var_name).unwrap();
                }
                ReadSource::File(path_expr) => {
                    let path_str = codegen_expr(path_expr, interner);
                    writeln!(
                        output,
                        "{}let {} = logos_core::file::read({}.to_string()).expect(\"Failed to read file\");",
                        indent_str, var_name, path_str
                    ).unwrap();
                }
            }
        }

        // Phase 10: Write to file
        Stmt::WriteFile { content, path } => {
            let content_str = codegen_expr(content, interner);
            let path_str = codegen_expr(path, interner);
            writeln!(
                output,
                "{}logos_core::file::write({}.to_string(), {}.to_string()).expect(\"Failed to write file\");",
                indent_str, path_str, content_str
            ).unwrap();
        }
    }

    output
}

pub fn codegen_expr(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Literal(lit) => codegen_literal(lit, interner),

        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),

        Expr::BinaryOp { op, left, right } => {
            let left_str = codegen_expr(left, interner);
            let right_str = codegen_expr(right, interner);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And => "&&",
                BinaryOpKind::Or => "||",
            };
            format!("({} {} {})", left_str, op_str, right_str)
        }

        Expr::Call { function, args } => {
            let func_name = interner.resolve(*function);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, interner)).collect();
            format!("{}({})", func_name, args_str.join(", "))
        }

        Expr::Index { collection, index } => {
            let coll_str = codegen_expr(collection, interner);
            let index_str = codegen_expr(index, interner);
            // Phase 43D: 1-based indexing with runtime bounds check
            format!("logos_index(&{}, {})", coll_str, index_str)
        }

        Expr::Slice { collection, start, end } => {
            let coll_str = codegen_expr(collection, interner);
            let start_str = codegen_expr(start, interner);
            let end_str = codegen_expr(end, interner);
            // Phase 43D: 1-indexed inclusive to 0-indexed exclusive
            // "items 1 through 3" → &items[0..3] (elements at indices 0, 1, 2)
            format!("&{}[({} - 1) as usize..{} as usize]", coll_str, start_str, end_str)
        }

        Expr::Copy { expr } => {
            let expr_str = codegen_expr(expr, interner);
            // Phase 43D: Explicit clone to owned Vec
            format!("{}.to_vec()", expr_str)
        }

        Expr::Length { collection } => {
            let coll_str = codegen_expr(collection, interner);
            // Phase 43D: Collection length - cast to i64 for LOGOS integer semantics
            format!("({}.len() as i64)", coll_str)
        }

        Expr::List(ref items) => {
            let item_strs: Vec<String> = items.iter()
                .map(|i| codegen_expr(i, interner))
                .collect();
            format!("vec![{}]", item_strs.join(", "))
        }

        Expr::Range { start, end } => {
            let start_str = codegen_expr(start, interner);
            let end_str = codegen_expr(end, interner);
            format!("({}..={})", start_str, end_str)
        }

        Expr::FieldAccess { object, field } => {
            let obj_str = codegen_expr(object, interner);
            let field_name = interner.resolve(*field);
            format!("{}.{}", obj_str, field_name)
        }

        Expr::New { type_name, type_args, init_fields } => {
            let type_str = interner.resolve(*type_name);
            if !init_fields.is_empty() {
                // Struct initialization with fields: Point { x: 10, y: 20 }
                let fields_str = init_fields.iter()
                    .map(|(name, value)| {
                        let field_name = interner.resolve(*name);
                        let value_str = codegen_expr(value, interner);
                        format!("{}: {}", field_name, value_str)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} {{ {} }}", type_str, fields_str)
            } else if type_args.is_empty() {
                format!("{}::default()", type_str)
            } else {
                // Phase 34: Turbofish syntax for generic instantiation
                let args_str = type_args.iter()
                    .map(|s| map_type_to_rust(interner.resolve(*s)))
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
                // Struct variant: Shape::Circle { radius: 10 }
                let fields_str: Vec<String> = fields.iter()
                    .map(|(field_name, value)| {
                        let name = interner.resolve(*field_name);
                        let val = codegen_expr(value, interner);
                        format!("{}: {}", name, val)
                    })
                    .collect();
                format!("{}::{} {{ {} }}", enum_str, variant_str, fields_str.join(", "))
            }
        }
    }
}

fn codegen_literal(lit: &Literal, interner: &Interner) -> String {
    match lit {
        Literal::Number(n) => n.to_string(),
        // String literals are converted to String for consistent Text type handling
        Literal::Text(sym) => format!("String::from(\"{}\")", interner.resolve(*sym)),
        Literal::Boolean(b) => b.to_string(),
        Literal::Nothing => "()".to_string(),
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
        let expr = Expr::Literal(Literal::Number(42));
        assert_eq!(codegen_expr(&expr, &interner), "42");
    }

    #[test]
    fn test_literal_boolean() {
        let interner = Interner::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(true)), &interner), "true");
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(false)), &interner), "false");
    }

    #[test]
    fn test_literal_nothing() {
        let interner = Interner::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Nothing), &interner), "()");
    }
}
