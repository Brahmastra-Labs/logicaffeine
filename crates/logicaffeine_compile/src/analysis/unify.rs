//! Unification engine for the bidirectional type checker.
//!
//! Provides Robinson unification for [`InferType`], which extends [`LogosType`]
//! with type variables for the inference pass. After inference, [`UnificationTable::zonk`]
//! converts `InferType → LogosType`. Unsolved variables become `LogosType::Unknown`,
//! preserving the existing codegen safety net.
//!
//! # Architecture
//!
//! ```text
//! InferType  ←  inference pass (this module + check.rs)
//!     │
//!     │  zonk (after inference)
//!     ▼
//! LogosType  →  codegen (unchanged)
//! ```

use std::collections::HashMap;

use crate::intern::{Interner, Symbol};
use crate::analysis::{FieldType, LogosType};

// ============================================================================
// Core types
// ============================================================================

/// A type variable used during inference.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TyVar(pub u32);

/// Inference-time type representation.
///
/// Extends [`LogosType`] with:
/// - [`InferType::Var`] — an unbound type variable
/// - [`InferType::Function`] — a function type (not needed in codegen)
/// - [`InferType::Unknown`] — propagated uncertainty (unifies with anything)
#[derive(Clone, PartialEq, Debug)]
pub enum InferType {
    // Ground types mirroring LogosType
    Int,
    Float,
    Bool,
    Char,
    Byte,
    String,
    Unit,
    Nat,
    Duration,
    Date,
    Moment,
    Time,
    Span,
    Seq(Box<InferType>),
    Map(Box<InferType>, Box<InferType>),
    Set(Box<InferType>),
    Option(Box<InferType>),
    UserDefined(Symbol),
    // Inference-only types
    Var(TyVar),
    Function(Vec<InferType>, Box<InferType>),
    Unknown,
}

/// A type error detected during unification or checking.
#[derive(Debug, Clone)]
pub enum TypeError {
    Mismatch { expected: InferType, found: InferType },
    InfiniteType { var: TyVar, ty: InferType },
    ArityMismatch { expected: usize, found: usize },
    FieldNotFound { type_name: Symbol, field_name: Symbol },
    NotAFunction { found: InferType },
}

impl TypeError {
    pub fn expected_str(&self) -> std::string::String {
        match self {
            TypeError::Mismatch { expected, .. } => expected.to_logos_name(),
            TypeError::ArityMismatch { expected, .. } => format!("{} arguments", expected),
            TypeError::FieldNotFound { .. } => "a known field".to_string(),
            TypeError::NotAFunction { .. } => "a function".to_string(),
            TypeError::InfiniteType { .. } => "a finite type".to_string(),
        }
    }

    pub fn found_str(&self) -> std::string::String {
        match self {
            TypeError::Mismatch { found, .. } => found.to_logos_name(),
            TypeError::ArityMismatch { found, .. } => format!("{} arguments", found),
            TypeError::FieldNotFound { field_name, .. } => format!("{:?}", field_name),
            TypeError::NotAFunction { found } => found.to_logos_name(),
            TypeError::InfiniteType { ty, .. } => ty.to_logos_name(),
        }
    }

    /// Convert this TypeError into a `ParseErrorKind` for user-facing reporting.
    ///
    /// Uses `interner` to resolve symbol names (field names, type names) into strings.
    pub fn to_parse_error_kind(
        &self,
        interner: &crate::intern::Interner,
    ) -> crate::error::ParseErrorKind {
        use crate::error::ParseErrorKind;
        match self {
            TypeError::Mismatch { expected, found } => ParseErrorKind::TypeMismatchDetailed {
                expected: expected.to_logos_name(),
                found: found.to_logos_name(),
                context: String::new(),
            },
            TypeError::InfiniteType { var, ty } => ParseErrorKind::InfiniteType {
                var_description: format!("type variable α{}", var.0),
                type_description: ty.to_logos_name(),
            },
            TypeError::ArityMismatch { expected, found } => ParseErrorKind::ArityMismatch {
                function: String::from("function"),
                expected: *expected,
                found: *found,
            },
            TypeError::FieldNotFound { type_name, field_name } => ParseErrorKind::FieldNotFound {
                type_name: interner.resolve(*type_name).to_string(),
                field_name: interner.resolve(*field_name).to_string(),
                available: vec![],
            },
            TypeError::NotAFunction { found } => ParseErrorKind::NotAFunction {
                found_type: found.to_logos_name(),
            },
        }
    }
}

// ============================================================================
// Unification table
// ============================================================================

/// A quantified polymorphic type.
///
/// Stores a set of bound type variables and a body type that may reference them.
/// Used for generic function signatures: `forall [T]. T -> T`.
#[derive(Clone, Debug)]
pub struct TypeScheme {
    /// The bound type variables (one per generic type parameter).
    pub vars: Vec<TyVar>,
    /// The body type, which may contain `InferType::Var(tv)` for each `tv` in `vars`.
    pub body: InferType,
}

/// Union-Find table implementing Robinson unification with occurs check.
///
/// Type variables are allocated by [`fresh`] and resolved by [`find`].
/// [`zonk`] fully resolves a type after inference, converting remaining
/// unbound variables to [`InferType::Unknown`] (which maps to `LogosType::Unknown`).
pub struct UnificationTable {
    bindings: Vec<Option<InferType>>,
    next_id: u32,
}

impl UnificationTable {
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            next_id: 0,
        }
    }

    /// Allocate a fresh unbound type variable, returning the wrapped `InferType`.
    pub fn fresh(&mut self) -> InferType {
        let id = self.next_id;
        self.next_id += 1;
        self.bindings.push(None);
        InferType::Var(TyVar(id))
    }

    /// Allocate a fresh unbound type variable, returning the raw `TyVar`.
    pub fn fresh_var(&mut self) -> TyVar {
        let id = self.next_id;
        self.next_id += 1;
        self.bindings.push(None);
        TyVar(id)
    }

    /// Instantiate a `TypeScheme` by replacing each quantified variable with a fresh one.
    ///
    /// Each call site of a generic function gets independent fresh variables so that
    /// two calls to `identity(42)` and `identity(true)` do not interfere.
    pub fn instantiate(&mut self, scheme: &TypeScheme) -> InferType {
        if scheme.vars.is_empty() {
            return scheme.body.clone();
        }
        let subst: HashMap<TyVar, TyVar> = scheme.vars.iter()
            .map(|&old_tv| (old_tv, self.fresh_var()))
            .collect();
        self.substitute_vars(&scheme.body, &subst)
    }

    /// Substitute type variables according to `subst`, walking the type recursively.
    fn substitute_vars(&self, ty: &InferType, subst: &HashMap<TyVar, TyVar>) -> InferType {
        match ty {
            InferType::Var(tv) => {
                let resolved = self.find(*tv);
                match &resolved {
                    InferType::Var(rtv) => {
                        if let Some(&new_tv) = subst.get(rtv) {
                            InferType::Var(new_tv)
                        } else {
                            InferType::Var(*rtv)
                        }
                    }
                    other => self.substitute_vars(&other.clone(), subst),
                }
            }
            InferType::Seq(inner) => InferType::Seq(Box::new(self.substitute_vars(inner, subst))),
            InferType::Map(k, v) => InferType::Map(
                Box::new(self.substitute_vars(k, subst)),
                Box::new(self.substitute_vars(v, subst)),
            ),
            InferType::Set(inner) => InferType::Set(Box::new(self.substitute_vars(inner, subst))),
            InferType::Option(inner) => InferType::Option(Box::new(self.substitute_vars(inner, subst))),
            InferType::Function(params, ret) => InferType::Function(
                params.iter().map(|p| self.substitute_vars(p, subst)).collect(),
                Box::new(self.substitute_vars(ret, subst)),
            ),
            other => other.clone(),
        }
    }

    /// Follow the binding chain for a type variable (iterative, no path compression).
    pub fn find(&self, tv: TyVar) -> InferType {
        let mut current = tv;
        loop {
            match &self.bindings[current.0 as usize] {
                None => return InferType::Var(current),
                Some(InferType::Var(tv2)) => current = *tv2,
                Some(ty) => return ty.clone(),
            }
        }
    }

    /// Walk the top level of a type: if it's a bound variable, resolve one step.
    fn walk(&self, ty: &InferType) -> InferType {
        match ty {
            InferType::Var(tv) => self.find(*tv),
            other => other.clone(),
        }
    }

    /// Resolve type variables, keeping unbound variables as `Var(tv)`.
    ///
    /// Unlike [`zonk`], this does not convert unbound variables to `Unknown`.
    /// Use this during inference to preserve generic type params as `Var(tv)`
    /// so they can be unified at call sites.
    pub fn resolve(&self, ty: &InferType) -> InferType {
        match ty {
            InferType::Var(tv) => {
                let resolved = self.find(*tv);
                match &resolved {
                    InferType::Var(_) => resolved, // keep as Var — intentionally unbound
                    other => self.resolve(&other.clone()),
                }
            }
            InferType::Seq(inner) => InferType::Seq(Box::new(self.resolve(inner))),
            InferType::Map(k, v) => {
                InferType::Map(Box::new(self.resolve(k)), Box::new(self.resolve(v)))
            }
            InferType::Set(inner) => InferType::Set(Box::new(self.resolve(inner))),
            InferType::Option(inner) => InferType::Option(Box::new(self.resolve(inner))),
            InferType::Function(params, ret) => {
                let params = params.iter().map(|p| self.resolve(p)).collect();
                InferType::Function(params, Box::new(self.resolve(ret)))
            }
            other => other.clone(),
        }
    }

    /// Fully resolve all type variables in a type.
    ///
    /// Unbound variables become [`InferType::Unknown`], which maps to
    /// `LogosType::Unknown` when converted by [`to_logos_type`].
    pub fn zonk(&self, ty: &InferType) -> InferType {
        match ty {
            InferType::Var(tv) => {
                let resolved = self.find(*tv);
                match &resolved {
                    InferType::Var(_) => InferType::Unknown,
                    other => self.zonk(other),
                }
            }
            InferType::Seq(inner) => InferType::Seq(Box::new(self.zonk(inner))),
            InferType::Map(k, v) => {
                InferType::Map(Box::new(self.zonk(k)), Box::new(self.zonk(v)))
            }
            InferType::Set(inner) => InferType::Set(Box::new(self.zonk(inner))),
            InferType::Option(inner) => InferType::Option(Box::new(self.zonk(inner))),
            InferType::Function(params, ret) => {
                let params = params.iter().map(|p| self.zonk(p)).collect();
                InferType::Function(params, Box::new(self.zonk(ret)))
            }
            other => other.clone(),
        }
    }

    /// Zonk then convert to `LogosType`. Unsolved variables become `Unknown`.
    pub fn to_logos_type(&self, ty: &InferType) -> LogosType {
        let zonked = self.zonk(ty);
        infer_to_logos(&zonked)
    }

    /// Unify two types, binding type variables as needed.
    ///
    /// Returns `Ok(())` if unification succeeds (bindings may be updated).
    /// Returns `Err(TypeError)` on a genuine type contradiction.
    pub fn unify(&mut self, a: &InferType, b: &InferType) -> Result<(), TypeError> {
        let a = self.walk(a);
        let b = self.walk(b);
        self.unify_walked(&a, &b)
    }

    fn unify_walked(&mut self, a: &InferType, b: &InferType) -> Result<(), TypeError> {
        match (a, b) {
            // Same variable: trivially unified
            (InferType::Var(va), InferType::Var(vb)) if va == vb => Ok(()),

            // Bind a variable to a type
            (InferType::Var(tv), ty) => {
                let tv = *tv;
                let ty = ty.clone();
                self.occurs_check(tv, &ty)?;
                self.bindings[tv.0 as usize] = Some(ty);
                Ok(())
            }
            (ty, InferType::Var(tv)) => {
                let tv = *tv;
                let ty = ty.clone();
                self.occurs_check(tv, &ty)?;
                self.bindings[tv.0 as usize] = Some(ty);
                Ok(())
            }

            // Unknown unifies with anything (propagated uncertainty)
            (InferType::Unknown, _) | (_, InferType::Unknown) => Ok(()),

            // Ground type equality
            (InferType::Int, InferType::Int) => Ok(()),
            (InferType::Float, InferType::Float) => Ok(()),
            (InferType::Bool, InferType::Bool) => Ok(()),
            (InferType::Char, InferType::Char) => Ok(()),
            (InferType::Byte, InferType::Byte) => Ok(()),
            (InferType::String, InferType::String) => Ok(()),
            (InferType::Unit, InferType::Unit) => Ok(()),
            (InferType::Nat, InferType::Nat) => Ok(()),
            (InferType::Duration, InferType::Duration) => Ok(()),
            (InferType::Date, InferType::Date) => Ok(()),
            (InferType::Moment, InferType::Moment) => Ok(()),
            (InferType::Time, InferType::Time) => Ok(()),
            (InferType::Span, InferType::Span) => Ok(()),

            // Nat embeds into Int for numeric contexts
            (InferType::Nat, InferType::Int) | (InferType::Int, InferType::Nat) => Ok(()),

            // User-defined types unify if same name
            (InferType::UserDefined(a), InferType::UserDefined(b)) if a == b => Ok(()),

            // Structural recursion
            (InferType::Seq(a_inner), InferType::Seq(b_inner)) => {
                let a_inner = (**a_inner).clone();
                let b_inner = (**b_inner).clone();
                self.unify(&a_inner, &b_inner)
            }
            (InferType::Set(a_inner), InferType::Set(b_inner)) => {
                let a_inner = (**a_inner).clone();
                let b_inner = (**b_inner).clone();
                self.unify(&a_inner, &b_inner)
            }
            (InferType::Option(a_inner), InferType::Option(b_inner)) => {
                let a_inner = (**a_inner).clone();
                let b_inner = (**b_inner).clone();
                self.unify(&a_inner, &b_inner)
            }
            (InferType::Map(ak, av), InferType::Map(bk, bv)) => {
                let ak = (**ak).clone();
                let bk = (**bk).clone();
                let av = (**av).clone();
                let bv = (**bv).clone();
                self.unify(&ak, &bk)?;
                self.unify(&av, &bv)
            }
            (InferType::Function(a_params, a_ret), InferType::Function(b_params, b_ret)) => {
                if a_params.len() != b_params.len() {
                    return Err(TypeError::ArityMismatch {
                        expected: a_params.len(),
                        found: b_params.len(),
                    });
                }
                let a_params = a_params.clone();
                let b_params = b_params.clone();
                let a_ret = (**a_ret).clone();
                let b_ret = (**b_ret).clone();
                for (ap, bp) in a_params.iter().zip(b_params.iter()) {
                    self.unify(ap, bp)?;
                }
                self.unify(&a_ret, &b_ret)
            }

            // Type mismatch
            (a, b) => Err(TypeError::Mismatch {
                expected: a.clone(),
                found: b.clone(),
            }),
        }
    }

    /// Check that `tv` does not appear in `ty`.
    ///
    /// Prevents infinite types like `α = List<α>`.
    fn occurs_check(&self, tv: TyVar, ty: &InferType) -> Result<(), TypeError> {
        match ty {
            InferType::Var(tv2) => {
                let resolved = self.find(*tv2);
                match &resolved {
                    InferType::Var(rtv) => {
                        if *rtv == tv {
                            Err(TypeError::InfiniteType { var: tv, ty: ty.clone() })
                        } else {
                            Ok(())
                        }
                    }
                    other => self.occurs_check(tv, &other.clone()),
                }
            }
            InferType::Seq(inner) | InferType::Set(inner) | InferType::Option(inner) => {
                self.occurs_check(tv, inner)
            }
            InferType::Map(k, v) => {
                self.occurs_check(tv, k)?;
                self.occurs_check(tv, v)
            }
            InferType::Function(params, ret) => {
                for p in params {
                    self.occurs_check(tv, p)?;
                }
                self.occurs_check(tv, ret)
            }
            _ => Ok(()),
        }
    }
}

// ============================================================================
// InferType helpers
// ============================================================================

impl InferType {
    /// Convert a `TypeExpr` AST node to `InferType`.
    ///
    /// Unlike `LogosType::from_type_expr`, this correctly handles
    /// `TypeExpr::Function { inputs, output }` by producing
    /// `InferType::Function(...)` instead of `Unknown`.
    pub fn from_type_expr(ty: &crate::ast::stmt::TypeExpr, interner: &Interner) -> InferType {
        Self::from_type_expr_with_params(ty, interner, &HashMap::new())
    }

    /// Convert a `TypeExpr` to `InferType`, substituting generic type parameters.
    ///
    /// When `type_params` maps a name like `T` to a `TyVar`, any `TypeExpr::Primitive(T)`
    /// or `TypeExpr::Named(T)` in the expression produces `InferType::Var(tv)` instead
    /// of `Unknown`. This is used for generic function signatures where `T` is a type
    /// parameter, not a concrete type name.
    pub fn from_type_expr_with_params(
        ty: &crate::ast::stmt::TypeExpr,
        interner: &Interner,
        type_params: &HashMap<Symbol, TyVar>,
    ) -> InferType {
        use crate::ast::stmt::TypeExpr;
        match ty {
            TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                // Check if this name is a generic type parameter
                if let Some(&tv) = type_params.get(sym) {
                    return InferType::Var(tv);
                }
                Self::from_type_name(interner.resolve(*sym))
            }
            TypeExpr::Generic { base, params } => {
                let base_name = interner.resolve(*base);
                match base_name {
                    "Seq" | "List" | "Vec" => {
                        let elem = params
                            .first()
                            .map(|p| InferType::from_type_expr_with_params(p, interner, type_params))
                            .unwrap_or(InferType::Unit);
                        InferType::Seq(Box::new(elem))
                    }
                    "Map" | "HashMap" => {
                        let key = params
                            .first()
                            .map(|p| InferType::from_type_expr_with_params(p, interner, type_params))
                            .unwrap_or(InferType::String);
                        let val = params
                            .get(1)
                            .map(|p| InferType::from_type_expr_with_params(p, interner, type_params))
                            .unwrap_or(InferType::String);
                        InferType::Map(Box::new(key), Box::new(val))
                    }
                    "Set" | "HashSet" => {
                        let elem = params
                            .first()
                            .map(|p| InferType::from_type_expr_with_params(p, interner, type_params))
                            .unwrap_or(InferType::Unit);
                        InferType::Set(Box::new(elem))
                    }
                    "Option" | "Maybe" => {
                        let inner = params
                            .first()
                            .map(|p| InferType::from_type_expr_with_params(p, interner, type_params))
                            .unwrap_or(InferType::Unit);
                        InferType::Option(Box::new(inner))
                    }
                    _ => InferType::Unknown,
                }
            }
            TypeExpr::Refinement { base, .. } => {
                InferType::from_type_expr_with_params(base, interner, type_params)
            }
            TypeExpr::Persistent { inner } => {
                InferType::from_type_expr_with_params(inner, interner, type_params)
            }
            TypeExpr::Function { inputs, output } => {
                let param_types: Vec<InferType> = inputs
                    .iter()
                    .map(|p| InferType::from_type_expr_with_params(p, interner, type_params))
                    .collect();
                let ret_type = InferType::from_type_expr_with_params(output, interner, type_params);
                InferType::Function(param_types, Box::new(ret_type))
            }
        }
    }

    /// Convert a registry `FieldType` to `InferType`.
    ///
    /// `type_params` maps generic type parameter names (e.g., `T`, `U`) to
    /// type variable indices allocated by [`UnificationTable::fresh`].
    pub fn from_field_type(
        ty: &FieldType,
        interner: &Interner,
        type_params: &HashMap<Symbol, TyVar>,
    ) -> InferType {
        match ty {
            FieldType::Primitive(sym) => InferType::from_type_name(interner.resolve(*sym)),
            FieldType::Named(sym) => {
                let name = interner.resolve(*sym);
                let primitive = InferType::from_type_name(name);
                if primitive == InferType::Unknown {
                    InferType::UserDefined(*sym)
                } else {
                    primitive
                }
            }
            FieldType::Generic { base, params } => {
                let base_name = interner.resolve(*base);
                let converted: Vec<InferType> = params
                    .iter()
                    .map(|p| InferType::from_field_type(p, interner, type_params))
                    .collect();
                match base_name {
                    "Seq" | "List" | "Vec" => {
                        InferType::Seq(Box::new(
                            converted.into_iter().next().unwrap_or(InferType::Unit),
                        ))
                    }
                    "Map" | "HashMap" => {
                        let mut it = converted.into_iter();
                        let k = it.next().unwrap_or(InferType::String);
                        let v = it.next().unwrap_or(InferType::String);
                        InferType::Map(Box::new(k), Box::new(v))
                    }
                    "Set" | "HashSet" => {
                        InferType::Set(Box::new(
                            converted.into_iter().next().unwrap_or(InferType::Unit),
                        ))
                    }
                    "Option" | "Maybe" => {
                        InferType::Option(Box::new(
                            converted.into_iter().next().unwrap_or(InferType::Unit),
                        ))
                    }
                    _ => InferType::Unknown,
                }
            }
            FieldType::TypeParam(sym) => {
                if let Some(tv) = type_params.get(sym) {
                    InferType::Var(*tv)
                } else {
                    InferType::Unknown
                }
            }
        }
    }

    /// Infer `InferType` from a literal value.
    pub fn from_literal(lit: &crate::ast::stmt::Literal) -> InferType {
        use crate::ast::stmt::Literal;
        match lit {
            Literal::Number(_) => InferType::Int,
            Literal::Float(_) => InferType::Float,
            Literal::Text(_) => InferType::String,
            Literal::Boolean(_) => InferType::Bool,
            Literal::Char(_) => InferType::Char,
            Literal::Nothing => InferType::Unit,
            Literal::Duration(_) => InferType::Duration,
            Literal::Date(_) => InferType::Date,
            Literal::Moment(_) => InferType::Moment,
            Literal::Span { .. } => InferType::Span,
            Literal::Time(_) => InferType::Time,
        }
    }

    /// Parse a LOGOS type name into `InferType`.
    pub fn from_type_name(name: &str) -> InferType {
        match name {
            "Int" => InferType::Int,
            "Nat" => InferType::Nat,
            "Real" | "Float" => InferType::Float,
            "Bool" | "Boolean" => InferType::Bool,
            "Text" | "String" => InferType::String,
            "Char" => InferType::Char,
            "Byte" => InferType::Byte,
            "Unit" | "()" => InferType::Unit,
            "Duration" => InferType::Duration,
            "Date" => InferType::Date,
            "Moment" => InferType::Moment,
            "Time" => InferType::Time,
            "Span" => InferType::Span,
            _ => InferType::Unknown,
        }
    }

    /// Human-readable type name in Logos terms, for error messages.
    pub fn to_logos_name(&self) -> std::string::String {
        match self {
            InferType::Int => "Int".into(),
            InferType::Float => "Real".into(),
            InferType::Bool => "Bool".into(),
            InferType::Char => "Char".into(),
            InferType::Byte => "Byte".into(),
            InferType::String => "Text".into(),
            InferType::Unit => "Unit".into(),
            InferType::Nat => "Nat".into(),
            InferType::Duration => "Duration".into(),
            InferType::Date => "Date".into(),
            InferType::Moment => "Moment".into(),
            InferType::Time => "Time".into(),
            InferType::Span => "Span".into(),
            InferType::Seq(inner) => format!("Seq of {}", inner.to_logos_name()),
            InferType::Map(k, v) => {
                format!("Map of {} and {}", k.to_logos_name(), v.to_logos_name())
            }
            InferType::Set(inner) => format!("Set of {}", inner.to_logos_name()),
            InferType::Option(inner) => format!("Option of {}", inner.to_logos_name()),
            InferType::UserDefined(_) => "a user-defined type".into(),
            InferType::Var(_) => "an unknown type".into(),
            InferType::Function(params, ret) => {
                let params_str = params
                    .iter()
                    .map(|p| p.to_logos_name())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("fn({}) -> {}", params_str, ret.to_logos_name())
            }
            InferType::Unknown => "unknown".into(),
        }
    }

    /// Convert a known-ground `InferType` to `LogosType`.
    ///
    /// # Panics
    ///
    /// Panics if called on a `Var`. Callers must [`zonk`] first.
    pub fn to_logos_type_ground(&self) -> LogosType {
        match self {
            InferType::Int => LogosType::Int,
            InferType::Float => LogosType::Float,
            InferType::Bool => LogosType::Bool,
            InferType::Char => LogosType::Char,
            InferType::Byte => LogosType::Byte,
            InferType::String => LogosType::String,
            InferType::Unit => LogosType::Unit,
            InferType::Nat => LogosType::Nat,
            InferType::Duration => LogosType::Duration,
            InferType::Date => LogosType::Date,
            InferType::Moment => LogosType::Moment,
            InferType::Time => LogosType::Time,
            InferType::Span => LogosType::Span,
            InferType::Seq(inner) => LogosType::Seq(Box::new(inner.to_logos_type_ground())),
            InferType::Map(k, v) => LogosType::Map(
                Box::new(k.to_logos_type_ground()),
                Box::new(v.to_logos_type_ground()),
            ),
            InferType::Set(inner) => LogosType::Set(Box::new(inner.to_logos_type_ground())),
            InferType::Option(inner) => LogosType::Option(Box::new(inner.to_logos_type_ground())),
            InferType::UserDefined(sym) => LogosType::UserDefined(*sym),
            InferType::Function(params, ret) => LogosType::Function(
                params.iter().map(|p| p.to_logos_type_ground()).collect(),
                Box::new(ret.to_logos_type_ground()),
            ),
            InferType::Unknown => LogosType::Unknown,
            InferType::Var(_) => panic!("to_logos_type_ground called on unresolved Var"),
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Numeric promotion: Float wins; Int + Int = Int; Nat + Nat = Nat; Byte + Byte = Byte; otherwise error.
pub fn unify_numeric(a: &InferType, b: &InferType) -> Result<InferType, TypeError> {
    match (a, b) {
        (InferType::Float, _) | (_, InferType::Float) => Ok(InferType::Float),
        (InferType::Int, InferType::Int) => Ok(InferType::Int),
        (InferType::Nat, InferType::Int) | (InferType::Int, InferType::Nat) => Ok(InferType::Int),
        (InferType::Nat, InferType::Nat) => Ok(InferType::Nat),
        (InferType::Byte, InferType::Byte) => Ok(InferType::Byte),
        _ => Err(TypeError::Mismatch {
            expected: InferType::Int,
            found: a.clone(),
        }),
    }
}

/// Convert a zonked `InferType` to `LogosType`.
pub fn infer_to_logos(ty: &InferType) -> LogosType {
    match ty {
        InferType::Int => LogosType::Int,
        InferType::Float => LogosType::Float,
        InferType::Bool => LogosType::Bool,
        InferType::Char => LogosType::Char,
        InferType::Byte => LogosType::Byte,
        InferType::String => LogosType::String,
        InferType::Unit => LogosType::Unit,
        InferType::Nat => LogosType::Nat,
        InferType::Duration => LogosType::Duration,
        InferType::Date => LogosType::Date,
        InferType::Moment => LogosType::Moment,
        InferType::Time => LogosType::Time,
        InferType::Span => LogosType::Span,
        InferType::Seq(inner) => LogosType::Seq(Box::new(infer_to_logos(inner))),
        InferType::Map(k, v) => {
            LogosType::Map(Box::new(infer_to_logos(k)), Box::new(infer_to_logos(v)))
        }
        InferType::Set(inner) => LogosType::Set(Box::new(infer_to_logos(inner))),
        InferType::Option(inner) => LogosType::Option(Box::new(infer_to_logos(inner))),
        InferType::UserDefined(sym) => LogosType::UserDefined(*sym),
        InferType::Function(params, ret) => LogosType::Function(
            params.iter().map(infer_to_logos).collect(),
            Box::new(infer_to_logos(ret)),
        ),
        InferType::Unknown | InferType::Var(_) => LogosType::Unknown,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{FieldDef, TypeDef};

    // =========================================================================
    // UnificationTable::fresh / find
    // =========================================================================

    #[test]
    fn fresh_produces_distinct_vars() {
        let mut table = UnificationTable::new();
        let a = table.fresh();
        let b = table.fresh();
        assert_ne!(a, b);
    }

    #[test]
    fn unbound_var_finds_itself() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            assert_eq!(table.find(tv), InferType::Var(tv));
        } else {
            panic!("expected Var");
        }
    }

    // =========================================================================
    // Unify ground types
    // =========================================================================

    #[test]
    fn unify_identical_ground_types() {
        let mut table = UnificationTable::new();
        assert!(table.unify(&InferType::Int, &InferType::Int).is_ok());
        assert!(table.unify(&InferType::Float, &InferType::Float).is_ok());
        assert!(table.unify(&InferType::Bool, &InferType::Bool).is_ok());
        assert!(table.unify(&InferType::String, &InferType::String).is_ok());
        assert!(table.unify(&InferType::Unit, &InferType::Unit).is_ok());
    }

    #[test]
    fn unify_different_ground_types_fails() {
        let mut table = UnificationTable::new();
        let result = table.unify(&InferType::Int, &InferType::String);
        assert!(result.is_err());
        assert!(matches!(result, Err(TypeError::Mismatch { .. })));
    }

    #[test]
    fn unify_int_float_fails() {
        let mut table = UnificationTable::new();
        let result = table.unify(&InferType::Int, &InferType::Float);
        assert!(result.is_err());
    }

    #[test]
    fn unify_nat_int_succeeds() {
        let mut table = UnificationTable::new();
        assert!(table.unify(&InferType::Nat, &InferType::Int).is_ok());
        assert!(table.unify(&InferType::Int, &InferType::Nat).is_ok());
    }

    #[test]
    fn unify_unknown_with_any_succeeds() {
        let mut table = UnificationTable::new();
        assert!(table.unify(&InferType::Unknown, &InferType::Int).is_ok());
        assert!(table.unify(&InferType::String, &InferType::Unknown).is_ok());
        assert!(table.unify(&InferType::Unknown, &InferType::Unknown).is_ok());
    }

    // =========================================================================
    // Unify variables with ground types
    // =========================================================================

    #[test]
    fn var_unifies_with_int() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            table.unify(&InferType::Var(tv), &InferType::Int).unwrap();
            assert_eq!(table.find(tv), InferType::Int);
        }
    }

    #[test]
    fn int_unifies_with_var() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            table.unify(&InferType::Int, &InferType::Var(tv)).unwrap();
            assert_eq!(table.find(tv), InferType::Int);
        }
    }

    #[test]
    fn two_vars_unify_chain() {
        let mut table = UnificationTable::new();
        let va = table.fresh();
        let vb = table.fresh();
        let tva = if let InferType::Var(tv) = va { tv } else { panic!() };
        let tvb = if let InferType::Var(tv) = vb { tv } else { panic!() };
        table.unify(&InferType::Var(tva), &InferType::Var(tvb)).unwrap();
        // Bind tvb to Int, then zonk(tva) should follow chain to Int
        table.unify(&InferType::Var(tvb), &InferType::Int).unwrap();
        let zonked = table.zonk(&InferType::Var(tva));
        assert_eq!(zonked, InferType::Int);
    }

    #[test]
    fn var_conflicting_types_fails() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            table.unify(&InferType::Var(tv), &InferType::Int).unwrap();
            let result = table.unify(&InferType::Var(tv), &InferType::String);
            // After binding tv → Int, unifying Int with String fails
            assert!(result.is_err());
        }
    }

    // =========================================================================
    // Occurs check
    // =========================================================================

    #[test]
    fn occurs_check_detects_infinite_type() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            let circular = InferType::Seq(Box::new(InferType::Var(tv)));
            let result = table.unify(&InferType::Var(tv), &circular);
            assert!(result.is_err());
            assert!(matches!(result, Err(TypeError::InfiniteType { .. })));
        }
    }

    // =========================================================================
    // Zonk
    // =========================================================================

    #[test]
    fn zonk_resolves_bound_var() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            table.unify(&InferType::Var(tv), &InferType::Bool).unwrap();
            let zonked = table.zonk(&InferType::Var(tv));
            assert_eq!(zonked, InferType::Bool);
        }
    }

    #[test]
    fn zonk_unbound_var_becomes_unknown() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            let zonked = table.zonk(&InferType::Var(tv));
            assert_eq!(zonked, InferType::Unknown);
        }
    }

    #[test]
    fn zonk_nested_resolves_inner_var() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            table.unify(&InferType::Var(tv), &InferType::Int).unwrap();
            let ty = InferType::Seq(Box::new(InferType::Var(tv)));
            let zonked = table.zonk(&ty);
            assert_eq!(zonked, InferType::Seq(Box::new(InferType::Int)));
        }
    }

    #[test]
    fn zonk_chain_of_vars() {
        let mut table = UnificationTable::new();
        let tva = if let InferType::Var(tv) = table.fresh() { tv } else { panic!() };
        let tvb = if let InferType::Var(tv) = table.fresh() { tv } else { panic!() };
        let tvc = if let InferType::Var(tv) = table.fresh() { tv } else { panic!() };
        // Chain: tva → tvb → tvc → Float
        table.unify(&InferType::Var(tva), &InferType::Var(tvb)).unwrap();
        table.unify(&InferType::Var(tvb), &InferType::Var(tvc)).unwrap();
        table.unify(&InferType::Var(tvc), &InferType::Float).unwrap();
        assert_eq!(table.zonk(&InferType::Var(tva)), InferType::Float);
    }

    // =========================================================================
    // Nested generic unification
    // =========================================================================

    #[test]
    fn unify_seq_of_same_type() {
        let mut table = UnificationTable::new();
        let a = InferType::Seq(Box::new(InferType::Int));
        let b = InferType::Seq(Box::new(InferType::Int));
        assert!(table.unify(&a, &b).is_ok());
    }

    #[test]
    fn unify_seq_of_different_types_fails() {
        let mut table = UnificationTable::new();
        let a = InferType::Seq(Box::new(InferType::Int));
        let b = InferType::Seq(Box::new(InferType::String));
        assert!(table.unify(&a, &b).is_err());
    }

    #[test]
    fn unify_seq_with_var_element() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            let a = InferType::Seq(Box::new(InferType::Var(tv)));
            let b = InferType::Seq(Box::new(InferType::Float));
            table.unify(&a, &b).unwrap();
            assert_eq!(table.find(tv), InferType::Float);
        }
    }

    #[test]
    fn unify_map_types() {
        let mut table = UnificationTable::new();
        let a = InferType::Map(Box::new(InferType::String), Box::new(InferType::Int));
        let b = InferType::Map(Box::new(InferType::String), Box::new(InferType::Int));
        assert!(table.unify(&a, &b).is_ok());
    }

    #[test]
    fn unify_function_types_same_arity() {
        let mut table = UnificationTable::new();
        let a = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
        let b = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
        assert!(table.unify(&a, &b).is_ok());
    }

    #[test]
    fn unify_function_arity_mismatch_fails() {
        let mut table = UnificationTable::new();
        let a = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
        let b = InferType::Function(
            vec![InferType::Int, InferType::Int],
            Box::new(InferType::Bool),
        );
        let result = table.unify(&a, &b);
        assert!(matches!(result, Err(TypeError::ArityMismatch { expected: 1, found: 2 })));
    }

    // =========================================================================
    // InferType → LogosType conversion (to_logos_type)
    // =========================================================================

    #[test]
    fn to_logos_type_ground_primitives() {
        let table = UnificationTable::new();
        assert_eq!(table.to_logos_type(&InferType::Int), LogosType::Int);
        assert_eq!(table.to_logos_type(&InferType::Float), LogosType::Float);
        assert_eq!(table.to_logos_type(&InferType::Bool), LogosType::Bool);
        assert_eq!(table.to_logos_type(&InferType::String), LogosType::String);
        assert_eq!(table.to_logos_type(&InferType::Unit), LogosType::Unit);
        assert_eq!(table.to_logos_type(&InferType::Nat), LogosType::Nat);
    }

    #[test]
    fn to_logos_type_unbound_var_becomes_unknown() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        assert_eq!(table.to_logos_type(&v), LogosType::Unknown);
    }

    #[test]
    fn to_logos_type_bound_var_resolves() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            table.unify(&InferType::Var(tv), &InferType::Int).unwrap();
            assert_eq!(table.to_logos_type(&InferType::Var(tv)), LogosType::Int);
        }
    }

    #[test]
    fn to_logos_type_seq_resolves_inner() {
        let mut table = UnificationTable::new();
        let v = table.fresh();
        if let InferType::Var(tv) = v {
            table.unify(&InferType::Var(tv), &InferType::String).unwrap();
            let ty = InferType::Seq(Box::new(InferType::Var(tv)));
            assert_eq!(
                table.to_logos_type(&ty),
                LogosType::Seq(Box::new(LogosType::String))
            );
        }
    }

    #[test]
    fn to_logos_type_function_converts_to_logos_function() {
        let table = UnificationTable::new();
        let ty = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
        assert_eq!(
            table.to_logos_type(&ty),
            LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool))
        );
    }

    #[test]
    fn to_logos_type_function_two_params_converts() {
        let table = UnificationTable::new();
        let ty = InferType::Function(
            vec![InferType::Int, InferType::String],
            Box::new(InferType::Bool),
        );
        assert_eq!(
            table.to_logos_type(&ty),
            LogosType::Function(
                vec![LogosType::Int, LogosType::String],
                Box::new(LogosType::Bool)
            )
        );
    }

    #[test]
    fn to_logos_type_function_zero_params_converts() {
        let table = UnificationTable::new();
        let ty = InferType::Function(vec![], Box::new(InferType::Unit));
        assert_eq!(
            table.to_logos_type(&ty),
            LogosType::Function(vec![], Box::new(LogosType::Unit))
        );
    }

    #[test]
    fn to_logos_type_function_nested_converts() {
        // fn(fn(Int) -> Bool) -> String
        let table = UnificationTable::new();
        let inner = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
        let outer = InferType::Function(vec![inner], Box::new(InferType::String));
        assert_eq!(
            table.to_logos_type(&outer),
            LogosType::Function(
                vec![LogosType::Function(
                    vec![LogosType::Int],
                    Box::new(LogosType::Bool)
                )],
                Box::new(LogosType::String)
            )
        );
    }

    // =========================================================================
    // from_type_expr
    // =========================================================================

    #[test]
    fn from_type_expr_function_produces_function_type() {
        use crate::ast::stmt::TypeExpr;
        let mut interner = Interner::new();
        let int_sym = interner.intern("Int");
        let bool_sym = interner.intern("Bool");
        let int_ty = TypeExpr::Primitive(int_sym);
        let bool_ty = TypeExpr::Primitive(bool_sym);
        let fn_ty = TypeExpr::Function {
            inputs: std::slice::from_ref(&int_ty),
            output: &bool_ty,
        };
        let result = InferType::from_type_expr(&fn_ty, &interner);
        assert_eq!(
            result,
            InferType::Function(vec![InferType::Int], Box::new(InferType::Bool))
        );
    }

    #[test]
    fn from_type_expr_seq_of_int() {
        use crate::ast::stmt::TypeExpr;
        let mut interner = Interner::new();
        let seq_sym = interner.intern("Seq");
        let int_sym = interner.intern("Int");
        let int_ty = TypeExpr::Primitive(int_sym);
        let ty = TypeExpr::Generic {
            base: seq_sym,
            params: std::slice::from_ref(&int_ty),
        };
        let result = InferType::from_type_expr(&ty, &interner);
        assert_eq!(result, InferType::Seq(Box::new(InferType::Int)));
    }

    // =========================================================================
    // from_field_type with TypeParam
    // =========================================================================

    #[test]
    fn from_field_type_type_param_resolves_to_var() {
        let mut interner = Interner::new();
        let t_sym = interner.intern("T");
        let tv = TyVar(0);
        let mut type_params = HashMap::new();
        type_params.insert(t_sym, tv);

        let field_ty = FieldType::TypeParam(t_sym);
        let result = InferType::from_field_type(&field_ty, &interner, &type_params);
        assert_eq!(result, InferType::Var(tv));
    }

    #[test]
    fn from_field_type_missing_type_param_becomes_unknown() {
        let mut interner = Interner::new();
        let t_sym = interner.intern("T");
        let type_params = HashMap::new();
        let field_ty = FieldType::TypeParam(t_sym);
        let result = InferType::from_field_type(&field_ty, &interner, &type_params);
        assert_eq!(result, InferType::Unknown);
    }

    #[test]
    fn from_field_type_primitive() {
        let mut interner = Interner::new();
        let int_sym = interner.intern("Int");
        let field_ty = FieldType::Primitive(int_sym);
        let result = InferType::from_field_type(&field_ty, &interner, &HashMap::new());
        assert_eq!(result, InferType::Int);
    }

    #[test]
    fn from_field_type_generic_seq_of_type_param() {
        let mut interner = Interner::new();
        let seq_sym = interner.intern("Seq");
        let t_sym = interner.intern("T");
        let tv = TyVar(0);
        let mut type_params = HashMap::new();
        type_params.insert(t_sym, tv);

        let field_ty = FieldType::Generic {
            base: seq_sym,
            params: vec![FieldType::TypeParam(t_sym)],
        };
        let result = InferType::from_field_type(&field_ty, &interner, &type_params);
        assert_eq!(result, InferType::Seq(Box::new(InferType::Var(tv))));
    }

    // =========================================================================
    // unify_numeric helper
    // =========================================================================

    #[test]
    fn numeric_float_wins() {
        assert_eq!(
            unify_numeric(&InferType::Int, &InferType::Float).unwrap(),
            InferType::Float
        );
        assert_eq!(
            unify_numeric(&InferType::Float, &InferType::Int).unwrap(),
            InferType::Float
        );
    }

    #[test]
    fn numeric_int_plus_int_is_int() {
        assert_eq!(
            unify_numeric(&InferType::Int, &InferType::Int).unwrap(),
            InferType::Int
        );
    }

    #[test]
    fn numeric_nat_plus_int_is_int() {
        assert_eq!(
            unify_numeric(&InferType::Nat, &InferType::Int).unwrap(),
            InferType::Int
        );
    }

    #[test]
    fn numeric_nat_plus_nat_is_nat() {
        assert_eq!(
            unify_numeric(&InferType::Nat, &InferType::Nat).unwrap(),
            InferType::Nat
        );
    }

    #[test]
    fn numeric_string_fails() {
        let result = unify_numeric(&InferType::String, &InferType::Int);
        assert!(result.is_err());
    }

    // =========================================================================
    // to_logos_name
    // =========================================================================

    #[test]
    fn logos_name_primitives() {
        assert_eq!(InferType::Int.to_logos_name(), "Int");
        assert_eq!(InferType::Float.to_logos_name(), "Real");
        assert_eq!(InferType::String.to_logos_name(), "Text");
        assert_eq!(InferType::Bool.to_logos_name(), "Bool");
    }

    #[test]
    fn logos_name_seq() {
        let ty = InferType::Seq(Box::new(InferType::Int));
        assert_eq!(ty.to_logos_name(), "Seq of Int");
    }

    #[test]
    fn logos_name_function() {
        let ty = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
        assert_eq!(ty.to_logos_name(), "fn(Int) -> Bool");
    }

    // =========================================================================
    // TypeError helpers
    // =========================================================================

    #[test]
    fn type_error_mismatch_strings() {
        let err = TypeError::Mismatch {
            expected: InferType::Int,
            found: InferType::String,
        };
        assert_eq!(err.expected_str(), "Int");
        assert_eq!(err.found_str(), "Text");
    }

    #[test]
    fn type_error_arity_mismatch_strings() {
        let err = TypeError::ArityMismatch { expected: 2, found: 3 };
        assert_eq!(err.expected_str(), "2 arguments");
        assert_eq!(err.found_str(), "3 arguments");
    }

    // =========================================================================
    // Phase 5: infer_to_logos for Function types
    // =========================================================================

    #[test]
    fn infer_to_logos_function_single_param() {
        let ty = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
        assert_eq!(
            super::infer_to_logos(&ty),
            LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool))
        );
    }

    #[test]
    fn infer_to_logos_function_zero_params() {
        let ty = InferType::Function(vec![], Box::new(InferType::Unit));
        assert_eq!(
            super::infer_to_logos(&ty),
            LogosType::Function(vec![], Box::new(LogosType::Unit))
        );
    }

    #[test]
    fn infer_to_logos_function_two_params() {
        let ty = InferType::Function(
            vec![InferType::String, InferType::Float],
            Box::new(InferType::Bool),
        );
        assert_eq!(
            super::infer_to_logos(&ty),
            LogosType::Function(
                vec![LogosType::String, LogosType::Float],
                Box::new(LogosType::Bool)
            )
        );
    }

    #[test]
    fn infer_to_logos_function_nested() {
        // fn(fn(Int) -> Bool) -> String
        let inner = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
        let outer = InferType::Function(vec![inner], Box::new(InferType::String));
        assert_eq!(
            super::infer_to_logos(&outer),
            LogosType::Function(
                vec![LogosType::Function(
                    vec![LogosType::Int],
                    Box::new(LogosType::Bool)
                )],
                Box::new(LogosType::String)
            )
        );
    }
}
