use super::*;

#[test]
fn every_abelian_op_is_a_certified_bijection() {
    // ★ The semantic bijectivity certificate for the zero-garbage core ★ — every `Rev*` op's
    // declared inverse genuinely inverts it, proved over the full BV64 domain (no sampling).
    for spec in all_rev_specs() {
        prove_bijective(&spec).unwrap_or_else(|e| panic!("bijectivity proof failed: {e}"));
    }
    prove_swap_involution().expect("RevSwap must be a certified involution");
}

#[test]
fn a_non_inverting_inverse_is_rejected() {
    // Canary: `RevAdd` with a WRONG inverse (add-again, not sub) computes `x + 2d`, which differs
    // from `x` whenever `d ≠ 0`. The harness MUST reject it — proving it can fail, not rubber-stamp.
    let canary = RevSpec {
        name: "RevAdd-BROKEN",
        forward: add,
        inverse: add,
    };
    assert!(
        prove_bijective(&canary).is_err(),
        "the harness must reject a non-inverting inverse — else it proves nothing"
    );
}
