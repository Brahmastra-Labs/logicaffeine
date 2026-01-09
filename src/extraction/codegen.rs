//! Rust code generation from kernel terms.
//!
//! Converts kernel terms to executable Rust source code.

use super::error::ExtractError;
use crate::kernel::{Context, Literal, Term};
use std::collections::HashSet;

/// Context for code generation within a term.
struct TermGenCtx<'a> {
    /// The name of the definition being emitted
    def_name: &'a str,
    /// The name of the recursive reference (from Fix)
    rec_name: &'a str,
    /// Variables that need to be dereferenced (boxed in match patterns)
    deref_vars: &'a HashSet<String>,
}

/// Code generator for extracting Rust from kernel terms.
pub struct CodeGen<'a> {
    ctx: &'a Context,
    output: String,
    emitted: HashSet<String>,
}

impl<'a> CodeGen<'a> {
    /// Create a new code generator.
    pub fn new(ctx: &'a Context) -> Self {
        Self {
            ctx,
            output: String::new(),
            emitted: HashSet::new(),
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

        let ctors = self.ctx.get_constructors(name);
        if ctors.is_empty() {
            return Err(ExtractError::NotFound(name.to_string()));
        }

        // Check if recursive (any constructor references the type)
        let is_recursive = ctors.iter().any(|(_, ty)| term_references(ty, name));

        self.output.push_str(&format!("enum {} {{\n", name));
        for (ctor_name, ctor_ty) in &ctors {
            let args = extract_ctor_args(ctor_ty, name, is_recursive);
            if args.is_empty() {
                self.output.push_str(&format!("    {},\n", ctor_name));
            } else {
                self.output
                    .push_str(&format!("    {}({}),\n", ctor_name, args));
            }
        }
        self.output.push_str("}\n\n");
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
            // Simple constant
            self.emit_const(name, body, ty)?;
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
        // Extract parameters from nested lambdas
        let (params, inner_body) = extract_lambda_params(fix_body);

        // Extract parameter types from the Pi type
        let param_types = extract_pi_params(ty);

        // Get return type
        let ret_ty = extract_return_type(ty);

        // Build function signature
        self.output.push_str(&format!("fn {}(", def_name));
        for (i, (param_name, _)) in params.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            let param_ty = param_types
                .get(i)
                .map(|(_, t)| type_to_rust(t))
                .unwrap_or_else(|| "()".to_string());
            self.output.push_str(&format!("{}: {}", param_name, param_ty));
        }
        self.output
            .push_str(&format!(") -> {} {{\n", type_to_rust(&ret_ty)));

        // Generate body, replacing recursive calls
        let body_code = self.term_to_rust(inner_body, def_name, rec_name);
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
        // Extract parameters from nested lambdas
        let (params, inner_body) = extract_lambda_params(body);

        // Extract parameter types from the Pi type
        let param_types = extract_pi_params(ty);

        // Get return type
        let ret_ty = extract_return_type(ty);

        // Build function signature
        self.output.push_str(&format!("fn {}(", def_name));
        for (i, (param_name, _)) in params.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            let param_ty = param_types
                .get(i)
                .map(|(_, t)| type_to_rust(t))
                .unwrap_or_else(|| "()".to_string());
            self.output.push_str(&format!("{}: {}", param_name, param_ty));
        }
        self.output
            .push_str(&format!(") -> {} {{\n", type_to_rust(&ret_ty)));

        // Generate body (no recursive name replacement)
        let body_code = self.term_to_rust(inner_body, def_name, "");
        self.output.push_str(&format!("    {}\n", body_code));
        self.output.push_str("}\n\n");

        Ok(())
    }

    /// Emit a simple constant.
    fn emit_const(&mut self, name: &str, body: &Term, ty: &Term) -> Result<(), ExtractError> {
        let ty_str = type_to_rust(ty);
        let body_str = self.term_to_rust(body, name, "");
        self.output
            .push_str(&format!("const {}: {} = {};\n\n", name.to_uppercase(), ty_str, body_str));
        Ok(())
    }

    /// Convert a kernel term to Rust code.
    fn term_to_rust(&self, term: &Term, def_name: &str, rec_name: &str) -> String {
        let empty_deref = HashSet::new();
        let ctx = TermGenCtx {
            def_name,
            rec_name,
            deref_vars: &empty_deref,
        };
        self.term_to_rust_ctx(term, &ctx)
    }

    /// Convert a kernel term to Rust code with context for dereferencing.
    fn term_to_rust_ctx(&self, term: &Term, tctx: &TermGenCtx) -> String {
        match term {
            Term::Var(name) => {
                // Check if this is a reference to the recursive function
                if name == tctx.rec_name && !tctx.rec_name.is_empty() {
                    tctx.def_name.to_string()
                } else if tctx.deref_vars.contains(name) {
                    // Need to dereference boxed value from match pattern
                    format!("(*{})", name)
                } else {
                    name.clone()
                }
            }
            Term::Global(name) => {
                // Check if it's a constructor
                if self.ctx.is_constructor(name) {
                    if let Some(ind) = self.ctx.constructor_inductive(name) {
                        return format!("{}::{}", ind, name);
                    }
                }
                // Check if it's the recursive reference
                if name == tctx.rec_name && !tctx.rec_name.is_empty() {
                    return tctx.def_name.to_string();
                }
                name.clone()
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
                            // Check if the type is recursive
                            let ctors = self.ctx.get_constructors(ind);
                            let is_recursive =
                                ctors.iter().any(|(_, ty)| term_references(ty, ind));
                            if is_recursive && args_strs.len() == 1 {
                                return format!("{}::{}(Box::new({}))", ind, name, args_strs[0]);
                            } else {
                                return format!("{}::{}({})", ind, name, args_strs.join(", "));
                            }
                        }
                    }
                }

                // Check if head is a Var that should be renamed (recursive call)
                let head_str = if let Term::Var(name) = head {
                    if name == tctx.rec_name && !tctx.rec_name.is_empty() {
                        tctx.def_name.to_string()
                    } else {
                        name.clone()
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
                let param_ty = type_to_rust(param_type);
                let body_str = self.term_to_rust_ctx(body, tctx);
                format!("|{}: {}| {}", param, param_ty, body_str)
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                let disc_str = self.term_to_rust_ctx(discriminant, tctx);

                // Get the inductive type from the motive (λx:T. ReturnType)
                // The param_type of the motive lambda gives us the inductive
                let ind_name = self.infer_inductive_from_motive(motive)
                    .or_else(|| self.infer_inductive_type(discriminant));

                let mut result = format!("match {} {{\n", disc_str);
                if let Some(ind) = &ind_name {
                    let ctors = self.ctx.get_constructors(ind);
                    let is_recursive = ctors.iter().any(|(_, ty)| term_references(ty, ind));

                    for (i, (ctor_name, ctor_ty)) in ctors.iter().enumerate() {
                        if i < cases.len() {
                            let case = &cases[i];
                            let ctor_arity = count_ctor_args(ctor_ty);

                            result.push_str(&format!("        {}::{}", ind, ctor_name));
                            if ctor_arity > 0 {
                                // Generate pattern with bindings
                                let (bindings, case_body) = extract_case_bindings(case, ctor_arity);
                                result.push_str("(");
                                for (j, binding) in bindings.iter().enumerate() {
                                    if j > 0 {
                                        result.push_str(", ");
                                    }
                                    result.push_str(binding);
                                }
                                result.push_str(")");

                                // If recursive, bindings need to be dereferenced in the body
                                let case_deref_vars: HashSet<String> = if is_recursive {
                                    bindings.iter().cloned().collect()
                                } else {
                                    HashSet::new()
                                };
                                let case_tctx = TermGenCtx {
                                    def_name: tctx.def_name,
                                    rec_name: tctx.rec_name,
                                    deref_vars: &case_deref_vars,
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
                };
                self.term_to_rust_ctx(body, &fix_tctx)
            }
            Term::Lit(lit) => match lit {
                Literal::Int(n) => format!("{}i64", n),
                Literal::Float(f) => format!("{}f64", f),
                Literal::Text(s) => format!("{:?}", s),
            },
            Term::Pi { .. } => "/* type */".to_string(),
            Term::Sort(_) => "/* sort */".to_string(),
        }
    }

    /// Extract the inductive type from a match motive.
    ///
    /// The motive is typically `λx:T. ReturnType` where T is the inductive.
    fn infer_inductive_from_motive(&self, motive: &Term) -> Option<String> {
        if let Term::Lambda { param_type, .. } = motive {
            if let Term::Global(name) = param_type.as_ref() {
                if self.ctx.is_inductive(name) {
                    return Some(name.clone());
                }
            }
        }
        None
    }

    /// Try to infer the inductive type from a term.
    fn infer_inductive_type(&self, term: &Term) -> Option<String> {
        match term {
            Term::Var(_) => {
                // Cannot infer from Var alone - use motive instead
                None
            }
            Term::Global(name) => {
                if self.ctx.is_constructor(name) {
                    self.ctx.constructor_inductive(name).map(|s| s.to_string())
                } else if self.ctx.is_inductive(name) {
                    Some(name.clone())
                } else {
                    None
                }
            }
            Term::App(f, _) => self.infer_inductive_type(f),
            _ => None,
        }
    }
}

/// Check if a term references a given name.
fn term_references(term: &Term, name: &str) -> bool {
    match term {
        Term::Global(n) => n == name,
        Term::App(f, a) => term_references(f, name) || term_references(a, name),
        Term::Lambda { body, param_type, .. } => {
            term_references(body, name) || term_references(param_type, name)
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => term_references(param_type, name) || term_references(body_type, name),
        Term::Fix { body, .. } => term_references(body, name),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            term_references(discriminant, name)
                || term_references(motive, name)
                || cases.iter().any(|c| term_references(c, name))
        }
        // Base cases: no references
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) => false,
    }
}

/// Extract constructor argument types as a Rust type string.
fn extract_ctor_args(ty: &Term, inductive: &str, is_recursive: bool) -> String {
    // For a type like Nat -> Nat, extract the argument types
    let mut args = Vec::new();
    let mut current = ty;

    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = current
    {
        let arg_ty = type_to_rust(param_type);
        // If this is a recursive reference, wrap in Box
        if is_recursive && matches!(param_type.as_ref(), Term::Global(n) if n == inductive) {
            args.push(format!("Box<{}>", arg_ty));
        } else {
            args.push(arg_ty);
        }
        current = body_type;
    }

    args.join(", ")
}

/// Convert a kernel type to a Rust type string.
fn type_to_rust(ty: &Term) -> String {
    match ty {
        Term::Global(name) => {
            // Map kernel primitive types to Rust types
            match name.as_str() {
                "Int" => "i64".to_string(),
                "Float" => "f64".to_string(),
                "Text" => "String".to_string(),
                _ => name.clone(),
            }
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            // Non-dependent function type: A -> B
            let arg = type_to_rust(param_type);
            let ret = type_to_rust(body_type);
            format!("fn({}) -> {}", arg, ret)
        }
        Term::App(f, a) => {
            // Generic application (rare in extraction)
            format!("{}<{}>", type_to_rust(f), type_to_rust(a))
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
