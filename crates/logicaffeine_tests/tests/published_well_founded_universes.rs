//! Published accessibility recursion supports higher motives without new axioms.
//! These use the actual prelude globals, never a replacement recursor.
//! Annotated goals and reflexivity witnesses require each checker to do conversion.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, derive_recursor_universe_poly, infer_type, normalize_checked, recheck,
    Context, KernelError, ReCheckError, Term, Universe,
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
fn well_founded() -> Term {
    lam(
        "point",
        g("Bool"),
        bool_match(
            v("point"),
            lam("selected", g("Bool"), acc(v("selected"))),
            accessible_true(),
            accessible_false(),
        ),
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
fn acc_call(head: Term, target: Term, base: Term) -> Term {
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
        head,
        vec![
            g("Bool"),
            relation(),
            lam("point", g("Bool"), lam("proof", acc(v("point")), target)),
            minor,
            g("true"),
            accessible_true(),
        ],
    )
}
fn wf_call(head: Term, target: Term, base: Term) -> Term {
    apps(
        head,
        vec![
            g("Bool"),
            relation(),
            lam("point", g("Bool"), target.clone()),
            well_founded(),
            step(target, base),
            g("true"),
        ],
    )
}

#[test]
fn finite_relation_and_its_nonempty_accessibility_edge_are_checked() {
    let c = context();
    checks(
        &c,
        relation(),
        pi("y", g("Bool"), pi("x", g("Bool"), prop())),
    );
    checks(&c, accessible_false(), acc(g("false")));
    checks(&c, accessible_true(), acc(g("true")));
    checks(
        &c,
        well_founded(),
        apps(g("WellFounded"), vec![g("Bool"), relation()]),
    );
    checks(&c, refl(g("Bool"), g("false")), edge(g("false"), g("true")));
    rejects(&c, refl(g("Bool"), g("true")), edge(g("true"), g("true")));
}

#[test]
fn published_combinators_keep_checked_ordinary_bodies_and_no_axioms() {
    let c = context();
    for name in ["Acc_rec", "WellFounded_fix"] {
        assert!(
            c.is_definition(name),
            "{name} must remain an ordinary definition"
        );
        let ty = c.get_global(name).expect("published type").clone();
        let body = c
            .get_definition_body(name)
            .expect("published checked body")
            .clone();
        checks(&c, body, ty);
        let usage = axiom_dependencies(&c, &g(name));
        assert!(
            usage.axioms.is_empty(),
            "{name} must not introduce an axiom: {usage:?}"
        );
    }
    // The existing polymorphic generator is the implementation substrate;
    // its body is checked here but is NEVER substituted for the published calls.
    let (_, ty, body) =
        derive_recursor_universe_poly(&c, "Acc").expect("polymorphic Acc generator");
    checks(&c, body, ty);
}

#[test]
fn ordinary_global_calls_preserve_prop_and_type_zero_computation() {
    let c = context();
    let proposition = eq(g("Bool"), g("true"), g("true"));
    for (target, base) in [
        (g("Nat"), g("Zero")),
        (proposition, refl(g("Bool"), g("true"))),
    ] {
        computes(
            &c,
            acc_call(g("Acc_rec"), target.clone(), base.clone()),
            target.clone(),
            base.clone(),
        );
        computes(
            &c,
            wf_call(g("WellFounded_fix"), target.clone(), base.clone()),
            target,
            base,
        );
    }
}

#[test]
fn published_acc_rec_computes_types_through_its_recursive_hypothesis() {
    let c = context();
    // A motive returning Type(k) itself has sort Type(k+1).
    for (level, base) in [(0, g("Nat")), (1, sort(0))] {
        computes(
            &c,
            acc_call(g("Acc_rec"), sort(level), base.clone()),
            sort(level),
            base,
        );
    }
}

#[test]
fn published_well_founded_fix_computes_types_through_its_recursive_hypothesis() {
    let c = context();
    for (level, base) in [(0, g("Nat")), (1, sort(0))] {
        computes(
            &c,
            wf_call(g("WellFounded_fix"), sort(level), base.clone()),
            sort(level),
            base,
        );
    }
}

#[test]
fn explicit_motive_levels_keep_both_published_combinators_callable() {
    let c = context();
    // Acc's carrier is currently monomorphic. The new published telescope has
    // exactly ONE level parameter: the motive, not an invented carrier level.
    for (level, base) in [(0, g("Nat")), (1, sort(0))] {
        for name in ["Acc_rec", "WellFounded_fix"] {
            let head = Term::Const {
                name: name.into(),
                levels: vec![Universe::Type(level + 1)],
            };
            let value = if name == "Acc_rec" {
                acc_call(head, sort(level), base.clone())
            } else {
                wf_call(head, sort(level), base.clone())
            };
            computes(&c, value, sort(level), base.clone());
        }
    }
}

#[test]
fn higher_results_and_wrong_steps_are_rejected_by_both_checkers() {
    let c = context();
    for name in ["Acc_rec", "WellFounded_fix"] {
        let call = |target, base| {
            if name == "Acc_rec" {
                acc_call(g(name), target, base)
            } else {
                wf_call(g(name), target, base)
            }
        };
        let computed_nat = call(sort(0), g("Nat"));
        checks(&c, computed_nat.clone(), sort(0));
        checks(&c, g("Zero"), computed_nat.clone());
        rejects(&c, g("true"), computed_nat);
        let computed_sort = call(sort(1), sort(0));
        checks(&c, computed_sort.clone(), sort(1));
        rejects(&c, computed_sort, sort(0)); // Cumulativity must not lower Type1.
        rejects(&c, call(sort(0), g("Zero")), sort(0)); // Invalid original step.
    }
}
