//! T04-017: checked equation lemmas are declaration products with tracked ownership.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, defeq_for_test, double_check, infer_type, is_subtype, Context, DoubleCheck,
    Term, Universe,
};
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(n: &str, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(g(n), app)
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
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn eq(a: Term, b: Term) -> Term {
    apps("Eq", &[g("Nat"), a, b])
}
fn zero_proof() -> Term {
    apps("refl", &[g("Nat"), g("Zero")])
}
fn declare_double(c: &mut Context) {
    let body = Term::Fix {
        name: "rec".into(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![
                    g("Zero"),
                    lam(
                        "k",
                        g("Nat"),
                        app(g("Succ"), app(g("Succ"), app(v("rec"), v("k")))),
                    ),
                ],
            },
        )),
    };
    c.declare("doubled", pi("_", g("Nat"), g("Nat")), body)
        .unwrap();
}
fn checked_lemma(c: &Context, name: &str) {
    let ty = c
        .get_definition_type(name)
        .unwrap_or_else(|| panic!("missing {name}"));
    let body = c.get_definition_body(name).unwrap();
    let inferred = infer_type(c, body).unwrap();
    assert!(is_subtype(c, &inferred, ty), "{name}");
    assert_eq!(double_check(c, body), DoubleCheck::Agreed, "{name}");
    let audit = axiom_dependencies(c, &g(name));
    assert!(
        audit.axioms.is_empty() && audit.unresolved.is_empty(),
        "{name}: {audit:?}"
    );
}
#[test]
fn checked_definitions_automatically_register_real_constructor_equations() {
    let mut c = ctx();
    declare_double(&mut c);
    for n in ["doubled_eq_def", "doubled_eq_1", "doubled_eq_2"] {
        checked_lemma(&c, n);
    }
    assert!(defeq_for_test(
        &c,
        c.get_definition_type("doubled_eq_1").unwrap(),
        &eq(app(g("doubled"), g("Zero")), g("Zero"))
    ));
    assert!(c.refused_declarations().is_empty());
}
#[test]
fn replacing_or_removing_a_source_invalidates_only_its_generated_equations() {
    let mut c = ctx();
    declare_double(&mut c);
    checked_lemma(&c, "doubled_eq_2");
    c.declare(
        "doubled",
        pi("_", g("Nat"), g("Nat")),
        lam("n", g("Nat"), g("Zero")),
    )
    .unwrap();
    assert!(c.get_global("doubled_eq_1").is_none());
    assert!(c.get_global("doubled_eq_2").is_none());
    checked_lemma(&c, "doubled_eq_def");
    c.add_declaration("doubled", pi("_", g("Nat"), g("Nat")));
    assert!(c.get_global("doubled_eq_def").is_none());
}
#[test]
fn user_equations_survive_source_replacement_and_automatic_regeneration() {
    let mut c = ctx();
    c.declare("doubled_eq_1", eq(g("Zero"), g("Zero")), zero_proof())
        .unwrap();
    declare_double(&mut c);
    assert_eq!(c.get_definition_body("doubled_eq_1"), Some(&zero_proof()));
    c.declare("doubled_eq_2", eq(g("Zero"), g("Zero")), zero_proof())
        .unwrap();
    c.declare(
        "doubled",
        pi("_", g("Nat"), g("Nat")),
        lam("n", g("Nat"), g("Zero")),
    )
    .unwrap();
    assert_eq!(c.get_definition_body("doubled_eq_1"), Some(&zero_proof()));
    assert_eq!(c.get_definition_body("doubled_eq_2"), Some(&zero_proof()));
    checked_lemma(&c, "doubled_eq_def");
}
#[test]
fn pending_definitions_get_no_equations_until_their_check_succeeds() {
    let mut c = ctx();
    c.declare("waiting", g("Nat"), g("later")).unwrap();
    logicaffeine_kernel::equations::generate(&mut c, "waiting");
    assert!(c.get_global("waiting_eq_def").is_none());
    c.declare("later", g("Nat"), g("Zero")).unwrap();
    checked_lemma(&c, "waiting_eq_def");
    c.declare("bad_waiting", g("Nat"), g("bad_later")).unwrap();
    assert!(c.get_global("bad_waiting_eq_def").is_none());
    c.declare("bad_later", g("Bool"), g("true")).unwrap();
    assert!(c.get_global("bad_waiting").is_none());
    assert!(c.get_global("bad_waiting_eq_def").is_none());
}
#[test]
fn pre_equality_definitions_receive_checked_equations_after_prelude_bootstrap() {
    // `Nat` is declared whole, as the prelude declares it, before anything depends on it: the
    // prelude's registration of the identical `Nat` then changes nothing, where extending a
    // one-constructor `Nat` under `early` would be a change of meaning the gate refuses.
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor_checked("Zero", "Nat", g("Nat")).unwrap();
    c.add_constructor_checked("Succ", "Nat", pi("n", g("Nat"), g("Nat")))
        .unwrap();
    c.declare("early", g("Nat"), g("Zero")).unwrap();
    assert!(c.get_global("early_eq_def").is_none());
    StandardLibrary::register(&mut c);
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    checked_lemma(&c, "early_eq_def");
}
#[test]
fn sealed_source_replacement_preserves_its_checked_generated_equations() {
    let mut c = ctx();
    c.declare("closed_index", g("Nat"), g("Zero")).unwrap();
    let wrap_ty = pi(
        "A",
        Term::Sort(Universe::Type(0)),
        Term::Sort(Universe::Type(0)),
    );
    c.add_indexed_inductive("ProofWrap", wrap_ty, 1);
    c.add_constructor_checked(
        "ProofWrap_mk",
        "ProofWrap",
        pi(
            "A",
            Term::Sort(Universe::Type(0)),
            pi(
                "h",
                eq(g("closed_index"), g("Zero")),
                pi("x", v("A"), app(g("ProofWrap"), v("A"))),
            ),
        ),
    )
    .unwrap();
    c.add_inductive("WrappedTree", Term::Sort(Universe::Type(0)));
    c.add_constructor_checked(
        "WrappedTree_mk",
        "WrappedTree",
        pi(
            "xs",
            app(g("ProofWrap"), g("WrappedTree")),
            g("WrappedTree"),
        ),
    )
    .unwrap();
    checked_lemma(&c, "closed_index_eq_def");
    let old = c.get_definition_body("closed_index_eq_def").cloned();
    assert!(c
        .declare("closed_index", g("Nat"), app(g("Succ"), g("Zero")))
        .is_err());
    assert_eq!(c.get_definition_body("closed_index_eq_def"), old.as_ref());
    checked_lemma(&c, "closed_index_eq_def");
}

#[test]
fn a_pending_user_equation_reserves_its_name_until_its_proof_arrives() {
    let mut c = ctx();
    c.declare("doubled_eq_1", eq(g("Zero"), g("Zero")), g("later_proof"))
        .unwrap();
    declare_double(&mut c);
    logicaffeine_kernel::equations::generate(&mut c, "doubled");
    assert!(c
        .pending_obligations()
        .iter()
        .any(|(name, _)| *name == "doubled_eq_1"));
    c.declare("later_proof", eq(g("Zero"), g("Zero")), zero_proof())
        .unwrap();
    assert_eq!(
        c.get_definition_body("doubled_eq_1"),
        Some(&g("later_proof"))
    );
    checked_lemma(&c, "doubled_eq_1");
}

#[test]
fn minimal_importer_context_without_global_equality_accumulates_no_auxiliary_obligations() {
    let mut c = Context::new();
    c.add_inductive("EncodedNat", Term::Sort(Universe::Type(0)));
    c.add_constructor_checked("EncodedZero", "EncodedNat", g("EncodedNat"))
        .unwrap();
    for i in 0..128 {
        let name = format!("encoded_definition_{i}");
        c.declare(&name, g("EncodedNat"), g("EncodedZero")).unwrap();
        assert!(c.get_global(&format!("{name}_eq_def")).is_none());
        assert!(c.derived_aux_names(&name).is_none());
    }
    assert!(c.pending_obligations().is_empty());
    assert!(c.refused_declarations().is_empty());
    assert_eq!(
        c.iter_definitions()
            .filter(|(name, _, _)| name.starts_with("encoded_definition_"))
            .count(),
        128
    );
}

#[test]
fn temporary_auxiliary_synthesis_preserves_all_final_decider_equations() {
    let mut c = ctx();
    c.add_inductive("Palette", Term::Sort(Universe::Type(0)));
    c.add_constructor_checked("PaletteRed", "Palette", g("Palette"))
        .unwrap();
    c.add_constructor_checked("PaletteBlue", "Palette", g("Palette"))
        .unwrap();
    assert!(c.is_derived_aux("decEqPalette"));
    let expected = [
        "decEqPalette_eq_def",
        "decEqPalette_eq_1",
        "decEqPalette_eq_2",
    ];
    assert_eq!(
        c.derived_aux_names("decEqPalette").unwrap(),
        expected.map(str::to_string),
    );
    for name in expected {
        checked_lemma(&c, name);
    }
    assert!(c.pending_obligations().is_empty());
    assert!(c.refused_declarations().is_empty());
}
