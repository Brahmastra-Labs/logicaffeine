//! Theorem bodies are independently checked before transparent publication.
//! Actual opaque declarations retain their existing opacity contract.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, infer_type, normalize_checked, recheck, Context,
    KernelError, ReCheckError, Term, Universe,
};
use std::collections::BTreeSet;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
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
fn eq(ty: Term, left: Term, right: Term) -> Term {
    apps(g("Eq"), vec![ty, left, right])
}
fn refl(ty: Term, value: Term) -> Term {
    apps(g("refl"), vec![ty, value])
}
fn bool_match(value: Term, motive: Term, yes: Term, no: Term) -> Term {
    // StandardLibrary registers true before false.
    Term::Match {
        discriminant: Box::new(value),
        motive: Box::new(motive),
        cases: vec![yes, no],
    }
}
fn annotated(value: Term, expected: Term) -> Term {
    Term::Let {
        name: "wf_checked_goal".into(),
        ty: Box::new(expected),
        value: Box::new(value),
        body: Box::new(v("wf_checked_goal")),
    }
}
fn checks(c: &Context, value: Term, expected: Term) {
    let goal = annotated(value, expected);
    infer_type(c, &goal).expect("main checker must check the actual declared goal");
    recheck(c, &goal).expect("independent checker must check the actual declared goal");
}
fn rejects(c: &Context, value: Term, expected: Term) {
    let goal = annotated(value, expected);
    let main = infer_type(c, &goal);
    assert!(
        matches!(&main, Err(KernelError::TypeMismatch { .. })),
        "expected a main type mismatch, got {main:?}"
    );
    let independent = recheck(c, &goal);
    assert!(
        matches!(&independent, Err(ReCheckError::Ill(_))),
        "expected independent ill-typing, got {independent:?}"
    );
}
fn computes(c: &Context, value: Term, ty: Term, expected: Term) {
    checks(c, value.clone(), ty.clone());
    checks(c, expected.clone(), ty.clone());
    // Both checkers must convert the actual result in the equality's endpoint.
    checks(
        c,
        refl(ty.clone(), expected.clone()),
        eq(ty, value.clone(), expected.clone()),
    );
    assert_eq!(
        normalize_checked(c, &value).expect("checked normalization"),
        expected
    );
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}

// ==== A finite relation with one genuine recursive edge ====

// R y false = False; R y true = Eq Bool y false.
fn relation() -> Term {
    lam(
        "predecessor",
        g("Bool"),
        lam(
            "point",
            g("Bool"),
            bool_match(
                v("point"),
                lam("selected", g("Bool"), prop()),
                eq(g("Bool"), v("predecessor"), g("false")),
                g("False"),
            ),
        ),
    )
}
fn edge(y: Term, x: Term) -> Term {
    apps(relation(), vec![y, x])
}
fn acc(x: Term) -> Term {
    apps(g("Acc"), vec![g("Bool"), relation(), x])
}
fn access_field(x: Term) -> Term {
    pi(
        "predecessor",
        g("Bool"),
        pi("edge", edge(v("predecessor"), x), acc(v("predecessor"))),
    )
}
fn accessible_false() -> Term {
    let impossible = Term::Match {
        discriminant: Box::new(v("edge")),
        motive: Box::new(lam("impossible", g("False"), acc(v("predecessor")))),
        cases: vec![],
    };
    apps(
        g("Acc_intro"),
        vec![
            g("Bool"),
            relation(),
            g("false"),
            lam(
                "predecessor",
                g("Bool"),
                lam("edge", edge(v("predecessor"), g("false")), impossible),
            ),
        ],
    )
}
fn accessible_true() -> Term {
    // Transport the actual accessibility proof of false along false = y.
    let reversed = apps(
        g("Eq_sym"),
        vec![g("Bool"), v("predecessor"), g("false"), v("edge")],
    );
    let transport = apps(
        g("Eq_rec_dep"),
        vec![
            g("Bool"),
            g("false"),
            lam(
                "target",
                g("Bool"),
                lam(
                    "path",
                    eq(g("Bool"), g("false"), v("target")),
                    acc(v("target")),
                ),
            ),
            accessible_false(),
            v("predecessor"),
            reversed,
        ],
    );
    apps(
        g("Acc_intro"),
        vec![
            g("Bool"),
            relation(),
            g("true"),
            lam(
                "predecessor",
                g("Bool"),
                lam("edge", edge(v("predecessor"), g("true")), transport),
            ),
        ],
    )
}
fn ih(x: Term, target: Term) -> Term {
    pi(
        "predecessor",
        g("Bool"),
        pi("edge", edge(v("predecessor"), x), target),
    )
}
fn step(target: Term, base: Term) -> Term {
    // At true the result MUST come from recursion at false; only false contains base.
    let recursive = apps(v("smaller"), vec![g("false"), refl(g("Bool"), g("false"))]);
    lam(
        "point",
        g("Bool"),
        bool_match(
            v("point"),
            lam(
                "selected",
                g("Bool"),
                pi("smaller", ih(v("selected"), target.clone()), target.clone()),
            ),
            lam("smaller", ih(g("true"), target.clone()), recursive),
            lam("smaller", ih(g("false"), target), base),
        ),
    )
}
fn acc_call(proof: Term, target: Term, base: Term) -> Term {
    let minor = lam(
        "point",
        g("Bool"),
        lam(
            "access",
            access_field(v("point")),
            lam(
                "smaller",
                ih(v("point"), target.clone()),
                apps(step(target.clone(), base), vec![v("point"), v("smaller")]),
            ),
        ),
    );
    apps(
        g("Acc_rec"),
        vec![
            g("Bool"),
            relation(),
            lam("point", g("Bool"), lam("proof", acc(v("point")), target)),
            minor,
            g("true"),
            proof,
        ],
    )
}
fn absent(c: &Context, name: &str) {
    assert!(c.get_global(name).is_none(), "{name}");
    assert!(c.poly_declaration(name).is_none(), "{name}");
    assert!(c.get_definition_body(name).is_none(), "{name}");
    assert!(c.get_universe_poly(name).is_none(), "{name}");
    assert!(axiom_dependencies_of(c, name).is_none(), "{name}");
}
fn usage(c: &Context, name: &str, expected: &[&str]) {
    let actual = axiom_dependencies_of(c, name).expect("published declaration audit");
    assert!(actual.unresolved.is_empty(), "{actual:?}");
    let expected: BTreeSet<String> = expected.iter().map(|s| (*s).into()).collect();
    assert_eq!(actual.axioms, expected);
}
fn reflexive() -> (Term, Term) {
    (
        eq(g("Nat"), g("Zero"), g("Zero")),
        refl(g("Nat"), g("Zero")),
    )
}
fn poly_reflexive() -> (Term, Term) {
    let domain = Term::Sort(Universe::Var("u".into()));
    (
        pi(
            "A",
            domain.clone(),
            pi("x", v("A"), eq(v("A"), v("x"), v("x"))),
        ),
        lam("A", domain, lam("x", v("A"), refl(v("A"), v("x")))),
    )
}

#[test]
fn transparent_theorem_accessibility_drives_actual_recursive_elimination() {
    let mut c = context();
    checks(&c, accessible_true(), acc(g("true")));
    c.declare_theorem_checked("checked_access", vec![], acc(g("true")), accessible_true())
        .expect("valid accessibility theorem");
    checks(&c, g("checked_access"), acc(g("true")));
    let result = acc_call(g("checked_access"), g("Nat"), g("Zero"));
    // The true branch uses the recursive hypothesis at false, whose proof must
    // be exposed from the actual theorem body. No replacement recursor is used.
    computes(&c, result.clone(), g("Nat"), g("Zero"));
    rejects(
        &c,
        refl(g("Nat"), app(g("Succ"), g("Zero"))),
        eq(g("Nat"), result, app(g("Succ"), g("Zero"))),
    );
    usage(&c, "checked_access", &[]);
    assert!(axiom_dependencies(&c, &g("checked_access"))
        .axioms
        .is_empty());
}

#[test]
fn theorem_transparency_does_not_change_actual_opaque_publication() {
    let mut c = context();
    let (ty, body) = reflexive();
    c.declare_theorem_checked("visible", vec![], ty.clone(), body.clone())
        .unwrap();
    c.declare_opaque_checked("hidden", vec![], ty.clone(), body.clone())
        .unwrap();
    for term in [
        g("visible"),
        Term::Const {
            name: "visible".into(),
            levels: vec![],
        },
    ] {
        checks(&c, term.clone(), ty.clone());
        assert_eq!(
            normalize_checked(&c, &term).unwrap(),
            normalize_checked(&c, &body).unwrap()
        );
    }
    for term in [
        g("hidden"),
        Term::Const {
            name: "hidden".into(),
            levels: vec![],
        },
    ] {
        checks(&c, term.clone(), ty.clone());
        assert_eq!(normalize_checked(&c, &term).unwrap(), term);
    }
    assert!(c.get_definition_body("hidden").is_none());
    assert!(c.get_universe_poly("hidden").is_none());
    usage(&c, "visible", &[]);
    usage(&c, "hidden", &[]);
}

#[test]
fn explicit_theorem_universe_order_unused_levels_and_global_calls_are_preserved() {
    let mut c = context();
    let (ty, body) = poly_reflexive();
    // The unused first level is deliberate: publication must not rediscover or
    // sort the level list from the body and silently shift the used parameter.
    c.declare_theorem_checked("poly_refl", vec!["unused".into(), "u".into()], ty, body)
        .unwrap();
    for (level, carrier, value) in [
        (Universe::Type(0), g("Nat"), g("Zero")),
        (Universe::Type(1), sort(0), g("Nat")),
    ] {
        let explicit = Term::Const {
            name: "poly_refl".into(),
            levels: vec![Universe::Type(2), level],
        };
        let expected = eq(carrier.clone(), value.clone(), value.clone());
        let applied = apps(explicit, vec![carrier.clone(), value.clone()]);
        checks(&c, applied.clone(), expected.clone());
        assert_eq!(
            normalize_checked(&c, &applied).unwrap(),
            normalize_checked(&c, &refl(carrier.clone(), value.clone())).unwrap()
        );
        checks(&c, apps(g("poly_refl"), vec![carrier, value]), expected);
    }
    for levels in [
        vec![],
        vec![Universe::Type(0)],
        vec![Universe::Type(0), Universe::Type(0), Universe::Type(0)],
    ] {
        let bad = Term::Const {
            name: "poly_refl".into(),
            levels,
        };
        assert!(infer_type(&c, &bad).is_err());
        assert!(recheck(&c, &bad).is_err());
    }
    usage(&c, "poly_refl", &[]);
}

#[test]
fn invalid_nonpropositional_open_and_unresolved_theorems_publish_nothing() {
    let mut c = context();
    let (ty, body) = reflexive();
    c.add("local_proof", ty.clone());
    let (poly_ty, poly_body) = poly_reflexive();
    for (name, levels, declared, value) in [
        ("wrong_proof", vec![], g("False"), body.clone()),
        ("nonprop", vec![], g("Nat"), g("Zero")),
        ("not_a_type", vec![], g("Zero"), body.clone()),
        ("open_proof", vec![], ty.clone(), v("local_proof")),
        ("hole_proof", vec![], ty.clone(), Term::Hole),
        ("hole_header", vec![], Term::Hole, body.clone()),
        ("missing_proof", vec![], ty.clone(), g("undeclared_proof")),
        ("self_reference", vec![], ty.clone(), g("self_reference")),
        ("missing_level", vec![], poly_ty, poly_body),
        (
            "duplicate_levels",
            vec!["u".into(), "u".into()],
            ty.clone(),
            body.clone(),
        ),
    ] {
        let before = c.pending_obligations().len();
        assert!(
            c.declare_theorem_checked(name, levels, declared, value)
                .is_err(),
            "{name}"
        );
        absent(&c, name);
        assert_eq!(c.pending_obligations().len(), before, "{name}");
        checks(&c, body.clone(), ty.clone());
    }
}

#[test]
fn replacements_preserve_the_existing_body_or_true_assumption() {
    let mut c = context();
    let (ty, body) = reflexive();
    c.declare_theorem_checked("original", vec![], ty.clone(), body.clone())
        .unwrap();
    let before = normalize_checked(&c, &g("original")).unwrap();
    assert!(c
        .declare_theorem_checked("original", vec![], ty.clone(), body.clone())
        .is_err());
    assert_eq!(normalize_checked(&c, &g("original")).unwrap(), before);
    usage(&c, "original", &[]);
    c.add_universe_poly_declaration("assumption", vec![], ty.clone())
        .unwrap();
    assert!(c
        .declare_theorem_checked("assumption", vec![], ty.clone(), body.clone())
        .is_err());
    checks(&c, g("assumption"), ty);
    usage(&c, "assumption", &["assumption"]);
    assert!(c.get_definition_body("assumption").is_none());
    assert!(c.get_universe_poly("assumption").is_none());
}

#[test]
fn genuine_axioms_remain_in_transitive_theorem_and_opaque_audits() {
    let mut c = context();
    let (ty, _) = reflexive();
    c.add_declaration("true_assumption", ty.clone());
    c.declare_theorem_checked("first", vec![], ty.clone(), g("true_assumption"))
        .unwrap();
    c.declare_theorem_checked("second", vec![], ty.clone(), g("first"))
        .unwrap();
    c.declare_opaque_checked("sealed", vec![], ty.clone(), g("second"))
        .unwrap();
    for name in ["first", "second", "sealed"] {
        checks(&c, g(name), ty.clone());
        usage(&c, name, &["true_assumption"]);
    }
}

#[test]
fn pending_dependency_and_pending_name_are_not_completed_by_theorem_publication() {
    let mut c = context();
    let (ty, body) = reflexive();
    c.add_definition("waiting".into(), ty.clone(), g("future_proof"));
    let before = c.pending_obligations().len();
    assert!(before > 0, "fixture must contain a real pending obligation");
    assert!(c
        .declare_theorem_checked("dependent", vec![], ty.clone(), g("waiting"))
        .is_err());
    absent(&c, "dependent");
    assert!(c
        .declare_theorem_checked("waiting", vec![], ty, body)
        .is_err());
    assert_eq!(c.pending_obligations().len(), before);
}

#[test]
fn active_simp_rules_refuse_theorem_publication_atomically() {
    let mut c = context();
    let lhs = apps(g("Nat_add"), vec![g("Zero"), g("Zero")]);
    let ty = eq(g("Nat"), lhs, g("Zero"));
    let body = refl(g("Nat"), g("Zero"));
    c.declare("zero_add_rule", ty.clone(), body.clone())
        .unwrap();
    c.add_simp_lemma("zero_add_rule").unwrap();
    assert!(c
        .declare_theorem_checked("simp_refused", vec![], ty, body)
        .is_err());
    absent(&c, "simp_refused");
    assert!(c.pending_obligations().is_empty());
}

#[test]
fn theorem_proposition_sort_uses_universe_equivalence() {
    let mut c = Context::new();
    let equivalent_prop = Universe::Max(Box::new(Universe::Prop), Box::new(Universe::Prop));
    assert!(equivalent_prop.equiv(&Universe::Prop));
    c.add_universe_poly_declaration("P", vec![], Term::Sort(equivalent_prop))
        .unwrap();
    c.add_universe_poly_declaration("p", vec![], g("P"))
        .unwrap();
    checks(&c, g("p"), g("P"));
    c.declare_theorem_checked("equivalent_sort", vec![], g("P"), g("p"))
        .expect("a proposition's equivalent universe spelling must not change admission");
    checks(&c, g("equivalent_sort"), g("P"));
    assert_eq!(
        normalize_checked(&c, &g("equivalent_sort")).unwrap(),
        g("p")
    );
}
