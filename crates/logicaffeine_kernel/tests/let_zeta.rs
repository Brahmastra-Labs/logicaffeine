//! `Term::Let` and zeta reduction — local definitions in the kernel, checked
//! in BOTH kernels (main type-checker and the de Bruijn re-checker).

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, is_subtype, normalize, Context, DoubleCheck, Literal, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn nat() -> Term {
    g("Nat")
}
fn let_(name: &str, ty: Term, value: Term, body: Term) -> Term {
    Term::Let {
        name: name.to_string(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(body),
    }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn let_typechecks() {
    // let x : Nat := Zero in Succ x  :  Nat
    let ctx = std_ctx();
    let t = let_("x", nat(), g("Zero"), app(g("Succ"), Term::Var("x".to_string())));
    let ty = infer_type(&ctx, &t).expect("let should type-check");
    assert!(is_subtype(&ctx, &ty, &nat()), "let expression has type Nat, got {ty}");
}

#[test]
fn let_value_checked_against_annotation() {
    // let x : Nat := true in x  — the value `true : Bool` fails the `: Nat`.
    let ctx = std_ctx();
    let t = let_("x", nat(), g("true"), Term::Var("x".to_string()));
    assert!(infer_type(&ctx, &t).is_err(), "ill-typed let value must be rejected");
}

#[test]
fn let_zeta_reduces() {
    // normalize(let x := Zero in Succ x) == Succ Zero
    let ctx = std_ctx();
    let t = let_("x", nat(), g("Zero"), app(g("Succ"), Term::Var("x".to_string())));
    let nf = normalize(&ctx, &t);
    assert_eq!(nf, app(g("Succ"), g("Zero")), "zeta: the body with the value substituted");
}

#[test]
fn let_transparent_in_body_type() {
    // let n : Nat := Zero in refl Nat n  :  Eq Nat n Zero — the typing must see
    // the definition (n ≡ Zero), not treat n as opaque.
    let ctx = std_ctx();
    let refl_n = app(app(g("refl"), nat()), Term::Var("n".to_string()));
    let t = let_("n", nat(), g("Zero"), refl_n);
    let ty = infer_type(&ctx, &t).expect("let-transparent refl should check");
    // The result is `Eq Nat Zero Zero` after zeta — assignable to that.
    let expected = app(app(app(g("Eq"), nat()), g("Zero")), g("Zero"));
    assert!(is_subtype(&ctx, &ty, &expected), "let body typed with n ≡ Zero, got {ty}");
}

#[test]
fn let_capture_avoidance() {
    // let x := y in (λ(y:Nat). Succ x) — zeta must substitute the FREE `y` into
    // the body WITHOUT the inner `λy` capturing it. The reduct must therefore
    // rename the inner binder: `λ(y':Nat). Succ y`, where the `y` in the body is
    // the free value, not the bound one.
    let ctx = std_ctx();
    let inner = Term::Lambda {
        param: "y".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(app(g("Succ"), Term::Var("x".to_string()))),
    };
    let t = let_("x", nat(), Term::Var("y".to_string()), inner);
    let nf = normalize(&ctx, &t);
    // The reduct is a lambda whose binder is renamed away from `y`, and whose
    // body applies `Succ` to the FREE `y` — capture was avoided.
    match nf {
        Term::Lambda { param, body, .. } => {
            assert_ne!(param, "y", "the inner binder must be renamed to avoid capture");
            assert_eq!(*body, app(g("Succ"), Term::Var("y".to_string())), "body mentions the free y");
        }
        other => panic!("expected a lambda after zeta, got {other}"),
    }
}

#[test]
fn let_nested_zeta() {
    // let a := Zero in let b := Succ a in Succ b  ⟶  Succ (Succ Zero)
    let ctx = std_ctx();
    let inner = let_(
        "b",
        nat(),
        app(g("Succ"), Term::Var("a".to_string())),
        app(g("Succ"), Term::Var("b".to_string())),
    );
    let t = let_("a", nat(), g("Zero"), inner);
    let nf = normalize(&ctx, &t);
    assert_eq!(nf, app(g("Succ"), app(g("Succ"), g("Zero"))));
}

#[test]
fn let_int_literal_value() {
    // let n : Int := 5 in n  :  Int, normalizing to 5.
    let ctx = std_ctx();
    let t = let_("n", g("Int"), Term::Lit(Literal::Int(5)), Term::Var("n".to_string()));
    let ty = infer_type(&ctx, &t).expect("let over Int literal checks");
    assert!(is_subtype(&ctx, &ty, &g("Int")));
    assert_eq!(normalize(&ctx, &t), Term::Lit(Literal::Int(5)));
}

#[test]
fn recheck_agrees_on_let() {
    // The independent de Bruijn re-checker must accept a Let-bearing term and
    // AGREE with the main kernel — Let is real two-kernel machinery, not a
    // fail-safe `Unsupported` skip.
    let ctx = std_ctx();
    let t = let_("x", nat(), g("Zero"), app(g("Succ"), Term::Var("x".to_string())));
    match double_check(&ctx, &t) {
        DoubleCheck::Agreed => {
            // Both kernels accepted and agreed on the type. Confirm the main
            // kernel's inferred type is Nat.
            let ty = infer_type(&ctx, &t).expect("main kernel checks the Let");
            assert!(is_subtype(&ctx, &ty, &nat()), "both kernels: type Nat");
        }
        other => panic!("both kernels must AGREE on a Let term, got {other:?}"),
    }
}

#[test]
fn let_sort_in_type_position() {
    // let T : Type 0 := Nat in (λ(z:T). z)  — the annotation `T` is a Sort-level
    // let; the lambda's domain uses it, and zeta makes it Nat → Nat.
    let ctx = std_ctx();
    let lam = Term::Lambda {
        param: "z".to_string(),
        param_type: Box::new(Term::Var("T".to_string())),
        body: Box::new(Term::Var("z".to_string())),
    };
    let t = let_("T", Term::Sort(Universe::Type(0)), nat(), lam);
    let ty = infer_type(&ctx, &t).expect("type-level let should check");
    let expected = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body_type: Box::new(nat()),
    };
    assert!(is_subtype(&ctx, &ty, &expected), "type-level let → Nat→Nat, got {ty}");
}
