//! Rust code generation from kernel terms.
//!
//! Converts verified kernel terms to executable Rust source code while
//! preserving the structural properties of the proof.
//!
//! # Code Generation Strategy
//!
//! | Kernel Term | Rust Output |
//! |-------------|-------------|
//! | Inductive | `enum Name { Ctor(Args...) }` |
//! | Fix + Lambda | `fn name(args) -> Ret { body }` |
//! | Lambda | `fn name(args) -> Ret { body }` |
//! | Const | `const NAME: Ty = val;` |
//! | Match | `match disc { Name::Ctor(vars) => body }` |
//! | App | `func(arg)` |
//!
//! # Recursive Types
//!
//! Recursive inductive types (like `Nat`) have their recursive fields
//! wrapped in `Box<T>` to ensure finite size:
//!
//! ```text
//! Kernel:  Inductive Nat := Zero | Succ (Nat)
//! Rust:    enum Nat { Zero, Succ(Box<Nat>) }
//! ```

use super::error::ExtractError;
use crate::kernel::{Context, Literal, Term};
use std::collections::{HashMap, HashSet};

/// Context for code generation within a term.
struct TermGenCtx<'a> {
    /// The name of the definition being emitted
    def_name: &'a str,
    /// The name of the recursive reference (from Fix)
    rec_name: &'a str,
    /// Variables that need to be dereferenced (boxed in match patterns)
    deref_vars: &'a HashSet<String>,
    /// Variables referenced more than once in the body. The kernel's semantics
    /// are non-linear (a value may be used any number of times), but Rust moves
    /// by default, so each use of such a variable is emitted as `x.clone()`.
    multi_use: &'a HashSet<String>,
    /// Kernel binder name → the Rust identifier it is emitted as, for the binders
    /// whose kernel spelling `rustc` would reject. See [`rust_locals`]. Keyed by the
    /// KERNEL name, which is also how `deref_vars` and `multi_use` are keyed, so the
    /// renaming is a property of the OUTPUT and nothing upstream has to know about it.
    renames: &'a HashMap<String, String>,
}

impl TermGenCtx<'_> {
    /// The Rust identifier for a kernel local — [`rust_local`] against this walk's map.
    fn local(&self, name: &str) -> String {
        rust_local(self.renames, name)
    }
}

/// Code generator for extracting Rust from kernel terms.
pub struct CodeGen<'a> {
    ctx: &'a Context,
    output: String,
    emitted: HashSet<String>,
    /// Binder name → the INDUCTIVE type it was declared at, for the definition currently being
    /// emitted. Rebuilt per definition from its Pi telescope; empty outside one.
    binder_inductives: HashMap<String, String>,
}

impl<'a> CodeGen<'a> {
    /// Create a new code generator.
    pub fn new(ctx: &'a Context) -> Self {
        Self {
            ctx,
            output: String::new(),
            emitted: HashSet::new(),
            binder_inductives: HashMap::new(),
        }
    }

    /// Get the generated Rust code.
    pub fn finish(self) -> String {
        self.output
    }

    /// Emit an inductive type as a Rust enum.
    pub fn emit_inductive(&mut self, name: &str) -> Result<(), ExtractError> {
        if self.emitted.contains(name) {
            return Ok(());
        }
        self.emitted.insert(name.to_string());

        let ctors = distinct_constructors(self.ctx, name);
        if ctors.is_empty() {
            return Err(ExtractError::NotFound(name.to_string()));
        }

        // Type parameters of a polymorphic inductive (e.g. `A` in `MyList (A : Type)`)
        // are encoded as leading Π-binders on the sort and on each constructor type;
        // they become Rust generics, NOT data fields.
        let params = type_params(self.ctx, name);
        let generics = if params.is_empty() {
            String::new()
        } else {
            format!("<{}>", params.join(", "))
        };

        // Clone: non-linear use needs `.clone()`. Debug/PartialEq: the
        // self-verifying demo `main` prints values (`{:?}`) and `assert_eq!`s them
        // against the kernel-computed result.
        let rust_name = rust_global(self.ctx, name);
        self.output.push_str("#[derive(Clone, Debug, PartialEq)]\n");
        self.output
            .push_str(&format!("pub enum {}{} {{\n", rust_name, generics));
        for (ctor_name, ctor_ty) in &ctors {
            let fields = extract_ctor_fields(self.ctx, ctor_ty, name, params.len());
            let variant = rust_global(self.ctx, ctor_name);
            if fields.is_empty() {
                self.output.push_str(&format!("    {},\n", variant));
            } else {
                self.output
                    .push_str(&format!("    {}({}),\n", variant, fields.join(", ")));
            }
        }
        self.output.push_str("}\n\n");

        // For a non-generic inductive, also emit a CONSTRUCTOR API + Display impl so
        // IMPERATIVE LOGOS can build, pass, and `Show` proven values by name through
        // `use proven::*;` (e.g. `MSucc(MSucc(MZero))`) with no codegen changes:
        //   * nullary ctor  → `pub const Z: Nat = Nat::Z;`           (a value, not a fn)
        //   * n-ary ctor    → `pub fn S(a0: Nat) -> Nat { Nat::S(Box::new(a0)) }`
        //     (recursive fields boxed to match the tuple-variant shape)
        //   * `impl Display` via Debug, since imperative `Show` formats with `{}`.
        // Generic inductives are skipped (their values aren't constructible from the
        // imperative side without type arguments).
        if params.is_empty() {
            self.output.push_str(&format!(
                "impl std::fmt::Display for {n} {{\n    \
                 fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ write!(f, \"{{:?}}\", self) }}\n}}\n\n",
                n = rust_name,
            ));
            for (ctor_key, ctor_ty) in &ctors {
                let ctor_name = rust_global(self.ctx, ctor_key);
                // A ctor named after a std prelude value (`Some`/`None`/`Ok`/`Err`)
                // gets NO free wrapper: under `use proven::*;` it would silently shadow
                // the prelude in the imperative half. The qualified variant still works.
                if matches!(ctor_name.as_str(), "Some" | "None" | "Ok" | "Err") {
                    continue;
                }
                let cps = ctor_params(self.ctx, ctor_ty, 0, name);
                if cps.is_empty() {
                    self.output.push_str(&format!(
                        "pub const {c}: {n} = {n}::{c};\n",
                        c = ctor_name,
                        n = rust_name,
                    ));
                } else {
                    let sig: Vec<String> = cps
                        .iter()
                        .enumerate()
                        .map(|(i, (t, _))| format!("a{i}: {t}"))
                        .collect();
                    let args: Vec<String> = cps
                        .iter()
                        .enumerate()
                        .map(|(i, (_, rec))| {
                            if *rec {
                                format!("Box::new(a{i})")
                            } else {
                                format!("a{i}")
                            }
                        })
                        .collect();
                    self.output.push_str(&format!(
                        "pub fn {c}({sig}) -> {n} {{ {n}::{c}({args}) }}\n",
                        c = ctor_name,
                        n = rust_name,
                        sig = sig.join(", "),
                        args = args.join(", "),
                    ));
                }
            }
            self.output.push('\n');
        }
        Ok(())
    }

    /// Emit a definition as a Rust function or constant.
    pub fn emit_definition(&mut self, name: &str) -> Result<(), ExtractError> {
        if self.emitted.contains(name) {
            return Ok(());
        }
        self.emitted.insert(name.to_string());

        let body = self
            .ctx
            .get_definition_body(name)
            .ok_or_else(|| ExtractError::NotFound(name.to_string()))?;
        let ty = self
            .ctx
            .get_definition_type(name)
            .ok_or_else(|| ExtractError::NotFound(name.to_string()))?;

        // Check if it's a fixpoint (recursive function)
        if let Term::Fix {
            name: rec_name,
            body: fix_body,
        } = body
        {
            self.emit_fix_as_fn(name, rec_name, fix_body, ty)?;
        } else if is_lambda(body) {
            self.emit_lambda_as_fn(name, body, ty)?;
        } else {
            // A value definition. Emitted as a nullary function rather than a
            // `const` because constructor values allocate (`Box::new`), which is
            // not allowed in `const` context.
            self.emit_value_fn(name, body, ty)?;
        }
        Ok(())
    }

    /// Emit a fixpoint as a recursive Rust function.
    fn emit_fix_as_fn(
        &mut self,
        def_name: &str,
        rec_name: &str,
        fix_body: &Term,
        ty: &Term,
    ) -> Result<(), ExtractError> {
        let def_name = rust_global(self.ctx, def_name);
        let def_name = def_name.as_str();
        // Extract parameters from nested lambdas
        let (params, inner_body) = extract_lambda_params(fix_body);

        // Extract parameter types from the Pi type
        let param_types = extract_pi_params(ty);

        // Record each parameter's INDUCTIVE type for this definition, so a match whose
        // discriminant is a parameter can be resolved from its binder rather than from the
        // motive (which, for a non-dependent match, is the return type and not the scrutinee's).
        let binder_inductives: HashMap<String, String> = params
            .iter()
            .enumerate()
            .filter_map(|(i, (pname, _))| {
                let (_, t) = param_types.get(i)?;
                head_inductive(self.ctx, t).map(|n| (pname.clone(), n))
            })
            .collect();
        self.binder_inductives = binder_inductives;

        // Get return type
        let ret_ty = extract_return_type(ty);

        // One renaming for the WHOLE definition (signature and body), so a parameter
        // spelled unrepresentably in Rust is the same identifier in both.
        let renames = rust_locals(fix_body);

        // Build function signature
        self.output.push_str(&format!("pub fn {}(", def_name));
        for (i, (param_name, _)) in params.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            let param_ty = param_types
                .get(i)
                .map(|(_, t)| type_to_rust(self.ctx, t))
                .unwrap_or_else(|| "()".to_string());
            self.output.push_str(&format!(
                "{}: {}",
                rust_local(&renames, param_name),
                param_ty
            ));
        }
        self.output
            .push_str(&format!(") -> {} {{\n", type_to_rust(self.ctx, &ret_ty)));

        // Generate body, replacing recursive calls
        let body_code = self.term_to_rust(inner_body, def_name, rec_name, &renames);
        self.output.push_str(&format!("    {}\n", body_code));
        self.output.push_str("}\n\n");

        Ok(())
    }

    /// Emit a non-recursive lambda as a Rust function.
    fn emit_lambda_as_fn(
        &mut self,
        def_name: &str,
        body: &Term,
        ty: &Term,
    ) -> Result<(), ExtractError> {
        let def_name = rust_global(self.ctx, def_name);
        let def_name = def_name.as_str();
        // Extract parameters from nested lambdas
        let (params, inner_body) = extract_lambda_params(body);

        // Extract parameter types from the Pi type
        let param_types = extract_pi_params(ty);

        // Record each parameter's INDUCTIVE type for this definition, so a match whose
        // discriminant is a parameter can be resolved from its binder rather than from the
        // motive (which, for a non-dependent match, is the return type and not the scrutinee's).
        let binder_inductives: HashMap<String, String> = params
            .iter()
            .enumerate()
            .filter_map(|(i, (pname, _))| {
                let (_, t) = param_types.get(i)?;
                head_inductive(self.ctx, t).map(|n| (pname.clone(), n))
            })
            .collect();
        self.binder_inductives = binder_inductives;

        // Get return type
        let ret_ty = extract_return_type(ty);

        let renames = rust_locals(body);

        // Build function signature
        self.output.push_str(&format!("pub fn {}(", def_name));
        for (i, (param_name, _)) in params.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            let param_ty = param_types
                .get(i)
                .map(|(_, t)| type_to_rust(self.ctx, t))
                .unwrap_or_else(|| "()".to_string());
            self.output.push_str(&format!(
                "{}: {}",
                rust_local(&renames, param_name),
                param_ty
            ));
        }
        self.output
            .push_str(&format!(") -> {} {{\n", type_to_rust(self.ctx, &ret_ty)));

        // Generate body (no recursive name replacement)
        let body_code = self.term_to_rust(inner_body, def_name, "", &renames);
        self.output.push_str(&format!("    {}\n", body_code));
        self.output.push_str("}\n\n");

        Ok(())
    }

    /// Emit a value definition as a nullary function.
    ///
    /// `Definition one : Nat := Succ Zero` becomes `fn one() -> Nat { … }`. A
    /// function (not a `const`) because the body may allocate (`Box::new`).
    fn emit_value_fn(&mut self, name: &str, body: &Term, ty: &Term) -> Result<(), ExtractError> {
        let name = rust_global(self.ctx, name);
        let name = name.as_str();
        let ty_str = type_to_rust(self.ctx, ty);
        let body_str = self.term_to_rust(body, name, "", &rust_locals(body));
        self.output.push_str(&format!(
            "pub fn {}() -> {} {{\n    {}\n}}\n\n",
            name, ty_str, body_str
        ));
        Ok(())
    }

    /// Convert a kernel term to Rust code.
    fn term_to_rust(
        &self,
        term: &Term,
        def_name: &str,
        rec_name: &str,
        renames: &HashMap<String, String>,
    ) -> String {
        self.term_to_rust_forced(term, def_name, rec_name, &HashSet::new(), renames)
    }

    /// As [`Self::term_to_rust`], but `force_clone` names are `.clone()`d at every
    /// use even if they appear once — used when a value is shared across separately
    /// emitted sub-terms (e.g. both sides of a property `lhs == rhs`).
    fn term_to_rust_forced(
        &self,
        term: &Term,
        def_name: &str,
        rec_name: &str,
        force_clone: &HashSet<String>,
        renames: &HashMap<String, String>,
    ) -> String {
        let empty_deref = HashSet::new();
        // Variables used more than once must be cloned at each use (Rust moves by
        // default; the kernel's lambda calculus does not).
        let mut counts: HashMap<String, usize> = HashMap::new();
        count_vars(term, &mut counts);
        let mut multi_use: HashSet<String> = counts
            .into_iter()
            .filter(|(_, c)| *c > 1)
            .map(|(name, _)| name)
            .collect();
        multi_use.extend(force_clone.iter().cloned());
        let ctx = TermGenCtx {
            def_name,
            rec_name,
            deref_vars: &empty_deref,
            multi_use: &multi_use,
            renames,
        };
        self.term_to_rust_ctx(term, &ctx)
    }

    /// Convert a kernel term to Rust code with context for dereferencing.
    fn term_to_rust_ctx(&self, term: &Term, tctx: &TermGenCtx) -> String {
        match term {
            // A universe-polymorphic reference extracts by its name; universe arguments
            // carry no runtime content.
            Term::Const { name, .. } => rust_global(self.ctx, name),
            Term::Var(name) => {
                // Check if this is a reference to the recursive function
                if name == tctx.rec_name && !tctx.rec_name.is_empty() {
                    tctx.def_name.to_string()
                } else {
                    // Dereference boxed bindings from recursive match patterns.
                    let ident = tctx.local(name);
                    let base = if tctx.deref_vars.contains(name) {
                        format!("(*{})", ident)
                    } else {
                        ident
                    };
                    // Clone variables used more than once so non-linear use type-checks.
                    if tctx.multi_use.contains(name) {
                        format!("{}.clone()", base)
                    } else {
                        base
                    }
                }
            }
            Term::Global(name) => {
                // Check if it's a constructor
                if self.ctx.is_constructor(name) {
                    if let Some(ind) = self.ctx.constructor_inductive(name) {
                        return format!(
                            "{}::{}",
                            rust_global(self.ctx, ind),
                            rust_global(self.ctx, name)
                        );
                    }
                }
                // Check if it's the recursive reference
                if name == tctx.rec_name && !tctx.rec_name.is_empty() {
                    return tctx.def_name.to_string();
                }
                // A value definition is emitted as a nullary `fn`, so reference it
                // by calling it.
                if is_value_def(self.ctx, name) {
                    return format!("{}()", rust_global(self.ctx, name));
                }
                rust_global(self.ctx, name)
            }
            Term::App(_, _) => {
                // Collect the head function and all arguments
                let (head, args) = collect_app_chain(term);
                let args_strs: Vec<String> = args
                    .iter()
                    .map(|a| self.term_to_rust_ctx(a, tctx))
                    .collect();

                // Check if the head is a constructor
                if let Term::Global(name) = head {
                    if self.ctx.is_constructor(name) {
                        if let Some(ind) = self.ctx.constructor_inductive(name) {
                            // Erase the leading type arguments (they correspond to
                            // the inductive's type parameters) and box each field
                            // whose type is recursive.
                            let n_params = type_params(self.ctx, ind).len();
                            let value_args: &[String] = if args_strs.len() >= n_params {
                                &args_strs[n_params..]
                            } else {
                                &args_strs
                            };
                            let rec = ctor_field_recursion(
                                self.ctx,
                                self.ctx.get_global(name),
                                ind,
                                n_params,
                            );
                            let fields: Vec<String> = value_args
                                .iter()
                                .enumerate()
                                .map(|(i, a)| {
                                    if rec.get(i).copied().unwrap_or(false) {
                                        format!("Box::new({})", a)
                                    } else {
                                        a.clone()
                                    }
                                })
                                .collect();
                            let (ind, name) =
                                (rust_global(self.ctx, ind), rust_global(self.ctx, name));
                            if fields.is_empty() {
                                return format!("{}::{}", ind, name);
                            } else {
                                return format!("{}::{}({})", ind, name, fields.join(", "));
                            }
                        }
                    }
                }

                // Arithmetic builtins (`add`/`sub`/`mul`/`div`/`mod` : Int -> Int ->
                // Int) are opaque kernel declarations whose computational behavior IS
                // the Rust integer operator — their kernel laws (add_comm/add_assoc/
                // add_zero, …) are exactly `+`'s. Emit the operator so primitive proven
                // functions (`fun n => add n n`) extract to real, runnable Rust rather
                // than a call to an undefined `add`. CRITICAL: only the BUILTIN (a
                // bodyless declaration) maps to an operator — a user-defined function
                // that happens to be named `add` (e.g. Peano `add : Num -> Num -> Num`,
                // which HAS a body) is extracted as a real `fn` and must be CALLED.
                if let Term::Global(name) = head {
                    if let Some(op) = arith_builtin_operator(self.ctx, name) {
                        if args_strs.len() == 2 {
                            return format!("({} {} {})", args_strs[0], op, args_strs[1]);
                        }
                    }
                }

                // Check if head is a Var that should be renamed (recursive call)
                let head_str = if let Term::Var(name) = head {
                    if name == tctx.rec_name && !tctx.rec_name.is_empty() {
                        tctx.def_name.to_string()
                    } else {
                        tctx.local(name)
                    }
                } else {
                    self.term_to_rust_ctx(head, tctx)
                };

                // Regular function call with all arguments comma-separated
                format!("{}({})", head_str, args_strs.join(", "))
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                let param_ty = type_to_rust(self.ctx, param_type);
                let body_str = self.term_to_rust_ctx(body, tctx);
                format!("|{}: {}| {}", tctx.local(param), param_ty, body_str)
            }
            Term::Let {
                name, value, body, ..
            } => {
                let value_str = self.term_to_rust_ctx(value, tctx);
                let body_str = self.term_to_rust_ctx(body, tctx);
                format!(
                    "{{ let {} = {}; {} }}",
                    tctx.local(name),
                    value_str,
                    body_str
                )
            }
            Term::MutualFix { defs, index } => {
                // Extract the definition this occurrence denotes (best-effort: mutual recursion
                // is emitted as the selected body; a full extraction would hoist all defs).
                match defs.get(*index) {
                    Some((_, body)) => self.term_to_rust_ctx(body, tctx),
                    None => "()".to_string(),
                }
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                let disc_str = self.term_to_rust_ctx(discriminant, tctx);

                // Get the inductive type from the motive (λx:T. ReturnType)
                // The param_type of the motive lambda gives us the inductive
                let ind_name = self
                    .infer_inductive_from_motive(motive)
                    .or_else(|| self.infer_inductive_type(discriminant));

                let mut result = format!("match {} {{\n", disc_str);
                if let Some(ind) = &ind_name {
                    let ctors = distinct_constructors(self.ctx, ind);
                    let n_params = type_params(self.ctx, ind).len();

                    for (i, (ctor_name, ctor_ty)) in ctors.iter().enumerate() {
                        if i < cases.len() {
                            let case = &cases[i];
                            // Type parameters are erased, so they are not pattern fields.
                            let ctor_arity = count_ctor_args(ctor_ty).saturating_sub(n_params);

                            result.push_str(&format!(
                                "        {}::{}",
                                rust_global(self.ctx, ind),
                                rust_global(self.ctx, ctor_name)
                            ));
                            if ctor_arity > 0 {
                                // Generate pattern with bindings
                                let (bindings, case_body) = extract_case_bindings(case, ctor_arity);
                                result.push_str("(");
                                for (j, binding) in bindings.iter().enumerate() {
                                    if j > 0 {
                                        result.push_str(", ");
                                    }
                                    result.push_str(&tctx.local(binding));
                                }
                                result.push_str(")");

                                // Only bindings for BOXED fields are dereferenced in the
                                // body — a per-field mask (not all-or-nothing), so a
                                // non-recursive field (e.g. an `Int` beside a recursive
                                // field) is not wrongly `(*x)`-derefed, and mutual-
                                // recursion's boxed cross-fields ARE.
                                let box_mask =
                                    ctor_field_recursion(self.ctx, Some(ctor_ty), ind, n_params);
                                let case_deref_vars: HashSet<String> = bindings
                                    .iter()
                                    .enumerate()
                                    .filter(|(j, _)| box_mask.get(*j).copied().unwrap_or(false))
                                    .map(|(_, b)| b.clone())
                                    .collect();
                                let case_tctx = TermGenCtx {
                                    def_name: tctx.def_name,
                                    rec_name: tctx.rec_name,
                                    deref_vars: &case_deref_vars,
                                    multi_use: tctx.multi_use,
                                    renames: tctx.renames,
                                };
                                let body_str = self.term_to_rust_ctx(&case_body, &case_tctx);
                                result.push_str(&format!(" => {},\n", body_str));
                            } else {
                                let case_str = self.term_to_rust_ctx(case, tctx);
                                result.push_str(&format!(" => {},\n", case_str));
                            }
                        }
                    }
                }
                result.push_str("    }");
                result
            }
            Term::Fix { name, body } => {
                // Inline fixpoints are tricky - for now, just extract the body
                let fix_tctx = TermGenCtx {
                    def_name: tctx.def_name,
                    rec_name: name,
                    deref_vars: tctx.deref_vars,
                    multi_use: tctx.multi_use,
                    renames: tctx.renames,
                };
                self.term_to_rust_ctx(body, &fix_tctx)
            }
            Term::Lit(lit) => match lit {
                Literal::Int(n) => format!("{}i64", n),
                Literal::BigInt(n) => format!("{}i128 /* bigint */", n),
                Literal::Nat(n) => format!("{}u64 /* nat */", n),
                Literal::Float(f) => format!("{}f64", f),
                Literal::Text(s) => format!("{:?}", s),
                Literal::Duration(nanos) => format!("{}i64 /* nanos */", nanos),
                Literal::Date(days) => format!("{}i32 /* days since epoch */", days),
                Literal::Moment(nanos) => format!("{}i64 /* nanos since epoch */", nanos),
            },
            Term::Pi { .. } => "/* type */".to_string(),
            Term::Sort(_) => "/* sort */".to_string(),
            Term::Hole => "_".to_string(), // Type placeholder
            // An annotation carries no runtime content: the term under it is what is extracted.
            Term::Annotated { term, .. } => self.term_to_rust_ctx(term, tctx),
        }
    }

    /// Extract the inductive type from a match motive.
    ///
    /// The motive is typically `λx:T. ReturnType` where T is the inductive.
    fn infer_inductive_from_motive(&self, motive: &Term) -> Option<String> {
        // ★ A MOTIVE COMES IN TWO SHAPES AND THIS KNEW ONLY ONE.
        //
        // The dependent form is `λx:T. ReturnType`, and the inductive is the binder's type. But a
        // NON-dependent match carries its motive as the return type alone — for `my_add` the kernel
        // hands us `Global("MyNat")` directly, with no lambda to look inside. This matched only the
        // lambda, returned `None` for the bare form, and the caller's `if let Some(ind)` then skipped
        // the entire arm loop and emitted `match n { }` — with `cases` sitting right there, populated.
        // Valid Rust to us, a non-exhaustive-match error in the code we hand the user. MEASURED at the
        // failure: `disc="n" ind_name=None cases=2 motive=Global("MyNat")`.
        //
        // Both spellings of a declared name are accepted (`Global(n)` and the universe-instantiated
        // `Const { name, .. }`), because a site asking "which declaration is this?" that knows only one
        // of them stops recognising the name the moment it is instantiated.
        // ONLY the dependent form names the discriminant's type. `λx:T. ReturnType` binds the
        // inductive as `T`; a NON-dependent match carries its motive as the return type ALONE, and
        // that is a different type whenever the function changes type — `is_zero : MyNat -> MyBool`
        // has motive `Global("MyBool")` over a discriminant of `MyNat`. Reading the bare motive as
        // the inductive emits `MyBool` arms for a `MyNat` scrutinee, which is why the bare case is
        // resolved from the DISCRIMINANT's binder instead (see `binder_inductives`).
        if let Term::Lambda { param_type, .. } = motive {
            if let Some(name) = global_or_const_name(param_type.as_ref()) {
                if self.ctx.is_inductive(name) {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    /// Try to infer the inductive type from a term.
    fn infer_inductive_type(&self, term: &Term) -> Option<String> {
        match term {
            Term::Var(name) => {
                // A bare `Var` carries no type, but the binder that introduced it does: the
                // enclosing definition's Pi telescope gives each parameter's type, and a
                // non-dependent match's discriminant is one of them. Without this the only
                // remaining source is the motive, which for a non-dependent match is the RETURN
                // type — a different type, and emitting its constructors produced arms that did
                // not typecheck against the scrutinee.
                self.binder_inductives.get(name).cloned()
            }
            Term::Global(name) | Term::Const { name, .. } => {
                if self.ctx.is_constructor(name) {
                    self.ctx.constructor_inductive(name).map(|s| s.to_string())
                } else if self.ctx.is_inductive(name) {
                    Some(name.clone())
                } else {
                    None
                }
            }
            Term::App(f, _) => self.infer_inductive_type(f),
            Term::Hole => None, // Holes are type placeholders
            _ => None,
        }
    }
}

/// A kernel binder name → the Rust identifier it is emitted as, for the binders of `term`
/// whose kernel spelling `rustc` would reject. Names Rust already accepts are ABSENT, so
/// ordinary code is emitted byte-identically and only an unrepresentable name is touched.
///
/// The two name spaces are not the same one. The kernel's `scan_name` accepts a quoted name,
/// and its own passes mint names that are deliberately outside the ordinary identifier space
/// so they cannot be captured: the equation compiler's field binders are `#fld0`, `#fld1`, …
/// (`interface::match_compile`), a self-recursive definition's binder carries `✧`, an
/// induction hypothesis' variable carries `#eq`. Emitting one verbatim produces Rust that does
/// not PARSE — `MyNat::MSucc(#fld0) => …` — which is the one failure mode extraction must not
/// have, because output that does not compile is not extraction.
///
/// The rename is collision-free by CHECK, not by hope. A character-level escape alone is not:
/// `#fld0` and a user's own `_fld0` both escape to `_fld0`, and the two would silently become
/// one variable — a term that compiles and computes something else. So every name the term
/// can spell is collected first, and a candidate is stepped with a numeric suffix until it is
/// outside that set and outside the names already handed out. This is the same discipline
/// `match_compile::State::next_name` uses on the other side of the wall.
fn rust_locals(term: &Term) -> HashMap<String, String> {
    let mut spelled = HashSet::new();
    let mut binders = Vec::new();
    collect_names(term, &mut spelled, &mut binders);

    let mut renames = HashMap::new();
    let mut handed_out: HashSet<String> = HashSet::new();
    for name in binders {
        if is_rust_ident(&name) || renames.contains_key(&name) {
            continue;
        }
        let base = escape_to_ident(&name);
        let mut candidate = base.clone();
        let mut n = 2;
        while spelled.contains(&candidate) || handed_out.contains(&candidate) {
            candidate = format!("{}_{}", base, n);
            n += 1;
        }
        handed_out.insert(candidate.clone());
        renames.insert(name, candidate);
    }
    renames
}

/// The Rust identifier a kernel local is emitted as, given a [`rust_locals`] map.
fn rust_local(renames: &HashMap<String, String>, name: &str) -> String {
    renames
        .get(name)
        .cloned()
        .unwrap_or_else(|| name.to_string())
}

/// Every name `term` SPELLS (into `spelled`) and every name it BINDS (into `binders`, in
/// traversal order so the renaming is deterministic). Exhaustive over [`Term`]: a variant
/// added without an arm here would leak an unrepresentable binder into the output.
fn collect_names(term: &Term, spelled: &mut HashSet<String>, binders: &mut Vec<String>) {
    match term {
        Term::Var(name) => {
            spelled.insert(name.clone());
        }
        Term::Global(name) | Term::Const { name, .. } => {
            spelled.insert(name.clone());
        }
        Term::App(f, a) => {
            collect_names(f, spelled, binders);
            collect_names(a, spelled, binders);
        }
        Term::Lambda {
            param,
            param_type,
            body,
        }
        | Term::Pi {
            param,
            param_type,
            body_type: body,
        } => {
            spelled.insert(param.clone());
            binders.push(param.clone());
            collect_names(param_type, spelled, binders);
            collect_names(body, spelled, binders);
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            spelled.insert(name.clone());
            binders.push(name.clone());
            collect_names(ty, spelled, binders);
            collect_names(value, spelled, binders);
            collect_names(body, spelled, binders);
        }
        Term::Fix { name, body } => {
            spelled.insert(name.clone());
            collect_names(body, spelled, binders);
        }
        Term::MutualFix { defs, .. } => {
            for (name, body) in defs {
                spelled.insert(name.clone());
                collect_names(body, spelled, binders);
            }
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_names(discriminant, spelled, binders);
            collect_names(motive, spelled, binders);
            for case in cases {
                collect_names(case, spelled, binders);
            }
        }
        Term::Annotated { term, .. } => collect_names(term, spelled, binders),
        Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// Whether `name` can be emitted as a Rust identifier as written. ASCII only (the kernel's
/// marks are not `XID_Continue` anyway, and a conservative answer only ever renames), and a
/// Rust KEYWORD is not one either — `Definition f := fun match : Nat => match` is a perfectly
/// good LOGOS declaration and `fn f(match: Nat)` is not Rust.
fn is_rust_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !crate::codegen::is_rust_keyword(name)
}

/// The character-level part of the rename: every character Rust will not take becomes `_`, a
/// leading digit gains a `_`, and a keyword gains a trailing `_`. [`rust_locals`] is what makes
/// the result unique; this only makes it well-formed.
fn escape_to_ident(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if crate::codegen::is_rust_keyword(&out) {
        out.push('_');
    }
    out
}

/// Count how many times each variable is referenced in a term.
///
/// Used to decide which bindings must be `.clone()`d at their use sites: a value
/// used more than once cannot be moved each time under Rust's ownership rules.
fn count_vars(term: &Term, counts: &mut HashMap<String, usize>) {
    match term {
        Term::Const { .. } => {}
        Term::Var(name) => {
            *counts.entry(name.clone()).or_insert(0) += 1;
        }
        Term::App(f, a) => {
            count_vars(f, counts);
            count_vars(a, counts);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            count_vars(param_type, counts);
            count_vars(body, counts);
        }
        Term::Let {
            ty, value, body, ..
        } => {
            count_vars(ty, counts);
            count_vars(value, counts);
            count_vars(body, counts);
        }
        Term::MutualFix { defs, .. } => {
            for (_, body) in defs {
                count_vars(body, counts);
            }
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            count_vars(param_type, counts);
            count_vars(body_type, counts);
        }
        Term::Fix { body, .. } => count_vars(body, counts),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            count_vars(discriminant, counts);
            count_vars(motive, counts);
            for case in cases {
                count_vars(case, counts);
            }
        }
        Term::Annotated { term, .. } => count_vars(term, counts),
        Term::Sort(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// Emit a (normalized) data-value `Term` as a Rust expression — used by the
/// self-verifying demo `main` to write the kernel-computed expected result.
pub fn emit_value(ctx: &Context, term: &Term) -> String {
    CodeGen::new(ctx).term_to_rust(term, "", "", &rust_locals(term))
}

/// Emit a runnable property check `fn check_<name>(params) -> bool { … }` from a
/// theorem's PROPOSITION (its kernel type), or `None` if the statement isn't a
/// finite, executable property over extractable functions. A proven theorem like
/// `∀n. Eq Nat (add Zero n) n` becomes `fn check_…(n: Nat) -> bool { add(Nat::Zero, n) == n }`.
pub fn emit_property_check(ctx: &Context, name: &str, prop: &Term) -> Option<String> {
    // Peel `forall` (Pi) binders into parameters over data types.
    let mut params: Vec<(String, String)> = Vec::new();
    let mut cur = prop;
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
        if !prop_type_is_data(ctx, param_type) {
            return None; // quantifies over Prop/opaque — not runnable
        }
        params.push((param.clone(), type_to_rust(ctx, param_type)));
        cur = body_type;
    }
    if params.is_empty() {
        // No quantifier — only worthwhile when it's a closed equation; allow it.
    }
    let param_names: HashSet<String> = params.iter().map(|(n, _)| n.clone()).collect();
    // One renaming for the whole proposition, so a quantified binder the kernel spells
    // unrepresentably (`x#eq`, a fixpoint's `f✧`) is the SAME identifier in the signature
    // and on both sides of the equation.
    let renames = rust_locals(prop);
    let body = emit_prop_body(ctx, cur, &param_names, &renames)?;
    let sig: Vec<String> = params
        .iter()
        .map(|(n, t)| format!("{}: {}", rust_local(&renames, n), t))
        .collect();
    Some(format!(
        "/// Runnable property from the proven theorem `{}`.\n\
         pub fn check_{}({}) -> bool {{\n    {}\n}}\n\n",
        ctx.written_name(name),
        rust_global(ctx, name),
        sig.join(", "),
        body
    ))
}

/// A concrete data type (mapped primitive, or user inductive with constructors).
fn prop_type_is_data(ctx: &Context, ty: &Term) -> bool {
    match ty {
        Term::Global(n) => {
            matches!(
                n.as_str(),
                "Int" | "Float" | "Text" | "Bool" | "Duration" | "Date" | "Moment"
            ) || (ctx.is_inductive(n) && !ctx.get_constructors(n).is_empty())
        }
        Term::App(_, _) => {
            let (h, args) = collect_app_chain(ty);
            prop_type_is_data(ctx, h) && args.iter().all(|a| prop_type_is_data(ctx, a))
        }
        _ => false,
    }
}

/// A proposition body → a Rust `bool` expression (`Eq`→`==`, `And`/`Or`/`Not`),
/// or `None` if not a finitely-checkable shape.
fn emit_prop_body(
    ctx: &Context,
    p: &Term,
    params: &HashSet<String>,
    renames: &HashMap<String, String>,
) -> Option<String> {
    let (head, args) = collect_app_chain(p);
    if let Term::Global(h) = head {
        match (h.as_str(), args.len()) {
            ("Eq", 3) => {
                // `Eq A l r` is runnable only when `A` is DATA — a type with a Rust value and
                // a `==`. It is not for a FUNCTION (`add = λn. λm. …`, the definitional
                // equation every definition now carries) and not for a PROOF
                // (`lemma = λn. refl …`, the equation of a proof term): the first emits a
                // comparison of an `fn` item against a closure, the second a comparison of
                // proof objects that have no runtime form at all. Both are true statements
                // with no finite check, which is exactly what this function returns `None` for.
                if !prop_type_is_data(ctx, args[0]) {
                    return None;
                }
                let lhs = emit_checkable_term(ctx, args[1], params, renames)?;
                let rhs = emit_checkable_term(ctx, args[2], params, renames)?;
                return Some(format!("({} == {})", lhs, rhs));
            }
            ("And", 2) => {
                return Some(format!(
                    "({} && {})",
                    emit_prop_body(ctx, args[0], params, renames)?,
                    emit_prop_body(ctx, args[1], params, renames)?
                ))
            }
            ("Or", 2) => {
                return Some(format!(
                    "({} || {})",
                    emit_prop_body(ctx, args[0], params, renames)?,
                    emit_prop_body(ctx, args[1], params, renames)?
                ))
            }
            ("Not", 1) => {
                return Some(format!(
                    "(!{})",
                    emit_prop_body(ctx, args[0], params, renames)?
                ))
            }
            _ => {}
        }
    }
    None
}

/// One side of an equation → Rust, requiring it reference only extractable things
/// (so the check compiles), with quantified `params` cloned (shared across sides).
fn emit_checkable_term(
    ctx: &Context,
    term: &Term,
    params: &HashSet<String>,
    renames: &HashMap<String, String>,
) -> Option<String> {
    let mut refs = Vec::new();
    super::collector::collect_globals(term, &mut refs);
    if !refs
        .iter()
        .all(|g| g == "_" || crate::extraction::is_extractable(ctx, g))
    {
        return None;
    }
    Some(CodeGen::new(ctx).term_to_rust_forced(term, "", "", params, renames))
}

/// Constructors of an inductive, de-duplicated by name (keeping the first of each).
///
/// Redefining an inductive that already exists (e.g. the StandardLibrary `Nat`)
/// appends its constructors again to the registration order; emitting those
/// verbatim would produce an enum with duplicate variants. Dedup by name so the
/// extracted enum is valid.
/// The declared NAME a term refers to, however the kernel happens to spell it.
///
/// `Global(n)` and `Const { name, .. }` are the SAME reference — the second simply carries a universe
/// instance. Every site that asks "which declaration is this?" must accept both, or it silently stops
/// recognising a name the moment it is instantiated.
fn global_or_const_name(t: &Term) -> Option<&str> {
    match t {
        Term::Global(n) => Some(n.as_str()),
        Term::Const { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

/// The INDUCTIVE a type expression is headed by, if any.
///
/// A parameter's declared type is either the inductive itself (`MyNat`) or an application of one
/// (`TList A`), so the head is what names the type. Type ARGUMENTS are erased in extraction, which
/// is why only the head matters here.
fn head_inductive(ctx: &Context, ty: &Term) -> Option<String> {
    let mut cur = ty;
    loop {
        match cur {
            Term::App(f, _) => cur = f.as_ref(),
            other => {
                let n = global_or_const_name(other)?;
                return ctx.is_inductive(n).then(|| n.to_string());
            }
        }
    }
}

fn distinct_constructors<'a>(ctx: &'a Context, inductive: &str) -> Vec<(&'a str, &'a Term)> {
    let mut seen = HashSet::new();
    ctx.get_constructors(inductive)
        .into_iter()
        .filter(|(n, _)| seen.insert(n.to_string()))
        .collect()
}

/// The type parameter names of an inductive (e.g. `["A"]` for `MyList (A : Type)`).
///
/// They are encoded as leading Π-binders over a `Sort` on the inductive's own
/// type, so we read them off the inductive's sort.
fn type_params(ctx: &Context, inductive: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = match ctx.get_global(inductive) {
        Some(ty) => ty,
        None => return names,
    };
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = current
    {
        // Only leading `: Type` binders are type parameters (→ Rust generics).
        if matches!(param_type.as_ref(), Term::Sort(_)) {
            names.push(param.clone());
            current = body_type;
        } else {
            break;
        }
    }
    names
}

/// Extract a constructor's Rust field types, skipping the `skip` leading type
/// parameters and boxing any field whose type is recursive (mentions the
/// inductive) to keep the enum finitely sized.
/// Collect the inductive-type names referenced anywhere in a (field) type term.
fn collect_inductive_globals(ctx: &Context, term: &Term, out: &mut Vec<String>) {
    match term {
        Term::Global(n) => {
            if ctx.is_inductive(n) {
                out.push(n.clone());
            }
        }
        Term::App(f, a) => {
            collect_inductive_globals(ctx, f, out);
            collect_inductive_globals(ctx, a, out);
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            collect_inductive_globals(ctx, param_type, out);
            collect_inductive_globals(ctx, body_type, out);
        }
        _ => {}
    }
}

/// The inductive types referenced by a constructor's FIELDS (its Pi params after the
/// inductive's leading type parameters) — the out-edges of an inductive in the type
/// graph.
fn ctor_field_inductives(ctx: &Context, ctor_ty: &Term, skip: usize, out: &mut Vec<String>) {
    let mut cur = ctor_ty;
    for _ in 0..skip {
        if let Term::Pi { body_type, .. } = cur {
            cur = body_type;
        } else {
            break;
        }
    }
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        collect_inductive_globals(ctx, param_type, out);
        cur = body_type;
    }
}

/// Whether inductive `from` can reach `to` by following constructor-field references
/// (`X → Y` iff a constructor of `X` has a field referencing inductive `Y`). Self- and
/// mutual-recursion both show up as `reaches(T, T)` / mutual reachability.
fn inductive_reaches(ctx: &Context, from: &str, to: &str) -> bool {
    let mut visited: HashSet<String> = HashSet::new();
    let mut stack = vec![from.to_string()];
    while let Some(x) = stack.pop() {
        let skip = type_params(ctx, &x).len();
        for (_, cty) in distinct_constructors(ctx, &x) {
            let mut refs = Vec::new();
            ctor_field_inductives(ctx, cty, skip, &mut refs);
            for r in refs {
                if r == to {
                    return true;
                }
                if visited.insert(r.clone()) {
                    stack.push(r);
                }
            }
        }
    }
    false
}

/// Whether a constructor field of type `field_type` (inside inductive `enclosing`)
/// must be `Box`ed: it references some inductive that reaches back to `enclosing`,
/// closing a recursion cycle — self-recursion OR a mutual-recursion group. Acyclic
/// inductive fields stay unboxed (no over-boxing), so single-recursion output is
/// unchanged; only genuine cycles (incl. mutual recursion) gain the needed `Box`.
fn field_boxed(ctx: &Context, field_type: &Term, enclosing: &str) -> bool {
    let mut refs = Vec::new();
    collect_inductive_globals(ctx, field_type, &mut refs);
    refs.iter().any(|t| inductive_reaches(ctx, t, enclosing))
}

fn extract_ctor_fields(ctx: &Context, ty: &Term, inductive: &str, skip: usize) -> Vec<String> {
    let mut current = ty;
    for _ in 0..skip {
        if let Term::Pi { body_type, .. } = current {
            current = body_type;
        } else {
            break;
        }
    }

    let mut fields = Vec::new();
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = current
    {
        let field_ty = type_to_rust(ctx, param_type);
        if field_boxed(ctx, param_type, inductive) {
            fields.push(format!("Box<{}>", field_ty));
        } else {
            fields.push(field_ty);
        }
        current = body_type;
    }
    fields
}

/// Which of a constructor's value fields are recursive (mention the inductive),
/// after skipping `skip` leading type parameters. Parallel to the field list
/// produced by [`extract_ctor_fields`] — used to decide where to `Box::new`.
/// A constructor's parameters as `(unboxed_rust_type, is_recursive)`, after skipping
/// the inductive's leading type parameters. Used to emit constructor-wrapper fns: the
/// wrapper takes the UNBOXED field type and boxes recursive fields itself.
fn ctor_params(ctx: &Context, ty: &Term, skip: usize, inductive: &str) -> Vec<(String, bool)> {
    let mut current = ty;
    for _ in 0..skip {
        if let Term::Pi { body_type, .. } = current {
            current = body_type;
        } else {
            break;
        }
    }
    let mut out = Vec::new();
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = current
    {
        out.push((
            type_to_rust(ctx, param_type),
            field_boxed(ctx, param_type, inductive),
        ));
        current = body_type;
    }
    out
}

fn ctor_field_recursion(
    ctx: &Context,
    ty: Option<&Term>,
    inductive: &str,
    skip: usize,
) -> Vec<bool> {
    let mut flags = Vec::new();
    let mut current = match ty {
        Some(t) => t,
        None => return flags,
    };
    for _ in 0..skip {
        if let Term::Pi { body_type, .. } = current {
            current = body_type;
        } else {
            break;
        }
    }
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = current
    {
        flags.push(field_boxed(ctx, param_type, inductive));
        current = body_type;
    }
    flags
}

/// A value definition is one whose body is data (not a function): it is emitted
/// as a nullary `fn` and referenced by calling it.
fn is_value_def(ctx: &Context, name: &str) -> bool {
    ctx.get_definition_body(name)
        .map(|b| !matches!(b, Term::Lambda { .. } | Term::Fix { .. }))
        .unwrap_or(false)
}

/// The canonical kernel-primitive → Rust-type bridge: the SINGLE source of truth
/// for how the seven opaque kernel primitives lower to Rust. Both the extractor and
/// any caller that reasons about extractable types consult this, so they can never
/// disagree on a primitive's Rust type — the soundness contract that lets imperative
/// code call a bundled proven function over primitives. `None` means the name is not
/// a primitive (it may still be a user inductive or a generic type variable). These
/// are registered in the StandardLibrary as constructorless inductives, so they have
/// no enum form and are represented directly by their Rust counterpart.
pub fn primitive_rust_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "Int" => "i64",
        "Float" => "f64",
        "Text" => "String",
        "Bool" => "bool",
        "Duration" => "i64",
        "Date" => "i32",
        "Moment" => "i64",
        _ => return None,
    })
}

/// Whether every use of an arithmetic builtin in `term` is a full BINARY application
/// — the only form that lowers to a Rust operator. A bare or partially-applied builtin
/// (`add`, `add n`) has no Rust definition to call, so a definition containing one is
/// NOT cleanly extractable (it would emit a call to an undefined `add`). `term_to_rust`
/// only handles the 2-arg case; this gate keeps such definitions out of extraction.
pub(crate) fn arith_uses_well_formed(ctx: &Context, term: &Term) -> bool {
    match term {
        // A builtin appearing anywhere except as the head of a 2-arg application.
        Term::Global(name) => arith_builtin_operator(ctx, name).is_none(),
        Term::App(_, _) => {
            let (head, args) = collect_app_chain(term);
            if let Term::Global(name) = head {
                if arith_builtin_operator(ctx, name).is_some() {
                    // Head is a builtin: require exactly 2 args, and they must be clean.
                    return args.len() == 2 && args.iter().all(|a| arith_uses_well_formed(ctx, a));
                }
            }
            // Non-builtin head: head and every arg must be clean.
            arith_uses_well_formed(ctx, head) && args.iter().all(|a| arith_uses_well_formed(ctx, a))
        }
        Term::Lambda {
            param_type, body, ..
        } => arith_uses_well_formed(ctx, param_type) && arith_uses_well_formed(ctx, body),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => arith_uses_well_formed(ctx, param_type) && arith_uses_well_formed(ctx, body_type),
        Term::Fix { body, .. } => arith_uses_well_formed(ctx, body),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            arith_uses_well_formed(ctx, discriminant)
                && arith_uses_well_formed(ctx, motive)
                && cases.iter().all(|c| arith_uses_well_formed(ctx, c))
        }
        _ => true,
    }
}

/// The Rust operator for the kernel's arithmetic builtin `name` **as registered in `ctx`** —
/// the only form of the question extraction may ask.
///
/// The spelling alone does not identify the builtin. `add` is the prelude's `Int -> Int -> Int`
/// arithmetic, whose computational content IS Rust's `+`; a user's own
/// `Definition add : Nat -> Nat -> Nat` is a different function that happens to share the name,
/// is extracted as a real `fn`, and must be CALLED. The two were told apart by asking whether
/// the global had a BODY — true while the prelude's arithmetic was an opaque declaration, and
/// false the moment `Int` became a real inductive and `add` became the definition
/// `Int_add`. Every arithmetic use then stopped lowering to an operator and started dragging
/// the structural integer tower into the output.
///
/// The type is what actually decides it, and it is what the operator mapping is faithful to:
/// `Int -> Int -> Int`, where `Int` is the primitive that extracts to `i64`. Anything else at
/// that name is user code.
pub(crate) fn arith_builtin_operator(ctx: &Context, name: &str) -> Option<&'static str> {
    let op = arith_operator(name)?;
    let ty = ctx.get_global(name)?;
    if !is_int_binop(ty) {
        return None;
    }
    // …and it must be the PRELUDE'S declaration. The type narrows the question but does not
    // settle it: `Definition add : Int -> Int -> Int := fun a => fun b => mul a b` is a user's
    // own function at exactly that signature, and lowering it to `+` would silently compile a
    // different program. Comparing against the prelude as REGISTERED is the same discipline
    // `ui_bridge::Baseline` uses to tell a user declaration from a prelude one.
    match (prelude_arith_body(name), ctx.get_definition_body(name)) {
        // Still an opaque declaration (as `add` was before `Int` became an inductive).
        (None, None) => Some(op),
        (Some(prelude), Some(here)) if prelude == here => Some(op),
        _ => None,
    }
}

/// The body the STANDARD LIBRARY gives one of the five arithmetic names, read once per process
/// from a cached prelude (`interface::standard_context` hands out a copy-on-write clone, so this
/// is one registration, not one per call).
fn prelude_arith_body(name: &str) -> Option<&'static Term> {
    static BODIES: std::sync::OnceLock<HashMap<&'static str, Option<Term>>> =
        std::sync::OnceLock::new();
    let bodies = BODIES.get_or_init(|| {
        let prelude = crate::kernel::interface::standard_context();
        ["add", "sub", "mul", "div", "mod"]
            .into_iter()
            .map(|n| (n, prelude.get_definition_body(n).cloned()))
            .collect()
    });
    bodies.get(name)?.as_ref()
}

/// Whether `ty` is exactly `Int -> Int -> Int` (binder names irrelevant).
fn is_int_binop(ty: &Term) -> bool {
    fn is_int(t: &Term) -> bool {
        matches!(t, Term::Global(n) if n == "Int")
    }
    let Term::Pi {
        param_type,
        body_type,
        ..
    } = ty
    else {
        return false;
    };
    if !is_int(param_type) {
        return false;
    }
    let Term::Pi {
        param_type,
        body_type,
        ..
    } = body_type.as_ref()
    else {
        return false;
    };
    is_int(param_type) && is_int(body_type)
}

/// The Rust integer operator for a kernel arithmetic builtin NAME. Callers that have a
/// [`Context`] must use [`arith_builtin_operator`], which also checks the type; this is the
/// name table it is built on.
pub(crate) fn arith_operator(name: &str) -> Option<&'static str> {
    Some(match name {
        "add" => "+",
        "sub" => "-",
        "mul" => "*",
        "div" => "/",
        "mod" => "%",
        _ => return None,
    })
}

/// The Rust identifier the kernel constant `key` is emitted as.
///
/// A key is its own identifier, except where a SHADOW is involved (`Context::shadows`, the ruling
/// of 2026-09-24). A declaration registered under a fresh identity (`add_zero✝⟨1⟩`), which no
/// Rust identifier can spell, is emitted as the name its author wrote for as long as that name
/// denotes it — so the author's `add_zero` extracts to `check_add_zero`, the function an
/// imperative `Require that check_add_zero().` calls — and as `<name>_shadowed<n>` once a later
/// declaration has taken the name. A declaration a live shadow hides is emitted as `<key>_root`,
/// so it never meets the shadow under one Rust name. A constant derived from an identity follows
/// it.
pub fn rust_global(ctx: &Context, key: &str) -> String {
    if let Some(identity) = ctx.shadow_root(key) {
        let origin = ctx.shadow_origin(&identity).unwrap_or(identity.as_str());
        let name = if ctx.is_live_shadow(&identity) {
            origin.to_string()
        } else {
            let minted = identity
                .rsplit('\u{27e8}')
                .next()
                .and_then(|tail| tail.strip_suffix('\u{27e9}'))
                .unwrap_or_default();
            format!("{origin}_shadowed{minted}")
        };
        return key.replacen(identity.as_str(), &name, 1);
    }
    if ctx.shadow_of(key).is_some() {
        return format!("{key}_root");
    }
    key.to_string()
}

/// Convert a kernel type to a Rust type string.
fn type_to_rust(ctx: &Context, ty: &Term) -> String {
    match ty {
        // A type variable (an inductive's type parameter) → a Rust generic.
        Term::Var(name) => name.clone(),
        Term::Global(name) => primitive_rust_type(name)
            .map(str::to_string)
            .unwrap_or_else(|| rust_global(ctx, name)),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            // Non-dependent function type: A -> B
            let arg = type_to_rust(ctx, param_type);
            let ret = type_to_rust(ctx, body_type);
            format!("fn({}) -> {}", arg, ret)
        }
        Term::App(_, _) => {
            // A generic instantiation like `MyList Nat` or `MyPair A B`. Flatten
            // the application chain into `Head<arg0, arg1, …>`.
            let (head, args) = collect_app_chain(ty);
            let head_str = type_to_rust(ctx, head);
            let arg_strs: Vec<String> = args.iter().map(|a| type_to_rust(ctx, a)).collect();
            format!("{}<{}>", head_str, arg_strs.join(", "))
        }
        Term::Sort(_) => "()".to_string(),
        Term::Lit(_) => "()".to_string(), // Literals shouldn't appear as types
        _ => "()".to_string(),
    }
}

/// Check if a term is a lambda.
fn is_lambda(term: &Term) -> bool {
    matches!(term, Term::Lambda { .. })
}

/// Extract parameters from nested lambdas.
fn extract_lambda_params(term: &Term) -> (Vec<(String, Term)>, &Term) {
    let mut params = Vec::new();
    let mut current = term;

    while let Term::Lambda {
        param,
        param_type,
        body,
    } = current
    {
        params.push((param.clone(), (**param_type).clone()));
        current = body;
    }

    (params, current)
}

/// Extract parameter types from a Pi type.
fn extract_pi_params(ty: &Term) -> Vec<(String, Term)> {
    let mut params = Vec::new();
    let mut current = ty;

    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = current
    {
        params.push((param.clone(), (**param_type).clone()));
        current = body_type;
    }

    params
}

/// Extract the return type from a (possibly nested) Pi type.
fn extract_return_type(ty: &Term) -> Term {
    let mut current = ty;
    while let Term::Pi { body_type, .. } = current {
        current = body_type;
    }
    current.clone()
}

/// Count the number of arguments a constructor takes.
fn count_ctor_args(ty: &Term) -> usize {
    let mut count = 0;
    let mut current = ty;
    while let Term::Pi { body_type, .. } = current {
        count += 1;
        current = body_type;
    }
    count
}

/// Extract bindings from a case (which is typically a lambda).
fn extract_case_bindings(case: &Term, arity: usize) -> (Vec<String>, Term) {
    let mut bindings = Vec::new();
    let mut current = case;

    for _ in 0..arity {
        if let Term::Lambda { param, body, .. } = current {
            bindings.push(param.clone());
            current = body;
        } else {
            break;
        }
    }

    (bindings, current.clone())
}

/// Collect the head and all arguments from a chain of applications.
///
/// For `((f a) b) c`, returns `(f, [a, b, c])`.
fn collect_app_chain(term: &Term) -> (&Term, Vec<&Term>) {
    let mut args = Vec::new();
    let mut current = term;

    while let Term::App(f, a) = current {
        args.push(a.as_ref());
        current = f.as_ref();
    }

    // Reverse to get args in application order
    args.reverse();
    (current, args)
}

/// Check if a term is a constructor application.
#[allow(dead_code)]
fn is_constructor_app(term: &Term, ctx: &Context) -> bool {
    match term {
        Term::Global(name) => ctx.is_constructor(name),
        Term::App(f, _) => is_constructor_app(f, ctx),
        _ => false,
    }
}
