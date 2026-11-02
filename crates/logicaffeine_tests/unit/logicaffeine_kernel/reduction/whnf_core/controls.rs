use super::*;
use crate::{BigInt, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn env() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_definition(
        "two".into(),
        g("Nat"),
        app(g("Succ"), app(g("Succ"), g("Zero"))),
    );
    c.add_declaration(
        "pair_of",
        Term::Pi {
            param: "a".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(Term::Pi {
                param: "b".into(),
                param_type: Box::new(g("Nat")),
                body_type: Box::new(g("Nat")),
            }),
        },
    );
    c.add_definition(
        "id_nat".into(),
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
        lam("n", g("Nat"), v("n")),
    );
    c
}
fn is_zero(scrutinee: Term) -> Term {
    Term::Match {
        discriminant: Box::new(scrutinee),
        motive: Box::new(lam("_", g("Nat"), g("Bool"))),
        cases: vec![g("true"), lam("p", g("Nat"), g("false"))],
    }
}

#[test]
fn beta_zeta_and_iota_reduce_at_the_head_and_the_head_constant_stays_folded() {
    let c = env();
    let mut a = Allowance::new(1 << 10);
    assert_eq!(
        whnf_core(&c, app(lam("x", g("Nat"), v("x")), g("Zero")), &mut a),
        Ok(g("Zero"))
    );
    let shared = Term::Let {
        name: "x".into(),
        ty: Box::new(g("Nat")),
        value: Box::new(g("Zero")),
        body: Box::new(app(app(g("pair_of"), v("x")), v("x"))),
    };
    assert_eq!(
        whnf_core(&c, shared, &mut a),
        Ok(app(app(g("pair_of"), g("Zero")), g("Zero"))),
        "ζ at the head"
    );
    // ι after putting the discriminant in weak-head form, δ included: `two` unfolds to `Succ …`.
    assert_eq!(whnf_core(&c, is_zero(g("two")), &mut a), Ok(g("false")));
    // The head constant itself is left for the conversion loop.
    assert_eq!(
        whnf_core(&c, app(g("id_nat"), g("Zero")), &mut a),
        Ok(app(g("id_nat"), g("Zero")))
    );
    assert_eq!(delta_head(&c, &app(g("id_nat"), g("Zero"))), Some("id_nat"));
    assert_eq!(
        delta_step(&c, &app(g("id_nat"), g("Zero"))),
        Some(app(lam("n", g("Nat"), v("n")), g("Zero")))
    );
    assert_eq!(
        whnf(&c, &app(g("id_nat"), g("Zero")), &mut a),
        Ok(g("Zero"))
    );
}

#[test]
fn a_literal_discriminant_selects_its_constructor_branch() {
    let c = env();
    let mut a = Allowance::new(1 << 10);
    assert_eq!(whnf_core(&c, is_zero(lit(0)), &mut a), Ok(g("true")));
    assert_eq!(whnf_core(&c, is_zero(lit(7)), &mut a), Ok(g("false")));
}

#[test]
fn a_stuck_head_is_returned_unchanged() {
    let c = env().extend("x", g("Nat"));
    let mut a = Allowance::new(1 << 10);
    assert_eq!(whnf_core(&c, is_zero(v("x")), &mut a), Ok(is_zero(v("x"))));
    assert_eq!(whnf(&c, &v("x"), &mut a), Ok(v("x")));
    assert_eq!(delta_head(&c, &v("x")), None);
    assert_eq!(delta_head(&c, &Term::Sort(Universe::Type(0))), None);
}

#[test]
fn the_beta_normal_form_contracts_every_redex_and_unfolds_nothing() {
    let c = env();
    let redex = |t: Term| app(lam("y", g("Nat"), v("y")), t);
    let term = Term::Pi {
        param: "p".into(),
        param_type: Box::new(redex(g("Nat"))),
        body_type: Box::new(app(g("id_nat"), redex(app(g("Succ"), redex(g("two")))))),
    };
    let expected = Term::Pi {
        param: "p".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(app(g("id_nat"), app(g("Succ"), g("two")))),
    };
    assert_eq!(
        beta_normal_form(&term, &mut 16),
        Some(expected),
        "every β-redex, no δ (`id_nat`, `two` stay)"
    );
    assert_eq!(
        beta_normal_form(&term, &mut 2),
        None,
        "three contractions need three steps"
    );
    let _ = &c;
}

/// Lean's `reduce_nat`: a body-less primitive computes on the weak-head normal forms of its
/// arguments. Left stuck, the term would look neutral to the conversion loop while `normalize`
/// (which normalizes the arguments first) computes it.
#[test]
fn a_primitive_computes_once_its_arguments_reach_literals() {
    let mut c = env();
    c.add_definition(
        "hello".into(),
        g("Text"),
        Term::Lit(Literal::Text("ab".into())),
    );
    let mut a = Allowance::new(1 << 10);
    assert_eq!(
        whnf_core(&c, app(g("Text_length"), g("hello")), &mut a),
        Ok(lit(2))
    );
    assert_eq!(
        whnf_core(&c, is_zero(app(g("Text_length"), g("hello"))), &mut a),
        Ok(g("false")),
        "a discriminant that is a primitive over a definition selects its branch"
    );
    let c = c.extend("t", g("Text"));
    assert_eq!(
        whnf_core(&c, app(g("Text_length"), v("t")), &mut a),
        Ok(app(g("Text_length"), v("t"))),
        "a primitive over a local stays stuck, unchanged"
    );
}

/// Lean's quotient reduction puts the eliminator's major premise in weak-head normal form
/// before looking for `Quot.mk`.
#[test]
fn a_quotient_lift_reduces_once_its_major_premise_reaches_quot_mk() {
    let mut c = env();
    let rel = Term::Lambda {
        param: "a".into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(Term::Lambda {
            param: "b".into(),
            param_type: Box::new(g("Nat")),
            body: Box::new(g("True")),
        }),
    };
    let quot = app(app(g("Quot"), g("Nat")), rel.clone());
    let made = app(app(app(g("Quot_mk"), g("Nat")), rel.clone()), g("Zero"));
    c.add_definition("made".into(), quot, made);
    let lift = |major: Term| {
        [
            g("Nat"),
            rel.clone(),
            g("Nat"),
            lam("n", g("Nat"), app(g("Succ"), v("n"))),
            g("respects"),
            major,
        ]
        .into_iter()
        .fold(g("Quot_lift"), app)
    };
    let mut a = Allowance::new(1 << 10);
    assert_eq!(
        whnf_core(&c, lift(g("made")), &mut a),
        Ok(app(g("Succ"), g("Zero")))
    );
    let c = c.extend("q", app(app(g("Quot"), g("Nat")), rel.clone()));
    assert_eq!(
        whnf_core(&c, lift(v("q")), &mut a),
        Ok(lift(v("q"))),
        "a lift of a local quotient stays stuck"
    );
}

#[test]
fn the_allowance_is_an_answer_of_its_own() {
    let c = env();
    let mut a = Allowance::new(0);
    assert_eq!(
        whnf_core(&c, app(lam("x", g("Nat"), v("x")), g("Zero")), &mut a),
        Err(Exhausted)
    );
}
