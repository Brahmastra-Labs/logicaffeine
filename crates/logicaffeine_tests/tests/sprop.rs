//! SProp (S) — a definitionally-proof-irrelevant sort, distinct from `Prop`, at the bottom
//! of the universe hierarchy (`SProp ≤ Prop ≤ Type n`) and impredicative (`Π` into `SProp`
//! is `SProp`). Any two terms of an `SProp`-typed type are DEFINITIONALLY equal — in both
//! kernels.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, Context, DoubleCheck, MutualInductive, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn sprop() -> Term {
    Term::Sort(Universe::SProp)
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn sprop_is_a_sort_at_type_0() {
    let ctx = std_ctx();
    // `SProp : Type 0` — `to_lnat` puts `SProp` at level 0 like `Prop`, so its successor is
    // level 1, spelled `Type 0`. The old `Type 1` was the same off-by-one `Prop` carried.
    assert_eq!(
        infer_type(&ctx, &sprop()).unwrap(),
        Term::Sort(Universe::Type(0)),
        "SProp : Type 0"
    );
}

#[test]
fn sprop_is_below_prop_below_type() {
    // The hierarchy: `SProp ≤ Prop ≤ Type 0`, and NOT the reverse.
    assert!(
        Universe::SProp.is_subtype_of(&Universe::Prop),
        "SProp ≤ Prop"
    );
    assert!(
        Universe::SProp.is_subtype_of(&Universe::Type(0)),
        "SProp ≤ Type 0"
    );
    assert!(
        Universe::Prop.is_subtype_of(&Universe::Type(0)),
        "Prop ≤ Type 0"
    );
    assert!(
        !Universe::Prop.is_subtype_of(&Universe::SProp),
        "¬(Prop ≤ SProp)"
    );
    assert!(
        !Universe::Type(0).is_subtype_of(&Universe::SProp),
        "¬(Type 0 ≤ SProp)"
    );
    assert!(
        Universe::SProp.is_subtype_of(&Universe::SProp),
        "SProp ≤ SProp"
    );
}

#[test]
fn pi_into_sprop_is_impredicatively_sprop() {
    let mut ctx = std_ctx();
    ctx.add_declaration("P", sprop()); // P : SProp
                                       // `Π(x:Nat). P` (a Π into SProp) is itself in SProp.
    let pi_ty = pi("x", g("Nat"), g("P"));
    assert_eq!(
        infer_type(&ctx, &pi_ty).unwrap(),
        sprop(),
        "Π(x:Nat). P : SProp (impredicative)"
    );
}

#[test]
fn any_two_terms_of_an_sprop_type_are_definitionally_equal() {
    // `P : SProp`, `a b : P`. Definitional irrelevance: `a ≡ b`.
    let mut ctx = std_ctx();
    ctx.add_declaration("P", sprop());
    ctx.add_declaration("a", g("P"));
    ctx.add_declaration("b", g("P"));
    assert!(
        defeq_for_test(&ctx, &g("a"), &g("b")),
        "a ≡ b by SProp definitional irrelevance"
    );
}

#[test]
fn sprop_irrelevance_is_two_kernel() {
    // A term that type-checks ONLY via SProp irrelevance: `Vec : P → Type`, `v : Vec b`, and
    // `(λx : Vec a. x) v` — well-typed only because `a ≡ b` makes `Vec a ≡ Vec b`. BOTH
    // kernels must accept it (the re-checker has its own irrelevance rule).
    let mut ctx = std_ctx();
    ctx.add_declaration("P", sprop());
    ctx.add_declaration("a", g("P"));
    ctx.add_declaration("b", g("P"));
    ctx.add_declaration("Vec", arrow(g("P"), ty0()));
    ctx.add_declaration("v", app(g("Vec"), g("b")));
    let coerce = app(lam("x", app(g("Vec"), g("a")), v("x")), g("v"));

    assert_eq!(
        infer_type(&ctx, &coerce).unwrap(),
        app(g("Vec"), g("a")),
        "coercion type-checks"
    );
    match double_check(&ctx, &coerce) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must agree via SProp irrelevance, got {other:?}"),
    }
}

fn family(
    name: &str,
    sort: Term,
    num_params: usize,
    ctor: &str,
    ctor_ty: Term,
) -> [MutualInductive; 1] {
    [MutualInductive {
        name: name.to_string(),
        sort,
        num_params,
        constructors: vec![(ctor.to_string(), ctor_ty)],
    }]
}

#[test]
fn an_sprop_family_stores_arguments_of_any_sort() {
    // `SProp` is impredicative, so an `SProp` family stores an argument of any sort, as Rocq's
    // `Squash` does: what it stores never comes back out, since it eliminates into `SProp` alone.
    let mut ctx = std_ctx();
    ctx.add_mutual_inductives(&family("SBig", sprop(), 0, "sbig", arrow(ty0(), g("SBig"))))
        .expect("an SProp family stores a type, which lives in Type 1");
    let u = || Term::Sort(Universe::Var("u".to_string()));
    let squash = Term::Const {
        name: "SSquash".to_string(),
        levels: vec![Universe::Var("u".to_string())],
    };
    ctx.add_universe_poly_inductives(
        &["u".to_string()],
        &family(
            "SSquash",
            pi("A", u(), sprop()),
            1,
            "ssquash",
            pi("A", u(), arrow(v("A"), app(squash, v("A")))),
        ),
    )
    .expect("an SProp squashes a type of any universe");
    assert_eq!(
        infer_type(&ctx, &app(g("sbig"), g("Nat"))).unwrap(),
        g("SBig"),
        "`sbig Nat : SBig`"
    );

    // The predicative sorts keep the constraint: a `Type 0` family storing a `Type 0` or a `Type 1`
    // is refused beside them.
    for (name, sort, field) in [
        ("TBig", ty0(), ty0()),
        ("UBig", ty0(), Term::Sort(Universe::Type(1))),
    ] {
        let refused =
            ctx.add_mutual_inductives(&family(name, sort, 0, "mk", arrow(field, g(name))));
        assert!(
            matches!(&refused, Err(e) if e.to_string().contains("universe inconsistency")),
            "`{name}` stores a sort above its own and must be refused by the universe constraint: {refused:?}"
        );
        assert!(
            ctx.get_global(name).is_none(),
            "`{name}` must never reach the environment"
        );
    }
}
