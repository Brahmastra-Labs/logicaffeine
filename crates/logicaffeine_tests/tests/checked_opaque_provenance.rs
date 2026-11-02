//! Checked opaque publication keeps proof dependencies without exposing a body.
use logicaffeine_kernel::Reducibility;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, infer_type, normalize_checked, recheck, Context,
    Term, Universe,
};
use std::collections::BTreeSet;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
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
fn names(ns: &[&str]) -> BTreeSet<String> {
    ns.iter().map(|s| s.to_string()).collect()
}
fn context() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("P", Term::Sort(Universe::Prop));
    c.add_constructor("p", "P", g("P"));
    c
}
fn publish(c: &mut Context, n: &str, ty: Term, b: Term) {
    c.declare_opaque_checked(n, vec![], ty, b).unwrap();
}
fn usage(c: &Context, n: &str, expected: &[&str]) {
    let named = axiom_dependencies_of(c, n).unwrap();
    assert!(named.unresolved.is_empty());
    assert_eq!(named.axioms, names(expected));
}
fn absent(c: &Context, n: &str) {
    assert!(c.get_global(n).is_none());
    assert!(c.poly_declaration(n).is_none());
    assert!(c.get_definition_body(n).is_none());
    assert!(axiom_dependencies_of(c, n).is_none());
}

#[test]
fn checked_theorem_is_opaque_but_has_no_invented_axiom() {
    let mut c = context();
    publish(&mut c, "proved", g("P"), g("p"));
    assert_eq!(infer_type(&c, &g("proved")).unwrap(), g("P"));
    assert_eq!(recheck(&c, &g("proved")).unwrap(), g("P"));
    assert!(c.get_definition_body("proved").is_none());
    assert!(c.get_universe_poly("proved").is_none());
    assert_eq!(normalize_checked(&c, &g("proved")).unwrap(), g("proved"));
    let explicit = Term::Const {
        name: "proved".into(),
        levels: vec![],
    };
    assert_eq!(infer_type(&c, &explicit).unwrap(), g("P"));
    assert_eq!(recheck(&c, &explicit).unwrap(), g("P"));
    assert_eq!(normalize_checked(&c, &explicit).unwrap(), explicit);
    usage(&c, "proved", &[]);
    assert!(axiom_dependencies(&c, &g("proved")).axioms.is_empty());
}
#[test]
fn opaque_data_and_polymorphic_proofs_keep_exact_headers() {
    let mut c = context();
    publish(
        &mut c,
        "data",
        Term::Sort(Universe::Type(0)),
        Term::Sort(Universe::Prop),
    );
    assert_eq!(normalize_checked(&c, &g("data")).unwrap(), g("data"));
    usage(&c, "data", &[]);
    let sort = Term::Sort(Universe::Var("u".into()));
    let ty = pi("A", sort.clone(), pi("x", v("A"), v("A")));
    let body = lam("A", sort, lam("x", v("A"), v("x")));
    c.declare_opaque_checked("id", vec!["u".into()], ty, body)
        .unwrap();
    let good = Term::Const {
        name: "id".into(),
        levels: vec![Universe::Type(0)],
    };
    assert!(infer_type(&c, &good).is_ok());
    assert!(recheck(&c, &good).is_ok());
    assert_eq!(normalize_checked(&c, &good).unwrap(), good);
    usage(&c, "id", &[]);
    for levels in [vec![], vec![Universe::Type(0), Universe::Type(0)]] {
        let bad = Term::Const {
            name: "id".into(),
            levels,
        };
        assert!(infer_type(&c, &bad).is_err());
        assert!(recheck(&c, &bad).is_err());
    }
}
#[test]
fn ordinary_headers_cannot_forge_checked_provenance() {
    let mut c = context();
    c.add_declaration("raw", g("P"));
    c.add_universe_poly_declaration("raw_poly", vec![], g("P"))
        .unwrap();
    usage(&c, "raw", &["raw"]);
    usage(&c, "raw_poly", &["raw_poly"]);
    publish(&mut c, "from_raw", g("P"), g("raw"));
    usage(&c, "from_raw", &["raw"]);
}
#[test]
fn true_axioms_survive_transparent_and_checked_opaque_transitives() {
    let mut c = context();
    c.add_declaration("ax", g("P"));
    c.declare("helper", g("P"), g("ax")).unwrap();
    publish(&mut c, "first", g("P"), g("helper"));
    publish(&mut c, "second", g("P"), g("first"));
    usage(&c, "first", &["ax"]);
    usage(&c, "second", &["ax"]);
}
#[test]
fn formerly_axiom_free_intermediate_rebinding_invalidates_all_dependents() {
    let mut c = context();
    c.declare("helper", g("P"), g("p")).unwrap();
    publish(&mut c, "first", g("P"), g("helper"));
    publish(&mut c, "second", g("P"), g("first"));
    let mut changed = c.clone();
    changed.add_declaration("helper", g("P"));
    usage(&changed, "first", &["first"]);
    usage(&changed, "second", &["second"]);
    usage(&c, "first", &[]);
    usage(&c, "second", &[]);
    c.add_declaration("unrelated", g("P"));
    usage(&c, "second", &[]);
}
#[test]
fn checked_opaque_name_replacement_cannot_reuse_provenance() {
    let mut c = context();
    publish(&mut c, "first", g("P"), g("p"));
    publish(&mut c, "second", g("P"), g("first"));
    c.add_declaration("first", g("P"));
    usage(&c, "first", &["first"]);
    usage(&c, "second", &["second"]);
}
#[test]
fn an_axiom_header_the_certificate_rests_on_cannot_be_re_signed() {
    // Re-signing `A` under `x` and `witness` is a change of meaning the stable-meaning gate
    // refuses (recorded, since `add_declaration` cannot answer), so the certificate keeps its
    // evidence. That a certificate is invalidated when a dependency does change is locked in the
    // kernel's own unit tests, with the change permitted
    // (`context::checked_opaque_storage_controls`).
    let mut c = context();
    c.add_declaration("A", Term::Sort(Universe::Type(0)));
    c.add_declaration("x", g("A"));
    publish(&mut c, "witness", g("A"), g("x"));
    usage(&c, "witness", &["A", "x"]);
    c.add_declaration("A", Term::Sort(Universe::Type(1)));
    assert!(
        c.refused_declarations()
            .iter()
            .any(|(n, why)| n == "A" && why.starts_with("Stable meaning violation")),
        "{:?}",
        c.refused_declarations()
    );
    assert_eq!(c.get_global("A"), Some(&Term::Sort(Universe::Type(0))));
    usage(&c, "witness", &["A", "x"]);
}
#[test]
fn constructor_owner_inventory_and_parameter_changes_leave_the_certificate_standing() {
    // Each change is refused by the stable-meaning gate (a new constructor for `P`, `p` moved
    // to `Q`) or is the identical declaration and changes nothing (`P`'s header, its parameter
    // count), so the certificate keeps its evidence in every copy. Invalidation on a change
    // that does happen is locked in the kernel's own unit tests, with the change permitted.
    let refused = |c: &Context, name: &str| {
        assert!(
            c.refused_declarations()
                .iter()
                .any(|(n, why)| n == name && why.starts_with("Stable meaning violation")),
            "{name}: {:?}",
            c.refused_declarations()
        )
    };
    let mut original = context();
    publish(&mut original, "proved", g("P"), g("p"));
    let mut inventory = original.clone();
    inventory.add_constructor("q", "P", g("P"));
    refused(&inventory, "q");
    assert_eq!(inventory.get_constructors("P").len(), 1);
    usage(&inventory, "proved", &[]);
    let mut owner = original.clone();
    owner.add_inductive("P", Term::Sort(Universe::Prop));
    assert!(owner.refused_declarations().is_empty());
    usage(&owner, "proved", &[]);
    let mut parameters = original.clone();
    parameters.set_inductive_params("P", 0);
    assert!(parameters.refused_declarations().is_empty());
    usage(&parameters, "proved", &[]);
    let mut remapped = original.clone();
    remapped.add_inductive("Q", Term::Sort(Universe::Prop));
    remapped.add_constructor("p", "Q", g("Q"));
    refused(&remapped, "p");
    assert_eq!(remapped.constructor_inductive("p"), Some("P"));
    usage(&remapped, "proved", &[]);
    usage(&original, "proved", &[]);
}
#[test]
fn closedness_typing_and_universe_failures_publish_nothing() {
    let mut c = context();
    c.add("local", g("P"));
    for (name, params, ty, body) in [
        ("bad_body", vec![], g("P"), Term::Sort(Universe::Type(0))),
        ("bad_type", vec![], g("p"), g("p")),
        ("open", vec![], g("P"), v("local")),
        ("hole", vec![], g("P"), Term::Hole),
        ("missing", vec![], g("P"), g("not_declared")),
        (
            "levels",
            vec![],
            Term::Sort(Universe::Var("u".into())),
            Term::Sort(Universe::Prop),
        ),
        (
            "duplicate_levels",
            vec!["u".into(), "u".into()],
            g("P"),
            g("p"),
        ),
    ] {
        assert!(
            c.declare_opaque_checked(name, params, ty, body).is_err(),
            "{name}"
        );
        absent(&c, name);
        assert!(c.pending_obligations().is_empty());
    }
    c.add_definition("waiting".into(), g("P"), g("later"));
    let before = c.pending_obligations().len();
    assert!(c
        .declare_opaque_checked("pending", vec![], g("P"), g("waiting"))
        .is_err());
    absent(&c, "pending");
    assert_eq!(c.pending_obligations().len(), before);
}
#[test]
fn duplicate_headers_are_rejected_without_destroying_old_evidence() {
    let mut c = context();
    publish(&mut c, "proved", g("P"), g("p"));
    for params in [vec![], vec!["u".into()]] {
        assert!(c
            .declare_opaque_checked("proved", params, g("P"), g("p"))
            .is_err());
        usage(&c, "proved", &[]);
    }
    c.add_universe_poly_declaration("old", vec![], g("P"))
        .unwrap();
    assert!(c
        .declare_opaque_checked("old", vec![], g("P"), g("p"))
        .is_err());
    usage(&c, "old", &["old"]);
}
#[test]
fn scope_exit_keeps_global_proofs_and_clone_changes_are_isolated() {
    let mut c = context();
    c.push_scope();
    publish(&mut c, "proved", g("P"), g("p"));
    c.pop_scope();
    usage(&c, "proved", &[]);
    let mut changed = c.clone();
    changed.push_scope();
    changed.set_reducibility("P", Reducibility::Irreducible);
    usage(&changed, "proved", &[]);
    changed.pop_scope();
    usage(&changed, "proved", &[]);
    usage(&c, "proved", &[]);
}
#[test]
fn active_simp_rule_refuses_checked_opaque_publication_atomically() {
    let mut c = Context::new();
    logicaffeine_kernel::prelude::StandardLibrary::register(&mut c);
    let lhs = app(app(g("Nat_add"), g("Zero")), g("Zero"));
    let ty = app(app(app(g("Eq"), g("Nat")), lhs), g("Zero"));
    let body = app(app(g("refl"), g("Nat")), g("Zero"));
    c.declare("zero_add", ty.clone(), body.clone()).unwrap();
    c.add_simp_lemma("zero_add").unwrap();
    assert!(c
        .declare_opaque_checked("refused", vec![], ty, body)
        .is_err());
    absent(&c, "refused");
}
