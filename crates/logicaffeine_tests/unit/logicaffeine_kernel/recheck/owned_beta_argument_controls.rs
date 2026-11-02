use super::*;
fn g(s: &str) -> Db {
    Db::Global(s.into())
}
fn app(a: Db, b: Db) -> Db {
    Db::App(DbBox::new(a), DbBox::new(b))
}
fn lam(a: Db, b: Db) -> Db {
    Db::Lam(DbBox::new(a), DbBox::new(b))
}
fn argument() -> Db {
    app(g(&"retained_argument".repeat(50)), Db::Var(3))
}
fn storage(t: &Db) -> (*const Db, *const Db, *const u8) {
    let Db::App(f, a) = t else {
        panic!("application")
    };
    let Db::Global(n) = f.as_ref() else {
        panic!("name")
    };
    (f.as_ref(), a.as_ref(), n.as_ptr())
}
#[test]
fn sole_occurrence_moves_argument_storage() {
    let arg = argument();
    let ptr = storage(&arg);
    let expected = arg.clone();
    let result = beta_open_consuming(Db::Var(0), arg);
    assert_eq!(result, expected);
    assert_eq!(storage(&result), ptr);
}
#[test]
fn final_of_multiple_occurrences_moves_storage_at_its_own_depth() {
    let arg = argument();
    let ptr = storage(&arg);
    let body = app(Db::Var(0), lam(g("A"), Db::Var(1)));
    let expected = beta_open(&body, &arg);
    let result = beta_open_consuming(body, arg);
    assert_eq!(result, expected);
    let Db::App(first, last) = &result else {
        panic!()
    };
    let Db::Lam(_, last) = last.as_ref() else {
        panic!()
    };
    assert_eq!(storage(last), ptr);
    assert_ne!(storage(first), ptr);
}
#[test]
fn owned_shift_preserves_storage_and_bound_indices() {
    for d in [-1, 0, 1, 4] {
        let t = lam(argument(), app(Db::Var(0), argument()));
        let Db::Lam(a, b) = &t else { panic!() };
        let ptr = (a.as_ref() as *const Db, b.as_ref() as *const Db, storage(a));
        let expected = shift(&t, d, 1);
        let result = shift_owned(t, d, 1);
        assert_eq!(result, expected);
        let Db::Lam(a, b) = &result else { panic!() };
        assert_eq!(
            (a.as_ref() as *const Db, b.as_ref() as *const Db, storage(a)),
            ptr
        );
    }
}
#[test]
fn every_syntax_form_matches_borrowed_beta_and_shift() {
    let leaves = [
        Db::Var(0),
        Db::Var(1),
        Db::Var(2),
        Db::Var(6),
        g("G"),
        Db::Sort(Universe::Type(2)),
        Db::Const {
            name: "Poly".into(),
            levels: vec![Universe::Var("u".into())],
        },
        Db::Lit(Literal::Int(9)),
    ];
    let args = [
        argument(),
        lam(Db::Var(2), app(Db::Var(0), Db::Var(3))),
        g("closed"),
    ];
    for a in &leaves {
        for b in &leaves {
            let forms = [
                app(a.clone(), b.clone()),
                lam(a.clone(), b.clone()),
                Db::Pi(DbBox::new(a.clone()), DbBox::new(b.clone())),
                Db::Fix(FixBody::owned(Box::new(app(a.clone(), b.clone())))),
                Db::MutualFix {
                    defs: vec![a.clone(), b.clone()],
                    index: 1,
                },
                Db::Let(
                    DbBox::new(a.clone()),
                    DbBox::new(b.clone()),
                    DbBox::new(app(a.clone(), b.clone())),
                ),
                Db::Match {
                    disc: DbBox::new(a.clone()),
                    motive: DbBox::new(b.clone()),
                    cases: vec![a.clone(), b.clone()],
                },
            ];
            for form in forms {
                for arg in &args {
                    assert_eq!(
                        beta_open_consuming(form.clone(), arg.clone()),
                        beta_open(&form, arg)
                    );
                }
                for cutoff in 0..3 {
                    for delta in [0, 1, 3] {
                        assert_eq!(
                            shift_owned(form.clone(), delta, cutoff),
                            shift(&form, delta, cutoff)
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn bounded_scan_falls_back_without_changing_substitution() {
    let mut t = Db::Var(0);
    for _ in 0..OWNED_BETA_SCAN_NODES {
        t = app(g("f"), t);
    }
    let mut left = OWNED_BETA_SCAN_NODES;
    assert_eq!(beta_occurrences(&t, 0, &mut left), None);
    assert_eq!(left, 0);
    let arg = argument();
    let expected = beta_open(&t, &arg);
    assert_eq!(beta_open_consuming(t, arg), expected);
    let mut left = 1;
    assert_eq!(beta_occurrences(&Db::Var(0), 0, &mut left), Some(1));
    assert_eq!(left, 0);
}
#[test]
fn unused_argument_and_shadowed_variables_remain_untouched() {
    let t = lam(Db::Var(2), Db::Var(0));
    let mut left = 256;
    assert_eq!(beta_occurrences(&t, 0, &mut left), Some(0));
    assert_eq!(
        beta_open_consuming(t, argument()),
        lam(Db::Var(1), Db::Var(0))
    );
}
#[test]
fn shared_fix_owners_and_expired_metadata_preserve_scope() {
    let c = Context::new();
    let escaped;
    {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        let fixed = Db::Fix(FixBody::new(lam(g("A"), Db::Var(1))));
        let Db::Fix(owner) = &fixed else { panic!() };
        assert!(owner.facts().is_some());
        let t = app(fixed.clone(), Db::Var(0));
        let result = beta_open_consuming(t, argument());
        let Db::App(f, _) = &result else { panic!() };
        let Db::Fix(result_owner) = f.as_ref() else {
            panic!()
        };
        assert!(owner.same_current(result_owner));
        assert_eq!(owner.as_ref(), result_owner.as_ref());
        escaped = fixed;
    }
    let mut left = 256;
    assert_eq!(beta_occurrences(&escaped, 0, &mut left), None);
    let expected = beta_open(&escaped, &argument());
    assert_eq!(beta_open_consuming(escaped, argument()), expected);
}
#[test]
fn evaluator_application_and_let_consume_owned_arguments() {
    let c = Context::new();
    let arg = argument();
    let ptr = storage(&arg);
    let step = resume_whnf(
        &c,
        &[],
        WhnfFrame::Apply(DbBox::owned(Box::new(arg))),
        lam(g("A"), Db::Var(0)),
    );
    let WhnfStep::Reduce(result) = step else {
        panic!("beta reduction")
    };
    assert_eq!(storage(&result), ptr);
    // Let follows the same consuming substitution; nested binder shifts still apply.
    let arg = argument();
    let ptr = storage(&arg);
    let t = Db::Let(
        DbBox::new(g("A")),
        DbBox::new(arg),
        DbBox::new(lam(g("A"), Db::Var(1))),
    );
    let WhnfStep::Reduce(result) = whnf_step(&c, &[], t) else {
        panic!("let reduction")
    };
    let Db::Lam(_, body) = result else { panic!() };
    assert_eq!(storage(&body), ptr);
}
#[test]
fn work_counter_includes_scan_and_rewrite_without_argument_copies() {
    let arg = argument();
    BETA_WORK.with(|w| w.set(Some((0, 0))));
    let result = beta_open_consuming(Db::Var(0), arg);
    let work = BETA_WORK.with(|w| w.replace(None).unwrap());
    assert_eq!(result, argument());
    assert_eq!(work, (2, 0));
    BETA_WORK.with(|w| w.set(Some((0, 0))));
    let body = lam(g("A"), Db::Var(0));
    let result = beta_open_consuming(body, argument());
    let work = BETA_WORK.with(|w| w.replace(None).unwrap());
    assert_eq!(result, lam(g("A"), Db::Var(0)));
    assert_eq!(work, (3, 0));
}
