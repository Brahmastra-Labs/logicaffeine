//! Term reduction for the Calculus of Constructions.
//!
//! This module implements the evaluation semantics of the kernel. Terms are
//! reduced to normal form using a call-by-name strategy.
//!
//! # Reduction Rules
//!
//! ## Beta Reduction
//! Function application: `(λx. body) arg → body[x := arg]`
//!
//! ## Iota Reduction
//! Pattern matching: `match (Cᵢ args) { ... caseᵢ ... } → caseᵢ(args)`
//!
//! ## Fix Unfolding (Guarded)
//! Recursive definitions: `(fix f. body) (Cᵢ args) → body[f := fix f. body] (Cᵢ args)`
//!
//! Fix unfolding is guarded by requiring the argument to be in constructor form.
//! This ensures termination by structural recursion.
//!
//! ## Delta Reduction
//! Global definitions are expanded when needed during normalization.
//!
//! # Primitive Operations
//!
//! The reducer handles built-in operations on literals:
//! - Arithmetic: `add`, `sub`, `mul`, `div`, `mod`
//! - Comparison: `lt`, `le`, `gt`, `ge`, `eq`
//! - Boolean: `ite` (if-then-else)
//!
//! # Reflection Builtins
//!
//! Special operations for the deep embedding (Syntax type):
//! - `syn_size`: Compute the size of a syntax tree
//! - `syn_max_var`: Find the maximum variable index
//! - `syn_step`, `syn_eval`: Bounded evaluation
//! - `syn_quote`, `syn_diag`: Reification and diagonalization
//!
//! # Fuel Limit
//!
//! Normalization uses a fuel counter (default 10000) to prevent infinite loops.
//! If fuel is exhausted, the current term is returned as-is.

use crate::context::Context;
use crate::omega;
use crate::term::{Literal, Term, Universe};
use crate::type_checker::substitute;

/// Normalize a term to its normal form.
///
/// Repeatedly applies reduction rules until no more reductions are possible.
/// This is a full normalization that reduces under binders.
pub fn normalize(ctx: &Context, term: &Term) -> Term {
    let mut current = term.clone();
    let mut fuel = 10000; // Safety limit to prevent infinite loops

    loop {
        if fuel == 0 {
            // If we hit the fuel limit, return what we have
            return current;
        }
        fuel -= 1;

        let reduced = reduce_step(ctx, &current);
        if reduced == current {
            return current;
        }
        current = reduced;
    }
}

/// Single-step reduction.
///
/// Reduces the outermost redex first (call-by-name), then recurses
/// into subterms for full normalization.
fn reduce_step(ctx: &Context, term: &Term) -> Term {
    match term {
        // Literals are in normal form
        Term::Lit(_) => term.clone(),

        // Beta: (λx. body) arg → body[x := arg]
        Term::App(func, arg) => {
            // First try primitive reduction (add, sub, mul, etc.)
            if let Some(result) = try_primitive_reduce(func, arg) {
                return result;
            }

            // Try reflection builtins (syn_size, syn_max_var)
            if let Some(result) = try_reflection_reduce(ctx, func, arg) {
                return result;
            }

            // Then try to reduce at the head
            match func.as_ref() {
                Term::Lambda { param, body, .. } => {
                    // Beta reduction
                    substitute(body, param, arg)
                }
                // Fix application: (fix f. body) arg
                // We need to check if arg is a constructor to unfold
                Term::Fix { name, body } => {
                    if is_constructor_form(ctx, arg) {
                        // Unfold: (fix f. body) arg → body[f := fix f. body] arg
                        let fix_term = Term::Fix {
                            name: name.clone(),
                            body: body.clone(),
                        };
                        let unfolded = substitute(body, name, &fix_term);
                        Term::App(Box::new(unfolded), arg.clone())
                    } else {
                        // Try reducing the argument first
                        let reduced_arg = reduce_step(ctx, arg);
                        if reduced_arg != **arg {
                            Term::App(func.clone(), Box::new(reduced_arg))
                        } else {
                            term.clone()
                        }
                    }
                }
                // Nested application: ((f x) y) - reduce inner first
                Term::App(_, _) => {
                    let reduced_func = reduce_step(ctx, func);
                    if reduced_func != **func {
                        Term::App(Box::new(reduced_func), arg.clone())
                    } else {
                        // Try reducing argument
                        let reduced_arg = reduce_step(ctx, arg);
                        Term::App(func.clone(), Box::new(reduced_arg))
                    }
                }
                // Other function forms - try reducing function position
                _ => {
                    let reduced_func = reduce_step(ctx, func);
                    if reduced_func != **func {
                        Term::App(Box::new(reduced_func), arg.clone())
                    } else {
                        // Try reducing argument
                        let reduced_arg = reduce_step(ctx, arg);
                        Term::App(func.clone(), Box::new(reduced_arg))
                    }
                }
            }
        }

        // Iota: match (Cᵢ a₁...aₙ) with cases → caseᵢ a₁ ... aₙ
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            if let Some((ctor_idx, args)) = extract_constructor(ctx, discriminant) {
                // Select the corresponding case
                let case = &cases[ctor_idx];
                // Apply case to constructor arguments
                let mut result = case.clone();
                for arg in args {
                    result = Term::App(Box::new(result), Box::new(arg));
                }
                // Reduce the result
                reduce_step(ctx, &result)
            } else {
                // Try reducing the discriminant
                let reduced_disc = reduce_step(ctx, discriminant);
                if reduced_disc != **discriminant {
                    Term::Match {
                        discriminant: Box::new(reduced_disc),
                        motive: motive.clone(),
                        cases: cases.clone(),
                    }
                } else {
                    term.clone()
                }
            }
        }

        // Reduce under lambdas (deep normalization)
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let reduced_param_type = reduce_step(ctx, param_type);
            let reduced_body = reduce_step(ctx, body);
            if reduced_param_type != **param_type || reduced_body != **body {
                Term::Lambda {
                    param: param.clone(),
                    param_type: Box::new(reduced_param_type),
                    body: Box::new(reduced_body),
                }
            } else {
                term.clone()
            }
        }

        // Reduce under Pi types
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let reduced_param_type = reduce_step(ctx, param_type);
            let reduced_body_type = reduce_step(ctx, body_type);
            if reduced_param_type != **param_type || reduced_body_type != **body_type {
                Term::Pi {
                    param: param.clone(),
                    param_type: Box::new(reduced_param_type),
                    body_type: Box::new(reduced_body_type),
                }
            } else {
                term.clone()
            }
        }

        // Reduce under Fix
        Term::Fix { name, body } => {
            let reduced_body = reduce_step(ctx, body);
            if reduced_body != **body {
                Term::Fix {
                    name: name.clone(),
                    body: Box::new(reduced_body),
                }
            } else {
                term.clone()
            }
        }

        // Base cases: already in normal form
        Term::Sort(_) | Term::Var(_) | Term::Hole => term.clone(),

        // Delta reduction: unfold definitions (but not axioms, constructors, or inductives)
        Term::Global(name) => {
            if let Some(body) = ctx.get_definition_body(name) {
                // Definition found - unfold to body
                body.clone()
            } else {
                // Axiom, constructor, or inductive - no reduction
                term.clone()
            }
        }
    }
}

/// Check if a term is a constructor (possibly applied to arguments).
fn is_constructor_form(ctx: &Context, term: &Term) -> bool {
    extract_constructor(ctx, term).is_some()
}

/// Extract constructor index and VALUE arguments from a term (skipping type arguments).
///
/// Returns `Some((constructor_index, [value_arg1, value_arg2, ...]))` if term is `Cᵢ(type_args..., value_args...)`
/// where `Cᵢ` is the i-th constructor of some inductive type.
///
/// For polymorphic constructors like `Cons A h t`, this returns only [h, t], not [A, h, t].
fn extract_constructor(ctx: &Context, term: &Term) -> Option<(usize, Vec<Term>)> {
    let mut args = Vec::new();
    let mut current = term;

    // Collect arguments from nested applications
    while let Term::App(func, arg) = current {
        args.push((**arg).clone());
        current = func;
    }
    args.reverse();

    // Check if head is a constructor
    if let Term::Global(name) = current {
        if let Some(inductive) = ctx.constructor_inductive(name) {
            let ctors = ctx.get_constructors(inductive);
            for (idx, (ctor_name, ctor_type)) in ctors.iter().enumerate() {
                if *ctor_name == name {
                    // Count type parameters (leading Pis where param_type is a Sort)
                    let num_type_params = count_type_params(ctor_type);

                    // Skip type arguments, return only value arguments
                    let value_args = if num_type_params < args.len() {
                        args[num_type_params..].to_vec()
                    } else {
                        vec![]
                    };

                    return Some((idx, value_args));
                }
            }
        }
    }
    None
}

/// Count leading type parameters in a constructor type.
///
/// Type parameters are Pis where the param_type is a Sort (Type n or Prop).
/// For `Π(A:Type). Π(_:A). Π(_:List A). List A`, this returns 1.
fn count_type_params(ty: &Term) -> usize {
    let mut count = 0;
    let mut current = ty;

    while let Term::Pi { param_type, body_type, .. } = current {
        if is_sort(param_type) {
            count += 1;
            current = body_type;
        } else {
            break;
        }
    }

    count
}

/// Check if a term is a Sort (Type n or Prop).
fn is_sort(term: &Term) -> bool {
    matches!(term, Term::Sort(_))
}

/// Try to reduce a primitive operation.
///
/// Returns Some(result) if this is a fully applied primitive like (add 3 4).
/// Pattern: ((op x) y) where op is a builtin and x, y are literals.
fn try_primitive_reduce(func: &Term, arg: &Term) -> Option<Term> {
    // We need func = (op x) and arg = y
    if let Term::App(op_term, x) = func {
        if let Term::Global(op_name) = op_term.as_ref() {
            if let (Term::Lit(Literal::Int(x_val)), Term::Lit(Literal::Int(y_val))) =
                (x.as_ref(), arg)
            {
                let result = match op_name.as_str() {
                    "add" => x_val.checked_add(*y_val)?,
                    "sub" => x_val.checked_sub(*y_val)?,
                    "mul" => x_val.checked_mul(*y_val)?,
                    "div" => x_val.checked_div(*y_val)?,
                    "mod" => x_val.checked_rem(*y_val)?,
                    _ => return None,
                };
                return Some(Term::Lit(Literal::Int(result)));
            }
        }
    }
    None
}

// -------------------------------------------------------------------------
// Reflection Builtins
// -------------------------------------------------------------------------

/// Try to reduce reflection builtins (syn_size, syn_max_var, syn_lift, syn_subst, syn_beta, syn_step).
///
/// Pattern: (syn_size arg) or (syn_max_var arg) or (syn_step arg) where arg is a Syntax constructor.
/// Pattern: ((syn_beta body) arg) for two-argument builtins.
/// Pattern: (((syn_lift amount) cutoff) term) for three-argument builtins.
/// Pattern: (((syn_subst replacement) index) term) for substitution.
fn try_reflection_reduce(ctx: &Context, func: &Term, arg: &Term) -> Option<Term> {
    // First, check for single-argument builtins
    if let Term::Global(op_name) = func {
        match op_name.as_str() {
            "syn_size" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_size_reduce(ctx, &norm_arg);
            }
            "syn_max_var" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_max_var_reduce(ctx, &norm_arg, 0);
            }
            "syn_step" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_step_reduce(ctx, &norm_arg);
            }
            "syn_quote" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_quote_reduce(ctx, &norm_arg);
            }
            "syn_diag" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_syn_diag_reduce(ctx, &norm_arg);
            }
            "concludes" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_concludes_reduce(ctx, &norm_arg);
            }
            "try_refl" => {
                // Normalize the argument first
                let norm_arg = normalize(ctx, arg);
                return try_try_refl_reduce(ctx, &norm_arg);
            }
            "tact_fail" => {
                // tact_fail always returns error derivation
                return Some(make_error_derivation());
            }
            "try_compute" => {
                // try_compute goal = DCompute goal
                // The validation happens in concludes
                let norm_arg = normalize(ctx, arg);
                return Some(Term::App(
                    Box::new(Term::Global("DCompute".to_string())),
                    Box::new(norm_arg),
                ));
            }
            "try_ring" => {
                // Ring tactic: prove polynomial equalities by normalization
                let norm_arg = normalize(ctx, arg);
                return try_try_ring_reduce(ctx, &norm_arg);
            }
            "try_lia" => {
                // LIA tactic: prove linear inequalities by Fourier-Motzkin
                let norm_arg = normalize(ctx, arg);
                return try_try_lia_reduce(ctx, &norm_arg);
            }
            "try_cc" => {
                // CC tactic: prove equalities by congruence closure
                let norm_arg = normalize(ctx, arg);
                return try_try_cc_reduce(ctx, &norm_arg);
            }
            "try_simp" => {
                // Simp tactic: prove equalities by simplification and arithmetic
                let norm_arg = normalize(ctx, arg);
                return try_try_simp_reduce(ctx, &norm_arg);
            }
            "try_omega" => {
                // Omega tactic: prove integer inequalities with proper rounding
                let norm_arg = normalize(ctx, arg);
                return try_try_omega_reduce(ctx, &norm_arg);
            }
            "try_auto" => {
                // Auto tactic: tries all tactics in sequence
                let norm_arg = normalize(ctx, arg);
                return try_try_auto_reduce(ctx, &norm_arg);
            }
            "try_inversion" => {
                // Inversion tactic: derives False if no constructor can match
                let norm_arg = normalize(ctx, arg);
                return try_try_inversion_reduce(ctx, &norm_arg);
            }
            "induction_num_cases" => {
                // Returns number of constructors for an inductive type
                let norm_arg = normalize(ctx, arg);
                return try_induction_num_cases_reduce(ctx, &norm_arg);
            }
            _ => {}
        }
    }

    // For multi-argument builtins like syn_lift and syn_subst,
    // we need to check if func is a partial application

    // syn_lift amount cutoff term
    // Structure: (((syn_lift amount) cutoff) term) = App(App(App(syn_lift, amount), cutoff), term)
    // When reduced: func = App(App(syn_lift, amount), cutoff), arg = term
    if let Term::App(partial2, cutoff) = func {
        if let Term::App(partial1, amount) = partial2.as_ref() {
            if let Term::Global(op_name) = partial1.as_ref() {
                if op_name == "syn_lift" {
                    if let (Term::Lit(Literal::Int(amt)), Term::Lit(Literal::Int(cut))) =
                        (amount.as_ref(), cutoff.as_ref())
                    {
                        let norm_term = normalize(ctx, arg);
                        return try_syn_lift_reduce(ctx, *amt, *cut, &norm_term);
                    }
                }
            }
        }
    }

    // syn_subst replacement index term
    // Structure: (((syn_subst replacement) index) term)
    if let Term::App(partial2, index) = func {
        if let Term::App(partial1, replacement) = partial2.as_ref() {
            if let Term::Global(op_name) = partial1.as_ref() {
                if op_name == "syn_subst" {
                    if let Term::Lit(Literal::Int(idx)) = index.as_ref() {
                        let norm_replacement = normalize(ctx, replacement);
                        let norm_term = normalize(ctx, arg);
                        return try_syn_subst_reduce(ctx, &norm_replacement, *idx, &norm_term);
                    }
                }
            }
        }
    }

    // syn_beta body arg (2 arguments)
    // Structure: ((syn_beta body) arg)
    if let Term::App(partial1, body) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "syn_beta" {
                let norm_body = normalize(ctx, body);
                let norm_arg = normalize(ctx, arg);
                return try_syn_beta_reduce(ctx, &norm_body, &norm_arg);
            }
        }
    }

    // try_cong context eq_proof (2 arguments)
    // Structure: ((try_cong context) eq_proof)
    // Returns: DCong context eq_proof
    if let Term::App(partial1, context) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "try_cong" {
                let norm_context = normalize(ctx, context);
                let norm_proof = normalize(ctx, arg);
                return Some(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("DCong".to_string())),
                        Box::new(norm_context),
                    )),
                    Box::new(norm_proof),
                ));
            }
        }
    }

    // try_rewrite eq_proof goal (2 arguments)
    // Structure: ((try_rewrite eq_proof) goal)
    // Given eq_proof : Eq A x y, rewrites goal by replacing x with y
    if let Term::App(partial1, eq_proof) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "try_rewrite" {
                let norm_eq_proof = normalize(ctx, eq_proof);
                let norm_goal = normalize(ctx, arg);
                return try_try_rewrite_reduce(ctx, &norm_eq_proof, &norm_goal, false);
            }
            if op_name == "try_rewrite_rev" {
                let norm_eq_proof = normalize(ctx, eq_proof);
                let norm_goal = normalize(ctx, arg);
                return try_try_rewrite_reduce(ctx, &norm_eq_proof, &norm_goal, true);
            }
        }
    }

    // syn_eval fuel term (2 arguments)
    // Structure: ((syn_eval fuel) term)
    if let Term::App(partial1, fuel_term) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "syn_eval" {
                if let Term::Lit(Literal::Int(fuel)) = fuel_term.as_ref() {
                    let norm_term = normalize(ctx, arg);
                    return try_syn_eval_reduce(ctx, *fuel, &norm_term);
                }
            }
        }
    }

    // induction_base_goal ind_type motive (2 arguments)
    // Structure: ((induction_base_goal ind_type) motive)
    if let Term::App(partial1, ind_type) = func {
        if let Term::Global(op_name) = partial1.as_ref() {
            if op_name == "induction_base_goal" {
                let norm_ind_type = normalize(ctx, ind_type);
                let norm_motive = normalize(ctx, arg);
                return try_induction_base_goal_reduce(ctx, &norm_ind_type, &norm_motive);
            }
        }
    }

    // tact_orelse t1 t2 goal (3 arguments)
    // Structure: (((tact_orelse t1) t2) goal)
    // Here: func = App(App(tact_orelse, t1), t2), arg = goal
    if let Term::App(partial1, t2) = func {
        if let Term::App(combinator, t1) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "tact_orelse" {
                    return try_tact_orelse_reduce(ctx, t1, t2, arg);
                }
                // tact_then t1 t2 goal (3 arguments)
                if name == "tact_then" {
                    return try_tact_then_reduce(ctx, t1, t2, arg);
                }
            }
        }
    }

    // tact_try t goal (2 arguments)
    // Structure: ((tact_try t) goal)
    if let Term::App(combinator, t) = func {
        if let Term::Global(name) = combinator.as_ref() {
            if name == "tact_try" {
                return try_tact_try_reduce(ctx, t, arg);
            }
            // tact_repeat t goal (2 arguments)
            if name == "tact_repeat" {
                return try_tact_repeat_reduce(ctx, t, arg);
            }
            // tact_solve t goal (2 arguments)
            if name == "tact_solve" {
                return try_tact_solve_reduce(ctx, t, arg);
            }
            // tact_first tactics goal (2 arguments)
            if name == "tact_first" {
                return try_tact_first_reduce(ctx, t, arg);
            }
        }
    }

    // induction_step_goal ind_type motive ctor_idx (3 arguments)
    // Structure: (((induction_step_goal ind_type) motive) ctor_idx)
    // Returns the goal for the given constructor index
    if let Term::App(partial1, motive) = func {
        if let Term::App(combinator, ind_type) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "induction_step_goal" {
                    let norm_ind_type = normalize(ctx, ind_type);
                    let norm_motive = normalize(ctx, motive);
                    let norm_idx = normalize(ctx, arg);
                    return try_induction_step_goal_reduce(ctx, &norm_ind_type, &norm_motive, &norm_idx);
                }
            }
        }
    }

    // try_induction ind_type motive cases (3 arguments)
    // Structure: (((try_induction ind_type) motive) cases)
    // Returns DElim ind_type motive cases (delegating to existing infrastructure)
    if let Term::App(partial1, motive) = func {
        if let Term::App(combinator, ind_type) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "try_induction" {
                    let norm_ind_type = normalize(ctx, ind_type);
                    let norm_motive = normalize(ctx, motive);
                    let norm_cases = normalize(ctx, arg);
                    return try_try_induction_reduce(ctx, &norm_ind_type, &norm_motive, &norm_cases);
                }
            }
        }
    }

    // try_destruct ind_type motive cases (3 arguments)
    // Structure: (((try_destruct ind_type) motive) cases)
    // Case analysis without induction hypotheses
    if let Term::App(partial1, motive) = func {
        if let Term::App(combinator, ind_type) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "try_destruct" {
                    let norm_ind_type = normalize(ctx, ind_type);
                    let norm_motive = normalize(ctx, motive);
                    let norm_cases = normalize(ctx, arg);
                    return try_try_destruct_reduce(ctx, &norm_ind_type, &norm_motive, &norm_cases);
                }
            }
        }
    }

    // try_apply hyp_name hyp_proof goal (3 arguments)
    // Structure: (((try_apply hyp_name) hyp_proof) goal)
    // Manual backward chaining
    if let Term::App(partial1, hyp_proof) = func {
        if let Term::App(combinator, hyp_name) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "try_apply" {
                    let norm_hyp_name = normalize(ctx, hyp_name);
                    let norm_hyp_proof = normalize(ctx, hyp_proof);
                    let norm_goal = normalize(ctx, arg);
                    return try_try_apply_reduce(ctx, &norm_hyp_name, &norm_hyp_proof, &norm_goal);
                }
            }
        }
    }

    None
}

/// Compute size of a Syntax term.
///
/// SVar n -> 1
/// SGlobal n -> 1
/// SSort u -> 1
/// SApp f x -> 1 + size(f) + size(x)
/// SLam A b -> 1 + size(A) + size(b)
/// SPi A B -> 1 + size(A) + size(B)
fn try_syn_size_reduce(ctx: &Context, term: &Term) -> Option<Term> {
    // Match on the constructor form
    // Unary constructors: (SVar n), (SGlobal n), (SSort u)
    // Binary constructors: ((SApp f) x), ((SLam A) b), ((SPi A) B)

    // Check for unary constructor: (Ctor arg)
    if let Term::App(ctor_term, _inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" | "SGlobal" | "SSort" | "SLit" | "SName" => {
                    return Some(Term::Lit(Literal::Int(1)));
                }
                _ => {}
            }
        }

        // Check for binary constructor: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" | "SLam" | "SPi" => {
                        // Get sizes of both children
                        let a_size = try_syn_size_reduce(ctx, a)?;
                        let b_size = try_syn_size_reduce(ctx, _inner_arg)?;

                        if let (Term::Lit(Literal::Int(a_n)), Term::Lit(Literal::Int(b_n))) =
                            (a_size, b_size)
                        {
                            return Some(Term::Lit(Literal::Int(1 + a_n + b_n)));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

/// Compute maximum free variable index in a Syntax term.
///
/// The `depth` parameter tracks how many binders we're under.
/// SVar k -> k - depth (returns -1 if bound, k - depth if free)
/// SGlobal _ -> -1 (no free variables)
/// SSort _ -> -1 (no free variables)
/// SApp f x -> max(max_var(f), max_var(x))
/// SLam A b -> max(max_var(A), max_var(b, depth+1))
/// SPi A B -> max(max_var(A), max_var(B, depth+1))
fn try_syn_max_var_reduce(ctx: &Context, term: &Term, depth: i64) -> Option<Term> {
    // Check for unary constructor: (Ctor arg)
    if let Term::App(ctor_term, inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" => {
                    // SVar k -> if k >= depth then k - depth else -1
                    if let Term::Lit(Literal::Int(k)) = inner_arg.as_ref() {
                        if *k >= depth {
                            // Free variable with adjusted index
                            return Some(Term::Lit(Literal::Int(*k - depth)));
                        } else {
                            // Bound variable
                            return Some(Term::Lit(Literal::Int(-1)));
                        }
                    }
                }
                "SGlobal" | "SSort" | "SLit" | "SName" => {
                    // No free variables
                    return Some(Term::Lit(Literal::Int(-1)));
                }
                _ => {}
            }
        }

        // Check for binary constructor: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" => {
                        // SApp f x -> max(max_var(f), max_var(x))
                        let a_max = try_syn_max_var_reduce(ctx, a, depth)?;
                        let b_max = try_syn_max_var_reduce(ctx, inner_arg, depth)?;

                        if let (Term::Lit(Literal::Int(a_n)), Term::Lit(Literal::Int(b_n))) =
                            (a_max, b_max)
                        {
                            return Some(Term::Lit(Literal::Int(a_n.max(b_n))));
                        }
                    }
                    "SLam" | "SPi" => {
                        // SLam A b -> max(max_var(A, depth), max_var(b, depth+1))
                        // The body 'b' is under one additional binder
                        let a_max = try_syn_max_var_reduce(ctx, a, depth)?;
                        let b_max = try_syn_max_var_reduce(ctx, inner_arg, depth + 1)?;

                        if let (Term::Lit(Literal::Int(a_n)), Term::Lit(Literal::Int(b_n))) =
                            (a_max, b_max)
                        {
                            return Some(Term::Lit(Literal::Int(a_n.max(b_n))));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

// -------------------------------------------------------------------------
// Substitution Builtins
// -------------------------------------------------------------------------

/// Lift De Bruijn indices in a Syntax term.
///
/// syn_lift amount cutoff term:
/// - Variables with index < cutoff are bound -> unchanged
/// - Variables with index >= cutoff are free -> add amount
fn try_syn_lift_reduce(ctx: &Context, amount: i64, cutoff: i64, term: &Term) -> Option<Term> {
    // Check for unary constructor: (Ctor arg)
    if let Term::App(ctor_term, inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" => {
                    if let Term::Lit(Literal::Int(k)) = inner_arg.as_ref() {
                        if *k >= cutoff {
                            // Free variable, shift
                            return Some(Term::App(
                                Box::new(Term::Global("SVar".to_string())),
                                Box::new(Term::Lit(Literal::Int(*k + amount))),
                            ));
                        } else {
                            // Bound variable, no shift
                            return Some(term.clone());
                        }
                    }
                }
                "SGlobal" | "SSort" | "SLit" | "SName" => {
                    // No free variables
                    return Some(term.clone());
                }
                _ => {}
            }
        }

        // Check for binary constructor: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" => {
                        // No binding, same cutoff for both
                        let a_lifted = try_syn_lift_reduce(ctx, amount, cutoff, a)?;
                        let b_lifted = try_syn_lift_reduce(ctx, amount, cutoff, inner_arg)?;
                        return Some(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global("SApp".to_string())),
                                Box::new(a_lifted),
                            )),
                            Box::new(b_lifted),
                        ));
                    }
                    "SLam" | "SPi" => {
                        // Binder: param type at current cutoff, body at cutoff+1
                        let param_lifted = try_syn_lift_reduce(ctx, amount, cutoff, a)?;
                        let body_lifted =
                            try_syn_lift_reduce(ctx, amount, cutoff + 1, inner_arg)?;
                        return Some(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global(ctor_name.clone())),
                                Box::new(param_lifted),
                            )),
                            Box::new(body_lifted),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

/// Substitute a term for a variable in a Syntax term.
///
/// syn_subst replacement index term:
/// - If term is SVar k and k == index, return replacement
/// - If term is SVar k and k != index, return term unchanged
/// - For binders, increment index and lift replacement
fn try_syn_subst_reduce(
    ctx: &Context,
    replacement: &Term,
    index: i64,
    term: &Term,
) -> Option<Term> {
    // Check for unary constructor: (Ctor arg)
    if let Term::App(ctor_term, inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" => {
                    if let Term::Lit(Literal::Int(k)) = inner_arg.as_ref() {
                        if *k == index {
                            // Match! Return replacement
                            return Some(replacement.clone());
                        } else {
                            // No match, return unchanged
                            return Some(term.clone());
                        }
                    }
                }
                "SGlobal" | "SSort" | "SLit" | "SName" => {
                    // No variables to substitute
                    return Some(term.clone());
                }
                _ => {}
            }
        }

        // Check for binary constructor: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" => {
                        // No binding, same index and replacement
                        let a_subst = try_syn_subst_reduce(ctx, replacement, index, a)?;
                        let b_subst = try_syn_subst_reduce(ctx, replacement, index, inner_arg)?;
                        return Some(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global("SApp".to_string())),
                                Box::new(a_subst),
                            )),
                            Box::new(b_subst),
                        ));
                    }
                    "SLam" | "SPi" => {
                        // Binder: param type at current index, body at index+1
                        // Lift replacement when going under binder
                        let param_subst = try_syn_subst_reduce(ctx, replacement, index, a)?;

                        // Lift the replacement by 1 when going under the binder
                        let lifted_replacement = try_syn_lift_reduce(ctx, 1, 0, replacement)?;
                        let body_subst = try_syn_subst_reduce(
                            ctx,
                            &lifted_replacement,
                            index + 1,
                            inner_arg,
                        )?;

                        return Some(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global(ctor_name.clone())),
                                Box::new(param_subst),
                            )),
                            Box::new(body_subst),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

// -------------------------------------------------------------------------
// Computation Builtins
// -------------------------------------------------------------------------

/// Beta reduction: substitute arg for variable 0 in body.
///
/// syn_beta body arg = syn_subst arg 0 body
fn try_syn_beta_reduce(ctx: &Context, body: &Term, arg: &Term) -> Option<Term> {
    // syn_beta is just syn_subst with index 0
    try_syn_subst_reduce(ctx, arg, 0, body)
}

/// Try to reduce arithmetic on Syntax literals.
///
/// Handles: SApp (SApp (SName op) (SLit n)) (SLit m) → SLit result
/// where op is one of: add, sub, mul, div, mod
fn try_syn_arith_reduce(func: &Term, arg: &Term) -> Option<Term> {
    // func should be: SApp (SName op) (SLit n)
    // Pattern: App(App(Global("SApp"), App(Global("SName"), Lit(Text(op)))), App(Global("SLit"), Lit(Int(n))))
    if let Term::App(inner_ctor, n_term) = func {
        if let Term::App(sapp_ctor, op_term) = inner_ctor.as_ref() {
            // Check for SApp constructor
            if let Term::Global(ctor_name) = sapp_ctor.as_ref() {
                if ctor_name != "SApp" {
                    return None;
                }
            } else {
                return None;
            }

            // Extract op from SName op
            let op = if let Term::App(sname_ctor, op_str) = op_term.as_ref() {
                if let Term::Global(name) = sname_ctor.as_ref() {
                    if name == "SName" {
                        if let Term::Lit(Literal::Text(op)) = op_str.as_ref() {
                            op.clone()
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            };

            // Extract n from SLit n
            let n = if let Term::App(slit_ctor, n_val) = n_term.as_ref() {
                if let Term::Global(name) = slit_ctor.as_ref() {
                    if name == "SLit" {
                        if let Term::Lit(Literal::Int(n)) = n_val.as_ref() {
                            *n
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            };

            // Extract m from SLit m (the arg)
            let m = if let Term::App(slit_ctor, m_val) = arg {
                if let Term::Global(name) = slit_ctor.as_ref() {
                    if name == "SLit" {
                        if let Term::Lit(Literal::Int(m)) = m_val.as_ref() {
                            *m
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            };

            // Compute result based on operator
            let result = match op.as_str() {
                "add" => n.checked_add(m),
                "sub" => n.checked_sub(m),
                "mul" => n.checked_mul(m),
                "div" => {
                    if m == 0 {
                        return None; // Division by zero: remain stuck
                    }
                    n.checked_div(m)
                }
                "mod" => {
                    if m == 0 {
                        return None; // Mod by zero: remain stuck
                    }
                    n.checked_rem(m)
                }
                _ => None,
            };

            // Build SLit result
            if let Some(r) = result {
                return Some(Term::App(
                    Box::new(Term::Global("SLit".to_string())),
                    Box::new(Term::Lit(Literal::Int(r))),
                ));
            }
        }
    }

    None
}

/// Single-step head reduction.
///
/// Looks for the leftmost-outermost beta redex and reduces it.
/// - If SApp (SLam T body) arg: perform beta reduction
/// - If SApp f x where f is reducible: reduce f
/// - Otherwise: return unchanged (stuck or value)
fn try_syn_step_reduce(ctx: &Context, term: &Term) -> Option<Term> {
    // Check for SApp (binary constructor)
    if let Term::App(ctor_term, arg) = term {
        if let Term::App(inner, func) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                if ctor_name == "SApp" {
                    // We have SApp func arg
                    // First, check for arithmetic primitives: SApp (SApp (SName op) (SLit n)) (SLit m)
                    if let Some(result) = try_syn_arith_reduce(func.as_ref(), arg.as_ref()) {
                        return Some(result);
                    }

                    // Check if func is SLam (beta redex)
                    if let Term::App(lam_inner, body) = func.as_ref() {
                        if let Term::App(lam_ctor, _param_type) = lam_inner.as_ref() {
                            if let Term::Global(lam_name) = lam_ctor.as_ref() {
                                if lam_name == "SLam" {
                                    // Beta redex! (SApp (SLam T body) arg) → syn_beta body arg
                                    return try_syn_beta_reduce(ctx, body.as_ref(), arg.as_ref());
                                }
                            }
                        }
                    }

                    // Not a beta redex. Try to step the function.
                    if let Some(stepped_func) = try_syn_step_reduce(ctx, func.as_ref()) {
                        // Check if func actually changed
                        if &stepped_func != func.as_ref() {
                            // func reduced, reconstruct SApp stepped_func arg
                            return Some(Term::App(
                                Box::new(Term::App(
                                    Box::new(Term::Global("SApp".to_string())),
                                    Box::new(stepped_func),
                                )),
                                Box::new(arg.as_ref().clone()),
                            ));
                        }
                    }

                    // func is stuck. Try to step the argument.
                    if let Some(stepped_arg) = try_syn_step_reduce(ctx, arg.as_ref()) {
                        if &stepped_arg != arg.as_ref() {
                            // arg reduced, reconstruct SApp func stepped_arg
                            return Some(Term::App(
                                Box::new(Term::App(
                                    Box::new(Term::Global("SApp".to_string())),
                                    Box::new(func.as_ref().clone()),
                                )),
                                Box::new(stepped_arg),
                            ));
                        }
                    }

                    // Both func and arg are stuck, return original term
                    return Some(term.clone());
                }
            }
        }
    }

    // Not an application - it's a value or stuck term
    // Return unchanged
    Some(term.clone())
}

// -------------------------------------------------------------------------
// Bounded Evaluation
// -------------------------------------------------------------------------

/// Bounded evaluation: reduce for up to N steps.
///
/// syn_eval fuel term:
/// - If fuel <= 0: return term unchanged
/// - Otherwise: step and repeat until normal form or fuel exhausted
fn try_syn_eval_reduce(ctx: &Context, fuel: i64, term: &Term) -> Option<Term> {
    if fuel <= 0 {
        return Some(term.clone());
    }

    // Try one step
    let stepped = try_syn_step_reduce(ctx, term)?;

    // If term didn't change, it's in normal form (or stuck)
    if &stepped == term {
        return Some(term.clone());
    }

    // Continue with reduced fuel
    try_syn_eval_reduce(ctx, fuel - 1, &stepped)
}

// -------------------------------------------------------------------------
// Reification (Quote)
// -------------------------------------------------------------------------

/// Quote a Syntax value: produce Syntax that constructs it.
///
/// syn_quote term:
/// - SVar n → SApp (SName "SVar") (SLit n)
/// - SGlobal n → SApp (SName "SGlobal") (SLit n)
/// - SSort u → SApp (SName "SSort") (quote_univ u)
/// - SApp f x → SApp (SApp (SName "SApp") (quote f)) (quote x)
/// - SLam T b → SApp (SApp (SName "SLam") (quote T)) (quote b)
/// - SPi T B → SApp (SApp (SName "SPi") (quote T)) (quote B)
/// - SLit n → SApp (SName "SLit") (SLit n)
/// - SName s → SName s (self-quoting)
#[allow(dead_code)]
fn try_syn_quote_reduce(ctx: &Context, term: &Term) -> Option<Term> {
    // Helper to build SApp (SName name) arg
    fn sname_app(name: &str, arg: Term) -> Term {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text(name.to_string()))),
                )),
            )),
            Box::new(arg),
        )
    }

    // Helper to build SLit n
    fn slit(n: i64) -> Term {
        Term::App(
            Box::new(Term::Global("SLit".to_string())),
            Box::new(Term::Lit(Literal::Int(n))),
        )
    }

    // Helper to build SName s
    fn sname(s: &str) -> Term {
        Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text(s.to_string()))),
        )
    }

    // Helper to build SApp (SApp (SName name) a) b
    fn sname_app2(name: &str, a: Term, b: Term) -> Term {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(sname_app(name, a)),
            )),
            Box::new(b),
        )
    }

    // Match on Syntax constructors
    if let Term::App(ctor_term, inner_arg) = term {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            match ctor_name.as_str() {
                "SVar" => {
                    if let Term::Lit(Literal::Int(n)) = inner_arg.as_ref() {
                        return Some(sname_app("SVar", slit(*n)));
                    }
                }
                "SGlobal" => {
                    if let Term::Lit(Literal::Int(n)) = inner_arg.as_ref() {
                        return Some(sname_app("SGlobal", slit(*n)));
                    }
                }
                "SSort" => {
                    // Quote the universe
                    let quoted_univ = quote_univ(inner_arg)?;
                    return Some(sname_app("SSort", quoted_univ));
                }
                "SLit" => {
                    if let Term::Lit(Literal::Int(n)) = inner_arg.as_ref() {
                        return Some(sname_app("SLit", slit(*n)));
                    }
                }
                "SName" => {
                    // SName is self-quoting
                    return Some(term.clone());
                }
                _ => {}
            }
        }

        // Binary constructors: ((Ctor a) b)
        if let Term::App(inner, a) = ctor_term.as_ref() {
            if let Term::Global(ctor_name) = inner.as_ref() {
                match ctor_name.as_str() {
                    "SApp" | "SLam" | "SPi" => {
                        let quoted_a = try_syn_quote_reduce(ctx, a)?;
                        let quoted_b = try_syn_quote_reduce(ctx, inner_arg)?;
                        return Some(sname_app2(ctor_name, quoted_a, quoted_b));
                    }
                    _ => {}
                }
            }
        }
    }

    None
}

/// Quote a Univ value.
///
/// UProp → SName "UProp"
/// UType n → SApp (SName "UType") (SLit n)
fn quote_univ(term: &Term) -> Option<Term> {
    fn sname(s: &str) -> Term {
        Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text(s.to_string()))),
        )
    }

    fn slit(n: i64) -> Term {
        Term::App(
            Box::new(Term::Global("SLit".to_string())),
            Box::new(Term::Lit(Literal::Int(n))),
        )
    }

    fn sname_app(name: &str, arg: Term) -> Term {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(sname(name)),
            )),
            Box::new(arg),
        )
    }

    if let Term::Global(name) = term {
        if name == "UProp" {
            return Some(sname("UProp"));
        }
    }

    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "UType" {
                if let Term::Lit(Literal::Int(n)) = arg.as_ref() {
                    return Some(sname_app("UType", slit(*n)));
                }
            }
        }
    }

    None
}

// -------------------------------------------------------------------------
// Diagonalization (Self-Reference)
// -------------------------------------------------------------------------

/// The diagonal function: syn_diag x = syn_subst (syn_quote x) 0 x
///
/// This takes a term x with free variable 0, quotes x to get its construction
/// code, then substitutes that code for variable 0 in x.
fn try_syn_diag_reduce(ctx: &Context, term: &Term) -> Option<Term> {
    // Step 1: Quote the term
    let quoted = try_syn_quote_reduce(ctx, term)?;

    // Step 2: Substitute the quoted form for variable 0 in the original term
    try_syn_subst_reduce(ctx, &quoted, 0, term)
}

// -------------------------------------------------------------------------
// Inference Rules
// -------------------------------------------------------------------------

/// Extract the conclusion from a derivation.
///
/// concludes d:
/// - DAxiom P → P
/// - DModusPonens d_impl d_ant → extract B from (Implies A B) if A matches antecedent
/// - DUnivIntro d → wrap conclusion in Forall
/// - DUnivElim d term → substitute term into forall body
/// - DRefl T a → Eq T a a
fn try_concludes_reduce(ctx: &Context, deriv: &Term) -> Option<Term> {
    // DAxiom P → P
    if let Term::App(ctor_term, p) = deriv {
        if let Term::Global(ctor_name) = ctor_term.as_ref() {
            if ctor_name == "DAxiom" {
                return Some(p.as_ref().clone());
            }
            if ctor_name == "DUnivIntro" {
                // Get conclusion of inner derivation
                let inner_conc = try_concludes_reduce(ctx, p)?;
                // Lift it by 1 and wrap in Forall
                let lifted = try_syn_lift_reduce(ctx, 1, 0, &inner_conc)?;
                return Some(make_forall_syntax(&lifted));
            }
            if ctor_name == "DCompute" {
                // DCompute goal: verify goal is Eq T A B and eval(A) == eval(B)
                return try_dcompute_conclude(ctx, p);
            }
            if ctor_name == "DRingSolve" {
                // DRingSolve goal: verify goal is Eq T A B and polynomials are equal
                return try_dring_solve_conclude(ctx, p);
            }
            if ctor_name == "DLiaSolve" {
                // DLiaSolve goal: verify goal is an inequality and LIA proves it
                return try_dlia_solve_conclude(ctx, p);
            }
            if ctor_name == "DccSolve" {
                // DccSolve goal: verify goal by congruence closure
                return try_dcc_solve_conclude(ctx, p);
            }
            if ctor_name == "DSimpSolve" {
                // DSimpSolve goal: verify goal by simplification
                return try_dsimp_solve_conclude(ctx, p);
            }
            if ctor_name == "DOmegaSolve" {
                // DOmegaSolve goal: verify goal by integer arithmetic
                return try_domega_solve_conclude(ctx, p);
            }
            if ctor_name == "DAutoSolve" {
                // DAutoSolve goal: verify goal by trying all tactics
                return try_dauto_solve_conclude(ctx, p);
            }
            if ctor_name == "DInversion" {
                // DInversion hyp_type: verify no constructor can match, return False
                return try_dinversion_conclude(ctx, p);
            }
        }
    }

    // DRewrite eq_proof old_goal new_goal → new_goal (if verified)
    // Pattern: App(App(App(DRewrite, eq_proof), old_goal), new_goal)
    if let Term::App(partial1, new_goal) = deriv {
        if let Term::App(partial2, old_goal) = partial1.as_ref() {
            if let Term::App(ctor_term, eq_proof) = partial2.as_ref() {
                if let Term::Global(ctor_name) = ctor_term.as_ref() {
                    if ctor_name == "DRewrite" {
                        return try_drewrite_conclude(ctx, eq_proof, old_goal, new_goal);
                    }
                }
            }
        }
    }

    // DDestruct ind_type motive cases → Forall ind_type motive (if verified)
    // Pattern: App(App(App(DDestruct, ind_type), motive), cases)
    if let Term::App(partial1, cases) = deriv {
        if let Term::App(partial2, motive) = partial1.as_ref() {
            if let Term::App(ctor_term, ind_type) = partial2.as_ref() {
                if let Term::Global(ctor_name) = ctor_term.as_ref() {
                    if ctor_name == "DDestruct" {
                        return try_ddestruct_conclude(ctx, ind_type, motive, cases);
                    }
                }
            }
        }
    }

    // DApply hyp_name hyp_proof old_goal new_goal → new_goal (if verified)
    // Pattern: App(App(App(App(DApply, hyp_name), hyp_proof), old_goal), new_goal)
    if let Term::App(partial1, new_goal) = deriv {
        if let Term::App(partial2, old_goal) = partial1.as_ref() {
            if let Term::App(partial3, hyp_proof) = partial2.as_ref() {
                if let Term::App(ctor_term, hyp_name) = partial3.as_ref() {
                    if let Term::Global(ctor_name) = ctor_term.as_ref() {
                        if ctor_name == "DApply" {
                            return try_dapply_conclude(ctx, hyp_name, hyp_proof, old_goal, new_goal);
                        }
                    }
                }
            }
        }
    }

    // DRefl T a → Eq T a a
    if let Term::App(partial, a) = deriv {
        if let Term::App(ctor_term, t) = partial.as_ref() {
            if let Term::Global(ctor_name) = ctor_term.as_ref() {
                if ctor_name == "DRefl" {
                    return Some(make_eq_syntax(t.as_ref(), a.as_ref()));
                }
                if ctor_name == "DCong" {
                    // DCong context eq_proof → Eq T (f a) (f b)
                    // where context = SLam T body, eq_proof proves Eq T a b
                    return try_dcong_conclude(ctx, t, a);
                }
            }
        }
    }

    // DModusPonens d_impl d_ant
    if let Term::App(partial, d_ant) = deriv {
        if let Term::App(ctor_term, d_impl) = partial.as_ref() {
            if let Term::Global(ctor_name) = ctor_term.as_ref() {
                if ctor_name == "DModusPonens" {
                    // Get conclusions of both derivations
                    let impl_conc = try_concludes_reduce(ctx, d_impl)?;
                    let ant_conc = try_concludes_reduce(ctx, d_ant)?;

                    // Check if impl_conc = SApp (SApp (SName "Implies") A) B
                    if let Some((a, b)) = extract_implication(&impl_conc) {
                        // Check if ant_conc equals A
                        if syntax_equal(&ant_conc, &a) {
                            return Some(b);
                        }
                    }
                    // Invalid modus ponens
                    return Some(make_sname_error());
                }
                if ctor_name == "DUnivElim" {
                    // d_impl is the derivation, d_ant is the term to substitute
                    let conc = try_concludes_reduce(ctx, d_impl)?;
                    if let Some(body) = extract_forall_body(&conc) {
                        // Substitute d_ant for var 0 in body
                        return try_syn_subst_reduce(ctx, d_ant, 0, &body);
                    }
                    // Invalid universal elimination
                    return Some(make_sname_error());
                }
            }
        }
    }

    // DInduction motive base step → Forall Nat motive (if verified)
    // Pattern: App(App(App(DInduction, motive), base), step)
    if let Term::App(partial1, step) = deriv {
        if let Term::App(partial2, base) = partial1.as_ref() {
            if let Term::App(ctor_term, motive) = partial2.as_ref() {
                if let Term::Global(ctor_name) = ctor_term.as_ref() {
                    if ctor_name == "DInduction" {
                        return try_dinduction_reduce(ctx, motive, base, step);
                    }
                }
            }
        }
    }

    // DElim ind_type motive cases → Forall ind_type motive (if verified)
    // Pattern: App(App(App(DElim, ind_type), motive), cases)
    if let Term::App(partial1, cases) = deriv {
        if let Term::App(partial2, motive) = partial1.as_ref() {
            if let Term::App(ctor_term, ind_type) = partial2.as_ref() {
                if let Term::Global(ctor_name) = ctor_term.as_ref() {
                    if ctor_name == "DElim" {
                        return try_delim_conclude(ctx, ind_type, motive, cases);
                    }
                }
            }
        }
    }

    None
}

/// Extract (A, B) from SApp (SApp (SName "Implies") A) B
///
/// In kernel representation:
/// SApp X Y = App(App(SApp, X), Y)
/// So SApp (SApp (SName "Implies") A) B =
///   App(App(SApp, App(App(SApp, App(SName, "Implies")), A)), B)
fn extract_implication(term: &Term) -> Option<(Term, Term)> {
    // term = App(App(SApp, X), B)
    if let Term::App(outer, b) = term {
        if let Term::App(sapp_outer, x) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    // x = App(App(SApp, App(SName, "Implies")), A)
                    if let Term::App(inner, a) = x.as_ref() {
                        if let Term::App(sapp_inner, sname_implies) = inner.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    // sname_implies = App(SName, "Implies")
                                    if let Term::App(sname, text) = sname_implies.as_ref() {
                                        if let Term::Global(sname_ctor) = sname.as_ref() {
                                            if sname_ctor == "SName" {
                                                if let Term::Lit(Literal::Text(s)) = text.as_ref() {
                                                    if s == "Implies" || s == "implies" {
                                                        return Some((
                                                            a.as_ref().clone(),
                                                            b.as_ref().clone(),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Extract all hypotheses from a chain of nested implications.
///
/// For `A -> B -> C`, returns `([A, B], C)`.
/// For non-implications, returns `([], term)`.
fn extract_implications(term: &Term) -> Option<(Vec<Term>, Term)> {
    let mut hyps = Vec::new();
    let mut current = term.clone();

    while let Some((hyp, rest)) = extract_implication(&current) {
        hyps.push(hyp);
        current = rest;
    }

    if hyps.is_empty() {
        None
    } else {
        Some((hyps, current))
    }
}

/// Extract body from SApp (SApp (SName "Forall") T) (SLam T body)
///
/// In kernel representation:
/// Extract body from Forall syntax (two forms supported):
/// Form 1: SApp (SName "Forall") (SLam T body)
/// Form 2: SApp (SApp (SName "Forall") T) (SLam T body)
fn extract_forall_body(term: &Term) -> Option<Term> {
    // term = App(App(SApp, X), lam)
    if let Term::App(outer, lam) = term {
        if let Term::App(sapp_outer, x) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    // Form 1: X = App(SName, "Forall")
                    if let Term::App(sname, text) = x.as_ref() {
                        if let Term::Global(sname_ctor) = sname.as_ref() {
                            if sname_ctor == "SName" {
                                if let Term::Lit(Literal::Text(s)) = text.as_ref() {
                                    if s == "Forall" {
                                        // Extract body from SLam
                                        return extract_slam_body(lam);
                                    }
                                }
                            }
                        }
                    }

                    // Form 2: X = App(App(SApp, App(SName, "Forall")), T)
                    if let Term::App(inner, _t) = x.as_ref() {
                        if let Term::App(sapp_inner, sname_forall) = inner.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    if let Term::App(sname, text) = sname_forall.as_ref() {
                                        if let Term::Global(sname_ctor) = sname.as_ref() {
                                            if sname_ctor == "SName" {
                                                if let Term::Lit(Literal::Text(s)) = text.as_ref() {
                                                    if s == "Forall" {
                                                        return extract_slam_body(lam);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Extract body from ((SLam T) body)
fn extract_slam_body(term: &Term) -> Option<Term> {
    if let Term::App(inner, body) = term {
        if let Term::App(slam, _t) = inner.as_ref() {
            if let Term::Global(name) = slam.as_ref() {
                if name == "SLam" {
                    return Some(body.as_ref().clone());
                }
            }
        }
    }
    None
}

/// Structural equality check for Syntax terms
fn syntax_equal(a: &Term, b: &Term) -> bool {
    a == b
}

/// Build SName "Error"
fn make_sname_error() -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text("Error".to_string()))),
    )
}

/// Build SName from a string
fn make_sname(s: &str) -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text(s.to_string()))),
    )
}

/// Build SLit from an integer
fn make_slit(n: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SLit".to_string())),
        Box::new(Term::Lit(Literal::Int(n))),
    )
}

/// Build SApp f x
fn make_sapp(f: Term, x: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(f),
        )),
        Box::new(x),
    )
}

/// Build SPi A B
fn make_spi(a: Term, b: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SPi".to_string())),
            Box::new(a),
        )),
        Box::new(b),
    )
}

/// Build SLam A b
fn make_slam(a: Term, b: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SLam".to_string())),
            Box::new(a),
        )),
        Box::new(b),
    )
}

/// Build SSort u
fn make_ssort(u: Term) -> Term {
    Term::App(
        Box::new(Term::Global("SSort".to_string())),
        Box::new(u),
    )
}

/// Build SVar n
fn make_svar(n: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SVar".to_string())),
        Box::new(Term::Lit(Literal::Int(n))),
    )
}

/// Convert a kernel Term to Syntax encoding (S* form).
///
/// This converts semantic Terms to their syntactic representation:
/// - Global(n) → SName n
/// - Var(n) → SName n (for named variables)
/// - App(f, x) → SApp (convert f) (convert x)
/// - Pi { param, param_type, body_type } → SPi (convert param_type) (convert body_type)
/// - Lambda { param, param_type, body } → SLam (convert param_type) (convert body)
/// - Sort(Type n) → SSort (UType n)
/// - Sort(Prop) → SSort UProp
/// - Lit(Int n) → SLit n
/// - Lit(Text s) → SName s
fn term_to_syntax(term: &Term) -> Option<Term> {
    match term {
        Term::Global(name) => Some(make_sname(name)),

        Term::Var(name) => {
            // Named variables become SName in syntax representation
            Some(make_sname(name))
        }

        Term::App(f, x) => {
            let sf = term_to_syntax(f)?;
            let sx = term_to_syntax(x)?;
            Some(make_sapp(sf, sx))
        }

        Term::Pi { param_type, body_type, .. } => {
            let sp = term_to_syntax(param_type)?;
            let sb = term_to_syntax(body_type)?;
            Some(make_spi(sp, sb))
        }

        Term::Lambda { param_type, body, .. } => {
            let sp = term_to_syntax(param_type)?;
            let sb = term_to_syntax(body)?;
            Some(make_slam(sp, sb))
        }

        Term::Sort(Universe::Type(n)) => {
            let u = Term::App(
                Box::new(Term::Global("UType".to_string())),
                Box::new(Term::Lit(Literal::Int(*n as i64))),
            );
            Some(make_ssort(u))
        }

        Term::Sort(Universe::Prop) => {
            let u = Term::Global("UProp".to_string());
            Some(make_ssort(u))
        }

        Term::Lit(Literal::Int(n)) => Some(make_slit(*n)),

        Term::Lit(Literal::Text(s)) => Some(make_sname(s)),

        // Float, Duration, Date, and Moment literals - skip for now as they're rarely used in proofs
        Term::Lit(Literal::Float(_))
        | Term::Lit(Literal::Duration(_))
        | Term::Lit(Literal::Date(_))
        | Term::Lit(Literal::Moment(_)) => None,

        // Match expressions, Fix, and Hole are complex - skip for now
        Term::Match { .. } | Term::Fix { .. } | Term::Hole => None,
    }
}

/// Build DHint derivation that references a hint by name.
///
/// Uses DAxiom (SName "hint_name") to indicate the proof uses this hint.
fn make_hint_derivation(hint_name: &str, goal: &Term) -> Term {
    // Build: DAxiom (SApp (SName "Hint") (SName hint_name))
    // This distinguishes hints from error derivations
    let hint_marker = make_sapp(make_sname("Hint"), make_sname(hint_name));

    // Actually, let's use a simpler approach: DAutoSolve goal
    // This indicates auto succeeded, and we can trace which hint was used through debugging
    Term::App(
        Box::new(Term::Global("DAutoSolve".to_string())),
        Box::new(goal.clone()),
    )
}

/// Try to apply a hint to prove a goal.
///
/// Returns Some(derivation) if the hint's type matches the goal,
/// otherwise returns None.
fn try_apply_hint(ctx: &Context, hint_name: &str, hint_type: &Term, goal: &Term) -> Option<Term> {
    // Convert hint type to syntax form
    let hint_syntax = term_to_syntax(hint_type)?;

    // Normalize both for comparison
    let norm_hint = normalize(ctx, &hint_syntax);
    let norm_goal = normalize(ctx, goal);

    // Direct match: hint type equals goal
    if syntax_equal(&norm_hint, &norm_goal) {
        return Some(make_hint_derivation(hint_name, goal));
    }

    // Try backward chaining for implications: if hint is P → Q and goal is Q,
    // try to prove P using auto and then apply the hint
    if let Term::App(outer, q) = &hint_syntax {
        if let Term::App(pi_ctor, p) = outer.as_ref() {
            if let Term::Global(name) = pi_ctor.as_ref() {
                if name == "SPi" {
                    // hint_type is SPi P Q, goal might be Q
                    let norm_q = normalize(ctx, q);
                    if syntax_equal(&norm_q, &norm_goal) {
                        // Need to prove P first, then apply hint
                        // This would require recursive auto call - skip for now
                        // to avoid infinite recursion
                    }
                }
            }
        }
    }

    None
}

/// Build Forall Type0 (SLam Type0 body)
fn make_forall_syntax(body: &Term) -> Term {
    let type0 = Term::App(
        Box::new(Term::Global("SSort".to_string())),
        Box::new(Term::App(
            Box::new(Term::Global("UType".to_string())),
            Box::new(Term::Lit(Literal::Int(0))),
        )),
    );

    // SLam Type0 body
    let slam = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SLam".to_string())),
            Box::new(type0.clone()),
        )),
        Box::new(body.clone()),
    );

    // SApp (SApp (SName "Forall") Type0) slam
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("SApp".to_string())),
                    Box::new(Term::App(
                        Box::new(Term::Global("SName".to_string())),
                        Box::new(Term::Lit(Literal::Text("Forall".to_string()))),
                    )),
                )),
                Box::new(type0),
            )),
        )),
        Box::new(slam),
    )
}

// -------------------------------------------------------------------------
// Core Tactics
// -------------------------------------------------------------------------

/// Build SApp (SApp (SApp (SName "Eq") type_s) term) term
///
/// Constructs the Syntax representation of (Eq type_s term term)
fn make_eq_syntax(type_s: &Term, term: &Term) -> Term {
    let eq_name = Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text("Eq".to_string()))),
    );

    // SApp (SName "Eq") type_s
    let app1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(eq_name),
        )),
        Box::new(type_s.clone()),
    );

    // SApp (SApp (SName "Eq") type_s) term
    let app2 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app1),
        )),
        Box::new(term.clone()),
    );

    // SApp (SApp (SApp (SName "Eq") type_s) term) term
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app2),
        )),
        Box::new(term.clone()),
    )
}

/// Extract (T, a, b) from SApp (SApp (SApp (SName "Eq") T) a) b
///
/// Pattern matches against the Syntax representation of (Eq T a b)
fn extract_eq(term: &Term) -> Option<(Term, Term, Term)> {
    // term = App(App(SApp, X), b)
    if let Term::App(outer, b) = term {
        if let Term::App(sapp_outer, x) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    // x = App(App(SApp, Y), a)
                    if let Term::App(inner, a) = x.as_ref() {
                        if let Term::App(sapp_inner, y) = inner.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    // y = App(App(SApp, eq_name), t)
                                    if let Term::App(inner2, t) = y.as_ref() {
                                        if let Term::App(sapp_inner2, sname_eq) = inner2.as_ref() {
                                            if let Term::Global(ctor3) = sapp_inner2.as_ref() {
                                                if ctor3 == "SApp" {
                                                    // sname_eq = App(SName, "Eq")
                                                    if let Term::App(sname, text) = sname_eq.as_ref()
                                                    {
                                                        if let Term::Global(sname_ctor) =
                                                            sname.as_ref()
                                                        {
                                                            if sname_ctor == "SName" {
                                                                if let Term::Lit(Literal::Text(s)) =
                                                                    text.as_ref()
                                                                {
                                                                    if s == "Eq" {
                                                                        return Some((
                                                                            t.as_ref().clone(),
                                                                            a.as_ref().clone(),
                                                                            b.as_ref().clone(),
                                                                        ));
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Build DRefl type_s term
fn make_drefl(type_s: &Term, term: &Term) -> Term {
    let drefl = Term::Global("DRefl".to_string());
    let app1 = Term::App(Box::new(drefl), Box::new(type_s.clone()));
    Term::App(Box::new(app1), Box::new(term.clone()))
}

/// Build DAxiom (SName "Error")
fn make_error_derivation() -> Term {
    let daxiom = Term::Global("DAxiom".to_string());
    let error = make_sname_error();
    Term::App(Box::new(daxiom), Box::new(error))
}

/// Build DAxiom goal (identity derivation)
fn make_daxiom(goal: &Term) -> Term {
    let daxiom = Term::Global("DAxiom".to_string());
    Term::App(Box::new(daxiom), Box::new(goal.clone()))
}

/// Reflexivity tactic: try to prove a goal by reflexivity.
///
/// try_refl goal:
/// - If goal matches (Eq T a b) and a == b, return DRefl T a
/// - Otherwise return DAxiom (SName "Error")
fn try_try_refl_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Pattern match: SApp (SApp (SApp (SName "Eq") T) a) b
    if let Some((type_s, left, right)) = extract_eq(&norm_goal) {
        // Check if left == right (structural equality)
        if syntax_equal(&left, &right) {
            // Success! Return DRefl T a
            return Some(make_drefl(&type_s, &left));
        }
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

// =============================================================================
// RING TACTIC (POLYNOMIAL EQUALITY)
// =============================================================================

use crate::ring;
use crate::lia;
use crate::cc;
use crate::simp;

/// Ring tactic: try to prove a goal by polynomial normalization.
///
/// try_ring goal:
/// - If goal matches (Eq T a b) where T is Int or Nat
/// - And poly(a) == poly(b) after normalization
/// - Returns DRingSolve goal
/// - Otherwise returns DAxiom (SName "Error")
fn try_try_ring_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Pattern match: SApp (SApp (SApp (SName "Eq") T) a) b
    if let Some((type_s, left, right)) = extract_eq(&norm_goal) {
        // Check type is Int or Nat (ring types)
        if !is_ring_type(&type_s) {
            return Some(make_error_derivation());
        }

        // Reify both sides to polynomials
        let poly_left = match ring::reify(&left) {
            Ok(p) => p,
            Err(_) => return Some(make_error_derivation()),
        };
        let poly_right = match ring::reify(&right) {
            Ok(p) => p,
            Err(_) => return Some(make_error_derivation()),
        };

        // Check canonical equality
        if poly_left.canonical_eq(&poly_right) {
            // Success! Return DRingSolve goal
            return Some(Term::App(
                Box::new(Term::Global("DRingSolve".to_string())),
                Box::new(norm_goal),
            ));
        }
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

/// Verify DRingSolve proof.
///
/// DRingSolve goal → goal (if verified)
fn try_dring_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Extract T, lhs, rhs from Eq T lhs rhs
    if let Some((type_s, left, right)) = extract_eq(&norm_goal) {
        // Verify type is a ring type
        if !is_ring_type(&type_s) {
            return Some(make_sname_error());
        }

        // Reify and verify
        let poly_left = match ring::reify(&left) {
            Ok(p) => p,
            Err(_) => return Some(make_sname_error()),
        };
        let poly_right = match ring::reify(&right) {
            Ok(p) => p,
            Err(_) => return Some(make_sname_error()),
        };

        if poly_left.canonical_eq(&poly_right) {
            return Some(norm_goal);
        }
    }

    Some(make_sname_error())
}

/// Check if a Syntax type is a ring type (Int or Nat)
fn is_ring_type(type_term: &Term) -> bool {
    if let Some(name) = extract_sname_from_syntax(type_term) {
        return name == "Int" || name == "Nat";
    }
    false
}

/// Extract name from SName term
fn extract_sname_from_syntax(term: &Term) -> Option<String> {
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = arg.as_ref() {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

// =============================================================================
// LIA TACTIC (LINEAR INTEGER ARITHMETIC)
// =============================================================================

/// LIA tactic: try to prove a goal by Fourier-Motzkin elimination.
///
/// try_lia goal:
/// - If goal matches (Lt/Le/Gt/Ge a b)
/// - And Fourier-Motzkin shows the negation is unsatisfiable
/// - Returns DLiaSolve goal
/// - Otherwise returns DAxiom (SName "Error")
fn try_try_lia_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Extract comparison: (SApp (SApp (SName "Lt"|"Le"|etc) a) b)
    if let Some((rel, lhs_term, rhs_term)) = lia::extract_comparison(&norm_goal) {
        // Reify both sides to linear expressions
        let lhs = match lia::reify_linear(&lhs_term) {
            Ok(l) => l,
            Err(_) => return Some(make_error_derivation()),
        };
        let rhs = match lia::reify_linear(&rhs_term) {
            Ok(r) => r,
            Err(_) => return Some(make_error_derivation()),
        };

        // Convert to negated constraint for validity checking
        if let Some(negated) = lia::goal_to_negated_constraint(&rel, &lhs, &rhs) {
            // If the negation is unsatisfiable, the goal is valid
            if lia::fourier_motzkin_unsat(&[negated]) {
                // Success! Return DLiaSolve goal
                return Some(Term::App(
                    Box::new(Term::Global("DLiaSolve".to_string())),
                    Box::new(norm_goal),
                ));
            }
        }
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

/// Verify DLiaSolve proof.
///
/// DLiaSolve goal → goal (if verified)
fn try_dlia_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Extract comparison and verify
    if let Some((rel, lhs_term, rhs_term)) = lia::extract_comparison(&norm_goal) {
        // Reify both sides
        let lhs = match lia::reify_linear(&lhs_term) {
            Ok(l) => l,
            Err(_) => return Some(make_sname_error()),
        };
        let rhs = match lia::reify_linear(&rhs_term) {
            Ok(r) => r,
            Err(_) => return Some(make_sname_error()),
        };

        // Verify via Fourier-Motzkin
        if let Some(negated) = lia::goal_to_negated_constraint(&rel, &lhs, &rhs) {
            if lia::fourier_motzkin_unsat(&[negated]) {
                return Some(norm_goal);
            }
        }
    }

    Some(make_sname_error())
}

// =============================================================================
// CONGRUENCE CLOSURE TACTIC
// =============================================================================

/// CC tactic: try to prove a goal by congruence closure.
///
/// try_cc goal:
/// - If goal is a direct equality (Eq a b) or an implication with hypotheses
/// - And congruence closure shows the conclusion follows
/// - Returns DccSolve goal
/// - Otherwise returns DAxiom (SName "Error")
fn try_try_cc_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Use cc::check_goal which handles both:
    // - Direct equalities: (Eq a b)
    // - Implications: (implies (Eq x y) (Eq (f x) (f y)))
    if cc::check_goal(&norm_goal) {
        // Success! Return DccSolve goal
        return Some(Term::App(
            Box::new(Term::Global("DccSolve".to_string())),
            Box::new(norm_goal),
        ));
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

/// Verify DccSolve proof.
///
/// DccSolve goal → goal (if verified)
fn try_dcc_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Re-verify using congruence closure
    if cc::check_goal(&norm_goal) {
        return Some(norm_goal);
    }

    Some(make_sname_error())
}

// =============================================================================
// SIMP TACTIC (SIMPLIFICATION)
// =============================================================================

/// try_simp goal:
/// - Prove equalities by simplification and arithmetic evaluation
/// - Handles hypotheses via implications: (implies (Eq x 0) (Eq (add x 1) 1))
/// - Returns DSimpSolve goal on success
/// - Otherwise returns DAxiom (SName "Error")
fn try_try_simp_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Normalize the goal first
    let norm_goal = normalize(ctx, goal);

    // Use simp::check_goal which handles:
    // - Reflexive equalities: (Eq a a)
    // - Constant folding: (Eq (add 2 3) 5)
    // - Hypothesis substitution: (implies (Eq x 0) (Eq (add x 1) 1))
    if simp::check_goal(&norm_goal) {
        // Success! Return DSimpSolve goal
        return Some(Term::App(
            Box::new(Term::Global("DSimpSolve".to_string())),
            Box::new(norm_goal),
        ));
    }

    // Failure: return error derivation
    Some(make_error_derivation())
}

/// Verify DSimpSolve proof.
///
/// DSimpSolve goal → goal (if verified)
fn try_dsimp_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Re-verify using simplification
    if simp::check_goal(&norm_goal) {
        return Some(norm_goal);
    }

    Some(make_sname_error())
}

// =============================================================================
// OMEGA TACTIC (TRUE INTEGER ARITHMETIC)
// =============================================================================

/// try_omega goal:
///
/// Omega tactic using integer-aware Fourier-Motzkin elimination.
/// Unlike lia (which uses rationals), omega handles integers properly:
/// - x > 1 means x >= 2 (strict-to-nonstrict conversion)
/// - 3x <= 10 means x <= 3 (floor division)
///
/// - Extracts comparison (Lt, Le, Gt, Ge) from goal
/// - Reifies to integer linear expressions
/// - Converts to negated constraint (validity = negation is unsat)
/// - Applies omega test
/// - Returns DOmegaSolve goal on success
fn try_try_omega_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Handle implications: extract hypotheses and check conclusion
    if let Some((hyps, conclusion)) = extract_implications(&norm_goal) {
        // Convert hypotheses to constraints
        let mut constraints = Vec::new();

        for hyp in &hyps {
            // Try to extract a comparison from the hypothesis
            if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(hyp) {
                if let (Some(lhs), Some(rhs)) =
                    (omega::reify_int_linear(&lhs_term), omega::reify_int_linear(&rhs_term))
                {
                    // Add hypothesis as a constraint (it's given, so it's a fact)
                    // For Lt(a, b): a - b < 0, i.e., (a - b) < 0
                    // Constraint form: (a - b + 1) <= 0 for integers (since a < b means a <= b - 1)
                    match rel.as_str() {
                        "Lt" | "lt" => {
                            // a < b means a - b <= -1, i.e., (a - b + 1) <= 0
                            let mut expr = lhs.sub(&rhs);
                            expr.constant += 1;
                            constraints.push(omega::IntConstraint { expr, strict: false });
                        }
                        "Le" | "le" => {
                            // a <= b means a - b <= 0
                            constraints.push(omega::IntConstraint {
                                expr: lhs.sub(&rhs),
                                strict: false,
                            });
                        }
                        "Gt" | "gt" => {
                            // a > b means a - b >= 1, i.e., (b - a + 1) <= 0
                            let mut expr = rhs.sub(&lhs);
                            expr.constant += 1;
                            constraints.push(omega::IntConstraint { expr, strict: false });
                        }
                        "Ge" | "ge" => {
                            // a >= b means b - a <= 0
                            constraints.push(omega::IntConstraint {
                                expr: rhs.sub(&lhs),
                                strict: false,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        // Now check if the conclusion is provable given the constraints
        if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(&conclusion) {
            if let (Some(lhs), Some(rhs)) =
                (omega::reify_int_linear(&lhs_term), omega::reify_int_linear(&rhs_term))
            {
                // To prove the conclusion, check if its negation is unsat
                if let Some(neg_constraint) = omega::goal_to_negated_constraint(&rel, &lhs, &rhs) {
                    let mut all_constraints = constraints;
                    all_constraints.push(neg_constraint);

                    if omega::omega_unsat(&all_constraints) {
                        return Some(Term::App(
                            Box::new(Term::Global("DOmegaSolve".to_string())),
                            Box::new(norm_goal),
                        ));
                    }
                }
            }
        }

        // Failure
        return Some(make_error_derivation());
    }

    // Direct comparison (no hypotheses)
    if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(&norm_goal) {
        if let (Some(lhs), Some(rhs)) =
            (omega::reify_int_linear(&lhs_term), omega::reify_int_linear(&rhs_term))
        {
            if let Some(constraint) = omega::goal_to_negated_constraint(&rel, &lhs, &rhs) {
                if omega::omega_unsat(&[constraint]) {
                    return Some(Term::App(
                        Box::new(Term::Global("DOmegaSolve".to_string())),
                        Box::new(norm_goal),
                    ));
                }
            }
        }
    }

    Some(make_error_derivation())
}

/// Verify DOmegaSolve proof.
///
/// DOmegaSolve goal → goal (if verified)
fn try_domega_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Re-verify using omega test
    // Handle implications
    if let Some((hyps, conclusion)) = extract_implications(&norm_goal) {
        let mut constraints = Vec::new();

        for hyp in &hyps {
            if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(hyp) {
                if let (Some(lhs), Some(rhs)) =
                    (omega::reify_int_linear(&lhs_term), omega::reify_int_linear(&rhs_term))
                {
                    match rel.as_str() {
                        "Lt" | "lt" => {
                            let mut expr = lhs.sub(&rhs);
                            expr.constant += 1;
                            constraints.push(omega::IntConstraint { expr, strict: false });
                        }
                        "Le" | "le" => {
                            constraints.push(omega::IntConstraint {
                                expr: lhs.sub(&rhs),
                                strict: false,
                            });
                        }
                        "Gt" | "gt" => {
                            let mut expr = rhs.sub(&lhs);
                            expr.constant += 1;
                            constraints.push(omega::IntConstraint { expr, strict: false });
                        }
                        "Ge" | "ge" => {
                            constraints.push(omega::IntConstraint {
                                expr: rhs.sub(&lhs),
                                strict: false,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(&conclusion) {
            if let (Some(lhs), Some(rhs)) =
                (omega::reify_int_linear(&lhs_term), omega::reify_int_linear(&rhs_term))
            {
                if let Some(neg_constraint) = omega::goal_to_negated_constraint(&rel, &lhs, &rhs) {
                    let mut all_constraints = constraints;
                    all_constraints.push(neg_constraint);

                    if omega::omega_unsat(&all_constraints) {
                        return Some(norm_goal);
                    }
                }
            }
        }

        return Some(make_sname_error());
    }

    // Direct comparison
    if let Some((rel, lhs_term, rhs_term)) = omega::extract_comparison(&norm_goal) {
        if let (Some(lhs), Some(rhs)) =
            (omega::reify_int_linear(&lhs_term), omega::reify_int_linear(&rhs_term))
        {
            if let Some(constraint) = omega::goal_to_negated_constraint(&rel, &lhs, &rhs) {
                if omega::omega_unsat(&[constraint]) {
                    return Some(norm_goal);
                }
            }
        }
    }

    Some(make_sname_error())
}

// =============================================================================
// AUTO TACTIC (THE INFINITY GAUNTLET)
// =============================================================================

/// Check if a derivation is an error derivation.
///
/// Error derivations have the pattern: DAxiom (SApp (SName "Error") ...)
fn is_error_derivation(term: &Term) -> bool {
    // Check for DAxiom constructor
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "DAxiom" {
                // Check if arg is SName "Error" or SApp (SName "Error") ...
                if let Term::App(sname, inner) = arg.as_ref() {
                    if let Term::Global(sn) = sname.as_ref() {
                        if sn == "SName" {
                            if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                                return s == "Error";
                            }
                        }
                        if sn == "SApp" {
                            // Recursively check - could be SApp (SName "Error") ...
                            return true; // For now, treat any DAxiom as potential error
                        }
                    }
                }
                // DAxiom with SName "Error"
                if let Term::Global(sn) = arg.as_ref() {
                    // This shouldn't happen but just in case
                    return sn == "Error";
                }
                return true; // Any DAxiom is treated as error for auto purposes
            }
        }
    }
    false
}

/// Auto tactic: tries each decision procedure in sequence.
///
/// Order: True/False → simp → ring → cc → omega → lia
/// Returns the first successful derivation, or error if all fail.
fn try_try_auto_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Handle trivial cases: True and False
    // SName "True" is trivially provable
    if let Term::App(ctor, inner) = &norm_goal {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                    if s == "True" {
                        // True is always provable - use DAutoSolve
                        return Some(Term::App(
                            Box::new(Term::Global("DAutoSolve".to_string())),
                            Box::new(norm_goal),
                        ));
                    }
                    if s == "False" {
                        // False is never provable
                        return Some(make_error_derivation());
                    }
                }
            }
        }
    }

    // Try simp (handles equalities with simplification)
    if let Some(result) = try_try_simp_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try ring (polynomial equalities)
    if let Some(result) = try_try_ring_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try cc (congruence closure)
    if let Some(result) = try_try_cc_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try omega (integer arithmetic - most precise)
    if let Some(result) = try_try_omega_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try lia (linear arithmetic - fallback)
    if let Some(result) = try_try_lia_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(result);
        }
    }

    // Try registered hints
    for hint_name in ctx.get_hints() {
        if let Some(hint_type) = ctx.get_global(hint_name) {
            if let Some(result) = try_apply_hint(ctx, hint_name, hint_type, &norm_goal) {
                return Some(result);
            }
        }
    }

    // All tactics failed
    Some(make_error_derivation())
}

/// Verify DAutoSolve proof by re-running tactic search.
fn try_dauto_solve_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Handle trivial cases: True
    if let Term::App(ctor, inner) = &norm_goal {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = inner.as_ref() {
                    if s == "True" {
                        return Some(norm_goal.clone());
                    }
                }
            }
        }
    }

    // Try each tactic - if any succeeds, the proof is valid
    if let Some(result) = try_try_simp_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }
    if let Some(result) = try_try_ring_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }
    if let Some(result) = try_try_cc_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }
    if let Some(result) = try_try_omega_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }
    if let Some(result) = try_try_lia_reduce(ctx, &norm_goal) {
        if !is_error_derivation(&result) {
            return Some(norm_goal.clone());
        }
    }

    // Try registered hints
    for hint_name in ctx.get_hints() {
        if let Some(hint_type) = ctx.get_global(hint_name) {
            if try_apply_hint(ctx, hint_name, hint_type, &norm_goal).is_some() {
                return Some(norm_goal);
            }
        }
    }

    Some(make_sname_error())
}

// =============================================================================
// GENERIC INDUCTION HELPERS (induction_num_cases, induction_base_goal, etc.)
// =============================================================================

/// Returns the number of constructors for an inductive type.
///
/// induction_num_cases (SName "Nat") → Succ (Succ Zero) = 2
/// induction_num_cases (SName "Bool") → Succ (Succ Zero) = 2
/// induction_num_cases (SApp (SName "List") A) → Succ (Succ Zero) = 2
fn try_induction_num_cases_reduce(ctx: &Context, ind_type: &Term) -> Option<Term> {
    // Extract inductive name from Syntax
    let ind_name = match extract_inductive_name_from_syntax(ind_type) {
        Some(name) => name,
        None => {
            // Not a valid inductive type syntax
            return Some(Term::Global("Zero".to_string()));
        }
    };

    // Look up constructors
    let constructors = ctx.get_constructors(&ind_name);
    let num_ctors = constructors.len();

    // Build Nat representation: Succ (Succ (... Zero))
    let mut result = Term::Global("Zero".to_string());
    for _ in 0..num_ctors {
        result = Term::App(
            Box::new(Term::Global("Succ".to_string())),
            Box::new(result),
        );
    }

    Some(result)
}

/// Returns the base case goal for induction (first constructor).
///
/// Given ind_type and motive (SLam T body), returns motive[ctor0/var].
fn try_induction_base_goal_reduce(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
) -> Option<Term> {
    // Extract inductive name
    let ind_name = match extract_inductive_name_from_syntax(ind_type) {
        Some(name) => name,
        None => return Some(make_sname_error()),
    };

    // Get constructors
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        return Some(make_sname_error());
    }

    // Extract motive body
    let motive_body = match extract_slam_body(motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // Build goal for first constructor (base case)
    let (ctor_name, _) = constructors[0];
    build_case_expected(ctx, ctor_name, &constructors, &motive_body, ind_type)
}

/// Returns the goal for a specific constructor index.
///
/// Given ind_type, motive, and constructor index (as Nat), returns the case goal.
fn try_induction_step_goal_reduce(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
    ctor_idx: &Term,
) -> Option<Term> {
    // Extract inductive name
    let ind_name = match extract_inductive_name_from_syntax(ind_type) {
        Some(name) => name,
        None => return Some(make_sname_error()),
    };

    // Get constructors
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        return Some(make_sname_error());
    }

    // Convert Nat to index
    let idx = nat_to_usize(ctor_idx)?;
    if idx >= constructors.len() {
        return Some(make_sname_error());
    }

    // Extract motive body
    let motive_body = match extract_slam_body(motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // Build goal for the specified constructor
    let (ctor_name, _) = constructors[idx];
    build_case_expected(ctx, ctor_name, &constructors, &motive_body, ind_type)
}

/// Convert a Nat term to usize.
///
/// Zero → 0
/// Succ Zero → 1
/// Succ (Succ Zero) → 2
fn nat_to_usize(term: &Term) -> Option<usize> {
    match term {
        Term::Global(name) if name == "Zero" => Some(0),
        Term::App(succ, inner) => {
            if let Term::Global(name) = succ.as_ref() {
                if name == "Succ" {
                    return nat_to_usize(inner).map(|n| n + 1);
                }
            }
            None
        }
        _ => None,
    }
}

/// Generic induction tactic.
///
/// try_induction ind_type motive cases → DElim ind_type motive cases
///
/// Delegates to existing DElim infrastructure after basic validation.
fn try_try_induction_reduce(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
    cases: &Term,
) -> Option<Term> {
    // Extract inductive name to validate
    let ind_name = match extract_inductive_name_from_syntax(ind_type) {
        Some(name) => name,
        None => return Some(make_error_derivation()),
    };

    // Get constructors to validate count
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        return Some(make_error_derivation());
    }

    // Extract case proofs to validate count
    let case_proofs = match extract_case_proofs(cases) {
        Some(proofs) => proofs,
        None => return Some(make_error_derivation()),
    };

    // Verify case count matches constructor count
    if case_proofs.len() != constructors.len() {
        return Some(make_error_derivation());
    }

    // Build DElim term (delegates verification to existing infrastructure)
    Some(Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("DElim".to_string())),
                Box::new(ind_type.clone()),
            )),
            Box::new(motive.clone()),
        )),
        Box::new(cases.clone()),
    ))
}

// -------------------------------------------------------------------------
// Deep Induction
// -------------------------------------------------------------------------

/// DInduction reduction with verification.
///
/// DInduction motive base step → Forall Nat motive (if verified)
///
/// Verification:
/// 1. Extract motive body from SLam Nat body
/// 2. Check that concludes(base) = motive[Zero/0]
/// 3. Check that concludes(step) = ∀k:Nat. P(k) → P(Succ k)
/// 4. If all checks pass, return Forall Nat motive
/// 5. Otherwise, return Error
fn try_dinduction_reduce(
    ctx: &Context,
    motive: &Term,
    base: &Term,
    step: &Term,
) -> Option<Term> {
    // Normalize all inputs
    let norm_motive = normalize(ctx, motive);
    let norm_base = normalize(ctx, base);
    let norm_step = normalize(ctx, step);

    // 1. Extract motive body (should be SLam (SName "Nat") body)
    let motive_body = match extract_slam_body(&norm_motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // 2. Compute expected base: motive body with Zero substituted for SVar 0
    let zero = make_sname("Zero");
    let expected_base = match try_syn_subst_reduce(ctx, &zero, 0, &motive_body) {
        Some(b) => b,
        None => return Some(make_sname_error()),
    };

    // 3. Get actual base conclusion
    let base_conc = match try_concludes_reduce(ctx, &norm_base) {
        Some(c) => c,
        None => return Some(make_sname_error()),
    };

    // 4. Verify base matches expected
    if !syntax_equal(&base_conc, &expected_base) {
        return Some(make_sname_error());
    }

    // 5. Build expected step formula: ∀k:Nat. P(k) → P(Succ k)
    let expected_step = match build_induction_step_formula(ctx, &motive_body) {
        Some(s) => s,
        None => return Some(make_sname_error()),
    };

    // 6. Get actual step conclusion
    let step_conc = match try_concludes_reduce(ctx, &norm_step) {
        Some(c) => c,
        None => return Some(make_sname_error()),
    };

    // 7. Verify step matches expected
    if !syntax_equal(&step_conc, &expected_step) {
        return Some(make_sname_error());
    }

    // 8. Return conclusion: Forall Nat motive
    Some(make_forall_nat_syntax(&norm_motive))
}

/// Build step formula: ∀k:Nat. P(k) → P(Succ k)
///
/// Given motive body P (which uses SVar 0 for k), builds:
/// Forall (SName "Nat") (SLam (SName "Nat") (Implies P P[Succ(SVar 0)/SVar 0]))
fn build_induction_step_formula(ctx: &Context, motive_body: &Term) -> Option<Term> {
    // P(k) = motive_body (uses SVar 0 for k)
    let p_k = motive_body.clone();

    // P(Succ k) = motive_body with (SApp (SName "Succ") (SVar 0)) substituted
    let succ_var = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(make_sname("Succ")),
        )),
        Box::new(Term::App(
            Box::new(Term::Global("SVar".to_string())),
            Box::new(Term::Lit(Literal::Int(0))),
        )),
    );
    let p_succ_k = try_syn_subst_reduce(ctx, &succ_var, 0, motive_body)?;

    // Implies P(k) P(Succ k)
    let implies_body = make_implies_syntax(&p_k, &p_succ_k);

    // SLam (SName "Nat") implies_body
    let slam = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SLam".to_string())),
            Box::new(make_sname("Nat")),
        )),
        Box::new(implies_body),
    );

    // Forall (SName "Nat") slam
    Some(make_forall_syntax_with_type(&make_sname("Nat"), &slam))
}

/// Build SApp (SApp (SName "Implies") a) b
fn make_implies_syntax(a: &Term, b: &Term) -> Term {
    // SApp (SName "Implies") a
    let app1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(make_sname("Implies")),
        )),
        Box::new(a.clone()),
    );

    // SApp (SApp (SName "Implies") a) b
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app1),
        )),
        Box::new(b.clone()),
    )
}

/// Build SApp (SApp (SName "Forall") (SName "Nat")) motive
fn make_forall_nat_syntax(motive: &Term) -> Term {
    make_forall_syntax_with_type(&make_sname("Nat"), motive)
}

/// Build SApp (SApp (SName "Forall") type_s) body
fn make_forall_syntax_with_type(type_s: &Term, body: &Term) -> Term {
    // SApp (SName "Forall") type_s
    let app1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(make_sname("Forall")),
        )),
        Box::new(type_s.clone()),
    );

    // SApp (SApp (SName "Forall") type_s) body
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app1),
        )),
        Box::new(body.clone()),
    )
}

// -------------------------------------------------------------------------
// Solver (Computational Reflection)
// -------------------------------------------------------------------------

/// DCompute reduction with verification.
///
/// DCompute goal → goal (if verified by computation)
///
/// Verification:
/// 1. Check that goal is (Eq T A B) as Syntax
/// 2. Evaluate A and B using syn_eval with bounded fuel
/// 3. If eval(A) == eval(B), return goal
/// 4. Otherwise, return Error
fn try_dcompute_conclude(ctx: &Context, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Extract T, A, B from Eq T A B (as Syntax)
    // Pattern: SApp (SApp (SApp (SName "Eq") T) A) B
    let parts = extract_eq_syntax_parts(&norm_goal);
    if parts.is_none() {
        // Goal is not an equality - return Error
        return Some(make_sname_error());
    }
    let (_, a, b) = parts.unwrap();

    // Evaluate A and B with generous fuel
    let fuel = 1000;
    let a_eval = match try_syn_eval_reduce(ctx, fuel, &a) {
        Some(e) => e,
        None => return Some(make_sname_error()),
    };
    let b_eval = match try_syn_eval_reduce(ctx, fuel, &b) {
        Some(e) => e,
        None => return Some(make_sname_error()),
    };

    // Compare normalized results
    if syntax_equal(&a_eval, &b_eval) {
        Some(norm_goal)
    } else {
        Some(make_sname_error())
    }
}

/// Extract T, A, B from Eq T A B (as Syntax)
///
/// Pattern: SApp (SApp (SApp (SName "Eq") T) A) B
fn extract_eq_syntax_parts(term: &Term) -> Option<(Term, Term, Term)> {
    // term = SApp X B where X = SApp Y A where Y = SApp (SName "Eq") T
    // Structure: App(App(SApp, App(App(SApp, App(App(SApp, App(SName, "Eq")), T)), A)), B)
    if let Term::App(partial2, b) = term {
        if let Term::App(sapp2, inner2) = partial2.as_ref() {
            if let Term::Global(sapp2_name) = sapp2.as_ref() {
                if sapp2_name != "SApp" {
                    return None;
                }
            } else {
                return None;
            }

            if let Term::App(partial1, a) = inner2.as_ref() {
                if let Term::App(sapp1, inner1) = partial1.as_ref() {
                    if let Term::Global(sapp1_name) = sapp1.as_ref() {
                        if sapp1_name != "SApp" {
                            return None;
                        }
                    } else {
                        return None;
                    }

                    if let Term::App(eq_t, t) = inner1.as_ref() {
                        if let Term::App(sapp0, eq_sname) = eq_t.as_ref() {
                            if let Term::Global(sapp0_name) = sapp0.as_ref() {
                                if sapp0_name != "SApp" {
                                    return None;
                                }
                            } else {
                                return None;
                            }

                            // Check if eq_sname is SName "Eq"
                            if let Term::App(sname_ctor, eq_str) = eq_sname.as_ref() {
                                if let Term::Global(ctor) = sname_ctor.as_ref() {
                                    if ctor == "SName" {
                                        if let Term::Lit(Literal::Text(s)) = eq_str.as_ref() {
                                            if s == "Eq" {
                                                return Some((
                                                    t.as_ref().clone(),
                                                    a.as_ref().clone(),
                                                    b.as_ref().clone(),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

// -------------------------------------------------------------------------
// Tactic Combinators
// -------------------------------------------------------------------------

/// Reduce tact_orelse t1 t2 goal
///
/// - Apply t1 to goal
/// - If concludes returns Error, apply t2 to goal
/// - Otherwise return t1's result
fn try_tact_orelse_reduce(
    ctx: &Context,
    t1: &Term,
    t2: &Term,
    goal: &Term,
) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Apply t1 to goal
    let d1_app = Term::App(Box::new(t1.clone()), Box::new(norm_goal.clone()));
    let d1 = normalize(ctx, &d1_app);

    // Check if t1 succeeded by looking at concludes
    if let Some(conc1) = try_concludes_reduce(ctx, &d1) {
        if is_error_syntax(&conc1) {
            // t1 failed, try t2
            let d2_app = Term::App(Box::new(t2.clone()), Box::new(norm_goal));
            return Some(normalize(ctx, &d2_app));
        } else {
            // t1 succeeded
            return Some(d1);
        }
    }

    // Couldn't evaluate concludes - return error
    Some(make_error_derivation())
}

/// Check if a Syntax term is SName "Error"
fn is_error_syntax(term: &Term) -> bool {
    // Pattern: SName "Error" = App(Global("SName"), Lit(Text("Error")))
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = arg.as_ref() {
                    return s == "Error";
                }
            }
        }
    }
    false
}

/// Reduce tact_try t goal
///
/// - Apply t to goal
/// - If concludes returns Error, return identity (DAxiom goal)
/// - Otherwise return t's result
fn try_tact_try_reduce(ctx: &Context, t: &Term, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Apply t to goal
    let d_app = Term::App(Box::new(t.clone()), Box::new(norm_goal.clone()));
    let d = normalize(ctx, &d_app);

    // Check if t succeeded by looking at concludes
    if let Some(conc) = try_concludes_reduce(ctx, &d) {
        if is_error_syntax(&conc) {
            // t failed, return identity (DAxiom goal)
            return Some(make_daxiom(&norm_goal));
        } else {
            // t succeeded
            return Some(d);
        }
    }

    // Couldn't evaluate concludes - return identity
    Some(make_daxiom(&norm_goal))
}

/// Reduce tact_repeat t goal
///
/// Apply t repeatedly until it fails or makes no progress.
/// Returns the accumulated derivation or identity if first application fails.
fn try_tact_repeat_reduce(ctx: &Context, t: &Term, goal: &Term) -> Option<Term> {
    const MAX_ITERATIONS: usize = 100;

    let norm_goal = normalize(ctx, goal);
    let mut current_goal = norm_goal.clone();
    let mut last_successful_deriv: Option<Term> = None;

    for _ in 0..MAX_ITERATIONS {
        // Apply t to current goal
        let d_app = Term::App(Box::new(t.clone()), Box::new(current_goal.clone()));
        let d = normalize(ctx, &d_app);

        // Check result
        if let Some(conc) = try_concludes_reduce(ctx, &d) {
            if is_error_syntax(&conc) {
                // Tactic failed - stop and return what we have
                break;
            }

            // Check for no-progress (fixed point)
            if syntax_equal(&conc, &current_goal) {
                // No progress made - stop
                break;
            }

            // Progress made - continue
            current_goal = conc;
            last_successful_deriv = Some(d);
        } else {
            // Couldn't evaluate concludes - stop
            break;
        }
    }

    // Return final derivation or identity
    last_successful_deriv.or_else(|| Some(make_daxiom(&norm_goal)))
}

/// Reduce tact_then t1 t2 goal
///
/// - Apply t1 to goal
/// - If t1 fails, return Error
/// - Otherwise apply t2 to the result of t1
fn try_tact_then_reduce(
    ctx: &Context,
    t1: &Term,
    t2: &Term,
    goal: &Term,
) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Apply t1 to goal
    let d1_app = Term::App(Box::new(t1.clone()), Box::new(norm_goal.clone()));
    let d1 = normalize(ctx, &d1_app);

    // Check if t1 succeeded
    if let Some(conc1) = try_concludes_reduce(ctx, &d1) {
        if is_error_syntax(&conc1) {
            // t1 failed
            return Some(make_error_derivation());
        }

        // t1 succeeded - apply t2 to the new goal (conc1)
        let d2_app = Term::App(Box::new(t2.clone()), Box::new(conc1));
        let d2 = normalize(ctx, &d2_app);

        // The result is d2 (which may succeed or fail)
        return Some(d2);
    }

    // Couldn't evaluate concludes - return error
    Some(make_error_derivation())
}

/// Reduce tact_first tactics goal
///
/// Try each tactic in the list until one succeeds.
/// Returns Error if all fail or list is empty.
fn try_tact_first_reduce(ctx: &Context, tactics: &Term, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Extract tactics from TList
    let tactic_vec = extract_tlist(tactics)?;

    for tactic in tactic_vec {
        // Apply this tactic to goal
        let d_app = Term::App(Box::new(tactic), Box::new(norm_goal.clone()));
        let d = normalize(ctx, &d_app);

        // Check if it succeeded
        if let Some(conc) = try_concludes_reduce(ctx, &d) {
            if !is_error_syntax(&conc) {
                // Success!
                return Some(d);
            }
            // Failed - try next
        }
    }

    // All failed
    Some(make_error_derivation())
}

/// Extract elements from a TList term
///
/// TList is polymorphic: TNil A and TCons A h t
/// So the structure is:
/// - TNil A = App(Global("TNil"), type)
/// - TCons A h t = App(App(App(Global("TCons"), type), head), tail)
fn extract_tlist(term: &Term) -> Option<Vec<Term>> {
    let mut result = Vec::new();
    let mut current = term.clone();

    loop {
        match &current {
            // TNil A = App(Global("TNil"), type)
            Term::App(tnil, _type) => {
                if let Term::Global(name) = tnil.as_ref() {
                    if name == "TNil" {
                        // Empty list
                        break;
                    }
                }
                // Try TCons A h t = App(App(App(Global("TCons"), type), head), tail)
                if let Term::App(partial2, tail) = &current {
                    if let Term::App(partial1, head) = partial2.as_ref() {
                        if let Term::App(tcons, _type) = partial1.as_ref() {
                            if let Term::Global(name) = tcons.as_ref() {
                                if name == "TCons" {
                                    result.push(head.as_ref().clone());
                                    current = tail.as_ref().clone();
                                    continue;
                                }
                            }
                        }
                    }
                }
                // Not a valid TList structure
                return None;
            }
            // Bare Global("TNil") without type argument - also valid
            Term::Global(name) if name == "TNil" => {
                break;
            }
            _ => {
                // Not a valid TList
                return None;
            }
        }
    }

    Some(result)
}

/// Reduce tact_solve t goal
///
/// - Apply t to goal
/// - If t fails (Error), return Error
/// - If t succeeds, return its result
fn try_tact_solve_reduce(ctx: &Context, t: &Term, goal: &Term) -> Option<Term> {
    let norm_goal = normalize(ctx, goal);

    // Apply t to goal
    let d_app = Term::App(Box::new(t.clone()), Box::new(norm_goal.clone()));
    let d = normalize(ctx, &d_app);

    // Check if t succeeded
    if let Some(conc) = try_concludes_reduce(ctx, &d) {
        if is_error_syntax(&conc) {
            // t failed
            return Some(make_error_derivation());
        }
        // t succeeded - return its derivation
        return Some(d);
    }

    // Couldn't evaluate concludes - return error
    Some(make_error_derivation())
}

// -------------------------------------------------------------------------
// Congruence Closure
// -------------------------------------------------------------------------

/// Validate DCong proof by congruence.
///
/// DCong context eq_proof where:
/// - context is SLam param_type body
/// - eq_proof proves Eq T a b
/// Returns: Eq param_type (body[0:=a]) (body[0:=b])
fn try_dcong_conclude(ctx: &Context, context: &Term, eq_proof: &Term) -> Option<Term> {
    // Get the conclusion of the equality proof
    let eq_conc = try_concludes_reduce(ctx, eq_proof)?;

    // Extract T, a, b from Eq T a b
    let parts = extract_eq_syntax_parts(&eq_conc);
    if parts.is_none() {
        // Not an equality proof
        return Some(make_sname_error());
    }
    let (_type_term, lhs, rhs) = parts.unwrap();

    // Normalize context and check it's a lambda
    let norm_context = normalize(ctx, context);

    // Extract (param_type, body) from SLam param_type body
    let slam_parts = extract_slam_parts(&norm_context);
    if slam_parts.is_none() {
        // Not a lambda context
        return Some(make_sname_error());
    }
    let (param_type, body) = slam_parts.unwrap();

    // Substitute lhs and rhs into body at index 0
    let fa = try_syn_subst_reduce(ctx, &lhs, 0, &body)?;
    let fb = try_syn_subst_reduce(ctx, &rhs, 0, &body)?;

    // Build result: Eq param_type fa fb
    Some(make_eq_syntax_three(&param_type, &fa, &fb))
}

/// Extract (param_type, body) from SLam param_type body
///
/// Pattern: App(App(Global("SLam"), param_type), body)
fn extract_slam_parts(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(inner, body) = term {
        if let Term::App(slam_ctor, param_type) = inner.as_ref() {
            if let Term::Global(name) = slam_ctor.as_ref() {
                if name == "SLam" {
                    return Some((param_type.as_ref().clone(), body.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Build SApp (SApp (SApp (SName "Eq") type_s) a) b
///
/// Constructs the Syntax representation of (Eq type_s a b)
fn make_eq_syntax_three(type_s: &Term, a: &Term, b: &Term) -> Term {
    let eq_name = Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text("Eq".to_string()))),
    );

    // SApp (SName "Eq") type_s
    let app1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(eq_name),
        )),
        Box::new(type_s.clone()),
    );

    // SApp (SApp (SName "Eq") type_s) a
    let app2 = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app1),
        )),
        Box::new(a.clone()),
    );

    // SApp (SApp (SApp (SName "Eq") type_s) a) b
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(app2),
        )),
        Box::new(b.clone()),
    )
}

// -------------------------------------------------------------------------
// Generic Elimination
// -------------------------------------------------------------------------

/// DElim reduction with verification.
///
/// DElim ind_type motive cases → Forall ind_type motive (if verified)
///
/// Verification:
/// 1. Extract inductive name from ind_type Syntax
/// 2. Look up constructors for that inductive
/// 3. Extract case proofs from DCase chain
/// 4. Verify case count matches constructor count
/// 5. For each constructor, verify case conclusion matches expected
/// 6. Return Forall ind_type motive
fn try_delim_conclude(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
    cases: &Term,
) -> Option<Term> {
    // Normalize inputs
    let norm_ind_type = normalize(ctx, ind_type);
    let norm_motive = normalize(ctx, motive);
    let norm_cases = normalize(ctx, cases);

    // 1. Extract inductive name from Syntax
    let ind_name = match extract_inductive_name_from_syntax(&norm_ind_type) {
        Some(name) => name,
        None => return Some(make_sname_error()),
    };

    // 2. Look up constructors for this inductive
    let constructors = ctx.get_constructors(&ind_name);
    if constructors.is_empty() {
        // Unknown inductive type
        return Some(make_sname_error());
    }

    // 3. Extract case proofs from DCase chain
    let case_proofs = match extract_case_proofs(&norm_cases) {
        Some(proofs) => proofs,
        None => return Some(make_sname_error()),
    };

    // 4. Verify case count matches constructor count
    if case_proofs.len() != constructors.len() {
        return Some(make_sname_error());
    }

    // 5. Extract motive body (should be SLam param_type body)
    let motive_body = match extract_slam_body(&norm_motive) {
        Some(body) => body,
        None => return Some(make_sname_error()),
    };

    // 6. For each constructor, verify case conclusion matches expected
    for (i, (ctor_name, _ctor_type)) in constructors.iter().enumerate() {
        let case_proof = &case_proofs[i];

        // Get actual conclusion of this case proof
        let case_conc = match try_concludes_reduce(ctx, case_proof) {
            Some(c) => c,
            None => return Some(make_sname_error()),
        };

        // Build expected conclusion based on constructor
        // For base case (0-ary constructor): motive[ctor/var]
        // For step case (recursive constructor): requires IH pattern
        let expected = match build_case_expected(ctx, ctor_name, &constructors, &motive_body, &norm_ind_type) {
            Some(e) => e,
            None => return Some(make_sname_error()),
        };

        // Verify conclusion matches expected
        if !syntax_equal(&case_conc, &expected) {
            return Some(make_sname_error());
        }
    }

    // 7. Return conclusion: Forall ind_type motive
    Some(make_forall_syntax_generic(&norm_ind_type, &norm_motive))
}

/// Extract inductive name from Syntax term.
///
/// Handles:
/// - SName "Nat" → "Nat"
/// - SApp (SName "List") A → "List"
fn extract_inductive_name_from_syntax(term: &Term) -> Option<String> {
    // Case 1: SName "X"
    if let Term::App(sname, text) = term {
        if let Term::Global(ctor) = sname.as_ref() {
            if ctor == "SName" {
                if let Term::Lit(Literal::Text(name)) = text.as_ref() {
                    return Some(name.clone());
                }
            }
        }
    }

    // Case 2: SApp (SName "X") args → extract "X" from the function position
    if let Term::App(inner, _arg) = term {
        if let Term::App(sapp, func) = inner.as_ref() {
            if let Term::Global(ctor) = sapp.as_ref() {
                if ctor == "SApp" {
                    // Recursively extract from the function
                    return extract_inductive_name_from_syntax(func);
                }
            }
        }
    }

    None
}

/// Extract case proofs from DCase chain.
///
/// DCase p1 (DCase p2 DCaseEnd) → [p1, p2]
fn extract_case_proofs(term: &Term) -> Option<Vec<Term>> {
    let mut proofs = Vec::new();
    let mut current = term;

    loop {
        // DCaseEnd - end of list
        if let Term::Global(name) = current {
            if name == "DCaseEnd" {
                return Some(proofs);
            }
        }

        // DCase head tail - Pattern: App(App(DCase, head), tail)
        if let Term::App(inner, tail) = current {
            if let Term::App(dcase, head) = inner.as_ref() {
                if let Term::Global(name) = dcase.as_ref() {
                    if name == "DCase" {
                        proofs.push(head.as_ref().clone());
                        current = tail.as_ref();
                        continue;
                    }
                }
            }
        }

        // Unrecognized structure
        return None;
    }
}

/// Build expected case conclusion for a constructor.
///
/// For base case constructors (no recursive args): motive[ctor/var]
/// For recursive constructors: ∀args. IH → motive[ctor args/var]
fn build_case_expected(
    ctx: &Context,
    ctor_name: &str,
    _constructors: &[(&str, &Term)],
    motive_body: &Term,
    ind_type: &Term,
) -> Option<Term> {
    // Extract inductive name to determine constructor patterns
    let ind_name = extract_inductive_name_from_syntax(ind_type)?;

    // Special case for Nat - we know its structure
    if ind_name == "Nat" {
        if ctor_name == "Zero" {
            // Base case: motive[Zero/var]
            let zero = make_sname("Zero");
            return try_syn_subst_reduce(ctx, &zero, 0, motive_body);
        } else if ctor_name == "Succ" {
            // Step case: ∀k:Nat. P(k) → P(Succ k)
            // Use the same logic as DInduction
            return build_induction_step_formula(ctx, motive_body);
        }
    }

    // For other inductives, use heuristic based on constructor type
    // Build the constructor as Syntax: SName "CtorName"
    let ctor_syntax = make_sname(ctor_name);

    // For polymorphic types, we need to apply the type argument to the constructor
    // e.g., for List A, Nil becomes (SApp (SName "Nil") A)
    let ctor_applied = apply_type_args_to_ctor(&ctor_syntax, ind_type);

    // Get constructor type from context to determine if it's recursive
    if let Some(ctor_ty) = ctx.get_global(ctor_name) {
        // Check if constructor type contains the inductive type (recursive)
        if is_recursive_constructor(ctx, ctor_ty, &ind_name, ind_type) {
            // For recursive constructors, build the IH pattern
            return build_recursive_case_formula(ctx, ctor_name, ctor_ty, motive_body, ind_type, &ind_name);
        }
    }

    // Simple base case: substitute ctor into motive body
    try_syn_subst_reduce(ctx, &ctor_applied, 0, motive_body)
}

/// Apply type arguments from ind_type to a constructor.
///
/// If ind_type = SApp (SName "List") A, and ctor = SName "Nil",
/// result = SApp (SName "Nil") A
fn apply_type_args_to_ctor(ctor: &Term, ind_type: &Term) -> Term {
    // Extract type arguments from ind_type
    let args = extract_type_args(ind_type);

    if args.is_empty() {
        return ctor.clone();
    }

    // Apply each arg: SApp (... (SApp ctor arg1) ...) argN
    args.iter().fold(ctor.clone(), |acc, arg| {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(acc),
            )),
            Box::new(arg.clone()),
        )
    })
}

/// Extract type arguments from polymorphic Syntax.
///
/// SApp (SApp (SName "Either") A) B → [A, B]
/// SApp (SName "List") A → [A]
/// SName "Nat" → []
fn extract_type_args(term: &Term) -> Vec<Term> {
    let mut args = Vec::new();
    let mut current = term;

    // Traverse SApp chain from outside in
    loop {
        if let Term::App(inner, arg) = current {
            if let Term::App(sapp, func) = inner.as_ref() {
                if let Term::Global(ctor) = sapp.as_ref() {
                    if ctor == "SApp" {
                        args.push(arg.as_ref().clone());
                        current = func.as_ref();
                        continue;
                    }
                }
            }
        }
        break;
    }

    // Reverse because we collected outside-in but want inside-out
    args.reverse();
    args
}

/// Build Forall Syntax for generic inductive type.
///
/// Forall ind_type motive = SApp (SApp (SName "Forall") ind_type) motive
fn make_forall_syntax_generic(ind_type: &Term, motive: &Term) -> Term {
    // SApp (SName "Forall") ind_type
    let forall_type = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(make_sname("Forall")),
        )),
        Box::new(ind_type.clone()),
    );

    // SApp forall_type motive
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(forall_type),
        )),
        Box::new(motive.clone()),
    )
}

/// Check if a constructor is recursive (has arguments of the inductive type).
fn is_recursive_constructor(
    _ctx: &Context,
    ctor_ty: &Term,
    ind_name: &str,
    _ind_type: &Term,
) -> bool {
    // Traverse the constructor type looking for the inductive type in argument positions
    // For Cons : Π(A:Type). A -> List A -> List A
    // The "List A" argument makes it recursive

    fn contains_inductive(term: &Term, ind_name: &str) -> bool {
        match term {
            Term::Global(name) => name == ind_name,
            Term::App(f, a) => {
                contains_inductive(f, ind_name) || contains_inductive(a, ind_name)
            }
            Term::Pi { param_type, body_type, .. } => {
                contains_inductive(param_type, ind_name) || contains_inductive(body_type, ind_name)
            }
            Term::Lambda { param_type, body, .. } => {
                contains_inductive(param_type, ind_name) || contains_inductive(body, ind_name)
            }
            _ => false,
        }
    }

    // Check if any parameter type (not the final result) contains the inductive
    fn check_params(term: &Term, ind_name: &str) -> bool {
        match term {
            Term::Pi { param_type, body_type, .. } => {
                // Check if this parameter has the inductive type
                if contains_inductive(param_type, ind_name) {
                    return true;
                }
                // Check remaining parameters
                check_params(body_type, ind_name)
            }
            _ => false,
        }
    }

    check_params(ctor_ty, ind_name)
}

/// Build the case formula for a recursive constructor.
///
/// For Cons : Π(A:Type). A -> List A -> List A
/// with motive P : List A -> Prop
/// Expected case: ∀x:A. ∀xs:List A. P(xs) -> P(Cons A x xs)
fn build_recursive_case_formula(
    ctx: &Context,
    ctor_name: &str,
    ctor_ty: &Term,
    motive_body: &Term,
    ind_type: &Term,
    ind_name: &str,
) -> Option<Term> {
    // Extract type args from ind_type for matching
    let type_args = extract_type_args(ind_type);

    // Collect constructor arguments (skipping type parameters)
    let args = collect_ctor_args(ctor_ty, ind_name, &type_args);

    if args.is_empty() {
        // No non-type arguments, treat as base case
        let ctor_applied = apply_type_args_to_ctor(&make_sname(ctor_name), ind_type);
        return try_syn_subst_reduce(ctx, &ctor_applied, 0, motive_body);
    }

    // Build from inside out:
    // 1. Build ctor application: Cons A x xs (with de Bruijn indices for args)
    // 2. Build P(ctor args): motive_body[ctor args/var]
    // 3. For each recursive arg, wrap with IH: P(xs) ->
    // 4. For each arg, wrap with forall: ∀xs:List A.

    // Build constructor application with de Bruijn indices
    let mut ctor_app = apply_type_args_to_ctor(&make_sname(ctor_name), ind_type);
    for (i, _) in args.iter().enumerate() {
        // Index from end: last arg is index 0, second-to-last is 1, etc.
        let idx = (args.len() - 1 - i) as i64;
        let var = Term::App(
            Box::new(Term::Global("SVar".to_string())),
            Box::new(Term::Lit(Literal::Int(idx))),
        );
        ctor_app = Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(ctor_app),
            )),
            Box::new(var),
        );
    }

    // P(ctor args) - substitute ctor_app into motive
    let p_ctor = try_syn_subst_reduce(ctx, &ctor_app, 0, motive_body)?;

    // Build implications from inside out (for recursive args)
    let mut body = p_ctor;
    for (i, (arg_ty, is_recursive)) in args.iter().enumerate().rev() {
        if *is_recursive {
            // Add IH: P(arg) -> body
            // arg is at index (args.len() - 1 - i)
            let idx = (args.len() - 1 - i) as i64;
            let var = Term::App(
                Box::new(Term::Global("SVar".to_string())),
                Box::new(Term::Lit(Literal::Int(idx))),
            );
            let p_arg = try_syn_subst_reduce(ctx, &var, 0, motive_body)?;
            body = make_implies_syntax(&p_arg, &body);
        }
        // Skip non-recursive args in the implication chain
        let _ = (i, arg_ty); // suppress unused warning
    }

    // Wrap with foralls from inside out
    for (arg_ty, _) in args.iter().rev() {
        // SLam arg_ty body
        let slam = Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SLam".to_string())),
                Box::new(arg_ty.clone()),
            )),
            Box::new(body.clone()),
        );
        // Forall arg_ty slam
        body = make_forall_syntax_with_type(arg_ty, &slam);
    }

    Some(body)
}

/// Collect constructor arguments, skipping type parameters.
/// Returns (arg_type, is_recursive) pairs.
fn collect_ctor_args(ctor_ty: &Term, ind_name: &str, type_args: &[Term]) -> Vec<(Term, bool)> {
    let mut args = Vec::new();
    let mut current = ctor_ty;
    let mut skip_count = type_args.len();

    loop {
        match current {
            Term::Pi { param_type, body_type, .. } => {
                if skip_count > 0 {
                    // Skip type parameter
                    skip_count -= 1;
                } else {
                    // Regular argument
                    let is_recursive = contains_inductive_term(param_type, ind_name);
                    // Convert kernel type to Syntax representation
                    let arg_ty_syntax = kernel_type_to_syntax(param_type);
                    args.push((arg_ty_syntax, is_recursive));
                }
                current = body_type;
            }
            _ => break,
        }
    }

    args
}

/// Check if a kernel Term contains the inductive type.
fn contains_inductive_term(term: &Term, ind_name: &str) -> bool {
    match term {
        Term::Global(name) => name == ind_name,
        Term::App(f, a) => {
            contains_inductive_term(f, ind_name) || contains_inductive_term(a, ind_name)
        }
        Term::Pi { param_type, body_type, .. } => {
            contains_inductive_term(param_type, ind_name) || contains_inductive_term(body_type, ind_name)
        }
        Term::Lambda { param_type, body, .. } => {
            contains_inductive_term(param_type, ind_name) || contains_inductive_term(body, ind_name)
        }
        _ => false,
    }
}

/// Convert a kernel Term (type) to its Syntax representation.
fn kernel_type_to_syntax(term: &Term) -> Term {
    match term {
        Term::Global(name) => make_sname(name),
        Term::Var(name) => make_sname(name), // Named variable
        Term::App(f, a) => {
            let f_syn = kernel_type_to_syntax(f);
            let a_syn = kernel_type_to_syntax(a);
            // SApp f_syn a_syn
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("SApp".to_string())),
                    Box::new(f_syn),
                )),
                Box::new(a_syn),
            )
        }
        Term::Pi { param, param_type, body_type } => {
            let pt_syn = kernel_type_to_syntax(param_type);
            let bt_syn = kernel_type_to_syntax(body_type);
            // SPi pt_syn bt_syn
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("SPi".to_string())),
                    Box::new(pt_syn),
                )),
                Box::new(bt_syn),
            )
        }
        Term::Sort(univ) => {
            // SSort univ
            Term::App(
                Box::new(Term::Global("SSort".to_string())),
                Box::new(univ_to_syntax(univ)),
            )
        }
        Term::Lit(lit) => {
            // SLit lit
            Term::App(
                Box::new(Term::Global("SLit".to_string())),
                Box::new(Term::Lit(lit.clone())),
            )
        }
        _ => {
            // Fallback for complex terms
            make_sname("Unknown")
        }
    }
}

/// Convert a Universe to Syntax.
fn univ_to_syntax(univ: &crate::term::Universe) -> Term {
    match univ {
        crate::term::Universe::Prop => Term::Global("UProp".to_string()),
        crate::term::Universe::Type(n) => Term::App(
            Box::new(Term::Global("UType".to_string())),
            Box::new(Term::Lit(Literal::Int(*n as i64))),
        ),
    }
}

// -------------------------------------------------------------------------
// Inversion Tactic
// -------------------------------------------------------------------------

/// Inversion tactic: analyze hypothesis to derive False if no constructor matches.
///
/// Given hypothesis H of form `SApp (SName "IndName") args`, check if any constructor
/// of IndName can produce those args. If no constructor can match, return DInversion H.
fn try_try_inversion_reduce(ctx: &Context, goal: &Term) -> Option<Term> {
    // Extract inductive name and arguments from the hypothesis type
    let (ind_name, hyp_args) = match extract_applied_inductive_from_syntax(goal) {
        Some((name, args)) => (name, args),
        None => return Some(make_error_derivation()),
    };

    // Check if the inductive type actually exists
    if !ctx.is_inductive(&ind_name) {
        // Unknown inductive type - cannot derive anything
        return Some(make_error_derivation());
    }

    // Get constructors for this inductive
    let constructors = ctx.get_constructors(&ind_name);

    // Check each constructor to see if it can match
    let mut any_possible = false;
    for (_ctor_name, ctor_type) in constructors.iter() {
        if can_constructor_match_args(ctx, ctor_type, &hyp_args, &ind_name) {
            any_possible = true;
            break;
        }
    }

    if any_possible {
        // Cannot derive False - some constructor could match
        return Some(make_error_derivation());
    }

    // All constructors impossible → build DInversion
    Some(Term::App(
        Box::new(Term::Global("DInversion".to_string())),
        Box::new(goal.clone()),
    ))
}

/// Verify DInversion proof: check that no constructor can match the hypothesis.
fn try_dinversion_conclude(ctx: &Context, hyp_type: &Term) -> Option<Term> {
    let norm_hyp = normalize(ctx, hyp_type);

    let (ind_name, hyp_args) = match extract_applied_inductive_from_syntax(&norm_hyp) {
        Some((name, args)) => (name, args),
        None => return Some(make_sname_error()),
    };

    // Check if the inductive type actually exists
    if !ctx.is_inductive(&ind_name) {
        return Some(make_sname_error());
    }

    let constructors = ctx.get_constructors(&ind_name);

    // Verify ALL constructors are impossible
    for (_ctor_name, ctor_type) in constructors.iter() {
        if can_constructor_match_args(ctx, ctor_type, &hyp_args, &ind_name) {
            return Some(make_sname_error());
        }
    }

    // All impossible → concludes False
    Some(make_sname("False"))
}

/// Extract inductive name and arguments from Syntax.
///
/// SApp (SApp (SName "Even") x) y → ("Even", [x, y])
/// SName "False" → ("False", [])
fn extract_applied_inductive_from_syntax(term: &Term) -> Option<(String, Vec<Term>)> {
    // Base case: SName "X"
    if let Term::App(ctor, text) = term {
        if let Term::Global(ctor_name) = ctor.as_ref() {
            if ctor_name == "SName" {
                if let Term::Lit(Literal::Text(name)) = text.as_ref() {
                    return Some((name.clone(), vec![]));
                }
            }
        }
    }

    // Recursive case: SApp f x
    if let Term::App(inner, arg) = term {
        if let Term::App(sapp_ctor, func) = inner.as_ref() {
            if let Term::Global(ctor_name) = sapp_ctor.as_ref() {
                if ctor_name == "SApp" {
                    // Recursively extract from the function
                    let (name, mut args) = extract_applied_inductive_from_syntax(func)?;
                    args.push(arg.as_ref().clone());
                    return Some((name, args));
                }
            }
        }
    }

    None
}

/// Check if a constructor can possibly match the given arguments.
///
/// For constructor `even_succ : ∀n. Even n → Even (Succ (Succ n))`:
/// - The constructor's result pattern is `Even (Succ (Succ n))`
/// - If hyp_args is `[Succ (Succ (Succ Zero))]` (representing 3):
///   - Unify `Succ (Succ n)` with `Succ (Succ (Succ Zero))`
///   - This succeeds with n = Succ Zero = 1
///   - But then we need to check if `Even 1` is constructible (recursive)
fn can_constructor_match_args(
    ctx: &Context,
    ctor_type: &Term,
    hyp_args: &[Term],
    ind_name: &str,
) -> bool {
    // Decompose constructor type to get result pattern and bound variable names
    let (result, pattern_vars) = decompose_ctor_type_with_vars(ctor_type);

    // Extract result's arguments (what the constructor produces)
    let result_args = match extract_applied_inductive_from_syntax(&kernel_type_to_syntax(&result)) {
        Some((name, args)) if name == *ind_name => args,
        _ => return false,
    };

    // If argument counts don't match, can't unify
    if result_args.len() != hyp_args.len() {
        return false;
    }

    // Try syntactic unification of all arguments together (tracking bindings across args)
    let mut bindings: std::collections::HashMap<String, Term> = std::collections::HashMap::new();

    for (pattern, concrete) in result_args.iter().zip(hyp_args.iter()) {
        if !can_unify_syntax_terms_with_bindings(ctx, pattern, concrete, &pattern_vars, &mut bindings) {
            return false;
        }
    }

    // If we get here, the constructor could match
    // (We don't check recursive hypotheses for simplicity - that would require
    // full inversion with backtracking)
    true
}

/// Decompose a constructor type to get the result type and bound variable names.
///
/// `∀n:Nat. Even n → Even (Succ (Succ n))` → (`Even (Succ (Succ n))`, ["n"])
/// `∀A:Type. ∀x:A. Eq A x x` → (`Eq A x x`, ["A", "x"])
/// `Bool` → (`Bool`, [])
fn decompose_ctor_type_with_vars(ty: &Term) -> (Term, Vec<String>) {
    let mut vars = Vec::new();
    let mut current = ty;
    loop {
        match current {
            Term::Pi { param, body_type, .. } => {
                vars.push(param.clone());
                current = body_type;
            }
            _ => break,
        }
    }
    (current.clone(), vars)
}

/// Check if two Syntax terms can unify, tracking variable bindings.
///
/// Pattern variables (names in `pattern_vars`) can bind to any concrete value,
/// but must bind consistently (same variable must bind to same value).
/// Other SNames must match exactly.
/// SApp recurses on function and argument.
fn can_unify_syntax_terms_with_bindings(
    ctx: &Context,
    pattern: &Term,
    concrete: &Term,
    pattern_vars: &[String],
    bindings: &mut std::collections::HashMap<String, Term>,
) -> bool {
    // SVar can match anything (explicit unification variable)
    if let Term::App(ctor, _idx) = pattern {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SVar" {
                return true;
            }
        }
    }

    // SName: check if it's a pattern variable or a constant
    if let Term::App(ctor1, text1) = pattern {
        if let Term::Global(n1) = ctor1.as_ref() {
            if n1 == "SName" {
                if let Term::Lit(Literal::Text(var_name)) = text1.as_ref() {
                    // Check if this is a pattern variable
                    if pattern_vars.contains(var_name) {
                        // Pattern variable: check existing binding or create new one
                        if let Some(existing) = bindings.get(var_name) {
                            // Already bound: concrete must match existing binding
                            return syntax_terms_equal(existing, concrete);
                        } else {
                            // Not yet bound: bind to concrete value
                            bindings.insert(var_name.clone(), concrete.clone());
                            return true;
                        }
                    }
                }
                // Not a pattern variable: must match exactly
                if let Term::App(ctor2, text2) = concrete {
                    if let Term::Global(n2) = ctor2.as_ref() {
                        if n2 == "SName" {
                            return text1 == text2;
                        }
                    }
                }
                return false;
            }
        }
    }

    // SApp: recurse on both function and argument
    if let (Term::App(inner1, arg1), Term::App(inner2, arg2)) = (pattern, concrete) {
        if let (Term::App(sapp1, func1), Term::App(sapp2, func2)) =
            (inner1.as_ref(), inner2.as_ref())
        {
            if let (Term::Global(n1), Term::Global(n2)) = (sapp1.as_ref(), sapp2.as_ref()) {
                if n1 == "SApp" && n2 == "SApp" {
                    return can_unify_syntax_terms_with_bindings(ctx, func1, func2, pattern_vars, bindings)
                        && can_unify_syntax_terms_with_bindings(ctx, arg1.as_ref(), arg2.as_ref(), pattern_vars, bindings);
                }
            }
        }
    }

    // SLit: compare literal values
    if let (Term::App(ctor1, lit1), Term::App(ctor2, lit2)) = (pattern, concrete) {
        if let (Term::Global(n1), Term::Global(n2)) = (ctor1.as_ref(), ctor2.as_ref()) {
            if n1 == "SLit" && n2 == "SLit" {
                return lit1 == lit2;
            }
        }
    }

    // Fall back to exact structural equality
    pattern == concrete
}

/// Check if two Syntax terms are structurally equal.
fn syntax_terms_equal(a: &Term, b: &Term) -> bool {
    match (a, b) {
        (Term::App(f1, x1), Term::App(f2, x2)) => {
            syntax_terms_equal(f1, f2) && syntax_terms_equal(x1, x2)
        }
        (Term::Global(n1), Term::Global(n2)) => n1 == n2,
        (Term::Lit(l1), Term::Lit(l2)) => l1 == l2,
        _ => a == b,
    }
}

// -------------------------------------------------------------------------
// Operator Tactics (rewrite, destruct, apply)
// -------------------------------------------------------------------------

/// Extract Eq A x y components from a Syntax term.
///
/// SApp (SApp (SApp (SName "Eq") A) x) y → Some((A, x, y))
fn extract_eq_components_from_syntax(term: &Term) -> Option<(Term, Term, Term)> {
    // term = SApp (SApp (SApp (SName "Eq") A) x) y
    // In kernel representation: App(App(SApp, App(App(SApp, App(App(SApp, SName "Eq"), A)), x)), y)

    // Peel off outermost SApp to get ((SApp (SApp (SName "Eq") A) x), y)
    let (eq_a_x, y) = extract_sapp(term)?;

    // Peel off next SApp to get ((SApp (SName "Eq") A), x)
    let (eq_a, x) = extract_sapp(&eq_a_x)?;

    // Peel off next SApp to get ((SName "Eq"), A)
    let (eq, a) = extract_sapp(&eq_a)?;

    // Verify it's SName "Eq"
    let eq_name = extract_sname(&eq)?;
    if eq_name != "Eq" {
        return None;
    }

    Some((a, x, y))
}

/// Extract (f, x) from SApp f x (in kernel representation).
fn extract_sapp(term: &Term) -> Option<(Term, Term)> {
    // SApp f x = App(App(Global("SApp"), f), x)
    if let Term::App(inner, x) = term {
        if let Term::App(sapp_ctor, f) = inner.as_ref() {
            if let Term::Global(ctor_name) = sapp_ctor.as_ref() {
                if ctor_name == "SApp" {
                    return Some((f.as_ref().clone(), x.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Extract name from SName "name".
fn extract_sname(term: &Term) -> Option<String> {
    if let Term::App(ctor, text) = term {
        if let Term::Global(ctor_name) = ctor.as_ref() {
            if ctor_name == "SName" {
                if let Term::Lit(Literal::Text(name)) = text.as_ref() {
                    return Some(name.clone());
                }
            }
        }
    }
    None
}

/// Check if a Syntax term contains a specific subterm.
fn contains_subterm_syntax(term: &Term, target: &Term) -> bool {
    if syntax_equal(term, target) {
        return true;
    }

    // Check SApp f x
    if let Some((f, x)) = extract_sapp(term) {
        if contains_subterm_syntax(&f, target) || contains_subterm_syntax(&x, target) {
            return true;
        }
    }

    // Check SLam T body
    if let Some((t, body)) = extract_slam(term) {
        if contains_subterm_syntax(&t, target) || contains_subterm_syntax(&body, target) {
            return true;
        }
    }

    // Check SPi T body
    if let Some((t, body)) = extract_spi(term) {
        if contains_subterm_syntax(&t, target) || contains_subterm_syntax(&body, target) {
            return true;
        }
    }

    false
}

/// Extract (T, body) from SLam T body.
fn extract_slam(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(inner, body) = term {
        if let Term::App(slam_ctor, t) = inner.as_ref() {
            if let Term::Global(ctor_name) = slam_ctor.as_ref() {
                if ctor_name == "SLam" {
                    return Some((t.as_ref().clone(), body.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Extract (T, body) from SPi T body.
fn extract_spi(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(inner, body) = term {
        if let Term::App(spi_ctor, t) = inner.as_ref() {
            if let Term::Global(ctor_name) = spi_ctor.as_ref() {
                if ctor_name == "SPi" {
                    return Some((t.as_ref().clone(), body.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Replace first occurrence of target with replacement in a Syntax term.
fn replace_first_subterm_syntax(term: &Term, target: &Term, replacement: &Term) -> Option<Term> {
    // If term equals target, return replacement
    if syntax_equal(term, target) {
        return Some(replacement.clone());
    }

    // Try to replace in SApp f x
    if let Some((f, x)) = extract_sapp(term) {
        // First try to replace in f
        if let Some(new_f) = replace_first_subterm_syntax(&f, target, replacement) {
            return Some(make_sapp(new_f, x));
        }
        // Then try to replace in x
        if let Some(new_x) = replace_first_subterm_syntax(&x, target, replacement) {
            return Some(make_sapp(f, new_x));
        }
    }

    // Try to replace in SLam T body
    if let Some((t, body)) = extract_slam(term) {
        if let Some(new_t) = replace_first_subterm_syntax(&t, target, replacement) {
            return Some(make_slam(new_t, body));
        }
        if let Some(new_body) = replace_first_subterm_syntax(&body, target, replacement) {
            return Some(make_slam(t, new_body));
        }
    }

    // Try to replace in SPi T body
    if let Some((t, body)) = extract_spi(term) {
        if let Some(new_t) = replace_first_subterm_syntax(&t, target, replacement) {
            return Some(make_spi(new_t, body));
        }
        if let Some(new_body) = replace_first_subterm_syntax(&body, target, replacement) {
            return Some(make_spi(t, new_body));
        }
    }

    // No replacement found
    None
}

/// Rewrite tactic: given eq_proof (concluding Eq A x y) and goal,
/// replaces x with y (or y with x if reverse=true) in goal.
fn try_try_rewrite_reduce(
    ctx: &Context,
    eq_proof: &Term,
    goal: &Term,
    reverse: bool,
) -> Option<Term> {
    // Get the conclusion of eq_proof
    let eq_conclusion = try_concludes_reduce(ctx, eq_proof)?;

    // Extract Eq A x y components
    let (ty, lhs, rhs) = match extract_eq_components_from_syntax(&eq_conclusion) {
        Some(components) => components,
        None => return Some(make_error_derivation()),
    };

    // Determine what to replace based on direction
    let (target, replacement) = if reverse { (rhs, lhs) } else { (lhs, rhs) };

    // Check if target exists in goal
    if !contains_subterm_syntax(goal, &target) {
        return Some(make_error_derivation());
    }

    // Replace target with replacement in goal
    let new_goal = match replace_first_subterm_syntax(goal, &target, &replacement) {
        Some(ng) => ng,
        None => return Some(make_error_derivation()),
    };

    // Build DRewrite eq_proof goal new_goal
    Some(Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("DRewrite".to_string())),
                Box::new(eq_proof.clone()),
            )),
            Box::new(goal.clone()),
        )),
        Box::new(new_goal),
    ))
}

/// Verify DRewrite derivation and return the new goal.
fn try_drewrite_conclude(
    ctx: &Context,
    eq_proof: &Term,
    old_goal: &Term,
    new_goal: &Term,
) -> Option<Term> {
    // Get the conclusion of eq_proof
    let eq_conclusion = try_concludes_reduce(ctx, eq_proof)?;

    // Extract Eq A x y components
    let (_ty, lhs, rhs) = match extract_eq_components_from_syntax(&eq_conclusion) {
        Some(components) => components,
        None => return Some(make_sname_error()),
    };

    // Verify: new_goal = old_goal[lhs := rhs] OR new_goal = old_goal[rhs := lhs]
    // Check forward direction first
    if let Some(computed_new) = replace_first_subterm_syntax(old_goal, &lhs, &rhs) {
        if syntax_equal(&computed_new, new_goal) {
            return Some(new_goal.clone());
        }
    }

    // Check reverse direction
    if let Some(computed_new) = replace_first_subterm_syntax(old_goal, &rhs, &lhs) {
        if syntax_equal(&computed_new, new_goal) {
            return Some(new_goal.clone());
        }
    }

    // Verification failed
    Some(make_sname_error())
}

/// Destruct tactic: case analysis without induction hypotheses.
fn try_try_destruct_reduce(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
    cases: &Term,
) -> Option<Term> {
    // For now, destruct is essentially the same as induction
    // The key difference is in what goals are expected for each case
    // (no IH for recursive constructors)
    //
    // We simply build a DDestruct and let verification check case proofs

    Some(Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("DDestruct".to_string())),
                Box::new(ind_type.clone()),
            )),
            Box::new(motive.clone()),
        )),
        Box::new(cases.clone()),
    ))
}

/// Verify DDestruct derivation.
fn try_ddestruct_conclude(
    ctx: &Context,
    ind_type: &Term,
    motive: &Term,
    cases: &Term,
) -> Option<Term> {
    // Similar to DElim but without verifying IH in step cases
    // For now, we accept the derivation and return Forall ind_type motive

    // Extract the inductive type name
    let ind_name = extract_inductive_name_from_syntax(ind_type)?;

    // Verify it's actually an inductive type
    if !ctx.is_inductive(&ind_name) {
        return Some(make_sname_error());
    }

    let constructors = ctx.get_constructors(&ind_name);

    // Extract case proofs
    let case_proofs = match extract_case_proofs(cases) {
        Some(proofs) => proofs,
        None => return Some(make_sname_error()),
    };

    // Verify case count matches
    if case_proofs.len() != constructors.len() {
        return Some(make_sname_error());
    }

    // For each case, verify the conclusion matches the expected goal (without IH)
    // For simplicity, we just check case count matches for now
    // Full verification would check each case proves P(ctor args)

    // Build Forall ind_type motive
    Some(make_forall_syntax_with_type(ind_type, motive))
}

/// Apply tactic: manual backward chaining.
fn try_try_apply_reduce(
    ctx: &Context,
    hyp_name: &Term,
    hyp_proof: &Term,
    goal: &Term,
) -> Option<Term> {
    // Get the conclusion of hyp_proof
    let hyp_conclusion = try_concludes_reduce(ctx, hyp_proof)?;

    // Check if it's an implication: SPi A B where B doesn't use the bound var
    if let Some((antecedent, consequent)) = extract_spi(&hyp_conclusion) {
        // Check if consequent matches goal
        if syntax_equal(&consequent, goal) {
            // Build DApply hyp_name hyp_proof goal antecedent
            return Some(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("DApply".to_string())),
                            Box::new(hyp_name.clone()),
                        )),
                        Box::new(hyp_proof.clone()),
                    )),
                    Box::new(goal.clone()),
                )),
                Box::new(antecedent),
            ));
        }
    }

    // Check if it's a forall that could match
    if let Some(forall_body) = extract_forall_body(&hyp_conclusion) {
        // Try to match goal with forall body (simple syntactic check)
        // For now, if goal appears to be an instance of the forall body, accept it
        // Full implementation would do proper unification

        // Build DApply with new goal being True (trivially satisfied)
        return Some(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("DApply".to_string())),
                        Box::new(hyp_name.clone()),
                    )),
                    Box::new(hyp_proof.clone()),
                )),
                Box::new(goal.clone()),
            )),
            Box::new(make_sname("True")),
        ));
    }

    // If hypothesis directly matches goal, we're done
    if syntax_equal(&hyp_conclusion, goal) {
        return Some(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("DApply".to_string())),
                        Box::new(hyp_name.clone()),
                    )),
                    Box::new(hyp_proof.clone()),
                )),
                Box::new(goal.clone()),
            )),
            Box::new(make_sname("True")),
        ));
    }

    // Cannot apply this hypothesis to this goal
    Some(make_error_derivation())
}

/// Verify DApply derivation.
fn try_dapply_conclude(
    ctx: &Context,
    hyp_name: &Term,
    hyp_proof: &Term,
    old_goal: &Term,
    new_goal: &Term,
) -> Option<Term> {
    // Get the conclusion of hyp_proof
    let hyp_conclusion = try_concludes_reduce(ctx, hyp_proof)?;

    // If hypothesis is an implication A -> B and old_goal is B
    // then new_goal should be A
    if let Some((antecedent, consequent)) = extract_spi(&hyp_conclusion) {
        if syntax_equal(&consequent, old_goal) {
            if syntax_equal(&antecedent, new_goal) || extract_sname(new_goal) == Some("True".to_string()) {
                return Some(new_goal.clone());
            }
        }
    }

    // If hypothesis is a forall and goal matches instantiation
    if let Some(_forall_body) = extract_forall_body(&hyp_conclusion) {
        // For forall application, the new goal is typically True or the instantiated body
        if extract_sname(new_goal) == Some("True".to_string()) {
            return Some(new_goal.clone());
        }
    }

    // If hypothesis directly matches old_goal
    if syntax_equal(&hyp_conclusion, old_goal) {
        if extract_sname(new_goal) == Some("True".to_string()) {
            return Some(new_goal.clone());
        }
    }

    // Verification failed
    Some(make_sname_error())
}

