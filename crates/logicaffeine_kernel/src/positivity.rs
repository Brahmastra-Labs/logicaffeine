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

use crate::error::{KernelError, KernelResult};
use crate::term::Term;

/// Check strict positivity of an inductive type in a constructor type.
///
/// This is the main entry point for positivity checking.
pub fn check_positivity(inductive: &str, constructor: &str, ty: &Term) -> KernelResult<()> {
    check_strictly_positive(&[inductive], constructor, ty)
}

/// Check strict positivity of a MUTUAL BLOCK of inductives in a constructor type.
///
/// A constructor of one block member may recursively reference ANY member — `Even`'s
/// `even_succ : Π(n). Odd n → Even (Succ n)` places the sibling `Odd` in a
/// strictly-positive recursive position. The block is treated as a single "inductive"
/// for positivity: an occurrence of any member to the RIGHT of every arrow is a
/// (recursive) occurrence and allowed; any member in a DOMAIN is a negative
/// occurrence and rejected — so a cross-block paradox `(Even n → False) → Odd n` is
/// caught exactly as a self-negative one is.
pub fn check_positivity_mutual(block: &[&str], constructor: &str, ty: &Term) -> KernelResult<()> {
    check_strictly_positive(block, constructor, ty)
}

/// Check that the inductive appears only strictly positively.
///
/// At the top level of constructor type, we allow:
/// - I as a direct parameter type (recursive argument)
/// - I in the final result type
/// - But NOT I nested inside function types within parameters
fn check_strictly_positive(block: &[&str], constructor: &str, ty: &Term) -> KernelResult<()> {
    match ty {
        // A universe-polymorphic reference cannot mention this (newly-declared) inductive.
        Term::Const { .. } => Ok(()),

        // Direct occurrence of a block member is always fine
        // (either as recursive argument or result type)
        Term::Global(name) if block.contains(&name.as_str()) => Ok(()),

        // Pi type: Π(x:A). B
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            // Check the parameter type A.
            // If A is a recursive argument (a block member `I` applied to its
            // parameters, with no block member occurring in the arguments — `I`,
            // `I a`, `List A`, `Odd n`, …), it is a strictly-positive recursive
            // occurrence (allowed). Otherwise no block member may occur in A at all.
            if !is_recursive_arg(block, param_type) && occurs_in(block, param_type) {
                return Err(KernelError::PositivityViolation {
                    inductive: block.join("/"),
                    constructor: constructor.to_string(),
                    reason: format!(
                        "'{}' occurs in negative position (inside parameter type)",
                        block.join("/")
                    ),
                });
            }

            // Recursively check the body type B
            check_strictly_positive(block, constructor, body_type)
        }

        // Application: check both parts
        Term::App(func, arg) => {
            check_strictly_positive(block, constructor, func)?;
            check_strictly_positive(block, constructor, arg)
        }

        // Lambda (unusual in types, but handle it)
        Term::Lambda {
            param_type, body, ..
        } => {
            // Same rule as Pi for param_type
            if !is_recursive_arg(block, param_type) && occurs_in(block, param_type) {
                return Err(KernelError::PositivityViolation {
                    inductive: block.join("/"),
                    constructor: constructor.to_string(),
                    reason: format!(
                        "'{}' occurs in negative position (inside lambda parameter)",
                        block.join("/")
                    ),
                });
            }
            check_strictly_positive(block, constructor, body)
        }

        // Other terms: no occurrences of the inductive to worry about
        Term::Sort(_) => Ok(()),
        Term::Var(_) => Ok(()),
        Term::Global(_) => Ok(()), // Other globals, not a block member
        Term::Lit(_) => Ok(()),    // Literals cannot contain inductives

        // Match in types (unusual but possible)
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            check_strictly_positive(block, constructor, discriminant)?;
            check_strictly_positive(block, constructor, motive)?;
            for case in cases {
                check_strictly_positive(block, constructor, case)?;
            }
            Ok(())
        }

        // Fix in types (very unusual)
        Term::Fix { body, .. } => check_strictly_positive(block, constructor, body),

        // Mutual fix in types (very unusual): check every definition's body.
        Term::MutualFix { defs, .. } => {
            for (_, body) in defs {
                check_strictly_positive(block, constructor, body)?;
            }
            Ok(())
        }

        // Let in types: the value and type are checked; the body carries the
        // positivity obligation of the constructor's remaining shape.
        Term::Let { ty, value, body, .. } => {
            if occurs_in(block, ty) || occurs_in(block, value) {
                return Err(KernelError::PositivityViolation {
                    inductive: block.join("/"),
                    constructor: constructor.to_string(),
                    reason: format!("'{}' occurs in a let-binding's type or value", block.join("/")),
                });
            }
            check_strictly_positive(block, constructor, body)
        }

        // Hole: type placeholder, no occurrences to check
        Term::Hole => Ok(()),
    }
}

/// True if `term` is a strictly-positive recursive occurrence of `inductive` —
/// possibly a FUNCTIONAL one. Two shapes:
/// - a TELESCOPE `Π(z:B). rest` where `inductive` does not occur in the domain
///   `B` (so it is not in a negative position) and `rest` is itself a recursive
///   occurrence — e.g. `Acc_intro`'s field `Π(y:A). R y x → Acc A R y`, or a
///   rose tree's `Nat → Tree`; and
/// - the BASE `I e₁ … eₙ`: the inductive applied to arguments that do not mention
///   it (`I`, `I a`, `List A`, an indexed `Acc A R y`).
///
/// This is exactly CIC strict positivity: the inductive may appear only to the
/// RIGHT of every arrow. A negative occurrence (`Bad → …`, `(Bad → X) → …`) puts
/// it in a domain, so `is_recursive_arg` returns `false` and the caller's
/// `occurs_in` check then rejects the constructor.
fn is_recursive_arg(block: &[&str], term: &Term) -> bool {
    match term {
        Term::Pi { param_type, body_type, .. } => {
            !occurs_in(block, param_type) && is_recursive_arg(block, body_type)
        }
        _ => {
            let mut head = term;
            let mut args: Vec<&Term> = Vec::new();
            while let Term::App(func, arg) = head {
                args.push(arg.as_ref());
                head = func.as_ref();
            }
            matches!(head, Term::Global(name) if block.contains(&name.as_str()))
                && args.iter().all(|a| !occurs_in(block, a))
        }
    }
}

/// Check if any block member's name occurs anywhere in the term.
fn occurs_in(block: &[&str], term: &Term) -> bool {
    match term {
        Term::Global(name) => block.contains(&name.as_str()),
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Const { .. } => false,
        Term::Pi {
            param_type,
            body_type,
            ..
        } => occurs_in(block, param_type) || occurs_in(block, body_type),
        Term::Lambda {
            param_type, body, ..
        } => occurs_in(block, param_type) || occurs_in(block, body),
        Term::App(func, arg) => occurs_in(block, func) || occurs_in(block, arg),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            occurs_in(block, discriminant)
                || occurs_in(block, motive)
                || cases.iter().any(|c| occurs_in(block, c))
        }
        Term::Fix { body, .. } => occurs_in(block, body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| occurs_in(block, b)),
        Term::Let { ty, value, body, .. } => {
            occurs_in(block, ty) || occurs_in(block, value) || occurs_in(block, body)
        }
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
