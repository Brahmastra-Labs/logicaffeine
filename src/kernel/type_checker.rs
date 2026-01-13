//! Bidirectional type checker for the Calculus of Constructions.
//!
//! The type checker implements the typing rules of CoC:
//!
//! ```text
//! ─────────────────────── (Sort)
//!   Γ ⊢ Type n : Type (n+1)
//!
//!   Γ(x) = A
//! ─────────────── (Var)
//!   Γ ⊢ x : A
//!
//!   Γ ⊢ A : Type i    Γ, x:A ⊢ B : Type j
//! ─────────────────────────────────────────── (Pi)
//!          Γ ⊢ Π(x:A). B : Type max(i,j)
//!
//!   Γ ⊢ A : Type i    Γ, x:A ⊢ t : B
//! ─────────────────────────────────────── (Lambda)
//!      Γ ⊢ λ(x:A). t : Π(x:A). B
//!
//!   Γ ⊢ f : Π(x:A). B    Γ ⊢ a : A
//! ───────────────────────────────────── (App)
//!         Γ ⊢ f a : B[x := a]
//! ```

use super::context::Context;
use super::error::{KernelError, KernelResult};
use super::reduction::normalize;
use super::term::{Literal, Term, Universe};

/// Infer the type of a term in a context.
///
/// This is the main entry point for type checking.
pub fn infer_type(ctx: &Context, term: &Term) -> KernelResult<Term> {
    match term {
        // Sort: Type n : Type (n+1)
        Term::Sort(u) => Ok(Term::Sort(u.succ())),

        // Var: lookup in local context
        Term::Var(name) => ctx
            .get(name)
            .cloned()
            .ok_or_else(|| KernelError::UnboundVariable(name.clone())),

        // Global: lookup in global context (inductives and constructors)
        Term::Global(name) => ctx
            .get_global(name)
            .cloned()
            .ok_or_else(|| KernelError::UnboundVariable(name.clone())),

        // Pi: Π(x:A). B : Type max(sort(A), sort(B))
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            // A must be a type
            let a_sort = infer_sort(ctx, param_type)?;

            // B must be a type in the extended context
            let extended_ctx = ctx.extend(param, (**param_type).clone());
            let b_sort = infer_sort(&extended_ctx, body_type)?;

            // The Pi type lives in the max of the two universes
            Ok(Term::Sort(a_sort.max(&b_sort)))
        }

        // Lambda: λ(x:A). t : Π(x:A). T where t : T
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            // Check param_type is well-formed (is a type)
            let _ = infer_sort(ctx, param_type)?;

            // Infer body type in extended context
            let extended_ctx = ctx.extend(param, (**param_type).clone());
            let body_type = infer_type(&extended_ctx, body)?;

            // The lambda has a Pi type
            Ok(Term::Pi {
                param: param.clone(),
                param_type: param_type.clone(),
                body_type: Box::new(body_type),
            })
        }

        // App: (f a) : B[x := a] where f : Π(x:A). B and a : A
        Term::App(func, arg) => {
            let func_type = infer_type(ctx, func)?;

            match func_type {
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                } => {
                    // Check argument has expected type
                    check_type(ctx, arg, &param_type)?;

                    // Substitute argument into body type
                    Ok(substitute(&body_type, &param, arg))
                }
                _ => Err(KernelError::NotAFunction(format!("{}", func)))
            }
        }

        // Match: pattern matching on inductive types
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            // 1. Discriminant must have an inductive type
            let disc_type = infer_type(ctx, discriminant)?;
            let inductive_name = extract_inductive_name(ctx, &disc_type)
                .ok_or_else(|| KernelError::NotAnInductive(format!("{}", disc_type)))?;

            // 2. Check motive is well-formed
            // The motive can be either:
            // - A function λ_:I. T (proper motive)
            // - A raw type T (constant motive, wrapped automatically)
            let motive_type = infer_type(ctx, motive)?;
            let effective_motive = match &motive_type {
                Term::Pi {
                    param_type,
                    body_type,
                    ..
                } => {
                    // Motive is a function - check it takes the inductive type
                    if !types_equal(param_type, &disc_type) {
                        return Err(KernelError::InvalidMotive(format!(
                            "motive parameter {} doesn't match discriminant type {}",
                            param_type, disc_type
                        )));
                    }
                    // body_type should be a Sort
                    match infer_type(ctx, body_type) {
                        Ok(Term::Sort(_)) => {}
                        _ => {
                            return Err(KernelError::InvalidMotive(format!(
                                "motive body {} is not a type",
                                body_type
                            )));
                        }
                    }
                    // Use motive as-is
                    (**motive).clone()
                }
                Term::Sort(_) => {
                    // Motive is a raw type - wrap in a constant function
                    // λ_:disc_type. motive
                    Term::Lambda {
                        param: "_".to_string(),
                        param_type: Box::new(disc_type.clone()),
                        body: motive.clone(),
                    }
                }
                _ => {
                    return Err(KernelError::InvalidMotive(format!(
                        "motive {} is not a function or type",
                        motive
                    )));
                }
            };

            // 3. Check case count matches constructor count
            let constructors = ctx.get_constructors(&inductive_name);
            if cases.len() != constructors.len() {
                return Err(KernelError::WrongNumberOfCases {
                    expected: constructors.len(),
                    found: cases.len(),
                });
            }

            // 4. Check each case has the correct type
            for (case, (ctor_name, ctor_type)) in cases.iter().zip(constructors.iter()) {
                let expected_case_type = compute_case_type(&effective_motive, ctor_name, ctor_type, &disc_type);
                check_type(ctx, case, &expected_case_type)?;
            }

            // 5. Return type is Motive(discriminant), beta-reduced
            // Without beta reduction, (λ_:T. R) x returns the un-reduced form
            // which causes type mismatches in nested matches.
            let return_type = Term::App(
                Box::new(effective_motive),
                discriminant.clone(),
            );
            Ok(beta_reduce(&return_type))
        }

        // Literal: infer type based on literal kind
        Term::Lit(lit) => {
            match lit {
                Literal::Int(_) => Ok(Term::Global("Int".to_string())),
                Literal::Float(_) => Ok(Term::Global("Float".to_string())),
                Literal::Text(_) => Ok(Term::Global("Text".to_string())),
            }
        }

        // Hole: implicit argument, cannot infer type standalone
        // Holes are handled specially in check_type
        Term::Hole => Err(KernelError::CannotInferHole),

        // Fix: fix f. body
        // The type of (fix f. body) is the type of body when f is bound to that type.
        // This is a fixpoint equation: T = type_of(body) where f : T.
        //
        // For typical fixpoints, body is a lambda: fix f. λx:A. e
        // The type is Π(x:A). B where B is the type of e (with f : Π(x:A). B).
        Term::Fix { name, body } => {
            // For fix f. body, we need to handle the recursive reference to f.
            // We structurally infer the type from lambda structure.
            //
            // This works because:
            // 1. The body is typically nested lambdas with explicit parameter types
            // 2. The return type is determined by the innermost expression's motive
            // 3. Recursive calls have the same type as the fixpoint itself

            // Extract the structural type from nested lambdas and motive
            let structural_type = infer_fix_type_structurally(ctx, body)?;

            // *** THE GUARDIAN: TERMINATION CHECK ***
            // Verify that recursive calls decrease structurally.
            // Without this check, we could "prove" False by looping forever.
            super::termination::check_termination(ctx, name, body)?;

            // Sanity check: verify the body is well-formed with f bound
            let extended = ctx.extend(name, structural_type.clone());
            let _ = infer_type(&extended, body)?;

            Ok(structural_type)
        }
    }
}

/// Infer the type of a fixpoint body structurally.
///
/// For `λx:A. body`, returns `Π(x:A). <type of body>`.
/// This recursively handles nested lambdas.
///
/// The key insight is that for well-formed fixpoints, the body structure
/// determines the type: parameters have explicit types, and the return type
/// can be inferred from the innermost expression.
fn infer_fix_type_structurally(ctx: &Context, term: &Term) -> KernelResult<Term> {
    match term {
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            // Check param_type is well-formed
            let _ = infer_sort(ctx, param_type)?;

            // Extend context and recurse into body
            let extended = ctx.extend(param, (**param_type).clone());
            let body_type = infer_fix_type_structurally(&extended, body)?;

            // Build Pi type
            Ok(Term::Pi {
                param: param.clone(),
                param_type: param_type.clone(),
                body_type: Box::new(body_type),
            })
        }
        // For non-lambda bodies (the base case), we need to determine the return type.
        // This is typically a Match expression whose motive determines the return type.
        Term::Match { motive, .. } => {
            // The motive λx:I. T tells us the return type when applied to discriminant.
            // For a simple motive λ_. Nat, the return type is Nat.
            // We extract the body of the motive as the return type.
            if let Term::Lambda { body, .. } = motive.as_ref() {
                Ok((**body).clone())
            } else {
                // Motive is a raw type (constant motive) - return it directly
                // This handles cases like `match xs return Nat with ...`
                // where the return type is just `Nat`
                Ok((**motive).clone())
            }
        }
        // For other expressions, try normal inference
        _ => infer_type(ctx, term),
    }
}

/// Check that a term has the expected type (with subtyping/cumulativity).
///
/// Implements bidirectional type checking: when checking a Lambda against a Pi,
/// we can use the Pi's parameter type instead of the Lambda's (which may be a
/// placeholder from match case parsing).
fn check_type(ctx: &Context, term: &Term, expected: &Term) -> KernelResult<()> {
    // Hole as term: accept if expected is a Sort (Hole stands for a type)
    // This allows `Eq Hole X Y` where Eq expects Type as first arg
    if matches!(term, Term::Hole) {
        if matches!(expected, Term::Sort(_)) {
            return Ok(());
        }
        return Err(KernelError::TypeMismatch {
            expected: format!("{}", expected),
            found: "_".to_string(),
        });
    }

    // Hole as expected type: accept any well-typed term
    // This allows checking args against Hole in `(Eq Hole) X Y` intermediates
    if matches!(expected, Term::Hole) {
        let _ = infer_type(ctx, term)?; // Just verify term is well-typed
        return Ok(());
    }

    // Special case: Lambda with placeholder type checked against Pi
    // This handles match cases where binder types come from the expected type
    if let Term::Lambda {
        param,
        param_type,
        body,
    } = term
    {
        // Check if param_type is a placeholder ("_")
        if let Term::Global(name) = param_type.as_ref() {
            if name == "_" {
                // Bidirectional mode: get param type from expected
                if let Term::Pi {
                    param_type: expected_param_type,
                    body_type: expected_body_type,
                    param: expected_param,
                } = expected
                {
                    // Check body in extended context using expected param type
                    let extended_ctx = ctx.extend(param, (**expected_param_type).clone());
                    // Substitute in expected_body_type if param names differ
                    let body_expected = if param != expected_param {
                        substitute(expected_body_type, expected_param, &Term::Var(param.clone()))
                    } else {
                        (**expected_body_type).clone()
                    };
                    return check_type(&extended_ctx, body, &body_expected);
                }
            }
        }
    }

    let inferred = infer_type(ctx, term)?;
    if is_subtype(ctx, &inferred, expected) {
        Ok(())
    } else {
        Err(KernelError::TypeMismatch {
            expected: format!("{}", expected),
            found: format!("{}", inferred),
        })
    }
}

/// Infer the sort (universe) of a type.
///
/// A term is a type if its type is a Sort.
fn infer_sort(ctx: &Context, term: &Term) -> KernelResult<Universe> {
    let ty = infer_type(ctx, term)?;
    match ty {
        Term::Sort(u) => Ok(u),
        _ => Err(KernelError::NotAType(format!("{}", term))),
    }
}

/// Beta-reduce a term (single step, at the head).
///
/// (λx.body) arg → body[x := arg]
fn beta_reduce(term: &Term) -> Term {
    match term {
        Term::App(func, arg) => {
            match func.as_ref() {
                Term::Lambda { param, body, .. } => {
                    // Beta reduction: (λx.body) arg → body[x := arg]
                    substitute(body, param, arg)
                }
                _ => term.clone(),
            }
        }
        _ => term.clone(),
    }
}

/// Compute the expected type for a match case.
///
/// For a constructor C : A₁ → A₂ → ... → I,
/// the case type is: Πa₁:A₁. Πa₂:A₂. ... P(C a₁ a₂ ...)
///
/// For a zero-argument constructor like Zero : Nat,
/// the case type is just P(Zero).
///
/// For polymorphic constructors like Nil : Π(A:Type). List A,
/// when matching on `xs : List A`, we skip the type parameter
/// and use the instantiated type argument instead.
fn compute_case_type(motive: &Term, ctor_name: &str, ctor_type: &Term, disc_type: &Term) -> Term {
    // Extract type arguments from discriminant type
    // e.g., List A → [A], List → []
    let type_args = extract_type_args(disc_type);
    let num_type_args = type_args.len();

    // Collect parameters from constructor type
    let mut all_params: Vec<(String, Term)> = Vec::new();
    let mut current = ctor_type;

    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = current
    {
        all_params.push((param.clone(), (**param_type).clone()));
        current = body_type;
    }

    // Split into type parameters (to skip) and value parameters (to bind)
    let (type_params, value_params): (Vec<_>, Vec<_>) = all_params
        .into_iter()
        .enumerate()
        .partition(|(i, _)| *i < num_type_args);

    // Generate unique names for value parameters to avoid shadowing issues
    // Use pattern: __arg0, __arg1, etc.
    let named_value_params: Vec<(usize, (String, Term))> = value_params
        .into_iter()
        .enumerate()
        .map(|(i, (idx, (_, param_type)))| {
            (idx, (format!("__arg{}", i), param_type))
        })
        .collect();

    // Build C(type_args..., value_params...)
    let mut ctor_applied = Term::Global(ctor_name.to_string());

    // Apply type arguments (from discriminant type)
    for type_arg in &type_args {
        ctor_applied = Term::App(Box::new(ctor_applied), Box::new(type_arg.clone()));
    }

    // Apply value parameters (as bound variables with unique names)
    for (_, (param_name, _)) in &named_value_params {
        ctor_applied = Term::App(
            Box::new(ctor_applied),
            Box::new(Term::Var(param_name.clone())),
        );
    }

    // Build P(C type_args value_params) and beta-reduce it
    let motive_applied = Term::App(Box::new(motive.clone()), Box::new(ctor_applied));
    let result_type = beta_reduce(&motive_applied);

    // Wrap in Πa₁:A₁. Πa₂:A₂. ... for value parameters only
    // (in reverse order to get correct nesting)
    let mut case_type = result_type;
    for (_, (param_name, param_type)) in named_value_params.into_iter().rev() {
        // Substitute type arguments into parameter type
        let mut subst_param_type = param_type;
        for ((_, (type_param_name, _)), type_arg) in type_params.iter().zip(type_args.iter()) {
            subst_param_type = substitute(&subst_param_type, type_param_name, type_arg);
        }

        case_type = Term::Pi {
            param: param_name,
            param_type: Box::new(subst_param_type),
            body_type: Box::new(case_type),
        };
    }

    case_type
}

/// Extract type arguments from a type application.
///
/// - `List A` → `[A]`
/// - `Either A B` → `[A, B]`
/// - `Nat` → `[]`
fn extract_type_args(ty: &Term) -> Vec<Term> {
    let mut args = Vec::new();
    let mut current = ty;

    while let Term::App(func, arg) = current {
        args.push((**arg).clone());
        current = func;
    }

    args.reverse();
    args
}

/// Substitute a term for a variable: body[var := replacement]
///
/// This handles capture-avoidance by not substituting under
/// binders that shadow the variable.
pub fn substitute(body: &Term, var: &str, replacement: &Term) -> Term {
    match body {
        Term::Sort(u) => Term::Sort(u.clone()),

        // Literals are never substituted
        Term::Lit(lit) => Term::Lit(lit.clone()),

        // Holes are never substituted (they're implicit type placeholders)
        Term::Hole => Term::Hole,

        Term::Var(name) if name == var => replacement.clone(),
        Term::Var(name) => Term::Var(name.clone()),

        // Globals are never substituted (they're not bound variables)
        Term::Global(name) => Term::Global(name.clone()),

        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let new_param_type = substitute(param_type, var, replacement);
            // Don't substitute in body if the parameter shadows var
            let new_body_type = if param == var {
                (**body_type).clone()
            } else {
                substitute(body_type, var, replacement)
            };
            Term::Pi {
                param: param.clone(),
                param_type: Box::new(new_param_type),
                body_type: Box::new(new_body_type),
            }
        }

        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let new_param_type = substitute(param_type, var, replacement);
            // Don't substitute in body if the parameter shadows var
            let new_body = if param == var {
                (**body).clone()
            } else {
                substitute(body, var, replacement)
            };
            Term::Lambda {
                param: param.clone(),
                param_type: Box::new(new_param_type),
                body: Box::new(new_body),
            }
        }

        Term::App(func, arg) => Term::App(
            Box::new(substitute(func, var, replacement)),
            Box::new(substitute(arg, var, replacement)),
        ),

        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(substitute(discriminant, var, replacement)),
            motive: Box::new(substitute(motive, var, replacement)),
            cases: cases
                .iter()
                .map(|c| substitute(c, var, replacement))
                .collect(),
        },

        Term::Fix { name, body } => {
            // Don't substitute in body if the fixpoint name shadows var
            if name == var {
                Term::Fix {
                    name: name.clone(),
                    body: body.clone(),
                }
            } else {
                Term::Fix {
                    name: name.clone(),
                    body: Box::new(substitute(body, var, replacement)),
                }
            }
        }
    }
}

/// Check if type `a` is a subtype of type `b` (cumulativity).
///
/// Subtyping rules:
/// - Sort(u1) ≤ Sort(u2) if u1 ≤ u2 (universe cumulativity)
/// - Π(x:A). B ≤ Π(x:A'). B' if A' ≤ A (contravariant) and B ≤ B' (covariant)
/// - For other terms, normalize and compare structurally
pub fn is_subtype(ctx: &Context, a: &Term, b: &Term) -> bool {
    // Normalize both terms before comparison
    // This ensures that e.g. `ReachesOne (collatzStep 2)` equals `ReachesOne 1`
    let a_norm = normalize(ctx, a);
    let b_norm = normalize(ctx, b);

    is_subtype_normalized(ctx, &a_norm, &b_norm)
}

/// Check subtyping on already-normalized terms.
fn is_subtype_normalized(ctx: &Context, a: &Term, b: &Term) -> bool {
    match (a, b) {
        // Universe subtyping
        (Term::Sort(u1), Term::Sort(u2)) => u1.is_subtype_of(u2),

        // Pi subtyping (contravariant in param, covariant in body)
        (
            Term::Pi {
                param: p1,
                param_type: t1,
                body_type: b1,
            },
            Term::Pi {
                param: p2,
                param_type: t2,
                body_type: b2,
            },
        ) => {
            // Contravariant: t2 ≤ t1 (the expected param can be more specific)
            is_subtype_normalized(ctx, t2, t1) && {
                // Covariant: b1 ≤ b2 (alpha-rename to compare bodies)
                let b2_renamed = substitute(b2, p2, &Term::Var(p1.clone()));
                is_subtype_normalized(ctx, b1, &b2_renamed)
            }
        }

        // Fall back to structural equality for other terms
        _ => types_equal(a, b),
    }
}

/// Extract the inductive type name from a type.
///
/// Handles both:
/// - Simple inductives: `Nat` → Some("Nat")
/// - Polymorphic inductives: `List A` → Some("List")
///
/// Returns None if the type is not an inductive type.
fn extract_inductive_name(ctx: &Context, ty: &Term) -> Option<String> {
    match ty {
        // Simple case: Global("Nat")
        Term::Global(name) if ctx.is_inductive(name) => Some(name.clone()),

        // Polymorphic case: App(App(...App(Global("List"), _)...), _)
        // Recursively unwrap App to find the base Global
        Term::App(func, _) => extract_inductive_name(ctx, func),

        _ => None,
    }
}

/// Check if two types are equal (up to alpha-equivalence).
///
/// Two terms are alpha-equivalent if they are the same up to
/// renaming of bound variables.
fn types_equal(a: &Term, b: &Term) -> bool {
    // Hole matches anything (it's a type wildcard)
    if matches!(a, Term::Hole) || matches!(b, Term::Hole) {
        return true;
    }

    match (a, b) {
        (Term::Sort(u1), Term::Sort(u2)) => u1 == u2,

        (Term::Lit(l1), Term::Lit(l2)) => l1 == l2,

        (Term::Var(n1), Term::Var(n2)) => n1 == n2,

        (Term::Global(n1), Term::Global(n2)) => n1 == n2,

        (
            Term::Pi {
                param: p1,
                param_type: t1,
                body_type: b1,
            },
            Term::Pi {
                param: p2,
                param_type: t2,
                body_type: b2,
            },
        ) => {
            types_equal(t1, t2) && {
                // Alpha-equivalence: rename p2 to p1 in b2
                let b2_renamed = substitute(b2, p2, &Term::Var(p1.clone()));
                types_equal(b1, &b2_renamed)
            }
        }

        (
            Term::Lambda {
                param: p1,
                param_type: t1,
                body: b1,
            },
            Term::Lambda {
                param: p2,
                param_type: t2,
                body: b2,
            },
        ) => {
            types_equal(t1, t2) && {
                let b2_renamed = substitute(b2, p2, &Term::Var(p1.clone()));
                types_equal(b1, &b2_renamed)
            }
        }

        (Term::App(f1, a1), Term::App(f2, a2)) => types_equal(f1, f2) && types_equal(a1, a2),

        (
            Term::Match {
                discriminant: d1,
                motive: m1,
                cases: c1,
            },
            Term::Match {
                discriminant: d2,
                motive: m2,
                cases: c2,
            },
        ) => {
            types_equal(d1, d2)
                && types_equal(m1, m2)
                && c1.len() == c2.len()
                && c1.iter().zip(c2.iter()).all(|(a, b)| types_equal(a, b))
        }

        (
            Term::Fix {
                name: n1,
                body: b1,
            },
            Term::Fix {
                name: n2,
                body: b2,
            },
        ) => {
            // Alpha-equivalence: rename n2 to n1 in b2
            let b2_renamed = substitute(b2, n2, &Term::Var(n1.clone()));
            types_equal(b1, &b2_renamed)
        }

        _ => false,
    }
}
