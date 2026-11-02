use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn ap(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
#[test]
fn body_application_boxes_and_unaffected_names_survive_opening() {
    let t = ap(g(&"retained".repeat(100)), Db::Var(0));
    let Db::App(f, a) = &t else { panic!() };
    let pointers = (f.as_ref() as *const Db, a.as_ref() as *const Db);
    let Db::Global(name) = f.as_ref() else {
        panic!()
    };
    let storage = name.as_ptr();
    let expected = beta_open(&t, &Db::Var(3));
    let result = beta_open_owned(t, &Db::Var(3));
    assert_eq!(result, expected);
    let Db::App(f, a) = &result else { panic!() };
    assert_eq!((f.as_ref() as *const Db, a.as_ref() as *const Db), pointers);
    let Db::Global(name) = f.as_ref() else {
        panic!()
    };
    assert_eq!(name.as_ptr(), storage);
}
#[test]
fn match_and_mutual_vectors_survive_opening() {
    let t = Db::Match {
        disc: DbBox::new(Db::Var(0)),
        motive: DbBox::new(g("motive")),
        cases: vec![Db::Var(0), Db::Var(1)],
    };
    let Db::Match { cases, .. } = &t else {
        panic!()
    };
    let storage = cases.as_ptr();
    let expected = beta_open(&t, &Db::Var(4));
    let result = beta_open_owned(t, &Db::Var(4));
    assert_eq!(result, expected);
    let Db::Match { cases, .. } = &result else {
        panic!()
    };
    assert_eq!(cases.as_ptr(), storage);
    let t = Db::MutualFix {
        defs: vec![ap(Db::Var(0), Db::Var(2)), ap(Db::Var(1), Db::Var(3))],
        index: 1,
    };
    let Db::MutualFix { defs, .. } = &t else {
        panic!()
    };
    let storage = defs.as_ptr();
    let expected = beta_open(&t, &Db::Var(7));
    let result = beta_open_owned(t, &Db::Var(7));
    assert_eq!(result, expected);
    let Db::MutualFix { defs, .. } = &result else {
        panic!()
    };
    assert_eq!(defs.as_ptr(), storage);
}
#[test]
fn every_binding_form_and_open_argument_matches_borrowed_substitution() {
    let leaves = vec![
        Db::Var(0),
        Db::Var(1),
        Db::Var(2),
        Db::Var(5),
        g("global"),
        Db::Sort(Universe::Type(1)),
        Db::Const {
            name: "poly".into(),
            levels: vec![Universe::Var("u".into())],
        },
        Db::Lit(Literal::Int(7)),
    ];
    let args = [
        Db::Var(0),
        Db::Var(4),
        g("closed"),
        Db::Lam(
            DbBox::new(Db::Var(2)),
            DbBox::new(ap(Db::Var(0), Db::Var(3))),
        ),
    ];
    for a in &leaves {
        for b in &leaves {
            let forms = [
                ap(a.clone(), b.clone()),
                Db::Pi(DbBox::new(a.clone()), DbBox::new(b.clone())),
                Db::Lam(DbBox::new(a.clone()), DbBox::new(b.clone())),
                Db::Fix(FixBody::new(ap(a.clone(), b.clone()))),
                Db::MutualFix {
                    defs: vec![a.clone(), b.clone()],
                    index: 0,
                },
                Db::Let(
                    DbBox::new(a.clone()),
                    DbBox::new(b.clone()),
                    DbBox::new(ap(a.clone(), b.clone())),
                ),
                Db::Match {
                    disc: DbBox::new(a.clone()),
                    motive: DbBox::new(b.clone()),
                    cases: vec![a.clone(), b.clone()],
                },
            ];
            for form in forms {
                for arg in &args {
                    assert_eq!(beta_open_owned(form.clone(), arg), beta_open(&form, arg));
                }
            }
        }
    }
}
#[test]
fn owned_body_never_visits_unused_arguments_or_reconstructs_nodes() {
    let mut arg = g("large");
    for _ in 0..100 {
        arg = ap(arg, Db::Var(0));
    }
    let body = Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(0)));
    let expected = body.clone();
    BETA_WORK.with(|work| work.set(Some((0, 0))));
    let result = beta_open_owned(body, &arg);
    let work = BETA_WORK.with(|work| work.replace(None).unwrap());
    assert_eq!(result, expected);
    assert_eq!(work, (3, 0));
}
#[test]
fn monomorphic_domain_solving_retains_both_owned_terms() {
    let dom = ap(g("TypeHead"), g("argument"));
    let body = g(&"large_codomain".repeat(100));
    let Db::App(f, a) = &dom else { panic!() };
    let pointers = (f.as_ref() as *const Db, a.as_ref() as *const Db);
    let Db::Global(n) = &body else { panic!() };
    let storage = n.as_ptr();
    let (d, b) = solve_domain_levels_owned(&Context::new(), &[], dom, body, &g("unknown"));
    let Db::App(f, a) = &d else { panic!() };
    assert_eq!((f.as_ref() as *const Db, a.as_ref() as *const Db), pointers);
    let Db::Global(n) = &b else { panic!() };
    assert_eq!(n.as_ptr(), storage);
}
#[test]
fn domain_unification_and_failure_fallbacks_match_existing_solver() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let meta = crate::term::fresh_level_metas("test", &["u".into()])["u"].clone();
    let domains = [
        Db::Sort(meta.clone()),
        Db::Sort(Universe::Var("rigid".into())),
        g("Nat"),
    ];
    let body = Db::Pi(DbBox::new(Db::Sort(meta)), DbBox::new(Db::Var(0)));
    let args = [g("Nat"), g("unknown"), Db::Var(0), Db::Lit(Literal::Int(3))];
    for dom in domains {
        for arg in &args {
            assert_eq!(
                solve_domain_levels_owned(&c, &[g("Nat")], dom.clone(), body.clone(), arg),
                solve_domain_levels(&c, &[g("Nat")], &dom, &body, arg)
            );
        }
    }
}
