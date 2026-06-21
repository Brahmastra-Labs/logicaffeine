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

use crate::context::Context;
use crate::error::{KernelError, KernelResult};
use crate::term::Term;

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
    /// False once an inner binder has shadowed `struct_param`. A `match` on that
    /// name then refers to the shadowing binding, not the structural argument, so
    /// it must NOT be treated as the guarding match.
    struct_param_live: bool,
    /// False once an inner binder has shadowed `fix_name`. A call to that name
    /// then refers to the shadowing binding, not our fixpoint.
    fix_name_live: bool,
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
        struct_param_live: true,
        fix_name_live: true,
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

/// Build the guard context for the scope under a binder that introduces `param`.
///
/// The binder shadows any prior meaning of `param`: within its body `param` no
/// longer refers to the structural parameter, a structurally-smaller variable,
/// or the fixpoint itself. Failing to honor this lets an inner `match` on a
/// shadowed name be mistaken for the guarding match (admitting a non-decreasing
/// recursion), or an inner binding of a smaller-marked name keep its "smaller"
/// status after being rebound to an arbitrary value.
fn enter_binder(guard_ctx: &GuardContext, param: &str) -> GuardContext {
    let mut child = GuardContext {
        fix_name: guard_ctx.fix_name.clone(),
        struct_param: guard_ctx.struct_param.clone(),
        struct_type: guard_ctx.struct_type.clone(),
        smaller_than: guard_ctx.smaller_than.clone(),
        struct_param_live: guard_ctx.struct_param_live,
        fix_name_live: guard_ctx.fix_name_live,
    };
    child.smaller_than.remove(param);
    if param == guard_ctx.struct_param {
        child.struct_param_live = false;
    }
    if param == guard_ctx.fix_name {
        child.fix_name_live = false;
    }
    child
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
            // Check if we're matching on the structural parameter (and that it
            // has not been shadowed by an inner binder).
            if let Term::Var(disc_name) = discriminant.as_ref() {
                if guard_ctx.struct_param_live && disc_name == &guard_ctx.struct_param {
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

        // Lambda: the binder may shadow the structural parameter, a smaller
        // variable, or the fixpoint name — recurse under a scope reflecting that.
        Term::Lambda { param, body, .. } => {
            let child = enter_binder(guard_ctx, param);
            check_guarded(ctx, &child, body)
        }

        // Pi: same binder-shadowing handling as Lambda.
        Term::Pi { param, body_type, .. } => {
            let child = enter_binder(guard_ctx, param);
            check_guarded(ctx, &child, body_type)
        }

        // Nested fixpoint: its own name shadows ours; its body gets its own
        // termination check when type-checked.
        Term::Fix { name, body } => {
            let child = enter_binder(guard_ctx, name);
            check_guarded(ctx, &child, body)
        }

        // Leaves: no recursive calls possible
        Term::Sort(_) | Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => Ok(()),
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

    // Check if the head is a reference to our fixpoint (and that the name has
    // not been shadowed by an inner binder).
    if let Term::Var(name) = head {
        if guard_ctx.fix_name_live && name == &guard_ctx.fix_name {
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
            struct_param_live: guard_ctx.struct_param_live,
            fix_name_live: guard_ctx.fix_name_live,
        };

        // Mark every constructor parameter as structurally smaller.
        //
        // Soundness: we only reach here when the discriminant is the structural
        // parameter itself (see `check_guarded`'s `Match` arm), so the matched
        // value is `ctor x1 … xn` and each `xi` is a *direct argument* of that
        // constructor — hence a genuine structural subterm of the parameter.
        // Recursing on any `xi` therefore decreases. Parameters of a foreign
        // type are still marked, but a recursive call on one cannot type-check
        // (the fixpoint expects the structural type), and complex (non-variable)
        // recursive arguments are rejected outright — so the over-approximation
        // never admits a non-terminating fixpoint. `ctor_name`/`ctor_type` are
        // bound for the constructor-arity computation above.
        let _ = ctor_name;
        for var in &smaller_vars {
            extended_ctx.smaller_than.insert(var.clone());
            // A constructor binder may reuse the name of the structural parameter
            // or the fixpoint; inside this case that name now refers to the
            // (smaller) bound variable, so the original meaning is shadowed.
            if var == &guard_ctx.struct_param {
                extended_ctx.struct_param_live = false;
            }
            if var == &guard_ctx.fix_name {
                extended_ctx.fix_name_live = false;
            }
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

