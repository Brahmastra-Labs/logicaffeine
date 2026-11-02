//! Simplifier Tactic
//!
//! Normalizes goals by applying rewrite rules from the context and performing
//! arithmetic evaluation. This is a general-purpose simplification tactic.
//!
//! # Algorithm
//!
//! The simp tactic works in four steps:
//! 1. **Extract hypotheses**: Peel implications and extract `x = t` as rewrites
//! 2. **Simplify LHS**: Apply substitutions and arithmetic bottom-up
//! 3. **Simplify RHS**: Apply substitutions and arithmetic bottom-up
//! 4. **Compare**: Goal succeeds if simplified LHS equals simplified RHS
//!
//! # Supported Simplifications
//!
//! - **Reflexive equalities**: `Eq a a` succeeds immediately
//! - **Constant folding**: `Eq (add 2 3) 5` simplifies to `Eq 5 5`
//! - **Hypothesis substitution**: Given `x = 0`, `x + 1` becomes `0 + 1` then `1`
//!
//! # Arithmetic Operations
//!
//! Supports `add`, `sub`, `mul`, `div`, `mod` on integer literals.
//! Non-literal arithmetic is left unevaluated.
//!
//! # Fuel Limit
//!
//! Simplification uses a fuel counter to prevent infinite loops from
//! cyclic rewrites. The default fuel is 1000 simplification steps.

use std::collections::HashMap;

use crate::term::{Literal, Term};

// =============================================================================
// SIMPLIFIED SYNTAX REPRESENTATION
// =============================================================================

/// Simplified representation of Syntax terms for rewriting.
///
/// This is a flattened version of the kernel's `Term` type, specialized
/// for the Syntax deep embedding. Unlike `Term`, `STerm` is designed
/// for efficient pattern matching and rewriting during simplification.
///
/// The correspondence is:
/// - `Term::App(Global("SLit"), Lit(n))` -> `STerm::Lit(n)`
/// - `Term::App(Global("SVar"), Lit(i))` -> `STerm::Var(i)`
/// - `Term::App(Global("SName"), Lit(s))` -> `STerm::Name(s)`
/// - `Term::App(App(Global("SApp"), f), a)` -> `STerm::App(f, a)`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum STerm {
    /// Integer literal from `SLit n`.
    Lit(i64),
    /// De Bruijn variable from `SVar i`.
    Var(i64),
    /// Named constant or function symbol from `SName s`.
    Name(String),
    /// Function application from `SApp f a`.
    App(Box<STerm>, Box<STerm>),
}

/// Substitution mapping variable indices to replacement terms.
///
/// Extracted from hypothesis equalities like `x = t`, where `x` is a
/// De Bruijn variable. Applied during simplification to replace variables
/// with their known values.
pub type Substitution = HashMap<i64, STerm>;

// =============================================================================
// TERM CONVERSION
// =============================================================================

/// Convert a kernel Term (representing Syntax) to our simplified STerm
fn term_to_sterm(term: &Term) -> Option<STerm> {
    // SLit n
    if let Some(n) = extract_slit(term) {
        return Some(STerm::Lit(n));
    }

    // SVar i
    if let Some(i) = extract_svar(term) {
        return Some(STerm::Var(i));
    }

    // SName s
    if let Some(s) = extract_sname(term) {
        return Some(STerm::Name(s));
    }

    // SApp f a
    if let Some((f, a)) = extract_sapp(term) {
        let sf = term_to_sterm(&f)?;
        let sa = term_to_sterm(&a)?;
        return Some(STerm::App(Box::new(sf), Box::new(sa)));
    }

    None
}

// =============================================================================
// SIMPLIFICATION ENGINE
// =============================================================================

/// Simplify an STerm, reporting honestly when the budget ran out before it finished.
///
/// `None` means EXHAUSTED — the simplification did not reach a fixpoint, and what it had
/// reached is not an answer. This is the same separation `reduction::normalize_checked` draws
/// against `reduction::normalize`, and it is here for the same reason, one rung more urgent:
/// [`check_goal`] decides a goal by simplifying BOTH sides and comparing, and
/// `reduction::try_dsimp_solve_conclude` ACCEPTS PROOFS through that verdict. A simplifier that
/// hands back how far it got makes "they came out equal" mean two different things — "both
/// reached the same normal form" and "both were abandoned in the same place" — and the door
/// cannot tell which it was handed.
///
/// The budget is reached by a SUBSTITUTION THAT NEVER SETTLES (`x = f x` as a hypothesis, which
/// nothing here forbids: `decompose_goal` reads hypotheses off the goal's own implications and
/// never checks that the variable is absent from the right-hand side) and by a term deeper than
/// the budget. Both are now refusals rather than answers.
fn simplify_checked(term: &STerm, subst: &Substitution, fuel: usize) -> Option<STerm> {
    if fuel == 0 {
        return None;
    }

    Some(match term {
        // Variables: apply substitution if bound
        STerm::Var(i) => {
            if let Some(replacement) = subst.get(i) {
                // Re-simplify the replacement (may enable more rewrites)
                simplify_checked(replacement, subst, fuel - 1)?
            } else {
                term.clone()
            }
        }

        // Literals and names are already simplified
        STerm::Lit(_) => term.clone(),
        STerm::Name(_) => term.clone(),

        // Applications: simplify children first, then try arithmetic
        STerm::App(f, a) => {
            let sf = simplify_checked(f, subst, fuel - 1)?;
            let sa = simplify_checked(a, subst, fuel - 1)?;

            // Try arithmetic simplification on the simplified application
            if let Some(result) = try_arithmetic(&sf, &sa) {
                return simplify_checked(&result, subst, fuel - 1);
            }

            STerm::App(Box::new(sf), Box::new(sa))
        }
    })
}

/// The BEST-EFFORT face of [`simplify_checked`]: an exhausted budget yields how far the
/// simplification got. ONE implementation with two faces, exactly as `normalize` is the
/// best-effort face of `normalize_checked`, so the two can never drift apart and answer
/// differently about the same term.
///
/// `#[cfg(test)]` states the fact rather than hiding it: NOTHING that certifies may use this
/// face, and today nothing else wants it either — the module's own unit tests pin the
/// simplifier's arithmetic directly and are its only caller. A production caller that genuinely
/// wants best-effort simplification lifts the attribute and inherits the argument above; one
/// that is about to CERTIFY must call [`simplify_checked`] instead.
#[cfg(test)]
fn simplify_sterm(term: &STerm, subst: &Substitution, fuel: usize) -> STerm {
    simplify_checked(term, subst, fuel).unwrap_or_else(|| term.clone())
}

/// Lean 4's `Int.ediv` / `Int.emod` — this folder's own implementation of the specification
/// written out in `reduction.rs::ediv_emod_i64`:
///
/// ```text
///   b ≠ 0  ⟹  a = b·q + r   ∧   0 ≤ r < |b|
///   b = 0  ⟹  q = 0  ∧  r = a
/// ```
///
/// It has to be exactly the reducer's answer, not merely a defensible one: `check_goal` is a
/// door `try_dsimp_solve_conclude` accepts proofs through, so a folder that still truncated
/// would CERTIFY `(div (-7) 2) = -3` — false under the kernel's own `div`.
fn ediv_emod(a: i64, b: i64) -> Option<(i64, i64)> {
    if b == 0 {
        return Some((0, a));
    }
    Some((a.checked_div_euclid(b)?, a.checked_rem_euclid(b)?))
}

/// Try to evaluate arithmetic operations on literals.
/// Handles: add, sub, mul, div, mod
fn try_arithmetic(func: &STerm, arg: &STerm) -> Option<STerm> {
    // Pattern: (add x) y, (sub x) y, (mul x) y, etc.
    // func = App(Name("add"), x)
    // arg = y
    if let STerm::App(op_box, x_box) = func {
        if let STerm::Name(op) = op_box.as_ref() {
            if let (STerm::Lit(x), STerm::Lit(y)) = (x_box.as_ref(), arg) {
                let result = match op.as_str() {
                    "add" => x.checked_add(*y)?,
                    "sub" => x.checked_sub(*y)?,
                    "mul" => x.checked_mul(*y)?,
                    "div" => ediv_emod(*x, *y)?.0,
                    "mod" => ediv_emod(*x, *y)?.1,
                    _ => return None,
                };
                return Some(STerm::Lit(result));
            }
        }
    }
    None
}

// =============================================================================
// GOAL DECOMPOSITION
// =============================================================================

/// Extract hypotheses and conclusion from a goal.
/// Handles nested implications: h1 -> h2 -> ... -> conclusion
/// Returns (substitution from hypotheses, conclusion)
fn decompose_goal(goal: &Term) -> (Substitution, Term) {
    let mut subst = HashMap::new();
    let mut current = goal.clone();

    // Peel off nested implications
    while let Some((hyp, rest)) = extract_implication(&current) {
        // Extract equality from hypothesis
        if let Some((lhs, rhs)) = extract_equality(&hyp) {
            // Convert LHS to check if it's a variable
            if let Some(st_lhs) = term_to_sterm(&lhs) {
                if let STerm::Var(i) = st_lhs {
                    // Variable on LHS: add substitution i → rhs
                    if let Some(st_rhs) = term_to_sterm(&rhs) {
                        subst.insert(i, st_rhs);
                    }
                }
            }
        }
        current = rest;
    }

    (subst, current)
}

/// Check if a goal is provable by simplification.
///
/// This is the main entry point for the simp tactic. It extracts
/// hypothesis equalities as rewrites, simplifies both sides of the
/// conclusion equality, and checks for syntactic equality.
///
/// # Supported Goals
///
/// - Bare equalities: `Eq a b` where `a` simplifies to `b`
/// - Implications: `(Eq x 0) -> (Eq (add x 1) 1)` using hypothesis as rewrite
/// - Nested implications with multiple hypothesis rewrites
///
/// # Returns
///
/// `true` if the simplified LHS equals the simplified RHS, `false` otherwise.
pub fn check_goal(goal: &Term) -> bool {
    let (subst, conclusion) = decompose_goal(goal);

    // Conclusion must be an equality
    let (lhs, rhs) = match extract_equality(&conclusion) {
        Some(eq) => eq,
        None => return false,
    };

    // Convert to STerm
    let st_lhs = match term_to_sterm(&lhs) {
        Some(t) => t,
        None => return false,
    };

    let st_rhs = match term_to_sterm(&rhs) {
        Some(t) => t,
        None => return false,
    };

    // Simplify both sides — through the HONEST face. A side that did not finish is not
    // evidence of anything, and `reduction::try_dsimp_solve_conclude` accepts proofs through
    // this verdict, so "both were abandoned in the same place" must not read as "both reached
    // the same normal form".
    const FUEL: usize = 1000;
    let (Some(simp_lhs), Some(simp_rhs)) = (
        simplify_checked(&st_lhs, &subst, FUEL),
        simplify_checked(&st_rhs, &subst, FUEL),
    ) else {
        return false;
    };

    // Check if they're equal
    simp_lhs == simp_rhs
}

// =============================================================================
// HELPER EXTRACTORS (same pattern as cc.rs)
// =============================================================================

/// Extract integer from SLit n
fn extract_slit(term: &Term) -> Option<i64> {
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SLit" {
                if let Term::Lit(Literal::Int(n)) = arg.as_ref() {
                    return Some(*n);
                }
            }
        }
    }
    None
}

/// Extract variable index from SVar i
fn extract_svar(term: &Term) -> Option<i64> {
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SVar" {
                if let Term::Lit(Literal::Int(i)) = arg.as_ref() {
                    return Some(*i);
                }
            }
        }
    }
    None
}

/// Extract name from SName "x"
fn extract_sname(term: &Term) -> Option<String> {
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

/// Extract unary application: SApp f a
fn extract_sapp(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(outer, arg) = term {
        if let Term::App(sapp, func) = outer.as_ref() {
            if let Term::Global(ctor) = sapp.as_ref() {
                if ctor == "SApp" {
                    return Some((func.as_ref().clone(), arg.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Extract an implication, in EVERY spelling the reflected surface has for it.
///
/// THE SPECIFICATION, stated once and implemented here in this module's own idiom (never copied
/// from `reduction.rs`'s reader, which is written over its own extractors):
///
/// > `A → B` is `SApp (SApp (SName "implies") A) B`, `SApp (SApp (SName "Implies") A) B`, or a
/// > NON-DEPENDENT `SPi A B` — one whose consequent mentions no de Bruijn index. A dependent
/// > `SPi` is a quantifier and is not an implication.
///
/// Reading only the lower-case spelling is why an induction step case — built capitalised — was
/// invisible to every goal reader, and why the literate `Statement:` surface's infix `implies`,
/// which desugars to a `Pi` and reflects as an `SPi`, could not be simplified at all.
fn extract_implication(term: &Term) -> Option<(Term, Term)> {
    if let Some((op, hyp, concl)) = extract_binary_app(term) {
        if op == "implies" || op == "Implies" {
            return Some((hyp, concl));
        }
    }
    let (hyp, concl) = extract_spi(term)?;
    if mentions_svar(&concl) {
        return None;
    }
    Some((hyp, concl))
}

/// Extract `SPi A B`.
fn extract_spi(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(outer, body) = term {
        if let Term::App(spi, param) = outer.as_ref() {
            if let Term::Global(ctor) = spi.as_ref() {
                if ctor == "SPi" {
                    return Some((param.as_ref().clone(), body.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Whether a reflected term mentions any de Bruijn index.
fn mentions_svar(term: &Term) -> bool {
    if extract_svar(term).is_some() {
        return true;
    }
    match term {
        Term::App(f, x) => mentions_svar(f) || mentions_svar(x),
        _ => false,
    }
}

/// Extract equality: SApp (SApp (SName "Eq") lhs) rhs
/// Also handles: SApp (SApp (SApp (SName "Eq") ty) lhs) rhs
fn extract_equality(term: &Term) -> Option<(Term, Term)> {
    // Try binary Eq first (no type annotation)
    if let Some((op, lhs, rhs)) = extract_binary_app(term) {
        if op == "Eq" || op == "eq" {
            return Some((lhs, rhs));
        }
    }

    // Try ternary Eq (with type annotation): (Eq T) lhs rhs
    if let Some((lhs, rhs)) = extract_ternary_eq(term) {
        return Some((lhs, rhs));
    }

    None
}

/// Extract ternary equality: SApp (SApp (SApp (SName "Eq") ty) lhs) rhs
fn extract_ternary_eq(term: &Term) -> Option<(Term, Term)> {
    // term = SApp func rhs, where func = SApp (SApp (SName "Eq") ty) lhs
    let (func, rhs) = extract_sapp(term)?;

    // func = SApp func2 lhs, where func2 = SApp (SName "Eq") ty
    let (func2, lhs) = extract_sapp(&func)?;

    // func2 = SApp eq_name ty, where eq_name = SName "Eq"
    let (eq_name, _ty) = extract_sapp(&func2)?;

    // Check that eq_name is SName "Eq"
    let name = extract_sname(&eq_name)?;
    if name == "Eq" {
        return Some((lhs, rhs));
    }

    None
}

/// Extract binary application: SApp (SApp (SName "op") a) b
fn extract_binary_app(term: &Term) -> Option<(String, Term, Term)> {
    if let Term::App(outer, b) = term {
        if let Term::App(sapp_outer, inner) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    if let Term::App(partial, a) = inner.as_ref() {
                        if let Term::App(sapp_inner, op_term) = partial.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    if let Some(op) = extract_sname(op_term) {
                                        return Some((op, a.as_ref().clone(), b.as_ref().clone()));
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
// UNIT TESTS
// =============================================================================

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/simp/tests.rs"]
mod tests;
