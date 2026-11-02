//! **Ring identities proved end-to-end through the top-level engine — the same `prove_certify_check`
//! entry `largo prove` drives.** Not the arithmetic oracle in isolation: the full backward chainer
//! selects its `try_arithmetic` strategy, the certifier re-runs the oracle into a kernel term, and
//! `infer_type` re-checks it. So a ✓ here is a genuine kernel-certified theorem reached through the
//! public proving API — "theorem-prove them, not only run them" for the ring identities the engine
//! now decides.
//!
//! These also lock that THIS is reachable: the coefficient-aware monomial sort (signed, high-degree
//! products) and the literal `0`/`1` annihilation added to the arithmetic oracle. If the engine
//! stopped routing arithmetic identities to the oracle, or the oracle regressed on these shapes, a
//! previously-green ✓ would flip.

use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn c(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn add(x: ProofTerm, y: ProofTerm) -> ProofTerm {
    ProofTerm::Function("add".to_string(), vec![x, y])
}
fn mul(x: ProofTerm, y: ProofTerm) -> ProofTerm {
    ProofTerm::Function("mul".to_string(), vec![x, y])
}
/// The primitive `sub` operator. The oracle's normalizer now unfolds it via the `sub_elim` axiom
/// (`a − b = a + (−1)·b`), so identities written with `sub` are PROVED, not just decided.
fn sub(x: ProofTerm, y: ProofTerm) -> ProofTerm {
    ProofTerm::Function("sub".to_string(), vec![x, y])
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}
/// True iff the top-level engine certifies `goal` to a kernel-checked proof term.
fn certified(goal: &ProofExpr) -> bool {
    prove_certify_check(&[], goal).verified
}

#[test]
fn commutativity_certifies_through_the_engine() {
    // a·b = b·a
    assert!(certified(&eq(mul(c("a"), c("b")), mul(c("b"), c("a")))));
}

#[test]
fn distributivity_certifies_through_the_engine() {
    // a·(b + c) = a·b + a·c
    assert!(certified(&eq(
        mul(c("a"), add(c("b"), c("c"))),
        add(mul(c("a"), c("b")), mul(c("a"), c("c"))),
    )));
}

#[test]
fn difference_of_squares_certifies_through_the_engine() {
    // (a − b)·(a + b) = a² − b² — signed monomials, the cross terms a·b and −b·a cancel. Exercises
    // the coefficient-aware sort + like-term cancellation through the engine.
    assert!(certified(&eq(
        mul(sub(c("a"), c("b")), add(c("a"), c("b"))),
        sub(mul(c("a"), c("a")), mul(c("b"), c("b"))),
    )));
}

#[test]
fn cube_of_a_difference_certifies_through_the_engine() {
    // (a − b)³ = a³ − 3a²b + 3ab² − b³ — a degree-3 SIGNED identity; the eight raw monomials only
    // collect to the four canonical terms via the coefficient-aware monomial sort.
    let a = || c("a");
    let b = || c("b");
    let lhs = mul(mul(sub(a(), b()), sub(a(), b())), sub(a(), b()));
    let a3 = mul(mul(a(), a()), a());
    let three_a2b = mul(c("3"), mul(mul(a(), a()), b()));
    let three_ab2 = mul(c("3"), mul(a(), mul(b(), b())));
    let b3 = mul(mul(b(), b()), b());
    let rhs = sub(add(sub(a3, three_a2b), three_ab2), b3);
    assert!(certified(&eq(lhs, rhs)));
}

#[test]
fn zero_annihilation_certifies_through_the_engine() {
    // a·(0·b) = 0 — the literal-0 factor collapses (left-zero annihilation).
    assert!(certified(&eq(mul(c("a"), mul(c("0"), c("b"))), c("0"))));
}

#[test]
fn one_identity_certifies_through_the_engine() {
    // a·(1·b) = a·b — the literal-1 factor drops (left-one identity).
    assert!(certified(&eq(
        mul(c("a"), mul(c("1"), c("b"))),
        mul(c("a"), c("b"))
    )));
}

#[test]
fn raw_subtraction_operator_certifies_through_the_engine() {
    // (a − b) + b = a, written with the PRIMITIVE `sub` operator — the oracle's normalizer unfolds it
    // through `sub_elim` (a − b = a + (−1)·b), so it is genuinely PROVED, not merely decided.
    assert!(certified(&eq(add(sub(c("a"), c("b")), c("b")), c("a"))));
}

#[test]
fn frontend_canonical_capitalized_names_certify_through_the_engine() {
    // The Logic frontend's `term_to_proof_term` canonicalizes arithmetic to the capitalized `Mul`/`Add`
    // (lowered to the kernel by `proof_term_to_kernel_term`). The engine's `try_arithmetic` now accepts
    // these too — so an arithmetic identity lowered from a `.lg` theorem reaches the oracle. This is the
    // EXACT ProofExpr shape `Eq(Mul(a, b), Mul(b, a))` lowers to.
    let cap_mul = |x: ProofTerm, y: ProofTerm| ProofTerm::Function("Mul".to_string(), vec![x, y]);
    let cap_add = |x: ProofTerm, y: ProofTerm| ProofTerm::Function("Add".to_string(), vec![x, y]);
    assert!(certified(&eq(
        cap_mul(c("a"), c("b")),
        cap_mul(c("b"), c("a"))
    )));
    assert!(certified(&eq(
        cap_mul(c("a"), cap_add(c("b"), c("c"))),
        cap_add(cap_mul(c("a"), c("b")), cap_mul(c("a"), c("c"))),
    )));
}

#[test]
fn a_false_ring_identity_is_not_certified() {
    // a·b ≠ a + b for general a, b — the engine must NOT fabricate a proof (fail-closed).
    assert!(!certified(&eq(mul(c("a"), c("b")), add(c("a"), c("b")))));
}
