use super::*;
use crate::prelude::StandardLibrary;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(name: &str, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}
fn lam(name: &str, domain: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(domain),
        body: Box::new(body),
    }
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn hidden(ctx: &mut Context, ty: Term, mode: usize) -> Term {
    match mode {
        0 => ty,
        1 => app(lam("T", sort(), v("T")), ty),
        2 => Term::Let {
            name: "T".into(),
            ty: Box::new(sort()),
            value: Box::new(ty),
            body: Box::new(v("T")),
        },
        3 => {
            ctx.declare("WorkTypeAlias", sort(), ty).unwrap();
            g("WorkTypeAlias")
        }
        _ => unreachable!(),
    }
}
fn measured(ctx: &Context, term: &Term) -> (KernelResult<Term>, [usize; 2], usize) {
    TYPE_HEAD_FULL_NORMALIZATIONS.with(|n| n.set([0; 2]));
    crate::term_store::reset_intern_visits();
    let result = infer_type(ctx, term);
    let visits = crate::term_store::intern_visits();
    let counts = TYPE_HEAD_FULL_NORMALIZATIONS.with(|n| n.get());
    eprintln!("type-head full normalizations {counts:?}; actual TermStore intern visits {visits}");
    (result, counts, visits)
}

#[test]
fn wrong_requested_shape_still_uses_full_fallback_and_rejects() {
    let ctx = context().extend("f", g("Nat"));
    let (result, counts, _) = measured(&ctx, &app(v("f"), g("Zero")));
    assert!(matches!(result, Err(KernelError::NotAFunction(_))));
    assert_eq!(counts, [1, 0]);
    let term = Term::Match {
        discriminant: Box::new(lam("n", g("Nat"), v("n"))),
        motive: Box::new(g("Nat")),
        cases: vec![],
    };
    let (result, counts, _) = measured(&ctx, &term);
    assert!(matches!(result, Err(KernelError::NotAnInductive(_))));
    assert_eq!(counts, [0, 1]);
}

#[test]
fn custom_simp_context_keeps_the_full_type_head_path() {
    let mut ctx = context();
    ctx.declare(
        "WorkIdentity",
        pi("n", g("Nat"), g("Nat")),
        lam("n", g("Nat"), v("n")),
    )
    .unwrap();
    let equation = app(
        app(app(g("Eq"), g("Nat")), app(g("WorkIdentity"), v("n"))),
        v("n"),
    );
    ctx.declare(
        "WorkIdentity.rule",
        pi("n", g("Nat"), equation),
        lam("n", g("Nat"), app(app(g("refl"), g("Nat")), v("n"))),
    )
    .unwrap();
    ctx.add_simp_lemma("WorkIdentity.rule").unwrap();
    let ty = hidden(&mut ctx, pi("n", g("Nat"), g("Nat")), 3);
    let ctx = ctx.extend("f", ty);
    let (result, counts, _) = measured(&ctx, &app(v("f"), g("Zero")));
    assert!(result.is_ok());
    assert_eq!(counts, [1, 0]);
    let ctx = ctx.extend("major", g("Nat"));
    let term = Term::Match {
        discriminant: Box::new(v("major")),
        motive: Box::new(g("Nat")),
        cases: vec![g("Zero"), lam("n", g("Nat"), g("Zero"))],
    };
    let (result, counts, _) = measured(&ctx, &term);
    assert!(result.is_ok());
    assert_eq!(counts, [0, 1]);
}

#[test]
fn closed_real_application_exposes_direct_beta_let_delta_pi_without_full_normalization() {
    let mut observations = Vec::new();
    for mode in 0..4 {
        let mut ctx = context();
        let ty = hidden(&mut ctx, pi("n", g("Nat"), g("Nat")), mode);
        assert!(infer_sort(&ctx, &ty).is_ok());
        let ctx = ctx.extend("f", ty);
        let term = app(v("f"), g("Zero"));
        let (result, counts, _) = measured(&ctx, &term);
        assert!(def_eq(&ctx, &result.unwrap(), &g("Nat")));
        assert!(crate::recheck::recheck(
            &ctx,
            &lam("f", ctx.get("f").unwrap().clone(), term.clone())
        )
        .is_ok());
        observations.push((mode, counts));
    }
    assert!(
        observations.iter().all(|(_, counts)| *counts == [0, 0]),
        "all requested Pi heads must avoid full normalization: {observations:?}"
    );
}

#[test]
fn closed_real_application_keeps_unneeded_pi_suffixes_lazy() {
    let mut observations = Vec::new();
    for depth in [8, 16, 32] {
        let mut ctx = context();
        let lazy_nat = app(lam("T", sort(), v("T")), g("Nat"));
        let mut suffix = g("Nat");
        for index in 0..depth {
            suffix = pi(&format!("n{index}"), lazy_nat.clone(), suffix);
        }
        let ty = hidden(&mut ctx, pi("unused", g("Nat"), suffix.clone()), 3);
        let ctx = ctx.extend("f", ty);
        let term = app(v("f"), g("Zero"));
        let (result, counts, visits) = measured(&ctx, &term);
        assert!(def_eq(&ctx, &result.unwrap(), &suffix));
        assert!(crate::recheck::recheck(
            &ctx,
            &lam("f", ctx.get("f").unwrap().clone(), term.clone())
        )
        .is_ok());
        eprintln!("suffix depth {depth}, intern visits {visits}");
        observations.push((depth, counts, visits));
    }
    assert!(
        observations.iter().all(|(_, counts, _)| *counts == [0, 0]),
        "exposing Pi must not normalize its entire suffix: {observations:?}"
    );
}

#[test]
fn closed_real_match_exposes_direct_beta_let_delta_inductive_without_full_normalization() {
    let mut observations = Vec::new();
    for mode in 0..4 {
        let mut ctx = context();
        let ty = hidden(&mut ctx, g("Nat"), mode);
        assert!(infer_sort(&ctx, &ty).is_ok());
        let ctx = ctx.extend("major", ty);
        let term = Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(g("Nat")),
            cases: vec![g("Zero"), lam("n", g("Nat"), g("Zero"))],
        };
        let (result, counts, _) = measured(&ctx, &term);
        assert!(def_eq(&ctx, &result.unwrap(), &g("Nat")));
        assert!(crate::recheck::recheck(
            &ctx,
            &lam("major", ctx.get("major").unwrap().clone(), term.clone())
        )
        .is_ok());
        observations.push((mode, counts));
    }
    assert!(
        observations.iter().all(|(_, counts)| *counts == [0, 0]),
        "registered inductive heads must leave arguments lazy: {observations:?}"
    );
}

#[test]
fn closed_oversized_positive_probe_retains_full_fallback() {
    // A shallow, balanced type exceeds the probe's 4096-node bound without
    // approaching the kernel depth guard. It must still type-check normally.
    let mut domain = g("Nat");
    for _ in 0..12 {
        domain = pi("unused", domain.clone(), domain);
    }
    let mut ctx = context();
    let ty = hidden(&mut ctx, pi("unused", g("Nat"), domain.clone()), 3);
    let ctx = ctx.extend("f", ty);
    let term = app(v("f"), g("Zero"));
    let (result, counts, _) = measured(&ctx, &term);
    assert!(def_eq(&ctx, &result.unwrap(), &domain));
    assert!(
        crate::recheck::recheck(&ctx, &lam("f", ctx.get("f").unwrap().clone(), term.clone()))
            .is_ok()
    );
    assert_eq!(counts, [1, 0]);
}
