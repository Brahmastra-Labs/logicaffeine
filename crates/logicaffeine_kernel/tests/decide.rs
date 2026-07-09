//! `Decidable` / `decide` — proof by decision procedure (Lean's `decide`), all DERIVED.
//!
//! `Decidable (p:Prop) : Type` carries a decision (`isTrue h` / `isFalse h`); `decide` reads
//! it off as a `Bool`; and `of_decide_eq_true` — PROVEN from the recursor + Bool
//! no-confusion, not axiomatized — turns `decide p inst = true` into a proof of `p`. So a
//! goal `p` is discharged by `of_decide_eq_true p inst (refl Bool true)`, which type-checks
//! exactly when the decision procedure computes to `isTrue` (fail-closed otherwise).

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    derive_recursor, double_check, infer_type, is_subtype, normalize, Context, DoubleCheck, Term,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// `Not False` is inhabited by the identity `λx:False. x`.
fn not_false() -> Term {
    lam("x", g("False"), Term::Var("x".to_string()))
}
fn refl(ty: Term, x: Term) -> Term {
    app(app(g("refl"), ty), x)
}

#[test]
fn decidable_layer_is_registered_and_well_typed() {
    let ctx = std_ctx();
    for name in ["Decidable", "isTrue", "isFalse", "Decidable_rec", "decide", "of_decide_eq_true"] {
        assert!(
            infer_type(&ctx, &g(name)).is_ok(),
            "{name} must be registered and well-typed: {:?}",
            infer_type(&ctx, &g(name))
        );
    }
}

#[test]
fn decide_computes_true_and_false() {
    let ctx = std_ctx();
    // decide True (isTrue True I)  ↝  true
    let dt = app(app(g("decide"), g("True")), app(app(g("isTrue"), g("True")), g("I")));
    assert!(infer_type(&ctx, &dt).is_ok(), "decide True (isTrue …) must type-check");
    assert_eq!(normalize(&ctx, &dt), g("true"), "decide of isTrue must compute to true");

    // decide False (isFalse False (λx:False. x))  ↝  false
    let df = app(
        app(g("decide"), g("False")),
        app(app(g("isFalse"), g("False")), not_false()),
    );
    assert!(infer_type(&ctx, &df).is_ok(), "decide False (isFalse …) must type-check");
    assert_eq!(normalize(&ctx, &df), g("false"), "decide of isFalse must compute to false");
}

#[test]
fn decidable_recursor_and_of_decide_are_derived_two_kernel_verified() {
    let ctx = std_ctx();
    // Decidable_rec derives cleanly and is verified by both kernels.
    let (_ty, rec) = derive_recursor(&ctx, "Decidable").expect("Decidable.rec derives");
    assert_eq!(double_check(&ctx, &rec), DoubleCheck::Agreed, "Decidable_rec two-kernel");

    // of_decide_eq_true is a DEFINITION (not a trusted axiom), two-kernel verified, and its
    // body inhabits its declared type.
    assert!(ctx.is_definition("of_decide_eq_true"), "of_decide_eq_true must be derived, not an axiom");
    let body = ctx.get_definition_body("of_decide_eq_true").expect("has a body").clone();
    assert_eq!(double_check(&ctx, &body), DoubleCheck::Agreed, "of_decide_eq_true two-kernel");
    let decl = ctx.get_definition_type("of_decide_eq_true").expect("has a type").clone();
    let got = infer_type(&ctx, &body).expect("of_decide_eq_true body well-typed");
    assert!(is_subtype(&ctx, &got, &decl), "of_decide_eq_true body must inhabit its declared type");
}

#[test]
fn of_decide_proves_a_true_decidable_proposition() {
    // `of_decide_eq_true True (isTrue True I) (refl Bool true)` is a proof of `True`.
    let ctx = std_ctx();
    let inst = app(app(g("isTrue"), g("True")), g("I"));
    let proof = app(app(app(g("of_decide_eq_true"), g("True")), inst), refl(g("Bool"), g("true")));
    let ty = infer_type(&ctx, &proof).expect("decide proof must type-check");
    assert_eq!(normalize(&ctx, &ty), g("True"), "the decide proof must have type True");
}

/// `decEqBool a b`
fn dec_eq_bool(a: Term, b: Term) -> Term {
    app(app(g("decEqBool"), a), b)
}

#[test]
fn dec_eq_bool_is_derived_and_two_kernel_verified() {
    let ctx = std_ctx();
    assert!(infer_type(&ctx, &g("decEqBool")).is_ok(), "decEqBool must be well-typed");
    assert!(ctx.is_definition("decEqBool"), "decEqBool is a derived definition");
    let body = ctx.get_definition_body("decEqBool").expect("has a body").clone();
    assert_eq!(double_check(&ctx, &body), DoubleCheck::Agreed, "decEqBool two-kernel verified");
    let decl = ctx.get_definition_type("decEqBool").expect("has a type").clone();
    let got = infer_type(&ctx, &body).expect("decEqBool body well-typed");
    assert!(is_subtype(&ctx, &got, &decl), "decEqBool body inhabits its declared type");
}

#[test]
fn dec_eq_bool_computes_the_verdict() {
    let ctx = std_ctx();
    // decide (Eq Bool true true) (decEqBool true true) ↝ true
    let same = app(
        app(g("decide"), app(app(app(g("Eq"), g("Bool")), g("true")), g("true"))),
        dec_eq_bool(g("true"), g("true")),
    );
    assert_eq!(normalize(&ctx, &same), g("true"), "true = true decides true");
    // decide (Eq Bool true false) (decEqBool true false) ↝ false
    let diff = app(
        app(g("decide"), app(app(app(g("Eq"), g("Bool")), g("true")), g("false"))),
        dec_eq_bool(g("true"), g("false")),
    );
    assert_eq!(normalize(&ctx, &diff), g("false"), "true = false decides false");
}

#[test]
fn decide_proves_a_true_bool_equality() {
    // `decide` discharges the real goal `Eq Bool true true` via its decision procedure.
    let ctx = std_ctx();
    let prop = app(app(app(g("Eq"), g("Bool")), g("true")), g("true"));
    let proof = app(
        app(app(g("of_decide_eq_true"), prop.clone()), dec_eq_bool(g("true"), g("true"))),
        refl(g("Bool"), g("true")),
    );
    let ty = infer_type(&ctx, &proof).expect("decide must prove true = true");
    assert!(is_subtype(&ctx, &ty, &prop), "the proof has type Eq Bool true true");
}

#[test]
fn decide_is_fail_closed_on_a_false_bool_equality() {
    // `decide (Eq Bool true false)` computes `false`, so no proof can be built — the refl
    // witness cannot check against `Eq Bool false true`.
    let ctx = std_ctx();
    let prop = app(app(app(g("Eq"), g("Bool")), g("true")), g("false"));
    let bogus = app(
        app(app(g("of_decide_eq_true"), prop), dec_eq_bool(g("true"), g("false"))),
        refl(g("Bool"), g("true")),
    );
    assert!(infer_type(&ctx, &bogus).is_err(), "decide must NOT prove true = false");
}

/// The `Nat` numeral `Succ^n Zero`.
fn nat_lit(n: usize) -> Term {
    let mut t = g("Zero");
    for _ in 0..n {
        t = app(g("Succ"), t);
    }
    t
}
/// `Eq Nat a b`
fn eqn(a: Term, b: Term) -> Term {
    app(app(app(g("Eq"), g("Nat")), a), b)
}
/// `decEqNat a b`
fn dec_eq_nat(a: Term, b: Term) -> Term {
    app(app(g("decEqNat"), a), b)
}

#[test]
fn dec_eq_nat_is_derived_and_two_kernel_verified() {
    // The recursive decision procedure — a `fix` with nested matches and a match on its own
    // recursive result — must derive, inhabit its type, and be verified by BOTH kernels.
    let ctx = std_ctx();
    for name in ["decEqNat", "nat_zs_ne", "nat_sz_ne", "succ_cong", "succ_inj"] {
        assert!(infer_type(&ctx, &g(name)).is_ok(), "{name} must be well-typed: {:?}", infer_type(&ctx, &g(name)));
        assert!(ctx.is_definition(name), "{name} must be a derived definition (no axiom)");
        let body = ctx.get_definition_body(name).unwrap_or_else(|| panic!("{name} body")).clone();
        assert_eq!(double_check(&ctx, &body), DoubleCheck::Agreed, "{name} two-kernel verified");
        let decl = ctx.get_definition_type(name).unwrap_or_else(|| panic!("{name} type")).clone();
        let got = infer_type(&ctx, &body).unwrap_or_else(|_| panic!("{name} body type"));
        assert!(is_subtype(&ctx, &got, &decl), "{name} body inhabits its declared type");
    }
}

#[test]
fn dec_eq_nat_computes_the_verdict() {
    let ctx = std_ctx();
    // decide (Eq Nat 2 2) (decEqNat 2 2) ↝ true
    let same = app(app(g("decide"), eqn(nat_lit(2), nat_lit(2))), dec_eq_nat(nat_lit(2), nat_lit(2)));
    assert_eq!(normalize(&ctx, &same), g("true"), "2 = 2 decides true");
    // decide (Eq Nat 2 3) (decEqNat 2 3) ↝ false
    let diff = app(app(g("decide"), eqn(nat_lit(2), nat_lit(3))), dec_eq_nat(nat_lit(2), nat_lit(3)));
    assert_eq!(normalize(&ctx, &diff), g("false"), "2 = 3 decides false");
}

#[test]
fn decide_proves_arithmetic_equality() {
    // The flagship: `decide` discharges the real goal `2 = 2` (as `Nat`s).
    let ctx = std_ctx();
    let prop = eqn(nat_lit(2), nat_lit(2));
    let proof = app(
        app(app(g("of_decide_eq_true"), prop.clone()), dec_eq_nat(nat_lit(2), nat_lit(2))),
        refl(g("Bool"), g("true")),
    );
    let ty = infer_type(&ctx, &proof).expect("decide must prove 2 = 2");
    assert!(is_subtype(&ctx, &ty, &prop), "the proof has type Eq Nat 2 2");
}

#[test]
fn decide_is_fail_closed_on_false_arithmetic() {
    // `decide (Eq Nat 2 3)` computes `false` — no proof can be built.
    let ctx = std_ctx();
    let bogus = app(
        app(app(g("of_decide_eq_true"), eqn(nat_lit(2), nat_lit(3))), dec_eq_nat(nat_lit(2), nat_lit(3))),
        refl(g("Bool"), g("true")),
    );
    assert!(infer_type(&ctx, &bogus).is_err(), "decide must NOT prove 2 = 3");
}

#[test]
fn decide_is_fail_closed_on_a_false_proposition() {
    // A FALSE proposition's `decide` computes to `false`, so `of_decide_eq_true` cannot be
    // completed with `refl Bool true` — `Eq Bool false true` is not `Eq Bool true true`.
    // This is what makes `decide` sound: it can only ever prove things that compute true.
    let ctx = std_ctx();
    let inst = app(app(g("isFalse"), g("False")), not_false());
    let bogus = app(app(app(g("of_decide_eq_true"), g("False")), inst), refl(g("Bool"), g("true")));
    assert!(
        infer_type(&ctx, &bogus).is_err(),
        "decide must NOT be able to prove False: the refl witness cannot type-check against \
         Eq Bool false true"
    );
}
