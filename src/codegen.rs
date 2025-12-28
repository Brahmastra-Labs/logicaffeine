use std::fmt::Write;

use crate::analysis::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::ast::logic::{LogicExpr, NumberKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::token::TokenType;

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

    // Phase 32: Emit function definitions before main
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type } = stmt {
            output.push_str(&codegen_function_def(*name, params, body, *return_type, interner));
        }
    }

    // Main function
    writeln!(output, "fn main() {{").unwrap();
    for stmt in stmts {
        // Skip function definitions - they're already emitted above
        if matches!(stmt, Stmt::FunctionDef { .. }) {
            continue;
        }
        output.push_str(&codegen_stmt(stmt, interner, 1));
    }
    writeln!(output, "}}").unwrap();
    output
}

/// Phase 32: Generate a function definition.
fn codegen_function_def(
    name: Symbol,
    params: &[(Symbol, Symbol)],
    body: &[Stmt],
    return_type: Option<Symbol>,
    interner: &Interner,
) -> String {
    let mut output = String::new();
    let func_name = interner.resolve(name);

    // Build parameter list
    let params_str: Vec<String> = params.iter()
        .map(|(param_name, param_type)| {
            let name = interner.resolve(*param_name);
            let ty = map_type_to_rust(interner.resolve(*param_type));
            format!("{}: {}", name, ty)
        })
        .collect();

    // Infer return type from body if not specified
    let inferred_return = return_type.map(|s| interner.resolve(s).to_string())
        .or_else(|| infer_return_type_from_body(body, interner));

    // Emit function signature
    if let Some(ret_ty) = inferred_return {
        let rust_ret = map_type_to_rust(&ret_ty);
        if rust_ret != "()" {
            writeln!(output, "fn {}({}) -> {} {{", func_name, params_str.join(", "), rust_ret).unwrap();
        } else {
            writeln!(output, "fn {}({}) {{", func_name, params_str.join(", ")).unwrap();
        }
    } else {
        writeln!(output, "fn {}({}) {{", func_name, params_str.join(", ")).unwrap();
    }

    // Emit body
    for stmt in body {
        output.push_str(&codegen_stmt(stmt, interner, 1));
    }

    writeln!(output, "}}\n").unwrap();
    output
}

/// Infer return type from function body by looking at Return statements.
fn infer_return_type_from_body(body: &[Stmt], _interner: &Interner) -> Option<String> {
    for stmt in body {
        if let Stmt::Return { value: Some(_) } = stmt {
            // For now, assume Int for any expression return
            // TODO: Implement proper type inference
            return Some("Int".to_string());
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

pub fn codegen_stmt(stmt: &Stmt, interner: &Interner, indent: usize) -> String {
    let indent_str = "    ".repeat(indent);
    let mut output = String::new();

    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            let var_name = interner.resolve(*var);
            let value_str = codegen_expr(value, interner);
            let type_annotation = ty.map(|t| codegen_type_expr(t, interner));

            match (*mutable, type_annotation) {
                (true, Some(t)) => writeln!(output, "{}let mut {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (true, None) => writeln!(output, "{}let mut {} = {};", indent_str, var_name, value_str).unwrap(),
                (false, Some(t)) => writeln!(output, "{}let {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (false, None) => writeln!(output, "{}let {} = {};", indent_str, var_name, value_str).unwrap(),
            }
        }

        Stmt::Set { target, value } => {
            let target_name = interner.resolve(*target);
            let value_str = codegen_expr(value, interner);
            writeln!(output, "{}{} = {};", indent_str, target_name, value_str).unwrap();
        }

        Stmt::Call { function, args } => {
            let func_name = interner.resolve(*function);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, interner)).collect();
            writeln!(output, "{}{}({});", indent_str, func_name, args_str.join(", ")).unwrap();
        }

        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr(cond, interner);
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            for stmt in *then_block {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1));
            }
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                for stmt in *else_stmts {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 1));
                }
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::While { cond, body } => {
            let cond_str = codegen_expr(cond, interner);
            writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1));
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Repeat { var, iterable, body } => {
            let var_name = interner.resolve(*var);
            let iter_str = codegen_expr(iterable, interner);
            writeln!(output, "{}for {} in {} {{", indent_str, var_name, iter_str).unwrap();
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1));
            }
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
                                    format!("ref {}", field_name)
                                } else {
                                    format!("{}: ref {}", field_name, binding_name)
                                }
                            })
                            .collect();
                        writeln!(output, "{}    {}{} {{ {} }} => {{", indent_str, enum_prefix, variant_name, bindings_str.join(", ")).unwrap();
                    }
                } else {
                    // Otherwise (wildcard) pattern
                    writeln!(output, "{}    _ => {{", indent_str).unwrap();
                }

                for stmt in arm.body {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 2));
                }
                writeln!(output, "{}    }}", indent_str).unwrap();
            }

            writeln!(output, "{}}}", indent_str).unwrap();
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
            format!("{}[{}]", coll_str, index - 1)
        }

        Expr::Slice { collection, start, end } => {
            let coll_str = codegen_expr(collection, interner);
            // 1-indexed to 0-indexed: items 2 through 5 → &list[1..5]
            format!("&{}[{}..{}]", coll_str, start - 1, end)
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

        Expr::New { type_name, type_args } => {
            let type_str = interner.resolve(*type_name);
            if type_args.is_empty() {
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
        Literal::Text(sym) => format!("\"{}\"", interner.resolve(*sym)),
        Literal::Boolean(b) => b.to_string(),
        Literal::Nothing => "()".to_string(),
    }
}

fn codegen_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) => {
            match interner.resolve(*sym) {
                "Int" => "i64".to_string(),
                "Nat" => "u64".to_string(),  // Spec §10.6.1: Nat → u64
                "Text" => "String".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Unit" => "()".to_string(),
                other => other.to_string(),
            }
        }
        TypeExpr::Named(sym) => interner.resolve(*sym).to_string(),
        TypeExpr::Generic { base, params } => {
            let base_str = match interner.resolve(*base) {
                "List" | "Seq" => "Vec",
                "Option" => "Option",
                "Result" => "Result",
                other => other,
            };
            let param_strs: Vec<String> = params.iter()
                .map(|p| codegen_type_expr(p, interner))
                .collect();
            format!("{}<{}>", base_str, param_strs.join(", "))
        }
        TypeExpr::Function { inputs, output } => {
            let input_strs: Vec<String> = inputs.iter()
                .map(|p| codegen_type_expr(p, interner))
                .collect();
            let output_str = codegen_type_expr(output, interner);
            format!("fn({}) -> {}", input_strs.join(", "), output_str)
        }
    }
}

pub fn codegen_assertion(expr: &LogicExpr, interner: &Interner) -> String {
    match expr {
        LogicExpr::Atom(sym) => interner.resolve(*sym).to_string(),

        LogicExpr::Identity { left, right } => {
            let left_str = codegen_term(left, interner);
            let right_str = codegen_term(right, interner);
            format!("({} == {})", left_str, right_str)
        }

        LogicExpr::Predicate { name, args } => {
            let pred_name = interner.resolve(*name).to_lowercase();
            match pred_name.as_str() {
                "greater" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} > {})", left, right)
                }
                "less" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} < {})", left, right)
                }
                "equal" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} == {})", left, right)
                }
                "greaterequal" | "greaterorequal" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} >= {})", left, right)
                }
                "lessequal" | "lessorequal" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} <= {})", left, right)
                }
                "positive" if args.len() == 1 => {
                    let arg = codegen_term(&args[0], interner);
                    format!("({} > 0)", arg)
                }
                "negative" if args.len() == 1 => {
                    let arg = codegen_term(&args[0], interner);
                    format!("({} < 0)", arg)
                }
                "zero" if args.len() == 1 => {
                    let arg = codegen_term(&args[0], interner);
                    format!("({} == 0)", arg)
                }
                _ => {
                    let args_str: Vec<String> = args.iter()
                        .map(|a| codegen_term(a, interner))
                        .collect();
                    format!("{}({})", interner.resolve(*name), args_str.join(", "))
                }
            }
        }

        LogicExpr::BinaryOp { left, op, right } => {
            let left_str = codegen_assertion(left, interner);
            let right_str = codegen_assertion(right, interner);
            let op_str = match op {
                TokenType::And => "&&",
                TokenType::Or => "||",
                TokenType::Iff => "==",
                _ => "/* unknown op */",
            };
            format!("({} {} {})", left_str, op_str, right_str)
        }

        LogicExpr::UnaryOp { op, operand } => {
            let operand_str = codegen_assertion(operand, interner);
            match op {
                TokenType::Not => format!("(!{})", operand_str),
                _ => format!("/* unknown unary op */({})", operand_str),
            }
        }

        LogicExpr::Comparative { adjective, subject, object, .. } => {
            let adj_name = interner.resolve(*adjective).to_lowercase();
            let subj_str = codegen_term(subject, interner);
            let obj_str = codegen_term(object, interner);
            match adj_name.as_str() {
                "great" | "big" | "large" | "tall" | "old" | "high" => {
                    format!("({} > {})", subj_str, obj_str)
                }
                "small" | "little" | "short" | "young" | "low" => {
                    format!("({} < {})", subj_str, obj_str)
                }
                _ => format!("({} > {})", subj_str, obj_str), // default to greater-than
            }
        }

        _ => "/* unsupported LogicExpr */true".to_string(),
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
