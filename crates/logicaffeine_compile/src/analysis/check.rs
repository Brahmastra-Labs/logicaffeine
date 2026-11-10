//! Bidirectional type checker for the LOGOS compilation pipeline.
//!
//! Replaces `TypeEnv::infer_program()` with a proper constraint-solving pass
//! that eliminates `Unknown` for field access, empty collections, option literals,
//! pipe receives, inspect arm bindings, and closure call expressions.
//!
//! # Architecture
//!
//! ```text
//! AST
//!  │
//!  ├── preregister_functions   ← forward-reference pre-pass
//!  │
//!  └── infer_stmt / infer_expr ← bidirectional checking
//!           │
//!           └── UnificationTable ← Robinson unification (from unify.rs)
//!                    │
//!                    └── zonk → TypeEnv (LogosType) → codegen
//! ```

use std::collections::HashMap;

use crate::analysis::unify::{
    unify_numeric, InferType, TyVar, TypeError, TypeScheme, TypeScope, UnificationTable,
};
use crate::analysis::{FieldType, FnSig, LogosType, TypeDef, TypeEnv, TypeRegistry};
use crate::ast::stmt::{BinaryOpKind, Expr, Pattern, Stmt, StmtId};
use crate::intern::{Interner, Symbol};

// ============================================================================
// Data structures
// ============================================================================

/// A registered function's signature, supporting both monomorphic and generic functions.
///
/// For generic functions (non-empty `scheme.vars`), each call site must instantiate
/// the scheme to get fresh type variables, preventing cross-call contamination.
/// For monomorphic functions (`scheme.vars` is empty), `scheme.body` is the direct type.
#[derive(Clone, Debug)]
struct FunctionRecord {
    /// Parameter names (for binding in the function scope).
    param_names: Vec<Symbol>,
    is_native: bool,
    /// The quantified type scheme: `forall vars. Function(param_types, return_type)`.
    /// For monomorphic functions, `vars` is empty and body is used directly.
    scheme: TypeScheme,
    /// The DECLARED type parameters in declaration order, each as the type its variable resolves
    /// to — instantiated with the scheme, they are the type arguments one call is solved for.
    declared: Vec<InferType>,
}

/// A node of the checked program, by address. The checker and a later pass over the SAME
/// statements agree on it without the AST carrying ids.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Site {
    Expr(usize),
    Stmt(usize),
}

impl Site {
    pub fn of_expr(e: &Expr) -> Self {
        Site::Expr(e as *const Expr as *const () as usize)
    }

    pub fn of_stmt(s: &Stmt) -> Self {
        Site::Stmt(s as *const Stmt as *const () as usize)
    }
}

/// One call of a generic function, as the checker solved it.
#[derive(Debug, Clone)]
pub struct CallInstance {
    /// The declared type arguments, in declaration order.
    pub type_args: Vec<InferType>,
    /// Every argument's type was fully known. When one was not, a type argument the call leaves
    /// open may be one the checker failed to see, not one that nothing constrains.
    pub precise: bool,
}

/// What the checker solved at every generic site of a program — the input to monomorphization.
///
/// Types are RESOLVED, not zonked: a site inside a generic function's body still names that
/// function's own type variables (listed in `type_params`), to be substituted once per instance.
/// A variable nothing constrained stays a variable.
#[derive(Debug, Default)]
pub struct Instantiations {
    /// Each call of a generic function.
    pub calls: HashMap<Site, CallInstance>,
    /// Each `New` / `NewVariant` and `some` / `none`: the type it builds.
    pub constructed: HashMap<Site, InferType>,
    /// Each empty sequence literal (`[]`): the sequence type its context gives it.
    pub empty_lists: HashMap<Site, InferType>,
    /// Each `Inspect`: the type of the value it inspects.
    pub scrutinees: HashMap<Site, InferType>,
    /// Each generic function's type parameters in declaration order, each as the type its
    /// variable resolved to (its own unbound variable, unless the body pinned it).
    pub type_params: HashMap<Symbol, Vec<(Symbol, InferType)>>,
}

/// Bidirectional type checking environment.
///
/// Scopes are pushed/popped around function bodies and match arms.
/// All bindings are also written to `all_vars` for later `TypeEnv` output.
struct CheckEnv<'r> {
    /// Stacked scopes (innermost last). Variables resolved from inner-to-outer.
    scopes: Vec<HashMap<Symbol, InferType>>,
    /// Flat map of every variable ever bound — accumulated for `TypeEnv` output.
    all_vars: HashMap<Symbol, InferType>,
    /// Registered function signatures.
    functions: HashMap<Symbol, FunctionRecord>,
    /// Expected return type inside the current function body.
    current_return_type: Option<InferType>,
    /// Unification table for type variables.
    table: UnificationTable,
    registry: &'r TypeRegistry,
    interner: &'r Interner,
    /// The type parameters of the generic function whose body is being checked, each standing
    /// for its variable — so a `T` written in the body (`a new Seq of T`) means that variable.
    type_params_in_scope: HashMap<Symbol, InferType>,
    /// Every generic site solved so far, unresolved until the program is checked.
    sites: Instantiations,
    /// Each generic call's argument types, to judge its precision once the program is checked.
    call_arg_types: HashMap<Site, Vec<InferType>>,
    /// Each bare type name that IS a namespaced type, so both spellings are one type.
    twins: HashMap<Symbol, Symbol>,
}

impl<'r> CheckEnv<'r> {
    fn new(registry: &'r TypeRegistry, interner: &'r Interner) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            all_vars: HashMap::new(),
            functions: HashMap::new(),
            current_return_type: None,
            table: UnificationTable::new(),
            registry,
            interner,
            type_params_in_scope: HashMap::new(),
            sites: Instantiations::default(),
            call_arg_types: HashMap::new(),
            twins: registry.namespaced_twins(interner),
        }
    }

    /// The one identity a user type named `name` has (see `TypeScope::canonical`).
    fn canonical(&self, name: Symbol) -> Symbol {
        self.twins.get(&name).copied().unwrap_or(name)
    }

    /// A written type, read with the registry and the type parameters in scope.
    fn written_type(&self, ty: &crate::ast::stmt::TypeExpr) -> InferType {
        self.written_type_with(ty, &self.type_params_in_scope)
    }

    fn written_type_with(
        &self,
        ty: &crate::ast::stmt::TypeExpr,
        params: &HashMap<Symbol, InferType>,
    ) -> InferType {
        InferType::from_type_expr_in(
            ty,
            &TypeScope {
                interner: self.interner,
                registry: Some(self.registry),
                params,
                twins: Some(&self.twins),
            },
        )
    }

    /// A field's declared type at one application of its record or enum, `args` bound to the
    /// type's own parameters in order.
    fn field_type_at(&self, ty: &FieldType, generics: &[Symbol], args: &[InferType]) -> InferType {
        let params: HashMap<Symbol, InferType> =
            generics.iter().copied().zip(args.iter().cloned()).collect();
        InferType::from_field_type_in(
            ty,
            &TypeScope {
                interner: self.interner,
                registry: Some(self.registry),
                params: &params,
                twins: Some(&self.twins),
            },
        )
    }

    /// The type arguments one construction of `type_sym` is solved for: the written ones, or a
    /// fresh variable per type parameter when none were written.
    fn applied_args(
        &mut self,
        type_sym: Symbol,
        written: &[crate::ast::stmt::TypeExpr],
    ) -> (Vec<Symbol>, Vec<InferType>) {
        let generics = match self.registry.get(type_sym) {
            Some(TypeDef::Struct { generics, .. } | TypeDef::Enum { generics, .. }) => {
                generics.clone()
            }
            _ => Vec::new(),
        };
        let args = if written.len() == generics.len() && !written.is_empty() {
            written.iter().map(|t| self.written_type(t)).collect()
        } else {
            generics.iter().map(|_| self.table.fresh()).collect()
        };
        (generics, args)
    }

    /// Every generic site with its types resolved as far as the whole program determines them.
    fn into_instantiations(self) -> Instantiations {
        fn mentions_unknown(t: &InferType) -> bool {
            match t {
                InferType::Unknown => true,
                InferType::Seq(i) | InferType::Set(i) | InferType::Option(i) => mentions_unknown(i),
                InferType::Map(k, v) => mentions_unknown(k) || mentions_unknown(v),
                InferType::UserDefined(_, args) => args.iter().any(mentions_unknown),
                InferType::Function(ps, r) => {
                    ps.iter().any(mentions_unknown) || mentions_unknown(r)
                }
                _ => false,
            }
        }
        let table = &self.table;
        let sites = self.sites;
        let arg_types = self.call_arg_types;
        Instantiations {
            calls: sites
                .calls
                .into_iter()
                .map(|(k, v)| {
                    let precise = arg_types.get(&k).map_or(true, |ts| {
                        ts.iter().all(|t| !mentions_unknown(&table.resolve(t)))
                    });
                    let type_args = v.type_args.iter().map(|t| table.resolve(t)).collect();
                    (k, CallInstance { type_args, precise })
                })
                .collect(),
            constructed: sites
                .constructed
                .into_iter()
                .map(|(k, v)| (k, table.resolve(&v)))
                .collect(),
            empty_lists: sites
                .empty_lists
                .into_iter()
                .map(|(k, v)| (k, table.resolve(&v)))
                .collect(),
            scrutinees: sites
                .scrutinees
                .into_iter()
                .map(|(k, v)| (k, table.resolve(&v)))
                .collect(),
            type_params: sites
                .type_params
                .into_iter()
                .map(|(f, ps)| {
                    (
                        f,
                        ps.into_iter()
                            .map(|(p, t)| (p, table.resolve(&t)))
                            .collect(),
                    )
                })
                .collect(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Bind a variable in the current scope, also recording in `all_vars`.
    fn bind_var(&mut self, sym: Symbol, ty: InferType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(sym, ty.clone());
        }
        self.all_vars.insert(sym, ty);
    }

    /// Look up a variable, searching scopes from innermost to outermost.
    ///
    /// Uses `resolve` (not `zonk`) so that unbound type variables from generic
    /// function parameters remain as `Var(tv)` during inference, enabling
    /// proper unification at call sites.
    fn lookup_var(&self, sym: Symbol) -> Option<InferType> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(&sym) {
                return Some(self.table.resolve(ty));
            }
        }
        None
    }

    /// Convert the check environment into a `TypeEnv` for codegen.
    fn into_type_env(self) -> TypeEnv {
        let mut type_env = TypeEnv::new();

        // Collect all variable bindings, zonk each to a concrete LogosType
        for (sym, ty) in self.all_vars {
            let logos_ty = self.table.to_logos_type(&ty);
            type_env.register(sym, logos_ty);
        }

        // Collect function signatures — instantiate monomorphic view for codegen
        for (name, rec) in self.functions {
            // Zonk the scheme body to extract concrete param/return types for TypeEnv.
            // For generic functions, unsolved vars zonk to Unknown, which is fine
            // since codegen uses TypeExpr (not TypeEnv) for generic param types.
            if let InferType::Function(param_types, ret_box) = &rec.scheme.body {
                let ret_logos = self.table.to_logos_type(ret_box);
                let params: Vec<(Symbol, LogosType)> = rec
                    .param_names
                    .iter()
                    .zip(param_types.iter())
                    .map(|(sym, ty)| (*sym, self.table.to_logos_type(ty)))
                    .collect();
                type_env.register_fn(
                    name,
                    FnSig {
                        params,
                        return_type: ret_logos,
                    },
                );
                type_env.set_native_fn(name, rec.is_native);
            }
        }

        type_env
    }
}

// ============================================================================
// Pre-pass: forward reference registration
// ============================================================================

impl<'r> CheckEnv<'r> {
    /// Register all top-level function signatures before the main checking pass.
    ///
    /// This enables forward references and mutual recursion: any function can
    /// call any other function regardless of declaration order.
    ///
    /// For generic functions (non-empty `generics`), allocates a fresh `TyVar` per
    /// type parameter and builds a `TypeScheme` so call sites can instantiate them.
    fn preregister_functions(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let Stmt::FunctionDef {
                name,
                generics,
                params,
                return_type,
                is_native,
                ..
            } = stmt
            {
                // Allocate one TyVar per generic type parameter
                let type_param_map: HashMap<Symbol, TyVar> = generics
                    .iter()
                    .map(|&sym| (sym, self.table.fresh_var()))
                    .collect();
                let in_scope: HashMap<Symbol, InferType> = type_param_map
                    .iter()
                    .map(|(s, tv)| (*s, InferType::Var(*tv)))
                    .collect();

                let param_types: Vec<InferType> = params
                    .iter()
                    .map(|(_, ty_expr)| self.written_type_with(ty_expr, &in_scope))
                    .collect();
                let param_names: Vec<Symbol> = params.iter().map(|(sym, _)| *sym).collect();

                let ret_type = if let Some(rt) = return_type {
                    self.written_type_with(rt, &in_scope)
                } else {
                    self.table.fresh()
                };

                let generic_vars: Vec<TyVar> = generics
                    .iter()
                    .filter_map(|sym| type_param_map.get(sym).copied())
                    .collect();
                let declared = generic_vars.iter().map(|tv| InferType::Var(*tv)).collect();

                let scheme = TypeScheme {
                    vars: generic_vars,
                    body: InferType::Function(param_types, Box::new(ret_type)),
                };

                self.functions.insert(
                    *name,
                    FunctionRecord {
                        param_names,
                        scheme,
                        declared,
                        is_native: *is_native,
                    },
                );
            }
        }
    }
}

// ============================================================================
// Core inference
// ============================================================================

impl<'r> CheckEnv<'r> {
    /// Check an expression against an expected type (checking mode).
    ///
    /// Handles numeric literal coercion (`5` against `Real` → `Float`) and
    /// structural checking before falling through to synthesis + unification.
    fn check_expr(&mut self, expr: &Expr, expected: &InferType) -> Result<InferType, TypeError> {
        use crate::ast::stmt::Literal;

        // Number literals are polymorphic: 5 checks against Int, Float, Nat, or Byte
        if let Expr::Literal(Literal::Number(_)) = expr {
            match expected {
                InferType::Float => return Ok(InferType::Float),
                InferType::Nat => return Ok(InferType::Nat),
                InferType::Int => return Ok(InferType::Int),
                InferType::Byte => return Ok(InferType::Byte),
                _ => {}
            }
        }

        // `nothing` is polymorphic: it is `None` when checked against Option(T),
        // and the unit value `()` in all other contexts.
        if let Expr::Literal(Literal::Nothing) = expr {
            if let InferType::Option(_) = expected {
                return Ok(expected.clone());
            }
        }

        // List literals: check EACH element against the expected element type so
        // the numeric-literal coercion applies element-wise (e.g. `[1, 2, 3]`
        // under a `Seq of Real` annotation), instead of synthesizing the element
        // type from items[0] alone and failing to unify with the annotation.
        if let Expr::List(items) = expr {
            if let InferType::Seq(elem) = self.table.resolve(expected) {
                for item in items {
                    self.check_expr(item, &elem)?;
                }
                return Ok(InferType::Seq(elem));
            }
        }

        // Default: synthesize then unify
        let inferred = self.infer_expr(expr)?;
        self.table.unify(&inferred, expected)?;
        Ok(self.table.resolve(expected))
    }

    /// Infer the type of an expression (synthesis mode).
    fn infer_expr(&mut self, expr: &Expr) -> Result<InferType, TypeError> {
        match expr {
            Expr::Literal(lit) => Ok(InferType::from_literal(lit)),

            Expr::Identifier(sym) => Ok(match crate::semantics::temporal::ClockRead::named(self.interner.resolve(*sym)) {
                Some(crate::semantics::temporal::ClockRead::Today) => InferType::Date,
                Some(crate::semantics::temporal::ClockRead::Now) => InferType::Moment,
                None => self.lookup_var(*sym).unwrap_or(InferType::Unknown),
            }),

            Expr::BinaryOp { op, left, right } => self.infer_binary_op(*op, left, right),

            Expr::Length { collection } => {
                self.infer_expr(collection)?;
                Ok(InferType::Int)
            }

            Expr::Call { function, args } => self.infer_call(*function, args, Site::of_expr(expr)),

            Expr::Index { collection, index } => {
                let coll_ty = self.infer_expr(collection)?;
                self.infer_expr(index)?;
                let walked = self.table.resolve(&coll_ty);
                match walked {
                    InferType::Seq(inner) => Ok(*inner),
                    InferType::Map(_, v) => Ok(*v),
                    _ => Ok(InferType::Unknown),
                }
            }

            Expr::List(items) => {
                // Every element is inferred, not just the first: a later element may hold a
                // generic call or construction whose instantiation must be solved too. The first
                // element names the element type; the rest join it where they can.
                let elem_type = match items.split_first() {
                    None => self.table.fresh(),
                    Some((first, rest)) => {
                        let first_ty = self.infer_expr(first)?;
                        for item in rest {
                            let item_ty = self.infer_expr(item)?;
                            self.table.unify(&first_ty, &item_ty).ok();
                        }
                        first_ty
                    }
                };
                let ty = InferType::Seq(Box::new(elem_type));
                if items.is_empty() {
                    self.sites
                        .empty_lists
                        .insert(Site::of_expr(expr), ty.clone());
                }
                Ok(ty)
            }

            Expr::OptionSome { value } => {
                let inner = self.infer_expr(value)?;
                let ty = InferType::Option(Box::new(inner));
                self.sites
                    .constructed
                    .insert(Site::of_expr(expr), ty.clone());
                Ok(ty)
            }

            Expr::OptionNone => {
                let elem_var = self.table.fresh();
                let ty = InferType::Option(Box::new(elem_var));
                self.sites
                    .constructed
                    .insert(Site::of_expr(expr), ty.clone());
                Ok(ty)
            }

            Expr::Range { start, end } => {
                self.infer_expr(start)?;
                self.infer_expr(end)?;
                Ok(InferType::Seq(Box::new(InferType::Int)))
            }

            Expr::Contains { collection, value } => {
                self.infer_expr(collection)?;
                self.infer_expr(value)?;
                Ok(InferType::Bool)
            }

            Expr::Copy { expr: inner } | Expr::Give { value: inner } => self.infer_expr(inner),

            Expr::WithCapacity { value, capacity } => {
                self.infer_expr(capacity)?;
                self.infer_expr(value)
            }

            Expr::FieldAccess { object, field } => {
                let obj_ty = self.infer_expr(object)?;
                self.infer_field_access(obj_ty, *field)
            }

            Expr::New {
                type_name,
                type_args,
                init_fields,
            } => {
                let name = self.interner.resolve(*type_name);
                let ty = match name {
                    "Seq" | "List" | "Vec" => {
                        let elem = type_args
                            .first()
                            .map(|t| self.written_type(t))
                            .unwrap_or_else(|| self.table.fresh());
                        InferType::Seq(Box::new(elem))
                    }
                    "Map" | "HashMap" => {
                        let key = type_args
                            .first()
                            .map(|t| self.written_type(t))
                            .unwrap_or(InferType::String);
                        let val = type_args
                            .get(1)
                            .map(|t| self.written_type(t))
                            .unwrap_or(InferType::String);
                        InferType::Map(Box::new(key), Box::new(val))
                    }
                    "Set" | "HashSet" => {
                        let elem = type_args
                            .first()
                            .map(|t| self.written_type(t))
                            .unwrap_or_else(|| self.table.fresh());
                        InferType::Set(Box::new(elem))
                    }
                    _ => {
                        // A struct constructor's field initialisers were never checked: the arm
                        // matched `..` and returned the nominal type, so `a new P with x "text"`
                        // stored a Text in an Int field and `a new P with z 5` invented a field,
                        // both silently. Each initialiser is checked against the declared field,
                        // read at this construction's type arguments.
                        let (_, args) = self.applied_args(*type_name, type_args);
                        let struct_ty = InferType::UserDefined(self.canonical(*type_name), args);
                        for (field, value) in init_fields {
                            let value_ty = self.infer_expr(value)?;
                            // `infer_field_access` raises `FieldNotFound` for a name the struct
                            // does not declare, so an invented field is caught here too.
                            let field_ty = self.infer_field_access(struct_ty.clone(), *field)?;
                            self.table.unify(&value_ty, &field_ty)?;
                        }
                        struct_ty
                    }
                };
                self.sites
                    .constructed
                    .insert(Site::of_expr(expr), ty.clone());
                Ok(ty)
            }

            // A variant's payload is checked against its declared fields, read at this
            // construction's type arguments — which the payload itself solves: `a new Just with
            // value 42` is a `Maybe of Int`. A payload-free variant (`a new Nothing`) leaves them
            // open for the context to settle.
            Expr::NewVariant {
                enum_name,
                variant,
                fields,
            } => {
                let (generics, args) = self.applied_args(*enum_name, &[]);
                let declared: Vec<(Symbol, FieldType)> = match self.registry.get(*enum_name) {
                    Some(TypeDef::Enum { variants, .. }) => variants
                        .iter()
                        .find(|v| v.name == *variant)
                        .map(|v| v.fields.iter().map(|f| (f.name, f.ty.clone())).collect())
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                for (field, value) in fields {
                    match declared.iter().find(|(name, _)| name == field) {
                        Some((_, fty)) => {
                            let expected = self.field_type_at(fty, &generics, &args);
                            self.check_widening(value, &expected)?;
                        }
                        None => {
                            self.infer_expr(value)?;
                        }
                    }
                }
                let ty = InferType::UserDefined(self.canonical(*enum_name), args);
                self.sites
                    .constructed
                    .insert(Site::of_expr(expr), ty.clone());
                Ok(ty)
            }

            Expr::CallExpr { callee, args } => self.infer_call_expr(callee, args),

            Expr::Closure {
                params,
                body: closure_body,
                return_type,
            } => self.infer_closure(params, closure_body, return_type),

            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let crate::ast::stmt::StringPart::Expr { value, .. } = part {
                        self.infer_expr(value)?;
                    }
                }
                Ok(InferType::String)
            }

            Expr::Slice {
                collection,
                start,
                end,
            } => {
                self.infer_expr(start)?;
                self.infer_expr(end)?;
                self.infer_expr(collection)
            }

            Expr::Union { left, right } | Expr::Intersection { left, right } => {
                self.infer_expr(right)?;
                self.infer_expr(left)
            }

            // EVERY REMAINING EXPRESSION KIND, NAMED: none has a type this checker models, but
            // each one's operands are still inferred — a generic call inside a tuple or a negation
            // must be solved like any other.
            Expr::Not { operand } => {
                self.infer_expr(operand)?;
                Ok(InferType::Unknown)
            }
            Expr::Tuple(items) => {
                for item in items {
                    self.infer_expr(item)?;
                }
                Ok(InferType::Unknown)
            }
            Expr::ManifestOf { zone } => {
                self.infer_expr(zone)?;
                Ok(InferType::Unknown)
            }
            Expr::ChunkAt { index, zone } => {
                self.infer_expr(index)?;
                self.infer_expr(zone)?;
                Ok(InferType::Unknown)
            }
            Expr::Dom(query) => {
                if let Some(selector) = query.selector() {
                    self.infer_expr(selector)?;
                }
                Ok(InferType::Unknown)
            }
            Expr::Escape { .. } | Expr::View(_) => Ok(InferType::Unknown),
        }
    }

    /// Infer a binary operation's result type.
    fn infer_binary_op(
        &mut self,
        op: BinaryOpKind,
        left: &Expr,
        right: &Expr,
    ) -> Result<InferType, TypeError> {
        match op {
            BinaryOpKind::Eq
            | BinaryOpKind::NotEq
            | BinaryOpKind::Lt
            | BinaryOpKind::Gt
            | BinaryOpKind::LtEq
            | BinaryOpKind::GtEq
            | BinaryOpKind::ApproxEq => {
                self.infer_expr(left)?;
                self.infer_expr(right)?;
                Ok(InferType::Bool)
            }

            // And/Or: the logical words — every operand's truth in, a Bool out (`&`/`|` are the bitwise spellings).
            BinaryOpKind::And | BinaryOpKind::Or => {
                self.infer_expr(left)?;
                self.infer_expr(right)?;
                Ok(InferType::Bool)
            }

            BinaryOpKind::Concat => {
                self.infer_expr(left)?;
                self.infer_expr(right)?;
                Ok(InferType::String)
            }

            // `a followed by b` — result is a sequence of the same type as the operands.
            BinaryOpKind::SeqConcat => {
                self.infer_expr(right)?;
                self.infer_expr(left)
            }

            // `& | xor`: on two Bools a Bool, on Sets the left operand's Set, on integers an Int.
            BinaryOpKind::BitXor | BinaryOpKind::BitAnd | BinaryOpKind::BitOr => {
                let lt = self.infer_expr(left)?;
                let rt = self.infer_expr(right)?;
                Ok(match (&lt, &rt) {
                    (InferType::Bool, InferType::Bool) => InferType::Bool,
                    (InferType::Set(_), _) => lt,
                    _ => InferType::Int,
                })
            }

            BinaryOpKind::Shl | BinaryOpKind::Shr => {
                self.infer_expr(left)?;
                self.infer_expr(right)?;
                Ok(InferType::Int)
            }

            BinaryOpKind::Add => {
                let lt = self.infer_expr(left)?;
                let rt = self.infer_expr(right)?;
                if lt == InferType::String || rt == InferType::String {
                    Ok(InferType::String)
                } else if lt == InferType::Unknown || rt == InferType::Unknown {
                    Ok(InferType::Unknown)
                } else {
                    unify_numeric(&lt, &rt).or(Ok(InferType::Unknown))
                }
            }

            // Exact division always yields a Rational (`7 / 2 → 7/2`); the resolve pass
            // only emits it in a Rational-typed context.
            BinaryOpKind::ExactDivide => {
                self.infer_expr(left)?;
                self.infer_expr(right)?;
                Ok(InferType::Rational)
            }

            BinaryOpKind::Subtract
            | BinaryOpKind::Multiply
            | BinaryOpKind::Divide
            | BinaryOpKind::FloorDivide
            | BinaryOpKind::Modulo
            | BinaryOpKind::Pow => {
                let lt = self.infer_expr(left)?;
                let rt = self.infer_expr(right)?;
                if lt == InferType::Unknown || rt == InferType::Unknown {
                    Ok(InferType::Unknown)
                } else {
                    unify_numeric(&lt, &rt).or(Ok(InferType::Unknown))
                }
            }
        }
    }

    /// Infer a named function call.
    ///
    /// For generic functions, instantiates the `TypeScheme` with fresh type variables,
    /// then unifies the instantiated parameter types with the argument types. The
    /// instantiated return type is then resolved and returned as the call result type.
    fn infer_call(
        &mut self,
        function: Symbol,
        args: &[&Expr],
        site: Site,
    ) -> Result<InferType, TypeError> {
        let name = self.interner.resolve(function);
        match name {
            "pow"
                if self
                    .functions
                    .get(&function)
                    .map_or(true, |rec| rec.is_native) =>
            {
                let arg_types = args
                    .iter()
                    .map(|arg| self.infer_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(match arg_types.as_slice() {
                    [base, exponent] => {
                        let base = self.table.resolve(base);
                        let exponent = self.table.resolve(exponent);
                        if base == InferType::Unknown || exponent == InferType::Unknown {
                            InferType::Unknown
                        } else {
                            unify_numeric(&base, &exponent).unwrap_or(InferType::Unknown)
                        }
                    }
                    _ => InferType::Unknown,
                })
            }
            // `readReals(path)`: the file's numbers, a `Seq of Real`.
            "readReals" => {
                for arg in args {
                    self.infer_expr(arg)?;
                }
                Ok(InferType::Seq(Box::new(InferType::Float)))
            }
            "sqrt" | "parseFloat" | "parseInt" | "floor" | "ceil" | "round" | "abs" | "min"
            | "max" => {
                // A built-in's arguments are expressions like any other: a generic call among them
                // is solved here too.
                let mut arg_types = Vec::with_capacity(args.len());
                for arg in args {
                    arg_types.push(self.infer_expr(arg)?);
                }
                Ok(match name {
                    "sqrt" | "parseFloat" => InferType::Float,
                    "parseInt" | "floor" | "ceil" | "round" => InferType::Int,
                    _ => arg_types.into_iter().next().unwrap_or(InferType::Unknown),
                })
            }
            // `{k: v, …}` is `mapOf(k, v, …)` in flat pairs and `{a, …}` is `setOf(a, …)`: a Map
            // of the first pair's key and value types, a Set of the first element's type, each
            // later pair or element joined to the first as a list literal's elements are.
            "mapOf" | "setOf" if !args.is_empty() && !self.functions.contains_key(&function) => {
                let mut arg_types = Vec::with_capacity(args.len());
                for arg in args {
                    arg_types.push(self.infer_expr(arg)?);
                }
                let stride = if name == "mapOf" { 2 } else { 1 };
                for (at, ty) in arg_types.iter().enumerate().skip(stride) {
                    self.table.unify(&arg_types[at % stride], ty).ok();
                }
                Ok(match name {
                    "mapOf" if arg_types.len() >= 2 => InferType::Map(
                        Box::new(arg_types[0].clone()),
                        Box::new(arg_types[1].clone()),
                    ),
                    "mapOf" => InferType::Unknown,
                    _ => InferType::Set(Box::new(arg_types[0].clone())),
                })
            }
            _ => {
                if let Some(rec) = self.functions.get(&function).cloned() {
                    // Instantiate the scheme: each call site gets fresh type variables
                    // for generic params so calls don't interfere with each other.
                    let (instantiated, type_args) =
                        self.table.instantiate_with(&rec.scheme, &rec.declared);
                    let generic = !type_args.is_empty();
                    if generic {
                        self.sites.calls.insert(
                            site,
                            CallInstance {
                                type_args,
                                precise: true,
                            },
                        );
                    }

                    if let InferType::Function(param_types, ret_box) = instantiated {
                        // Arity is checked BEFORE unification, because `zip` stops at the shorter
                        // side: a call with too few or too many arguments used to type-check
                        // cleanly against the arguments it did have, so `ArityMismatch` was
                        // unreachable from source and `add(1)` reached the runtime.
                        if args.len() != param_types.len() {
                            return Err(TypeError::ArityMismatch {
                                expected: param_types.len(),
                                found: args.len(),
                            });
                        }
                        // Unify each argument type with the instantiated parameter type
                        let mut arg_types = Vec::with_capacity(args.len());
                        for (arg, param_ty) in args.iter().zip(param_types.iter()) {
                            arg_types.push(self.check_widening(arg, param_ty)?);
                        }
                        if generic {
                            self.call_arg_types.insert(site, arg_types);
                        }
                        // RESOLVED, not zonked: a result still open (a generic call inside a
                        // generic body) keeps its variable, so the enclosing call can solve it.
                        Ok(self.table.resolve(&ret_box))
                    } else {
                        // Should not happen, but fall back gracefully
                        Ok(InferType::Unknown)
                    }
                } else {
                    Ok(InferType::Unknown)
                }
            }
        }
    }

    /// Check `value` flowing into a slot of type `expected` — a call's parameter, a variant's
    /// field.
    ///
    /// ★ AN INT VALUE WIDENS TO A `Real` SLOT — `scaleHalf(5)` into `(x: Real)`, the coercion
    /// LOGOS performs everywhere and the one
    /// `parity_stdlib_aot_rust::int_literal_to_real_param_coerces_on_aot_rust` specifies. The
    /// unifier has no `(Int, Float)` arm, and deliberately should not: making them unify
    /// SYMMETRICALLY would also accept a Real where an Int is wanted, which is a real mistake the
    /// checker has to keep catching.
    ///
    /// So it is handled HERE, directionally, and keyed on EACH SLOT'S OWN TYPE rather than on the
    /// call — `mix(1, 2)` into `(a: Int) and (b: Real)` must widen the second argument and leave
    /// the first an i64, because coercing it would emit `(2) as f64` where an i64 is wanted.
    fn check_widening(
        &mut self,
        value: &Expr,
        expected: &InferType,
    ) -> Result<InferType, TypeError> {
        let value_ty = self.infer_expr(value)?;
        let widens = matches!(
            self.table.resolve(&value_ty),
            InferType::Int | InferType::Nat
        ) && matches!(self.table.resolve(expected), InferType::Float);
        if !widens {
            self.table.unify(&value_ty, expected)?;
        }
        Ok(value_ty)
    }

    /// Infer a call-expression (calling a closure/function-value).
    fn infer_call_expr(&mut self, callee: &Expr, args: &[&Expr]) -> Result<InferType, TypeError> {
        let callee_ty = self.infer_expr(callee)?;
        let ret_var = self.table.fresh();
        let arg_types: Vec<InferType> = args
            .iter()
            .map(|a| self.infer_expr(a))
            .collect::<Result<_, _>>()?;
        let fn_ty = InferType::Function(arg_types, Box::new(ret_var.clone()));

        let walked = self.table.resolve(&callee_ty);
        match walked {
            InferType::Unknown => Ok(ret_var),
            InferType::Function(_, _) => {
                self.table.unify(&walked, &fn_ty)?;
                Ok(ret_var)
            }
            InferType::Var(_) => {
                self.table.unify(&walked, &fn_ty)?;
                Ok(ret_var)
            }
            other => Err(TypeError::NotAFunction { found: other }),
        }
    }

    /// Infer a closure literal.
    fn infer_closure(
        &mut self,
        params: &[(Symbol, &crate::ast::stmt::TypeExpr)],
        body: &crate::ast::stmt::ClosureBody,
        return_type: &Option<&crate::ast::stmt::TypeExpr>,
    ) -> Result<InferType, TypeError> {
        let param_types: Vec<InferType> = params
            .iter()
            .map(|(_, ty_expr)| self.written_type(ty_expr))
            .collect();

        let ret_type = if let Some(rt) = return_type {
            self.written_type(rt)
        } else {
            self.table.fresh()
        };

        self.push_scope();
        for ((sym, _), ty) in params.iter().zip(param_types.iter()) {
            self.bind_var(*sym, ty.clone());
        }

        let prev_return = self.current_return_type.take();
        self.current_return_type = Some(ret_type.clone());

        match body {
            crate::ast::stmt::ClosureBody::Expression(expr) => {
                let body_ty = self.infer_expr(expr)?;
                // Best-effort unification: won't fail compilation on ambiguity
                self.table.unify(&body_ty, &ret_type).ok();
            }
            crate::ast::stmt::ClosureBody::Block(stmts) => {
                // Expression inference speaks plain `TypeError`, so a finding inside a closure's
                // block is named from the statement that holds the closure outward.
                for stmt in *stmts {
                    self.infer_stmt(stmt).map_err(|traced| traced.error)?;
                }
            }
        }

        self.current_return_type = prev_return;
        self.pop_scope();

        Ok(InferType::Function(param_types, Box::new(ret_type)))
    }

    /// Infer the type of a field access on a struct.
    fn infer_field_access(&self, obj_ty: InferType, field: Symbol) -> Result<InferType, TypeError> {
        let resolved = self.table.resolve(&obj_ty);
        match &resolved {
            InferType::UserDefined(type_sym, args) => {
                if let Some(TypeDef::Struct {
                    fields, generics, ..
                }) = self.registry.get(*type_sym)
                {
                    if let Some(field_def) = fields.iter().find(|f| f.name == field) {
                        // A bare reference to a generic record reads its parameters as unknown.
                        let args: Vec<InferType> = if args.len() == generics.len() {
                            args.clone()
                        } else {
                            generics.iter().map(|_| InferType::Unknown).collect()
                        };
                        Ok(self.field_type_at(&field_def.ty, generics, &args))
                    } else {
                        Err(TypeError::FieldNotFound {
                            type_name: *type_sym,
                            field_name: field,
                        })
                    }
                } else {
                    // Not a struct in registry → Unknown (defensive)
                    Ok(InferType::Unknown)
                }
            }
            // Can't resolve field on non-struct type
            _ => Ok(InferType::Unknown),
        }
    }
}

// ============================================================================
// Statement inference
// ============================================================================

/// A type error on its way out of the statements it was found in, naming each
/// one as it passes, innermost first. The trail lives in the error itself, so
/// an error some caller recovers from takes its trail with it.
#[derive(Debug)]
struct Traced {
    error: TypeError,
    trail: Vec<StmtId>,
}

impl From<TypeError> for Traced {
    fn from(error: TypeError) -> Self {
        Traced {
            error,
            trail: Vec::new(),
        }
    }
}

impl<'r> CheckEnv<'r> {
    fn infer_stmt(&mut self, stmt: &Stmt) -> Result<(), Traced> {
        self.infer_stmt_kind(stmt).map_err(|mut traced| {
            traced.trail.push(StmtId::of(stmt));
            traced
        })
    }

    fn infer_stmt_kind(&mut self, stmt: &Stmt) -> Result<(), Traced> {
        match stmt {
            Stmt::Let { var, ty, value, .. } => {
                let final_ty = if let Some(type_expr) = ty {
                    let annotated = self.written_type(type_expr);
                    if annotated != InferType::Unknown {
                        // Checking mode: value must be compatible with annotation
                        self.check_expr(value, &annotated)?
                    } else {
                        self.infer_expr(value)?
                    }
                } else {
                    self.infer_expr(value)?
                };
                self.bind_var(*var, final_ty);
                Ok(())
            }

            // What a lent place holds while the call holds its value keeps the place's type.
            Stmt::Set { value, .. } | Stmt::SetIndex { value, .. } | Stmt::SetField { value, .. }
                if crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::Vacated, value) =>
            {
                Ok(())
            }

            Stmt::Set { target, value } => {
                let inferred = self.infer_expr(value)?;
                // If target already has a type, unify. Otherwise just bind.
                if let Some(existing) = self.lookup_var(*target) {
                    if existing != InferType::Unknown {
                        self.table.unify(&inferred, &existing).ok();
                    }
                }
                // Update binding
                let resolved = self.table.resolve(&inferred);
                if resolved != InferType::Unknown {
                    self.bind_var(*target, resolved);
                }
                Ok(())
            }

            Stmt::FunctionDef {
                name,
                generics,
                params,
                body,
                return_type,
                is_native,
                ..
            } => {
                // Build a type-param map: Symbol("T") → TyVar
                // Re-use the TyVars already allocated in preregister_functions if present,
                // or allocate fresh ones if this function was not pre-registered.
                let type_param_map: HashMap<Symbol, TyVar> = {
                    // Try to recover the same TyVars from the pre-registered scheme
                    let existing_vars: Vec<TyVar> = self.functions
                        .get(name)
                        .map(|rec| rec.scheme.vars.clone())
                        .unwrap_or_default();
                    if existing_vars.len() == generics.len() {
                        generics.iter().copied().zip(existing_vars).collect()
                    } else {
                        generics.iter().map(|&sym| (sym, self.table.fresh_var())).collect()
                    }
                };

                let in_scope: HashMap<Symbol, InferType> =
                    type_param_map.iter().map(|(s, tv)| (*s, InferType::Var(*tv))).collect();
                let param_types: Vec<InferType> = params
                    .iter()
                    .map(|(_, ty_expr)| self.written_type_with(ty_expr, &in_scope))
                    .collect();
                let param_names: Vec<Symbol> = params.iter().map(|(sym, _)| *sym).collect();

                let ret_type = if let Some(rt) = return_type {
                    self.written_type_with(rt, &in_scope)
                } else if let Some(rec) = self.functions.get(name) {
                    // Recover pre-registered return type from the scheme body
                    if let InferType::Function(_, ret_box) = &rec.scheme.body {
                        *ret_box.clone()
                    } else {
                        self.table.fresh()
                    }
                } else {
                    self.table.fresh()
                };

                let generic_vars: Vec<TyVar> = generics
                    .iter()
                    .filter_map(|sym| type_param_map.get(sym).copied())
                    .collect();
                if !generic_vars.is_empty() {
                    self.sites.type_params.insert(
                        *name,
                        generics.iter().map(|g| (*g, InferType::Var(type_param_map[g]))).collect(),
                    );
                }

                // Native functions: register scheme, no body to check
                if *is_native {
                    let declared = generic_vars.iter().map(|tv| InferType::Var(*tv)).collect();
                    let scheme = TypeScheme {
                        vars: generic_vars,
                        body: InferType::Function(param_types, Box::new(ret_type)),
                    };
                    self.functions.insert(*name, FunctionRecord { param_names, scheme, declared, is_native: *is_native });
                    return Ok(());
                }

                // Save previous return context
                let prev_return_type = self.current_return_type.take();
                self.current_return_type = Some(ret_type.clone());
                let prev_type_params = std::mem::replace(&mut self.type_params_in_scope, in_scope);

                // Check body in a new scope with params bound
                self.push_scope();
                for (sym, ty) in param_names.iter().zip(param_types.iter()) {
                    self.bind_var(*sym, ty.clone());
                }
                let checked = body.iter().try_for_each(|s| self.infer_stmt(s));
                self.pop_scope();

                self.type_params_in_scope = prev_type_params;
                self.current_return_type = prev_return_type;
                checked?;

                // After checking the body, update the registered scheme with resolved types.
                // Use `resolve` (not `zonk`) so generic TyVars remain as `Var(tv)` in
                // the scheme body — they will be instantiated fresh at each call site.
                let resolved_params: Vec<InferType> = param_types
                    .iter()
                    .map(|ty| self.table.resolve(ty))
                    .collect();
                let resolved_ret = self.table.resolve(&ret_type);

                // Generalize over EVERY free type variable in the resolved
                // signature, not just the declared generics: a function with an
                // INFERRED (unannotated) return type allocates a fresh return
                // variable that is genuinely polymorphic but absent from
                // `generic_vars`. Leaving it free would share it across call
                // sites (cross-call contamination); adding it to `vars` makes
                // `instantiate` freshen it per call.
                fn collect_type_vars(ty: &InferType, acc: &mut Vec<TyVar>) {
                    match ty {
                        InferType::Var(tv) => {
                            if !acc.contains(tv) {
                                acc.push(*tv);
                            }
                        }
                        InferType::Seq(i) | InferType::Set(i) | InferType::Option(i) => {
                            collect_type_vars(i, acc)
                        }
                        InferType::Map(k, v) => {
                            collect_type_vars(k, acc);
                            collect_type_vars(v, acc);
                        }
                        InferType::UserDefined(_, args) => {
                            for a in args {
                                collect_type_vars(a, acc);
                            }
                        }
                        InferType::Function(ps, r) => {
                            for p in ps {
                                collect_type_vars(p, acc);
                            }
                            collect_type_vars(r, acc);
                        }
                        _ => {}
                    }
                }
                // Generalize over the ROOTS the declared parameters resolved to: checking the body
                // may have bound a parameter's own variable to another (a recursive call unifies
                // it with that call's fresh variable), and a scheme quantifying the stale variable
                // would leave its body un-freshened at every later call.
                let declared: Vec<InferType> =
                    generic_vars.iter().map(|tv| self.table.resolve(&InferType::Var(*tv))).collect();
                let mut scheme_vars = Vec::new();
                for d in &declared {
                    collect_type_vars(d, &mut scheme_vars);
                }
                for p in &resolved_params {
                    collect_type_vars(p, &mut scheme_vars);
                }
                collect_type_vars(&resolved_ret, &mut scheme_vars);
                let scheme = TypeScheme {
                    vars: scheme_vars,
                    body: InferType::Function(resolved_params, Box::new(resolved_ret)),
                };
                self.functions.insert(*name, FunctionRecord { param_names, scheme, declared, is_native: *is_native });
                Ok(())
            }

            Stmt::Return { value } => {
                let ty = match value {
                    Some(expr) => self.infer_expr(expr)?,
                    None => InferType::Unit,
                };
                if let Some(expected) = self.current_return_type.clone() {
                    // Hard check for explicit return type annotations
                    if expected != InferType::Unknown {
                        self.table.unify(&ty, &expected)?;
                    }
                }
                Ok(())
            }

            Stmt::Repeat { pattern, iterable, body } => {
                let iterable_ty = self.infer_expr(iterable)?;
                let elem_ty = match self.table.resolve(&iterable_ty) {
                    InferType::Seq(inner) | InferType::Set(inner) => *inner,
                    // A Map yields (key, value) tuples per entry at runtime
                    // (semantics/collections.rs `iteration_snapshot`), NOT bare
                    // keys. InferType has no tuple type, so a single loop
                    // variable binds to Unknown — sound (it cannot drive a wrong
                    // specialization), unlike the bare key type `K`.
                    InferType::Map(_, _) => InferType::Unknown,
                    _ => InferType::Unknown,
                };
                match pattern {
                    Pattern::Identifier(sym) => self.bind_var(*sym, elem_ty),
                    Pattern::Tuple(syms) => {
                        for sym in syms {
                            self.bind_var(*sym, InferType::Unknown);
                        }
                    }
                }
                for s in *body {
                    self.infer_stmt(s)?;
                }
                Ok(())
            }

            Stmt::If { then_block, else_block, .. } => {
                for s in *then_block {
                    self.infer_stmt(s)?;
                }
                if let Some(else_b) = else_block {
                    for s in *else_b {
                        self.infer_stmt(s)?;
                    }
                }
                Ok(())
            }

            Stmt::While { body, .. } => {
                for s in *body {
                    self.infer_stmt(s)?;
                }
                Ok(())
            }

            Stmt::Inspect { target, arms, .. } => {
                let target_ty = self.infer_expr(target)?;
                self.sites.scrutinees.insert(Site::of_stmt(stmt), target_ty.clone());
                for arm in arms {
                    self.push_scope();
                    self.infer_inspect_arm(arm, &target_ty)?;
                    self.pop_scope();
                }
                Ok(())
            }

            Stmt::Zone { body, .. } => {
                for s in *body {
                    self.infer_stmt(s)?;
                }
                Ok(())
            }

            Stmt::ReadFrom { var, .. } => {
                self.bind_var(*var, InferType::String);
                Ok(())
            }

            Stmt::CreatePipe { var, element_type, .. } => {
                let elem = InferType::from_type_name(self.interner.resolve(*element_type));
                self.bind_var(*var, elem);
                Ok(())
            }

            Stmt::ReceivePipe { var, pipe } => {
                // Pipe var was registered with its element type by CreatePipe
                let elem_ty = self.infer_expr(pipe)?;
                self.bind_var(*var, elem_ty);
                Ok(())
            }

            Stmt::TryReceivePipe { var, pipe } => {
                let elem_ty = self.infer_expr(pipe)?;
                // TryReceivePipe yields Option of elem type
                self.bind_var(*var, InferType::Option(Box::new(elem_ty)));
                Ok(())
            }

            Stmt::Pop { into: Some(var), collection } => {
                let coll_ty = self.infer_expr(collection)?;
                let elem_ty = match self.table.resolve(&coll_ty) {
                    InferType::Seq(inner) | InferType::Set(inner) => *inner,
                    _ => InferType::Unknown,
                };
                self.bind_var(*var, elem_ty);
                Ok(())
            }

            Stmt::AwaitMessage { into, .. } => {
                self.bind_var(*into, InferType::Unknown);
                Ok(())
            }

            Stmt::LaunchTaskWithHandle { handle, .. } => {
                self.bind_var(*handle, InferType::Unknown);
                Ok(())
            }

            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                for s in *tasks {
                    self.infer_stmt(s)?;
                }
                Ok(())
            }

            Stmt::Select { branches } => {
                for branch in branches {
                    match branch {
                        crate::ast::stmt::SelectBranch::Receive { var, pipe, body } => {
                            let elem_ty = self.infer_expr(pipe)?;
                            self.push_scope();
                            self.bind_var(*var, elem_ty);
                            for s in *body {
                                self.infer_stmt(s)?;
                            }
                            self.pop_scope();
                        }
                        crate::ast::stmt::SelectBranch::Timeout { body, .. } => {
                            for s in *body {
                                self.infer_stmt(s)?;
                            }
                        }
                    }
                }
                Ok(())
            }

            // `Show x.` — the printed expression was never inferred, so ANY type error inside a
            // `Show` was invisible to the checker. That single gap accounted for most of the
            // ill-typed programs `largo check` used to accept: `Show totl.` (undefined),
            // `Show add(1).` (wrong arity), `Show double("abc").` (wrong argument type) all sat
            // inside a statement the checker skipped entirely.
            Stmt::Show { object, recipient } => {
                self.infer_expr(object)?;
                self.infer_expr(recipient)?;
                Ok(())
            }

            // A call used as a statement rather than a value: `add(1).` must still check its
            // arguments and arity.
            Stmt::Call { function, args } => {
                let arg_refs: Vec<&Expr> = args.iter().copied().collect();
                self.infer_call(*function, &arg_refs, Site::of_stmt(stmt))?;
                Ok(())
            }

            // `Push v to xs.` — the element type must match what the collection holds.
            Stmt::Push { value, collection } => {
                let value_ty = self.infer_expr(value)?;
                let coll_ty = self.infer_expr(collection)?;
                if let InferType::Seq(elem) = self.table.resolve(&coll_ty) {
                    self.table.unify(&value_ty, &elem)?;
                }
                Ok(())
            }


            // `Set p's x to v.` — the value must match the declared field type. Without this arm a
            // Text could be stored in an `Int` field and no tier ever complained.
            Stmt::SetField { object, field, value } => {
                let obj_ty = self.infer_expr(object)?;
                let value_ty = self.infer_expr(value)?;
                // `infer_field_access` also raises `FieldNotFound`, so assigning to a field the
                // struct does not have is caught here too, not just a type mismatch on one it has.
                let field_ty = self.infer_field_access(obj_ty, *field)?;
                self.table.unify(&value_ty, &field_ty)?;
                Ok(())
            }

            // `Set item k of m to v.` — both the key and the value must match the collection.
            Stmt::SetIndex { collection, index, value } => {
                let coll_ty = self.infer_expr(collection)?;
                let index_ty = self.infer_expr(index)?;
                let value_ty = self.infer_expr(value)?;
                match self.table.resolve(&coll_ty) {
                    InferType::Map(key_ty, val_ty) => {
                        self.table.unify(&index_ty, &key_ty)?;
                        self.table.unify(&value_ty, &val_ty)?;
                    }
                    InferType::Seq(elem) => {
                        self.table.unify(&value_ty, &elem)?;
                    }
                    _ => {}
                }
                Ok(())
            }

            // `Swap a and b.` — swapping two names requires them to hold the same type.
            Stmt::Swap { a, b } => {
                if let (Some(ta), Some(tb)) = (self.lookup_var(*a), self.lookup_var(*b)) {
                    self.table.unify(&ta, &tb)?;
                }
                Ok(())
            }

            // `Pop from xs.` with no destination. The existing `Pop` arm only matched the form
            // that binds a name, so this one fell through the old wildcard — a gap the
            // exhaustiveness check surfaced the moment it was removed. Nothing is bound, but the
            // collection is still an expression worth inferring.
            Stmt::Pop { collection, into: None, .. } => {
                self.infer_expr(collection)?;
                Ok(())
            }

            // EVERY REMAINING STATEMENT KIND, NAMED EXPLICITLY AND DELIBERATELY UNCHECKED.
            //
            // This match has NO WILDCARD, and that is the point. It used to end in `_ => Ok(())`,
            // which meant `Show`, `Push` and a statement-level call were silently skipped by the
            // type checker — the single biggest reason `largo check` accepted broken programs. A
            // wildcard cannot tell "this statement needs no checking" apart from "nobody has got
            // to this one yet", so it hid 62 statement kinds behind an answer that looked settled.
            //
            // Listing them by name costs one line per new variant and buys the property that
            // matters: a statement kind added to the language CANNOT silently skip type checking,
            // because this file stops compiling until someone classifies it. That is the same
            // discipline the op catalogs use, applied to the checker.
            //
            // These are not claimed to be type-safe — they are claimed to be UNCHECKED, on the
            // record. Most carry expressions that should eventually be inferred (the `Assert`
            // family, the CRDT operations, the IO and channel statements); the logic-mode
            // declarations (`Axiom`, `Theorem`, `Theory`, `ForAll`, `Definition`) genuinely have
            // nothing for this checker to constrain. Moving one out of this group is a Wave 2
            // task, and the ill-typed corpus in `type_checker_rejects_lock` says which ones pay.

            Stmt::Add { .. } | Stmt::AdvanceTime { .. } | Stmt::AppendToSequence { .. } |
            Stmt::Assert { .. } | Stmt::AwaitTask { .. } | Stmt::Axiom(..) | Stmt::Break |
            Stmt::Check { .. } | Stmt::CloseChannel { .. } | Stmt::ComponentDef(..) |
            Stmt::ConnectTo { .. } | Stmt::DecreaseCrdt { .. } | Stmt::Definition(..) |
            Stmt::DirectiveDef { .. } | Stmt::During { .. } | Stmt::Escape { .. } | Stmt::Expect { .. } |
            Stmt::ExpectFail { .. } | Stmt::ExpectOutput { .. } | Stmt::Fetch { .. } | Stmt::ForAll { .. } |
            Stmt::Give { .. } | Stmt::GivenFetch { .. } | Stmt::GivenFile { .. } |
            Stmt::IncreaseCrdt { .. } | Stmt::JoinCrdt { .. } | Stmt::LaunchTask { .. } |
            Stmt::LetPeerAgent { .. } | Stmt::Listen { .. } | Stmt::MergeCrdt { .. } |
            Stmt::MiddlewareDef { .. } | Stmt::Mount { .. } | Stmt::Observe { .. } |
            Stmt::RedirectDef { .. } | Stmt::Remove { .. } | Stmt::Replay { .. } | Stmt::Require { .. } |
            Stmt::ResolveConflict { .. } | Stmt::Reversibly { .. } | Stmt::RouteDef { .. } |
            Stmt::RuntimeAssert { .. } | Stmt::Fail { .. } | Stmt::SendMessage { .. } | Stmt::SendPipe { .. } |
            Stmt::Serve { .. } | Stmt::ServeFetches | Stmt::Sleep { .. } | Stmt::Spawn { .. } |
            Stmt::Splice { .. } | Stmt::StopTask { .. } | Stmt::StreamMessage { .. } |
            Stmt::StructDef { .. } | Stmt::Sync { .. } | Stmt::TestDef { .. } | Stmt::Theorem(..) |
            Stmt::Theory(..) | Stmt::Trust { .. } | Stmt::TrySendPipe { .. } | Stmt::Ui(..) |
            Stmt::WriteFile { .. } |
            // An opaque failure-site marker: an id, no type to check or infer.
            Stmt::Site(_) => Ok(()),
        }
    }

    /// Process a single Inspect match arm, binding variant field types — read at the inspected
    /// value's own type arguments when it has them (`When Just (v)` on a `Maybe of Int` binds an
    /// `Int`, and `When Some (v)` on an `Option of Text` a `Text`). A type parameter the value
    /// does not pin stays unknown.
    fn infer_inspect_arm(
        &mut self,
        arm: &crate::ast::stmt::MatchArm,
        target_ty: &InferType,
    ) -> Result<(), Traced> {
        let target = self.table.resolve(target_ty);
        let bound: Option<Vec<(Symbol, InferType)>> = arm.variant.map(|variant_sym| {
            if let InferType::Option(inner) = &target {
                // `Some (v)`: the one payload is the Option's element.
                return arm
                    .bindings
                    .iter()
                    .map(|(_, b)| (*b, (**inner).clone()))
                    .collect();
            }
            let applied = match &target {
                InferType::UserDefined(enum_sym, args) => match self.registry.get(*enum_sym) {
                    Some(TypeDef::Enum {
                        variants, generics, ..
                    }) => variants
                        .iter()
                        .find(|v| v.name == variant_sym)
                        .map(|v| (v.clone(), generics.clone(), args.clone())),
                    _ => None,
                },
                _ => None,
            };
            let (variant_def, generics, args) = match applied {
                Some(found) => found,
                None => match self.registry.find_variant(variant_sym) {
                    Some((enum_sym, v)) => {
                        let generics = match self.registry.get(enum_sym) {
                            Some(TypeDef::Enum { generics, .. }) => generics.clone(),
                            _ => Vec::new(),
                        };
                        (v.clone(), generics, Vec::new())
                    }
                    None => {
                        return arm
                            .bindings
                            .iter()
                            .map(|(_, b)| (*b, InferType::Unknown))
                            .collect();
                    }
                },
            };
            let args: Vec<InferType> = if args.len() == generics.len() {
                args
            } else {
                generics.iter().map(|_| InferType::Unknown).collect()
            };
            arm.bindings
                .iter()
                .map(|(field_sym, binding_sym)| {
                    let ty = variant_def
                        .fields
                        .iter()
                        .find(|f| f.name == *field_sym)
                        .map(|f| self.field_type_at(&f.ty, &generics, &args))
                        .unwrap_or(InferType::Unknown);
                    (*binding_sym, ty)
                })
                .collect()
        });
        match bound {
            Some(bindings) => {
                for (binding_sym, ty) in bindings {
                    self.bind_var(binding_sym, ty);
                }
            }
            // Otherwise arm: wildcard bindings
            None => {
                for (_, binding_sym) in &arm.bindings {
                    self.bind_var(*binding_sym, InferType::Unknown);
                }
            }
        }

        for s in arm.body {
            self.infer_stmt(s)?;
        }
        Ok(())
    }
}

// ============================================================================
// Entry point
// ============================================================================

/// One typechecker finding with the top-level statement it belongs to.
///
/// `stmt_index` maps 1:1 onto `Parser::stmt_spans()`, giving IDE diagnostics
/// a real source span; `trail` names the statements the finding propagated
/// out through, innermost first, so a finding inside a function body or a loop
/// resolves through `Parser::nested_stmt_spans()` to the statement at fault.
#[derive(Debug)]
pub struct IndexedTypeError {
    pub stmt_index: Option<usize>,
    pub trail: Vec<StmtId>,
    pub error: TypeError,
}

/// Collect EVERY failing top-level statement instead of bailing at the first.
///
/// The environment after a failed statement is best-effort: inference simply
/// continues with whatever bindings succeeded, so one bad `Let` does not
/// cascade into errors on unrelated later statements. The strict fail-fast
/// contract lives in [`check_program_located`], which every compile path uses.
pub fn check_program_collect(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
) -> (TypeEnv, Vec<IndexedTypeError>) {
    let mut errors = Vec::new();

    if let Err(e) =
        crate::analysis::dimension_check::DimensionChecker::new(interner).check_program(stmts)
    {
        let owner = e.trail.last().copied();
        errors.push(IndexedTypeError {
            stmt_index: stmts.iter().position(|s| Some(StmtId::of(s)) == owner),
            trail: e.trail,
            error: TypeError::DimensionMismatch { message: e.message },
        });
    }

    let mut env = CheckEnv::new(registry, interner);
    env.preregister_functions(stmts);

    for (index, stmt) in stmts.iter().enumerate() {
        if let Err(Traced { error, trail }) = env.infer_stmt(stmt) {
            errors.push(IndexedTypeError {
                stmt_index: Some(index),
                trail,
                error,
            });
        }
    }

    (env.into_type_env(), errors)
}

/// Check `stmts` as far as each statement goes — a failing statement does not stop the rest — and
/// answer what every generic site was solved at. The input to monomorphization; a backend with
/// no generics of its own reads which instance each call and construction needs from here.
pub fn infer_instantiations(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
) -> Instantiations {
    let mut env = CheckEnv::new(registry, interner);
    env.preregister_functions(stmts);
    for stmt in stmts {
        let _ = env.infer_stmt(stmt);
    }
    env.into_instantiations()
}

/// Check a LOGOS program and return a typed `TypeEnv` for codegen.
///
/// Replaces `TypeEnv::infer_program`. Returns `Err(TypeError)` only on
/// genuine type contradictions (e.g., `Let x: Int be "hello"`).
/// Ambiguous types fall back to `LogosType::Unknown` silently.
pub fn check_program(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
) -> Result<TypeEnv, TypeError> {
    check_program_located(stmts, interner, registry).map_err(|e| e.error)
}

/// A type error and the top-level function it was found in — `None` for a `## Main` statement. A
/// checked program has no source spans left (lowering writes functions of its own), so the function
/// is the location a report can give.
#[derive(Debug, Clone)]
pub struct LocatedTypeError {
    pub error: TypeError,
    pub within: Option<Symbol>,
}

impl LocatedTypeError {
    /// The user-facing error, naming where a mismatch was found: the function its author wrote, or
    /// the construct (`## Route GET /agent`, a component's action) a compiler-written function was
    /// lowered from, found among the declarations in `program`. A synthesized name is never shown.
    pub fn to_parse_error_kind(
        &self,
        interner: &Interner,
        program: &[Stmt],
    ) -> crate::error::ParseErrorKind {
        let mut kind = self.error.to_parse_error_kind(interner);
        if let (crate::error::ParseErrorKind::TypeMismatchDetailed { context, .. }, Some(f)) =
            (&mut kind, self.within)
        {
            if context.is_empty() {
                let name = interner.resolve(f);
                let place = if logicaffeine_language::parser::lgx_lower::is_compiler_written(name) {
                    logicaffeine_language::parser::lgx_lower::authored_construct(
                        name, program, interner,
                    )
                } else {
                    Some(format!("`{name}`"))
                };
                if let Some(place) = place {
                    *context = format!("in {place}");
                }
            }
        }
        kind
    }
}

/// The first type the program names that nothing supplies, with where it is named: the index of the
/// top-level statement that names it, or `None` for a field of a declared record or variant (a
/// declaration is not a statement). A name is supplied by the registry (a record, enum or alias the
/// program declares or imports, a builtin, an intrinsic generic), by the language itself
/// (`codegen::types::is_language_type_name`, and the dimension words `Quantity of <Dimension>`
/// takes, `Dimension::by_name`), or as a type parameter the declaration itself introduces. Run on
/// the program the parser returned, whose top-level statements align with its statement spans, so a
/// statement's refusal is located. A written type counts wherever it stands: a construction, a
/// collection's element type, an annotation, a typed fetch, a function's parameters and return
/// type, a record's or variant's field types.
pub fn first_undeclared_type(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    interner: &Interner,
) -> Option<(Option<usize>, Symbol)> {
    use crate::analysis::callgraph::{collect_type_refs_from_stmts, type_names_in_type_expr};
    use std::collections::HashSet;
    let supplied = |sym: Symbol, own_params: &HashSet<Symbol>| {
        let name = interner.resolve(sym);
        own_params.contains(&sym)
            || registry.get(sym).is_some()
            || crate::codegen::types::is_language_type_name(name)
            || logicaffeine_base::dimension::Dimension::by_name(name).is_some()
    };
    let first_of = |named: HashSet<Symbol>, own_params: &HashSet<Symbol>| {
        let mut undeclared: Vec<Symbol> = named
            .into_iter()
            .filter(|sym| !supplied(*sym, own_params))
            .collect();
        undeclared.sort_by(|a, b| interner.resolve(*a).cmp(interner.resolve(*b)));
        undeclared.first().copied()
    };
    for (index, stmt) in stmts.iter().enumerate() {
        let mut named: HashSet<Symbol> = HashSet::new();
        let mut own_params: HashSet<Symbol> = HashSet::new();
        match stmt {
            Stmt::FunctionDef {
                generics,
                params,
                return_type,
                body,
                ..
            } => {
                own_params.extend(generics.iter().copied());
                for (_, ty) in params {
                    type_names_in_type_expr(ty, &mut named);
                }
                if let Some(ty) = return_type {
                    type_names_in_type_expr(ty, &mut named);
                }
                collect_type_refs_from_stmts(body, &mut named);
            }
            other => collect_type_refs_from_stmts(std::slice::from_ref(other), &mut named),
        }
        if let Some(first) = first_of(named, &own_params) {
            return Some((Some(index), first));
        }
    }
    let mut declared: Vec<(Symbol, &TypeDef)> = registry
        .iter_types()
        .map(|(sym, def)| (*sym, def))
        .collect();
    declared.sort_by(|a, b| interner.resolve(a.0).cmp(interner.resolve(b.0)));
    for (_, def) in declared {
        let (fields, generics): (Vec<&FieldType>, &Vec<Symbol>) = match def {
            TypeDef::Struct {
                fields, generics, ..
            } => (fields.iter().map(|f| &f.ty).collect(), generics),
            TypeDef::Enum {
                variants, generics, ..
            } => (
                variants
                    .iter()
                    .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                    .collect(),
                generics,
            ),
            _ => continue,
        };
        let mut named: HashSet<Symbol> = HashSet::new();
        for ty in fields {
            written_type_names(ty, &mut named);
        }
        let own_params: HashSet<Symbol> = generics.iter().copied().collect();
        if let Some(first) = first_of(named, &own_params) {
            return Some((None, first));
        }
    }
    None
}

/// The type names a declared field's type WRITES: every named type and generic head, never a
/// `Primitive` — those are the language's own scalars, or discovery's marker for a field whose type it
/// could not read, which names nothing the program wrote.
fn written_type_names(ty: &FieldType, names: &mut std::collections::HashSet<Symbol>) {
    match ty {
        FieldType::Named(sym) => {
            names.insert(*sym);
        }
        FieldType::Generic { base, params } => {
            names.insert(*base);
            for p in params {
                written_type_names(p, names);
            }
        }
        FieldType::Tuple(items) => {
            for t in items {
                written_type_names(t, names);
            }
        }
        FieldType::Record(fields) => {
            for (_, t) in fields {
                written_type_names(t, names);
            }
        }
        FieldType::Primitive(_) | FieldType::TypeParam(_) => {}
    }
}

/// [`check_program`], reporting which top-level function a failure is in.
pub fn check_program_located(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
) -> Result<TypeEnv, LocatedTypeError> {
    // Dimension coherence is a type property: reject `2 meters + 1 gram` here, statically, before
    // any code is generated (a length and a mass have no common dimension).
    crate::analysis::dimension_check::DimensionChecker::new(interner)
        .check_program(stmts)
        .map_err(|e| LocatedTypeError {
            error: TypeError::DimensionMismatch { message: e.message },
            within: None,
        })?;

    Ok(checked(stmts, interner, registry)?.into_type_env())
}

/// The type of every binding the program's ROOT declares, as the checker settled it once the whole
/// program was read. Exact where [`TypeEnv`] is not: its variables are one flat map, so a function's
/// own `x` and the program's `x` share an entry there.
pub fn root_binding_types(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
) -> Result<HashMap<Symbol, InferType>, LocatedTypeError> {
    let env = checked(stmts, interner, registry)?;
    let root = env.scopes.first().cloned().unwrap_or_default();
    Ok(root
        .into_iter()
        .map(|(name, ty)| (name, env.table.zonk(&ty)))
        .collect())
}

/// The environment after checking every top-level statement: function signatures registered first,
/// for forward references, then each statement in order.
fn checked<'r>(
    stmts: &[Stmt],
    interner: &'r Interner,
    registry: &'r TypeRegistry,
) -> Result<CheckEnv<'r>, LocatedTypeError> {
    let mut env = CheckEnv::new(registry, interner);
    env.preregister_functions(stmts);
    for stmt in stmts {
        if let Err(Traced { error, .. }) = env.infer_stmt(stmt) {
            let within = match stmt {
                Stmt::FunctionDef { name, .. } => Some(*name),
                _ => None,
            };
            return Err(LocatedTypeError { error, within });
        }
    }
    Ok(env)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/check/tests.rs"]
mod tests;
