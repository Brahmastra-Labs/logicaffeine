//! Decidable ground integer order — the substrate for linear-arithmetic
//! certificates. Comparison builtins (`le`/`lt`/`ge`/`gt`) compute on Int literals
//! (reduction.rs), so the Prop `Eq Bool (le m n) true` — the shallow encoding of
//! `m ≤ n` — holds by `refl` exactly when `m ≤ n`, and is unprovable otherwise.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, normalize, Context, Literal, Term};

fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn app2(f: &str, a: Term, b: Term) -> Term {
    Term::App(Box::new(Term::App(Box::new(g(f)), Box::new(a))), Box::new(b))
}
fn eq_bool(a: Term, b: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::App(Box::new(g("Eq")), Box::new(g("Bool")))),
            Box::new(a),
        )),
        Box::new(b),
    )
}

#[test]
fn ground_comparison_computes() {
    let c = ctx();
    assert_eq!(normalize(&c, &app2("le", lit(2), lit(5))), g("true"));
    assert_eq!(normalize(&c, &app2("le", lit(5), lit(2))), g("false"));
    assert_eq!(normalize(&c, &app2("le", lit(3), lit(3))), g("true"));
    assert_eq!(normalize(&c, &app2("lt", lit(3), lit(3))), g("false"));
    assert_eq!(normalize(&c, &app2("ge", lit(7), lit(7))), g("true"));
    assert_eq!(normalize(&c, &app2("gt", lit(8), lit(7))), g("true"));
    // composes with arithmetic: le (add 2 2) 5  ⇝  le 4 5  ⇝  true
    assert_eq!(
        normalize(&c, &app2("le", app2("add", lit(2), lit(2)), lit(5))),
        g("true")
    );
}

#[test]
fn comparison_types_as_bool() {
    let c = ctx();
    assert_eq!(infer_type(&c, &app2("le", lit(2), lit(5))).unwrap(), g("Bool"));
}

#[test]
fn ground_inequality_prop_is_decided_by_computation() {
    let c = ctx();
    // refl Bool true : Eq Bool true true
    let refl_true = Term::App(
        Box::new(Term::App(Box::new(g("refl")), Box::new(g("Bool")))),
        Box::new(g("true")),
    );
    let refl_ty = infer_type(&c, &refl_true).expect("refl Bool true type-checks");

    // 2 ≤ 5  ≡  Eq Bool (le 2 5) true  — holds (le 2 5 ⇝ true, defeq to true=true)
    let holds = eq_bool(app2("le", lit(2), lit(5)), g("true"));
    assert!(
        is_subtype(&c, &refl_ty, &holds),
        "refl should prove le(2,5)=true"
    );

    // 5 ≤ 2  ≡  Eq Bool (le 5 2) true  — must NOT hold (le 5 2 ⇝ false)
    let fails = eq_bool(app2("le", lit(5), lit(2)), g("true"));
    assert!(
        !is_subtype(&c, &refl_ty, &fails),
        "le(5,2)=true must be unprovable (soundness)"
    );
}
