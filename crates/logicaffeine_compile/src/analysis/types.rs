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

use crate::analysis::TypeRegistry;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

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
    /// A DOM element handle — nominal (never an `Int`), an i64 handle at runtime on every tier.
    Element,
    /// A DOM event handle — nominal, distinct from `Element`; an i64 handle at runtime.
    Event,
    /// A view value (a template with its captured bindings) — nominal.
    View,
    /// Exact rational (`logicaffeine_base::Rational`). A `Rational`-typed binding keeps
    /// division exact (`7 / 2 → 7/2`) where a bare `Int` floors. Lowers to `LogosRational`.
    Rational,
    /// Exact base-10 fixed-point (money). `+ − ×` stay exact `Decimal` (scale preserved);
    /// `÷` widens to `Rational`. Built via `decimal("19.99")`. Lowers to `LogosDecimal`.
    Decimal,
    /// Exact complex number `re + im·i`. `+ − × ÷` stay `Complex` (closed field). Built via
    /// `complex(re, im)`. NOT ordered. Lowers to `LogosComplex`.
    Complex,
    /// An element of the ring ℤ/nℤ. `+ − × ÷` stay `Modular` (same modulus required, no
    /// auto-lift). Built via `modular(value, modulus)`. NOT ordered. Lowers to `LogosModular`.
    Modular,
    /// A dimensioned physical quantity (`quantity(2, "inch")`). `+ −` require the same dimension,
    /// `× ÷` combine dimensions, and it may be scaled by a number; built via `quantity`/`convert`.
    /// The magnitude rides the exact rational tower. Lowers to `LogosQuantity`. Dimension safety is
    /// enforced at runtime (a clean error), like `Modular`'s ring check.
    Quantity,
    /// An exact monetary amount in a currency (`money(19.99, "USD")` / `19.99 USD`). `+ −` require
    /// the same currency, `× ÷` scale by a number; the amount rides the Decimal tower. Lowers to
    /// `LogosMoney`. Currency safety is enforced at runtime (a clean error), like `Quantity`.
    Money,
    /// A 128-bit UUID (`uuid "…"`, `a random uuid`). An opaque identifier — no arithmetic; lowers to
    /// `LogosUuid`. Comparable/orderable by its bytes.
    Uuid,
    UserDefined(Symbol),
    /// First-class function type: fn(P1, P2, ...) -> R
    Function(Vec<LogosType>, Box<LogosType>),
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
    native_functions: std::collections::HashSet<Symbol>,
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
                | LogosType::Element
                | LogosType::Event
                | LogosType::View
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
        if matches!(a, LogosType::Quantity) || matches!(b, LogosType::Quantity) {
            // A quantity carries through `+ − × ÷` (same-dimension for `+ −`, dimension-combining
            // for `× ÷`, or scaling by a number) — the dimensional rule is enforced at runtime.
            LogosType::Quantity
        } else if matches!(a, LogosType::Money) || matches!(b, LogosType::Money) {
            // Money carries through `+ −` (same currency) and scaling by a number. (A same-currency
            // `Money ÷ Money` is a Rational ratio — that operator-specific narrowing is handled in
            // the binary-op inference, not this operator-agnostic promotion.)
            LogosType::Money
        } else if a.is_float() || b.is_float() {
            LogosType::Float
        } else if matches!(a, LogosType::Rational) || matches!(b, LogosType::Rational) {
            // A Rational operand carries through `+ − ×`: the exact type wins over Int.
            LogosType::Rational
        } else if matches!(a, LogosType::Modular) && matches!(b, LogosType::Modular) {
            // Modular combines only with Modular (same ring; no auto-lift of a bare integer).
            LogosType::Modular
        } else if matches!(a, LogosType::Complex) || matches!(b, LogosType::Complex) {
            // Complex is the top of the exact tower: a real embeds as `re + 0i`, so any
            // operation with a Complex stays Complex (the field is closed under + − × ÷).
            LogosType::Complex
        } else if matches!(a, LogosType::Decimal) || matches!(b, LogosType::Decimal) {
            // A Decimal carries through `+ − ×` (Decimal ∘ Int stays Decimal, scale kept).
            LogosType::Decimal
        } else if a.is_numeric() && b.is_numeric() {
            LogosType::Int
        } else {
            LogosType::Unknown
        }
    }

    /// The type that holds a value of `a` and one of `b` where the language joins numbers exactly: an integer
    /// and a rational are both the rational they equal, item by item in lists. `None` for any other pair.
    pub fn number_join(a: &LogosType, b: &LogosType) -> Option<LogosType> {
        match (a, b) {
            _ if a == b => Some(a.clone()),
            (LogosType::Int, LogosType::Rational) | (LogosType::Rational, LogosType::Int) => Some(LogosType::Rational),
            (LogosType::Seq(x), LogosType::Seq(y)) => LogosType::number_join(x, y).map(|t| LogosType::Seq(Box::new(t))),
            _ => None,
        }
    }

    /// Whether a value of this type holds a rational: one, or a list of them at any depth.
    pub fn holds_rational(&self) -> bool {
        match self {
            LogosType::Rational => true,
            LogosType::Seq(inner) => inner.holds_rational(),
            _ => false,
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
            LogosType::Rational => "LogosRational".into(),
            LogosType::Decimal => "LogosDecimal".into(),
            LogosType::Complex => "LogosComplex".into(),
            LogosType::Modular => "LogosModular".into(),
            LogosType::Quantity => "LogosQuantity".into(),
            LogosType::Money => "LogosMoney".into(),
            LogosType::Uuid => "LogosUuid".into(),
            LogosType::Duration => "std::time::Duration".into(),
            LogosType::Date => "LogosDate".into(),
            LogosType::Moment => "LogosMoment".into(),
            LogosType::Time => "LogosTime".into(),
            LogosType::Span => "LogosSpan".into(),
            // Handles are the i64 every tier already moves; nominality lives in the checker.
            LogosType::Element | LogosType::Event | LogosType::View => "i64".into(),
            LogosType::Seq(inner) => format!("LogosSeq<{}>", inner.to_rust_type()),
            LogosType::Map(k, v) => format!("LogosMap<{}, {}>", k.to_rust_type(), v.to_rust_type()),
            LogosType::Set(inner) => {
                format!("Set<{}>", inner.to_rust_type())
            }
            LogosType::Option(inner) => format!("Option<{}>", inner.to_rust_type()),
            LogosType::Function(params, ret) => {
                let params_str = params
                    .iter()
                    .map(|p| p.to_rust_type())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("impl Fn({}) -> {}", params_str, ret.to_rust_type())
            }
            LogosType::UserDefined(_) => "_".into(),
            LogosType::Unknown => "_".into(),
        }
    }

    /// Build a LogosType from a TypeExpr AST node.
    pub fn from_type_expr(ty: &TypeExpr, interner: &Interner) -> LogosType {
        match ty {
            // A `mutable` parameter erases to its underlying type; the marker only
            // affects the parameter-passing convention (Mutable Value Semantics).
            TypeExpr::Mutable { inner } => Self::from_type_expr(inner, interner),
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
            // A tuple/record type — the analysis is structural-agnostic here (as `Pair`/`Triple` already
            // are, falling to `Unknown` above); the VM boundary + codegen carry the real per-position types.
            TypeExpr::Tuple(_) => LogosType::Unknown,
            TypeExpr::Record { .. } => LogosType::Unknown,
            TypeExpr::Refinement { base, .. } => LogosType::from_type_expr(base, interner),
            TypeExpr::Persistent { inner } => LogosType::from_type_expr(inner, interner),
            TypeExpr::Function { inputs, output } => {
                let params = inputs
                    .iter()
                    .map(|i| LogosType::from_type_expr(i, interner))
                    .collect();
                let ret = Box::new(LogosType::from_type_expr(output, interner));
                LogosType::Function(params, ret)
            }
        }
    }

    /// Parse a LOGOS type name string into a LogosType.
    pub(crate) fn from_type_name(name: &str) -> LogosType {
        match name {
            "Int" => LogosType::Int,
            "Nat" => LogosType::Nat,
            "Rational" => LogosType::Rational,
            "Decimal" => LogosType::Decimal,
            "Complex" => LogosType::Complex,
            "Modular" => LogosType::Modular,
            "Quantity" => LogosType::Quantity,
            "Money" => LogosType::Money,
            "Uuid" | "UUID" => LogosType::Uuid,
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
            "Element" => LogosType::Element,
            "Event" => LogosType::Event,
            "View" => LogosType::View,
            _ => LogosType::Unknown,
        }
    }

    /// Parse a Rust type string back into a LogosType (legacy bridge).
    /// Handles strings like "i64", "f64", "String", "Vec<i64>",
    /// "std::collections::HashMap<String, i64>", etc.
    pub fn from_rust_type_str(s: &str) -> LogosType {
        // Strip loop-bounds hoisting sentinel (e.g. "Vec<i64>|__hl:left_len" → "Vec<i64>").
        let s = s.split("|__hl:").next().unwrap_or(s);
        // Strip the promotable-Int sentinel ("i64|__bigint"): the value still classifies as
        // Int (so the exact-arithmetic path fires); the tag only steers codegen to store the
        // overflow-promoting `LogosInt` instead of a bare `i64`.
        let s = s.split("|__bigint").next().unwrap_or(s);
        match s {
            "i64" => LogosType::Int,
            "u64" => LogosType::Nat,
            "f64" => LogosType::Float,
            "bool" => LogosType::Bool,
            "char" => LogosType::Char,
            "u8" => LogosType::Byte,
            "LogosRational" => LogosType::Rational,
            "LogosDecimal" => LogosType::Decimal,
            "LogosComplex" => LogosType::Complex,
            "LogosModular" => LogosType::Modular,
            "LogosQuantity" => LogosType::Quantity,
            "LogosMoney" => LogosType::Money,
            "LogosUuid" => LogosType::Uuid,
            "String" => LogosType::String,
            "()" => LogosType::Unit,
            "std::time::Duration" => LogosType::Duration,
            "LogosDate" => LogosType::Date,
            "LogosMoment" => LogosType::Moment,
            "LogosTime" => LogosType::Time,
            "LogosSpan" => LogosType::Span,
            _ => {
                // A place that can hold `nothing` holds a value of its payload's type, or nothing.
                if let Some(inner) = s.strip_prefix("LogosOpt<").and_then(|s| s.strip_suffix('>')) {
                    return Self::from_rust_type_str(inner);
                }
                // Try to parse generic types
                if let Some(inner) = s
                    .strip_prefix("LogosSeq<")
                    .or_else(|| s.strip_prefix("Vec<"))
                    .and_then(|s| s.strip_suffix('>'))
                {
                    LogosType::Seq(Box::new(Self::from_rust_type_str(inner)))
                } else if let Some(inner) = s
                    .strip_prefix("LogosMap<")
                    .or_else(|| s.strip_prefix("std::collections::HashMap<"))
                    .or_else(|| s.strip_prefix("HashMap<"))
                    .or_else(|| s.strip_prefix("rustc_hash::FxHashMap<"))
                    .or_else(|| s.strip_prefix("FxHashMap<"))
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
                    .or_else(|| s.strip_prefix("rustc_hash::FxHashSet<"))
                    .or_else(|| s.strip_prefix("FxHashSet<"))
                    .or_else(|| s.strip_prefix("Set<"))
                    .and_then(|s| s.strip_suffix('>'))
                {
                    LogosType::Set(Box::new(Self::from_rust_type_str(inner)))
                } else if let Some(inner) =
                    s.strip_prefix("Option<").and_then(|s| s.strip_suffix('>'))
                {
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
            native_functions: std::collections::HashSet::new(),
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
        self.native_functions.remove(&sym);
    }

    pub(super) fn set_native_fn(&mut self, sym: Symbol, is_native: bool) {
        if is_native {
            self.native_functions.insert(sym);
        } else {
            self.native_functions.remove(&sym);
        }
    }

    /// Infer the type of an expression given the current environment.
    pub fn infer_expr(&self, expr: &Expr, interner: &Interner) -> LogosType {
        self.infer_expr_with(expr, interner, &|_| None)
    }

    /// [`TypeEnv::infer_expr`], with `field` typing each read of a record's field the environment does not know.
    pub fn infer_expr_with(
        &self,
        expr: &Expr,
        interner: &Interner,
        field: &dyn Fn(&Expr) -> Option<LogosType>,
    ) -> LogosType {
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
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::ApproxEq => LogosType::Bool,

                    // And/Or: logical — truthiness in, Bool out (`&`/`|` are the bitwise spellings).
                    BinaryOpKind::And | BinaryOpKind::Or => LogosType::Bool,

                    // Concat always produces String
                    BinaryOpKind::Concat => LogosType::String,

                    // `a followed by b` — a sequence of the same type as the operands.
                    BinaryOpKind::SeqConcat => self.infer_expr_with(left, interner, field),

                    // Bitwise ops always produce Int
                    // `& | ^` on Sets yield a Set of the left operand's type;
                    // on ints they stay Int.
                    BinaryOpKind::BitAnd | BinaryOpKind::BitOr => {
                        let lt = self.infer_expr_with(left, interner, field);
                        if matches!(lt, LogosType::Set(_)) {
                            lt
                        } else {
                            LogosType::Int
                        }
                    }
                    BinaryOpKind::BitXor | BinaryOpKind::Shl | BinaryOpKind::Shr => LogosType::Int,

                    // Add: could be numeric or string concatenation
                    BinaryOpKind::Add => {
                        let lt = self.infer_expr_with(left, interner, field);
                        let rt = self.infer_expr_with(right, interner, field);
                        if lt.is_string() || rt.is_string() {
                            LogosType::String
                        } else {
                            LogosType::numeric_promotion(&lt, &rt)
                        }
                    }

                    // Exact division always yields a Rational (`7 / 2 → 7/2`); the
                    // resolve pass only emits it in a Rational-typed context.
                    BinaryOpKind::ExactDivide => LogosType::Rational,

                    // Other arithmetic: numeric promotion
                    BinaryOpKind::Subtract
                    | BinaryOpKind::Pow
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
                    | BinaryOpKind::FloorDivide
                    | BinaryOpKind::Modulo => {
                        let lt = self.infer_expr_with(left, interner, field);
                        let rt = self.infer_expr_with(right, interner, field);
                        let promoted = LogosType::numeric_promotion(&lt, &rt);
                        // Decimal division widens to an exact Rational (base-10 division need
                        // not terminate); `+ − ×` keep it Decimal. Complex stays Complex.
                        if matches!(op, BinaryOpKind::Divide) && promoted == LogosType::Decimal {
                            LogosType::Rational
                        } else {
                            promoted
                        }
                    }
                }
            }

            // `not x` — logical negation of truthiness, Bool out (`~` is the bitwise complement).
            Expr::Not { .. } => LogosType::Bool,

            Expr::Length { .. } => LogosType::Int,

            Expr::Call { function, args } => {
                let name = interner.resolve(*function);
                match name {
                    "pow"
                        if self.lookup_fn(*function).is_none()
                            || self.native_functions.contains(function) =>
                    {
                        match args.as_slice() {
                            [base, exponent] => {
                                let base = self.infer_expr_with(base, interner, field);
                                let exponent = self.infer_expr_with(exponent, interner, field);
                                if base == LogosType::Modular && exponent == LogosType::Int {
                                    LogosType::Modular
                                } else {
                                    LogosType::numeric_promotion(&base, &exponent)
                                }
                            }
                            _ => LogosType::Unknown,
                        }
                    }
                    "sqrt" | "parseFloat" => LogosType::Float,
                    "decimal" => LogosType::Decimal,
                    "complex" => LogosType::Complex,
                    "modular" => LogosType::Modular,
                    // `quantity(value, "unit")` and `convert(q, "unit")` both yield a Quantity.
                    "quantity" | "convert" => LogosType::Quantity,
                    "money" => LogosType::Money,
                    // `<money> in EUR` desugars to `to_currency(money, "EUR")` — still Money.
                    "to_currency" => LogosType::Money,
                    // UUID constructors all yield a Uuid; `uuid_version` reads its version nibble.
                    "uuid" | "uuid_nil" | "uuid_max" | "uuid_v1" | "uuid_v3" | "uuid_v4"
                    | "uuid_v5" | "uuid_v6" | "uuid_v7" | "uuid_v8" | "uuid_dns" | "uuid_url"
                    | "uuid_oid" | "uuid_x500" => LogosType::Uuid,
                    "uuid_version" => LogosType::Int,
                    // Byte-level primitives for the Logos-written UUID constructors: hashes and byte
                    // views are `Seq of Int`; `uuid_from_bytes` rebuilds a Uuid.
                    "md5" | "sha1" | "text_bytes" | "uuid_bytes" => {
                        LogosType::Seq(Box::new(LogosType::Int))
                    }
                    "readReals" => LogosType::Seq(Box::new(LogosType::Float)),
                    "uuid_from_bytes" => LogosType::Uuid,
                    "set_rate" | "set_rates" => LogosType::Unit,
                    "parse_timestamp" => LogosType::Moment,
                    "format_timestamp" => LogosType::String,
                    // The byte→Text builtin (the inverse of `text_bytes`): rebuilds a `String` from a
                    // `Seq of Int` of UTF-8 bytes. Typing it lets a `Let s be text_from_bytes(bs)` binding
                    // know `s` is a non-Copy `String`, so the AOT value-copy clone fires (a bare move would
                    // strand a later read of `s` as a use-after-move).
                    "text_from_bytes" => LogosType::String,
                    "year_of" | "month_of" | "day_of" | "weekday_of" | "hour_of" | "minute_of"
                    | "second_of" | "week_of" | "quarter_of" => LogosType::Int,
                    "date_of" => LogosType::Date,
                    "time_of" => LogosType::Time,
                    "makeDate" => LogosType::Date,
                    "makeMoment" => LogosType::Moment,
                    "seconds_between" | "months_between" | "years_between" => LogosType::Int,
                    "add_seconds" => LogosType::Moment,
                    "in_zone" => LogosType::String,
                    "local_instant" => LogosType::Moment,
                    "parseInt" | "floor" | "ceil" | "round" => LogosType::Int,
                    "abs" | "min" | "max" => {
                        // Preserves type of arguments — infer from first arg
                        if let Some(first) = args.first() {
                            self.infer_expr_with(first, interner, field)
                        } else {
                            LogosType::Unknown
                        }
                    }
                    // `{k: v, …}` desugars to `mapOf(k, v, …)` (flat pairs) — a Map of the first
                    // pair's key/value types; `{a, …}` to `setOf(a, …)` — a Set of the first
                    // element's type. Typing the literal lets numeric key coercion and element
                    // reads specialise.
                    "mapOf" => match (args.first(), args.get(1)) {
                        (Some(k), Some(v)) => LogosType::Map(
                            Box::new(self.infer_expr_with(k, interner, field)),
                            Box::new(self.infer_expr_with(v, interner, field)),
                        ),
                        _ => LogosType::Unknown,
                    },
                    "setOf" => match args.first() {
                        Some(e) => LogosType::Set(Box::new(self.infer_expr_with(e, interner, field))),
                        None => LogosType::Unknown,
                    },
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
                let coll_ty = self.infer_expr_with(collection, interner, field);
                match &coll_ty {
                    LogosType::Seq(inner) => (**inner).clone(),
                    LogosType::Map(_, v) => (**v).clone(),
                    _ => LogosType::Unknown,
                }
            }

            Expr::List(items) => {
                let mut types = items.iter().map(|e| self.infer_expr_with(e, interner, field));
                let first = types.next().unwrap_or(LogosType::Unknown);
                let elem_type = types.try_fold(first.clone(), |held, t| LogosType::number_join(&held, &t)).unwrap_or(first);
                LogosType::Seq(Box::new(elem_type))
            }

            Expr::New {
                type_name,
                type_args,
                ..
            } => {
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

            Expr::FieldAccess { .. } => field(expr).unwrap_or(LogosType::Unknown),

            Expr::OptionSome { value } => {
                let inner = self.infer_expr_with(value, interner, field);
                LogosType::Option(Box::new(inner))
            }

            Expr::OptionNone => LogosType::Option(Box::new(LogosType::Unknown)),

            Expr::Range { .. } => LogosType::Seq(Box::new(LogosType::Int)),

            Expr::Contains { .. } => LogosType::Bool,

            Expr::Copy { expr: inner } | Expr::Give { value: inner } => {
                self.infer_expr_with(inner, interner, field)
            }

            Expr::WithCapacity { value, .. } => self.infer_expr_with(value, interner, field),

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
                is_native,
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
                self.set_native_fn(*name, *is_native);
            }

            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
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

            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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

            Stmt::CreatePipe {
                var, element_type, ..
            } => {
                let elem = LogosType::from_type_name(interner.resolve(*element_type));
                self.register(*var, elem);
            }

            Stmt::ReceivePipe { var, .. } | Stmt::TryReceivePipe { var, .. } => {
                // Pipe receive type depends on pipe type, default to Unknown
                self.register(*var, LogosType::Unknown);
            }

            Stmt::Pop {
                into: Some(var),
                collection,
            } => {
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

            // A `Reversibly:` block runs its body in the enclosing scope (output-transparent),
            // so a variable it introduces / retypes is visible after the block — infer through it.
            Stmt::Reversibly { body } => {
                self.infer_stmts(body, interner);
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/types/tests.rs"]
mod tests;
