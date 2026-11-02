//! Additive universe-gate controls; original T03-022 remains unchanged.
use logicaffeine_kernel::{infer_type, Context, KernelError, MutualInductive, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn let_type(value: Term, level: u32) -> Term {
    Term::Let {
        name: "C".into(),
        ty: Box::new(sort(level)),
        value: Box::new(value),
        body: Box::new(v("C")),
    }
}
fn ctx() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c
}
fn family(c: &mut Context, name: &str, n: u32) {
    c.add_inductive(name, sort(n));
}
fn store_type(name: &str, n: u32) -> Term {
    pi("stored_type", sort(n), g(name))
}
fn alias(c: &mut Context, name: &str, body: Term) -> Term {
    let ty = infer_type(c, &body).expect("alias body must first be a well-typed type");
    c.declare(name, ty, body)
        .expect("only the checked definition gateway publishes an alias");
    assert!(c.get_definition_body(name).is_some());
    g(name)
}
fn rejected(c: &mut Context, name: &str, parent: &str, ty: Term) {
    let previous: Vec<_> = c
        .get_constructors(parent)
        .iter()
        .map(|(n, t)| ((*n).to_string(), (*t).clone()))
        .collect();
    assert!(c.get_global(name).is_none());
    let result = c.add_constructor_checked(name, parent, ty);
    match result {
        Err(KernelError::InvalidDeclaration(message)) => {
            assert!(message.contains("universe inconsistency"), "{message}");
            assert!(
                message.contains(name) && message.contains(parent),
                "{message}"
            );
        }
        other => panic!("expected a decided universe rejection for {name}, got {other:?}"),
    }
    assert!(c.get_global(name).is_none() && c.get_definition_body(name).is_none());
    let after: Vec<_> = c
        .get_constructors(parent)
        .iter()
        .map(|(n, t)| ((*n).to_string(), (*t).clone()))
        .collect();
    assert_eq!(
        after, previous,
        "refusal must preserve constructor metadata"
    );
}
fn accepted(c: &mut Context, name: &str, parent: &str, ty: Term) {
    c.add_constructor_checked(name, parent, ty)
        .expect("legal neighbour must register");
    assert!(infer_type(c, &g(name)).is_ok());
    assert!(c.get_constructors(parent).iter().any(|(n, _)| *n == name));
}

#[test]
fn direct_oversized_fields_are_decided_rejections_at_three_levels() {
    for n in 0..3 {
        let mut c = ctx();
        family(&mut c, "Bad", n);
        rejected(&mut c, "bad", "Bad", store_type("Bad", n));
    }
}
#[test]
fn checked_alias_cannot_hide_an_oversized_first_field() {
    for n in 0..3 {
        let mut c = ctx();
        family(&mut c, "Bad", n);
        let ty = alias(&mut c, "CtorType", store_type("Bad", n));
        rejected(&mut c, "bad", "Bad", ty);
    }
}
#[test]
fn beta_exposure_cannot_hide_an_oversized_first_field() {
    let mut c = ctx();
    family(&mut c, "Bad", 0);
    let ty = app(lam("C", sort(1), v("C")), store_type("Bad", 0));
    assert_eq!(infer_type(&c, &ty).unwrap(), sort(1));
    rejected(&mut c, "bad", "Bad", ty);
}
#[test]
fn let_exposure_cannot_hide_an_oversized_first_field() {
    let mut c = ctx();
    family(&mut c, "Bad", 0);
    let ty = let_type(store_type("Bad", 0), 1);
    assert_eq!(infer_type(&c, &ty).unwrap(), sort(1));
    rejected(&mut c, "bad", "Bad", ty);
}
#[test]
fn a_telescope_tail_alias_cannot_end_the_field_walk_early() {
    let mut c = ctx();
    family(&mut c, "Token", 0);
    c.add_constructor("token", "Token", g("Token"));
    family(&mut c, "Bad", 0);
    let tail = alias(&mut c, "Tail", store_type("Bad", 0));
    rejected(&mut c, "bad", "Bad", pi("first", g("Token"), tail));
}
#[test]
fn legal_higher_sort_peers_accept_direct_alias_beta_and_let_types() {
    for n in 0..3 {
        for form in 0..4 {
            let mut c = ctx();
            family(&mut c, "Good", n + 1);
            let raw = store_type("Good", n);
            let ty = match form {
                0 => raw,
                1 => alias(&mut c, "CtorType", raw),
                2 => app(lam("C", sort(n + 1), v("C")), raw),
                _ => let_type(raw, n + 1),
            };
            accepted(&mut c, "good", "Good", ty);
        }
    }
}
#[test]
fn parameter_exemption_does_not_exempt_an_alias_hidden_stored_type() {
    let mut c = ctx();
    c.add_indexed_inductive("Box", pi("A", sort(0), sort(0)), 1);
    // The alias field B deliberately collides with the caller's parameter name.
    let body = lam(
        "A",
        sort(0),
        pi("B", v("A"), pi("huge", sort(0), app(g("Box"), v("A")))),
    );
    let tail = alias(&mut c, "Tail", body);
    rejected(&mut c, "bad", "Box", pi("B", sort(0), app(tail, v("B"))));
}
#[test]
fn legal_parameter_tail_alias_preserves_lexical_scope() {
    let mut c = ctx();
    c.add_indexed_inductive("Box", pi("A", sort(0), sort(0)), 1);
    let tail = alias(
        &mut c,
        "Tail",
        lam("A", sort(0), pi("B", v("A"), app(g("Box"), v("A")))),
    );
    accepted(&mut c, "box", "Box", pi("B", sort(0), app(tail, v("B"))));
}
#[test]
fn rejection_preserves_an_existing_constructor_and_alias() {
    // `old` carries a field: a FIELDLESS one-constructor `Bad` would make every value of it
    // convertible (unit η), so extending it under the alias would be a change of meaning the
    // stable-meaning gate refuses before the universe check is reached.
    let mut c = ctx();
    family(&mut c, "Bad", 0);
    c.add_constructor_checked("old", "Bad", pi("previous", g("Bad"), g("Bad")))
        .unwrap();
    let body = store_type("Bad", 0);
    let ty = alias(&mut c, "CtorType", body.clone());
    rejected(&mut c, "bad", "Bad", ty);
    assert_eq!(c.get_definition_body("CtorType"), Some(&body));
    assert!(c.get_global("old").is_some());
}
#[test]
fn raw_registration_is_distinct_from_the_checked_universe_gate() {
    let mut raw = ctx();
    family(&mut raw, "Bad", 0);
    raw.add_constructor("raw", "Bad", store_type("Bad", 0));
    assert!(
        raw.get_global("raw").is_some(),
        "explicitly unchecked metadata fixture"
    );
    let mut checked = ctx();
    family(&mut checked, "Bad", 0);
    rejected(&mut checked, "checked", "Bad", store_type("Bad", 0));
}
#[test]
fn mutual_block_rejects_a_let_hidden_bad_field_transactionally() {
    let mut c = ctx();
    let block = [
        MutualInductive {
            name: "MA".into(),
            sort: sort(0),
            num_params: 0,
            constructors: vec![(
                "ma".into(),
                let_type(pi("huge", sort(0), pi("b", g("MB"), g("MA"))), 1),
            )],
        },
        MutualInductive {
            name: "MB".into(),
            sort: sort(0),
            num_params: 0,
            constructors: vec![("mb".into(), g("MB"))],
        },
    ];
    let result = c.add_mutual_inductives(&block);
    assert!(
        matches!(result, Err(KernelError::InvalidDeclaration(_))),
        "{result:?}"
    );
    for n in ["MA", "MB", "ma", "mb"] {
        assert!(c.get_global(n).is_none());
    }
    assert!(!c.is_inductive("MA") && !c.is_inductive("MB"));
}
#[test]
fn prop_exemption_accepts_a_checked_alias_with_high_fields() {
    let mut c = ctx();
    c.add_inductive("P", Term::Sort(Universe::Prop));
    let ty = alias(
        &mut c,
        "CtorType",
        pi("A", sort(2), pi("a", v("A"), g("P"))),
    );
    accepted(&mut c, "p", "P", ty);
}

fn wrapped_sort(c: &mut Context, n: u32, form: usize) -> Term {
    match form {
        0 => alias(c, "ResultSort", sort(n)),
        1 => app(lam("S", sort(n + 1), v("S")), sort(n)),
        _ => let_type(sort(n), n + 1),
    }
}
fn checked_block_result(
    c: &mut Context,
    name: &str,
    header: Term,
    nparams: usize,
    ty: Term,
    bad: bool,
) {
    let ctor = format!("{name}_mk");
    let block = [MutualInductive {
        name: name.into(),
        sort: header,
        num_params: nparams,
        constructors: vec![(ctor.clone(), ty)],
    }];
    let result = c.add_mutual_inductives(&block);
    if bad {
        match result {
            Err(KernelError::InvalidDeclaration(message)) => {
                assert!(message.contains("universe inconsistency"), "{message}");
                assert!(
                    message.contains(name) && message.contains(&ctor),
                    "{message}"
                );
            }
            other => panic!("expected checked target-sort rejection, got {other:?}"),
        }
        assert!(!c.is_inductive(name) && c.get_global(name).is_none());
        assert!(c.get_global(&ctor).is_none() && c.get_constructors(name).is_empty());
    } else {
        result.expect("legal checked target-sort neighbour");
        assert!(c.is_inductive(name) && infer_type(c, &g(&ctor)).is_ok());
    }
}
#[test]
fn a_checked_target_sort_alias_cannot_disable_the_universe_gate() {
    let mut c = ctx();
    let header = wrapped_sort(&mut c, 0, 0);
    assert_eq!(infer_type(&c, &header).unwrap(), sort(1));
    checked_block_result(&mut c, "Bad", header, 0, store_type("Bad", 0), true);
    assert!(
        c.get_definition_body("ResultSort").is_some(),
        "prior checked alias is unchanged"
    );
}
#[test]
fn checked_beta_and_let_target_sorts_cannot_disable_the_gate() {
    for form in 1..3 {
        let mut c = ctx();
        let header = wrapped_sort(&mut c, 0, form);
        checked_block_result(&mut c, "Bad", header, 0, store_type("Bad", 0), true);
    }
}
#[test]
fn checked_parameterized_target_telescope_exposes_its_result_sort() {
    for form in 0..3 {
        let mut c = ctx();
        let target = wrapped_sort(&mut c, 0, form);
        let header = pi("A", sort(0), target);
        let ty = pi(
            "B",
            sort(0),
            pi("x", v("B"), pi("huge", sort(0), app(g("Box"), v("B")))),
        );
        checked_block_result(&mut c, "Box", header, 1, ty, true);
    }
}
#[test]
fn checked_target_sort_aliases_preserve_legal_higher_level_neighbors() {
    for n in 0..3 {
        for form in 0..3 {
            let mut c = ctx();
            let target = wrapped_sort(&mut c, n + 1, form);
            checked_block_result(&mut c, "Good", target, 0, store_type("Good", n), false);
        }
    }
    for form in 0..3 {
        let mut c = ctx();
        let target = wrapped_sort(&mut c, 1, form);
        let header = pi("A", sort(0), target);
        let ty = pi(
            "B",
            sort(0),
            pi("x", v("B"), pi("large", sort(0), app(g("Box"), v("B")))),
        );
        checked_block_result(&mut c, "Box", header, 1, ty, false);
    }
}
