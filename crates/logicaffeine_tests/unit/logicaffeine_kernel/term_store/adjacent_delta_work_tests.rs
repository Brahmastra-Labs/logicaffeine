use super::*;
use crate::{Context, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn standard() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn chain(depth: usize) -> Context {
    let mut c = standard();
    c.with_equations_suspended(|c| {
        let mut tail = g("Zero");
        for i in (0..depth).rev() {
            let name = format!("delta_tail_{i}");
            c.declare(&name, g("Nat"), tail).unwrap();
            tail = g(&name);
        }
    });
    c
}

#[test]
fn adjacent_definitions_do_not_expand_their_common_tail() {
    for depth in [16, 128, 512] {
        let c = chain(depth);
        reset_intern_visits();
        assert!(beta_delta_equivalent(
            &c,
            &g("delta_tail_0"),
            &g("delta_tail_1"),
            1 << 16
        ));
        let visits = intern_visits();
        eprintln!("adjacent delta depth={depth}: intern visits={visits}");
        assert!(
            visits <= 32,
            "one delta step should suffice independently of tail length: {visits}"
        );
    }
}

#[test]
fn actual_equation_registration_does_not_reexpand_a_shared_tail() {
    fn measured(depth: usize) -> usize {
        let mut c = chain(depth);
        reset_intern_visits();
        c.declare("delta_fresh", g("Nat"), g("delta_tail_0"))
            .unwrap();
        let visits = intern_visits();
        assert!(
            c.get_definition_body("delta_fresh_eq_def").is_some(),
            "the actual checked equation must still be generated"
        );
        assert!(c.pending_obligations().is_empty());
        visits
    }
    let small = measured(8);
    let large = measured(128);
    eprintln!("equation registration intern visits: depth8={small}, depth128={large}");
    assert!(
        large <= 4 * small + 64,
        "registering one equation should not expand every shared-tail definition: {small}->{large}"
    );
}

#[test]
fn delta_probe_still_rejects_open_definition_bodies() {
    let mut c = standard().extend("external", g("Nat"));
    c.with_equations_suspended(|c| c.declare("open_delta", g("Nat"), v("external")))
        .unwrap();
    assert_eq!(c.get_definition_body("open_delta"), Some(&v("external")));
    assert!(!beta_delta_equivalent(
        &c,
        &g("open_delta"),
        &v("external"),
        100
    ));
    assert!(!beta_delta_equivalent(
        &c,
        &lam("external", g("Nat"), g("open_delta")),
        &lam("external", g("Nat"), v("external")),
        100
    ));
}

#[test]
fn delta_probe_does_not_erase_holes_or_import_float_identity() {
    let mut c = standard();
    let nat_fn = Term::Pi {
        param: "x".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(g("Nat")),
    };
    c.with_equations_suspended(|c| {
        c.declare("delta_constant", nat_fn, lam("x", g("Nat"), g("Zero")))
    })
    .unwrap();
    assert!(!beta_delta_equivalent(
        &c,
        &app(g("delta_constant"), Term::Hole),
        &g("Zero"),
        100
    ));
    let nan = Term::Lit(crate::Literal::Float(f64::NAN));
    c.with_equations_suspended(|c| c.declare("delta_nan", g("Float"), nan.clone()))
        .unwrap();
    assert!(!beta_delta_equivalent(&c, &g("delta_nan"), &nan, 100));
    assert!(!beta_delta_equivalent(
        &c,
        &g("delta_constant"),
        &lam("x", g("Nat"), g("Zero")),
        0
    ));
}

#[test]
fn adjacent_delta_equality_observes_rebound_definitions_and_contexts() {
    let mut original = chain(4);
    let kept = original
        .refused_meaning_change_for_test(|c| {
            c.with_equations_suspended(|c| {
                c.declare("delta_tail_1", g("Nat"), app(g("Succ"), g("Zero")))
            })
        })
        .expect("the gate keeps `delta_tail_1` under `delta_tail_0`");
    assert!(beta_delta_equivalent(
        &kept,
        &g("delta_tail_0"),
        &g("delta_tail_1"),
        100
    ));
    original.permit_meaning_changes_for_cache_tests();
    let mut changed = original.clone();
    changed
        .with_equations_suspended(|c| {
            c.declare("delta_tail_0", g("Nat"), app(g("Succ"), g("Zero")))
        })
        .unwrap();
    assert!(!beta_delta_equivalent(
        &changed,
        &g("delta_tail_0"),
        &g("delta_tail_1"),
        100
    ));
    assert!(beta_delta_equivalent(
        &original,
        &g("delta_tail_0"),
        &g("delta_tail_1"),
        100
    ));
    original
        .with_equations_suspended(|c| {
            c.declare("delta_tail_1", g("Nat"), app(g("Succ"), g("Zero")))
        })
        .unwrap();
    assert!(beta_delta_equivalent(
        &original,
        &g("delta_tail_0"),
        &g("delta_tail_1"),
        100
    ));
    assert!(!beta_delta_equivalent(
        &original,
        &g("delta_tail_0"),
        &g("Zero"),
        100
    ));
}

#[test]
fn explicit_universe_delta_instances_remain_distinct() {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    c.declare_universe_poly(
        "delta_sort",
        vec!["u".into()],
        Term::Sort(u.clone().succ()),
        Term::Sort(u),
    )
    .unwrap();
    let at = |level| Term::Const {
        name: "delta_sort".into(),
        levels: vec![level],
    };
    let one = at(Universe::Type(0));
    let two = at(Universe::Type(1));
    // The public conversion already supports these instances; the shortcut
    // must preserve both the positive result and the invariant mismatch.
    assert!(crate::defeq_for_test(
        &c,
        &one,
        &Term::Sort(Universe::Type(0))
    ));
    assert!(!crate::defeq_for_test(&c, &one, &two));
    let wrong = Term::Const {
        name: "delta_sort".into(),
        levels: vec![],
    };
    assert!(!beta_delta_equivalent(
        &c,
        &wrong,
        &Term::Sort(Universe::Type(0)),
        100
    ));
}
