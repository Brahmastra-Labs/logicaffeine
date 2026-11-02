//! Immutable checked declaration payloads survive copy-on-write map updates.
//! Every address comparison borrows from both live snapshots AFTER publication.
use logicaffeine_kernel::{
    axiom_dependencies_of, infer_type, normalize_checked, recheck, Context, Term, Universe,
};
use std::{collections::BTreeSet, ptr};

fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
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
fn checks(c: &Context, body: Term, ty: Term) {
    let annotated = Term::Let {
        name: "checked_payload".into(),
        ty: Box::new(ty),
        value: Box::new(body),
        body: Box::new(v("checked_payload")),
    };
    infer_type(c, &annotated).expect("main checks actual declared type");
    recheck(c, &annotated).expect("independent checks actual declared type");
}
fn proof_identity() -> (Term, Term) {
    (
        pi("P", prop(), pi("p", v("P"), v("P"))),
        lam("P", prop(), lam("p", v("P"), v("p"))),
    )
}
fn poly_identity() -> (Term, Term) {
    let u = Term::Sort(Universe::Var("u".into()));
    (
        pi("A", u.clone(), pi("x", v("A"), v("A"))),
        lam("A", u, lam("x", v("A"), v("x"))),
    )
}
fn wide_body(depth: usize) -> Term {
    if depth == 0 {
        return prop();
    }
    // A genuinely typed branching application tree, not a malformed wide node.
    let first = lam("left", sort(0), lam("right", sort(0), v("left")));
    app(app(first, wide_body(depth - 1)), wide_body(depth - 1))
}
fn deep_body() -> (Term, Term) {
    let mut body = prop();
    let mut ty = sort(0);
    for i in 0..24 {
        let name = format!("argument_{i}");
        body = lam(&name, sort(0), body);
        ty = pi(&name, sort(0), ty);
    }
    (ty, body)
}
fn baseline() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    let (ty, body) = deep_body();
    checks(&c, body.clone(), ty.clone());
    c.declare("deep", ty, body).unwrap();
    let wide = wide_body(4);
    checks(&c, wide.clone(), sort(0));
    c.declare("wide", sort(0), wide).unwrap();
    let (ty, body) = poly_identity();
    c.declare_universe_poly("poly", vec!["unused".into(), "u".into()], ty, body)
        .unwrap();
    let (ty, body) = proof_identity();
    c.declare_theorem_checked("theorem", vec![], ty, body)
        .unwrap();
    assert!(c.pending_obligations().is_empty());
    c
}
fn same_mono(a: &Context, b: &Context, name: &str) {
    assert!(
        ptr::eq(
            a.get_definition_body(name).unwrap(),
            b.get_definition_body(name).unwrap()
        ),
        "existing body {name} must not be recursively cloned"
    );
    assert!(
        ptr::eq(
            a.get_definition_type(name).unwrap(),
            b.get_definition_type(name).unwrap()
        ),
        "existing type {name} must retain its allocation"
    );
    assert!(
        ptr::eq(a.get_global(name).unwrap(), b.get_global(name).unwrap()),
        "{name}"
    );
}
fn same_poly(a: &Context, b: &Context, name: &str) {
    let left = a.get_universe_poly(name).unwrap();
    let right = b.get_universe_poly(name).unwrap();
    assert!(
        ptr::eq(left, right),
        "existing polymorphic record {name} must be shared"
    );
    assert!(ptr::eq(&left.1, &right.1), "{name} type");
    assert!(ptr::eq(&left.2, &right.2), "{name} body");
    assert!(ptr::eq(
        a.poly_declaration(name).unwrap().1,
        b.poly_declaration(name).unwrap().1
    ));
}
fn usage(c: &Context, name: &str, expected: &[&str]) {
    let u = axiom_dependencies_of(c, name).unwrap();
    let expected: BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
    assert!(u.unresolved.is_empty(), "{u:?}");
    assert_eq!(u.axioms, expected);
}

#[test]
fn unrelated_monomorphic_publication_preserves_deep_and_wide_payload_addresses() {
    let original = baseline();
    let mut changed = original.clone();
    changed.declare("unrelated", sort(0), prop()).unwrap();
    for name in ["deep", "wide", "theorem"] {
        same_mono(&original, &changed, name);
    }
    assert!(original.get_global("unrelated").is_none());
    checks(&changed, g("wide"), sort(0));
    assert_eq!(normalize_checked(&changed, &g("wide")).unwrap(), prop());
}

#[test]
fn unrelated_polymorphic_publication_preserves_entire_existing_records() {
    let original = baseline();
    let mut changed = original.clone();
    let (ty, body) = poly_identity();
    changed
        .declare_universe_poly("another_poly", vec!["u".into()], ty, body)
        .unwrap();
    same_poly(&original, &changed, "poly");
    same_poly(&original, &changed, "theorem");
    assert!(original.get_universe_poly("another_poly").is_none());
}

#[test]
fn unrelated_theorem_publication_preserves_both_definition_registries() {
    let original = baseline();
    let mut changed = original.clone();
    let (ty, body) = proof_identity();
    changed
        .declare_theorem_checked("another_theorem", vec![], ty, body)
        .unwrap();
    for name in ["deep", "wide", "theorem"] {
        same_mono(&original, &changed, name);
    }
    for name in ["poly", "theorem"] {
        same_poly(&original, &changed, name);
    }
    usage(&original, "theorem", &[]);
    usage(&changed, "another_theorem", &[]);
}

#[test]
fn theorem_global_and_explicit_lookup_borrow_one_actual_type_and_body() {
    let c = baseline();
    let (_, poly_ty, poly_body) = c.get_universe_poly("theorem").unwrap();
    assert!(ptr::eq(c.get_global("theorem").unwrap(), poly_ty));
    assert!(ptr::eq(c.get_definition_type("theorem").unwrap(), poly_ty));
    assert!(ptr::eq(
        c.get_definition_body("theorem").unwrap(),
        poly_body
    ));
    let (ty, _) = proof_identity();
    checks(&c, g("theorem"), ty.clone());
    checks(
        &c,
        Term::Const {
            name: "theorem".into(),
            levels: vec![],
        },
        ty,
    );
}

#[test]
fn replacement_isolated_from_old_snapshot_preserves_old_types_bodies_and_audits() {
    let mut original = baseline();
    let (proof_ty, proof_body) = proof_identity();
    original.add_declaration("assumption", proof_ty.clone());
    original
        .declare("replace_proof", proof_ty.clone(), g("assumption"))
        .unwrap();
    original.declare("replace_type", sort(0), prop()).unwrap();
    let mut changed = original.clone();
    changed
        .declare("replace_proof", proof_ty.clone(), proof_body.clone())
        .unwrap();
    changed.declare("replace_type", sort(1), sort(0)).unwrap();
    assert_eq!(
        original.get_definition_body("replace_proof"),
        Some(&g("assumption"))
    );
    assert_eq!(
        changed.get_definition_body("replace_proof"),
        Some(&proof_body)
    );
    assert_eq!(original.get_definition_type("replace_type"), Some(&sort(0)));
    assert_eq!(changed.get_definition_type("replace_type"), Some(&sort(1)));
    assert_eq!(
        normalize_checked(&original, &g("replace_type")).unwrap(),
        prop()
    );
    assert_eq!(
        normalize_checked(&changed, &g("replace_type")).unwrap(),
        sort(0)
    );
    checks(&original, g("replace_proof"), proof_ty.clone());
    checks(&changed, g("replace_proof"), proof_ty);
    usage(&original, "replace_proof", &["assumption"]);
    usage(&changed, "replace_proof", &[]);
    same_mono(&original, &changed, "deep");
    same_poly(&original, &changed, "poly");
}

#[test]
fn failed_publication_keeps_existing_payloads_and_registry_visibility() {
    let original = baseline();
    let mut changed = original.clone();
    assert!(changed.declare("deep", sort(0), sort(0)).is_err());
    assert!(changed
        .declare_universe_poly("invalid_poly", vec![], sort(0), sort(0))
        .is_err());
    assert!(changed
        .declare_theorem_checked("invalid_theorem", vec![], sort(0), prop())
        .is_err());
    for name in ["deep", "wide", "theorem"] {
        same_mono(&original, &changed, name);
    }
    for name in ["poly", "theorem"] {
        same_poly(&original, &changed, name);
    }
    assert!(changed.get_global("invalid_theorem").is_none());
    assert!(changed.get_universe_poly("invalid_poly").is_none());
    assert!(changed.pending_obligations().is_empty());
    usage(&changed, "theorem", &[]);
}

#[test]
fn resolving_pending_in_one_snapshot_leaves_the_other_pending_and_payloads_shared() {
    let mut original = baseline();
    original.declare("waiting", sort(0), g("later")).unwrap();
    assert_eq!(original.pending_obligations().len(), 1);
    let mut changed = original.clone();
    changed.declare("later", sort(0), prop()).unwrap();
    assert!(changed.pending_obligations().is_empty());
    assert_eq!(original.pending_obligations().len(), 1);
    assert!(original.get_global("waiting").is_none());
    assert!(original.get_definition_body("waiting").is_none());
    assert!(original.get_global("later").is_none());
    checks(&changed, g("waiting"), sort(0));
    assert_eq!(normalize_checked(&changed, &g("waiting")).unwrap(), prop());
    usage(&changed, "waiting", &[]);
    for name in ["deep", "wide", "theorem"] {
        same_mono(&original, &changed, name);
    }
    same_poly(&original, &changed, "poly");
}

#[test]
fn explicit_universe_telescope_order_and_unused_parameters_survive_snapshot_updates() {
    let original = baseline();
    let mut changed = original.clone();
    let (ty, body) = poly_identity();
    changed
        .declare_universe_poly("extra", vec!["u".into()], ty, body)
        .unwrap();
    for c in [&original, &changed] {
        let (params, _, _) = c.get_universe_poly("poly").unwrap();
        assert_eq!(params, &vec!["unused".to_string(), "u".to_string()]);
        let explicit = Term::Const {
            name: "poly".into(),
            levels: vec![Universe::Type(2), Universe::Type(0)],
        };
        let applied = app(app(explicit, sort(0)), prop());
        // A=Type0 requires u=Type1. The unused first level must not
        // accidentally be selected as the carrier universe.
        assert!(infer_type(c, &applied).is_err());
        assert!(recheck(c, &applied).is_err());
        let explicit = Term::Const {
            name: "poly".into(),
            levels: vec![Universe::Type(2), Universe::Type(1)],
        };
        let applied = app(app(explicit, sort(0)), prop());
        checks(c, applied.clone(), sort(0));
        assert_eq!(normalize_checked(c, &applied).unwrap(), prop());
        let wrong_arity = Term::Const {
            name: "poly".into(),
            levels: vec![Universe::Type(1)],
        };
        assert!(infer_type(c, &wrong_arity).is_err());
        assert!(recheck(c, &wrong_arity).is_err());
    }
    same_poly(&original, &changed, "poly");
}

#[test]
fn snapshot_updates_preserve_negative_mono_and_poly_registry_membership() {
    let mut original = baseline();
    // An explicitly polymorphic registration with NO level parameters still
    // belongs to the polymorphic registry; an empty vector is not a mono tag.
    original
        .declare_universe_poly("empty_poly", vec![], sort(0), prop())
        .unwrap();
    let mut changed = original.clone();
    changed.declare("new_mono", sort(0), prop()).unwrap();
    changed
        .declare_universe_poly("new_empty_poly", vec![], sort(0), prop())
        .unwrap();
    let (ty, body) = proof_identity();
    changed
        .declare_theorem_checked("new_theorem", vec![], ty, body)
        .unwrap();
    for c in [&original, &changed] {
        assert!(c.get_universe_poly("wide").is_none());
        assert!(c.poly_declaration("wide").is_none());
        assert!(c.is_definition("wide"));
        assert!(c.iter_definitions().any(|(name, _, _)| name == "wide"));
        checks(c, g("wide"), sort(0));
        let mono_as_const = Term::Const {
            name: "wide".into(),
            levels: vec![],
        };
        assert!(infer_type(c, &mono_as_const).is_err());
        assert!(recheck(c, &mono_as_const).is_err());

        for name in ["poly", "empty_poly"] {
            assert!(c.get_global(name).is_none(), "{name}");
            assert!(c.get_definition_body(name).is_none(), "{name}");
            assert!(c.get_definition_type(name).is_none(), "{name}");
            assert!(!c.is_definition(name), "{name}");
            assert!(
                !c.iter_definitions().any(|(found, _, _)| found == name),
                "{name}"
            );
            assert!(c.get_universe_poly(name).is_some(), "{name}");
            assert!(c.poly_declaration(name).is_some(), "{name}");
        }
        let empty = Term::Const {
            name: "empty_poly".into(),
            levels: vec![],
        };
        checks(c, empty.clone(), sort(0));
        assert_eq!(normalize_checked(c, &empty).unwrap(), prop());
        let poly = Term::Const {
            name: "poly".into(),
            levels: vec![Universe::Type(2), Universe::Type(1)],
        };
        let applied = app(app(poly, sort(0)), prop());
        checks(c, applied.clone(), sort(0));
        assert_eq!(normalize_checked(c, &applied).unwrap(), prop());
    }
}
