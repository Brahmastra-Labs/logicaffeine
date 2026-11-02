//! Lib-unit coverage for the symmetry `Perm` algebra (apply/compose/extended/is_identity) —
//! a wrong permutation would break the soundness of every symmetry-breaking proof step.
use super::*;
use crate::cdcl::Lit;

#[test]
fn identity_fixes_every_literal() {
    let p = Perm::identity(3);
    assert!(p.is_identity());
    assert_eq!(p.num_vars(), 3);
    for v in 0..3 {
        assert_eq!(p.apply(Lit::pos(v)), Lit::pos(v));
        assert_eq!(p.apply(Lit::neg(v)), Lit::neg(v));
    }
}

#[test]
fn from_images_swaps_and_preserves_sign() {
    // Swap var 0 ↔ var 1.
    let p = Perm::from_images(vec![Lit::pos(1), Lit::pos(0)]);
    assert!(!p.is_identity());
    assert_eq!(p.apply(Lit::pos(0)), Lit::pos(1));
    assert_eq!(p.apply(Lit::pos(1)), Lit::pos(0));
    assert_eq!(
        p.apply(Lit::neg(0)),
        Lit::neg(1),
        "sign carries through the permutation"
    );
    assert_eq!(
        p.apply_clause(&[Lit::pos(0), Lit::neg(1)]),
        vec![Lit::pos(1), Lit::neg(0)]
    );
}

#[test]
fn compose_is_apply_other_then_self_and_a_swap_is_involutive() {
    let swap = Perm::from_images(vec![Lit::pos(1), Lit::pos(0)]);
    assert!(
        swap.compose(&swap).is_identity(),
        "a transposition ∘ itself = identity"
    );
}

#[test]
fn extended_is_identity_on_the_new_variables() {
    let swap = Perm::from_images(vec![Lit::pos(1), Lit::pos(0)]);
    let ext = swap.extended(4);
    assert_eq!(ext.num_vars(), 4);
    assert_eq!(
        ext.apply(Lit::pos(2)),
        Lit::pos(2),
        "a new variable is untouched"
    );
    assert_eq!(
        ext.apply(Lit::pos(0)),
        Lit::pos(1),
        "the original swap is preserved"
    );
}

#[test]
fn compose_applies_other_first_then_self_so_order_matters() {
    // swap∘swap = id is involutive and can't distinguish `self(other(l))` from `other(self(l))`.
    // Compose a 3-cycle (0→1→2→0) with a transposition (0↔1) — the two orders DISAGREE, pinning
    // that `other` is applied first.
    let cyc = Perm::from_images(vec![Lit::pos(1), Lit::pos(2), Lit::pos(0)]);
    let swap = Perm::from_images(vec![Lit::pos(1), Lit::pos(0), Lit::pos(2)]);
    // (cyc ∘ swap)(+0) = cyc(swap(+0)) = cyc(+1) = +2
    assert_eq!(
        cyc.compose(&swap).apply(Lit::pos(0)),
        Lit::pos(2),
        "other (swap) first"
    );
    // (swap ∘ cyc)(+0) = swap(cyc(+0)) = swap(+1) = +0  ≠  +2
    assert_eq!(
        swap.compose(&cyc).apply(Lit::pos(0)),
        Lit::pos(0),
        "order is not commutative"
    );
    // Sign is carried through a composition too.
    assert_eq!(cyc.compose(&swap).apply(Lit::neg(0)), Lit::neg(2));
}

#[test]
fn proofstep_clause_reads_the_added_or_deleted_clause_of_every_variant() {
    let c = vec![Lit::pos(0), Lit::neg(1)];
    assert_eq!(
        ProofStep::Rup(c.clone()).clause(),
        &c[..],
        "Rup exposes its clause"
    );
    assert_eq!(
        ProofStep::Delete(c.clone()).clause(),
        &c[..],
        "Delete exposes its clause"
    );
    let pr = ProofStep::Pr {
        clause: c.clone(),
        witness: Witness::Assignment(vec![Lit::pos(0)]),
    };
    assert_eq!(
        pr.clause(),
        &c[..],
        "Pr exposes its added clause, not the witness"
    );
}
