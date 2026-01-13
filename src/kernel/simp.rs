//! Simplifier Tactic
//!
//! Normalizes goals by applying rewrite rules from the context.
//! Uses bottom-up term rewriting with pattern matching and arithmetic evaluation.
//!
//! The simp tactic handles:
//! - Reflexive equalities: (Eq a a)
//! - Constant folding: (Eq (add 2 3) 5)
//! - Hypothesis substitution: (implies (Eq x 0) (Eq (add x 1) 1))
//!
//! Algorithm: Bottom-up term rewriting
//! 1. Extract hypotheses from implications as rewrite rules (LHS → RHS)
//! 2. Simplify both sides of the equality goal
//! 3. Apply arithmetic simplification (add, sub, mul on literals)
//! 4. Check if simplified LHS equals simplified RHS

use std::collections::HashMap;

use super::term::{Literal, Term};

// =============================================================================
// SIMPLIFIED SYNTAX REPRESENTATION
// =============================================================================

/// A simplified representation of Syntax terms for rewriting
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum STerm {
    /// Integer literal (SLit n)
    Lit(i64),
    /// De Bruijn variable (SVar i)
    Var(i64),
    /// Named constant/function symbol (SName s)
    Name(String),
    /// Application (SApp f a)
    App(Box<STerm>, Box<STerm>),
}

/// Substitution from variable indices to STerm values
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

/// Convert STerm back to kernel Term (Syntax encoding)
fn sterm_to_term(st: &STerm) -> Term {
    match st {
        STerm::Lit(n) => Term::App(
            Box::new(Term::Global("SLit".to_string())),
            Box::new(Term::Lit(Literal::Int(*n))),
        ),
        STerm::Var(i) => Term::App(
            Box::new(Term::Global("SVar".to_string())),
            Box::new(Term::Lit(Literal::Int(*i))),
        ),
        STerm::Name(s) => Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text(s.clone()))),
        ),
        STerm::App(f, a) => Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(sterm_to_term(f)),
            )),
            Box::new(sterm_to_term(a)),
        ),
    }
}

// =============================================================================
// SIMPLIFICATION ENGINE
// =============================================================================

/// Simplify an STerm using the given substitution (from hypotheses)
/// and arithmetic evaluation.
fn simplify_sterm(term: &STerm, subst: &Substitution, fuel: usize) -> STerm {
    if fuel == 0 {
        return term.clone();
    }

    match term {
        // Variables: apply substitution if bound
        STerm::Var(i) => {
            if let Some(replacement) = subst.get(i) {
                // Re-simplify the replacement (may enable more rewrites)
                simplify_sterm(replacement, subst, fuel - 1)
            } else {
                term.clone()
            }
        }

        // Literals and names are already simplified
        STerm::Lit(_) => term.clone(),
        STerm::Name(_) => term.clone(),

        // Applications: simplify children first, then try arithmetic
        STerm::App(f, a) => {
            let sf = simplify_sterm(f, subst, fuel - 1);
            let sa = simplify_sterm(a, subst, fuel - 1);

            // Try arithmetic simplification on the simplified application
            if let Some(result) = try_arithmetic(&sf, &sa) {
                return simplify_sterm(&result, subst, fuel - 1);
            }

            STerm::App(Box::new(sf), Box::new(sa))
        }
    }
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
                    "div" if *y != 0 => x.checked_div(*y)?,
                    "mod" if *y != 0 => x.checked_rem(*y)?,
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

    // Simplify both sides
    const FUEL: usize = 1000;
    let simp_lhs = simplify_sterm(&st_lhs, &subst, FUEL);
    let simp_rhs = simplify_sterm(&st_rhs, &subst, FUEL);

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

/// Extract implication: SApp (SApp (SName "implies") hyp) concl
fn extract_implication(term: &Term) -> Option<(Term, Term)> {
    if let Some((op, hyp, concl)) = extract_binary_app(term) {
        if op == "implies" {
            return Some((hyp, concl));
        }
    }
    None
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
                                        return Some((
                                            op,
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
    None
}

// =============================================================================
// UNIT TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build SName "s"
    fn make_sname(s: &str) -> Term {
        Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text(s.to_string()))),
        )
    }

    /// Helper to build SVar i
    fn make_svar(i: i64) -> Term {
        Term::App(
            Box::new(Term::Global("SVar".to_string())),
            Box::new(Term::Lit(Literal::Int(i))),
        )
    }

    /// Helper to build SLit n
    fn make_slit(n: i64) -> Term {
        Term::App(
            Box::new(Term::Global("SLit".to_string())),
            Box::new(Term::Lit(Literal::Int(n))),
        )
    }

    /// Helper to build SApp f a
    fn make_sapp(f: Term, a: Term) -> Term {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(f),
            )),
            Box::new(a),
        )
    }

    #[test]
    fn test_term_to_sterm_lit() {
        let term = make_slit(42);
        let result = term_to_sterm(&term);
        assert_eq!(result, Some(STerm::Lit(42)));
    }

    #[test]
    fn test_term_to_sterm_var() {
        let term = make_svar(0);
        let result = term_to_sterm(&term);
        assert_eq!(result, Some(STerm::Var(0)));
    }

    #[test]
    fn test_term_to_sterm_name() {
        let term = make_sname("add");
        let result = term_to_sterm(&term);
        assert_eq!(result, Some(STerm::Name("add".to_string())));
    }

    #[test]
    fn test_term_to_sterm_app() {
        // (add 2 3) = SApp (SApp (SName "add") (SLit 2)) (SLit 3)
        let add_2 = make_sapp(make_sname("add"), make_slit(2));
        let add_2_3 = make_sapp(add_2, make_slit(3));
        let result = term_to_sterm(&add_2_3);

        let expected = STerm::App(
            Box::new(STerm::App(
                Box::new(STerm::Name("add".to_string())),
                Box::new(STerm::Lit(2)),
            )),
            Box::new(STerm::Lit(3)),
        );
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn test_arithmetic_add() {
        // (add 2) applied to 3
        let func = STerm::App(
            Box::new(STerm::Name("add".to_string())),
            Box::new(STerm::Lit(2)),
        );
        let arg = STerm::Lit(3);
        let result = try_arithmetic(&func, &arg);
        assert_eq!(result, Some(STerm::Lit(5)));
    }

    #[test]
    fn test_arithmetic_mul() {
        let func = STerm::App(
            Box::new(STerm::Name("mul".to_string())),
            Box::new(STerm::Lit(4)),
        );
        let arg = STerm::Lit(5);
        let result = try_arithmetic(&func, &arg);
        assert_eq!(result, Some(STerm::Lit(20)));
    }

    #[test]
    fn test_arithmetic_sub() {
        let func = STerm::App(
            Box::new(STerm::Name("sub".to_string())),
            Box::new(STerm::Lit(10)),
        );
        let arg = STerm::Lit(3);
        let result = try_arithmetic(&func, &arg);
        assert_eq!(result, Some(STerm::Lit(7)));
    }

    #[test]
    fn test_simplify_constant_addition() {
        // 2 + 3 should simplify to 5
        let term = STerm::App(
            Box::new(STerm::App(
                Box::new(STerm::Name("add".to_string())),
                Box::new(STerm::Lit(2)),
            )),
            Box::new(STerm::Lit(3)),
        );
        let result = simplify_sterm(&term, &HashMap::new(), 100);
        assert_eq!(result, STerm::Lit(5));
    }

    #[test]
    fn test_simplify_nested_arithmetic() {
        // (1 + 1) * 3 = 6
        let one_plus_one = STerm::App(
            Box::new(STerm::App(
                Box::new(STerm::Name("add".to_string())),
                Box::new(STerm::Lit(1)),
            )),
            Box::new(STerm::Lit(1)),
        );
        let term = STerm::App(
            Box::new(STerm::App(
                Box::new(STerm::Name("mul".to_string())),
                Box::new(one_plus_one),
            )),
            Box::new(STerm::Lit(3)),
        );
        let result = simplify_sterm(&term, &HashMap::new(), 100);
        assert_eq!(result, STerm::Lit(6));
    }

    #[test]
    fn test_simplify_with_substitution() {
        // x + 1 with x = 0 should give 1
        let x_plus_1 = STerm::App(
            Box::new(STerm::App(
                Box::new(STerm::Name("add".to_string())),
                Box::new(STerm::Var(0)),
            )),
            Box::new(STerm::Lit(1)),
        );
        let mut subst = HashMap::new();
        subst.insert(0, STerm::Lit(0));

        let result = simplify_sterm(&x_plus_1, &subst, 100);
        assert_eq!(result, STerm::Lit(1));
    }

    #[test]
    fn test_check_goal_reflexive() {
        // (Eq x x) should be provable
        let x = make_svar(0);
        let goal = make_sapp(make_sapp(make_sname("Eq"), x.clone()), x);
        assert!(check_goal(&goal), "simp should prove x = x");
    }

    #[test]
    fn test_check_goal_constant() {
        // (Eq (add 2 3) 5) should be provable
        let add_2_3 = make_sapp(make_sapp(make_sname("add"), make_slit(2)), make_slit(3));
        let goal = make_sapp(make_sapp(make_sname("Eq"), add_2_3), make_slit(5));
        assert!(check_goal(&goal), "simp should prove 2+3 = 5");
    }

    #[test]
    fn test_check_goal_with_hypothesis() {
        // (implies (Eq x 0) (Eq (add x 1) 1)) should be provable
        let x = make_svar(0);
        let zero = make_slit(0);
        let one = make_slit(1);

        let x_plus_1 = make_sapp(make_sapp(make_sname("add"), x.clone()), one.clone());
        let hyp = make_sapp(make_sapp(make_sname("Eq"), x), zero);
        let concl = make_sapp(make_sapp(make_sname("Eq"), x_plus_1), one);
        let goal = make_sapp(make_sapp(make_sname("implies"), hyp), concl);

        assert!(check_goal(&goal), "simp should prove x=0 -> x+1=1");
    }

    #[test]
    fn test_check_goal_false_equality() {
        // (Eq 2 3) should NOT be provable
        let goal = make_sapp(make_sapp(make_sname("Eq"), make_slit(2)), make_slit(3));
        assert!(!check_goal(&goal), "simp should NOT prove 2 = 3");
    }
}
