//! Type inference pass for the LOGOS compilation pipeline.
//!
//! Provides a structured type representation (`LogosType`) and a type environment
//! (`TypeEnv`) that replaces the ad-hoc string-based type tracking previously
//! scattered across codegen.rs.
//!
//! # Pipeline Position
//!
//! ```text
//! Parse → TypeInfer → Optimize → Codegen
//!              ^ THIS MODULE
//! ```
//!
//! The `TypeEnv` is computed once from the AST and passed immutably to codegen,
//! replacing `variable_types: HashMap<Symbol, String>` and `string_vars: HashSet<Symbol>`.

use std::collections::HashMap;

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::analysis::TypeRegistry;

/// Structured type representation for LOGOS values.
///
/// Replaces string-based type tracking (e.g., `"Vec<i64>"`, `"String"`)
/// with a proper algebraic data type that supports precise queries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogosType {
    Int,
    Float,
    Bool,
    Char,
    Byte,
    String,
    Unit,
    Seq(Box<LogosType>),
    Map(Box<LogosType>, Box<LogosType>),
    Set(Box<LogosType>),
    Option(Box<LogosType>),
    Duration,
    Date,
    Moment,
    Time,
    Span,
    Nat,
    UserDefined(Symbol),
    Unknown,
}

/// Function signature for tracked functions.
#[derive(Debug, Clone)]
pub struct FnSig {
    pub params: Vec<(Symbol, LogosType)>,
    pub return_type: LogosType,
}

/// Type environment built by a forward pass over the AST.
///
/// Computed once before codegen and shared immutably. Replaces the
/// `variable_types: HashMap<Symbol, String>` and `string_vars: HashSet<Symbol>`
/// that were incrementally built during codegen.
#[derive(Debug)]
pub struct TypeEnv {
    variables: HashMap<Symbol, LogosType>,
    functions: HashMap<Symbol, FnSig>,
}

impl LogosType {
    /// Whether this type is Copy in Rust (no .clone() needed).
    ///
    /// This is the single source of truth for clone decisions,
    /// replacing `is_copy_type()`, `has_copy_element_type()`, `has_copy_value_type()`.
    pub fn is_copy(&self) -> bool {
        matches!(
            self,
            LogosType::Int
                | LogosType::Float
                | LogosType::Bool
                | LogosType::Char
                | LogosType::Byte
                | LogosType::Unit
                | LogosType::Nat
        )
    }

    /// Whether this type is numeric (Int or Float).
    pub fn is_numeric(&self) -> bool {
        matches!(self, LogosType::Int | LogosType::Float | LogosType::Nat)
    }

    /// Whether this type is a string.
    pub fn is_string(&self) -> bool {
        matches!(self, LogosType::String)
    }

    /// Whether this type is a float.
    pub fn is_float(&self) -> bool {
        matches!(self, LogosType::Float)
    }

    /// Get the element type for sequences and sets.
    pub fn element_type(&self) -> Option<&LogosType> {
        match self {
            LogosType::Seq(inner) | LogosType::Set(inner) => Some(inner),
            _ => None,
        }
    }

    /// Get the value type for maps.
    pub fn value_type(&self) -> Option<&LogosType> {
        match self {
            LogosType::Map(_, v) => Some(v),
            _ => None,
        }
    }

    /// Get the key type for maps.
    pub fn key_type(&self) -> Option<&LogosType> {
        match self {
            LogosType::Map(k, _) => Some(k),
            _ => None,
        }
    }

    /// Numeric promotion: Z embeds into R.
    /// If either operand is Float, the result is Float.
    /// If both are Int (or Nat), the result is Int.
    pub fn numeric_promotion(a: &LogosType, b: &LogosType) -> LogosType {
        if a.is_float() || b.is_float() {
            LogosType::Float
        } else if a.is_numeric() && b.is_numeric() {
            LogosType::Int
        } else {
            LogosType::Unknown
        }
    }

    /// Convert to the Rust type string used in codegen output.
    ///
    /// Replaces all ad-hoc string-based type conversions scattered
    /// across codegen.rs. This is the single point of truth for
    /// LogosType → Rust type string mapping.
    pub fn to_rust_type(&self) -> std::string::String {
        match self {
            LogosType::Int => "i64".into(),
            LogosType::Float => "f64".into(),
            LogosType::Bool => "bool".into(),
            LogosType::Char => "char".into(),
            LogosType::Byte => "u8".into(),
            LogosType::String => "String".into(),
            LogosType::Unit => "()".into(),
            LogosType::Nat => "u64".into(),
            LogosType::Duration => "std::time::Duration".into(),
            LogosType::Date => "LogosDate".into(),
            LogosType::Moment => "LogosMoment".into(),
            LogosType::Time => "LogosTime".into(),
            LogosType::Span => "LogosSpan".into(),
            LogosType::Seq(inner) => format!("Vec<{}>", inner.to_rust_type()),
            LogosType::Map(k, v) => format!(
                "std::collections::HashMap<{}, {}>",
                k.to_rust_type(),
                v.to_rust_type()
            ),
            LogosType::Set(inner) => {
                format!("std::collections::HashSet<{}>", inner.to_rust_type())
            }
            LogosType::Option(inner) => format!("Option<{}>", inner.to_rust_type()),
            LogosType::UserDefined(_) => "_".into(),
            LogosType::Unknown => "_".into(),
        }
    }

    /// Build a LogosType from a TypeExpr AST node.
    pub fn from_type_expr(ty: &TypeExpr, interner: &Interner) -> LogosType {
        match ty {
            TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                Self::from_type_name(interner.resolve(*sym))
            }
            TypeExpr::Generic { base, params } => {
                let base_name = interner.resolve(*base);
                match base_name {
                    "Seq" | "List" | "Vec" => {
                        let elem = params
                            .first()
                            .map(|p| LogosType::from_type_expr(p, interner))
                            .unwrap_or(LogosType::Unit);
                        LogosType::Seq(Box::new(elem))
                    }
                    "Map" | "HashMap" => {
                        let key = params
                            .first()
                            .map(|p| LogosType::from_type_expr(p, interner))
                            .unwrap_or(LogosType::String);
                        let val = params
                            .get(1)
                            .map(|p| LogosType::from_type_expr(p, interner))
                            .unwrap_or(LogosType::String);
                        LogosType::Map(Box::new(key), Box::new(val))
                    }
                    "Set" | "HashSet" => {
                        let elem = params
                            .first()
                            .map(|p| LogosType::from_type_expr(p, interner))
                            .unwrap_or(LogosType::Unit);
                        LogosType::Set(Box::new(elem))
                    }
                    "Option" | "Maybe" => {
                        let inner = params
                            .first()
                            .map(|p| LogosType::from_type_expr(p, interner))
                            .unwrap_or(LogosType::Unit);
                        LogosType::Option(Box::new(inner))
                    }
                    _ => LogosType::Unknown,
                }
            }
            TypeExpr::Refinement { base, .. } => LogosType::from_type_expr(base, interner),
            TypeExpr::Persistent { inner } => LogosType::from_type_expr(inner, interner),
            TypeExpr::Function { .. } => LogosType::Unknown,
        }
    }

    /// Parse a LOGOS type name string into a LogosType.
    fn from_type_name(name: &str) -> LogosType {
        match name {
            "Int" => LogosType::Int,
            "Nat" => LogosType::Nat,
            "Real" | "Float" => LogosType::Float,
            "Bool" | "Boolean" => LogosType::Bool,
            "Text" | "String" => LogosType::String,
            "Char" => LogosType::Char,
            "Byte" => LogosType::Byte,
            "Unit" | "()" => LogosType::Unit,
            "Duration" => LogosType::Duration,
            "Date" => LogosType::Date,
            "Moment" => LogosType::Moment,
            "Time" => LogosType::Time,
            "Span" => LogosType::Span,
            _ => LogosType::Unknown,
        }
    }

    /// Parse a Rust type string back into a LogosType (legacy bridge).
    /// Handles strings like "i64", "f64", "String", "Vec<i64>",
    /// "std::collections::HashMap<String, i64>", etc.
    pub fn from_rust_type_str(s: &str) -> LogosType {
        match s {
            "i64" => LogosType::Int,
            "u64" => LogosType::Nat,
            "f64" => LogosType::Float,
            "bool" => LogosType::Bool,
            "char" => LogosType::Char,
            "u8" => LogosType::Byte,
            "String" => LogosType::String,
            "()" => LogosType::Unit,
            "std::time::Duration" => LogosType::Duration,
            "LogosDate" => LogosType::Date,
            "LogosMoment" => LogosType::Moment,
            "LogosTime" => LogosType::Time,
            "LogosSpan" => LogosType::Span,
            _ => {
                // Try to parse generic types
                if let Some(inner) = s.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
                    LogosType::Seq(Box::new(Self::from_rust_type_str(inner)))
                } else if let Some(inner) = s
                    .strip_prefix("std::collections::HashMap<")
                    .or_else(|| s.strip_prefix("HashMap<"))
                    .and_then(|s| s.strip_suffix('>'))
                {
                    if let Some((key, val)) = inner.split_once(", ") {
                        LogosType::Map(
                            Box::new(Self::from_rust_type_str(key)),
                            Box::new(Self::from_rust_type_str(val)),
                        )
                    } else {
                        LogosType::Unknown
                    }
                } else if let Some(inner) = s
                    .strip_prefix("std::collections::HashSet<")
                    .or_else(|| s.strip_prefix("HashSet<"))
                    .and_then(|s| s.strip_suffix('>'))
                {
                    LogosType::Set(Box::new(Self::from_rust_type_str(inner)))
                } else if let Some(inner) = s.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
                    LogosType::Option(Box::new(Self::from_rust_type_str(inner)))
                } else {
                    LogosType::Unknown
                }
            }
        }
    }

    /// Infer a LogosType from a literal expression.
    pub fn from_literal(lit: &Literal) -> LogosType {
        match lit {
            Literal::Number(_) => LogosType::Int,
            Literal::Float(_) => LogosType::Float,
            Literal::Text(_) => LogosType::String,
            Literal::Boolean(_) => LogosType::Bool,
            Literal::Char(_) => LogosType::Char,
            Literal::Nothing => LogosType::Unit,
            Literal::Duration(_) => LogosType::Duration,
            Literal::Date(_) => LogosType::Date,
            Literal::Moment(_) => LogosType::Moment,
            Literal::Span { .. } => LogosType::Span,
            Literal::Time(_) => LogosType::Time,
        }
    }
}

impl TypeEnv {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    /// Look up the type of a variable.
    pub fn lookup(&self, sym: Symbol) -> &LogosType {
        self.variables.get(&sym).unwrap_or(&LogosType::Unknown)
    }

    /// Look up a function signature.
    pub fn lookup_fn(&self, sym: Symbol) -> Option<&FnSig> {
        self.functions.get(&sym)
    }

    /// Register a variable with its type.
    pub fn register(&mut self, sym: Symbol, ty: LogosType) {
        self.variables.insert(sym, ty);
    }

    /// Register a function signature.
    pub fn register_fn(&mut self, sym: Symbol, sig: FnSig) {
        self.functions.insert(sym, sig);
    }

    /// Infer the type of an expression given the current environment.
    pub fn infer_expr(&self, expr: &Expr, interner: &Interner) -> LogosType {
        match expr {
            Expr::Literal(lit) => LogosType::from_literal(lit),

            Expr::Identifier(sym) => self.lookup(*sym).clone(),

            Expr::BinaryOp { op, left, right } => {
                match op {
                    // Comparison operators always produce Bool
                    BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
                    | BinaryOpKind::Lt
                    | BinaryOpKind::Gt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::GtEq => LogosType::Bool,

                    // Logical operators produce Bool
                    BinaryOpKind::And | BinaryOpKind::Or => LogosType::Bool,

                    // Concat always produces String
                    BinaryOpKind::Concat => LogosType::String,

                    // Add: could be numeric or string concatenation
                    BinaryOpKind::Add => {
                        let lt = self.infer_expr(left, interner);
                        let rt = self.infer_expr(right, interner);
                        if lt.is_string() || rt.is_string() {
                            LogosType::String
                        } else {
                            LogosType::numeric_promotion(&lt, &rt)
                        }
                    }

                    // Other arithmetic: numeric promotion
                    BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
                    | BinaryOpKind::Modulo => {
                        let lt = self.infer_expr(left, interner);
                        let rt = self.infer_expr(right, interner);
                        LogosType::numeric_promotion(&lt, &rt)
                    }
                }
            }

            Expr::Length { .. } => LogosType::Int,

            Expr::Call { function, args } => {
                let name = interner.resolve(*function);
                match name {
                    "sqrt" | "parseFloat" | "pow" => LogosType::Float,
                    "parseInt" | "floor" | "ceil" | "round" => LogosType::Int,
                    "abs" | "min" | "max" => {
                        // Preserves type of arguments — infer from first arg
                        if let Some(first) = args.first() {
                            self.infer_expr(first, interner)
                        } else {
                            LogosType::Unknown
                        }
                    }
                    _ => {
                        // Look up function signature
                        if let Some(sig) = self.lookup_fn(*function) {
                            sig.return_type.clone()
                        } else {
                            LogosType::Unknown
                        }
                    }
                }
            }

            Expr::Index { collection, .. } => {
                let coll_ty = self.infer_expr(collection, interner);
                match &coll_ty {
                    LogosType::Seq(inner) => (**inner).clone(),
                    LogosType::Map(_, v) => (**v).clone(),
                    _ => LogosType::Unknown,
                }
            }

            Expr::List(items) => {
                let elem_type = items
                    .first()
                    .map(|e| self.infer_expr(e, interner))
                    .unwrap_or(LogosType::Unknown);
                LogosType::Seq(Box::new(elem_type))
            }

            Expr::New { type_name, type_args, .. } => {
                let name = interner.resolve(*type_name);
                match name {
                    "Seq" | "List" | "Vec" => {
                        let elem = type_args
                            .first()
                            .map(|t| LogosType::from_type_expr(t, interner))
                            .unwrap_or(LogosType::Unit);
                        LogosType::Seq(Box::new(elem))
                    }
                    "Map" | "HashMap" => {
                        let key = type_args
                            .first()
                            .map(|t| LogosType::from_type_expr(t, interner))
                            .unwrap_or(LogosType::String);
                        let val = type_args
                            .get(1)
                            .map(|t| LogosType::from_type_expr(t, interner))
                            .unwrap_or(LogosType::String);
                        LogosType::Map(Box::new(key), Box::new(val))
                    }
                    "Set" | "HashSet" => {
                        let elem = type_args
                            .first()
                            .map(|t| LogosType::from_type_expr(t, interner))
                            .unwrap_or(LogosType::Unit);
                        LogosType::Set(Box::new(elem))
                    }
                    _ => LogosType::Unknown,
                }
            }

            Expr::FieldAccess { .. } => LogosType::Unknown,

            Expr::OptionSome { value } => {
                let inner = self.infer_expr(value, interner);
                LogosType::Option(Box::new(inner))
            }

            Expr::OptionNone => LogosType::Option(Box::new(LogosType::Unknown)),

            Expr::Range { .. } => LogosType::Seq(Box::new(LogosType::Int)),

            Expr::Contains { .. } => LogosType::Bool,

            Expr::Copy { expr: inner } | Expr::Give { value: inner } => {
                self.infer_expr(inner, interner)
            }

            Expr::WithCapacity { value, .. } => self.infer_expr(value, interner),

            _ => LogosType::Unknown,
        }
    }

    /// Build a TypeEnv from a program's statements via a single forward pass.
    ///
    /// For each statement, registers variable types and function signatures
    /// into the environment. Since LOGOS programs are forward-declared,
    /// no fixpoint iteration is needed.
    pub fn infer_program(stmts: &[Stmt], interner: &Interner, _registry: &TypeRegistry) -> Self {
        let mut env = Self::new();
        env.infer_stmts(stmts, interner);
        env
    }

    /// Walk a slice of statements, registering types.
    fn infer_stmts(&mut self, stmts: &[Stmt], interner: &Interner) {
        for stmt in stmts {
            self.infer_stmt(stmt, interner);
        }
    }

    /// Infer types from a single statement.
    fn infer_stmt(&mut self, stmt: &Stmt, interner: &Interner) {
        match stmt {
            Stmt::Let { var, ty, value, .. } => {
                let inferred = if let Some(type_expr) = ty {
                    // Explicit type annotation takes priority
                    let base_ty = LogosType::from_type_expr(type_expr, interner);
                    if base_ty != LogosType::Unknown {
                        base_ty
                    } else {
                        self.infer_expr(value, interner)
                    }
                } else {
                    self.infer_expr(value, interner)
                };
                self.register(*var, inferred);
            }

            Stmt::Set { target, value } => {
                // If we don't already have a type for target, infer from value
                if self.lookup(*target) == &LogosType::Unknown {
                    let inferred = self.infer_expr(value, interner);
                    if inferred != LogosType::Unknown {
                        self.register(*target, inferred);
                    }
                }
            }

            Stmt::FunctionDef {
                name,
                params,
                body,
                return_type,
                ..
            } => {
                // Register parameter types
                let param_types: Vec<(Symbol, LogosType)> = params
                    .iter()
                    .map(|(sym, ty)| (*sym, LogosType::from_type_expr(ty, interner)))
                    .collect();

                // Register params in the env for body inference
                for (sym, ty) in &param_types {
                    self.register(*sym, ty.clone());
                }

                // Infer body
                self.infer_stmts(body, interner);

                // Determine return type
                let ret_ty = if let Some(rt) = return_type {
                    LogosType::from_type_expr(rt, interner)
                } else {
                    self.infer_return_type(body, interner)
                };

                self.register_fn(
                    *name,
                    FnSig {
                        params: param_types,
                        return_type: ret_ty,
                    },
                );
            }

            Stmt::Repeat { pattern, iterable, body } => {
                // Infer the element type from the iterable
                let iterable_ty = self.infer_expr(iterable, interner);
                let elem_ty = match &iterable_ty {
                    LogosType::Seq(inner) => (**inner).clone(),
                    LogosType::Set(inner) => (**inner).clone(),
                    LogosType::Map(k, _v) => (**k).clone(),
                    _ => LogosType::Unknown,
                };
                match pattern {
                    crate::ast::stmt::Pattern::Identifier(sym) => {
                        self.register(*sym, elem_ty);
                    }
                    crate::ast::stmt::Pattern::Tuple(syms) => {
                        // For tuple destructuring, we don't have enough info
                        for sym in syms {
                            self.register(*sym, LogosType::Unknown);
                        }
                    }
                }
                self.infer_stmts(body, interner);
            }

            Stmt::If { then_block, else_block, .. } => {
                self.infer_stmts(then_block, interner);
                if let Some(else_b) = else_block {
                    self.infer_stmts(else_b, interner);
                }
            }

            Stmt::While { body, .. } => {
                self.infer_stmts(body, interner);
            }

            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    // Register bindings from pattern match as Unknown
                    for (_field, binding) in &arm.bindings {
                        self.register(*binding, LogosType::Unknown);
                    }
                    self.infer_stmts(arm.body, interner);
                }
            }

            Stmt::Zone { body, .. } => {
                self.infer_stmts(body, interner);
            }

            Stmt::ReadFrom { var, .. } => {
                // ReadFrom always gives String
                self.register(*var, LogosType::String);
            }

            Stmt::CreatePipe { var, element_type, .. } => {
                let elem = LogosType::from_type_name(interner.resolve(*element_type));
                self.register(*var, elem);
            }

            Stmt::ReceivePipe { var, .. } | Stmt::TryReceivePipe { var, .. } => {
                // Pipe receive type depends on pipe type, default to Unknown
                self.register(*var, LogosType::Unknown);
            }

            Stmt::Pop { into: Some(var), collection } => {
                let coll_ty = self.infer_expr(collection, interner);
                let elem_ty = coll_ty
                    .element_type()
                    .cloned()
                    .unwrap_or(LogosType::Unknown);
                self.register(*var, elem_ty);
            }

            Stmt::AwaitMessage { into, .. } => {
                self.register(*into, LogosType::Unknown);
            }

            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                self.infer_stmts(tasks, interner);
            }

            _ => {}
        }
    }

    /// Infer the return type from a function body by scanning Return statements.
    fn infer_return_type(&self, body: &[Stmt], interner: &Interner) -> LogosType {
        for stmt in body {
            if let Stmt::Return { value: Some(expr) } = stmt {
                return self.infer_expr(expr, interner);
            }
        }
        LogosType::Unit
    }

    /// Export the type environment as a LogosType map for RefinementContext seeding.
    pub fn to_logos_type_map(&self) -> HashMap<Symbol, LogosType> {
        self.variables
            .iter()
            .filter(|(_, ty)| **ty != LogosType::Unknown)
            .map(|(sym, ty)| (*sym, ty.clone()))
            .collect()
    }

    /// Convert the type environment to the legacy string-based format.
    pub fn to_legacy_variable_types(&self) -> HashMap<Symbol, std::string::String> {
        self.variables
            .iter()
            .filter(|(_, ty)| **ty != LogosType::Unknown)
            .map(|(sym, ty)| (*sym, ty.to_rust_type()))
            .collect()
    }

    /// Convert the type environment to the legacy string var set.
    pub fn to_legacy_string_vars(&self) -> std::collections::HashSet<Symbol> {
        self.variables
            .iter()
            .filter(|(_, ty)| ty.is_string())
            .map(|(sym, _)| *sym)
            .collect()
    }
}

/// Centralized name resolution for Rust identifier output.
///
/// Wraps the interner and ensures all identifier output goes through
/// keyword escaping. Makes it impossible to forget escaping.
pub struct RustNames<'a> {
    interner: &'a Interner,
}

impl<'a> RustNames<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Self { interner }
    }

    /// Resolve a symbol to a Rust-safe identifier (always escapes keywords).
    pub fn ident(&self, sym: Symbol) -> std::string::String {
        crate::codegen::escape_rust_ident(self.interner.resolve(sym))
    }

    /// Get the raw name for pattern matching (native functions, type names, etc.).
    pub fn raw(&self, sym: Symbol) -> &str {
        self.interner.resolve(sym)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // LogosType::is_copy tests
    // =========================================================================

    #[test]
    fn copy_types_are_copy() {
        assert!(LogosType::Int.is_copy());
        assert!(LogosType::Float.is_copy());
        assert!(LogosType::Bool.is_copy());
        assert!(LogosType::Char.is_copy());
        assert!(LogosType::Byte.is_copy());
        assert!(LogosType::Unit.is_copy());
        assert!(LogosType::Nat.is_copy());
    }

    #[test]
    fn non_copy_types_are_not_copy() {
        assert!(!LogosType::String.is_copy());
        assert!(!LogosType::Seq(Box::new(LogosType::Int)).is_copy());
        assert!(!LogosType::Map(Box::new(LogosType::String), Box::new(LogosType::Int)).is_copy());
        assert!(!LogosType::Set(Box::new(LogosType::Int)).is_copy());
        assert!(!LogosType::Option(Box::new(LogosType::Int)).is_copy());
        assert!(!LogosType::Duration.is_copy());
        assert!(!LogosType::Unknown.is_copy());
    }

    // =========================================================================
    // LogosType::is_numeric / is_string / is_float
    // =========================================================================

    #[test]
    fn numeric_types() {
        assert!(LogosType::Int.is_numeric());
        assert!(LogosType::Float.is_numeric());
        assert!(LogosType::Nat.is_numeric());
        assert!(!LogosType::String.is_numeric());
        assert!(!LogosType::Bool.is_numeric());
    }

    #[test]
    fn string_type() {
        assert!(LogosType::String.is_string());
        assert!(!LogosType::Int.is_string());
        assert!(!LogosType::Unknown.is_string());
    }

    #[test]
    fn float_type() {
        assert!(LogosType::Float.is_float());
        assert!(!LogosType::Int.is_float());
    }

    // =========================================================================
    // LogosType::element_type / value_type / key_type
    // =========================================================================

    #[test]
    fn seq_element_type() {
        let seq = LogosType::Seq(Box::new(LogosType::Int));
        assert_eq!(seq.element_type(), Some(&LogosType::Int));
    }

    #[test]
    fn set_element_type() {
        let set = LogosType::Set(Box::new(LogosType::String));
        assert_eq!(set.element_type(), Some(&LogosType::String));
    }

    #[test]
    fn map_key_value_types() {
        let map = LogosType::Map(Box::new(LogosType::String), Box::new(LogosType::Int));
        assert_eq!(map.key_type(), Some(&LogosType::String));
        assert_eq!(map.value_type(), Some(&LogosType::Int));
    }

    #[test]
    fn non_collection_element_type() {
        assert_eq!(LogosType::Int.element_type(), None);
        assert_eq!(LogosType::String.element_type(), None);
    }

    // =========================================================================
    // LogosType::numeric_promotion
    // =========================================================================

    #[test]
    fn numeric_promotion_int_int() {
        assert_eq!(
            LogosType::numeric_promotion(&LogosType::Int, &LogosType::Int),
            LogosType::Int
        );
    }

    #[test]
    fn numeric_promotion_int_float() {
        assert_eq!(
            LogosType::numeric_promotion(&LogosType::Int, &LogosType::Float),
            LogosType::Float
        );
    }

    #[test]
    fn numeric_promotion_float_int() {
        assert_eq!(
            LogosType::numeric_promotion(&LogosType::Float, &LogosType::Int),
            LogosType::Float
        );
    }

    #[test]
    fn numeric_promotion_float_float() {
        assert_eq!(
            LogosType::numeric_promotion(&LogosType::Float, &LogosType::Float),
            LogosType::Float
        );
    }

    #[test]
    fn numeric_promotion_non_numeric() {
        assert_eq!(
            LogosType::numeric_promotion(&LogosType::String, &LogosType::Int),
            LogosType::Unknown
        );
    }

    // =========================================================================
    // LogosType::to_rust_type
    // =========================================================================

    #[test]
    fn to_rust_type_primitives() {
        assert_eq!(LogosType::Int.to_rust_type(), "i64");
        assert_eq!(LogosType::Float.to_rust_type(), "f64");
        assert_eq!(LogosType::Bool.to_rust_type(), "bool");
        assert_eq!(LogosType::Char.to_rust_type(), "char");
        assert_eq!(LogosType::Byte.to_rust_type(), "u8");
        assert_eq!(LogosType::String.to_rust_type(), "String");
        assert_eq!(LogosType::Unit.to_rust_type(), "()");
        assert_eq!(LogosType::Nat.to_rust_type(), "u64");
    }

    #[test]
    fn to_rust_type_temporal() {
        assert_eq!(LogosType::Duration.to_rust_type(), "std::time::Duration");
        assert_eq!(LogosType::Date.to_rust_type(), "LogosDate");
        assert_eq!(LogosType::Moment.to_rust_type(), "LogosMoment");
        assert_eq!(LogosType::Time.to_rust_type(), "LogosTime");
        assert_eq!(LogosType::Span.to_rust_type(), "LogosSpan");
    }

    #[test]
    fn to_rust_type_collections() {
        assert_eq!(
            LogosType::Seq(Box::new(LogosType::Int)).to_rust_type(),
            "Vec<i64>"
        );
        assert_eq!(
            LogosType::Map(Box::new(LogosType::String), Box::new(LogosType::Int)).to_rust_type(),
            "std::collections::HashMap<String, i64>"
        );
        assert_eq!(
            LogosType::Set(Box::new(LogosType::Int)).to_rust_type(),
            "std::collections::HashSet<i64>"
        );
        assert_eq!(
            LogosType::Option(Box::new(LogosType::String)).to_rust_type(),
            "Option<String>"
        );
    }

    #[test]
    fn to_rust_type_nested() {
        let nested = LogosType::Seq(Box::new(LogosType::Seq(Box::new(LogosType::Float))));
        assert_eq!(nested.to_rust_type(), "Vec<Vec<f64>>");
    }

    // =========================================================================
    // LogosType::from_literal
    // =========================================================================

    #[test]
    fn from_literal_number() {
        assert_eq!(LogosType::from_literal(&Literal::Number(42)), LogosType::Int);
    }

    #[test]
    fn from_literal_float() {
        assert_eq!(
            LogosType::from_literal(&Literal::Float(3.14)),
            LogosType::Float
        );
    }

    #[test]
    fn from_literal_text() {
        assert_eq!(
            LogosType::from_literal(&Literal::Text(Symbol::EMPTY)),
            LogosType::String
        );
    }

    #[test]
    fn from_literal_bool() {
        assert_eq!(
            LogosType::from_literal(&Literal::Boolean(true)),
            LogosType::Bool
        );
    }

    #[test]
    fn from_literal_nothing() {
        assert_eq!(LogosType::from_literal(&Literal::Nothing), LogosType::Unit);
    }

    // =========================================================================
    // LogosType::from_type_expr
    // =========================================================================

    #[test]
    fn from_type_expr_primitive_int() {
        let mut interner = Interner::new();
        let sym = interner.intern("Int");
        let ty = TypeExpr::Primitive(sym);
        assert_eq!(LogosType::from_type_expr(&ty, &interner), LogosType::Int);
    }

    #[test]
    fn from_type_expr_named_text() {
        let mut interner = Interner::new();
        let sym = interner.intern("Text");
        let ty = TypeExpr::Named(sym);
        assert_eq!(LogosType::from_type_expr(&ty, &interner), LogosType::String);
    }

    // =========================================================================
    // TypeEnv basics
    // =========================================================================

    #[test]
    fn env_lookup_unknown_returns_unknown() {
        let env = TypeEnv::new();
        assert_eq!(env.lookup(Symbol::EMPTY), &LogosType::Unknown);
    }

    #[test]
    fn env_register_and_lookup() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let x = interner.intern("x");
        env.register(x, LogosType::Int);
        assert_eq!(env.lookup(x), &LogosType::Int);
    }

    #[test]
    fn env_register_fn_and_lookup() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let f = interner.intern("add");
        let a = interner.intern("a");
        let b = interner.intern("b");
        env.register_fn(
            f,
            FnSig {
                params: vec![(a, LogosType::Int), (b, LogosType::Int)],
                return_type: LogosType::Int,
            },
        );
        let sig = env.lookup_fn(f).unwrap();
        assert_eq!(sig.return_type, LogosType::Int);
        assert_eq!(sig.params.len(), 2);
    }

    // =========================================================================
    // TypeEnv::infer_expr
    // =========================================================================

    #[test]
    fn infer_expr_literal_number() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let expr = Expr::Literal(Literal::Number(42));
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
    }

    #[test]
    fn infer_expr_literal_float() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let expr = Expr::Literal(Literal::Float(3.14));
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
    }

    #[test]
    fn infer_expr_literal_text() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let expr = Expr::Literal(Literal::Text(Symbol::EMPTY));
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::String);
    }

    #[test]
    fn infer_expr_identifier_registered() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let x = interner.intern("x");
        env.register(x, LogosType::Float);
        let expr = Expr::Identifier(x);
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
    }

    #[test]
    fn infer_expr_identifier_unknown() {
        let env = TypeEnv::new();
        let mut interner = Interner::new();
        let x = interner.intern("x");
        let expr = Expr::Identifier(x);
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Unknown);
    }

    #[test]
    fn infer_expr_add_int_int() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let left = Expr::Literal(Literal::Number(1));
        let right = Expr::Literal(Literal::Number(2));
        let expr = Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: &left,
            right: &right,
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
    }

    #[test]
    fn infer_expr_add_int_float_promotes() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let left = Expr::Literal(Literal::Number(1));
        let right = Expr::Literal(Literal::Float(2.0));
        let expr = Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: &left,
            right: &right,
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
    }

    #[test]
    fn infer_expr_add_string_int_is_string() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let s = interner.intern("s");
        env.register(s, LogosType::String);
        let left = Expr::Identifier(s);
        let right = Expr::Literal(Literal::Number(42));
        let expr = Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: &left,
            right: &right,
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::String);
    }

    #[test]
    fn infer_expr_comparison_is_bool() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let left = Expr::Literal(Literal::Number(1));
        let right = Expr::Literal(Literal::Number(2));
        let expr = Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left: &left,
            right: &right,
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Bool);
    }

    #[test]
    fn infer_expr_length_is_int() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let inner = Expr::Literal(Literal::Text(Symbol::EMPTY));
        let expr = Expr::Length { collection: &inner };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
    }

    #[test]
    fn infer_expr_call_sqrt_is_float() {
        let env = TypeEnv::new();
        let mut interner = Interner::new();
        let sqrt = interner.intern("sqrt");
        let arg = Expr::Literal(Literal::Number(4));
        let expr = Expr::Call {
            function: sqrt,
            args: vec![&arg],
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
    }

    #[test]
    fn infer_expr_call_parseint_is_int() {
        let env = TypeEnv::new();
        let mut interner = Interner::new();
        let pi = interner.intern("parseInt");
        let arg = Expr::Literal(Literal::Text(Symbol::EMPTY));
        let expr = Expr::Call {
            function: pi,
            args: vec![&arg],
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
    }

    #[test]
    fn infer_expr_call_user_function() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let f = interner.intern("compute");
        env.register_fn(
            f,
            FnSig {
                params: vec![],
                return_type: LogosType::Float,
            },
        );
        let expr = Expr::Call {
            function: f,
            args: vec![],
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
    }

    #[test]
    fn infer_expr_index_vec() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let items = interner.intern("items");
        env.register(items, LogosType::Seq(Box::new(LogosType::Int)));
        let coll = Expr::Identifier(items);
        let idx = Expr::Literal(Literal::Number(1));
        let expr = Expr::Index {
            collection: &coll,
            index: &idx,
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
    }

    #[test]
    fn infer_expr_index_map() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let data = interner.intern("data");
        env.register(
            data,
            LogosType::Map(Box::new(LogosType::String), Box::new(LogosType::Float)),
        );
        let coll = Expr::Identifier(data);
        let key = Expr::Literal(Literal::Text(Symbol::EMPTY));
        let expr = Expr::Index {
            collection: &coll,
            index: &key,
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
    }

    #[test]
    fn infer_expr_list_literal() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let a = Expr::Literal(Literal::Number(1));
        let b = Expr::Literal(Literal::Number(2));
        let expr = Expr::List(vec![&a, &b]);
        assert_eq!(
            env.infer_expr(&expr, &interner),
            LogosType::Seq(Box::new(LogosType::Int))
        );
    }

    #[test]
    fn infer_expr_option_some() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let inner = Expr::Literal(Literal::Number(42));
        let expr = Expr::OptionSome { value: &inner };
        assert_eq!(
            env.infer_expr(&expr, &interner),
            LogosType::Option(Box::new(LogosType::Int))
        );
    }

    #[test]
    fn infer_expr_contains_is_bool() {
        let env = TypeEnv::new();
        let interner = Interner::new();
        let coll = Expr::Literal(Literal::Text(Symbol::EMPTY));
        let val = Expr::Literal(Literal::Number(1));
        let expr = Expr::Contains {
            collection: &coll,
            value: &val,
        };
        assert_eq!(env.infer_expr(&expr, &interner), LogosType::Bool);
    }

    // =========================================================================
    // TypeEnv::to_legacy_variable_types / to_legacy_string_vars
    // =========================================================================

    #[test]
    fn legacy_variable_types_conversion() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let x = interner.intern("x");
        let y = interner.intern("y");
        let z = interner.intern("z");
        env.register(x, LogosType::Int);
        env.register(y, LogosType::Seq(Box::new(LogosType::Float)));
        env.register(z, LogosType::Unknown); // Should be filtered out
        let legacy = env.to_legacy_variable_types();
        assert_eq!(legacy.get(&x).unwrap(), "i64");
        assert_eq!(legacy.get(&y).unwrap(), "Vec<f64>");
        assert!(!legacy.contains_key(&z));
    }

    #[test]
    fn legacy_string_vars_conversion() {
        let mut env = TypeEnv::new();
        let mut interner = Interner::new();
        let s = interner.intern("s");
        let n = interner.intern("n");
        env.register(s, LogosType::String);
        env.register(n, LogosType::Int);
        let string_vars = env.to_legacy_string_vars();
        assert!(string_vars.contains(&s));
        assert!(!string_vars.contains(&n));
    }

    // =========================================================================
    // TypeEnv::infer_program — integration tests via compile pipeline
    // =========================================================================

    #[test]
    fn infer_program_let_literal() {
        let mut interner = Interner::new();
        let x = interner.intern("x");
        let val = Expr::Literal(Literal::Number(42));
        let stmts = [Stmt::Let {
            var: x,
            ty: None,
            value: &val,
            mutable: false,
        }];
        let registry = TypeRegistry::new();
        let env = TypeEnv::infer_program(&stmts, &interner, &registry);
        assert_eq!(env.lookup(x), &LogosType::Int);
    }

    #[test]
    fn infer_program_let_with_type_annotation() {
        let mut interner = Interner::new();
        let x = interner.intern("x");
        let float_sym = interner.intern("Real");
        let val = Expr::Literal(Literal::Number(5));
        let ty = TypeExpr::Primitive(float_sym);
        let stmts = [Stmt::Let {
            var: x,
            ty: Some(&ty),
            value: &val,
            mutable: false,
        }];
        let registry = TypeRegistry::new();
        let env = TypeEnv::infer_program(&stmts, &interner, &registry);
        assert_eq!(env.lookup(x), &LogosType::Float);
    }

    #[test]
    fn infer_program_let_string_tracked() {
        let mut interner = Interner::new();
        let s = interner.intern("name");
        let hello = interner.intern("hello");
        let val = Expr::Literal(Literal::Text(hello));
        let stmts = [Stmt::Let {
            var: s,
            ty: None,
            value: &val,
            mutable: false,
        }];
        let registry = TypeRegistry::new();
        let env = TypeEnv::infer_program(&stmts, &interner, &registry);
        assert!(env.lookup(s).is_string());
        let string_vars = env.to_legacy_string_vars();
        assert!(string_vars.contains(&s));
    }

    #[test]
    fn infer_program_readfrom_is_string() {
        let mut interner = Interner::new();
        let input = interner.intern("input");
        let stmts = [Stmt::ReadFrom {
            var: input,
            source: crate::ast::stmt::ReadSource::Console,
        }];
        let registry = TypeRegistry::new();
        let env = TypeEnv::infer_program(&stmts, &interner, &registry);
        assert_eq!(env.lookup(input), &LogosType::String);
    }

    // =========================================================================
    // RustNames tests
    // =========================================================================

    #[test]
    fn rust_names_escapes_keywords() {
        let mut interner = Interner::new();
        let r#move = interner.intern("move");
        let names = RustNames::new(&interner);
        assert_eq!(names.ident(r#move), "r#move");
    }

    #[test]
    fn rust_names_preserves_non_keywords() {
        let mut interner = Interner::new();
        let foo = interner.intern("foo");
        let names = RustNames::new(&interner);
        assert_eq!(names.ident(foo), "foo");
    }

    #[test]
    fn rust_names_raw_returns_original() {
        let mut interner = Interner::new();
        let r#move = interner.intern("move");
        let names = RustNames::new(&interner);
        assert_eq!(names.raw(r#move), "move");
    }
}
