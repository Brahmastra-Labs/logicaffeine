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

use crate::analysis::{FieldType, LogosType, TypeDef, TypeRegistry};
use crate::intern::{Interner, Symbol};

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
    Rational,
    Duration,
    Date,
    Moment,
    Time,
    Span,
    /// A DOM element handle. Nominal: distinct from `Int` and from `Event`, even though
    /// every tier represents it as an i64 handle at runtime.
    Element,
    /// A DOM event handle (the argument of an event handler). Nominal, like `Element`.
    Event,
    /// A view value (a template with its captured bindings). Nominal.
    View,
    Seq(Box<InferType>),
    Map(Box<InferType>, Box<InferType>),
    Set(Box<InferType>),
    Option(Box<InferType>),
    /// A user struct or enum, applied to its type arguments (`Box of Int` is `UserDefined(Box,
    /// [Int])`; a type with no parameters has none). An EMPTY argument list also stands for a bare
    /// reference to a generic type whose arguments were not written, and unifies with any
    /// application of it.
    UserDefined(Symbol, Vec<InferType>),
    // Inference-only types
    Var(TyVar),
    Function(Vec<InferType>, Box<InferType>),
    Unknown,
}

/// A type error detected during unification or checking.
#[derive(Debug, Clone)]
pub enum TypeError {
    Mismatch {
        expected: InferType,
        found: InferType,
    },
    InfiniteType {
        var: TyVar,
        ty: InferType,
    },
    ArityMismatch {
        expected: usize,
        found: usize,
    },
    FieldNotFound {
        type_name: Symbol,
        field_name: Symbol,
    },
    NotAFunction {
        found: InferType,
    },
    /// A call to a name that exists NOWHERE in the post-prelude program — not a
    /// user function, not a builtin, not a bound variable, not a variant
    /// constructor. Caught at check time by
    /// [`crate::analysis::known_calls::check_unknown_calls`], with the same
    /// message every runtime tier uses (`Unknown function: <name>`), or a
    /// retired builtin's own refusal
    /// ([`crate::semantics::builtins::retired_builtin`]).
    UnknownFunction {
        name: Symbol,
    },
    /// Dimension-incoherent quantity arithmetic (`2 meters + 1 gram`) caught statically by the
    /// [`crate::analysis::dimension_check::DimensionChecker`]. Carries its own ready message.
    DimensionMismatch {
        message: std::string::String,
    },
}

impl TypeError {
    pub fn expected_str(&self) -> std::string::String {
        match self {
            TypeError::Mismatch { expected, .. } => expected.to_logos_name(),
            TypeError::ArityMismatch { expected, .. } => format!("{} arguments", expected),
            TypeError::FieldNotFound { .. } => "a known field".to_string(),
            TypeError::NotAFunction { .. } => "a function".to_string(),
            TypeError::UnknownFunction { .. } => "a defined function".to_string(),
            TypeError::InfiniteType { .. } => "a finite type".to_string(),
            TypeError::DimensionMismatch { .. } => "a matching dimension".to_string(),
        }
    }

    pub fn found_str(&self) -> std::string::String {
        match self {
            TypeError::Mismatch { found, .. } => found.to_logos_name(),
            TypeError::ArityMismatch { found, .. } => format!("{} arguments", found),
            TypeError::FieldNotFound { field_name, .. } => format!("{:?}", field_name),
            TypeError::NotAFunction { found } => found.to_logos_name(),
            TypeError::UnknownFunction { name } => format!("{:?}", name),
            TypeError::InfiniteType { ty, .. } => ty.to_logos_name(),
            TypeError::DimensionMismatch { message } => message.clone(),
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
                expected: expected.describe(interner),
                found: found.describe(interner),
                context: String::new(),
            },
            TypeError::InfiniteType { var, ty } => ParseErrorKind::InfiniteType {
                var_description: format!("type variable α{}", var.0),
                type_description: ty.describe(interner),
            },
            TypeError::ArityMismatch { expected, found } => ParseErrorKind::ArityMismatch {
                function: String::from("function"),
                expected: *expected,
                found: *found,
            },
            TypeError::FieldNotFound {
                type_name,
                field_name,
            } => ParseErrorKind::FieldNotFound {
                type_name: interner.resolve(*type_name).to_string(),
                field_name: interner.resolve(*field_name).to_string(),
                available: vec![],
            },
            TypeError::NotAFunction { found } => ParseErrorKind::NotAFunction {
                found_type: found.describe(interner),
            },
            TypeError::UnknownFunction { name } => {
                let name = interner.resolve(*name);
                ParseErrorKind::Custom(match crate::semantics::builtins::retired_builtin(name) {
                    Some(refusal) => refusal.to_string(),
                    None => format!("Unknown function: {}", name),
                })
            }
            TypeError::DimensionMismatch { message } => ParseErrorKind::Custom(message.clone()),
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

/// Where the names in a written type are looked up while converting it: the program's registry
/// (so a user struct or enum is a `UserDefined`, not `Unknown`) and the type parameters in scope,
/// each standing for the type it is currently bound to.
pub struct TypeScope<'s> {
    pub interner: &'s Interner,
    pub registry: Option<&'s TypeRegistry>,
    pub params: &'s HashMap<Symbol, InferType>,
    /// Each bare type name that IS a namespaced type (`Point` for `Geometry::Point`), so both
    /// spellings read as one type (see `TypeRegistry::namespaced_twins`).
    pub twins: Option<&'s HashMap<Symbol, Symbol>>,
}

impl TypeScope<'_> {
    /// The one identity a user type named `name` has: its namespaced twin when the bare spelling
    /// names one, else itself.
    pub fn canonical(&self, name: Symbol) -> Symbol {
        self.twins
            .and_then(|t| t.get(&name))
            .copied()
            .unwrap_or(name)
    }
}

/// Union-Find table implementing Robinson unification with occurs check.
///
/// Type variables are allocated by `fresh` and resolved by `find`.
/// `zonk` fully resolves a type after inference, converting remaining
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
        self.instantiate_with(scheme, &[]).0
    }

    /// [`Self::instantiate`], also carrying `also` through the same fresh variables — so a
    /// function's declared type arguments come out as what THIS instantiation solves them to.
    pub fn instantiate_with(
        &mut self,
        scheme: &TypeScheme,
        also: &[InferType],
    ) -> (InferType, Vec<InferType>) {
        if scheme.vars.is_empty() {
            return (scheme.body.clone(), also.to_vec());
        }
        let subst: HashMap<TyVar, TyVar> = scheme
            .vars
            .iter()
            .map(|&old_tv| (old_tv, self.fresh_var()))
            .collect();
        let body = self.substitute_vars(&scheme.body, &subst);
        let also = also
            .iter()
            .map(|t| self.substitute_vars(t, &subst))
            .collect();
        (body, also)
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
            InferType::Option(inner) => {
                InferType::Option(Box::new(self.substitute_vars(inner, subst)))
            }
            InferType::UserDefined(sym, args) => InferType::UserDefined(
                *sym,
                args.iter()
                    .map(|a| self.substitute_vars(a, subst))
                    .collect(),
            ),
            InferType::Function(params, ret) => InferType::Function(
                params
                    .iter()
                    .map(|p| self.substitute_vars(p, subst))
                    .collect(),
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
    /// Unlike `zonk`, this does not convert unbound variables to `Unknown`.
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
            InferType::UserDefined(sym, args) => {
                InferType::UserDefined(*sym, args.iter().map(|a| self.resolve(a)).collect())
            }
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
    /// `LogosType::Unknown` when converted by `to_logos_type`.
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
            InferType::Map(k, v) => InferType::Map(Box::new(self.zonk(k)), Box::new(self.zonk(v))),
            InferType::Set(inner) => InferType::Set(Box::new(self.zonk(inner))),
            InferType::Option(inner) => InferType::Option(Box::new(self.zonk(inner))),
            InferType::UserDefined(sym, args) => {
                InferType::UserDefined(*sym, args.iter().map(|a| self.zonk(a)).collect())
            }
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

    /// Unify what was written (`found`) with what its position promises (`expected`) — a
    /// return type, a parameter, a field, an annotation, an element type — binding type
    /// variables as needed. The order only names the sides of a mismatch, and every caller
    /// passes the value's type first.
    ///
    /// Returns `Ok(())` if unification succeeds (bindings may be updated).
    /// Returns `Err(TypeError)` on a genuine type contradiction.
    pub fn unify(&mut self, found: &InferType, expected: &InferType) -> Result<(), TypeError> {
        let found = self.walk(found);
        let expected = self.walk(expected);
        self.unify_walked(&found, &expected)
    }

    fn unify_walked(&mut self, a: &InferType, b: &InferType) -> Result<(), TypeError> {
        match (a, b) {
            // Unknown unifies with anything (propagated uncertainty) — and binds nothing: a
            // variable joined with `Unknown` has learned nothing, and stays open for the
            // constraints that do know its type.
            (InferType::Unknown, _) | (_, InferType::Unknown) => Ok(()),

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
            // The DOM handle types are nominal: each unifies only with itself — never with
            // `Int` (the runtime representation is not the type) and never with each other.
            (InferType::Element, InferType::Element) => Ok(()),
            (InferType::Event, InferType::Event) => Ok(()),
            (InferType::View, InferType::View) => Ok(()),

            // Nat embeds into Int for numeric contexts
            (InferType::Nat, InferType::Int) | (InferType::Int, InferType::Nat) => Ok(()),

            // Byte ↔ Int promotion (numeric literals infer as Int, adapt to Byte in context)
            (InferType::Byte, InferType::Int) | (InferType::Int, InferType::Byte) => Ok(()),

            // Rational equality + integer embedding: an integer IS an exact rational, so a
            // `Let x: Rational be 5` (or `… be 7 / 2`, whose `ExactDivide` infers numeric)
            // unifies against the `Rational` annotation. Mirrors Nat/Byte embedding into Int.
            (InferType::Rational, InferType::Rational) => Ok(()),
            (InferType::Rational, InferType::Int | InferType::Nat)
            | (InferType::Int | InferType::Nat, InferType::Rational) => Ok(()),

            // User-defined types unify when they name the same type and their arguments unify. A
            // bare reference (no arguments) unifies with every application of its type.
            (InferType::UserDefined(a, a_args), InferType::UserDefined(b, b_args)) if a == b => {
                if a_args.is_empty() || b_args.is_empty() {
                    return Ok(());
                }
                if a_args.len() != b_args.len() {
                    return Err(TypeError::ArityMismatch {
                        expected: a_args.len(),
                        found: b_args.len(),
                    });
                }
                let (a_args, b_args) = (a_args.clone(), b_args.clone());
                for (x, y) in a_args.iter().zip(b_args.iter()) {
                    self.unify(x, y)?;
                }
                Ok(())
            }

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

            // Type mismatch: `a` is the value's type, `b` its position's (see `unify`).
            (a, b) => Err(TypeError::Mismatch {
                expected: b.clone(),
                found: a.clone(),
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
                            Err(TypeError::InfiniteType {
                                var: tv,
                                ty: ty.clone(),
                            })
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
            InferType::UserDefined(_, args) => {
                for a in args {
                    self.occurs_check(tv, a)?;
                }
                Ok(())
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
        let params = type_params
            .iter()
            .map(|(s, tv)| (*s, InferType::Var(*tv)))
            .collect();
        Self::from_type_expr_in(
            ty,
            &TypeScope {
                interner,
                registry: None,
                params: &params,
                twins: None,
            },
        )
    }

    /// Convert a `TypeExpr` to `InferType` inside `scope`: a type parameter becomes the type it
    /// stands for, and a user struct or enum becomes `UserDefined` applied to its arguments.
    pub fn from_type_expr_in(ty: &crate::ast::stmt::TypeExpr, scope: &TypeScope) -> InferType {
        use crate::ast::stmt::TypeExpr;
        match ty {
            // A `mutable` parameter unifies as its underlying type.
            TypeExpr::Mutable { inner } => Self::from_type_expr_in(inner, scope),
            TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                Self::from_applied_name(*sym, None, scope)
            }
            TypeExpr::Generic { base, params } => {
                let args = params
                    .iter()
                    .map(|p| Self::from_type_expr_in(p, scope))
                    .collect();
                Self::from_applied_name(*base, Some(args), scope)
            }
            // A tuple/record type — inference treats it structurally-opaquely (like `Pair`/`Triple`, which
            // fall to `Unknown` above). The VM boundary + codegen carry the real per-position types.
            TypeExpr::Tuple(_) => InferType::Unknown,
            TypeExpr::Record { .. } => InferType::Unknown,
            TypeExpr::Refinement { base, .. } => Self::from_type_expr_in(base, scope),
            TypeExpr::Persistent { inner } => Self::from_type_expr_in(inner, scope),
            TypeExpr::Function { inputs, output } => {
                let param_types: Vec<InferType> = inputs
                    .iter()
                    .map(|p| Self::from_type_expr_in(p, scope))
                    .collect();
                let ret_type = Self::from_type_expr_in(output, scope);
                InferType::Function(param_types, Box::new(ret_type))
            }
        }
    }

    /// The type a written name denotes, bare (`applied` is `None`) or applied to type arguments:
    /// a type parameter in scope, a built-in collection or `Option`, a primitive, or a registered
    /// user struct/enum (through any alias). `Option` is always the built-in, whatever the prelude
    /// declares; a user's own `Maybe` is theirs, and only an undeclared `Maybe` is `Option`. A
    /// bare collection name names no element type (`Unknown`); an applied one missing an argument
    /// keeps its historical default.
    fn from_applied_name(
        base: Symbol,
        applied: Option<Vec<InferType>>,
        scope: &TypeScope,
    ) -> InferType {
        if applied.as_ref().map_or(true, |args| args.is_empty()) {
            if let Some(param) = scope.params.get(&base) {
                return param.clone();
            }
        }
        let name = scope.interner.resolve(base);
        let user = scope.registry.and_then(|r| r.get(base));
        let declared_by_user = matches!(user, Some(TypeDef::Struct { .. } | TypeDef::Enum { .. }));
        let is_option = name == "Option" || (name == "Maybe" && !declared_by_user);
        let Some(args) = applied else {
            if is_option {
                return InferType::Option(Box::new(InferType::Unknown));
            }
            let primitive = Self::from_type_name(name);
            if primitive != InferType::Unknown {
                return primitive;
            }
            return match user {
                Some(TypeDef::Struct { .. } | TypeDef::Enum { .. }) => {
                    InferType::UserDefined(scope.canonical(base), Vec::new())
                }
                Some(TypeDef::Alias { target }) if *target != base => {
                    Self::from_applied_name(*target, None, scope)
                }
                _ => InferType::Unknown,
            };
        };
        let mut args = args.into_iter();
        if is_option {
            return InferType::Option(Box::new(args.next().unwrap_or(InferType::Unit)));
        }
        match name {
            "Seq" | "List" | "Vec" => {
                return InferType::Seq(Box::new(args.next().unwrap_or(InferType::Unit)))
            }
            "Set" | "HashSet" => {
                return InferType::Set(Box::new(args.next().unwrap_or(InferType::Unit)))
            }
            "Map" | "HashMap" => {
                let k = args.next().unwrap_or(InferType::String);
                let v = args.next().unwrap_or(InferType::String);
                return InferType::Map(Box::new(k), Box::new(v));
            }
            _ => {}
        }
        let args: Vec<InferType> = args.collect();
        if args.is_empty() {
            let primitive = Self::from_type_name(name);
            if primitive != InferType::Unknown {
                return primitive;
            }
        }
        match user {
            Some(TypeDef::Struct { .. } | TypeDef::Enum { .. }) => {
                InferType::UserDefined(scope.canonical(base), args)
            }
            Some(TypeDef::Alias { target }) if *target != base => {
                Self::from_applied_name(*target, Some(args), scope)
            }
            _ => InferType::Unknown,
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
        let params = type_params
            .iter()
            .map(|(s, tv)| (*s, InferType::Var(*tv)))
            .collect();
        Self::from_field_type_in(
            ty,
            &TypeScope {
                interner,
                registry: None,
                params: &params,
                twins: None,
            },
        )
    }

    /// Convert a registry `FieldType` to `InferType` inside `scope` — how a generic record's field
    /// is read at one application of the record (`value: T` of a `Box of Int` is `Int`).
    pub fn from_field_type_in(ty: &FieldType, scope: &TypeScope) -> InferType {
        match ty {
            FieldType::Primitive(sym) => InferType::from_type_name(scope.interner.resolve(*sym)),
            FieldType::Named(sym) => {
                if let Some(param) = scope.params.get(sym) {
                    return param.clone();
                }
                // A field of one of the exact extension's types is typed as a binding declared one is.
                if crate::analysis::tower::TOWER_TYPES.contains(&scope.interner.resolve(*sym)) {
                    return InferType::Unknown;
                }
                match Self::from_applied_name(*sym, None, scope) {
                    InferType::Unknown => InferType::UserDefined(scope.canonical(*sym), Vec::new()),
                    known => known,
                }
            }
            FieldType::Generic { base, params } => {
                let args = params
                    .iter()
                    .map(|p| Self::from_field_type_in(p, scope))
                    .collect();
                Self::from_applied_name(*base, Some(args), scope)
            }
            FieldType::TypeParam(sym) => {
                scope.params.get(sym).cloned().unwrap_or(InferType::Unknown)
            }
            // A tuple is structurally opaque to inference, exactly as a written `(A, B)` is.
            FieldType::Tuple(_) | FieldType::Record(_) => InferType::Unknown,
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
            "Rational" => InferType::Rational,
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
            "Element" => InferType::Element,
            "Event" => InferType::Event,
            "View" => InferType::View,
            _ => InferType::Unknown,
        }
    }

    /// Human-readable type name in Logos terms, for error messages: a user type by its own name
    /// and arguments (`Outcome`, `Box of Int`).
    pub fn describe(&self, interner: &crate::intern::Interner) -> std::string::String {
        self.render(Some(interner))
    }

    /// [`describe`](Self::describe) where no interner is at hand: a user type reads as "a
    /// user-defined type".
    pub fn to_logos_name(&self) -> std::string::String {
        self.render(None)
    }

    /// The name a program writes for this type when it is one of the language's own scalars
    /// (`Int`, `Real`, `Text`, …); `None` for a collection, a user type, a function or a variable.
    pub fn primitive_name(&self) -> Option<&'static str> {
        Some(match self {
            InferType::Int => "Int",
            InferType::Float => "Real",
            InferType::Bool => "Bool",
            InferType::Char => "Char",
            InferType::Byte => "Byte",
            InferType::String => "Text",
            InferType::Unit => "Unit",
            InferType::Nat => "Nat",
            InferType::Rational => "Rational",
            InferType::Duration => "Duration",
            InferType::Date => "Date",
            InferType::Moment => "Moment",
            InferType::Time => "Time",
            InferType::Span => "Span",
            InferType::Element => "Element",
            InferType::Event => "Event",
            InferType::View => "View",
            _ => return None,
        })
    }

    fn render(&self, interner: Option<&crate::intern::Interner>) -> std::string::String {
        match self {
            InferType::Int => "Int".into(),
            InferType::Float => "Real".into(),
            InferType::Bool => "Bool".into(),
            InferType::Char => "Char".into(),
            InferType::Byte => "Byte".into(),
            InferType::String => "Text".into(),
            InferType::Unit => "Unit".into(),
            InferType::Nat => "Nat".into(),
            InferType::Rational => "Rational".into(),
            InferType::Duration => "Duration".into(),
            InferType::Date => "Date".into(),
            InferType::Moment => "Moment".into(),
            InferType::Time => "Time".into(),
            InferType::Span => "Span".into(),
            InferType::Element => "Element".into(),
            InferType::Event => "Event".into(),
            InferType::View => "View".into(),
            InferType::Seq(inner) => format!("Seq of {}", inner.render(interner)),
            InferType::Map(k, v) => {
                format!("Map of {} and {}", k.render(interner), v.render(interner))
            }
            InferType::Set(inner) => format!("Set of {}", inner.render(interner)),
            InferType::Option(inner) => format!("Option of {}", inner.render(interner)),
            InferType::UserDefined(name, args) => match interner {
                Some(interner) if args.is_empty() => interner.resolve(*name).to_string(),
                Some(interner) => {
                    let args: Vec<std::string::String> =
                        args.iter().map(|a| a.render(Some(interner))).collect();
                    format!("{} of {}", interner.resolve(*name), args.join(" and "))
                }
                None => "a user-defined type".into(),
            },
            InferType::Var(_) => "an unknown type".into(),
            InferType::Function(params, ret) => {
                let params_str = params
                    .iter()
                    .map(|p| p.render(interner))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("fn({}) -> {}", params_str, ret.render(interner))
            }
            InferType::Unknown => "unknown".into(),
        }
    }

    /// Convert a known-ground `InferType` to `LogosType`.
    ///
    /// # Panics
    ///
    /// Panics if called on a `Var`. Callers must `zonk` first.
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
            InferType::Rational => LogosType::Rational,
            InferType::Duration => LogosType::Duration,
            InferType::Date => LogosType::Date,
            InferType::Moment => LogosType::Moment,
            InferType::Time => LogosType::Time,
            InferType::Span => LogosType::Span,
            InferType::Element => LogosType::Element,
            InferType::Event => LogosType::Event,
            InferType::View => LogosType::View,
            InferType::Seq(inner) => LogosType::Seq(Box::new(inner.to_logos_type_ground())),
            InferType::Map(k, v) => LogosType::Map(
                Box::new(k.to_logos_type_ground()),
                Box::new(v.to_logos_type_ground()),
            ),
            InferType::Set(inner) => LogosType::Set(Box::new(inner.to_logos_type_ground())),
            InferType::Option(inner) => LogosType::Option(Box::new(inner.to_logos_type_ground())),
            InferType::UserDefined(sym, _) => LogosType::UserDefined(*sym),
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

/// Numeric promotion: Float wins; Int + Int = Int; Nat + Nat = Nat; Byte + Byte = Byte;
/// Byte + Int (literal context) = Byte; otherwise error.
pub fn unify_numeric(a: &InferType, b: &InferType) -> Result<InferType, TypeError> {
    match (a, b) {
        (InferType::Float, _) | (_, InferType::Float) => Ok(InferType::Float),
        // A Rational operand promotes the result to Rational (the exact type wins over Int).
        (InferType::Rational, InferType::Rational | InferType::Int | InferType::Nat)
        | (InferType::Int | InferType::Nat, InferType::Rational) => Ok(InferType::Rational),
        (InferType::Int, InferType::Int) => Ok(InferType::Int),
        (InferType::Nat, InferType::Int) | (InferType::Int, InferType::Nat) => Ok(InferType::Int),
        (InferType::Nat, InferType::Nat) => Ok(InferType::Nat),
        (InferType::Byte, InferType::Byte) => Ok(InferType::Byte),
        (InferType::Byte, InferType::Int) | (InferType::Int, InferType::Byte) => {
            Ok(InferType::Byte)
        }
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
        InferType::Rational => LogosType::Rational,
        InferType::Duration => LogosType::Duration,
        InferType::Date => LogosType::Date,
        InferType::Moment => LogosType::Moment,
        InferType::Time => LogosType::Time,
        InferType::Span => LogosType::Span,
        InferType::Element => LogosType::Element,
        InferType::Event => LogosType::Event,
        InferType::View => LogosType::View,
        InferType::Seq(inner) => LogosType::Seq(Box::new(infer_to_logos(inner))),
        InferType::Map(k, v) => {
            LogosType::Map(Box::new(infer_to_logos(k)), Box::new(infer_to_logos(v)))
        }
        InferType::Set(inner) => LogosType::Set(Box::new(infer_to_logos(inner))),
        InferType::Option(inner) => LogosType::Option(Box::new(infer_to_logos(inner))),
        InferType::UserDefined(sym, _) => LogosType::UserDefined(*sym),
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/unify/tests.rs"]
mod tests;
