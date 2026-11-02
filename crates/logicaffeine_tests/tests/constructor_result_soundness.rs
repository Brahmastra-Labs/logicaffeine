use logicaffeine_kernel::{infer_type, Context, MutualInductive, Term, Universe};

fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn env() -> Context {
    let mut c = Context::new();
    c.add_inductive("Never", prop());
    c.add_inductive("Box", ty());
    c
}
#[test]
fn a_constructor_cannot_prove_an_unrelated_empty_proposition() {
    let mut c = env();
    assert!(c
        .add_constructor_checked("escape", "Box", g("Never"))
        .is_err());
    assert!(infer_type(&c, &g("escape")).is_err());
    assert!(c.get_constructors("Box").is_empty());
}
#[test]
fn a_constructor_type_cannot_be_a_sort_instead_of_its_parent() {
    let mut c = env();
    assert!(c.add_constructor_checked("escape", "Box", ty()).is_err());
    assert!(c.get_global("escape").is_none());
}
#[test]
fn a_prop_constructor_still_checks_every_field_is_a_type() {
    let mut c = env();
    c.add_declaration("nonsense", g("Box"));
    assert!(c
        .add_constructor_checked("escape", "Never", pi("x", g("nonsense"), g("Never")))
        .is_err());
    assert!(c.get_global("escape").is_none());
}
#[test]
fn a_uniform_constructor_cannot_change_its_result_parameter() {
    let mut c = env();
    c.add_indexed_inductive("Container", pi("A", ty(), ty()), 1);
    let bad = pi("A", ty(), app(g("Container"), g("Box")));
    assert!(c
        .add_constructor_checked("escape", "Container", bad)
        .is_err());
}
#[test]
fn a_shadowing_field_cannot_replace_a_uniform_parameter() {
    let mut c = env();
    c.add_indexed_inductive("Container", pi("A", ty(), ty()), 1);
    let bad = pi("A", ty(), pi("A", ty(), app(g("Container"), v("A"))));
    assert!(c
        .add_constructor_checked("escape", "Container", bad)
        .is_err());
}
#[test]
fn mutual_registration_cannot_bypass_the_same_false_proof_boundary() {
    let mut c = env();
    let block = vec![MutualInductive {
        name: "Bogus".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![("escape".into(), g("Never"))],
    }];
    assert!(c.add_mutual_inductives(&block).is_err());
    assert!(c.get_global("Bogus").is_none());
    assert!(c.get_global("escape").is_none());
}
#[test]
fn an_inductive_header_cannot_itself_be_a_proof_of_false() {
    let mut c = env();
    let block = vec![MutualInductive {
        name: "escape".into(),
        sort: g("Never"),
        num_params: 0,
        constructors: vec![],
    }];
    assert!(c.add_mutual_inductives(&block).is_err());
    assert!(c.get_global("escape").is_none());
}
#[test]
fn valid_recursive_and_parameterized_constructors_still_register() {
    let mut c = env();
    c.add_constructor_checked("box", "Box", g("Box")).unwrap();
    c.add_constructor_checked("more", "Box", pi("x", g("Box"), g("Box")))
        .unwrap();
    c.add_indexed_inductive("Container", pi("A", ty(), ty()), 1);
    c.add_constructor_checked(
        "wrap",
        "Container",
        pi("A", ty(), pi("value", v("A"), app(g("Container"), v("A")))),
    )
    .unwrap();
    assert_eq!(c.get_constructors("Box").len(), 2);
    assert_eq!(c.get_constructors("Container").len(), 1);
}

#[test]
fn the_first_checked_constructor_can_determine_that_an_argument_is_an_index() {
    let mut c = env();
    c.add_constructor_checked("box", "Box", g("Box")).unwrap();
    c.add_inductive("Indexed", pi("b", g("Box"), ty()));
    c.add_constructor_checked("atBox", "Indexed", app(g("Indexed"), g("box")))
        .unwrap();
    assert_eq!(c.inductive_num_params("Indexed"), 0);
}
#[test]
fn newly_inferred_indices_recheck_the_universes_of_earlier_constructors() {
    let mut c = env();
    c.add_inductive("Container", pi("A", ty(), ty()));
    c.add_constructor_checked(
        "universal",
        "Container",
        pi("A", ty(), app(g("Container"), v("A"))),
    )
    .unwrap();
    assert_eq!(c.inductive_num_params("Container"), 1);
    assert!(c
        .add_constructor_checked("specialized", "Container", app(g("Container"), g("Box")))
        .is_err());
    assert_eq!(c.inductive_num_params("Container"), 1);
    assert_eq!(c.get_constructors("Container").len(), 1);
}
#[test]
fn a_type_field_that_shadows_a_parameter_is_not_the_original_parameter() {
    let mut c = env();
    c.add_indexed_inductive(
        "HigherContainer",
        pi("A", ty(), Term::Sort(Universe::Type(1))),
        1,
    );
    let bad = pi("A", ty(), pi("A", ty(), app(g("HigherContainer"), v("A"))));
    assert!(c
        .add_constructor_checked("escape", "HigherContainer", bad)
        .is_err());
}

#[test]
fn inference_can_change_a_parameter_to_an_index_when_every_field_still_fits() {
    let mut c = env();
    c.add_inductive("Higher", pi("A", ty(), Term::Sort(Universe::Type(1))));
    c.add_constructor_checked("general", "Higher", pi("A", ty(), app(g("Higher"), v("A"))))
        .unwrap();
    c.add_constructor_checked("specialized", "Higher", app(g("Higher"), g("Box")))
        .unwrap();
    assert_eq!(c.inductive_num_params("Higher"), 0);
    assert_eq!(c.get_constructors("Higher").len(), 2);
}
