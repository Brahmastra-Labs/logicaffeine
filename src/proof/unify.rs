// =============================================================================
// PROOF ENGINE - FIRST-ORDER UNIFICATION
// =============================================================================
// Robinson's Unification Algorithm with Occurs Check.
//
// This is the engine that allows us to scale to CIC (Calculus of Inductive
// Constructions). Unification is the core pattern-matching algorithm that
// enables:
// - In FOL: Match Mortal(x) with Mortal(Socrates) → {x ↦ Socrates}
// - In Peano: Match Add(Succ(n), 0) with Succ(n) → recursive proof
// - In CIC: Match types modulo beta-reduction (higher-order unification)
//
// PHASE 65: Alpha-Equivalence
// Bound variable names are arbitrary. ∃e P(e) ≡ ∃x P(x).
// We implement this by substituting fresh constants for bound variables
// when unifying quantified expressions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::proof::error::{ProofError, ProofResult};
use crate::proof::{ProofExpr, ProofTerm};

/// A substitution mapping variable names to terms.
/// e.g., { "x" ↦ Constant("Socrates"), "y" ↦ Variable("z") }
pub type Substitution = HashMap<String, ProofTerm>;

/// Substitution for expression-level meta-variables.
/// Maps hole names to their solutions (typically lambda abstractions).
/// e.g., { "P" ↦ Lambda { variable: "x", body: ... } }
pub type ExprSubstitution = HashMap<String, ProofExpr>;

// =============================================================================
// ALPHA-EQUIVALENCE SUPPORT
// =============================================================================

/// Global counter for generating fresh constants during alpha-renaming.
static ALPHA_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Generate a fresh constant for alpha-renaming.
/// Uses a prefix that cannot appear in user input to avoid collisions.
fn fresh_alpha_constant() -> ProofTerm {
    let id = ALPHA_COUNTER.fetch_add(1, Ordering::SeqCst);
    ProofTerm::Constant(format!("#α{}", id))
}

// =============================================================================
// BETA-REDUCTION (PHASE 66)
// =============================================================================
//
// "The Crawl Before the Walk"
//
// Beta-reduction normalizes lambda applications:
//   (λx. P(x))(a) → P[x := a]
//
// This is the computational engine that underpins type theory.
// In CIC, computation equals truth: if 2+2 reduces to 4, then P(2+2) = P(4).

/// Check if an expression is a constructor form (safe for fix unfolding).
/// Only constructors are safe guards against non-termination.
fn is_constructor_form(expr: &ProofExpr) -> bool {
    matches!(expr, ProofExpr::Ctor { .. })
}

/// Beta-reduce an expression to Weak Head Normal Form (WHNF).
///
/// Reduces lambda applications: (λx. body)(arg) → body[x := arg]
/// This is called before unification to normalize both expressions.
pub fn beta_reduce(expr: &ProofExpr) -> ProofExpr {
    match expr {
        // Beta-reduction and Fix unfolding
        ProofExpr::App(func, arg) => {
            // First, reduce both function and argument
            let func_reduced = beta_reduce(func);
            let arg_reduced = beta_reduce(arg);

            match func_reduced {
                // Beta-reduction: (λx. body)(arg) → body[x := arg]
                ProofExpr::Lambda { variable, body } => {
                    let result = substitute_expr_for_var(&body, &variable, &arg_reduced);
                    // Recursively reduce the result (handle nested redexes)
                    beta_reduce(&result)
                }

                // Fix unfolding: (fix f. body) arg → body[f := fix f. body] arg
                // Only unfold when arg is a constructor (guard against non-termination)
                ProofExpr::Fixpoint { ref name, ref body } if is_constructor_form(&arg_reduced) => {
                    // Substitute fix for f in body
                    let fix_expr = ProofExpr::Fixpoint {
                        name: name.clone(),
                        body: body.clone(),
                    };
                    let unfolded = substitute_expr_for_var(body, name, &fix_expr);
                    // Apply unfolded body to arg and reduce
                    let applied = ProofExpr::App(Box::new(unfolded), Box::new(arg_reduced));
                    beta_reduce(&applied)
                }

                _ => {
                    // No reduction possible, return normalized application
                    ProofExpr::App(Box::new(func_reduced), Box::new(arg_reduced))
                }
            }
        }

        // Reduce inside binary connectives
        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(beta_reduce(l)),
            Box::new(beta_reduce(r)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(beta_reduce(l)),
            Box::new(beta_reduce(r)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(beta_reduce(l)),
            Box::new(beta_reduce(r)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(beta_reduce(l)),
            Box::new(beta_reduce(r)),
        ),
        ProofExpr::Not(inner) => ProofExpr::Not(Box::new(beta_reduce(inner))),

        // Reduce inside quantifiers
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(beta_reduce(body)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(beta_reduce(body)),
        },

        // Reduce inside lambda bodies (but not the lambda itself - that's a value)
        ProofExpr::Lambda { variable, body } => ProofExpr::Lambda {
            variable: variable.clone(),
            body: Box::new(beta_reduce(body)),
        },

        // Reduce inside modal and temporal operators
        ProofExpr::Modal { domain, force, flavor, body } => ProofExpr::Modal {
            domain: domain.clone(),
            force: *force,
            flavor: flavor.clone(),
            body: Box::new(beta_reduce(body)),
        },
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(beta_reduce(body)),
        },

        // Reduce inside Ctor arguments
        ProofExpr::Ctor { name, args } => ProofExpr::Ctor {
            name: name.clone(),
            args: args.iter().map(beta_reduce).collect(),
        },

        // Iota reduction: match (Ctor args) with arms → selected arm body
        ProofExpr::Match { scrutinee, arms } => {
            let reduced_scrutinee = beta_reduce(scrutinee);

            // Try iota reduction: if scrutinee is a Ctor, select matching arm
            if let ProofExpr::Ctor { name: ctor_name, args: ctor_args } = &reduced_scrutinee {
                for arm in arms {
                    if &arm.ctor == ctor_name {
                        // Found matching arm - substitute constructor args for bindings
                        let mut result = arm.body.clone();
                        for (binding, arg) in arm.bindings.iter().zip(ctor_args.iter()) {
                            result = substitute_expr_for_var(&result, binding, arg);
                        }
                        // Continue reducing the result
                        return beta_reduce(&result);
                    }
                }
            }

            // No iota reduction possible - just reduce subexpressions
            ProofExpr::Match {
                scrutinee: Box::new(reduced_scrutinee),
                arms: arms.iter().map(|arm| crate::proof::MatchArm {
                    ctor: arm.ctor.clone(),
                    bindings: arm.bindings.clone(),
                    body: beta_reduce(&arm.body),
                }).collect(),
            }
        }

        // Reduce inside Fixpoint
        ProofExpr::Fixpoint { name, body } => ProofExpr::Fixpoint {
            name: name.clone(),
            body: Box::new(beta_reduce(body)),
        },

        // Atomic expressions don't reduce
        ProofExpr::Predicate { .. }
        | ProofExpr::Identity(_, _)
        | ProofExpr::Atom(_)
        | ProofExpr::NeoEvent { .. }
        | ProofExpr::TypedVar { .. }
        | ProofExpr::Unsupported(_)
        | ProofExpr::Hole(_)
        | ProofExpr::Term(_) => expr.clone(),
    }
}

/// Substitute an expression for a variable name in another expression.
///
/// Used for beta-reduction: (λx. body)(arg) → body[x := arg]
/// Handles variable capture by not substituting inside shadowing binders.
fn substitute_expr_for_var(body: &ProofExpr, var: &str, replacement: &ProofExpr) -> ProofExpr {
    match body {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(|t| substitute_term_for_var(t, var, replacement)).collect(),
            world: world.clone(),
        },

        ProofExpr::Identity(l, r) => ProofExpr::Identity(
            substitute_term_for_var(l, var, replacement),
            substitute_term_for_var(r, var, replacement),
        ),

        ProofExpr::Atom(a) => {
            // If the atom matches the variable, replace it
            if a == var {
                replacement.clone()
            } else {
                ProofExpr::Atom(a.clone())
            }
        }

        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(substitute_expr_for_var(l, var, replacement)),
            Box::new(substitute_expr_for_var(r, var, replacement)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(substitute_expr_for_var(l, var, replacement)),
            Box::new(substitute_expr_for_var(r, var, replacement)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(substitute_expr_for_var(l, var, replacement)),
            Box::new(substitute_expr_for_var(r, var, replacement)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(substitute_expr_for_var(l, var, replacement)),
            Box::new(substitute_expr_for_var(r, var, replacement)),
        ),
        ProofExpr::Not(inner) => ProofExpr::Not(
            Box::new(substitute_expr_for_var(inner, var, replacement))
        ),

        // Quantifiers: don't substitute if the variable is shadowed
        ProofExpr::ForAll { variable, body: inner } => {
            if variable == var {
                // Variable is shadowed, don't substitute in body
                body.clone()
            } else {
                ProofExpr::ForAll {
                    variable: variable.clone(),
                    body: Box::new(substitute_expr_for_var(inner, var, replacement)),
                }
            }
        }
        ProofExpr::Exists { variable, body: inner } => {
            if variable == var {
                body.clone()
            } else {
                ProofExpr::Exists {
                    variable: variable.clone(),
                    body: Box::new(substitute_expr_for_var(inner, var, replacement)),
                }
            }
        }

        // Lambda: don't substitute if the variable is shadowed
        ProofExpr::Lambda { variable, body: inner } => {
            if variable == var {
                body.clone()
            } else {
                ProofExpr::Lambda {
                    variable: variable.clone(),
                    body: Box::new(substitute_expr_for_var(inner, var, replacement)),
                }
            }
        }

        ProofExpr::App(f, a) => ProofExpr::App(
            Box::new(substitute_expr_for_var(f, var, replacement)),
            Box::new(substitute_expr_for_var(a, var, replacement)),
        ),

        ProofExpr::Modal { domain, force, flavor, body: inner } => ProofExpr::Modal {
            domain: domain.clone(),
            force: *force,
            flavor: flavor.clone(),
            body: Box::new(substitute_expr_for_var(inner, var, replacement)),
        },

        ProofExpr::Temporal { operator, body: inner } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(substitute_expr_for_var(inner, var, replacement)),
        },

        ProofExpr::NeoEvent { event_var, verb, roles } => {
            // Substitute in roles, but not the event_var itself
            ProofExpr::NeoEvent {
                event_var: event_var.clone(),
                verb: verb.clone(),
                roles: roles.iter().map(|(r, t)| {
                    (r.clone(), substitute_term_for_var(t, var, replacement))
                }).collect(),
            }
        }

        ProofExpr::Ctor { name, args } => ProofExpr::Ctor {
            name: name.clone(),
            args: args.iter().map(|a| substitute_expr_for_var(a, var, replacement)).collect(),
        },

        ProofExpr::Match { scrutinee, arms } => ProofExpr::Match {
            scrutinee: Box::new(substitute_expr_for_var(scrutinee, var, replacement)),
            arms: arms.iter().map(|arm| {
                // Don't substitute if var is bound in this arm
                if arm.bindings.contains(&var.to_string()) {
                    arm.clone()
                } else {
                    crate::proof::MatchArm {
                        ctor: arm.ctor.clone(),
                        bindings: arm.bindings.clone(),
                        body: substitute_expr_for_var(&arm.body, var, replacement),
                    }
                }
            }).collect(),
        },

        ProofExpr::Fixpoint { name, body: inner } => {
            if name == var {
                body.clone()
            } else {
                ProofExpr::Fixpoint {
                    name: name.clone(),
                    body: Box::new(substitute_expr_for_var(inner, var, replacement)),
                }
            }
        }

        ProofExpr::TypedVar { .. } | ProofExpr::Unsupported(_) => body.clone(),

        // Holes are meta-variables - don't substitute into them
        ProofExpr::Hole(_) => body.clone(),

        // Terms: substitute into the inner term
        ProofExpr::Term(t) => ProofExpr::Term(substitute_term_for_var(t, var, replacement)),
    }
}

/// Substitute an expression for a variable in a term.
///
/// When a variable in a term matches, convert the replacement expression to a term.
fn substitute_term_for_var(term: &ProofTerm, var: &str, replacement: &ProofExpr) -> ProofTerm {
    match term {
        ProofTerm::Variable(v) if v == var => {
            // Variable matches, convert replacement to term
            expr_to_term(replacement)
        }
        // BoundVarRef also participates in substitution (for instantiating quantified formulas)
        ProofTerm::BoundVarRef(v) if v == var => {
            expr_to_term(replacement)
        }
        ProofTerm::Variable(_) | ProofTerm::Constant(_) | ProofTerm::BoundVarRef(_) => term.clone(),
        ProofTerm::Function(name, args) => ProofTerm::Function(
            name.clone(),
            args.iter().map(|a| substitute_term_for_var(a, var, replacement)).collect(),
        ),
        ProofTerm::Group(terms) => ProofTerm::Group(
            terms.iter().map(|t| substitute_term_for_var(t, var, replacement)).collect(),
        ),
    }
}

/// Convert a simple ProofExpr to ProofTerm.
///
/// Used during beta-reduction when substituting an expression argument
/// into a predicate's term position.
fn expr_to_term(expr: &ProofExpr) -> ProofTerm {
    match expr {
        // Atoms become constants
        ProofExpr::Atom(s) => ProofTerm::Constant(s.clone()),

        // Zero-arity predicates become constants (e.g., "John" as a predicate name)
        ProofExpr::Predicate { name, args, .. } if args.is_empty() => {
            ProofTerm::Constant(name.clone())
        }

        // Predicates with args become functions
        ProofExpr::Predicate { name, args, .. } => {
            ProofTerm::Function(name.clone(), args.clone())
        }

        // Constructors become functions
        ProofExpr::Ctor { name, args } => {
            ProofTerm::Function(name.clone(), args.iter().map(expr_to_term).collect())
        }

        // TypedVar becomes a variable
        ProofExpr::TypedVar { name, .. } => ProofTerm::Variable(name.clone()),

        // Term is already a term - extract it directly
        ProofExpr::Term(t) => t.clone(),

        // Fallback: stringify the expression (covers Hole, etc.)
        _ => ProofTerm::Constant(format!("{}", expr)),
    }
}

// =============================================================================
// TERM-LEVEL UNIFICATION
// =============================================================================

/// Unify two terms, returning the Most General Unifier (MGU).
///
/// The MGU is the smallest substitution that makes both terms identical.
/// Returns an error if unification is impossible.
pub fn unify_terms(t1: &ProofTerm, t2: &ProofTerm) -> ProofResult<Substitution> {
    let mut subst = Substitution::new();
    unify_terms_with_subst(t1, t2, &mut subst)?;
    Ok(subst)
}

/// Internal unification with accumulating substitution.
fn unify_terms_with_subst(
    t1: &ProofTerm,
    t2: &ProofTerm,
    subst: &mut Substitution,
) -> ProofResult<()> {
    // Apply current substitution to both terms first
    let t1 = apply_subst_to_term(t1, subst);
    let t2 = apply_subst_to_term(t2, subst);

    match (&t1, &t2) {
        // Identical terms unify trivially
        (ProofTerm::Constant(c1), ProofTerm::Constant(c2)) if c1 == c2 => Ok(()),

        // Different constants cannot unify
        (ProofTerm::Constant(c1), ProofTerm::Constant(c2)) => {
            Err(ProofError::SymbolMismatch {
                left: c1.clone(),
                right: c2.clone(),
            })
        }

        // Variable on the left: bind it to the right term
        (ProofTerm::Variable(v), t) => {
            // Check if they're the same variable
            if let ProofTerm::Variable(v2) = t {
                if v == v2 {
                    return Ok(());
                }
            }
            // Occurs check: prevent infinite types
            if occurs(v, t) {
                return Err(ProofError::OccursCheck {
                    variable: v.clone(),
                    term: t.clone(),
                });
            }
            subst.insert(v.clone(), t.clone());
            Ok(())
        }

        // Variable on the right: bind it to the left term
        (t, ProofTerm::Variable(v)) => {
            // Occurs check
            if occurs(v, t) {
                return Err(ProofError::OccursCheck {
                    variable: v.clone(),
                    term: t.clone(),
                });
            }
            subst.insert(v.clone(), t.clone());
            Ok(())
        }

        // BoundVarRef on the left: treat like a variable for unification
        // This allows matching quantified formulas like ∀x P(x) against P(butler)
        (ProofTerm::BoundVarRef(v), t) => {
            if let ProofTerm::BoundVarRef(v2) = t {
                if v == v2 {
                    return Ok(());
                }
            }
            if occurs(v, t) {
                return Err(ProofError::OccursCheck {
                    variable: v.clone(),
                    term: t.clone(),
                });
            }
            subst.insert(v.clone(), t.clone());
            Ok(())
        }

        // BoundVarRef on the right: bind it to the left term
        (t, ProofTerm::BoundVarRef(v)) => {
            if occurs(v, t) {
                return Err(ProofError::OccursCheck {
                    variable: v.clone(),
                    term: t.clone(),
                });
            }
            subst.insert(v.clone(), t.clone());
            Ok(())
        }

        // Function unification: same name and arity, unify arguments pairwise
        (ProofTerm::Function(f1, args1), ProofTerm::Function(f2, args2)) => {
            if f1 != f2 {
                return Err(ProofError::SymbolMismatch {
                    left: f1.clone(),
                    right: f2.clone(),
                });
            }
            if args1.len() != args2.len() {
                return Err(ProofError::ArityMismatch {
                    expected: args1.len(),
                    found: args2.len(),
                });
            }
            for (a1, a2) in args1.iter().zip(args2.iter()) {
                unify_terms_with_subst(a1, a2, subst)?;
            }
            Ok(())
        }

        // Group unification: same length, unify elements pairwise
        (ProofTerm::Group(g1), ProofTerm::Group(g2)) => {
            if g1.len() != g2.len() {
                return Err(ProofError::ArityMismatch {
                    expected: g1.len(),
                    found: g2.len(),
                });
            }
            for (t1, t2) in g1.iter().zip(g2.iter()) {
                unify_terms_with_subst(t1, t2, subst)?;
            }
            Ok(())
        }

        // Any other combination fails
        _ => Err(ProofError::UnificationFailed {
            left: t1,
            right: t2,
        }),
    }
}

/// Check if a variable occurs in a term (for occurs check).
/// Prevents infinite types like x = f(x).
fn occurs(var: &str, term: &ProofTerm) -> bool {
    match term {
        ProofTerm::Variable(v) => v == var,
        ProofTerm::BoundVarRef(v) => v == var, // BoundVarRef participates in occurs check
        ProofTerm::Constant(_) => false,
        ProofTerm::Function(_, args) => args.iter().any(|a| occurs(var, a)),
        ProofTerm::Group(terms) => terms.iter().any(|t| occurs(var, t)),
    }
}

/// Apply a substitution to a term.
pub fn apply_subst_to_term(term: &ProofTerm, subst: &Substitution) -> ProofTerm {
    match term {
        ProofTerm::Variable(v) => {
            if let Some(replacement) = subst.get(v) {
                // Recursively apply to handle chains like {x ↦ y, y ↦ z}
                apply_subst_to_term(replacement, subst)
            } else {
                term.clone()
            }
        }
        // BoundVarRef participates in substitution (for instantiating quantified formulas)
        ProofTerm::BoundVarRef(v) => {
            if let Some(replacement) = subst.get(v) {
                apply_subst_to_term(replacement, subst)
            } else {
                term.clone()
            }
        }
        ProofTerm::Constant(_) => term.clone(),
        ProofTerm::Function(name, args) => {
            let new_args = args.iter().map(|a| apply_subst_to_term(a, subst)).collect();
            ProofTerm::Function(name.clone(), new_args)
        }
        ProofTerm::Group(terms) => {
            let new_terms = terms.iter().map(|t| apply_subst_to_term(t, subst)).collect();
            ProofTerm::Group(new_terms)
        }
    }
}

// =============================================================================
// EXPRESSION-LEVEL UNIFICATION
// =============================================================================

/// Unify two expressions, returning the MGU.
pub fn unify_exprs(e1: &ProofExpr, e2: &ProofExpr) -> ProofResult<Substitution> {
    let mut subst = Substitution::new();
    unify_exprs_with_subst(e1, e2, &mut subst)?;
    Ok(subst)
}

/// Internal expression unification with accumulating substitution.
fn unify_exprs_with_subst(
    e1: &ProofExpr,
    e2: &ProofExpr,
    subst: &mut Substitution,
) -> ProofResult<()> {
    // PHASE 66: Beta-reduce both expressions before unification.
    // This normalizes lambda applications: (λx. P(x))(a) → P(a)
    let e1 = beta_reduce(e1);
    let e2 = beta_reduce(e2);

    match (&e1, &e2) {
        // Atom unification
        (ProofExpr::Atom(a1), ProofExpr::Atom(a2)) if a1 == a2 => Ok(()),

        // Predicate unification: same name, unify arguments
        (
            ProofExpr::Predicate { name: n1, args: a1, world: w1 },
            ProofExpr::Predicate { name: n2, args: a2, world: w2 },
        ) => {
            if n1 != n2 {
                return Err(ProofError::SymbolMismatch {
                    left: n1.clone(),
                    right: n2.clone(),
                });
            }
            if a1.len() != a2.len() {
                return Err(ProofError::ArityMismatch {
                    expected: a1.len(),
                    found: a2.len(),
                });
            }
            // Unify worlds if both present
            match (w1, w2) {
                (Some(w1), Some(w2)) if w1 != w2 => {
                    return Err(ProofError::SymbolMismatch {
                        left: w1.clone(),
                        right: w2.clone(),
                    });
                }
                _ => {}
            }
            // Unify arguments
            for (t1, t2) in a1.iter().zip(a2.iter()) {
                unify_terms_with_subst(t1, t2, subst)?;
            }
            Ok(())
        }

        // Identity unification
        (ProofExpr::Identity(l1, r1), ProofExpr::Identity(l2, r2)) => {
            unify_terms_with_subst(l1, l2, subst)?;
            unify_terms_with_subst(r1, r2, subst)?;
            Ok(())
        }

        // Binary operators: same operator, unify both sides
        (ProofExpr::And(l1, r1), ProofExpr::And(l2, r2))
        | (ProofExpr::Or(l1, r1), ProofExpr::Or(l2, r2))
        | (ProofExpr::Implies(l1, r1), ProofExpr::Implies(l2, r2))
        | (ProofExpr::Iff(l1, r1), ProofExpr::Iff(l2, r2)) => {
            unify_exprs_with_subst(l1, l2, subst)?;
            unify_exprs_with_subst(r1, r2, subst)?;
            Ok(())
        }

        // Negation
        (ProofExpr::Not(inner1), ProofExpr::Not(inner2)) => {
            unify_exprs_with_subst(inner1, inner2, subst)
        }

        // Quantifiers: Alpha-equivalence - bound variable names are arbitrary
        // ∃e P(e) ≡ ∃x P(x) because they describe the same logical content
        (
            ProofExpr::ForAll { variable: v1, body: b1 },
            ProofExpr::ForAll { variable: v2, body: b2 },
        )
        | (
            ProofExpr::Exists { variable: v1, body: b1 },
            ProofExpr::Exists { variable: v2, body: b2 },
        ) => {
            // Generate a fresh constant to substitute for both bound variables.
            // Using a constant (not a variable) avoids capture issues.
            let fresh = fresh_alpha_constant();

            // Create substitutions for each bound variable
            let subst1: Substitution = [(v1.clone(), fresh.clone())].into_iter().collect();
            let subst2: Substitution = [(v2.clone(), fresh)].into_iter().collect();

            // Apply substitutions to bodies
            let body1_renamed = apply_subst_to_expr(b1, &subst1);
            let body2_renamed = apply_subst_to_expr(b2, &subst2);

            // Recursively unify the renamed bodies
            unify_exprs_with_subst(&body1_renamed, &body2_renamed, subst)
        }

        // Lambda expressions: Alpha-equivalence - λx.P(x) ≡ λy.P(y)
        (
            ProofExpr::Lambda { variable: v1, body: b1 },
            ProofExpr::Lambda { variable: v2, body: b2 },
        ) => {
            // Same alpha-renaming technique as quantifiers
            let fresh = fresh_alpha_constant();
            let subst1: Substitution = [(v1.clone(), fresh.clone())].into_iter().collect();
            let subst2: Substitution = [(v2.clone(), fresh)].into_iter().collect();
            let body1_renamed = apply_subst_to_expr(b1, &subst1);
            let body2_renamed = apply_subst_to_expr(b2, &subst2);
            unify_exprs_with_subst(&body1_renamed, &body2_renamed, subst)
        }

        // Application
        (ProofExpr::App(f1, a1), ProofExpr::App(f2, a2)) => {
            unify_exprs_with_subst(f1, f2, subst)?;
            unify_exprs_with_subst(a1, a2, subst)?;
            Ok(())
        }

        // NeoEvent: Alpha-equivalence for event variables
        // ∃e(Run(e) ∧ Agent(e, John)) should unify with ∃x(Run(x) ∧ Agent(x, John))
        (
            ProofExpr::NeoEvent {
                event_var: e1,
                verb: v1,
                roles: r1,
            },
            ProofExpr::NeoEvent {
                event_var: e2,
                verb: v2,
                roles: r2,
            },
        ) => {
            // Verb names must match (case-insensitive for robustness)
            if v1.to_lowercase() != v2.to_lowercase() {
                return Err(ProofError::SymbolMismatch {
                    left: v1.clone(),
                    right: v2.clone(),
                });
            }

            // Roles must have same length
            if r1.len() != r2.len() {
                return Err(ProofError::ArityMismatch {
                    expected: r1.len(),
                    found: r2.len(),
                });
            }

            // Alpha-equivalence: generate fresh constant for event variable
            let fresh = fresh_alpha_constant();
            let subst1: Substitution = [(e1.clone(), fresh.clone())].into_iter().collect();
            let subst2: Substitution = [(e2.clone(), fresh)].into_iter().collect();

            // Unify roles pairwise with alpha-renamed event variables
            for ((role1, term1), (role2, term2)) in r1.iter().zip(r2.iter()) {
                // Role names must match
                if role1 != role2 {
                    return Err(ProofError::SymbolMismatch {
                        left: role1.clone(),
                        right: role2.clone(),
                    });
                }
                // Apply alpha-renaming to terms and unify
                let t1_renamed = apply_subst_to_term(term1, &subst1);
                let t2_renamed = apply_subst_to_term(term2, &subst2);
                unify_terms_with_subst(&t1_renamed, &t2_renamed, subst)?;
            }
            Ok(())
        }

        // Temporal operators: Past(P), Future(P)
        // Same operator required, then unify bodies
        (
            ProofExpr::Temporal { operator: op1, body: b1 },
            ProofExpr::Temporal { operator: op2, body: b2 },
        ) => {
            if op1 != op2 {
                return Err(ProofError::ExprUnificationFailed {
                    left: e1.clone(),
                    right: e2.clone(),
                });
            }
            unify_exprs_with_subst(b1, b2, subst)
        }

        // Anything else fails
        _ => Err(ProofError::ExprUnificationFailed {
            left: e1.clone(),
            right: e2.clone(),
        }),
    }
}

/// Apply a substitution to an expression.
pub fn apply_subst_to_expr(expr: &ProofExpr, subst: &Substitution) -> ProofExpr {
    match expr {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(|a| apply_subst_to_term(a, subst)).collect(),
            world: world.clone(),
        },
        ProofExpr::Identity(l, r) => ProofExpr::Identity(
            apply_subst_to_term(l, subst),
            apply_subst_to_term(r, subst),
        ),
        ProofExpr::Atom(a) => ProofExpr::Atom(a.clone()),
        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(apply_subst_to_expr(l, subst)),
            Box::new(apply_subst_to_expr(r, subst)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(apply_subst_to_expr(l, subst)),
            Box::new(apply_subst_to_expr(r, subst)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(apply_subst_to_expr(l, subst)),
            Box::new(apply_subst_to_expr(r, subst)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(apply_subst_to_expr(l, subst)),
            Box::new(apply_subst_to_expr(r, subst)),
        ),
        ProofExpr::Not(inner) => ProofExpr::Not(Box::new(apply_subst_to_expr(inner, subst))),
        ProofExpr::ForAll { variable, body } => {
            // If the variable is being renamed (maps to a Variable), update the binder
            let new_variable = match subst.get(variable) {
                Some(ProofTerm::Variable(new_name)) => new_name.clone(),
                _ => variable.clone(),
            };
            ProofExpr::ForAll {
                variable: new_variable,
                body: Box::new(apply_subst_to_expr(body, subst)),
            }
        }
        ProofExpr::Exists { variable, body } => {
            // If the variable is being renamed (maps to a Variable), update the binder
            let new_variable = match subst.get(variable) {
                Some(ProofTerm::Variable(new_name)) => new_name.clone(),
                _ => variable.clone(),
            };
            ProofExpr::Exists {
                variable: new_variable,
                body: Box::new(apply_subst_to_expr(body, subst)),
            }
        }
        ProofExpr::Modal { domain, force, flavor, body } => ProofExpr::Modal {
            domain: domain.clone(),
            force: *force,
            flavor: flavor.clone(),
            body: Box::new(apply_subst_to_expr(body, subst)),
        },
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(apply_subst_to_expr(body, subst)),
        },
        ProofExpr::Lambda { variable, body } => {
            // If the variable is being renamed (maps to a Variable), update the binder
            let new_variable = match subst.get(variable) {
                Some(ProofTerm::Variable(new_name)) => new_name.clone(),
                _ => variable.clone(),
            };
            ProofExpr::Lambda {
                variable: new_variable,
                body: Box::new(apply_subst_to_expr(body, subst)),
            }
        }
        ProofExpr::App(f, a) => ProofExpr::App(
            Box::new(apply_subst_to_expr(f, subst)),
            Box::new(apply_subst_to_expr(a, subst)),
        ),
        ProofExpr::NeoEvent { event_var, verb, roles } => ProofExpr::NeoEvent {
            event_var: event_var.clone(),
            verb: verb.clone(),
            roles: roles
                .iter()
                .map(|(r, t)| (r.clone(), apply_subst_to_term(t, subst)))
                .collect(),
        },
        // Peano / Inductive Types
        ProofExpr::Ctor { name, args } => ProofExpr::Ctor {
            name: name.clone(),
            args: args.iter().map(|a| apply_subst_to_expr(a, subst)).collect(),
        },
        ProofExpr::Match { scrutinee, arms } => ProofExpr::Match {
            scrutinee: Box::new(apply_subst_to_expr(scrutinee, subst)),
            arms: arms
                .iter()
                .map(|arm| crate::proof::MatchArm {
                    ctor: arm.ctor.clone(),
                    bindings: arm.bindings.clone(),
                    body: apply_subst_to_expr(&arm.body, subst),
                })
                .collect(),
        },
        ProofExpr::Fixpoint { name, body } => ProofExpr::Fixpoint {
            name: name.clone(),
            body: Box::new(apply_subst_to_expr(body, subst)),
        },
        ProofExpr::TypedVar { name, typename } => ProofExpr::TypedVar {
            name: name.clone(),
            typename: typename.clone(),
        },
        ProofExpr::Unsupported(s) => ProofExpr::Unsupported(s.clone()),
        // Holes are meta-variables - term substitution doesn't apply
        ProofExpr::Hole(name) => ProofExpr::Hole(name.clone()),
        // Terms: apply substitution to the inner term
        ProofExpr::Term(t) => ProofExpr::Term(apply_subst_to_term(t, subst)),
    }
}

/// Compose two substitutions: apply s2 after s1.
/// The resulting substitution applies s1 first, then s2.
pub fn compose_substitutions(s1: Substitution, s2: Substitution) -> Substitution {
    let mut result: Substitution = s1
        .into_iter()
        .map(|(k, v)| (k, apply_subst_to_term(&v, &s2)))
        .collect();

    // Add bindings from s2 that aren't in s1
    for (k, v) in s2 {
        result.entry(k).or_insert(v);
    }

    result
}

// =============================================================================
// HIGHER-ORDER PATTERN UNIFICATION (PHASE 67)
// =============================================================================
//
// Miller Pattern Unification:
// Given ?F(x₁, ..., xₙ) = Body where x_i are distinct bound variables
// Solution: ?F = λx₁...λxₙ. Body
//
// This is decidable (unlike full higher-order unification) and sufficient for
// motive inference in structural induction.

/// Attempt higher-order pattern unification.
///
/// Given `lhs` and `rhs`, if `lhs` is of the form `Hole(h)(args...)` where
/// args are distinct BoundVarRefs, solve: `h = λargs. rhs`
///
/// Returns ExprSubstitution mapping hole names to lambda solutions.
pub fn unify_pattern(lhs: &ProofExpr, rhs: &ProofExpr) -> ProofResult<ExprSubstitution> {
    let mut solution = ExprSubstitution::new();
    unify_pattern_internal(lhs, rhs, &mut solution)?;
    Ok(solution)
}

/// Internal pattern unification with accumulating solution.
fn unify_pattern_internal(
    lhs: &ProofExpr,
    rhs: &ProofExpr,
    solution: &mut ExprSubstitution,
) -> ProofResult<()> {
    // Beta-reduce both sides first
    let lhs = beta_reduce(lhs);
    let rhs = beta_reduce(rhs);

    match &lhs {
        // Case: Bare hole ?P = rhs
        ProofExpr::Hole(h) => {
            solution.insert(h.clone(), rhs.clone());
            Ok(())
        }

        // Case: Application - might be Hole(h)(args...)
        ProofExpr::App(_, _) => {
            // Collect all arguments and find the head
            let (head, args) = collect_app_args(&lhs);

            if let ProofExpr::Hole(h) = head {
                // Check Miller pattern: all args must be distinct BoundVarRefs
                let var_args = extract_distinct_vars(&args)?;

                // Check that rhs only uses variables from var_args (scope check)
                check_scope(&rhs, &var_args)?;

                // Construct solution: h = λargs. rhs
                // Need to rename variables in rhs to match the BoundVarRef names
                let renamed_rhs = rename_vars_to_bound(&rhs, &var_args);
                let lambda = build_lambda(var_args, renamed_rhs);
                solution.insert(h.clone(), lambda);
                Ok(())
            } else {
                // Not a pattern, try structural equality
                if lhs == rhs {
                    Ok(())
                } else {
                    Err(ProofError::ExprUnificationFailed {
                        left: lhs.clone(),
                        right: rhs.clone(),
                    })
                }
            }
        }

        // Other cases: structural equality
        _ => {
            if lhs == rhs {
                Ok(())
            } else {
                Err(ProofError::ExprUnificationFailed {
                    left: lhs.clone(),
                    right: rhs.clone(),
                })
            }
        }
    }
}

/// Collect f(a)(b)(c) into (f, [a, b, c])
fn collect_app_args(expr: &ProofExpr) -> (ProofExpr, Vec<ProofExpr>) {
    let mut args = Vec::new();
    let mut current = expr.clone();

    while let ProofExpr::App(func, arg) = current {
        args.push(*arg);
        current = *func;
    }

    args.reverse();
    (current, args)
}

/// Extract distinct variable names from pattern arguments.
/// Fails if any arg is not a Term(BoundVarRef) or if duplicates exist.
fn extract_distinct_vars(args: &[ProofExpr]) -> ProofResult<Vec<String>> {
    let mut vars = Vec::new();
    for arg in args {
        match arg {
            ProofExpr::Term(ProofTerm::BoundVarRef(v)) => {
                if vars.contains(v) {
                    return Err(ProofError::PatternNotDistinct(v.clone()));
                }
                vars.push(v.clone());
            }
            _ => return Err(ProofError::NotAPattern(arg.clone())),
        }
    }
    Ok(vars)
}

/// Check that all free variables in expr are in the allowed set.
fn check_scope(expr: &ProofExpr, allowed: &[String]) -> ProofResult<()> {
    let free_vars = collect_free_vars(expr);
    for var in free_vars {
        if !allowed.contains(&var) {
            return Err(ProofError::ScopeViolation {
                var,
                allowed: allowed.to_vec(),
            });
        }
    }
    Ok(())
}

/// Collect free variables from an expression.
fn collect_free_vars(expr: &ProofExpr) -> Vec<String> {
    let mut vars = Vec::new();
    collect_free_vars_impl(expr, &mut vars, &mut Vec::new());
    vars
}

fn collect_free_vars_impl(expr: &ProofExpr, vars: &mut Vec<String>, bound: &mut Vec<String>) {
    match expr {
        ProofExpr::Predicate { args, .. } => {
            for arg in args {
                collect_free_vars_term(arg, vars, bound);
            }
        }
        ProofExpr::Identity(l, r) => {
            collect_free_vars_term(l, vars, bound);
            collect_free_vars_term(r, vars, bound);
        }
        ProofExpr::Atom(s) => {
            if !bound.contains(s) && !vars.contains(s) {
                vars.push(s.clone());
            }
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_free_vars_impl(l, vars, bound);
            collect_free_vars_impl(r, vars, bound);
        }
        ProofExpr::Not(inner) => collect_free_vars_impl(inner, vars, bound),
        ProofExpr::ForAll { variable, body }
        | ProofExpr::Exists { variable, body }
        | ProofExpr::Lambda { variable, body } => {
            bound.push(variable.clone());
            collect_free_vars_impl(body, vars, bound);
            bound.pop();
        }
        ProofExpr::App(f, a) => {
            collect_free_vars_impl(f, vars, bound);
            collect_free_vars_impl(a, vars, bound);
        }
        ProofExpr::Term(t) => collect_free_vars_term(t, vars, bound),
        ProofExpr::Hole(_) => {} // Holes don't contribute free vars
        _ => {} // Other cases don't add free vars
    }
}

fn collect_free_vars_term(term: &ProofTerm, vars: &mut Vec<String>, bound: &[String]) {
    match term {
        ProofTerm::Variable(v) => {
            if !bound.contains(v) && !vars.contains(v) {
                vars.push(v.clone());
            }
        }
        ProofTerm::Function(_, args) => {
            for arg in args {
                collect_free_vars_term(arg, vars, bound);
            }
        }
        ProofTerm::Group(terms) => {
            for t in terms {
                collect_free_vars_term(t, vars, bound);
            }
        }
        ProofTerm::Constant(_) | ProofTerm::BoundVarRef(_) => {}
    }
}

/// Rename Variable(x) to Variable(x) if x is in the bound vars list.
/// This ensures the solution lambda binds the right names.
fn rename_vars_to_bound(expr: &ProofExpr, bound_vars: &[String]) -> ProofExpr {
    // For the basic case, we don't need to rename since the RHS already uses
    // Variable("x") and we want the lambda to bind "x".
    // The key is just to use the same names.
    expr.clone()
}

/// Build λx₁.λx₂...λxₙ. body
fn build_lambda(vars: Vec<String>, body: ProofExpr) -> ProofExpr {
    vars.into_iter().rev().fold(body, |acc, var| {
        ProofExpr::Lambda {
            variable: var,
            body: Box::new(acc),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unify_same_constant() {
        let t1 = ProofTerm::Constant("a".into());
        let t2 = ProofTerm::Constant("a".into());
        let result = unify_terms(&t1, &t2);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_unify_different_constants() {
        let t1 = ProofTerm::Constant("a".into());
        let t2 = ProofTerm::Constant("b".into());
        let result = unify_terms(&t1, &t2);
        assert!(result.is_err());
    }

    #[test]
    fn test_unify_var_constant() {
        let t1 = ProofTerm::Variable("x".into());
        let t2 = ProofTerm::Constant("a".into());
        let result = unify_terms(&t1, &t2);
        assert!(result.is_ok());
        let subst = result.unwrap();
        assert_eq!(subst.get("x"), Some(&ProofTerm::Constant("a".into())));
    }

    #[test]
    fn test_occurs_check() {
        let t1 = ProofTerm::Variable("x".into());
        let t2 = ProofTerm::Function("f".into(), vec![ProofTerm::Variable("x".into())]);
        let result = unify_terms(&t1, &t2);
        assert!(matches!(result, Err(ProofError::OccursCheck { .. })));
    }

    #[test]
    fn test_compose_substitutions() {
        let mut s1 = Substitution::new();
        s1.insert("x".into(), ProofTerm::Variable("y".into()));

        let mut s2 = Substitution::new();
        s2.insert("y".into(), ProofTerm::Constant("a".into()));

        let composed = compose_substitutions(s1, s2);

        // x should map to a (via y)
        assert_eq!(composed.get("x"), Some(&ProofTerm::Constant("a".into())));
        // y should also map to a
        assert_eq!(composed.get("y"), Some(&ProofTerm::Constant("a".into())));
    }

    // =========================================================================
    // ALPHA-EQUIVALENCE TESTS (Phase 65)
    // =========================================================================

    #[test]
    fn test_alpha_equivalence_exists() {
        // ∃e P(e) should unify with ∃x P(x)
        let e1 = ProofExpr::Exists {
            variable: "e".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "run".to_string(),
                args: vec![ProofTerm::Variable("e".to_string())],
                world: None,
            }),
        };

        let e2 = ProofExpr::Exists {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "run".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
        };

        let result = unify_exprs(&e1, &e2);
        assert!(
            result.is_ok(),
            "Alpha-equivalent expressions should unify: {:?}",
            result
        );
    }

    #[test]
    fn test_alpha_equivalence_forall() {
        // ∀x P(x) should unify with ∀y P(y)
        let e1 = ProofExpr::ForAll {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "mortal".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
        };

        let e2 = ProofExpr::ForAll {
            variable: "y".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "mortal".to_string(),
                args: vec![ProofTerm::Variable("y".to_string())],
                world: None,
            }),
        };

        let result = unify_exprs(&e1, &e2);
        assert!(
            result.is_ok(),
            "Alpha-equivalent universals should unify: {:?}",
            result
        );
    }

    #[test]
    fn test_alpha_equivalence_nested() {
        // ∃e (Run(e) ∧ Agent(e, John)) should unify with ∃x (Run(x) ∧ Agent(x, John))
        let e1 = ProofExpr::Exists {
            variable: "e".to_string(),
            body: Box::new(ProofExpr::And(
                Box::new(ProofExpr::Predicate {
                    name: "run".to_string(),
                    args: vec![ProofTerm::Variable("e".to_string())],
                    world: None,
                }),
                Box::new(ProofExpr::Predicate {
                    name: "agent".to_string(),
                    args: vec![
                        ProofTerm::Variable("e".to_string()),
                        ProofTerm::Constant("John".to_string()),
                    ],
                    world: None,
                }),
            )),
        };

        let e2 = ProofExpr::Exists {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::And(
                Box::new(ProofExpr::Predicate {
                    name: "run".to_string(),
                    args: vec![ProofTerm::Variable("x".to_string())],
                    world: None,
                }),
                Box::new(ProofExpr::Predicate {
                    name: "agent".to_string(),
                    args: vec![
                        ProofTerm::Variable("x".to_string()),
                        ProofTerm::Constant("John".to_string()),
                    ],
                    world: None,
                }),
            )),
        };

        let result = unify_exprs(&e1, &e2);
        assert!(
            result.is_ok(),
            "Nested alpha-equivalent expressions should unify: {:?}",
            result
        );
    }
}
