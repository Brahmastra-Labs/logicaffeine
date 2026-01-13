//! Strict positivity checking for inductive types.
//!
//! An inductive type must appear only in "strictly positive" positions in its constructors.
//! Without this check, we could define paradoxical types like:
//!
//! ```text
//! Inductive Bad := Cons : (Bad -> False) -> Bad
//! ```
//!
//! This would allow encoding Russell's paradox and proving False.
//!
//! Strict Positivity Rules (from CIC):
//!
//! I is strictly positive in T iff:
//! 1. I does not occur in T, OR
//! 2. T = Π(x:A). B where:
//!    - If A = I exactly, it's a "recursive argument" (allowed)
//!    - Otherwise, I must NOT occur in A at all
//!    - AND I must be strictly positive in B
//!
//! Examples:
//! - `I -> I` is valid: first I is a recursive argument, second is result
//! - `(I -> X) -> I` is INVALID: I occurs inside the param type of another arrow
//! - `X -> I -> I` is valid: X has no I, second param is recursive arg

use super::error::{KernelError, KernelResult};
use super::term::Term;

/// Check strict positivity of an inductive type in a constructor type.
///
/// This is the main entry point for positivity checking.
pub fn check_positivity(inductive: &str, constructor: &str, ty: &Term) -> KernelResult<()> {
    check_strictly_positive(inductive, constructor, ty)
}

/// Check that the inductive appears only strictly positively.
///
/// At the top level of constructor type, we allow:
/// - I as a direct parameter type (recursive argument)
/// - I in the final result type
/// - But NOT I nested inside function types within parameters
fn check_strictly_positive(inductive: &str, constructor: &str, ty: &Term) -> KernelResult<()> {
    match ty {
        // Direct occurrence of the inductive is always fine
        // (either as recursive argument or result type)
        Term::Global(name) if name == inductive => Ok(()),

        // Pi type: Π(x:A). B
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            // Check the parameter type A
            // If A = I directly, it's a recursive argument (allowed)
            // Otherwise, I must not occur in A at all (checked via occurs_in)
            match param_type.as_ref() {
                Term::Global(name) if name == inductive => {
                    // Direct recursive argument - allowed
                }
                _ => {
                    // A is not directly I, so I must not occur anywhere in A
                    if occurs_in(inductive, param_type) {
                        return Err(KernelError::PositivityViolation {
                            inductive: inductive.to_string(),
                            constructor: constructor.to_string(),
                            reason: format!(
                                "'{}' occurs in negative position (inside parameter type)",
                                inductive
                            ),
                        });
                    }
                }
            }

            // Recursively check the body type B
            check_strictly_positive(inductive, constructor, body_type)
        }

        // Application: check both parts
        Term::App(func, arg) => {
            check_strictly_positive(inductive, constructor, func)?;
            check_strictly_positive(inductive, constructor, arg)
        }

        // Lambda (unusual in types, but handle it)
        Term::Lambda {
            param_type, body, ..
        } => {
            // Same rule as Pi for param_type
            match param_type.as_ref() {
                Term::Global(name) if name == inductive => {}
                _ => {
                    if occurs_in(inductive, param_type) {
                        return Err(KernelError::PositivityViolation {
                            inductive: inductive.to_string(),
                            constructor: constructor.to_string(),
                            reason: format!(
                                "'{}' occurs in negative position (inside lambda parameter)",
                                inductive
                            ),
                        });
                    }
                }
            }
            check_strictly_positive(inductive, constructor, body)
        }

        // Other terms: no occurrences of the inductive to worry about
        Term::Sort(_) => Ok(()),
        Term::Var(_) => Ok(()),
        Term::Global(_) => Ok(()), // Other globals, not the inductive
        Term::Lit(_) => Ok(()),    // Literals cannot contain inductives

        // Match in types (unusual but possible)
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            check_strictly_positive(inductive, constructor, discriminant)?;
            check_strictly_positive(inductive, constructor, motive)?;
            for case in cases {
                check_strictly_positive(inductive, constructor, case)?;
            }
            Ok(())
        }

        // Fix in types (very unusual)
        Term::Fix { body, .. } => check_strictly_positive(inductive, constructor, body),

        // Hole: type placeholder, no occurrences to check
        Term::Hole => Ok(()),
    }
}

/// Check if the inductive name occurs anywhere in the term.
fn occurs_in(inductive: &str, term: &Term) -> bool {
    match term {
        Term::Global(name) => name == inductive,
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) => false,
        Term::Pi {
            param_type,
            body_type,
            ..
        } => occurs_in(inductive, param_type) || occurs_in(inductive, body_type),
        Term::Lambda {
            param_type, body, ..
        } => occurs_in(inductive, param_type) || occurs_in(inductive, body),
        Term::App(func, arg) => occurs_in(inductive, func) || occurs_in(inductive, arg),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            occurs_in(inductive, discriminant)
                || occurs_in(inductive, motive)
                || cases.iter().any(|c| occurs_in(inductive, c))
        }
        Term::Fix { body, .. } => occurs_in(inductive, body),
        Term::Hole => false, // Holes don't contain inductives
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_recursive_arg() {
        // Nat -> Nat is valid (first Nat is direct recursive arg, second is result)
        let ty = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Global("Nat".to_string())),
            body_type: Box::new(Term::Global("Nat".to_string())),
        };
        assert!(check_positivity("Nat", "Succ", &ty).is_ok());
    }

    #[test]
    fn test_negative_inside_arrow() {
        // (Bad -> False) -> Bad has Bad inside an arrow within a param
        // Bad occurs in param_type `Bad -> False`, which is not directly Bad
        let bad_to_false = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Global("Bad".to_string())),
            body_type: Box::new(Term::Global("False".to_string())),
        };
        let ty = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(bad_to_false),
            body_type: Box::new(Term::Global("Bad".to_string())),
        };
        assert!(check_positivity("Bad", "Cons", &ty).is_err());
    }

    #[test]
    fn test_nested_negative() {
        // ((Tricky -> Nat) -> Nat) -> Tricky
        // Tricky appears inside the param type of the outer Pi
        // The outer param is ((Tricky -> Nat) -> Nat), which contains Tricky
        let tricky_to_nat = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Global("Tricky".to_string())),
            body_type: Box::new(Term::Global("Nat".to_string())),
        };
        let inner = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tricky_to_nat),
            body_type: Box::new(Term::Global("Nat".to_string())),
        };
        let make_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(inner),
            body_type: Box::new(Term::Global("Tricky".to_string())),
        };

        let result = check_positivity("Tricky", "Make", &make_type);
        assert!(result.is_err(), "Should reject nested negative: {:?}", result);
    }

    #[test]
    fn test_list_cons_valid() {
        // Cons : Nat -> List -> List
        // Both params are fine: Nat doesn't contain List, second param IS List directly
        let ty = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Global("Nat".to_string())),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(Term::Global("List".to_string())),
                body_type: Box::new(Term::Global("List".to_string())),
            }),
        };
        assert!(check_positivity("List", "Cons", &ty).is_ok());
    }
}
