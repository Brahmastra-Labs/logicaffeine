//! Compact-natural conversion must not recurse once per constructor.
use logicaffeine_kernel::{
    defeq_for_test, prelude::StandardLibrary, BigInt, Context, Literal, Term,
};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn nat(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn succ(t: Term) -> Term {
    Term::App(Box::new(g("Succ")), Box::new(t))
}
fn spine(count: usize, tail: Term) -> Term {
    (0..count).fold(tail, |t, _| succ(t))
}
#[test]
fn compact_naturals_compare_against_long_and_mixed_peano_spines() {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    for (count, tail, value) in [(512, g("Zero"), 512), (256, nat(17), 273)] {
        let t = spine(count, tail);
        assert!(defeq_for_test(&c, &t, &nat(value)));
        assert!(defeq_for_test(&c, &nat(value), &t));
        assert!(!defeq_for_test(&c, &t, &nat(value - 1)));
        assert!(!defeq_for_test(&c, &nat(value + 1), &t));
    }
}
#[test]
fn peano_conversion_preserves_open_tails_and_negative_boundaries() {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    let c = c.extend("n", g("Nat"));
    assert!(!defeq_for_test(
        &c,
        &spine(3, Term::Var("n".into())),
        &nat(3)
    ));
    assert!(!defeq_for_test(&c, &nat(-1), &g("Zero")));
    assert!(!defeq_for_test(&c, &g("Zero"), &nat(-1)));
    assert!(!defeq_for_test(&c, &nat(0), &succ(nat(0))));
    assert!(defeq_for_test(&c, &nat(0), &g("Zero")));
}
