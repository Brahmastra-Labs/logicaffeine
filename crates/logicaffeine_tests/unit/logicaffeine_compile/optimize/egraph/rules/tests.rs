use super::*;

#[test]
fn mul_pow2_fits_i64_is_the_exact_overflow_boundary() {
    // 2^3 = 8: bounded multiplicands fit; unbounded / unproven do not.
    assert!(mul_pow2_fits_i64(Some((0, 255)), 3));
    assert!(mul_pow2_fits_i64(Some((-100, 100)), 3));
    assert!(!mul_pow2_fits_i64(None, 3));
    assert!(!mul_pow2_fits_i64(Some((0, i64::MAX)), 3));
    assert!(!mul_pow2_fits_i64(Some((i64::MIN, 0)), 1));
    // The exact rim: ⌊i64::MAX / 8⌋ * 8 fits, one more overflows.
    assert!(mul_pow2_fits_i64(Some((0, i64::MAX / 8)), 3));
    assert!(!mul_pow2_fits_i64(Some((0, i64::MAX / 8 + 1)), 3));
    // n ≥ 63 can never be safe (2^63 escapes i64).
    assert!(!mul_pow2_fits_i64(Some((1, 1)), 63));
}

fn mul_by_eight(lo: i64, hi: i64) -> bool {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    eg.set_int_range(x, lo, hi);
    let eight = eg.add(CompilerENode::Int(8));
    let mul = eg.add(CompilerENode::Mul(x, eight));
    matches!(
        r_mul_pow2_shl(&mut eg, mul),
        Some(n) if matches!(eg.canonical_node(n), CompilerENode::Shl(..))
    )
}

#[test]
fn mul_pow2_shl_fires_only_when_product_proven_to_fit() {
    // Proven-bounded x ∈ [0, 1000]: 1000 * 8 = 8000 fits → strength-reduce.
    assert!(
        mul_by_eight(0, 1000),
        "proven-bounded x*8 must become a shift"
    );
    // Unbounded x: x*8 may overflow — exact `*` promotes, `<<` would WRAP, so
    // the rewrite must be refused (the backend keeps the checked native imul).
    assert!(
        !mul_by_eight(1, i64::MAX),
        "unbounded x*8 must NOT become a wrapping shift under exact arithmetic"
    );
}

fn mk_int_var(eg: &mut CompilerEGraph, i: u32) -> NodeId {
    let v = eg.add(CompilerENode::Var(i, 0));
    eg.set_scalar(v, ScalarKind::Int);
    eg.set_int_range(v, 0, 1000);
    v
}

#[test]
fn gauss_butterfly_offers_the_three_multiply_form() {
    // a*b + c*d  must gain the Gauss alternative (a+c)(b+d) − a*d − c*b (a Sub at the root),
    // the form that costs 3 multiplies instead of 4 when the cross products are shared.
    let mut eg = CompilerEGraph::new();
    let a = mk_int_var(&mut eg, 0);
    let b = mk_int_var(&mut eg, 1);
    let c = mk_int_var(&mut eg, 2);
    let d = mk_int_var(&mut eg, 3);
    let ab = eg.add(CompilerENode::Mul(a, b));
    let cd = eg.add(CompilerENode::Mul(c, d));
    let sum = eg.add(CompilerENode::Add(ab, cd));
    let rewritten = r_gauss_butterfly(&mut eg, sum);
    assert!(
        matches!(rewritten, Some(n) if matches!(eg.canonical_node(n), CompilerENode::Sub(..))),
        "the Gauss butterfly rewrite must fire and offer the (a+c)(b+d) − a*d − c*b form"
    );
}

#[test]
fn every_rewrite_rule_including_gauss_is_kernel_certified() {
    // The soundness gate ("the symmetry won't break the algorithm"): EVERY rule's
    // certificate — including gauss-butterfly's `Ring` identity — is discharged by the
    // kernel, so a rewrite can never change the value of the node it fires on.
    let n = verify_all_with_kernel().expect("all rewrite rules must be kernel-certified");
    assert!(n >= 1, "at least one certified rule");
}

/// Count the distinct multiply e-classes reachable from `roots` — the multiply op-count of
/// the shared DAG (e-graph hash-consing makes equal products one class, so a product used
/// by two outputs is counted once).
fn count_mul_classes(eg: &mut CompilerEGraph, roots: &[NodeId]) -> usize {
    let mut seen = std::collections::HashSet::new();
    let mut muls = std::collections::HashSet::new();
    let mut stack: Vec<NodeId> = roots.to_vec();
    while let Some(id) = stack.pop() {
        let cid = eg.find(id);
        if !seen.insert(cid) {
            continue;
        }
        let node = eg.canonical_node(cid);
        if matches!(node, CompilerENode::Mul(..)) {
            muls.insert(cid);
        }
        for child in node.children() {
            stack.push(child);
        }
    }
    muls.len()
}

#[test]
fn gauss_cuts_ntt_base_multiply_from_five_to_four_multiplies() {
    // ML-KEM's NTT base case multiplies two degree-1 polynomials mod (X² − ζ):
    //   result0 = a0·b0 + ζ·(a1·b1)        result1 = a0·b1 + a1·b0
    // The cross products a0·b0 and a1·b1 are SHARED with result0, so Gauss turns the pair
    // from 5 multiplies to 4 — a measured 20% cut, certified sound by the kernel.
    let mut eg = CompilerEGraph::new();
    let a0 = mk_int_var(&mut eg, 0);
    let a1 = mk_int_var(&mut eg, 1);
    let b0 = mk_int_var(&mut eg, 2);
    let b1 = mk_int_var(&mut eg, 3);
    let zeta = mk_int_var(&mut eg, 4);

    let a0b0 = eg.add(CompilerENode::Mul(a0, b0));
    let a1b1 = eg.add(CompilerENode::Mul(a1, b1));
    let zeta_a1b1 = eg.add(CompilerENode::Mul(zeta, a1b1));
    let result0 = eg.add(CompilerENode::Add(a0b0, zeta_a1b1));

    // Naive cross term: a0·b1 + a1·b0 (two fresh multiplies).
    let a0b1 = eg.add(CompilerENode::Mul(a0, b1));
    let a1b0 = eg.add(CompilerENode::Mul(a1, b0));
    let result1_naive = eg.add(CompilerENode::Add(a0b1, a1b0));

    let naive = count_mul_classes(&mut eg, &[result0, result1_naive]);
    assert_eq!(naive, 5, "the naive NTT base multiply uses 5 multiplies");

    // Gauss the cross term → (a0+a1)(b1+b0) − a0·b0 − a1·b1, reusing a0·b0 and a1·b1.
    let result1_gauss = r_gauss_butterfly(&mut eg, result1_naive).expect("gauss must fire");
    let gauss = count_mul_classes(&mut eg, &[result0, result1_gauss]);
    assert_eq!(gauss, 4, "Gauss shares a0·b0 and a1·b1 → 4 multiplies");
    assert!(
        gauss < naive,
        "the certified symmetry strictly cuts the NTT base-multiply op-count"
    );
}
