//! Published J keeps bare-Global callers and supports higher motive universes.
//! Annotated Lets force BOTH checkers to verify the stated goals independently.
//! Existing conformance-corpus and recursor-generator tests remain unchanged.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    infer_type, normalize_checked, recheck, Context, KernelError, ReCheckError, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
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
fn annotated(value: Term, expected: Term) -> Term {
    Term::Let {
        name: "published_goal".into(),
        ty: Box::new(expected),
        value: Box::new(value),
        body: Box::new(v("published_goal")),
    }
}
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn eq_nat(a: Term, b: Term) -> Term {
    apps(g("Eq"), vec![g("Nat"), a, b])
}
fn refl_zero() -> Term {
    apps(g("refl"), vec![g("Nat"), g("Zero")])
}
fn constant_motive(target: Term) -> Term {
    lam("n", g("Nat"), lam("h", eq_nat(g("Zero"), v("n")), target))
}
fn reflexive_j(motive: Term, base: Term) -> Term {
    apps(
        g("Eq_rec_dep"),
        vec![g("Nat"), g("Zero"), motive, base, g("Zero"), refl_zero()],
    )
}
fn checks(c: &Context, value: Term, expected: Term) {
    let goal = annotated(value, expected);
    infer_type(c, &goal).expect("main checker must verify the actual annotated goal");
    recheck(c, &goal).expect("independent checker must verify the actual annotated goal");
}
fn rejects(c: &Context, value: Term, expected: Term) {
    let goal = annotated(value, expected);
    let main = infer_type(c, &goal);
    assert!(
        matches!(&main, Err(KernelError::TypeMismatch { .. })),
        "main checker must give a type mismatch, got {main:?}"
    );
    let independent = recheck(c, &goal);
    assert!(
        matches!(&independent, Err(ReCheckError::Ill(_))),
        "independent checker must reject as ill typed, got {independent:?}"
    );
}
fn assert_published(c: &Context, name: &str) {
    assert!(
        c.is_definition(name),
        "{name} must remain an ordinary definition"
    );
    assert!(
        c.get_global(name).is_some(),
        "{name} must support Global lookup"
    );
    assert!(
        c.get_definition_body(name).is_some(),
        "{name} must have a checked body"
    );
    assert!(
        !c.refused_declarations().iter().any(|(n, _)| n == name),
        "{name} must not have a recorded registration refusal"
    );
    // Global/body lookup hides pending declarations, so presence also rules out
    // merely deferred registration for this particular name.
}

#[test]
fn published_j_remains_a_global_definition_with_a_checked_body() {
    let c = ctx();
    assert_published(&c, "Eq_rec_dep");
    let ty = c.get_global("Eq_rec_dep").unwrap().clone();
    let body = c.get_definition_body("Eq_rec_dep").unwrap().clone();
    checks(&c, body, ty);
}

#[test]
fn published_j_preserves_prop_and_type_zero_instances() {
    let c = ctx();
    let proposition = eq_nat(g("Zero"), g("Zero"));
    checks(
        &c,
        reflexive_j(constant_motive(proposition.clone()), refl_zero()),
        proposition,
    );
    let value = reflexive_j(constant_motive(g("Nat")), g("Zero"));
    checks(&c, value.clone(), g("Nat"));
    assert_eq!(normalize_checked(&c, &value).unwrap(), g("Zero"));
}

#[test]
fn published_j_checks_index_and_proof_dependent_motives_at_higher_sorts() {
    let c = ctx();
    for level in [1, 2] {
        // P : Π n:Nat. Eq Nat a n → Type(level).
        // P b h depends on BOTH final arguments; the whole term is closed.
        let motive_ty = pi(
            "n",
            g("Nat"),
            pi("path", eq_nat(v("a"), v("n")), sort(level)),
        );
        let base_ty = apps(
            v("P"),
            vec![v("a"), apps(g("refl"), vec![g("Nat"), v("a")])],
        );
        let result_ty = apps(v("P"), vec![v("b"), v("h")]);
        let call = apps(
            g("Eq_rec_dep"),
            vec![g("Nat"), v("a"), v("P"), v("base"), v("b"), v("h")],
        );
        let binders = vec![
            ("a", g("Nat")),
            ("P", motive_ty),
            ("base", base_ty),
            ("b", g("Nat")),
            ("h", eq_nat(v("a"), v("b"))),
        ];
        let value = binders
            .iter()
            .rev()
            .fold(call, |body, (name, ty)| lam(name, ty.clone(), body));
        let expected = binders
            .iter()
            .rev()
            .fold(result_ty, |body, (name, ty)| pi(name, ty.clone(), body));
        checks(&c, value, expected);
    }
}

#[test]
fn published_j_transports_nat_type_zero_and_type_one_and_computes_at_refl() {
    let c = ctx();
    // The constant motive returning Type(k) has motive sort Type(k+1).
    // Bases: Nat : Type0, Type0 : Type1, and Type1 : Type2.
    for (level, base) in [(0, g("Nat")), (1, sort(0)), (2, sort(1))] {
        let value = reflexive_j(constant_motive(sort(level)), base.clone());
        checks(&c, value.clone(), sort(level));
        assert_eq!(normalize_checked(&c, &value).unwrap(), base);
    }
}

#[test]
fn partial_global_j_can_resume_after_motive_instantiation() {
    let c = ctx();
    let prefix = apps(
        g("Eq_rec_dep"),
        vec![g("Nat"), g("Zero"), constant_motive(sort(0))],
    );
    let prefix_ty = pi(
        "base",
        sort(0),
        pi("n", g("Nat"), pi("h", eq_nat(g("Zero"), v("n")), sort(0))),
    );
    let resumed = Term::Let {
        name: "saved_j".into(),
        ty: Box::new(prefix_ty),
        value: Box::new(prefix),
        body: Box::new(apps(v("saved_j"), vec![g("Nat"), g("Zero"), refl_zero()])),
    };
    checks(&c, resumed.clone(), sort(0));
    assert_eq!(normalize_checked(&c, &resumed).unwrap(), g("Nat"));
}

#[test]
fn both_checkers_reject_wrong_inhabitants_and_wrong_universe_results() {
    let c = ctx();
    let transported_nat = reflexive_j(constant_motive(sort(0)), g("Nat"));
    // Positive prerequisites prevent an unsupported J from satisfying rejection.
    checks(&c, transported_nat.clone(), sort(0));
    checks(&c, g("Zero"), transported_nat.clone());
    rejects(&c, g("true"), transported_nat);

    let transported_type_zero = reflexive_j(constant_motive(sort(1)), sort(0));
    checks(&c, transported_type_zero.clone(), sort(1));
    // Type0 inhabits Type1; cumulativity cannot lower it into Type0.
    rejects(&c, transported_type_zero, sort(0));
}

#[test]
fn existing_prelude_equality_consumers_keep_their_global_calling_conventions() {
    let c = ctx();
    for name in ["Eq_rec", "Eq_sym", "Eq_trans"] {
        assert_published(&c, name);
    }
    let expected = eq_nat(g("Zero"), g("Zero"));
    let rec = apps(
        g("Eq_rec"),
        vec![
            g("Nat"),
            g("Zero"),
            lam("n", g("Nat"), eq_nat(g("Zero"), v("n"))),
            refl_zero(),
            g("Zero"),
            refl_zero(),
        ],
    );
    let sym = apps(
        g("Eq_sym"),
        vec![g("Nat"), g("Zero"), g("Zero"), refl_zero()],
    );
    let trans = apps(
        g("Eq_trans"),
        vec![
            g("Nat"),
            g("Zero"),
            g("Zero"),
            g("Zero"),
            refl_zero(),
            refl_zero(),
        ],
    );
    for proof in [rec, sym, trans] {
        checks(&c, proof, expected.clone());
    }
}

#[test]
fn explicit_universes_allow_saving_j_before_its_higher_motive() {
    let c = ctx();
    // Ordinary-definition Const levels follow free_level_vars' first-occurrence
    // order: Eq's carrier level, then the independently derived motive level.
    for motive_level in [1, 2] {
        let prefix = apps(
            Term::Const {
                name: "Eq_rec_dep".into(),
                levels: vec![Universe::Type(0), Universe::Type(motive_level)],
            },
            vec![g("Nat"), g("Zero")],
        );
        let motive_ty = pi(
            "index",
            g("Nat"),
            pi("path", eq_nat(g("Zero"), v("index")), sort(motive_level)),
        );
        let prefix_ty = pi(
            "P",
            motive_ty,
            pi(
                "base",
                apps(v("P"), vec![g("Zero"), refl_zero()]),
                pi(
                    "n",
                    g("Nat"),
                    pi(
                        "h",
                        eq_nat(g("Zero"), v("n")),
                        apps(v("P"), vec![v("n"), v("h")]),
                    ),
                ),
            ),
        );
        // Check the complete remaining P/base/n/h telescope before resuming.
        checks(&c, prefix.clone(), prefix_ty.clone());
        let target = sort(motive_level - 1);
        let base = if motive_level == 1 { g("Nat") } else { sort(0) };
        let resumed = Term::Let {
            name: "saved_before_motive".into(),
            ty: Box::new(prefix_ty),
            value: Box::new(prefix),
            body: Box::new(apps(
                v("saved_before_motive"),
                vec![
                    constant_motive(target.clone()),
                    base.clone(),
                    g("Zero"),
                    refl_zero(),
                ],
            )),
        };
        checks(&c, resumed.clone(), target);
        assert_eq!(normalize_checked(&c, &resumed).unwrap(), base);
    }
}
