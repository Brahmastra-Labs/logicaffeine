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

use crate::context::Context;
use crate::error::{KernelError, KernelResult};
use crate::reduction::normalize;
use crate::term::{Literal, Term, Universe};

/// Infer the type of a term in a context.
///
/// This is the main entry point for type checking. It implements bidirectional
/// type inference for the Calculus of Constructions.
///
/// # Type Rules
///
/// - `Type n : Type (n+1)` - Universes form a hierarchy
/// - `x : A` if `x : A` in context - Variable lookup
/// - `Π(x:A). B : Type max(i,j)` if `A : Type i` and `B : Type j`
/// - `λ(x:A). t : Π(x:A). B` if `t : B` in extended context
/// - `f a : B[x := a]` if `f : Π(x:A). B` and `a : A`
///
/// # Errors
///
/// Returns [`KernelError`] variants for:
/// - Unbound variables
/// - Type mismatches in applications
/// - Invalid match constructs
/// - Termination check failures for fixpoints
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

        // Const: a universe-polymorphic global at explicit levels. Look up its stored
        // universe parameters and type, then instantiate the parameters with `levels`.
        Term::Const { name, levels } => {
            let (params, ty, _body) = ctx
                .get_universe_poly(name)
                .ok_or_else(|| KernelError::UnboundVariable(name.clone()))?;
            if params.len() != levels.len() {
                return Err(KernelError::CertificationError(format!(
                    "universe-polymorphic '{}' expects {} level argument(s), got {}",
                    name,
                    params.len(),
                    levels.len()
                )));
            }
            let subst: std::collections::HashMap<String, Universe> =
                params.iter().cloned().zip(levels.iter().cloned()).collect();
            Ok(crate::term::instantiate_universes(&ty.clone(), &subst))
        }

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

            // Product formation (CIC). `Prop` is **impredicative**: a Π whose
            // codomain is a proposition is itself a proposition, no matter the
            // domain's universe — so `∀x:Entity. P(x)` is a `Prop`, and FOL
            // formulas built from it (And/Or/Ex over universals) stay in `Prop`
            // where `And`/`Ex` require their arguments to live. `imax` is exactly
            // this rule: `imax(a, Prop) = Prop`, `imax(a, non-Prop) = max(a, b)`,
            // and it stays SYMBOLIC when the codomain level is a variable (whose
            // Prop-ness is not yet known) — the case the old `_ => max` got wrong.
            let pi_sort = a_sort.imax(&b_sort);
            Ok(Term::Sort(pi_sort))
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

        // Let: `let x : A := v in b`. Check `A` is a type and `v : A`, then type
        // the body with `x` bound TRANSPARENTLY — by zeta-substituting `v` for
        // `x` (so `b`'s type sees `x ≡ v`, not an opaque hypothesis). This is
        // exactly zeta-expansion, so it is trivially sound and identical in both
        // kernels; the value is duplicated per occurrence during checking.
        Term::Let { name, ty, value, body } => {
            let _ = infer_sort(ctx, ty)?;
            check_type(ctx, value, ty)?;
            let unfolded = substitute(body, name, value);
            infer_type(ctx, &unfolded)
        }

        // App: (f a) : B[x := a] where f : Π(x:A). B and a : A
        Term::App(func, arg) => {
            let func_type = infer_type(ctx, func)?;
            // The function's type must be a Π, but it may be a redex that only REDUCES to
            // one — e.g. a recursor result `P x` whose motive `P` is a λ. Normalize to
            // expose the Π head before matching (the de Bruijn re-checker whnf's here too).
            let func_type = match func_type {
                Term::Pi { .. } => func_type,
                other => normalize(ctx, &other),
            };

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
            // 1. Discriminant must have an inductive type. Normalize first, so a
            // scrutinee whose type is a redex that reduces to an inductive (e.g. a
            // motive application `(λb. …) true ⇝ False`) is still recognized.
            let disc_type = normalize(ctx, &infer_type(ctx, discriminant)?);
            let inductive_name = extract_inductive_name(ctx, &disc_type)
                .ok_or_else(|| KernelError::NotAnInductive(format!("{}", disc_type)))?;

            // Parameter/index split. `num_params` leading arguments of the inductive are
            // uniform PARAMETERS fixed by the discriminant; any remaining arguments are
            // INDICES that vary per constructor, over which the motive abstracts (`Eq`'s
            // `P : Π(y:A). Eq A x y → Sort`). When there are no indices this is the
            // ordinary eliminator, which takes the original path below byte-for-byte.
            let disc_args = extract_type_args(&disc_type);
            let num_params = ctx.inductive_num_params(&inductive_name).min(disc_args.len());
            if disc_args.len() > num_params {
                return infer_indexed_match(
                    ctx,
                    discriminant,
                    motive,
                    cases,
                    &disc_type,
                    &inductive_name,
                    num_params,
                    &disc_args,
                );
            }

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
            let return_type = beta_reduce(&Term::App(Box::new(effective_motive), discriminant.clone()));

            // 6. CIC large-elimination restriction. A `Prop` inductive may be eliminated into a larger
            // sort (`Type`) ONLY if it is a subsingleton — zero constructors (so `ex falso` over `False`
            // stays legal), or exactly one whose non-parameter arguments are all proofs (`And`, `eq`).
            // Otherwise large elimination extracts computational content from a proof and breaks
            // consistency: large-eliminating `Or` would let a *proof* pick a `Type`-level value.
            if matches!(normalize(ctx, &infer_type(ctx, &disc_type)?), Term::Sort(Universe::Prop)) {
                let large = !matches!(normalize(ctx, &infer_type(ctx, &return_type)?), Term::Sort(Universe::Prop));
                if large && !is_subsingleton_prop(ctx, &inductive_name)? {
                    return Err(KernelError::InvalidMotive(format!(
                        "large elimination of proposition '{}' into a larger sort is not allowed: only \
                         subsingleton propositions (empty, or one constructor with propositional arguments \
                         — e.g. False, And, eq) may be eliminated into Type",
                        inductive_name
                    )));
                }
            }

            Ok(return_type)
        }

        // Literal: infer type based on literal kind
        Term::Lit(lit) => {
            match lit {
                Literal::Int(_) | Literal::BigInt(_) => Ok(Term::Global("Int".to_string())),
                Literal::Nat(n) if *n < logicaffeine_base::BigInt::from_i64(0) => {
                    Err(KernelError::CertificationError(
                        "a `Nat` literal must be non-negative".to_string(),
                    ))
                }
                Literal::Nat(_) => Ok(Term::Global("Nat".to_string())),
                Literal::Float(_) => Ok(Term::Global("Float".to_string())),
                Literal::Text(_) => Ok(Term::Global("Text".to_string())),
                Literal::Duration(_) => Ok(Term::Global("Duration".to_string())),
                Literal::Date(_) => Ok(Term::Global("Date".to_string())),
                Literal::Moment(_) => Ok(Term::Global("Moment".to_string())),
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
            crate::termination::check_termination(ctx, name, body)?;

            // Sanity check: verify the body is well-formed with f bound
            let extended = ctx.extend(name, structural_type.clone());
            let _ = infer_type(&extended, body)?;

            Ok(structural_type)
        }

        // MutualFix: a block of mutually-recursive definitions; this occurrence denotes
        // the `index`-th one. Each definition's type is inferred structurally (like the
        // single Fix), all names are put in scope, and the MUTUAL Giménez guard verifies
        // termination before the bodies are sanity-checked with every sibling bound.
        Term::MutualFix { defs, index } => {
            if defs.is_empty() || *index >= defs.len() {
                return Err(KernelError::CertificationError(
                    "mutual fixpoint with an empty block or out-of-range index".to_string(),
                ));
            }

            // Structural type of each definition (independent of the others' bodies).
            let mut types = Vec::with_capacity(defs.len());
            for (_, body) in defs {
                types.push(infer_fix_type_structurally(ctx, body)?);
            }

            // *** THE GUARDIAN: MUTUAL TERMINATION CHECK ***
            crate::termination::check_termination_mutual(ctx, defs)?;

            // Sanity: every body is well-formed with ALL names bound to their structural
            // types (a sibling call `rec_Odd n' o` sees `rec_Odd`'s type this way).
            let mut extended = ctx.clone();
            for ((name, _), ty) in defs.iter().zip(types.iter()) {
                extended = extended.extend(name, ty.clone());
            }
            for (_, body) in defs {
                let _ = infer_type(&extended, body)?;
            }

            Ok(types[*index].clone())
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
        // This is typically a Match whose motive determines it: the return type is the
        // `motive` applied to the discriminant's INDEX arguments (for an indexed family) and
        // then the discriminant itself, β-normalized. Computing the application — rather
        // than just reading off the motive's body — is robust to the motive's binder names
        // (which need not match the fixpoint's) and to indexed families. The discriminant is
        // the fixpoint's structural binder, in scope, so its type is available without the
        // not-yet-bound recursive name.
        Term::Match { discriminant, motive, .. } => {
            // A constant motive `return T` — one whose own type is a Sort — IS the result
            // type and is not applied to the discriminant. Only a function motive `λx. P x`
            // is applied to the discriminant's index arguments and then the discriminant
            // itself. This mirrors the Term::Match inference rule, which wraps a Sort-typed
            // motive as the constant `λ_:I. T` rather than applying it; without this guard a
            // constant motive `Nat`/`List A` becomes the ill-formed `App(Nat, n)`.
            if let Ok(mt) = infer_type(ctx, motive) {
                if matches!(normalize(ctx, &mt), Term::Sort(_)) {
                    return Ok(normalize(ctx, motive));
                }
            }
            let mut applied = (**motive).clone();
            if let Ok(dt) = infer_type(ctx, discriminant) {
                let dt = normalize(ctx, &dt);
                if let Some(ind) = extract_inductive_name(ctx, &dt) {
                    let args = extract_type_args(&dt);
                    let p = ctx.inductive_num_params(&ind).min(args.len());
                    for idx in &args[p..] {
                        applied = Term::App(Box::new(applied), Box::new(idx.clone()));
                    }
                }
            }
            applied = Term::App(Box::new(applied), discriminant.clone());
            Ok(normalize(ctx, &applied))
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

/// The RESULT sort of an inductive's arity — peel its leading `Π`s to the final `Sort`.
fn result_sort_universe(t: &Term) -> Option<Universe> {
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        cur = body_type;
    }
    match cur {
        Term::Sort(u) => Some(u.clone()),
        _ => None,
    }
}

/// Check the CIC UNIVERSE CONSTRAINT of an inductive constructor: every VALUE argument's
/// sort must be `≤` the inductive's result sort — so a `Type 0` inductive cannot store a
/// `Type 0`-typed field (which lives in `Type 1`), the universe inconsistency that opens
/// Girard/Hurkens paradoxes. A `Prop` inductive is exempt (impredicative `Prop` admits
/// arguments of any sort), exactly as in Coq/Lean. The inductive (and any mutual siblings
/// a recursive field references) must already be registered in `ctx`.
pub fn check_constructor_universes(
    ctx: &Context,
    ind: &str,
    ctor: &str,
    ty: &Term,
) -> KernelResult<()> {
    let ind_ty = match ctx.get_global(ind) {
        Some(t) => t.clone(),
        None => return Ok(()),
    };
    let target = match result_sort_universe(&ind_ty) {
        // Impredicative Prop: no constraint on argument universes.
        Some(Universe::Prop) => return Ok(()),
        Some(u) => u,
        None => return Ok(()),
    };
    let num_params = ctx.inductive_num_params(ind);
    // Walk the constructor telescope; the leading `num_params` are the inductive's uniform
    // parameters, the rest are stored VALUE fields subject to the constraint.
    let mut ext = ctx.clone();
    let mut cur = ty;
    let mut i = 0usize;
    while let Term::Pi { param, param_type, body_type } = cur {
        if i >= num_params {
            let s = infer_sort(&ext, param_type)?;
            if !s.is_subtype_of(&target) {
                return Err(KernelError::CertificationError(format!(
                    "universe inconsistency: constructor '{ctor}' stores an argument in sort \
                     {s}, which exceeds the sort {target} of its inductive '{ind}'"
                )));
            }
        }
        ext = ext.extend(param, (**param_type).clone());
        cur = body_type;
        i += 1;
    }
    Ok(())
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

/// Type an INDEXED match: the discriminant's inductive has `num_params` uniform
/// parameters and one or more trailing INDICES, and the `motive` abstracts over those
/// indices plus the scrutinee — `P : Π(indices…). Π(z : I params indices). Sort`.
///
/// The return type is `motive` applied to the discriminant's own index arguments and then
/// the discriminant itself; each constructor's case is checked against `motive` applied to
/// THAT constructor's result indices and the constructor value. Soundness rides on the
/// final `infer_type(return_type)` being a `Sort`: an ill-shaped motive makes those
/// applications fail to type-check, so nothing unsound slips through.
#[allow(clippy::too_many_arguments)]
fn infer_indexed_match(
    ctx: &Context,
    discriminant: &Term,
    motive: &Term,
    cases: &[Term],
    disc_type: &Term,
    inductive_name: &str,
    num_params: usize,
    disc_args: &[Term],
) -> KernelResult<Term> {
    // The discriminant's own parameter args (fixed) and index args (what the motive is
    // instantiated at for the *result* type).
    let disc_params = &disc_args[0..num_params];
    let disc_indices = &disc_args[num_params..];

    // The motive must at least type-check; its shape is enforced structurally by the case
    // and return-type checks below.
    let _ = infer_type(ctx, motive)?;

    // Coverage: exactly one case per constructor, in registration order.
    let constructors = ctx.get_constructors(inductive_name);
    if cases.len() != constructors.len() {
        return Err(KernelError::WrongNumberOfCases {
            expected: constructors.len(),
            found: cases.len(),
        });
    }

    // Each case against its indexed constructor type.
    for (case, (ctor_name, ctor_type)) in cases.iter().zip(constructors.iter()) {
        let expected = compute_indexed_case_type(motive, ctor_name, ctor_type, num_params, disc_params);
        check_type(ctx, case, &expected)?;
    }

    // Return type: `motive disc_index₁ … disc_indexₖ discriminant`, normalized.
    let mut ret = motive.clone();
    for idx in disc_indices {
        ret = Term::App(Box::new(ret), Box::new(idx.clone()));
    }
    ret = Term::App(Box::new(ret), Box::new(discriminant.clone()));
    let ret = normalize(ctx, &ret);

    // The result must be a type — this is what certifies the motive is a well-formed
    // family into a sort (a non-family motive would not infer to a `Sort` here).
    match normalize(ctx, &infer_type(ctx, &ret)?) {
        Term::Sort(_) => {}
        other => {
            return Err(KernelError::InvalidMotive(format!(
                "indexed match on '{}' has non-type result {} — the motive is not a family into a sort",
                inductive_name, other
            )));
        }
    }

    // CIC large-elimination restriction (identical rule to the non-indexed path).
    if matches!(normalize(ctx, &infer_type(ctx, disc_type)?), Term::Sort(Universe::Prop)) {
        let large = !matches!(normalize(ctx, &infer_type(ctx, &ret)?), Term::Sort(Universe::Prop));
        if large && !is_subsingleton_prop(ctx, inductive_name)? {
            return Err(KernelError::InvalidMotive(format!(
                "large elimination of proposition '{}' into a larger sort is not allowed",
                inductive_name
            )));
        }
    }

    Ok(ret)
}

/// The expected type of one constructor's case in an INDEXED match: the constructor's
/// leading `num_params` parameters are instantiated by the discriminant's `disc_params`;
/// its remaining value arguments become the case's `Π` binders; and the codomain is the
/// `motive` applied to the constructor's RESULT indices (as they appear in its declared
/// return type) and then the constructor value itself.
fn compute_indexed_case_type(
    motive: &Term,
    ctor_name: &str,
    ctor_type: &Term,
    num_params: usize,
    disc_params: &[Term],
) -> Term {
    // Peel the constructor's full Π telescope; the residual is its result `I params… idx…`.
    let mut all_params: Vec<(String, Term)> = Vec::new();
    let mut current = ctor_type;
    while let Term::Pi { param, param_type, body_type } = current {
        all_params.push((param.clone(), (**param_type).clone()));
        current = body_type;
    }
    let result_args = extract_type_args(current);

    let split = num_params.min(all_params.len());
    let param_binders = &all_params[0..split];
    // Fresh names for the value parameters (those past the inductive's parameters).
    let value_named: Vec<(String, String, Term)> = all_params[split..]
        .iter()
        .enumerate()
        .map(|(i, (orig, ty))| (orig.clone(), format!("__arg{}", i), ty.clone()))
        .collect();

    // Rewrite a term from the constructor's scope into the case's scope: parameter names →
    // the discriminant's parameter arguments, and each value parameter → its fresh name.
    // `upto` bounds which value parameters are already in scope (for dependent arg types).
    let rewrite = |t: &Term, upto: usize| -> Term {
        let mut out = t.clone();
        for (i, (name, _)) in param_binders.iter().enumerate() {
            if name != "_" {
                out = substitute(&out, name, &disc_params[i]);
            }
        }
        for (orig, fresh, _) in value_named.iter().take(upto) {
            if orig != "_" {
                out = substitute(&out, orig, &Term::Var(fresh.clone()));
            }
        }
        out
    };

    // The constructor's result index expressions (its result args past the parameters),
    // rewritten into the case's scope.
    let index_exprs: Vec<Term> = result_args
        .iter()
        .skip(split)
        .map(|e| beta_reduce(&rewrite(e, value_named.len())))
        .collect();

    // `C disc_params… value_vars…`.
    let mut ctor_applied = Term::Global(ctor_name.to_string());
    for pa in disc_params {
        ctor_applied = Term::App(Box::new(ctor_applied), Box::new(pa.clone()));
    }
    for (_, fresh, _) in &value_named {
        ctor_applied = Term::App(Box::new(ctor_applied), Box::new(Term::Var(fresh.clone())));
    }

    // `motive index_exprs… ctor_applied`, beta-reduced.
    let mut body = motive.clone();
    for e in &index_exprs {
        body = Term::App(Box::new(body), Box::new(e.clone()));
    }
    body = Term::App(Box::new(body), Box::new(ctor_applied));
    let mut case_type = beta_reduce(&body);

    // Re-wrap the value parameters as `Π`, each type closed into the case's scope.
    for k in (0..value_named.len()).rev() {
        let (_, fresh, ty_k) = &value_named[k];
        let pty = beta_reduce(&rewrite(ty_k, k));
        case_type = Term::Pi {
            param: fresh.clone(),
            param_type: Box::new(pty),
            body_type: Box::new(case_type),
        };
    }

    case_type
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

    // Split into type parameters (fixed by the discriminant) and value
    // parameters (bound by the case). The type parameters are the first
    // `num_type_args` constructor arguments.
    let type_params: Vec<(String, Term)> = all_params
        .iter()
        .take(num_type_args)
        .map(|(n, t)| (n.clone(), t.clone()))
        .collect();
    // For each value parameter keep (original name, fresh name, type). The
    // original name matters: a *dependent* constructor (e.g. `Ex`'s
    // `witness : … Π(x:A). P x → Ex A P`) has later argument types that mention
    // earlier value parameters, so those references must be rewritten to the
    // fresh names too — not just the type parameters.
    let value_named: Vec<(String, String, Term)> = all_params
        .into_iter()
        .skip(num_type_args)
        .enumerate()
        .map(|(i, (orig, ty))| (orig, format!("__arg{}", i), ty))
        .collect();

    // Build `C type_args… value_args…` with the fresh value-arg names.
    let mut ctor_applied = Term::Global(ctor_name.to_string());
    for type_arg in &type_args {
        ctor_applied = Term::App(Box::new(ctor_applied), Box::new(type_arg.clone()));
    }
    for (_, new_name, _) in &value_named {
        ctor_applied = Term::App(Box::new(ctor_applied), Box::new(Term::Var(new_name.clone())));
    }

    // `motive (C …)`, beta-reduced.
    let result_type = beta_reduce(&Term::App(Box::new(motive.clone()), Box::new(ctor_applied)));

    // Wrap in Π over the value parameters (reverse order for correct nesting).
    // Each parameter's type is closed by substituting the type parameters and
    // every *earlier* value parameter (original → fresh), then beta-reduced so a
    // dependent type like `P x` collapses to its applied form (e.g. `evil x`).
    let mut case_type = result_type;
    for k in (0..value_named.len()).rev() {
        let (_, new_name, ty_k) = &value_named[k];
        let mut pty = ty_k.clone();
        for ((tp_name, _), type_arg) in type_params.iter().zip(type_args.iter()) {
            pty = substitute(&pty, tp_name, type_arg);
        }
        for (orig_j, new_j, _) in value_named.iter().take(k) {
            pty = substitute(&pty, orig_j, &Term::Var(new_j.clone()));
        }
        let pty = beta_reduce(&pty);
        case_type = Term::Pi {
            param: new_name.clone(),
            param_type: Box::new(pty),
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

/// Substitute a term for a variable: `body[var := replacement]`.
///
/// Performs capture-avoiding substitution. Variables bound by lambda,
/// pi, or fix that shadow `var` are not substituted into.
///
/// # Capture Avoidance
///
/// Given `substitute(λx. y, "y", x)`, the result is `λx. x` (not `λx. x`
/// with the inner x captured). This implementation relies on unique
/// variable names from parsing.
///
/// # Term Forms
///
/// - `Sort`, `Lit`, `Hole`, `Global` - Unchanged (no variables)
/// - `Var(name)` - Replaced if `name == var`, unchanged otherwise
/// - `Pi`, `Lambda`, `Fix` - Substitute in components, respecting shadowing
/// - `App`, `Match` - Substitute recursively in all subterms
pub fn substitute(body: &Term, var: &str, replacement: &Term) -> Term {
    // Fast path: if `var` does not occur free in `body`, the substitution is the
    // identity. Crucially this lets us skip `free_vars(replacement)` — and the
    // `replacement` is, in `App`/`Match` type inference, the WHOLE argument/discriminant
    // proof term. A propositional implication's codomain never mentions the bound proof
    // variable (non-dependent), so this is the common case and turns quadratic checking
    // (walk the giant argument at every application) into linear.
    if !occurs_free(body, var) {
        return body.clone();
    }
    // Compute the free variables of the replacement once; a binder in `body`
    // that captures any of them must be alpha-renamed before we descend.
    let replacement_fvs = free_vars(replacement);
    substitute_avoiding(body, var, replacement, &replacement_fvs)
}

/// Whether `var` occurs free in `term` (binder-aware, short-circuiting).
fn occurs_free(term: &Term, var: &str) -> bool {
    match term {
        Term::Var(name) => name == var,
        Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Global(_) | Term::Const { .. } => false,
        Term::App(func, arg) => occurs_free(func, var) || occurs_free(arg, var),
        Term::Pi { param, param_type, body_type } => {
            occurs_free(param_type, var) || (param != var && occurs_free(body_type, var))
        }
        Term::Lambda { param, param_type, body } => {
            occurs_free(param_type, var) || (param != var && occurs_free(body, var))
        }
        Term::Fix { name, body } => name != var && occurs_free(body, var),
        Term::MutualFix { defs, .. } => {
            // `var` is free only if it is not one of the (all-binding) def names AND
            // occurs free in some body.
            !defs.iter().any(|(n, _)| n == var) && defs.iter().any(|(_, b)| occurs_free(b, var))
        }
        Term::Let { name, ty, value, body } => {
            occurs_free(ty, var)
                || occurs_free(value, var)
                || (name != var && occurs_free(body, var))
        }
        Term::Match { discriminant, motive, cases } => {
            occurs_free(discriminant, var)
                || occurs_free(motive, var)
                || cases.iter().any(|c| occurs_free(c, var))
        }
    }
}

/// Collect the free variables of a term (named representation).
fn free_vars(term: &Term) -> std::collections::HashSet<String> {
    fn go(term: &Term, bound: &mut Vec<String>, acc: &mut std::collections::HashSet<String>) {
        match term {
            Term::Var(name) => {
                if !bound.iter().any(|b| b == name) {
                    acc.insert(name.clone());
                }
            }
            Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Global(_) | Term::Const { .. } => {}
            Term::App(func, arg) => {
                go(func, bound, acc);
                go(arg, bound, acc);
            }
            Term::Pi { param, param_type, body_type } => {
                go(param_type, bound, acc);
                bound.push(param.clone());
                go(body_type, bound, acc);
                bound.pop();
            }
            Term::Lambda { param, param_type, body } => {
                go(param_type, bound, acc);
                bound.push(param.clone());
                go(body, bound, acc);
                bound.pop();
            }
            Term::Fix { name, body } => {
                bound.push(name.clone());
                go(body, bound, acc);
                bound.pop();
            }
            Term::MutualFix { defs, .. } => {
                for (n, _) in defs {
                    bound.push(n.clone());
                }
                for (_, b) in defs {
                    go(b, bound, acc);
                }
                for _ in defs {
                    bound.pop();
                }
            }
            Term::Let { name, ty, value, body } => {
                go(ty, bound, acc);
                go(value, bound, acc);
                bound.push(name.clone());
                go(body, bound, acc);
                bound.pop();
            }
            Term::Match { discriminant, motive, cases } => {
                go(discriminant, bound, acc);
                go(motive, bound, acc);
                for c in cases {
                    go(c, bound, acc);
                }
            }
        }
    }
    let mut acc = std::collections::HashSet::new();
    let mut bound = Vec::new();
    go(term, &mut bound, &mut acc);
    acc
}

/// Collect every variable name appearing in a term (bound or free).
fn all_var_names(term: &Term, acc: &mut std::collections::HashSet<String>) {
    match term {
        Term::Var(name) => {
            acc.insert(name.clone());
        }
        Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Global(_) | Term::Const { .. } => {}
        Term::App(func, arg) => {
            all_var_names(func, acc);
            all_var_names(arg, acc);
        }
        Term::Pi { param, param_type, body_type } => {
            acc.insert(param.clone());
            all_var_names(param_type, acc);
            all_var_names(body_type, acc);
        }
        Term::Lambda { param, param_type, body } => {
            acc.insert(param.clone());
            all_var_names(param_type, acc);
            all_var_names(body, acc);
        }
        Term::Fix { name, body } => {
            acc.insert(name.clone());
            all_var_names(body, acc);
        }
        Term::MutualFix { defs, .. } => {
            for (n, b) in defs {
                acc.insert(n.clone());
                all_var_names(b, acc);
            }
        }
        Term::Let { name, ty, value, body } => {
            acc.insert(name.clone());
            all_var_names(ty, acc);
            all_var_names(value, acc);
            all_var_names(body, acc);
        }
        Term::Match { discriminant, motive, cases } => {
            all_var_names(discriminant, acc);
            all_var_names(motive, acc);
            for c in cases {
                all_var_names(c, acc);
            }
        }
    }
}

/// Pick a binder name derived from `base` that collides with nothing in `avoid`.
fn fresh_name(base: &str, avoid: &std::collections::HashSet<String>) -> String {
    let mut candidate = format!("{}'", base);
    let mut counter: u32 = 0;
    while avoid.contains(&candidate) {
        counter += 1;
        candidate = format!("{}'{}", base, counter);
    }
    candidate
}

/// Choose a fresh binder name for `param`, avoiding the replacement's free vars,
/// every name occurring in `body`, and the variable being substituted.
fn freshen(
    param: &str,
    body: &Term,
    replacement_fvs: &std::collections::HashSet<String>,
    var: &str,
) -> String {
    let mut avoid = replacement_fvs.clone();
    all_var_names(body, &mut avoid);
    avoid.insert(var.to_string());
    fresh_name(param, &avoid)
}

/// Rename free occurrences of `from` to `to` in `term`. `to` must be globally
/// fresh in `term`, so this rename cannot itself capture.
fn rename_var(term: &Term, from: &str, to: &str) -> Term {
    let repl = Term::Var(to.to_string());
    let mut fvs = std::collections::HashSet::new();
    fvs.insert(to.to_string());
    substitute_avoiding(term, from, &repl, &fvs)
}

/// Capture-avoiding substitution `body[var := replacement]`, where
/// `replacement_fvs` is the precomputed free-variable set of `replacement`.
///
/// When a binder (`Pi`/`Lambda`/`Fix`) would capture a free variable of
/// `replacement`, the binder is alpha-renamed to a fresh name before the
/// substitution descends into its body.
fn substitute_avoiding(
    body: &Term,
    var: &str,
    replacement: &Term,
    replacement_fvs: &std::collections::HashSet<String>,
) -> Term {
    match body {
        Term::Sort(u) => Term::Sort(u.clone()),

        // A universe-polymorphic reference has no term variables to substitute.
        Term::Const { .. } => body.clone(),

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
            let new_param_type = substitute_avoiding(param_type, var, replacement, replacement_fvs);
            if param == var {
                // The parameter shadows `var`: do not substitute in the body.
                Term::Pi {
                    param: param.clone(),
                    param_type: Box::new(new_param_type),
                    body_type: (*body_type).clone(),
                }
            } else if replacement_fvs.contains(param) {
                // Capture-avoidance: rename the binder away from the free vars
                // of `replacement` before substituting into the body.
                let fresh = freshen(param, body_type, replacement_fvs, var);
                let renamed = rename_var(body_type, param, &fresh);
                Term::Pi {
                    param: fresh,
                    param_type: Box::new(new_param_type),
                    body_type: Box::new(substitute_avoiding(&renamed, var, replacement, replacement_fvs)),
                }
            } else {
                Term::Pi {
                    param: param.clone(),
                    param_type: Box::new(new_param_type),
                    body_type: Box::new(substitute_avoiding(body_type, var, replacement, replacement_fvs)),
                }
            }
        }

        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let new_param_type = substitute_avoiding(param_type, var, replacement, replacement_fvs);
            if param == var {
                // The parameter shadows `var`: do not substitute in the body.
                Term::Lambda {
                    param: param.clone(),
                    param_type: Box::new(new_param_type),
                    body: (*body).clone(),
                }
            } else if replacement_fvs.contains(param) {
                // Capture-avoidance: rename the binder away from the free vars
                // of `replacement` before substituting into the body.
                let fresh = freshen(param, body, replacement_fvs, var);
                let renamed = rename_var(body, param, &fresh);
                Term::Lambda {
                    param: fresh,
                    param_type: Box::new(new_param_type),
                    body: Box::new(substitute_avoiding(&renamed, var, replacement, replacement_fvs)),
                }
            } else {
                Term::Lambda {
                    param: param.clone(),
                    param_type: Box::new(new_param_type),
                    body: Box::new(substitute_avoiding(body, var, replacement, replacement_fvs)),
                }
            }
        }

        Term::App(func, arg) => Term::App(
            Box::new(substitute_avoiding(func, var, replacement, replacement_fvs)),
            Box::new(substitute_avoiding(arg, var, replacement, replacement_fvs)),
        ),

        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(substitute_avoiding(discriminant, var, replacement, replacement_fvs)),
            motive: Box::new(substitute_avoiding(motive, var, replacement, replacement_fvs)),
            cases: cases
                .iter()
                .map(|c| substitute_avoiding(c, var, replacement, replacement_fvs))
                .collect(),
        },

        Term::Fix { name, body } => {
            if name == var {
                // The fixpoint name shadows `var`: do not substitute in the body.
                Term::Fix {
                    name: name.clone(),
                    body: body.clone(),
                }
            } else if replacement_fvs.contains(name) {
                // Capture-avoidance: rename the fixpoint binder.
                let fresh = freshen(name, body, replacement_fvs, var);
                let renamed = rename_var(body, name, &fresh);
                Term::Fix {
                    name: fresh,
                    body: Box::new(substitute_avoiding(&renamed, var, replacement, replacement_fvs)),
                }
            } else {
                Term::Fix {
                    name: name.clone(),
                    body: Box::new(substitute_avoiding(body, var, replacement, replacement_fvs)),
                }
            }
        }

        Term::MutualFix { defs, index } => {
            // Every def name binds in EVERY body. If `var` is one of them it is shadowed
            // throughout — leave the whole block untouched.
            if defs.iter().any(|(n, _)| n == var) {
                return body.clone();
            }
            // Capture-avoidance: any def name that is a free var of `replacement` must be
            // α-renamed CONSISTENTLY across all bodies (it is one mutual binder shared by
            // all) before the substitution descends.
            let mut names: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
            let mut bodies: Vec<Term> = defs.iter().map(|(_, b)| b.clone()).collect();
            for i in 0..names.len() {
                if replacement_fvs.contains(&names[i]) {
                    let mut avoid = replacement_fvs.clone();
                    for b in &bodies {
                        all_var_names(b, &mut avoid);
                    }
                    for n in &names {
                        avoid.insert(n.clone());
                    }
                    avoid.insert(var.to_string());
                    let fresh = fresh_name(&names[i], &avoid);
                    for b in bodies.iter_mut() {
                        *b = rename_var(b, &names[i], &fresh);
                    }
                    names[i] = fresh;
                }
            }
            let new_defs = names
                .into_iter()
                .zip(bodies.iter())
                .map(|(n, b)| (n, substitute_avoiding(b, var, replacement, replacement_fvs)))
                .collect();
            Term::MutualFix { defs: new_defs, index: *index }
        }

        Term::Let { name, ty, value, body } => {
            // `ty` and `value` are outside the `name` binder — always substitute.
            let new_ty = substitute_avoiding(ty, var, replacement, replacement_fvs);
            let new_value = substitute_avoiding(value, var, replacement, replacement_fvs);
            if name == var {
                // The let-binder shadows `var`: leave the body untouched.
                Term::Let {
                    name: name.clone(),
                    ty: Box::new(new_ty),
                    value: Box::new(new_value),
                    body: body.clone(),
                }
            } else if replacement_fvs.contains(name) {
                // Capture-avoidance: rename the let-binder away from the
                // replacement's free vars before substituting into the body.
                let fresh = freshen(name, body, replacement_fvs, var);
                let renamed = rename_var(body, name, &fresh);
                Term::Let {
                    name: fresh,
                    ty: Box::new(new_ty),
                    value: Box::new(new_value),
                    body: Box::new(substitute_avoiding(&renamed, var, replacement, replacement_fvs)),
                }
            } else {
                Term::Let {
                    name: name.clone(),
                    ty: Box::new(new_ty),
                    value: Box::new(new_value),
                    body: Box::new(substitute_avoiding(body, var, replacement, replacement_fvs)),
                }
            }
        }
    }
}

/// Check if type `a` is a subtype of type `b` (cumulativity).
///
/// Implements the subtyping relation for the Calculus of Constructions
/// with cumulative universes.
///
/// # Subtyping Rules
///
/// - **Universe cumulativity**: `Type i <= Type j` if `i <= j`
/// - **Pi contravariance**: `Π(x:A). B <= Π(x:A'). B'` if `A' <= A` and `B <= B'`
/// - **Structural equality**: Other terms are compared after normalization
///
/// # Normalization
///
/// Both types are normalized before comparison, ensuring that definitionally
/// equal types are recognized as subtypes.
///
/// # Cumulativity Examples
///
/// - `Type 0 <= Type 1` (lower universe is subtype of higher)
/// - `Nat -> Type 0 <= Nat -> Type 1` (covariant in return type)
/// - `Type 1 -> Nat <= Type 0 -> Nat` (contravariant in parameter type)
pub fn is_subtype(ctx: &Context, a: &Term, b: &Term) -> bool {
    // Fast path: already structurally (definitionally) equal — the overwhelming case
    // in propositional/FOL proofs, where types are atoms and ∧/∨/¬/→ in normal form.
    // Skipping the two `normalize` walks here is the difference between linear and
    // pathological checking on a large certified grid proof.
    if types_equal(a, b) {
        return true;
    }
    // Otherwise normalize both terms before comparison — this ensures that e.g.
    // `ReachesOne (collatzStep 2)` equals `ReachesOne 1`.
    let a_norm = normalize(ctx, a);
    let b_norm = normalize(ctx, b);

    is_subtype_normalized(ctx, &a_norm, &b_norm)
}

/// Check subtyping on already-normalized terms. Cumulativity lives ONLY here: universes
/// follow `Prop ≤ Type i ≤ Type j`, and a `Π` is contravariant in its domain and covariant
/// in its codomain. Every other position is INVARIANT and delegates to [`def_eq_normalized`]
/// (definitional equality: reduction + η + proof irrelevance) — using cumulative subtyping
/// in an invariant position (e.g. a function argument) would be unsound.
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
                // Covariant: b1 ≤ b2 (alpha-rename to compare bodies, under the binder)
                let ext = ctx.extend(p1, (**t1).clone());
                let b2_renamed = substitute(b2, p2, &Term::Var(p1.clone()));
                is_subtype_normalized(&ext, b1, &b2_renamed)
            }
        }

        // Everything else is invariant: definitional equality.
        _ => def_eq_normalized(ctx, a, b),
    }
}

/// Definitional equality (symmetric): reduction, congruence, η, and proof irrelevance. This
/// is the conversion used in INVARIANT positions (function arguments, `Lambda`/`Match`/`Fix`
/// subterms). `is_subtype` layers cumulativity on top of it.
pub(crate) fn def_eq(ctx: &Context, a: &Term, b: &Term) -> bool {
    if types_equal(a, b) {
        return true;
    }
    let a_norm = normalize(ctx, a);
    let b_norm = normalize(ctx, b);
    def_eq_normalized(ctx, &a_norm, &b_norm)
}

/// Decompose a term into its head and argument spine (`f a b c` → `(f, [a,b,c])`).
fn spine_of(t: &Term) -> (&Term, Vec<&Term>) {
    let mut args = Vec::new();
    let mut head = t;
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f;
    }
    args.reverse();
    (head, args)
}

/// Structure η in ONE direction: if `mk_term` is a fully-applied constructor of a
/// registered structure `S` (`S_mk p̄ ā`) and `other` is not itself constructor-
/// headed, return `Some(eq)` where `eq` decides `mk_term ≡ other` by comparing each
/// field `aᵢ` against `S_projᵢ p̄ other`. `None` when the shape does not apply, so the
/// caller falls through to ordinary congruence.
fn try_structure_eta(ctx: &Context, mk_term: &Term, other: &Term) -> Option<bool> {
    let (head, args) = spine_of(mk_term);
    let Term::Global(hname) = head else { return None };
    let (_sname, info) = ctx.struct_of_constructor(hname)?;
    let nfields = info.projections.len();
    // Must be fully applied: params + one argument per field.
    if args.len() != info.num_params + nfields {
        return None;
    }
    // Do not eta when `other` is ALSO this constructor — that is ordinary
    // congruence (and avoids a pointless expansion loop).
    let (ohead, _) = spine_of(other);
    if matches!(ohead, Term::Global(n) if n == hname) {
        return None;
    }
    let params = &args[..info.num_params];
    let field_args = &args[info.num_params..];
    // Each field argument must equal the projection of `other`.
    Some(info.projections.iter().enumerate().all(|(i, proj)| {
        let mut proj_applied = Term::Global(proj.clone());
        for p in params {
            proj_applied = Term::App(Box::new(proj_applied), Box::new((*p).clone()));
        }
        proj_applied = Term::App(Box::new(proj_applied), Box::new(other.clone()));
        def_eq(ctx, field_args[i], &proj_applied)
    }))
}

/// True if `t` is the head of a `Nat` Peano value — `Zero` or `Succ _` — the shape a
/// `Nat` literal bridges against (K6).
fn is_nat_peano_headed(t: &Term) -> bool {
    match t {
        Term::Global(n) => n == "Zero",
        Term::App(f, _) => matches!(f.as_ref(), Term::Global(n) if n == "Succ"),
        _ => false,
    }
}

/// One Peano-unfolding step of a `Nat` literal: `Nat(0) → Zero`, `Nat(n) → Succ (Nat(n-1))`.
fn nat_peano_step(t: &Term) -> Term {
    match t {
        // `n ≤ 0` collapses to `Zero`, so peeling always TERMINATES even on a malformed
        // negative literal (a well-formed Nat is non-negative).
        Term::Lit(Literal::Nat(n)) if *n <= logicaffeine_base::BigInt::from_i64(0) => {
            Term::Global("Zero".to_string())
        }
        Term::Lit(Literal::Nat(n)) => Term::App(
            Box::new(Term::Global("Succ".to_string())),
            Box::new(Term::Lit(Literal::Nat(n.sub(&logicaffeine_base::BigInt::from_i64(1))))),
        ),
        other => other.clone(),
    }
}

/// Definitional equality on already-normalized terms.
fn def_eq_normalized(ctx: &Context, a: &Term, b: &Term) -> bool {
    if types_equal(a, b) {
        return true;
    }

    // η-conversion: `f ≡ λx. f x`. When exactly one side is a λ, compare its body against
    // the other side applied to the bound variable, under the binder.
    if let Term::Lambda { param, param_type, body } = a {
        if !matches!(b, Term::Lambda { .. }) {
            let ext = ctx.extend(param, (**param_type).clone());
            let bx = normalize(ctx, &Term::App(Box::new(b.clone()), Box::new(Term::Var(param.clone()))));
            return def_eq_normalized(&ext, body, &bx);
        }
    }
    if let Term::Lambda { param, param_type, body } = b {
        if !matches!(a, Term::Lambda { .. }) {
            let ext = ctx.extend(param, (**param_type).clone());
            let ax = normalize(ctx, &Term::App(Box::new(a.clone()), Box::new(Term::Var(param.clone()))));
            return def_eq_normalized(&ext, &ax, body);
        }
    }

    // Structure η: `⟨p.1, …, p.n⟩ ≡ p`. When one side is a fully-applied
    // constructor of a REGISTERED structure and the other is not constructor-
    // headed, compare each field argument against the matching projection of the
    // other side. Keyed on the structure registry, so it never fires for an
    // ordinary inductive.
    if let Some(eq) = try_structure_eta(ctx, a, b) {
        return eq;
    }
    if let Some(eq) = try_structure_eta(ctx, b, a) {
        return eq;
    }

    // Peano bridge (K6): a `Nat(n)` literal is definitionally `Succ^n Zero`. Two Nat
    // literals are equal iff their counts are; a Nat literal and a `Zero`/`Succ`-headed
    // Peano term are compared by peeling one `Succ` at a time (terminating at
    // `Nat(0) ≡ Zero`). Sound because the bridge unfolds to EXACTLY `Succ^n Zero`.
    match (a, b) {
        (Term::Lit(Literal::Nat(x)), Term::Lit(Literal::Nat(y))) => return x == y,
        (Term::Lit(Literal::Nat(_)), _) if is_nat_peano_headed(b) => {
            return def_eq_normalized(ctx, &nat_peano_step(a), b);
        }
        (_, Term::Lit(Literal::Nat(_))) if is_nat_peano_headed(a) => {
            return def_eq_normalized(ctx, a, &nat_peano_step(b));
        }
        _ => {}
    }

    let congruent = match (a, b) {
        (Term::Sort(u1), Term::Sort(u2)) => u1.equiv(u2),
        (
            Term::Pi { param: p1, param_type: t1, body_type: b1 },
            Term::Pi { param: p2, param_type: t2, body_type: b2 },
        ) => {
            def_eq_normalized(ctx, t1, t2) && {
                let ext = ctx.extend(p1, (**t1).clone());
                let b2r = substitute(b2, p2, &Term::Var(p1.clone()));
                def_eq_normalized(&ext, b1, &b2r)
            }
        }
        (
            Term::Lambda { param: p1, param_type: t1, body: b1 },
            Term::Lambda { param: p2, param_type: t2, body: b2 },
        ) => {
            def_eq_normalized(ctx, t1, t2) && {
                let ext = ctx.extend(p1, (**t1).clone());
                let b2r = substitute(b2, p2, &Term::Var(p1.clone()));
                def_eq_normalized(&ext, b1, &b2r)
            }
        }
        (Term::App(f1, a1), Term::App(f2, a2)) => {
            def_eq_normalized(ctx, f1, f2) && def_eq_normalized(ctx, a1, a2)
        }
        (
            Term::Match { discriminant: d1, motive: m1, cases: c1 },
            Term::Match { discriminant: d2, motive: m2, cases: c2 },
        ) => {
            def_eq_normalized(ctx, d1, d2)
                && def_eq_normalized(ctx, m1, m2)
                && c1.len() == c2.len()
                && c1.iter().zip(c2.iter()).all(|(x, y)| def_eq_normalized(ctx, x, y))
        }
        (Term::Fix { name: n1, body: b1 }, Term::Fix { name: n2, body: b2 }) => {
            let b2r = substitute(b2, n2, &Term::Var(n1.clone()));
            def_eq_normalized(ctx, b1, &b2r)
        }
        _ => false,
    };
    if congruent {
        return true;
    }

    // Proof irrelevance: any two proofs of the same proposition are equal. Fires only when
    // structural comparison fails (so it never costs on the common path).
    proof_irrelevant(ctx, a, b)
}

/// Proof irrelevance: `a ≡ b` if `a`'s type is a proposition and `b` has a definitionally
/// equal type — i.e. both are proofs of the same `Prop`. Sound because `Prop` is a universe
/// of proof-irrelevant propositions.
fn proof_irrelevant(ctx: &Context, a: &Term, b: &Term) -> bool {
    let ta = match infer_type(ctx, a) {
        Ok(t) => t,
        Err(_) => return false,
    };
    // `a`'s type must itself be a proposition (`ta : Prop` or the definitionally-irrelevant
    // `ta : SProp`).
    match infer_type(ctx, &ta) {
        Ok(sort)
            if matches!(
                normalize(ctx, &sort),
                Term::Sort(Universe::Prop) | Term::Sort(Universe::SProp)
            ) => {}
        _ => return false,
    }
    // `b` must be a proof of a definitionally-equal proposition.
    match infer_type(ctx, b) {
        Ok(tb) => def_eq(ctx, &ta, &tb),
        Err(_) => false,
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
        (Term::Sort(u1), Term::Sort(u2)) => u1.equiv(u2),

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

/// Whether a `Prop` inductive may be **large-eliminated** (into `Type`): true iff it is a subsingleton —
/// zero constructors, or exactly one whose non-parameter arguments all live in `Prop`. This is the CIC
/// elimination criterion that keeps `False` (ex falso), `And`, and `eq` eliminable while forbidding
/// multi-constructor propositions like `Or` and existentials carrying a `Type`-level witness.
pub(crate) fn is_subsingleton_prop(ctx: &Context, inductive_name: &str) -> KernelResult<bool> {
    let ctors = ctx.get_constructors(inductive_name);
    match ctors.len() {
        0 => Ok(true),
        1 => {
            let ctor_type = ctors[0].1.clone();
            let ind_type = ctx
                .get_global(inductive_name)
                .cloned()
                .ok_or_else(|| KernelError::UnboundVariable(inductive_name.to_string()))?;
            // The inductive's arity prefix (parameters + indices) is not "data"; only constructor
            // arguments beyond it carry content and must be propositional.
            let arity = pi_param_count(&ind_type);
            let mut local = ctx.clone();
            let mut t = ctor_type;
            let mut i = 0;
            while let Term::Pi { param, param_type, body_type } = t {
                if i >= arity && infer_sort(&local, &param_type)? != Universe::Prop {
                    return Ok(false);
                }
                local = local.extend(&param, (*param_type).clone());
                t = *body_type;
                i += 1;
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Count the leading `Π` binders of a type (an inductive's arity).
fn pi_param_count(ty: &Term) -> usize {
    match ty {
        Term::Pi { body_type, .. } => 1 + pi_param_count(body_type),
        _ => 0,
    }
}

#[cfg(test)]
mod large_elim_tests {
    use super::*;
    use crate::context::Context;
    use crate::prelude::StandardLibrary;
    use crate::term::{Term, Universe};

    fn g(s: &str) -> Term { Term::Global(s.to_string()) }
    fn app(f: Term, x: Term) -> Term { Term::App(Box::new(f), Box::new(x)) }
    fn lam(p: &str, ty: Term, body: Term) -> Term {
        Term::Lambda { param: p.to_string(), param_type: Box::new(ty), body: Box::new(body) }
    }
    fn or_tt() -> Term { app(app(g("Or"), g("True")), g("True")) }

    /// CRITIQUE #1 (open half): CIC large-elimination restriction. Matching a proof of `Or` (a Prop with
    /// TWO constructors) into `Type` (here returning `Nat`) extracts computational content from a proof
    /// and breaks consistency — the kernel MUST reject it.
    #[test]
    fn large_elimination_of_or_into_type_is_rejected() {
        let mut ctx = Context::new();
        StandardLibrary::register(&mut ctx);
        let case = lam("_", g("True"), g("Zero")); // λ_:True. Zero  (Zero : Nat)
        let m = Term::Match {
            discriminant: Box::new(Term::Var("h".to_string())),
            motive: Box::new(lam("_", or_tt(), g("Nat"))), // λ_:Or True True. Nat  (large)
            cases: vec![case.clone(), case],
        };
        let term = lam("h", or_tt(), m);
        assert!(
            infer_type(&ctx, &term).is_err(),
            "large elimination of Or (2 constructors) into Type must be rejected for consistency"
        );
    }

    /// Regression: `ex falso` — large elimination of `False` (ZERO constructors, a subsingleton) into any
    /// type — MUST stay legal, or every proof-by-contradiction breaks.
    #[test]
    fn ex_falso_large_elimination_of_false_still_allowed() {
        let mut ctx = Context::new();
        StandardLibrary::register(&mut ctx);
        let m = Term::Match {
            discriminant: Box::new(Term::Var("h".to_string())),
            motive: Box::new(lam("_", g("False"), g("Nat"))), // λ_:False. Nat (large, but False is empty)
            cases: vec![],
        };
        let term = lam("h", g("False"), m);
        assert!(infer_type(&ctx, &term).is_ok(), "ex falso (large elim of empty False) must stay legal");
    }

    /// Regression: SMALL elimination of `Or` (into a `Prop`) is always fine — the restriction must not
    /// over-reach and reject ordinary propositional case analysis.
    #[test]
    fn small_elimination_of_or_into_prop_still_allowed() {
        let mut ctx = Context::new();
        StandardLibrary::register(&mut ctx);
        let case = lam("_", g("True"), g("I")); // λ_:True. I  (I : True)
        let m = Term::Match {
            discriminant: Box::new(Term::Var("h".to_string())),
            motive: Box::new(lam("_", or_tt(), g("True"))), // λ_:Or True True. True  (small, Prop)
            cases: vec![case.clone(), case],
        };
        let term = lam("h", or_tt(), m);
        assert!(infer_type(&ctx, &term).is_ok(), "small elimination of Or into Prop must stay legal");
    }
}
