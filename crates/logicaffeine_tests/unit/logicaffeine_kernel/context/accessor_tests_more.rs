//! Second batch of `Context` accessor coverage: inductive/constructor/definition queries and
//! the elaboration metadata registries (implicit args, instances, coercions, binder kinds,
//! universe-poly) — all simple setter/getter pairs the integration suite never pins directly.
use super::*;

fn sort0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}

#[test]
fn inductive_constructor_and_definition_accessors_are_faithful() {
    let mut c = Context::new();
    c.add_inductive("Nat", sort0());
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", arrow(g("Nat"), g("Nat")));
    c.add_definition(
        "two".to_string(),
        g("Nat"),
        Term::App(Box::new(g("Succ")), Box::new(g("Zero"))),
    );
    c.add_definition("bad".to_string(), g("Nat"), g("Succ"));

    assert!(c.is_inductive("Nat"));
    assert!(!c.is_inductive("Zero"));
    assert!(!c.is_inductive("missing"));
    assert!(c.is_constructor("Zero") && c.is_constructor("Succ"));
    assert!(!c.is_constructor("Nat"));
    assert_eq!(c.constructor_inductive("Zero"), Some("Nat"));
    assert_eq!(c.constructor_inductive("Nat"), None);

    assert_eq!(
        c.get_definition_body("two"),
        Some(&Term::App(Box::new(g("Succ")), Box::new(g("Zero"))))
    );
    assert_eq!(c.get_definition_type("two"), Some(&g("Nat")));
    assert_eq!(
        c.get_definition_body("bad"),
        None,
        "an ill-typed body is refused"
    );
    assert!(!c.is_definition("bad"));
    assert!(c.refused_declarations().iter().any(|(n, _)| n == "bad"));
    assert_eq!(
        c.get_definition_body("Nat"),
        None,
        "an inductive has no definition body"
    );

    let inds: Vec<&str> = c.iter_inductives().map(|(n, _)| n).collect();
    assert!(inds.contains(&"Nat"));
}

#[test]
fn indexed_inductive_param_and_index_counts() {
    let mut c = Context::new();
    c.add_inductive("Nat", sort0());
    // Vec : Type0 → Nat → Type0, 1 declared parameter (Type0) ⇒ 1 index (Nat).
    c.add_indexed_inductive("Vec", arrow(sort0(), arrow(g("Nat"), sort0())), 1);
    assert_eq!(c.inductive_declared_params("Vec"), Some(1));
    assert_eq!(
        c.inductive_num_indices("Vec"),
        1,
        "arity 2 − 1 param = 1 index"
    );
    assert_eq!(
        c.inductive_declared_params("Nat"),
        None,
        "no declared split"
    );
}

#[test]
fn struct_of_constructor_resolves_the_record() {
    let mut c = Context::new();
    c.add_structure("S", &[("A", sort0())], &[("f", Term::Var("A".to_string()))]);
    let (sname, _info) = c
        .struct_of_constructor("S_mk")
        .expect("S_mk is a struct constructor");
    assert_eq!(sname, "S");
    assert!(c.struct_of_constructor("not_a_ctor").is_none());
    c.add_inductive("Plain", sort0());
    c.add_constructor("plain_mk", "Plain", g("Plain"));
    assert!(
        c.struct_of_constructor("plain_mk").is_none(),
        "a plain inductive's constructor"
    );
}

#[test]
fn elaboration_metadata_setters_and_getters_round_trip() {
    let mut c = Context::new();
    c.set_implicit_args("f", 2);
    assert_eq!(c.implicit_args("f"), 2);
    assert_eq!(c.implicit_args("unset"), 0, "unknown ⇒ 0");

    c.add_instance(g("Inhabited"), g("mkInhabited"));
    assert_eq!(c.instances().len(), 1);
    assert_eq!(c.instances()[0].0, g("Inhabited"));

    c.add_coercion(g("Int"), g("Rat"), g("intToRat"));
    assert_eq!(c.coercions().len(), 1);
    assert_eq!(c.coercions()[0].2, g("intToRat"));

    let kinds: Vec<crate::elaborate::ParamKind> = Vec::new();
    c.set_binder_kinds("gg", kinds);
    assert!(
        c.binder_kinds("gg").is_some(),
        "recorded kinds are retrievable"
    );
    assert!(c.binder_kinds("unset").is_none(), "unrecorded ⇒ None");

    let u = || Universe::Var("u".to_string());
    let ty = Term::Sort(Universe::Succ(Box::new(u())));
    let body = Term::Sort(u());
    c.add_universe_poly("id", vec!["u".to_string()], ty.clone(), body.clone());
    let (params, got_ty, got_body) = c.get_universe_poly("id").expect("registered");
    assert_eq!(params, &vec!["u".to_string()]);
    assert_eq!(got_ty, &ty);
    assert_eq!(got_body, &body);
    assert!(c.get_universe_poly("unset").is_none());
    c.add_universe_poly("wrong", vec!["u".to_string()], Term::Sort(u()), body);
    assert!(
        c.get_universe_poly("wrong").is_none(),
        "`Sort u : Sort u` is refused"
    );
    assert!(c.refused_declarations().iter().any(|(n, _)| n == "wrong"));
}
