//! T10-017's defensive guard: a `match` computes only on constructors of ITS OWN inductive.
//!
//! `sub : Int → Int → Int` unfolds to a `match` on `Int`. Applied (ill-typed) to `Nat` literals,
//! the `Nat` literal `3` used to be read as `Int`'s second constructor — both inductives have two —
//! and `sub 3 8` normalized to `Int_ofNat 5`. Both kernels reject the term, so this was never a
//! soundness hole; it is wrong progress, and Lean's recursor (`getRecRuleFor`) makes none.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, recheck, BigInt, Context, Literal, Term};

fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn nat(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn sub(a: Term, b: Term) -> Term {
    app(app(g("sub"), a), b)
}
fn std_ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}

#[test]
fn a_nat_literal_does_not_select_a_branch_of_an_int_match() {
    let ctx = std_ctx();
    let bad = sub(nat(3), nat(8));
    assert!(
        infer_type(&ctx, &bad).is_err(),
        "ill-typed in the main kernel"
    );
    assert!(
        recheck(&ctx, &bad).is_err(),
        "ill-typed in the independent kernel"
    );
    let normal = normalize(&ctx, &bad);
    assert_ne!(normal, app(g("Int_ofNat"), nat(5)), "no wrong computation");
    let mut head = &normal;
    while let Term::App(f, _) = head {
        head = f;
    }
    assert!(
        !matches!(head, Term::Global(n) if n == "Int_ofNat" || n == "Int_negSucc"),
        "the ill-typed match is stuck, not an Int value: {normal}"
    );
}

#[test]
fn well_typed_int_subtraction_still_computes() {
    let ctx = std_ctx();
    for (a, b, want) in [(3, 8, -5), (8, 3, 5), (0, 0, 0), (-4, 6, -10)] {
        let t = sub(int(a), int(b));
        infer_type(&ctx, &t).expect("well-typed");
        assert_eq!(normalize(&ctx, &t), int(want), "sub {a} {b}");
    }
    let of_nat = |n| app(g("Int_ofNat"), nat(n));
    let t = sub(of_nat(3), of_nat(8));
    infer_type(&ctx, &t).expect("well-typed");
    assert_eq!(normalize(&ctx, &t), int(-5));
}
