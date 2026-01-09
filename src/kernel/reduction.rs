//! Term reduction for the Calculus of Constructions.
//!
//! Implements:
//! - Beta reduction: (λx. body) arg → body[x := arg]
//! - Iota reduction: match (Cᵢ args) ... → caseᵢ(args)
//! - Fix unfolding: fix f. body → body[f := fix f. body] (guarded)

use super::context::Context;
use super::term::{Literal, Term};
use super::type_checker::substitute;

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
        Term::Sort(_) | Term::Var(_) => term.clone(),

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

// =============================================================================
// PHASE 87: REFLECTION BUILTINS
// =============================================================================

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

    // tact_orelse t1 t2 goal (3 arguments)
    // Structure: (((tact_orelse t1) t2) goal)
    // Here: func = App(App(tact_orelse, t1), t2), arg = goal
    if let Term::App(partial1, t2) = func {
        if let Term::App(combinator, t1) = partial1.as_ref() {
            if let Term::Global(name) = combinator.as_ref() {
                if name == "tact_orelse" {
                    return try_tact_orelse_reduce(ctx, t1, t2, arg);
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

// =============================================================================
// PHASE 88: SUBSTITUTION BUILTINS
// =============================================================================

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

// =============================================================================
// PHASE 89: COMPUTATION BUILTINS
// =============================================================================

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

// =============================================================================
// PHASE 90: BOUNDED EVALUATION (THE CLOCK)
// =============================================================================

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

// =============================================================================
// PHASE 91: REIFICATION (THE QUOTE)
// =============================================================================

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

// =============================================================================
// PHASE 93: DIAGONAL LEMMA (SELF-REFERENCE)
// =============================================================================

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

// =============================================================================
// PHASE 92: INFERENCE RULES (THE LAW)
// =============================================================================

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
                                                    if s == "Implies" {
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

/// Extract body from SApp (SApp (SName "Forall") T) (SLam T body)
///
/// In kernel representation:
/// SApp (SApp (SName "Forall") T) (SLam T body) =
///   App(App(SApp, App(App(SApp, App(SName, "Forall")), T)), App(App(SLam, T), body))
fn extract_forall_body(term: &Term) -> Option<Term> {
    // term = App(App(SApp, X), lam)
    if let Term::App(outer, lam) = term {
        if let Term::App(sapp_outer, x) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    // x = App(App(SApp, App(SName, "Forall")), T)
                    if let Term::App(inner, _t) = x.as_ref() {
                        if let Term::App(sapp_inner, sname_forall) = inner.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    // sname_forall = App(SName, "Forall")
                                    if let Term::App(sname, text) = sname_forall.as_ref() {
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

// =============================================================================
// PHASE 96: TACTICS (THE WIZARD)
// =============================================================================

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
// PHASE 97: DEEP INDUCTION (THE INDUCTOR)
// =============================================================================

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

/// Build SName name
fn make_sname(name: &str) -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text(name.to_string()))),
    )
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

// =============================================================================
// PHASE 99: THE SOLVER (COMPUTATIONAL REFLECTION)
// =============================================================================

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

// =============================================================================
// PHASE 98: TACTIC COMBINATORS (THE STRATEGIST)
// =============================================================================

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

// =============================================================================
// PHASE 100: CONGRUENCE (THE SUMMIT)
// =============================================================================

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

// =============================================================================
// PHASE 101b: GENERIC ELIMINATION (DELIM)
// =============================================================================

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
fn univ_to_syntax(univ: &super::term::Universe) -> Term {
    match univ {
        super::term::Universe::Prop => Term::Global("UProp".to_string()),
        super::term::Universe::Type(n) => Term::App(
            Box::new(Term::Global("UType".to_string())),
            Box::new(Term::Lit(Literal::Int(*n as i64))),
        ),
    }
}
