//! Transparent theorems are unfolded LAST in conversion, as Lean's `lazy_delta_reduction` does.
//!
//! Tristen's ruling (A) made theorem bodies transparent: `Acc.rec` and well-founded recursion
//! need them. Unfolding them eagerly in every conversion is what took the IndexedProjection
//! witness from 194 s to 501 s and `Nat.modCore_eq_mod` from 4.3 s to 264 s. Conversion now
//! compares with theorems folded first and unfolds them only when that comparison fails.
//! These locks pin both halves: folding never changes a verdict, and unfolding still happens
//! when — and only when — a proof's constructor is what the conversion depends on.
//! The recursive-elimination witness is
//! `checked_theorem_publication::transparent_theorem_accessibility_drives_actual_recursive_elimination`
//! and the imported `wfSum` in `original_well_founded_promotion_bridge`.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Context, Term};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(f, app)
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}
fn one() -> Term {
    succ(g("Zero"))
}
fn two() -> Term {
    succ(one())
}
fn redex(t: Term) -> Term {
    app(lam("x", g("Nat"), Term::Var("x".into())), t)
}

fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.declare_theorem_checked("left_true", vec![], or_true_false(), in_left())
        .unwrap();
    c.declare_theorem_checked("right_true", vec![], or_false_true(), in_right())
        .unwrap();
    c.declare_theorem_checked("trivial_a", vec![], g("True"), g("I"))
        .unwrap();
    c.add_declaration("observe", pi("_", g("True"), pi("_", g("Nat"), g("Nat"))));
    c
}
fn or_true_false() -> Term {
    apps(g("Or"), vec![g("True"), g("False")])
}
fn or_false_true() -> Term {
    apps(g("Or"), vec![g("False"), g("True")])
}
fn in_left() -> Term {
    apps(g("left"), vec![g("True"), g("False"), g("I")])
}
fn in_right() -> Term {
    apps(g("right"), vec![g("False"), g("True"), g("I")])
}
fn which(proof: &str, ty: Term, on_left: Term, on_right: Term) -> Term {
    let Term::App(head, right) = &ty else {
        unreachable!()
    };
    let Term::App(_, left) = head.as_ref() else {
        unreachable!()
    };
    Term::Match {
        discriminant: Box::new(g(proof)),
        motive: Box::new(lam("_", ty.clone(), g("Nat"))),
        cases: vec![
            lam("_", (**left).clone(), on_left),
            lam("_", (**right).clone(), on_right),
        ],
    }
}

#[test]
fn a_folded_theorem_decides_a_conversion_without_its_proof() {
    let c = context();
    let a = apps(g("observe"), vec![g("trivial_a"), redex(two())]);
    let b = apps(g("observe"), vec![g("trivial_a"), two()]);
    assert!(defeq_for_test(&c, &a, &b));
    assert!(
        !defeq_for_test(&c, &a, &apps(g("observe"), vec![g("trivial_a"), one()])),
        "folding a theorem never makes unequal data equal"
    );
}

#[test]
fn distinct_proofs_of_one_proposition_convert_by_irrelevance() {
    let c = context();
    let a = apps(g("observe"), vec![g("trivial_a"), redex(one())]);
    let b = apps(g("observe"), vec![g("I"), one()]);
    assert!(defeq_for_test(&c, &a, &b));
}

#[test]
fn a_theorem_is_unfolded_when_its_constructor_decides_the_conversion() {
    let c = context();
    let left = which("left_true", or_true_false(), one(), two());
    let right = which("right_true", or_false_true(), one(), two());
    assert!(
        defeq_for_test(&c, &left, &one()),
        "left_true is `left`: the first branch"
    );
    assert!(
        defeq_for_test(&c, &right, &two()),
        "right_true is `right`: the second branch"
    );
    assert!(
        !defeq_for_test(&c, &left, &two()),
        "a false conversion is still refused"
    );
    assert!(
        !defeq_for_test(&c, &right, &one()),
        "a false conversion is still refused"
    );
    assert!(!defeq_for_test(&c, &left, &right));
}

/// Unfolding a theorem at conversion time exposes its constructor to `iota`, never to the
/// typing rules: the match that selects a branch of `Or` into `Nat` is still refused as a
/// large elimination by both kernels, however the proof is spelled.
#[test]
fn transparency_does_not_license_large_elimination_from_a_theorem() {
    let c = context();
    let annotated = |value: Term, ty: Term| Term::Let {
        name: "goal".into(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(Term::Var("goal".into())),
    };
    let eq = |a: Term, b: Term| apps(g("Eq"), vec![g("Nat"), a, b]);
    let refl = |a: Term| apps(g("refl"), vec![g("Nat"), a]);
    for (proof, ty, value) in [
        ("left_true", or_true_false(), one()),
        ("right_true", or_false_true(), two()),
    ] {
        let selected = which(proof, ty, one(), two());
        let goal = annotated(refl(value.clone()), eq(selected, value));
        let main = infer_type(&c, &goal);
        assert!(
            matches!(&main, Err(logicaffeine_kernel::KernelError::InvalidMotive(m)) if m.contains("large elimination")),
            "{proof}: {main:?}"
        );
        let independent = recheck(&c, &goal);
        assert!(
            matches!(&independent, Err(logicaffeine_kernel::ReCheckError::Ill(m)) if m.contains("large elimination")),
            "{proof}: {independent:?}"
        );
    }
}

/// The re-checker folds theorems in its own conversion too (`recheck::whnf_theorems_folded`);
/// both kernels must still decide the same goals the same way.
#[test]
fn both_kernels_convert_with_a_folded_theorem_and_refuse_a_false_goal() {
    let c = context();
    let annotated = |value: Term, ty: Term| Term::Let {
        name: "goal".into(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(Term::Var("goal".into())),
    };
    let eq = |a: Term, b: Term| apps(g("Eq"), vec![g("Nat"), a, b]);
    let refl = |a: Term| apps(g("refl"), vec![g("Nat"), a]);
    let observed = |proof: &str, n: Term| apps(g("observe"), vec![g(proof), n]);
    let good = annotated(
        refl(observed("trivial_a", two())),
        eq(
            observed("trivial_a", redex(two())),
            observed("trivial_a", two()),
        ),
    );
    infer_type(&c, &good).expect("main kernel converts with the theorem folded");
    recheck(&c, &good).expect("re-checker converts with the theorem folded");
    let irrelevant = annotated(
        refl(observed("trivial_a", two())),
        eq(observed("I", two()), observed("trivial_a", redex(two()))),
    );
    infer_type(&c, &irrelevant).expect("main kernel: distinct proofs are irrelevant");
    recheck(&c, &irrelevant).expect("re-checker: distinct proofs are irrelevant");
    let bad = annotated(
        refl(observed("trivial_a", two())),
        eq(
            observed("trivial_a", redex(two())),
            observed("trivial_a", one()),
        ),
    );
    assert!(
        infer_type(&c, &bad).is_err(),
        "main kernel refuses 2 = 1 under a folded theorem"
    );
    assert!(
        recheck(&c, &bad).is_err(),
        "re-checker refuses 2 = 1 under a folded theorem"
    );
}
