use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn ap(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", Term::Global("Nat".into()));
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(Term::Global("Nat".into())),
        },
    );
    c
}
fn result(step: WhnfStep) -> (bool, Db) {
    match step {
        WhnfStep::Reduce(t) => (true, t),
        WhnfStep::Ready(t) => (false, t),
        _ => panic!("unexpected demand"),
    }
}
#[test]
fn field_application_boxes_and_selected_branch_storage_are_retained() {
    let c = context();
    let d = ap(ap(g("Succ"), g("first")), g("second"));
    let Db::App(f, a) = &d else { panic!() };
    let outer = (f.as_ref() as *const Db, a.as_ref() as *const Db);
    let Db::App(f, a) = f.as_ref() else { panic!() };
    let inner = (f.as_ref() as *const Db, a.as_ref() as *const Db);
    let branch = g(&"selected".repeat(100));
    let Db::Global(name) = &branch else { panic!() };
    let storage = name.as_ptr();
    let (reduced, r) = result(resume_match(
        &c,
        &[],
        d,
        DbBox::new(g("Nat")),
        vec![g("unused"), branch],
    ));
    assert!(reduced);
    assert_eq!(
        r,
        ap(ap(g(&"selected".repeat(100)), g("first")), g("second"))
    );
    let Db::App(f, a) = &r else { panic!() };
    assert_eq!((f.as_ref() as *const Db, a.as_ref() as *const Db), outer);
    let Db::App(f, a) = f.as_ref() else { panic!() };
    assert_eq!((f.as_ref() as *const Db, a.as_ref() as *const Db), inner);
    let Db::Global(name) = f.as_ref() else {
        panic!()
    };
    assert_eq!(name.as_ptr(), storage);
}
#[test]
fn nullary_case_moves_selected_branch_without_cloning() {
    let c = context();
    let branch = g(&"zero_branch".repeat(100));
    let Db::Global(n) = &branch else { panic!() };
    let ptr = n.as_ptr();
    let (reduced, r) = result(resume_match(
        &c,
        &[],
        g("Zero"),
        DbBox::new(g("Nat")),
        vec![branch, g("unused")],
    ));
    assert!(reduced);
    let Db::Global(n) = r else { panic!() };
    assert_eq!(n.as_ptr(), ptr);
}
#[test]
fn parameter_erasure_partial_spines_and_explicit_levels_match_reference() {
    let mut c = context();
    c.set_inductive_params("Nat", 1);
    let heads = [
        g("Zero"),
        g("Succ"),
        Db::Const {
            name: "Succ".into(),
            levels: vec![Universe::Type(3)],
        },
        g("unknown"),
        Db::Var(0),
    ];
    for h in heads {
        for fields in 0..4 {
            for count in 0..4 {
                let d = (0..fields).fold(h.clone(), |d, i| ap(d, g(&format!("field{i}"))));
                let cases = (0..count)
                    .map(|i| g(&format!("case{i}")))
                    .collect::<Vec<_>>();
                let expected = result(reference_resume_match(
                    &c,
                    &[],
                    d.clone(),
                    DbBox::new(g("Nat")),
                    cases.clone(),
                ));
                assert_eq!(
                    result(resume_match(&c, &[], d, DbBox::new(g("Nat")), cases)),
                    expected
                );
            }
        }
    }
}
#[test]
fn literal_bridges_and_reversed_registration_order_match_reference() {
    let mut c = context();
    c.add_inductive("Int", Term::Sort(Universe::Type(0)));
    crate::int_inductive::register_int_constructors(&mut c).unwrap();
    let mut reversed = Context::new();
    reversed.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    reversed.add_constructor("Succ", "Nat", Term::Global("Nat".into()));
    reversed.add_constructor("Zero", "Nat", Term::Global("Nat".into()));
    for c in [&c, &reversed, &Context::new()] {
        for n in [-7, 0, 1, 27] {
            for literal in [
                Literal::Nat(crate::BigInt::from_i64(n)),
                Literal::Int(n),
                Literal::BigInt(crate::BigInt::from_i64(n)),
            ] {
                let d = Db::Lit(literal);
                let cases = vec![g("first"), g("second")];
                let expected = result(reference_resume_match(
                    c,
                    &[],
                    d.clone(),
                    DbBox::new(g("Nat")),
                    cases.clone(),
                ));
                assert_eq!(
                    result(resume_match(c, &[], d, DbBox::new(g("Nat")), cases)),
                    expected
                );
            }
        }
    }
}
#[test]
fn neutral_singleton_probe_and_ill_typed_cases_keep_existing_behavior() {
    let c = context();
    for count in 0..3 {
        let cases = vec![Db::Sort(Universe::Type(2)); count];
        let d = Db::Var(0);
        let expected = result(reference_resume_match(
            &c,
            &[g("Nat")],
            d.clone(),
            DbBox::new(g("Nat")),
            cases.clone(),
        ));
        assert_eq!(
            result(resume_match(
                &c,
                &[g("Nat")],
                d,
                DbBox::new(g("Nat")),
                cases
            )),
            expected
        );
        let term = Db::Match {
            disc: DbBox::new(g("Zero")),
            motive: DbBox::new(Db::Lam(DbBox::new(g("Nat")), DbBox::new(g("Nat")))),
            cases: vec![Db::Sort(Universe::Type(2)); count],
        };
        assert!(infer(&c, &[], &term).is_err());
    }
}
