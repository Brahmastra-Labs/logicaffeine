use super::*;
use logicaffeine_proof::sat::{prove_equivalence, EquivOutcome};

fn v(s: &str) -> BoundedExpr {
    BoundedExpr::Var(s.to_string())
}
fn b(e: BoundedExpr) -> Box<BoundedExpr> {
    Box::new(e)
}

#[test]
fn overlapping_implication_proves_equivalent() {
    // `req |-> ack` at t0, against the same FOL lowering.
    let sva = BoundedExpr::Implies(b(v("req@0")), b(v("ack@0")));
    let fol = BoundedExpr::Implies(b(v("req@0")), b(v("ack@0")));
    let p = bounded_to_proof(&sva).unwrap();
    let q = bounded_to_proof(&fol).unwrap();
    assert_eq!(prove_equivalence(&p, &q), EquivOutcome::Equivalent);
}

#[test]
fn implication_vs_consequent_differ() {
    let sva = BoundedExpr::Implies(b(v("req@0")), b(v("ack@0")));
    let other = v("ack@0");
    let p = bounded_to_proof(&sva).unwrap();
    let q = bounded_to_proof(&other).unwrap();
    match prove_equivalence(&p, &q) {
        EquivOutcome::Differ(_) => {}
        o => panic!("expected Differ, got {:?}", o),
    }
}

#[test]
fn safety_demorgan_equivalent() {
    // `!(a && b)` ≡ `!a || !b` — a real safety-property rewrite.
    let lhs = BoundedExpr::Not(b(BoundedExpr::And(b(v("a@0")), b(v("b@0")))));
    let rhs = BoundedExpr::Or(
        b(BoundedExpr::Not(b(v("a@0")))),
        b(BoundedExpr::Not(b(v("b@0")))),
    );
    let p = bounded_to_proof(&lhs).unwrap();
    let q = bounded_to_proof(&rhs).unwrap();
    assert_eq!(prove_equivalence(&p, &q), EquivOutcome::Equivalent);
}

#[test]
fn boolean_constants_lower_and_differ() {
    let t = bounded_to_proof(&BoundedExpr::Bool(true)).unwrap();
    let f = bounded_to_proof(&BoundedExpr::Bool(false)).unwrap();
    match prove_equivalence(&t, &f) {
        EquivOutcome::Differ(_) => {}
        o => panic!("True vs False must Differ, got {:?}", o),
    }
}

#[test]
fn datapath_returns_none() {
    // Integers and bitvectors are outside the Boolean fragment → escalate.
    assert!(bounded_to_proof(&BoundedExpr::Int(5)).is_none());
    assert!(bounded_to_proof(&BoundedExpr::BitVecVar("data@0".to_string(), 8)).is_none());
    // …and an ordered comparison built over them.
    let cmp = BoundedExpr::Lt(b(BoundedExpr::Int(1)), b(BoundedExpr::Int(2)));
    assert!(bounded_to_proof(&cmp).is_none());
}
