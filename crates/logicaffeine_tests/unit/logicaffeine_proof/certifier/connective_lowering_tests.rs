//! Curry-Howard lowering of the logical connectives to kernel types (`proof_expr_to_type`):
//! ¬P ≡ P→False, P→Q ≡ Π(_:P).Q, And/Or to their type constructors. Kills the And/Or swap,
//! implication param/body swap, and negation-target mutants.
use super::*;

fn atom(s: &str) -> ProofExpr {
    ProofExpr::Atom(s.to_string())
}
fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn imp(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}

#[test]
fn falsum_and_atoms_lower_to_globals() {
    assert_eq!(proof_expr_to_type(&atom("False")).unwrap(), g("False"));
    assert_eq!(proof_expr_to_type(&atom("⊥")).unwrap(), g("False"));
    assert_eq!(proof_expr_to_type(&atom("P")).unwrap(), g("P"));
}

#[test]
fn negation_lowers_to_an_arrow_into_false() {
    match proof_expr_to_type(&not(atom("P"))).unwrap() {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            assert_eq!(*param_type, g("P"), "¬P's domain is P");
            assert_eq!(*body_type, g("False"), "¬P's codomain is False");
        }
        other => panic!("¬P must lower to a Π into False, got {other:?}"),
    }
}

#[test]
fn implication_lowers_to_a_pi_domain_then_codomain() {
    match proof_expr_to_type(&imp(atom("P"), atom("Q"))).unwrap() {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            assert_eq!(*param_type, g("P"), "P→Q's domain is P (not Q)");
            assert_eq!(*body_type, g("Q"), "P→Q's codomain is Q (not P)");
        }
        other => panic!("P→Q must lower to a Π, got {other:?}"),
    }
}

#[test]
fn conjunction_and_disjunction_lower_to_their_type_constructors() {
    let expect = |head: &str| {
        Term::App(
            Box::new(Term::App(Box::new(g(head)), Box::new(g("P")))),
            Box::new(g("Q")),
        )
    };
    assert_eq!(
        proof_expr_to_type(&and(atom("P"), atom("Q"))).unwrap(),
        expect("And")
    );
    assert_eq!(
        proof_expr_to_type(&or(atom("P"), atom("Q"))).unwrap(),
        expect("Or")
    );
}
