use std::collections::HashMap;

use crate::analysis::TypeRegistry;
use crate::ast::stmt::*;
use crate::intern::{Interner, Symbol};

pub(super) fn is_c_reserved(name: &str) -> bool {
    matches!(name,
        // C keywords
        "auto" | "break" | "case" | "char" | "const" | "continue" | "default" |
        "do" | "double" | "else" | "enum" | "extern" | "float" | "for" | "goto" |
        "if" | "int" | "long" | "register" | "return" | "short" | "signed" |
        "sizeof" | "static" | "struct" | "switch" | "typedef" | "union" |
        "unsigned" | "void" | "volatile" | "while" |
        // C99
        "inline" | "restrict" | "_Bool" | "_Complex" | "_Imaginary" |
        // C11
        "_Alignas" | "_Alignof" | "_Atomic" | "_Generic" | "_Noreturn" |
        "_Static_assert" | "_Thread_local" |
        // C23
        "bool" | "true" | "false" | "nullptr" | "alignas" | "alignof" |
        "constexpr" | "static_assert" | "thread_local" | "typeof" |
        // Standard library types/functions that we use in the runtime
        "printf" | "malloc" | "calloc" | "realloc" | "free" | "memcpy" |
        "strlen" | "strdup" | "snprintf" | "atoll" | "atof" |
        // POSIX common
        "size_t" | "ssize_t" | "ptrdiff_t" | "intptr_t" |
        // Our runtime identifiers (prevent user collision)
        "main" | "argc" | "argv"
    )
}

pub(super) fn escape_c_ident(name: &str) -> String {
    if is_c_reserved(name) {
        format!("logos_{}", name)
    } else {
        name.to_string()
    }
}

// =============================================================================
// C Type System
// =============================================================================

#[derive(Clone, Debug, PartialEq)]
pub(super) enum CType {
    Int64,
    Float64,
    Bool,
    String,
    SeqI64,
    SeqBool,
    SeqStr,
    SeqF64,
    MapI64I64,
    MapStrI64,
    MapStrStr,
    MapI64Str,
    SetI64,
    SetStr,
    Struct(Symbol),
    Enum(Symbol),
    Void,
}

pub(super) fn c_type_str(ty: &CType) -> &'static str {
    match ty {
        CType::Int64 => "int64_t",
        CType::Float64 => "double",
        CType::Bool => "bool",
        CType::String => "char *",
        CType::SeqI64 => "Seq_i64",
        CType::SeqBool => "Seq_bool",
        CType::SeqStr => "Seq_str",
        CType::SeqF64 => "Seq_f64",
        CType::MapI64I64 => "Map_i64_i64",
        CType::MapStrI64 => "Map_str_i64",
        CType::MapStrStr => "Map_str_str",
        CType::MapI64Str => "Map_i64_str",
        CType::SetI64 => "Set_i64",
        CType::SetStr => "Set_str",
        CType::Struct(_) | CType::Enum(_) => "/* user type */",
        CType::Void => "void",
    }
}

pub(super) fn c_type_str_resolved(ty: &CType, interner: &Interner) -> String {
    match ty {
        CType::Struct(sym) | CType::Enum(sym) => escape_c_ident(interner.resolve(*sym)),
        _ => c_type_str(ty).to_string(),
    }
}

pub(super) fn field_type_to_ctype(ft: &crate::analysis::FieldType, interner: &Interner, registry: &TypeRegistry) -> CType {
    match ft {
        crate::analysis::FieldType::Primitive(sym) | crate::analysis::FieldType::Named(sym) => {
            match interner.resolve(*sym) {
                "Int" | "Nat" => CType::Int64,
                "Float" => CType::Float64,
                "Bool" => CType::Bool,
                "Text" => CType::String,
                _ => {
                    match registry.get(*sym) {
                        Some(crate::analysis::TypeDef::Struct { .. }) => CType::Struct(*sym),
                        Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(*sym),
                        _ => CType::Int64,
                    }
                }
            }
        }
        crate::analysis::FieldType::Generic { .. } => CType::Int64,
        crate::analysis::FieldType::TypeParam(_) => CType::Int64,
    }
}

pub(super) fn resolve_type_expr(ty: &TypeExpr, interner: &Interner) -> CType {
    resolve_type_expr_with_registry(ty, interner, None)
}

pub(super) fn resolve_type_expr_with_registry(ty: &TypeExpr, interner: &Interner, registry: Option<&TypeRegistry>) -> CType {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            match interner.resolve(*sym) {
                "Int" | "Nat" => CType::Int64,
                "Float" => CType::Float64,
                "Bool" => CType::Bool,
                "Text" => CType::String,
                _ => {
                    if let Some(reg) = registry {
                        match reg.get(*sym) {
                            Some(crate::analysis::TypeDef::Struct { .. }) => CType::Struct(*sym),
                            Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(*sym),
                            _ => CType::Int64,
                        }
                    } else {
                        CType::Int64
                    }
                }
            }
        }
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            match base_name {
                "Seq" | "List" => {
                    if let Some(elem) = params.first() {
                        match resolve_type_expr(elem, interner) {
                            CType::Bool => CType::SeqBool,
                            CType::String => CType::SeqStr,
                            CType::Float64 => CType::SeqF64,
                            _ => CType::SeqI64,
                        }
                    } else {
                        CType::SeqI64
                    }
                }
                "Map" => {
                    let key_type = params.first().map(|p| resolve_type_expr(p, interner)).unwrap_or(CType::Int64);
                    let val_type = params.get(1).map(|p| resolve_type_expr(p, interner)).unwrap_or(CType::Int64);
                    match (&key_type, &val_type) {
                        (CType::String, CType::Int64) => CType::MapStrI64,
                        (CType::String, CType::String) => CType::MapStrStr,
                        (CType::Int64, CType::String) => CType::MapI64Str,
                        _ => CType::MapI64I64,
                    }
                }
                "Set" => {
                    if let Some(elem) = params.first() {
                        match resolve_type_expr(elem, interner) {
                            CType::String => CType::SetStr,
                            _ => CType::SetI64,
                        }
                    } else {
                        CType::SetI64
                    }
                }
                _ => CType::Int64,
            }
        }
        _ => CType::Int64,
    }
}

// =============================================================================
// Codegen Context
// =============================================================================

pub(super) struct CContext<'a> {
    pub(super) vars: HashMap<Symbol, CType>,
    pub(super) funcs: HashMap<Symbol, CType>,
    pub(super) interner: &'a Interner,
    pub(super) registry: &'a TypeRegistry,
}

impl<'a> CContext<'a> {
    pub(super) fn new(interner: &'a Interner, registry: &'a TypeRegistry) -> Self {
        Self {
            vars: HashMap::new(),
            funcs: HashMap::new(),
            interner,
            registry,
        }
    }

    pub(super) fn resolve(&self, sym: Symbol) -> String {
        escape_c_ident(self.interner.resolve(sym))
    }
}

// =============================================================================
// Type Inference
// =============================================================================

pub(super) fn infer_expr_type(expr: &Expr, ctx: &CContext) -> CType {
    match expr {
        Expr::Literal(Literal::Number(_)) => CType::Int64,
        Expr::Literal(Literal::Float(_)) => CType::Float64,
        Expr::Literal(Literal::Boolean(_)) => CType::Bool,
        Expr::Literal(Literal::Text(_)) => CType::String,
        Expr::Literal(Literal::Nothing) => CType::Void,
        Expr::Literal(_) => CType::Int64,
        Expr::Identifier(sym) => ctx.vars.get(sym).cloned().unwrap_or(CType::Int64),
        Expr::BinaryOp { op, left, right } => {
            match op {
                BinaryOpKind::Eq | BinaryOpKind::NotEq
                | BinaryOpKind::Lt | BinaryOpKind::LtEq
                | BinaryOpKind::Gt | BinaryOpKind::GtEq
                | BinaryOpKind::And | BinaryOpKind::Or => CType::Bool,
                BinaryOpKind::Concat => CType::String,
                BinaryOpKind::BitXor | BinaryOpKind::Shl | BinaryOpKind::Shr => CType::Int64,
                BinaryOpKind::Add | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply | BinaryOpKind::Divide
                | BinaryOpKind::Modulo => {
                    let lt = infer_expr_type(left, ctx);
                    let rt = infer_expr_type(right, ctx);
                    if lt == CType::String || rt == CType::String {
                        CType::String
                    } else if lt == CType::Float64 || rt == CType::Float64 {
                        CType::Float64
                    } else {
                        CType::Int64
                    }
                }
            }
        }
        Expr::Call { function, .. } => ctx.funcs.get(function).cloned().unwrap_or(CType::Int64),
        Expr::CallExpr { .. } => CType::Int64,
        Expr::Length { .. } => CType::Int64,
        Expr::Index { collection, .. } => {
            if let Expr::Identifier(sym) = collection {
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => CType::Int64,
                    Some(CType::SeqBool) => CType::Bool,
                    Some(CType::SeqStr) => CType::String,
                    Some(CType::SeqF64) => CType::Float64,
                    Some(CType::MapI64I64) => CType::Int64,
                    Some(CType::MapStrI64) => CType::Int64,
                    Some(CType::MapStrStr) => CType::String,
                    Some(CType::MapI64Str) => CType::String,
                    _ => CType::Int64,
                }
            } else {
                CType::Int64
            }
        }
        Expr::New { type_name, type_args, .. } => {
            let name = ctx.interner.resolve(*type_name);
            match name {
                "Seq" | "List" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::Bool => CType::SeqBool,
                            CType::String => CType::SeqStr,
                            CType::Float64 => CType::SeqF64,
                            _ => CType::SeqI64,
                        }
                    } else {
                        CType::SeqI64
                    }
                }
                "Map" => {
                    let key_type = type_args.first().map(|p| resolve_type_expr(p, ctx.interner)).unwrap_or(CType::Int64);
                    let val_type = type_args.get(1).map(|p| resolve_type_expr(p, ctx.interner)).unwrap_or(CType::Int64);
                    match (&key_type, &val_type) {
                        (CType::String, CType::Int64) => CType::MapStrI64,
                        (CType::String, CType::String) => CType::MapStrStr,
                        (CType::Int64, CType::String) => CType::MapI64Str,
                        _ => CType::MapI64I64,
                    }
                }
                "Set" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::String => CType::SetStr,
                            _ => CType::SetI64,
                        }
                    } else {
                        CType::SetI64
                    }
                }
                _ => {
                    match ctx.registry.get(*type_name) {
                        Some(crate::analysis::TypeDef::Struct { .. }) => CType::Struct(*type_name),
                        Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(*type_name),
                        _ => CType::Int64,
                    }
                }
            }
        }
        Expr::WithCapacity { value, .. } => infer_expr_type(value, ctx),
        Expr::Copy { expr: inner } => infer_expr_type(inner, ctx),
        Expr::List(elems) => {
            if let Some(first) = elems.first() {
                match infer_expr_type(first, ctx) {
                    CType::Bool => CType::SeqBool,
                    CType::String => CType::SeqStr,
                    CType::Float64 => CType::SeqF64,
                    _ => CType::SeqI64,
                }
            } else {
                CType::SeqI64
            }
        }
        Expr::Contains { .. } => CType::Bool,
        Expr::Give { value } => infer_expr_type(value, ctx),
        Expr::Slice { collection, .. } => infer_expr_type(collection, ctx),
        Expr::FieldAccess { object, field } => {
            let obj_type = infer_expr_type(object, ctx);
            if let CType::Struct(sym) = obj_type {
                if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = ctx.registry.get(sym) {
                    for f in fields {
                        if f.name == *field {
                            return field_type_to_ctype(&f.ty, ctx.interner, ctx.registry);
                        }
                    }
                }
            }
            CType::Int64
        }
        Expr::InterpolatedString(_) => CType::String,
        Expr::NewVariant { enum_name, .. } => CType::Enum(*enum_name),
        Expr::OptionSome { .. } | Expr::OptionNone => CType::Int64,
        _ => CType::Int64,
    }
}
