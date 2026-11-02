use super::*;
use crate::{MutualInductive, Universe};

fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn at(name: &str) -> Term {
    Term::Const {
        name: name.into(),
        levels: vec![Universe::Var("u".into())],
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(name: &str, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}
fn input() -> (Context, Elimination) {
    let sort = Term::Sort(Universe::Var("u".into()));
    let family_sort = pi("A", sort.clone(), sort.clone());
    let source = MutualInductive {
        name: "Source".into(),
        sort: family_sort.clone(),
        num_params: 1,
        constructors: vec![(
            "sourceMk".into(),
            pi("A", sort.clone(), app(at("Source"), v("A"))),
        )],
    };
    let mut ctx = Context::new();
    ctx.add_universe_poly_inductives(&["u".into()], std::slice::from_ref(&source))
        .unwrap();
    let auxiliary = MutualInductive {
        name: "OldAux".into(),
        sort: family_sort,
        num_params: 1,
        constructors: vec![(
            "oldAuxMk".into(),
            pi("A", sort.clone(), app(at("OldAux"), v("A"))),
        )],
    };
    (
        ctx,
        Elimination {
            block: vec![source, auxiliary],
            parameters: vec![("A".into(), sort)],
            restoration: vec![
                ("OldAux".into(), app(at("Source"), v("A"))),
                ("oldAuxMk".into(), app(at("sourceMk"), v("A"))),
            ],
        },
    )
}

#[test]
fn fresh_namespace_avoids_names_of_automatically_derived_auxiliaries() {
    let (mut ctx, input) = input();
    let collision = "decEqarena.native.private.0.I0";
    ctx.add_declaration(collision, Term::Sort(Universe::Type(0)));
    let old_source = ctx.get_global("Source").unwrap().clone();
    let prepared = prepare(&ctx, input, &["u".into()], &mut Ordinary).unwrap();
    assert_eq!(prepared.block[0].name, "arena.native.private.1.I0");
    assert!(prepared
        .block
        .iter()
        .all(|m| m.name.starts_with("arena.native.private.1.")));
    assert_eq!(ctx.get_global("Source"), Some(&old_source));
    assert!(ctx.get_global(collision).is_some());
    assert!(
        ctx.get_global(&prepared.block[0].name).is_none(),
        "preparation never changes caller context"
    );
    let mut scratch = ctx.clone();
    scratch
        .add_universe_poly_inductives(&["u".into()], &prepared.block)
        .unwrap();
    assert_eq!(
        scratch.get_global("Source"),
        Some(&old_source),
        "registration cannot displace original source"
    );
    assert!(
        scratch.get_global(collision).is_some(),
        "auto-derived names cannot replace an existing declaration"
    );
}

#[test]
fn original_and_auxiliary_restoration_preserve_levels_parameters_and_trailing_arguments() {
    let (ctx, input) = input();
    let prepared = prepare(&ctx, input, &["u".into()], &mut Ordinary).unwrap();
    assert_eq!(prepared.parameters[0].0, "A");
    let originals = ["Source", "sourceMk", "Source", "sourceMk"];
    let images: HashMap<_, _> = prepared.restoration.into_iter().collect();
    assert_eq!(
        images.len(),
        4,
        "every source and auxiliary family/constructor restores"
    );
    let names = [
        prepared.block[0].name.as_str(),
        prepared.block[0].constructors[0].0.as_str(),
        prepared.block[1].name.as_str(),
        prepared.block[1].constructors[0].0.as_str(),
    ];
    for (private, original) in names.into_iter().zip(originals) {
        let term = app(
            app(at(private), v("replacementParameter")),
            v("trailingIndexOrField"),
        );
        let restored = super::super::restore_native_term(
            &term,
            &images,
            &["A".into()],
            &[Universe::Var("u".into())],
        )
        .unwrap();
        assert_eq!(
            restored,
            app(
                app(at(original), v("replacementParameter")),
                v("trailingIndexOrField")
            )
        );
    }
    assert_eq!(
        images[&prepared.block[1].name],
        app(at("Source"), v("A")),
        "pre-existing auxiliary images remain in the source namespace"
    );
}

#[test]
fn global_renaming_preserves_local_binders_universes_and_every_term_form() {
    let global = || Term::Global("Old".into());
    let mut term = Term::Let {
        name: "Old".into(),
        ty: Box::new(Term::Sort(Universe::Var("Old".into()))),
        value: Box::new(Term::Const {
            name: "Old".into(),
            levels: vec![Universe::Var("Old".into())],
        }),
        body: Box::new(Term::Pi {
            param: "Old".into(),
            param_type: Box::new(global()),
            body_type: Box::new(Term::Lambda {
                param: "Old".into(),
                param_type: Box::new(global()),
                body: Box::new(Term::Match {
                    discriminant: Box::new(app(global(), v("Old"))),
                    motive: Box::new(global()),
                    cases: vec![Term::Fix {
                        name: "Old".into(),
                        body: Box::new(Term::MutualFix {
                            defs: vec![
                                ("Old".into(), global()),
                                ("other".into(), app(global(), v("Old"))),
                            ],
                            index: 1,
                        }),
                    }],
                }),
            }),
        }),
    };
    rename(
        &mut term,
        &HashMap::from([("Old".into(), "New".into())]),
        4,
        &mut Ordinary,
    )
    .unwrap();
    let Term::Let {
        name,
        ty,
        value,
        body,
    } = term
    else {
        panic!("let shape")
    };
    assert_eq!(name, "Old");
    assert_eq!(*ty, Term::Sort(Universe::Var("Old".into())));
    assert_eq!(
        *value,
        Term::Const {
            name: "New".into(),
            levels: vec![Universe::Var("Old".into())]
        }
    );
    let Term::Pi {
        param,
        param_type,
        body_type,
    } = *body
    else {
        panic!("pi shape")
    };
    assert_eq!(param, "Old");
    assert_eq!(*param_type, Term::Global("New".into()));
    let Term::Lambda {
        param,
        param_type,
        body,
    } = *body_type
    else {
        panic!("lambda shape")
    };
    assert_eq!(param, "Old");
    assert_eq!(*param_type, Term::Global("New".into()));
    let Term::Match {
        discriminant,
        motive,
        cases,
    } = *body
    else {
        panic!("match shape")
    };
    assert_eq!(*discriminant, app(Term::Global("New".into()), v("Old")));
    assert_eq!(*motive, Term::Global("New".into()));
    let Term::Fix { name, body } = &cases[0] else {
        panic!("fix shape")
    };
    assert_eq!(name, "Old");
    let Term::MutualFix { defs, index } = body.as_ref() else {
        panic!("mutual-fix shape")
    };
    assert_eq!(*index, 1);
    assert_eq!(defs[0], ("Old".into(), Term::Global("New".into())));
    assert_eq!(
        defs[1],
        ("other".into(), app(Term::Global("New".into()), v("Old")))
    );
}

#[test]
fn global_rename_admits_resident_key_comparisons_before_mutating() {
    struct Work(u64);
    impl Meter for Work {
        type Error = ();
        fn work(&mut self, n: u64) -> Result<(), ()> {
            self.0 = self.0.checked_sub(n).ok_or(())?;
            Ok(())
        }
        fn reserve(&mut self, _: u64) -> Result<(), ()> {
            Ok(())
        }
        fn inventory(&mut self, _: usize) -> Result<(), ()> {
            Ok(())
        }
        fn admit(&mut self, _: &Term) -> Result<(), ()> {
            Ok(())
        }
        fn image(&mut self, _: &str, _: &[(String, Term)], _: &[String]) -> Result<Term, ()> {
            Err(())
        }
        fn invalid(&self, _: &str) {}
    }
    let unrelated = "z".repeat(8192);
    let resident = 4 + unrelated.len() as u64 + 1;
    let names = HashMap::from([("Old".into(), "New".into()), (unrelated, "Unused".into())]);
    let mut term = Term::Global("Old".into());
    // A tiny queried identifier does not justify ignoring long resident collision candidates.
    assert!(rename(&mut term, &names, resident, &mut Work(32)).is_err());
    assert_eq!(term, Term::Global("Old".into()));
    let mut work = Work(1 + resident + 4 + 4); // visit, lookup, replacement copy
    rename(&mut term, &names, resident, &mut work).unwrap();
    assert_eq!(term, Term::Global("New".into()));
    assert_eq!(work.0, 0);
}
