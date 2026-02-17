use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

pub(super) fn codegen_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
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
                "Option" | "Maybe" => {
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
                "Set" | "HashSet" => {
                    if !params_str.is_empty() {
                        format!("std::collections::HashSet<{}>", params_str[0])
                    } else {
                        "std::collections::HashSet<()>".to_string()
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
            format!("impl Fn({}) -> {}", inputs_str.join(", "), output_str)
        }
        // Phase 43C: Refinement types use the base type for Rust type annotation
        // The constraint predicate is handled separately via debug_assert!
        TypeExpr::Refinement { base, .. } => {
            codegen_type_expr(base, interner)
        }
        // Phase 53: Persistent storage wrapper
        TypeExpr::Persistent { inner } => {
            let inner_type = codegen_type_expr(inner, interner);
            format!("logicaffeine_system::storage::Persistent<{}>", inner_type)
        }
    }
}

/// Infer return type from function body by looking at Return statements.
pub(super) fn infer_return_type_from_body(body: &[Stmt], _interner: &Interner) -> Option<String> {
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
pub(super) fn map_type_to_rust(ty: &str) -> String {
    match ty {
        "Int" => "i64".to_string(),
        "Nat" => "u64".to_string(),
        "Text" => "String".to_string(),
        "Bool" | "Boolean" => "bool".to_string(),
        "Real" | "Float" => "f64".to_string(),
        "Char" => "char".to_string(),
        "Byte" => "u8".to_string(),
        "Unit" | "()" => "()".to_string(),
        "Duration" => "std::time::Duration".to_string(),
        other => other.to_string(),
    }
}

/// Generate a single struct definition with derives and visibility.
/// Phase 34: Now supports generic type parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now supports is_shared for CRDT Merge impl.
pub(super) fn codegen_struct_def(name: Symbol, fields: &[FieldDef], generics: &[Symbol], is_portable: bool, is_shared: bool, interner: &Interner, indent: usize, c_abi_value_structs: &HashSet<Symbol>, c_abi_ref_structs: &HashSet<Symbol>) -> String {
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

    // Value-type structs used in C ABI exports need #[repr(C)] for stable field layout
    if c_abi_value_structs.contains(&name) {
        writeln!(output, "{}#[repr(C)]", ind).unwrap();
    }

    // Phase 47: Add Serialize, Deserialize derives if portable
    // Phase 50: Add PartialEq for policy equality comparisons
    // Phase 52: Shared types also need Serialize/Deserialize for Synced<T>
    // C ABI reference-type structs also need serde for from_json/to_json support
    if is_portable || is_shared || c_abi_ref_structs.contains(&name) {
        writeln!(output, "{}#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]", ind).unwrap();
    } else {
        writeln!(output, "{}#[derive(Default, Debug, Clone, PartialEq)]", ind).unwrap();
    }
    writeln!(output, "{}pub struct {}{} {{", ind, interner.resolve(name), generic_str).unwrap();

    for field in fields {
        let vis = if field.is_public { "pub " } else { "" };
        let rust_type = codegen_field_type(&field.ty, interner);
        writeln!(output, "{}    {}{}: {},", ind, vis, interner.resolve(field.name), rust_type).unwrap();
    }

    writeln!(output, "{}}}\n", ind).unwrap();

    // Phase 49: Generate Merge impl for Shared structs
    if is_shared {
        output.push_str(&codegen_merge_impl(name, fields, generics, interner, indent));
    }

    output
}

/// Phase 49: Generate impl Merge for a Shared struct.
pub(super) fn codegen_merge_impl(name: Symbol, fields: &[FieldDef], generics: &[Symbol], interner: &Interner, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let name_str = interner.resolve(name);
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

    writeln!(output, "{}impl{} logicaffeine_data::crdt::Merge for {}{} {{", ind, generic_str, name_str, generic_str).unwrap();
    writeln!(output, "{}    fn merge(&mut self, other: &Self) {{", ind).unwrap();

    for field in fields {
        let field_name = interner.resolve(field.name);
        // Only merge fields that implement Merge (CRDT types)
        if is_crdt_field_type(&field.ty, interner) {
            writeln!(output, "{}        self.{}.merge(&other.{});", ind, field_name, field_name).unwrap();
        }
    }

    writeln!(output, "{}    }}", ind).unwrap();
    writeln!(output, "{}}}\n", ind).unwrap();

    output
}

/// Phase 49: Check if a field type is a CRDT type that implements Merge.
pub(super) fn is_crdt_field_type(ty: &FieldType, interner: &Interner) -> bool {
    match ty {
        FieldType::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(name,
                "ConvergentCount" | "GCounter" |
                "Tally" | "PNCounter"
            )
        }
        FieldType::Generic { base, .. } => {
            let name = interner.resolve(*base);
            matches!(name,
                "LastWriteWins" | "LWWRegister" |
                "SharedSet" | "ORSet" | "SharedSet_AddWins" | "SharedSet_RemoveWins" |
                "SharedSequence" | "RGA" | "SharedSequence_YATA" | "CollaborativeSequence" |
                "SharedMap" | "ORMap" |
                "Divergent" | "MVRegister"
            )
        }
        _ => false,
    }
}

/// Phase 33/34: Generate enum definition with optional generic parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now accepts is_shared parameter (enums don't generate Merge impl yet).
pub(super) fn codegen_enum_def(name: Symbol, variants: &[VariantDef], generics: &[Symbol], is_portable: bool, _is_shared: bool, interner: &Interner, indent: usize) -> String {
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

    // Phase 47: Add Serialize, Deserialize derives if portable
    if is_portable {
        writeln!(output, "{}#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]", ind).unwrap();
    } else {
        writeln!(output, "{}#[derive(Debug, Clone, PartialEq)]", ind).unwrap();
    }
    writeln!(output, "{}pub enum {}{} {{", ind, interner.resolve(name), generic_str).unwrap();

    for variant in variants {
        let variant_name = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            // Unit variant
            writeln!(output, "{}    {},", ind, variant_name).unwrap();
        } else {
            // Struct variant with named fields
            // Phase 102: Detect and box recursive fields
            let enum_name_str = interner.resolve(name);
            let fields_str: Vec<String> = variant.fields.iter()
                .map(|f| {
                    let rust_type = codegen_field_type(&f.ty, interner);
                    let field_name = interner.resolve(f.name);
                    // Check if this field references the enum itself (recursive type)
                    if is_recursive_field(&f.ty, enum_name_str, interner) {
                        format!("{}: Box<{}>", field_name, rust_type)
                    } else {
                        format!("{}: {}", field_name, rust_type)
                    }
                })
                .collect();
            writeln!(output, "{}    {} {{ {} }},", ind, variant_name, fields_str.join(", ")).unwrap();
        }
    }

    writeln!(output, "{}}}\n", ind).unwrap();

    // Generate Default impl for enum (defaults to first variant)
    // This is needed when the enum is used as a struct field and the struct derives Default
    // Only for non-generic enums — generic enums can't assume their type params implement Default
    if generics.is_empty() {
    if let Some(first_variant) = variants.first() {
        let enum_name_str = interner.resolve(name);
        let first_variant_name = interner.resolve(first_variant.name);
        writeln!(output, "{}impl{} Default for {}{} {{", ind, generic_str, enum_name_str, generic_str).unwrap();
        writeln!(output, "{}    fn default() -> Self {{", ind).unwrap();
        if first_variant.fields.is_empty() {
            writeln!(output, "{}        {}::{}", ind, enum_name_str, first_variant_name).unwrap();
        } else {
            // Default with default field values
            let default_fields: Vec<String> = first_variant.fields.iter()
                .map(|f| {
                    let field_name = interner.resolve(f.name);
                    let enum_name_check = interner.resolve(name);
                    if is_recursive_field(&f.ty, enum_name_check, interner) {
                        format!("{}: Box::new(Default::default())", field_name)
                    } else {
                        format!("{}: Default::default()", field_name)
                    }
                })
                .collect();
            writeln!(output, "{}        {}::{} {{ {} }}", ind, enum_name_str, first_variant_name, default_fields.join(", ")).unwrap();
        }
        writeln!(output, "{}    }}", ind).unwrap();
        writeln!(output, "{}}}\n", ind).unwrap();
    }
    }

    output
}

/// Convert FieldType to Rust type string.
pub(super) fn codegen_field_type(ty: &FieldType, interner: &Interner) -> String {
    match ty {
        FieldType::Primitive(sym) => {
            match interner.resolve(*sym) {
                "Int" => "i64".to_string(),
                "Nat" => "u64".to_string(),
                "Text" => "String".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Real" | "Float" => "f64".to_string(),
                "Char" => "char".to_string(),
                "Byte" => "u8".to_string(),
                "Unit" => "()".to_string(),
                "Duration" => "std::time::Duration".to_string(),
                other => other.to_string(),
            }
        }
        FieldType::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                // Phase 49: CRDT type mapping
                "ConvergentCount" => "logicaffeine_data::crdt::GCounter".to_string(),
                // Phase 49b: New CRDT types (Wave 5)
                "Tally" => "logicaffeine_data::crdt::PNCounter".to_string(),
                _ => name.to_string(),
            }
        }
        FieldType::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let param_strs: Vec<String> = params.iter()
                .map(|p| codegen_field_type(p, interner))
                .collect();

            // Phase 49c: Handle CRDT types with bias/algorithm modifiers
            match base_name {
                // SharedSet with explicit bias
                "SharedSet_RemoveWins" => {
                    return format!("logicaffeine_data::crdt::ORSet<{}, logicaffeine_data::crdt::RemoveWins>", param_strs.join(", "));
                }
                "SharedSet_AddWins" => {
                    return format!("logicaffeine_data::crdt::ORSet<{}, logicaffeine_data::crdt::AddWins>", param_strs.join(", "));
                }
                // SharedSequence with YATA algorithm
                "SharedSequence_YATA" | "CollaborativeSequence" => {
                    return format!("logicaffeine_data::crdt::YATA<{}>", param_strs.join(", "));
                }
                _ => {}
            }

            let base_str = match base_name {
                "List" | "Seq" => "Vec",
                "Set" => "std::collections::HashSet",
                "Map" => "std::collections::HashMap",
                "Option" | "Maybe" => "Option",
                "Result" => "Result",
                // Phase 49: CRDT generic type
                "LastWriteWins" => "logicaffeine_data::crdt::LWWRegister",
                // Phase 49b: New CRDT generic types (Wave 5) - default to AddWins for ORSet
                "SharedSet" | "ORSet" => "logicaffeine_data::crdt::ORSet",
                "SharedSequence" | "RGA" => "logicaffeine_data::crdt::RGA",
                "SharedMap" | "ORMap" => "logicaffeine_data::crdt::ORMap",
                "Divergent" | "MVRegister" => "logicaffeine_data::crdt::MVRegister",
                other => other,
            };
            format!("{}<{}>", base_str, param_strs.join(", "))
        }
        // Phase 34: Type parameter reference (T, U, etc.)
        FieldType::TypeParam(sym) => interner.resolve(*sym).to_string(),
    }
}

/// Phase 102: Check if a field type references the containing enum (recursive type).
/// Recursive types need to be wrapped in Box<T> for Rust to know the size.
pub(crate) fn is_recursive_field(ty: &FieldType, enum_name: &str, interner: &Interner) -> bool {
    match ty {
        FieldType::Primitive(sym) => interner.resolve(*sym) == enum_name,
        FieldType::Named(sym) => interner.resolve(*sym) == enum_name,
        FieldType::TypeParam(_) => false,
        FieldType::Generic { base, params } => {
            // Check if base matches or any type parameter contains the enum
            interner.resolve(*base) == enum_name ||
            params.iter().any(|p| is_recursive_field(p, enum_name, interner))
        }
    }
}

/// Phase 103: Infer type annotation for multi-param generic enum variants.
/// Returns Some(type_annotation) if the enum has multiple type params, None otherwise.
pub(super) fn infer_variant_type_annotation(
    expr: &Expr,
    registry: &TypeRegistry,
    interner: &Interner,
) -> Option<String> {
    // Only handle NewVariant expressions
    let (enum_name, variant_name, field_values) = match expr {
        Expr::NewVariant { enum_name, variant, fields } => (*enum_name, *variant, fields),
        _ => return None,
    };

    // Look up the enum in the registry
    let enum_def = registry.get(enum_name)?;
    let (generics, variants) = match enum_def {
        TypeDef::Enum { generics, variants, .. } => (generics, variants),
        _ => return None,
    };

    // Only generate type annotations for multi-param generics
    if generics.len() < 2 {
        return None;
    }

    // Find the variant definition
    let variant_def = variants.iter().find(|v| v.name == variant_name)?;

    // Collect which type params are bound by which field types
    let mut type_param_types: HashMap<Symbol, String> = HashMap::new();
    for (field_name, field_value) in field_values {
        // Find the field in the variant definition
        if let Some(field_def) = variant_def.fields.iter().find(|f| f.name == *field_name) {
            // If the field type is a type parameter, infer its type from the value
            if let FieldType::TypeParam(type_param) = &field_def.ty {
                let inferred = infer_rust_type_from_expr(field_value, interner);
                type_param_types.insert(*type_param, inferred);
            }
        }
    }

    // Build the type annotation: EnumName<T1, T2, ...>
    // For bound params, use the inferred type; for unbound, use ()
    let enum_str = interner.resolve(enum_name);
    let param_strs: Vec<String> = generics.iter()
        .map(|g| {
            type_param_types.get(g)
                .cloned()
                .unwrap_or_else(|| "()".to_string())
        })
        .collect();

    Some(format!("{}<{}>", enum_str, param_strs.join(", ")))
}

/// Infer Rust type string from a LOGOS expression.
/// Delegates to `LogosType::from_literal()` for literals.
pub(super) fn infer_rust_type_from_expr(expr: &Expr, _interner: &Interner) -> String {
    match expr {
        Expr::Literal(lit) => {
            let ty = crate::analysis::types::LogosType::from_literal(lit);
            ty.to_rust_type()
        }
        _ => "_".to_string(),
    }
}

/// Infer the numeric type of an expression for mixed Float*Int arithmetic coercion.
///
/// Follows the standard numeric promotion rule (Z embeds into R):
/// if either operand of an arithmetic operation is f64, the result is f64.
/// Returns "i64", "f64", or "unknown".
///
/// Delegates to a temporary TypeEnv built from variable_types for inference.
pub(super) fn infer_numeric_type(
    expr: &Expr,
    interner: &Interner,
    variable_types: &HashMap<Symbol, String>,
) -> &'static str {
    // Build a temporary TypeEnv from the string-based variable types
    let mut env = crate::analysis::types::TypeEnv::new();
    for (sym, ty_str) in variable_types {
        let ty = crate::analysis::types::LogosType::from_rust_type_str(ty_str);
        env.register(*sym, ty);
    }
    let inferred = env.infer_expr(expr, interner);
    match inferred {
        crate::analysis::types::LogosType::Int => "i64",
        crate::analysis::types::LogosType::Float => "f64",
        _ => "unknown",
    }
}
