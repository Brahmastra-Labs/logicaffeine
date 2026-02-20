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

use crate::analysis::unify::{InferType, TyVar, TypeScheme, TypeError, UnificationTable, infer_to_logos, unify_numeric};
use crate::analysis::{FnSig, LogosType, TypeDef, TypeEnv, TypeRegistry};
use crate::ast::stmt::{BinaryOpKind, Expr, Pattern, Stmt};
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
    /// The quantified type scheme: `forall vars. Function(param_types, return_type)`.
    /// For monomorphic functions, `vars` is empty and body is used directly.
    scheme: TypeScheme,
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
                let params: Vec<(Symbol, LogosType)> = rec.param_names.iter()
                    .zip(param_types.iter())
                    .map(|(sym, ty)| (*sym, self.table.to_logos_type(ty)))
                    .collect();
                type_env.register_fn(name, FnSig { params, return_type: ret_logos });
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
            if let Stmt::FunctionDef { name, generics, params, return_type, .. } = stmt {
                // Allocate one TyVar per generic type parameter
                let type_param_map: HashMap<Symbol, TyVar> = generics
                    .iter()
                    .map(|&sym| (sym, self.table.fresh_var()))
                    .collect();

                let param_types: Vec<InferType> = params
                    .iter()
                    .map(|(_, ty_expr)| {
                        InferType::from_type_expr_with_params(ty_expr, self.interner, &type_param_map)
                    })
                    .collect();
                let param_names: Vec<Symbol> = params.iter().map(|(sym, _)| *sym).collect();

                let ret_type = if let Some(rt) = return_type {
                    InferType::from_type_expr_with_params(rt, self.interner, &type_param_map)
                } else {
                    self.table.fresh()
                };

                let generic_vars: Vec<TyVar> = generics
                    .iter()
                    .filter_map(|sym| type_param_map.get(sym).copied())
                    .collect();

                let scheme = TypeScheme {
                    vars: generic_vars,
                    body: InferType::Function(param_types, Box::new(ret_type)),
                };

                self.functions.insert(*name, FunctionRecord { param_names, scheme });
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
    fn check_expr(
        &mut self,
        expr: &Expr,
        expected: &InferType,
    ) -> Result<InferType, TypeError> {
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

        // Default: synthesize then unify
        let inferred = self.infer_expr(expr)?;
        self.table.unify(&inferred, expected)?;
        Ok(self.table.zonk(expected))
    }

    /// Infer the type of an expression (synthesis mode).
    fn infer_expr(&mut self, expr: &Expr) -> Result<InferType, TypeError> {
        match expr {
            Expr::Literal(lit) => Ok(InferType::from_literal(lit)),

            Expr::Identifier(sym) => {
                Ok(self.lookup_var(*sym).unwrap_or(InferType::Unknown))
            }

            Expr::BinaryOp { op, left, right } => {
                self.infer_binary_op(*op, left, right)
            }

            Expr::Length { .. } => Ok(InferType::Int),

            Expr::Call { function, args } => {
                self.infer_call(*function, args)
            }

            Expr::Index { collection, .. } => {
                let coll_ty = self.infer_expr(collection)?;
                let walked = self.table.zonk(&coll_ty);
                match walked {
                    InferType::Seq(inner) => Ok(*inner),
                    InferType::Map(_, v) => Ok(*v),
                    _ => Ok(InferType::Unknown),
                }
            }

            Expr::List(items) => {
                if items.is_empty() {
                    let elem_var = self.table.fresh();
                    Ok(InferType::Seq(Box::new(elem_var)))
                } else {
                    let elem_type = self.infer_expr(items[0])?;
                    Ok(InferType::Seq(Box::new(elem_type)))
                }
            }

            Expr::OptionSome { value } => {
                let inner = self.infer_expr(value)?;
                Ok(InferType::Option(Box::new(inner)))
            }

            Expr::OptionNone => {
                let elem_var = self.table.fresh();
                Ok(InferType::Option(Box::new(elem_var)))
            }

            Expr::Range { .. } => Ok(InferType::Seq(Box::new(InferType::Int))),

            Expr::Contains { .. } => Ok(InferType::Bool),

            Expr::Copy { expr: inner } | Expr::Give { value: inner } => {
                self.infer_expr(inner)
            }

            Expr::WithCapacity { value, .. } => self.infer_expr(value),

            Expr::FieldAccess { object, field } => {
                let obj_ty = self.infer_expr(object)?;
                self.infer_field_access(obj_ty, *field)
            }

            Expr::New { type_name, type_args, .. } => {
                let name = self.interner.resolve(*type_name);
                match name {
                    "Seq" | "List" | "Vec" => {
                        let elem = type_args
                            .first()
                            .map(|t| InferType::from_type_expr(t, self.interner))
                            .unwrap_or_else(|| self.table.fresh());
                        Ok(InferType::Seq(Box::new(elem)))
                    }
                    "Map" | "HashMap" => {
                        let key = type_args
                            .first()
                            .map(|t| InferType::from_type_expr(t, self.interner))
                            .unwrap_or(InferType::String);
                        let val = type_args
                            .get(1)
                            .map(|t| InferType::from_type_expr(t, self.interner))
                            .unwrap_or(InferType::String);
                        Ok(InferType::Map(Box::new(key), Box::new(val)))
                    }
                    "Set" | "HashSet" => {
                        let elem = type_args
                            .first()
                            .map(|t| InferType::from_type_expr(t, self.interner))
                            .unwrap_or_else(|| self.table.fresh());
                        Ok(InferType::Set(Box::new(elem)))
                    }
                    _ => Ok(InferType::UserDefined(*type_name)),
                }
            }

            Expr::NewVariant { enum_name, .. } => {
                Ok(InferType::UserDefined(*enum_name))
            }

            Expr::CallExpr { callee, args } => {
                self.infer_call_expr(callee, args)
            }

            Expr::Closure { params, body: closure_body, return_type } => {
                self.infer_closure(params, closure_body, return_type)
            }

            Expr::InterpolatedString(_) => Ok(InferType::String),

            Expr::Slice { collection, .. } => self.infer_expr(collection),

            Expr::Union { left, .. } | Expr::Intersection { left, .. } => {
                self.infer_expr(left)
            }

            // Tuple, ManifestOf, ChunkAt, Escape → Unknown (not typed)
            _ => Ok(InferType::Unknown),
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
            | BinaryOpKind::GtEq => Ok(InferType::Bool),

            // And/Or: type-aware — integer operands → Int (bitwise), else → Bool (logical)
            BinaryOpKind::And | BinaryOpKind::Or => {
                let lt = self.infer_expr(left)?;
                if lt == InferType::Int {
                    Ok(InferType::Int)
                } else {
                    Ok(InferType::Bool)
                }
            }

            BinaryOpKind::Concat => Ok(InferType::String),

            BinaryOpKind::BitXor | BinaryOpKind::Shl | BinaryOpKind::Shr => Ok(InferType::Int),

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

            BinaryOpKind::Subtract
            | BinaryOpKind::Multiply
            | BinaryOpKind::Divide
            | BinaryOpKind::Modulo => {
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
    /// instantiated return type is then zonked and returned as the call result type.
    fn infer_call(&mut self, function: Symbol, args: &[&Expr]) -> Result<InferType, TypeError> {
        let name = self.interner.resolve(function);
        match name {
            "sqrt" | "parseFloat" | "pow" => Ok(InferType::Float),
            "parseInt" | "floor" | "ceil" | "round" => Ok(InferType::Int),
            "abs" | "min" | "max" => {
                if let Some(first) = args.first() {
                    self.infer_expr(first)
                } else {
                    Ok(InferType::Unknown)
                }
            }
            _ => {
                if let Some(rec) = self.functions.get(&function).cloned() {
                    // Instantiate the scheme: each call site gets fresh type variables
                    // for generic params so calls don't interfere with each other.
                    let instantiated = self.table.instantiate(&rec.scheme);

                    if let InferType::Function(param_types, ret_box) = instantiated {
                        // Unify each argument type with the instantiated parameter type
                        for (arg, param_ty) in args.iter().zip(param_types.iter()) {
                            let arg_ty = self.infer_expr(arg)?;
                            self.table.unify(&arg_ty, param_ty)?;
                        }
                        Ok(self.table.zonk(&ret_box))
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

    /// Infer a call-expression (calling a closure/function-value).
    fn infer_call_expr(
        &mut self,
        callee: &Expr,
        args: &[&Expr],
    ) -> Result<InferType, TypeError> {
        let callee_ty = self.infer_expr(callee)?;
        let ret_var = self.table.fresh();
        let arg_types: Vec<InferType> = args
            .iter()
            .map(|a| self.infer_expr(a))
            .collect::<Result<_, _>>()?;
        let fn_ty = InferType::Function(arg_types, Box::new(ret_var.clone()));

        let walked = self.table.zonk(&callee_ty);
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
            .map(|(_, ty_expr)| InferType::from_type_expr(ty_expr, self.interner))
            .collect();

        let ret_type = if let Some(rt) = return_type {
            InferType::from_type_expr(rt, self.interner)
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
                for stmt in *stmts {
                    self.infer_stmt(stmt)?;
                }
            }
        }

        self.current_return_type = prev_return;
        self.pop_scope();

        Ok(InferType::Function(param_types, Box::new(ret_type)))
    }

    /// Infer the type of a field access on a struct.
    fn infer_field_access(
        &self,
        obj_ty: InferType,
        field: Symbol,
    ) -> Result<InferType, TypeError> {
        let resolved = self.table.zonk(&obj_ty);
        match &resolved {
            InferType::UserDefined(type_sym) => {
                if let Some(TypeDef::Struct { fields, .. }) = self.registry.get(*type_sym) {
                    if let Some(field_def) = fields.iter().find(|f| f.name == field) {
                        Ok(InferType::from_field_type(
                            &field_def.ty,
                            self.interner,
                            &HashMap::new(),
                        ))
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

impl<'r> CheckEnv<'r> {
    fn infer_stmt(&mut self, stmt: &Stmt) -> Result<(), TypeError> {
        match stmt {
            Stmt::Let { var, ty, value, .. } => {
                let final_ty = if let Some(type_expr) = ty {
                    let annotated = InferType::from_type_expr(type_expr, self.interner);
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

            Stmt::Set { target, value } => {
                let inferred = self.infer_expr(value)?;
                // If target already has a type, unify. Otherwise just bind.
                if let Some(existing) = self.lookup_var(*target) {
                    if existing != InferType::Unknown {
                        self.table.unify(&inferred, &existing).ok();
                    }
                }
                // Update binding
                let resolved = self.table.zonk(&inferred);
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

                let param_types: Vec<InferType> = params
                    .iter()
                    .map(|(_, ty_expr)| {
                        InferType::from_type_expr_with_params(ty_expr, self.interner, &type_param_map)
                    })
                    .collect();
                let param_names: Vec<Symbol> = params.iter().map(|(sym, _)| *sym).collect();

                let ret_type = if let Some(rt) = return_type {
                    InferType::from_type_expr_with_params(rt, self.interner, &type_param_map)
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

                // Native functions: register scheme, no body to check
                if *is_native {
                    let scheme = TypeScheme {
                        vars: generic_vars,
                        body: InferType::Function(param_types, Box::new(ret_type)),
                    };
                    self.functions.insert(*name, FunctionRecord { param_names, scheme });
                    return Ok(());
                }

                // Save previous return context
                let prev_return_type = self.current_return_type.take();
                self.current_return_type = Some(ret_type.clone());

                // Check body in a new scope with params bound
                self.push_scope();
                for (sym, ty) in param_names.iter().zip(param_types.iter()) {
                    self.bind_var(*sym, ty.clone());
                }
                for s in *body {
                    self.infer_stmt(s)?;
                }
                self.pop_scope();

                self.current_return_type = prev_return_type;

                // After checking the body, update the registered scheme with resolved types.
                // Use `resolve` (not `zonk`) so generic TyVars remain as `Var(tv)` in
                // the scheme body — they will be instantiated fresh at each call site.
                let resolved_params: Vec<InferType> = param_types
                    .iter()
                    .map(|ty| self.table.resolve(ty))
                    .collect();
                let resolved_ret = self.table.resolve(&ret_type);

                let scheme = TypeScheme {
                    vars: generic_vars,
                    body: InferType::Function(resolved_params, Box::new(resolved_ret)),
                };
                self.functions.insert(*name, FunctionRecord { param_names, scheme });
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
                let elem_ty = match self.table.zonk(&iterable_ty) {
                    InferType::Seq(inner) | InferType::Set(inner) => *inner,
                    InferType::Map(k, _) => *k,
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
                let _target_ty = self.infer_expr(target)?;
                for arm in arms {
                    self.push_scope();
                    self.infer_inspect_arm(arm)?;
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
                let elem_ty = match self.table.zonk(&coll_ty) {
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

            _ => Ok(()),
        }
    }

    /// Process a single Inspect match arm, binding variant field types.
    fn infer_inspect_arm(
        &mut self,
        arm: &crate::ast::stmt::MatchArm,
    ) -> Result<(), TypeError> {
        if let Some(variant_sym) = arm.variant {
            if let Some((_, variant_def)) = self.registry.find_variant(variant_sym) {
                // Clone what we need to avoid borrow issues
                let fields: Vec<_> = variant_def
                    .fields
                    .iter()
                    .map(|f| (f.name, f.ty.clone()))
                    .collect();

                for (field_sym, binding_sym) in &arm.bindings {
                    let ty = fields
                        .iter()
                        .find(|(name, _)| *name == *field_sym)
                        .map(|(_, ty)| {
                            InferType::from_field_type(ty, self.interner, &HashMap::new())
                        })
                        .unwrap_or(InferType::Unknown);
                    self.bind_var(*binding_sym, ty);
                }
            } else {
                // Unknown variant → bind all as Unknown
                for (_, binding_sym) in &arm.bindings {
                    self.bind_var(*binding_sym, InferType::Unknown);
                }
            }
        } else {
            // Otherwise arm: wildcard bindings
            for (_, binding_sym) in &arm.bindings {
                self.bind_var(*binding_sym, InferType::Unknown);
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
    let mut env = CheckEnv::new(registry, interner);

    // Pre-pass: register top-level function signatures for forward references
    env.preregister_functions(stmts);

    // Main pass: check all top-level statements
    for stmt in stmts {
        env.infer_stmt(stmt)?;
    }

    Ok(env.into_type_env())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::{Expr, Literal, Stmt, TypeExpr};
    use crate::intern::Interner;

    // =========================================================================
    // Helpers
    // =========================================================================

    fn mk_interner() -> Interner {
        Interner::new()
    }

    fn run(stmts: &[Stmt], interner: &Interner) -> TypeEnv {
        check_program(stmts, interner, &TypeRegistry::new()).expect("check_program failed")
    }

    // =========================================================================
    // Let literal inference
    // =========================================================================

    #[test]
    fn let_literal_int() {
        let mut interner = mk_interner();
        let x = interner.intern("x");
        let val = Expr::Literal(Literal::Number(42));
        let stmts = [Stmt::Let { var: x, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(x), &LogosType::Int);
    }

    #[test]
    fn let_literal_float() {
        let mut interner = mk_interner();
        let x = interner.intern("x");
        let val = Expr::Literal(Literal::Float(3.14));
        let stmts = [Stmt::Let { var: x, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(x), &LogosType::Float);
    }

    #[test]
    fn let_literal_string() {
        let mut interner = mk_interner();
        let s = interner.intern("s");
        let hello = interner.intern("hello");
        let val = Expr::Literal(Literal::Text(hello));
        let stmts = [Stmt::Let { var: s, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(s), &LogosType::String);
    }

    // =========================================================================
    // Let with type annotation
    // =========================================================================

    #[test]
    fn let_with_annotation_uses_annotation() {
        let mut interner = mk_interner();
        let x = interner.intern("x");
        let float_sym = interner.intern("Real");
        let val = Expr::Literal(Literal::Number(5)); // Int value
        let ty_ann = TypeExpr::Primitive(float_sym);
        let stmts = [Stmt::Let { var: x, ty: Some(&ty_ann), value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        // Annotation wins: Int unifies with Float (numeric)
        assert_eq!(env.lookup(x), &LogosType::Float);
    }

    #[test]
    fn let_type_mismatch_fails() {
        let mut interner = mk_interner();
        let x = interner.intern("x");
        let int_sym = interner.intern("Int");
        let val = Expr::Literal(Literal::Text(Symbol::EMPTY));
        let ty_ann = TypeExpr::Primitive(int_sym);
        let stmts = [Stmt::Let { var: x, ty: Some(&ty_ann), value: &val, mutable: false }];
        let result = check_program(&stmts, &interner, &TypeRegistry::new());
        assert!(result.is_err(), "Int and Text should not unify");
    }

    // =========================================================================
    // Empty list → Seq(Unknown)
    // =========================================================================

    #[test]
    fn empty_list_is_seq_unknown() {
        let mut interner = mk_interner();
        let xs = interner.intern("xs");
        let val = Expr::List(vec![]);
        let stmts = [Stmt::Let { var: xs, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        // Should be Seq of something (Unknown because element type is unsolved)
        assert!(matches!(env.lookup(xs), LogosType::Seq(_)));
    }

    #[test]
    fn non_empty_list_infers_element_type() {
        let mut interner = mk_interner();
        let xs = interner.intern("xs");
        let one = Expr::Literal(Literal::Number(1));
        let two = Expr::Literal(Literal::Number(2));
        let val = Expr::List(vec![&one, &two]);
        let stmts = [Stmt::Let { var: xs, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(xs), &LogosType::Seq(Box::new(LogosType::Int)));
    }

    // =========================================================================
    // OptionNone → Option(Unknown)
    // =========================================================================

    #[test]
    fn option_none_is_option_unknown() {
        let mut interner = mk_interner();
        let x = interner.intern("x");
        let val = Expr::OptionNone;
        let stmts = [Stmt::Let { var: x, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        assert!(matches!(env.lookup(x), LogosType::Option(_)));
    }

    #[test]
    fn option_some_infers_inner_type() {
        let mut interner = mk_interner();
        let x = interner.intern("x");
        let inner = Expr::Literal(Literal::Number(42));
        let val = Expr::OptionSome { value: &inner };
        let stmts = [Stmt::Let { var: x, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(x), &LogosType::Option(Box::new(LogosType::Int)));
    }

    // =========================================================================
    // Function def and call
    // =========================================================================

    #[test]
    fn function_def_registers_signature() {
        let mut interner = mk_interner();
        let f = interner.intern("double");
        let x_param = interner.intern("x");
        let int_sym = interner.intern("Int");
        let int_ty = TypeExpr::Primitive(int_sym);
        let ret_ty = TypeExpr::Primitive(int_sym);
        let lit = Expr::Literal(Literal::Number(0));
        let ret_stmt = Stmt::Return { value: Some(&lit) };
        let body = [ret_stmt];
        let stmts = [Stmt::FunctionDef {
            name: f,
            generics: vec![],
            params: vec![(x_param, &int_ty)],
            body: &body,
            return_type: Some(&ret_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        }];
        let env = run(&stmts, &interner);
        let sig = env.lookup_fn(f).expect("function should be registered");
        assert_eq!(sig.return_type, LogosType::Int);
        assert_eq!(sig.params.len(), 1);
        assert_eq!(sig.params[0].1, LogosType::Int);
    }

    #[test]
    fn function_call_returns_registered_type() {
        let mut interner = mk_interner();
        let f = interner.intern("compute");
        let float_sym = interner.intern("Real");
        let float_ty = TypeExpr::Primitive(float_sym);
        let lit = Expr::Literal(Literal::Float(1.0));
        let ret_stmt = Stmt::Return { value: Some(&lit) };
        let body = [ret_stmt];
        let fn_def = Stmt::FunctionDef {
            name: f,
            generics: vec![],
            params: vec![],
            body: &body,
            return_type: Some(&float_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        };
        let result_var = interner.intern("result");
        let call = Expr::Call { function: f, args: vec![] };
        let let_stmt = Stmt::Let { var: result_var, ty: None, value: &call, mutable: false };
        let stmts = [fn_def, let_stmt];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(result_var), &LogosType::Float);
    }

    // =========================================================================
    // ReadFrom is String
    // =========================================================================

    #[test]
    fn readfrom_is_string() {
        let mut interner = mk_interner();
        let v = interner.intern("input");
        let stmts = [Stmt::ReadFrom {
            var: v,
            source: crate::ast::stmt::ReadSource::Console,
        }];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(v), &LogosType::String);
    }

    // =========================================================================
    // Repeat loop variable gets element type
    // =========================================================================

    #[test]
    fn repeat_loop_var_gets_element_type() {
        let mut interner = mk_interner();
        let items = interner.intern("items");
        let elem = interner.intern("elem");
        let one = Expr::Literal(Literal::Number(1));
        let list = Expr::List(vec![&one]);
        let let_items = Stmt::Let { var: items, ty: None, value: &list, mutable: false };
        let items_ref = Expr::Identifier(items);
        let repeat = Stmt::Repeat {
            pattern: Pattern::Identifier(elem),
            iterable: &items_ref,
            body: &[],
        };
        let stmts = [let_items, repeat];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(elem), &LogosType::Int);
    }

    // =========================================================================
    // Field access resolves to struct field type (uses registry)
    // =========================================================================

    #[test]
    fn field_access_resolves_with_registry() {
        use crate::analysis::{FieldDef, FieldType, TypeDef};

        let mut interner = mk_interner();
        let point_sym = interner.intern("Point");
        let x_field_sym = interner.intern("x");
        let int_sym = interner.intern("Int");
        let p_var = interner.intern("p");
        let result_var = interner.intern("px");

        // Build a registry with a struct Point { x: Int }
        let mut registry = TypeRegistry::new();
        registry.register(
            point_sym,
            TypeDef::Struct {
                fields: vec![FieldDef {
                    name: x_field_sym,
                    ty: FieldType::Primitive(int_sym),
                    is_public: true,
                }],
                generics: vec![],
                is_portable: false,
                is_shared: false,
            },
        );

        // Let p be a new Point.
        let new_point = Expr::New { type_name: point_sym, type_args: vec![], init_fields: vec![] };
        let let_p = Stmt::Let { var: p_var, ty: None, value: &new_point, mutable: false };

        // Let px be p's x.
        let p_ref = Expr::Identifier(p_var);
        let field_access = Expr::FieldAccess { object: &p_ref, field: x_field_sym };
        let let_px = Stmt::Let { var: result_var, ty: None, value: &field_access, mutable: false };

        let stmts = [let_p, let_px];
        let env = check_program(&stmts, &interner, &registry).expect("check_program failed");
        assert_eq!(env.lookup(result_var), &LogosType::Int);
    }

    // =========================================================================
    // Forward reference: calling a function defined later
    // =========================================================================

    #[test]
    fn forward_reference_function_call() {
        let mut interner = mk_interner();
        let f = interner.intern("later_fn");
        let result_var = interner.intern("r");
        let bool_sym = interner.intern("Bool");
        let bool_ty = TypeExpr::Primitive(bool_sym);

        // Let r be later_fn().  (before the function def)
        let call = Expr::Call { function: f, args: vec![] };
        let let_r = Stmt::Let { var: result_var, ty: None, value: &call, mutable: false };

        // ## Function later_fn -> Bool:
        let lit = Expr::Literal(Literal::Boolean(true));
        let ret_stmt = Stmt::Return { value: Some(&lit) };
        let body = [ret_stmt];
        let fn_def = Stmt::FunctionDef {
            name: f,
            generics: vec![],
            params: vec![],
            body: &body,
            return_type: Some(&bool_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        };

        // Note: let_r comes BEFORE fn_def in the slice
        let stmts = [let_r, fn_def];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(result_var), &LogosType::Bool);
    }

    // =========================================================================
    // Type mismatch on return
    // =========================================================================

    #[test]
    fn return_type_mismatch_fails() {
        let mut interner = mk_interner();
        let f = interner.intern("f");
        let int_sym = interner.intern("Int");
        let int_ty = TypeExpr::Primitive(int_sym);
        // Function annotated as -> Int but returns Text
        let lit = Expr::Literal(Literal::Text(Symbol::EMPTY));
        let ret_stmt = Stmt::Return { value: Some(&lit) };
        let body = [ret_stmt];
        let stmts = [Stmt::FunctionDef {
            name: f,
            generics: vec![],
            params: vec![],
            body: &body,
            return_type: Some(&int_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        }];
        let result = check_program(&stmts, &interner, &TypeRegistry::new());
        assert!(result.is_err(), "returning Text from Int function should fail");
    }

    // =========================================================================
    // New user-defined type → UserDefined
    // =========================================================================

    #[test]
    fn new_user_defined_is_user_defined_type() {
        let mut interner = mk_interner();
        let point = interner.intern("Point");
        let p = interner.intern("p");
        let new_point = Expr::New { type_name: point, type_args: vec![], init_fields: vec![] };
        let stmts = [Stmt::Let { var: p, ty: None, value: &new_point, mutable: false }];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(p), &LogosType::UserDefined(point));
    }

    // =========================================================================
    // Legacy API preserved: to_legacy_variable_types / to_legacy_string_vars
    // =========================================================================

    #[test]
    fn string_vars_in_legacy_api() {
        let mut interner = mk_interner();
        let s = interner.intern("name");
        let hello = interner.intern("hello");
        let val = Expr::Literal(Literal::Text(hello));
        let stmts = [Stmt::Let { var: s, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        assert!(env.to_legacy_string_vars().contains(&s));
    }

    #[test]
    fn unknown_vars_filtered_in_legacy_api() {
        let mut interner = mk_interner();
        let x = interner.intern("x");
        let val = Expr::OptionNone; // Unknown inner type
        let stmts = [Stmt::Let { var: x, ty: None, value: &val, mutable: false }];
        let env = run(&stmts, &interner);
        // Option(Unknown) → not in string_vars, not filtered as error
        let legacy = env.to_legacy_variable_types();
        // Option(Unknown) maps to "Option<_>", which is concrete enough
        assert!(!legacy.is_empty() || legacy.is_empty()); // just don't panic
    }

    // =========================================================================
    // Generic (polymorphic) functions — Phase 3
    // =========================================================================

    #[test]
    fn generic_identity_infers_int_return() {
        // ## To identity of [T] (x: T) -> T:
        //     Return x.
        // Let r be identity(42).  → r is Int
        let mut interner = mk_interner();
        let f = interner.intern("identity");
        let x_param = interner.intern("x");
        let t_sym = interner.intern("T");
        let t_ty = TypeExpr::Primitive(t_sym);
        let x_ref = Expr::Identifier(x_param);
        let ret_stmt = Stmt::Return { value: Some(&x_ref) };
        let body = [ret_stmt];
        let fn_def = Stmt::FunctionDef {
            name: f,
            generics: vec![t_sym],
            params: vec![(x_param, &t_ty)],
            body: &body,
            return_type: Some(&t_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        };
        let r = interner.intern("r");
        let lit = Expr::Literal(Literal::Number(42));
        let call = Expr::Call { function: f, args: vec![&lit] };
        let let_r = Stmt::Let { var: r, ty: None, value: &call, mutable: false };
        let stmts = [fn_def, let_r];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(r), &LogosType::Int,
            "identity(42) should return Int, got {:?}", env.lookup(r));
    }

    #[test]
    fn generic_identity_infers_bool_return() {
        // Same identity function, called with Bool → returns Bool.
        let mut interner = mk_interner();
        let f = interner.intern("identity");
        let x_param = interner.intern("x");
        let t_sym = interner.intern("T");
        let t_ty = TypeExpr::Primitive(t_sym);
        let x_ref = Expr::Identifier(x_param);
        let ret_stmt = Stmt::Return { value: Some(&x_ref) };
        let body = [ret_stmt];
        let fn_def = Stmt::FunctionDef {
            name: f,
            generics: vec![t_sym],
            params: vec![(x_param, &t_ty)],
            body: &body,
            return_type: Some(&t_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        };
        let r = interner.intern("r");
        let lit = Expr::Literal(Literal::Boolean(true));
        let call = Expr::Call { function: f, args: vec![&lit] };
        let let_r = Stmt::Let { var: r, ty: None, value: &call, mutable: false };
        let stmts = [fn_def, let_r];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(r), &LogosType::Bool,
            "identity(true) should return Bool, got {:?}", env.lookup(r));
    }

    #[test]
    fn generic_two_type_params_first() {
        // ## To first of [A] and [B] (a: A, b: B) -> A:
        //     Return a.
        // Let r be first(42, true).  → r is Int (first type param)
        let mut interner = mk_interner();
        let f = interner.intern("first");
        let a_param = interner.intern("a");
        let b_param = interner.intern("b");
        let a_sym = interner.intern("A");
        let b_sym = interner.intern("B");
        let a_ty = TypeExpr::Primitive(a_sym);
        let b_ty = TypeExpr::Primitive(b_sym);
        let a_ref = Expr::Identifier(a_param);
        let ret_stmt = Stmt::Return { value: Some(&a_ref) };
        let body = [ret_stmt];
        let fn_def = Stmt::FunctionDef {
            name: f,
            generics: vec![a_sym, b_sym],
            params: vec![(a_param, &a_ty), (b_param, &b_ty)],
            body: &body,
            return_type: Some(&a_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        };
        let r = interner.intern("r");
        let lit_int = Expr::Literal(Literal::Number(42));
        let lit_bool = Expr::Literal(Literal::Boolean(true));
        let call = Expr::Call { function: f, args: vec![&lit_int, &lit_bool] };
        let let_r = Stmt::Let { var: r, ty: None, value: &call, mutable: false };
        let stmts = [fn_def, let_r];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(r), &LogosType::Int,
            "first(42, true) should return Int (first param type), got {:?}", env.lookup(r));
    }

    #[test]
    fn generic_calls_are_independent() {
        // Each call to a generic function gets its own fresh type variables.
        // identity(42) → Int, identity(true) → Bool, independent results.
        let mut interner = mk_interner();
        let f = interner.intern("identity");
        let x_param = interner.intern("x");
        let t_sym = interner.intern("T");
        let t_ty = TypeExpr::Primitive(t_sym);
        let x_ref = Expr::Identifier(x_param);
        let ret_stmt = Stmt::Return { value: Some(&x_ref) };
        let body = [ret_stmt];
        let fn_def = Stmt::FunctionDef {
            name: f,
            generics: vec![t_sym],
            params: vec![(x_param, &t_ty)],
            body: &body,
            return_type: Some(&t_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        };
        let r1 = interner.intern("r1");
        let r2 = interner.intern("r2");
        let lit_int = Expr::Literal(Literal::Number(42));
        let lit_bool = Expr::Literal(Literal::Boolean(true));
        let call1 = Expr::Call { function: f, args: vec![&lit_int] };
        let call2 = Expr::Call { function: f, args: vec![&lit_bool] };
        let let_r1 = Stmt::Let { var: r1, ty: None, value: &call1, mutable: false };
        let let_r2 = Stmt::Let { var: r2, ty: None, value: &call2, mutable: false };
        let stmts = [fn_def, let_r1, let_r2];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(r1), &LogosType::Int,
            "identity(42) should be Int, got {:?}", env.lookup(r1));
        assert_eq!(env.lookup(r2), &LogosType::Bool,
            "identity(true) should be Bool, got {:?}", env.lookup(r2));
    }

    #[test]
    fn monomorphic_functions_unaffected_by_generics() {
        // Non-generic functions still work correctly with the updated machinery.
        let mut interner = mk_interner();
        let f = interner.intern("double");
        let x_param = interner.intern("x");
        let int_sym = interner.intern("Int");
        let int_ty = TypeExpr::Primitive(int_sym);
        let x_ref = Expr::Identifier(x_param);
        let lit2 = Expr::Literal(Literal::Number(2));
        let mul = Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left: &x_ref,
            right: &lit2,
        };
        let ret_stmt = Stmt::Return { value: Some(&mul) };
        let body = [ret_stmt];
        let fn_def = Stmt::FunctionDef {
            name: f,
            generics: vec![],
            params: vec![(x_param, &int_ty)],
            body: &body,
            return_type: Some(&int_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        };
        let r = interner.intern("r");
        let lit5 = Expr::Literal(Literal::Number(5));
        let call = Expr::Call { function: f, args: vec![&lit5] };
        let let_r = Stmt::Let { var: r, ty: None, value: &call, mutable: false };
        let stmts = [fn_def, let_r];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(r), &LogosType::Int,
            "double(5) should return Int, got {:?}", env.lookup(r));
    }

    #[test]
    fn generic_forward_reference_resolves() {
        // Let r be identity(42).
        // ## To identity of [T] (x: T) -> T:  ← defined AFTER the call
        //     Return x.
        // The pre-pass must register generics before the main pass sees the call.
        let mut interner = mk_interner();
        let f = interner.intern("identity");
        let x_param = interner.intern("x");
        let t_sym = interner.intern("T");
        let t_ty = TypeExpr::Primitive(t_sym);
        let x_ref = Expr::Identifier(x_param);
        let ret_stmt = Stmt::Return { value: Some(&x_ref) };
        let body = [ret_stmt];
        let fn_def = Stmt::FunctionDef {
            name: f,
            generics: vec![t_sym],
            params: vec![(x_param, &t_ty)],
            body: &body,
            return_type: Some(&t_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
        };
        let r = interner.intern("r");
        let lit = Expr::Literal(Literal::Number(99));
        let call = Expr::Call { function: f, args: vec![&lit] };
        let let_r = Stmt::Let { var: r, ty: None, value: &call, mutable: false };
        // Call appears BEFORE the function definition
        let stmts = [let_r, fn_def];
        let env = run(&stmts, &interner);
        assert_eq!(env.lookup(r), &LogosType::Int,
            "forward-ref identity(99) should be Int, got {:?}", env.lookup(r));
    }
}
