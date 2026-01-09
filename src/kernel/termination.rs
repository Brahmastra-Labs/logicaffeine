//! Termination checking for fixpoints.
//!
//! This module implements the syntactic guard condition that ensures
//! all recursive functions terminate. Without this check, the type system
//! would be unsound - we could "prove" False by writing `fix f. f`.
//!
//! The algorithm (following Coq):
//! 1. Identify the "structural parameter" - the first inductive-typed argument
//! 2. Track variables that are "structurally smaller" than the structural parameter
//! 3. Verify all recursive calls use a smaller argument
//!
//! A variable `k` is smaller than `n` if it was bound by matching on `n`:
//! `match n with Succ k => ...` means k < n structurally.

use std::collections::HashSet;

use super::context::Context;
use super::error::{KernelError, KernelResult};
use super::term::Term;

/// Context for termination checking.
struct GuardContext {
    /// The name of the fixpoint being checked
    fix_name: String,
    /// The structural parameter (decreasing argument)
    struct_param: String,
    /// The type of the structural parameter (inductive name)
    struct_type: String,
    /// Variables known to be structurally smaller than struct_param
    smaller_than: HashSet<String>,
}

/// Check that a Fix term satisfies the syntactic guard condition.
///
/// This is the main entry point for termination checking.
pub fn check_termination(ctx: &Context, fix_name: &str, body: &Term) -> KernelResult<()> {
    // Extract the structural parameter (first inductive-typed lambda param)
    let (struct_param, struct_type, inner) = extract_structural_param(ctx, body)?;

    // Create the guard context with empty smaller set
    let guard_ctx = GuardContext {
        fix_name: fix_name.to_string(),
        struct_param,
        struct_type,
        smaller_than: HashSet::new(),
    };

    // Check all recursive calls are guarded
    check_guarded(ctx, &guard_ctx, inner)
}

/// Extract the first inductive-typed parameter as the structural argument.
///
/// Returns (param_name, inductive_name, remaining_body).
fn extract_structural_param<'a>(
    ctx: &Context,
    body: &'a Term,
) -> KernelResult<(String, String, &'a Term)> {
    match body {
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            // Check if param_type is an inductive (handles both Nat and List A)
            if let Some(type_name) = extract_inductive_name(ctx, param_type) {
                return Ok((param.clone(), type_name, body));
            }
            // Not an inductive type, try the next parameter
            extract_structural_param(ctx, body)
        }
        _ => Err(KernelError::TerminationViolation {
            fix_name: String::new(),
            reason: "No inductive parameter found for structural recursion".to_string(),
        }),
    }
}

/// Extract the inductive type name from a type.
///
/// Handles both simple inductives like `Nat` and polymorphic ones like `List A`.
fn extract_inductive_name(ctx: &Context, ty: &Term) -> Option<String> {
    match ty {
        Term::Global(name) if ctx.is_inductive(name) => Some(name.clone()),
        Term::App(func, _) => extract_inductive_name(ctx, func),
        _ => None,
    }
}

/// Check that all recursive calls in `term` are guarded (use smaller arguments).
fn check_guarded(ctx: &Context, guard_ctx: &GuardContext, term: &Term) -> KernelResult<()> {
    match term {
        // Application: check for recursive calls
        Term::App(func, arg) => {
            // Check if this is a recursive call to the fixpoint
            check_recursive_call(ctx, guard_ctx, func, arg)?;

            // Recursively check subterms
            check_guarded(ctx, guard_ctx, func)?;
            check_guarded(ctx, guard_ctx, arg)
        }

        // Match on structural parameter introduces smaller variables
        Term::Match {
            discriminant,
            cases,
            ..
        } => {
            // Check if we're matching on the structural parameter
            if let Term::Var(disc_name) = discriminant.as_ref() {
                if disc_name == &guard_ctx.struct_param {
                    // This match guards recursive calls - check cases with
                    // constructor-bound variables marked as smaller
                    return check_match_cases_guarded(ctx, guard_ctx, cases);
                }
            }

            // Not matching on structural param - just recurse normally
            check_guarded(ctx, guard_ctx, discriminant)?;
            for case in cases {
                check_guarded(ctx, guard_ctx, case)?;
            }
            Ok(())
        }

        // Lambda: recurse into body (param shadows nothing relevant)
        Term::Lambda { body, .. } => check_guarded(ctx, guard_ctx, body),

        // Pi: recurse into body type
        Term::Pi { body_type, .. } => check_guarded(ctx, guard_ctx, body_type),

        // Nested fixpoint: check its body (with its own fix_name)
        Term::Fix { body, .. } => {
            // Note: nested fixpoints should have their own termination check
            // when they are type-checked. Here we just recurse.
            check_guarded(ctx, guard_ctx, body)
        }

        // Leaves: no recursive calls possible
        Term::Sort(_) | Term::Var(_) | Term::Global(_) | Term::Lit(_) => Ok(()),
    }
}

/// Check if an application is a recursive call, and if so, verify it uses a smaller argument.
fn check_recursive_call(
    _ctx: &Context,
    guard_ctx: &GuardContext,
    func: &Term,
    arg: &Term,
) -> KernelResult<()> {
    // Walk through nested applications to find the actual function
    // For `plus k m`, func is `App(Var("plus"), Var("k"))`, arg is `Var("m")`
    // So we need to find if the head of func is our fixpoint
    let (head, first_arg) = extract_head_and_first_arg(func, arg);

    // Check if the head is a reference to our fixpoint
    if let Term::Var(name) = head {
        if name == &guard_ctx.fix_name {
            // This is a recursive call! Check the first argument (structural argument)
            match first_arg {
                Term::Var(arg_name) => {
                    if !guard_ctx.smaller_than.contains(arg_name) {
                        return Err(KernelError::TerminationViolation {
                            fix_name: guard_ctx.fix_name.clone(),
                            reason: format!(
                                "Recursive call with '{}' which is not structurally smaller than '{}'",
                                arg_name, guard_ctx.struct_param
                            ),
                        });
                    }
                    // Valid: argument is in the smaller set
                    Ok(())
                }
                _ => {
                    // Argument is not a simple variable - could be a complex expression
                    // For safety, we reject this unless it's clearly smaller
                    Err(KernelError::TerminationViolation {
                        fix_name: guard_ctx.fix_name.clone(),
                        reason: format!(
                            "Recursive call with complex argument - cannot verify termination"
                        ),
                    })
                }
            }
        } else {
            Ok(()) // Not a recursive call
        }
    } else {
        Ok(()) // Head is not a variable, so not a recursive call
    }
}

/// Extract the head function and first argument from a (possibly nested) application.
///
/// For `f a b c`, returns `(f, a)`.
/// For `f a`, returns `(f, a)`.
fn extract_head_and_first_arg<'a>(func: &'a Term, arg: &'a Term) -> (&'a Term, &'a Term) {
    // Walk left through applications to find the head
    let mut current = func;
    let mut first_arg = arg;

    while let Term::App(inner_func, inner_arg) = current {
        first_arg = inner_arg;
        current = inner_func;
    }

    (current, first_arg)
}

/// Check match cases with structural variables marked as smaller.
fn check_match_cases_guarded(
    ctx: &Context,
    guard_ctx: &GuardContext,
    cases: &[Term],
) -> KernelResult<()> {
    // Get constructors for the inductive type
    let constructors = ctx.get_constructors(&guard_ctx.struct_type);

    for (case, (ctor_name, ctor_type)) in cases.iter().zip(constructors.iter()) {
        // Count constructor parameters
        let param_count = count_pi_params(ctor_type);

        // Extract the smaller variables from this case
        // The case is typically: λx1. λx2. ... λxn. body
        // where x1..xn are the constructor parameters
        let (smaller_vars, case_body) = extract_lambda_params(case, param_count);

        // Create extended guard context with these variables as smaller
        let mut extended_ctx = GuardContext {
            fix_name: guard_ctx.fix_name.clone(),
            struct_param: guard_ctx.struct_param.clone(),
            struct_type: guard_ctx.struct_type.clone(),
            smaller_than: guard_ctx.smaller_than.clone(),
        };

        // Only add variables that are of the same inductive type (recursive arguments)
        // For Succ : Nat -> Nat, the param `k` is smaller
        // For Zero : Nat, no smaller vars
        let recursive_params = get_recursive_params(ctx, &guard_ctx.struct_type, ctor_type);
        for (idx, _) in recursive_params {
            if idx < smaller_vars.len() {
                extended_ctx.smaller_than.insert(smaller_vars[idx].clone());
            }
        }

        // Also mark ALL constructor params as smaller (conservative approach)
        // This handles cases like `Succ k` where k is the direct subterm
        for var in &smaller_vars {
            extended_ctx.smaller_than.insert(var.clone());
        }

        // Check the case body with the extended context
        check_guarded(ctx, &extended_ctx, case_body)?;
    }

    Ok(())
}

/// Count the number of Pi parameters in a type.
fn count_pi_params(ty: &Term) -> usize {
    match ty {
        Term::Pi { body_type, .. } => 1 + count_pi_params(body_type),
        _ => 0,
    }
}

/// Extract lambda parameters and return (param_names, body).
fn extract_lambda_params(term: &Term, count: usize) -> (Vec<String>, &Term) {
    if count == 0 {
        return (Vec::new(), term);
    }

    match term {
        Term::Lambda { param, body, .. } => {
            let (mut params, final_body) = extract_lambda_params(body, count - 1);
            params.insert(0, param.clone());
            (params, final_body)
        }
        _ => (Vec::new(), term),
    }
}

/// Get indices of parameters that are of the inductive type (recursive positions).
fn get_recursive_params(ctx: &Context, inductive: &str, ctor_type: &Term) -> Vec<(usize, String)> {
    let mut result = Vec::new();
    let mut current = ctor_type;
    let mut idx = 0;

    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = current
    {
        // Check if param_type is the inductive type (handles polymorphic types like List A)
        if let Some(type_name) = extract_inductive_name(ctx, param_type) {
            if type_name == inductive {
                result.push((idx, param.clone()));
            }
        }
        idx += 1;
        current = body_type;
    }

    result
}
