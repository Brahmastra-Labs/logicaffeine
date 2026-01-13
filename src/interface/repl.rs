//! REPL for the Vernacular interface.
//!
//! Orchestrates command parsing and kernel execution.

use super::command::Command;
use super::command_parser::parse_command;
use super::error::InterfaceError;
use crate::kernel::prelude::StandardLibrary;
use crate::kernel::{infer_type, normalize, Context, Term};

/// The Vernacular REPL.
///
/// Maintains a kernel context and executes commands against it.
pub struct Repl {
    ctx: Context,
}

impl Repl {
    /// Create a new REPL with the standard library loaded.
    pub fn new() -> Self {
        let mut ctx = Context::new();
        StandardLibrary::register(&mut ctx);
        Self { ctx }
    }

    /// Execute a command string.
    ///
    /// Returns the output string (for Check/Eval) or empty string (for Definition/Inductive).
    pub fn execute(&mut self, input: &str) -> Result<String, InterfaceError> {
        let cmd = parse_command(input)?;

        match cmd {
            Command::Definition { name, ty, body, is_hint } => {
                // Type check the body
                let inferred_ty = infer_type(&self.ctx, &body)?;

                // Use provided type or inferred type
                let ty = ty.unwrap_or(inferred_ty);

                // Add definition to context
                self.ctx.add_definition(name.clone(), ty, body);

                // Register as hint if marked
                if is_hint {
                    self.ctx.add_hint(&name);
                }

                Ok(String::new()) // Silent success
            }

            Command::Check(term) => {
                // Infer the type
                let ty = infer_type(&self.ctx, &term)?;
                Ok(format!("{} : {}", term, ty))
            }

            Command::Eval(term) => {
                // Type check first (ensures well-typed)
                let _ = infer_type(&self.ctx, &term)?;

                // Normalize
                let result = normalize(&self.ctx, &term);
                Ok(format!("{}", result))
            }

            Command::Inductive {
                name,
                params,
                sort,
                constructors,
            } => {
                // Build polymorphic sort: Π(p1:T1). Π(p2:T2). ... Type
                let poly_sort = build_polymorphic_sort(&params, sort);

                // Register the inductive type with its polymorphic sort
                self.ctx.add_inductive(&name, poly_sort);

                // Register constructors with prepended parameters
                for (ctor_name, ctor_ty) in constructors {
                    // Prepend params to constructor type:
                    // If ctor_ty = A -> List A -> List A
                    // And params = [(A, Type)]
                    // Result = Π(A:Type). A -> List A -> List A
                    let poly_ctor_ty = build_polymorphic_constructor(&params, ctor_ty);
                    self.ctx.add_constructor(&ctor_name, &name, poly_ctor_ty);
                }

                Ok(String::new()) // Silent success
            }
        }
    }

    /// Get a reference to the underlying context.
    pub fn context(&self) -> &Context {
        &self.ctx
    }

    /// Get a mutable reference to the underlying context.
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.ctx
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a polymorphic sort from type parameters.
///
/// For params = [(A, Type), (B, Type)] and base_sort = Type,
/// produces: Π(A:Type). Π(B:Type). Type
fn build_polymorphic_sort(params: &[(String, Term)], base_sort: Term) -> Term {
    // Fold right to build nested Pi types
    params.iter().rev().fold(base_sort, |body, (name, ty)| {
        Term::Pi {
            param: name.clone(),
            param_type: Box::new(ty.clone()),
            body_type: Box::new(body),
        }
    })
}

/// Build a polymorphic constructor type by prepending parameters.
///
/// For params = [(A, Type)] and ctor_ty = A -> List A -> List A,
/// produces: Π(A:Type). A -> List A -> List A
///
/// The kernel uses named variables (Var(String)), so we convert
/// Global("A") to Var("A") for parameters.
fn build_polymorphic_constructor(params: &[(String, Term)], ctor_ty: Term) -> Term {
    if params.is_empty() {
        return ctor_ty;
    }

    // Convert Global(param_name) to Var(param_name) for all parameters
    let param_names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    let body = substitute_globals_with_vars(&ctor_ty, &param_names);

    // Wrap with Pi bindings (fold right to build nested Pi)
    params.iter().rev().fold(body, |body, (name, ty)| {
        Term::Pi {
            param: name.clone(),
            param_type: Box::new(ty.clone()),
            body_type: Box::new(body),
        }
    })
}

/// Convert Global(name) to Var(name) for names in the param list.
/// This makes parameter references in constructor types into bound variables.
fn substitute_globals_with_vars(term: &Term, param_names: &[&str]) -> Term {
    match term {
        Term::Global(n) if param_names.contains(&n.as_str()) => Term::Var(n.clone()),
        Term::Global(n) => Term::Global(n.clone()),
        Term::Var(n) => Term::Var(n.clone()),
        Term::Sort(u) => Term::Sort(u.clone()),
        Term::Lit(l) => Term::Lit(l.clone()),
        Term::App(f, a) => Term::App(
            Box::new(substitute_globals_with_vars(f, param_names)),
            Box::new(substitute_globals_with_vars(a, param_names)),
        ),
        Term::Lambda { param, param_type, body } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(substitute_globals_with_vars(param_type, param_names)),
            body: Box::new(substitute_globals_with_vars(body, param_names)),
        },
        Term::Pi { param, param_type, body_type } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(substitute_globals_with_vars(param_type, param_names)),
            body_type: Box::new(substitute_globals_with_vars(body_type, param_names)),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(substitute_globals_with_vars(body, param_names)),
        },
        Term::Match { discriminant, motive, cases } => Term::Match {
            discriminant: Box::new(substitute_globals_with_vars(discriminant, param_names)),
            motive: Box::new(substitute_globals_with_vars(motive, param_names)),
            cases: cases
                .iter()
                .map(|c| substitute_globals_with_vars(c, param_names))
                .collect(),
        },
        Term::Hole => Term::Hole, // Holes are unchanged
    }
}
