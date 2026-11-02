use super::*;
use logicaffeine_kernel::{infer_type, prelude::StandardLibrary};

fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.add_declaration("x", int());
    c.add_declaration("y", int());
    c
}

/// The oracle must find a proof AND the kernel must accept it as `Eq Int lhs rhs`.
fn assert_certifies(ctx: &Context, lhs: &Term, rhs: &Term) {
    let proof = prove_int_eq(ctx, lhs, rhs)
        .unwrap_or_else(|| panic!("oracle found no proof for {lhs:?} = {rhs:?}"));
    let ty = infer_type(ctx, &proof)
        .unwrap_or_else(|e| panic!("kernel rejected the proof for {lhs:?} = {rhs:?}: {e:?}"));
    let want = eq_int_term(lhs.clone(), rhs.clone());
    assert!(
        conv(ctx, &ty, &want),
        "proof types as {ty:?}, wanted Eq Int {lhs:?} {rhs:?}"
    );
}

fn add_t(a: Term, b: Term) -> Term {
    ax2("add", a, b)
}
fn mul_t(a: Term, b: Term) -> Term {
    ax2("mul", a, b)
}

#[test]
fn coefficients_summing_to_one_recombine() {
    // The merge gap: like monomials whose coefficients sum to exactly 1 must
    // recombine to the bare monomial (2x + (-1)x = x), not fail the proof.
    let ctx = ctx();
    let x = global("x");
    assert_certifies(
        &ctx,
        &add_t(mul_t(lit_t(2), x.clone()), mul_t(lit_t(-1), x.clone())),
        &x,
    );
    assert_certifies(
        &ctx,
        &add_t(mul_t(lit_t(-1), x.clone()), mul_t(lit_t(2), x.clone())),
        &x,
    );
    assert_certifies(
        &ctx,
        &add_t(mul_t(lit_t(3), x.clone()), mul_t(lit_t(-2), x.clone())),
        &x,
    );
}

#[test]
fn farkas_shape_big_l_certifies() {
    // cert_farkas's summed left side: Σ λᵢ·0 must certify equal to 0.
    let ctx = ctx();
    let big_l = add_t(mul_t(lit_t(1), lit_t(0)), mul_t(lit_t(1), lit_t(0)));
    assert_certifies(&ctx, &big_l, &lit_t(0));
}

#[test]
fn farkas_shape_double_constant_big_r_certifies() {
    // The exact BigR cert_farkas builds for the double-constant system
    // x+1 ≤ y ∧ y+1 ≤ x+1 with multipliers λ = (1, 1):
    //   1·(y − (x+1)) + 1·((x+1) − (y+1))  =  −1
    // encoded sub-free as add(r, mul(−1, l)) per hypothesis.
    let ctx = ctx();
    let x = global("x");
    let y = global("y");
    let l1 = add_t(x.clone(), lit_t(1));
    let r1 = y.clone();
    let l2 = add_t(y.clone(), lit_t(1));
    let r2 = add_t(x.clone(), lit_t(1));
    let diff1 = add_t(r1, mul_t(lit_t(-1), l1));
    let diff2 = add_t(r2, mul_t(lit_t(-1), l2));
    let big_r = add_t(mul_t(lit_t(1), diff1), mul_t(lit_t(1), diff2));
    assert_certifies(&ctx, &big_r, &lit_t(-1));
}

/// High-degree PURE-MULTIPLICATIVE rearrangements — beyond the old bounded search, now proven by
/// the constructive monomial sort (comm/assoc swaps). This is what unlocks det-multiplicativity.
#[test]
fn high_degree_monomial_reorder_certifies() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for v in ["p", "q", "r", "s", "t"] {
        ctx.add_declaration(v, int());
    }
    let (p, q, r, s, t) = (
        global("p"),
        global("q"),
        global("r"),
        global("s"),
        global("t"),
    );
    // degree-4: (p·q)·(r·s) = (p·r)·(q·s)  — the exact det-multiplicativity step shape
    assert_certifies(
        &ctx,
        &mul_t(mul_t(p.clone(), q.clone()), mul_t(r.clone(), s.clone())),
        &mul_t(mul_t(p.clone(), r.clone()), mul_t(q.clone(), s.clone())),
    );
    // degree-5, fully re-nested and reordered
    assert_certifies(
        &ctx,
        &mul_t(
            mul_t(mul_t(t.clone(), s.clone()), r.clone()),
            mul_t(q.clone(), p.clone()),
        ),
        &mul_t(
            p.clone(),
            mul_t(q.clone(), mul_t(r.clone(), mul_t(s.clone(), t.clone()))),
        ),
    );
}
