//! Bounded SVA/FOL obligation → `ProofExpr` (the certified-prover seam).
//!
//! `BoundedExpr` is an SVA assertion (or Kripke-lowered FOL spec) already unrolled to
//! discrete timesteps, with each signal-at-time a `signal@t` variable. This module lowers
//! that bounded obligation into the `logicaffeine_proof::ProofExpr` vocabulary, where the
//! pure-Rust CDCL → RUP → kernel trust tiers discharge it with a certificate — the very seam
//! the grid solver uses, and the reason hardware proving runs in the browser with no Z3.
//!
//! Boolean (1-bit signal) obligations lower directly. Data-path operands — integers,
//! bitvectors, arrays, comparisons, quantifiers, uninterpreted applications — return `None`,
//! signalling that the caller must bit-blast them to the Boolean fragment first (the next
//! layer). Returning `None` rather than guessing keeps the lowering fail-closed.

use super::sva_to_verify::BoundedExpr;
use logicaffeine_proof::ProofExpr;

#[inline]
fn boxed(e: ProofExpr) -> Box<ProofExpr> {
    Box::new(e)
}

/// `ProofExpr` has no Boolean-literal node, so the constants are encoded as a tautology /
/// contradiction over a reserved atom. `Or(c, ¬c)` is true and `And(c, ¬c)` is false
/// regardless of `c`, so the encoding stays correct even if a signal happened to share the
/// reserved name.
fn truth() -> ProofExpr {
    let c = ProofExpr::Atom("__bool_const".to_string());
    ProofExpr::Or(boxed(c.clone()), boxed(ProofExpr::Not(boxed(c))))
}
fn falsity() -> ProofExpr {
    let c = ProofExpr::Atom("__bool_const".to_string());
    ProofExpr::And(boxed(c.clone()), boxed(ProofExpr::Not(boxed(c))))
}

/// Lower a Boolean-fragment `BoundedExpr` to `ProofExpr`, or `None` for any node outside the
/// propositional fragment (integers, bitvectors, arrays, ordered comparisons, quantifiers,
/// uninterpreted applications) — those require bit-blasting, not direct lowering.
pub fn bounded_to_proof(e: &BoundedExpr) -> Option<ProofExpr> {
    match e {
        BoundedExpr::Var(name) => Some(ProofExpr::Atom(name.clone())),
        BoundedExpr::Bool(true) => Some(truth()),
        BoundedExpr::Bool(false) => Some(falsity()),
        BoundedExpr::Not(p) => Some(ProofExpr::Not(boxed(bounded_to_proof(p)?))),
        BoundedExpr::And(p, q) => {
            Some(ProofExpr::And(boxed(bounded_to_proof(p)?), boxed(bounded_to_proof(q)?)))
        }
        BoundedExpr::Or(p, q) => {
            Some(ProofExpr::Or(boxed(bounded_to_proof(p)?), boxed(bounded_to_proof(q)?)))
        }
        BoundedExpr::Implies(p, q) => {
            Some(ProofExpr::Implies(boxed(bounded_to_proof(p)?), boxed(bounded_to_proof(q)?)))
        }
        // Equality of two Booleans is the biconditional; if either side is not Boolean, fall
        // back to bit-blasted bitvector equality.
        BoundedExpr::Eq(p, q) => match (bounded_to_proof(p), bounded_to_proof(q)) {
            (Some(a), Some(b)) => Some(ProofExpr::Iff(boxed(a), boxed(b))),
            _ => super::bitblast::lower_bool(e),
        },
        // Everything else: try the bit-blaster (datapath comparisons over bitvectors). It
        // returns `None` for genuinely unsupported nodes (Int arithmetic, quantifiers, …).
        _ => super::bitblast::lower_bool(e),
    }
}

#[cfg(test)]
mod tests {
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
}
