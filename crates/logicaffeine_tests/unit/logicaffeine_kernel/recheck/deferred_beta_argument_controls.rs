use super::*;
fn g(s: &str) -> Db {
    Db::Global(s.into())
}
fn app(a: Db, b: Db) -> Db {
    Db::App(DbBox::new(a), DbBox::new(b))
}
fn arg() -> Db {
    app(g(&"retained".repeat(80)), Db::Var(4))
}
fn ptr(t: &Db) -> (*const Db, *const Db, *const u8) {
    let Db::App(a, b) = t else { panic!() };
    let Db::Global(n) = a.as_ref() else { panic!() };
    (a.as_ref(), b.as_ref(), n.as_ptr())
}
fn large(mut t: Db) -> Db {
    for _ in 0..300 {
        t = app(g("step"), t);
    }
    t
}
fn tail(mut t: &Db) -> &Db {
    for _ in 0..300 {
        let Db::App(_, b) = t else { panic!() };
        t = b;
    }
    t
}
#[test]
fn large_fallback_moves_the_final_argument_without_copying_it() {
    let a = arg();
    let storage = ptr(&a);
    let body = large(Db::Var(0));
    let mut allowance = OWNED_BETA_SCAN_NODES;
    assert_eq!(beta_occurrences(&body, 0, &mut allowance), None);
    let expected = beta_open(&body, &a);
    let result = beta_open_consuming(body, a);
    assert_eq!(result, expected);
    assert_eq!(ptr(tail(&result)), storage);
}
#[test]
fn deferred_slots_keep_their_own_binder_depth_across_siblings() {
    let a = arg();
    let storage = ptr(&a);
    let body = Db::Let(
        DbBox::new(Db::Var(0)),
        DbBox::new(Db::Var(0)),
        DbBox::new(Db::MutualFix {
            defs: vec![
                Db::Var(3),
                Db::Lam(DbBox::new(Db::Var(3)), DbBox::new(Db::Var(4))),
            ],
            index: 1,
        }),
    );
    let expected = beta_open(&body, &a);
    let result = beta_open_deferred(body, a);
    assert_eq!(result, expected);
    let Db::Let(_, _, body) = &result else {
        panic!()
    };
    let Db::MutualFix { defs, .. } = body.as_ref() else {
        panic!()
    };
    let Db::Lam(_, last) = &defs[1] else { panic!() };
    assert_eq!(ptr(last), storage);
}
#[test]
fn deferred_walk_visits_each_owned_body_node_once_and_moves_at_zero_depth() {
    let body = large(Db::Var(0));
    let a = arg();
    let expected = beta_open(&body, &a);
    BETA_WORK.with(|w| w.set(Some((0, 0))));
    let result = beta_open_deferred(body, a);
    let work = BETA_WORK.with(|w| w.replace(None).unwrap());
    assert_eq!(result, expected);
    assert_eq!(work, (601, 0));
}
#[test]
fn all_binding_forms_match_the_original_operation() {
    let leaves = [
        Db::Var(0),
        Db::Var(1),
        Db::Var(2),
        Db::Var(5),
        g("g"),
        Db::Sort(Universe::Type(0)),
        Db::Const {
            name: "C".into(),
            levels: vec![Universe::Var("u".into())],
        },
        Db::Lit(Literal::Int(6)),
    ];
    for a in &leaves {
        for b in &leaves {
            let forms = [
                app(a.clone(), b.clone()),
                Db::Lam(DbBox::new(a.clone()), DbBox::new(b.clone())),
                Db::Pi(DbBox::new(a.clone()), DbBox::new(b.clone())),
                Db::Fix(FixBody::owned(Box::new(app(a.clone(), b.clone())))),
                Db::MutualFix {
                    defs: vec![a.clone(), b.clone()],
                    index: 0,
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
            for t in forms {
                for replacement in [arg(), Db::Var(0), g("closed")] {
                    assert_eq!(
                        beta_open_deferred(t.clone(), replacement.clone()),
                        beta_open(&t, &replacement)
                    );
                }
            }
        }
    }
}
#[test]
fn unknown_shared_metadata_flushes_pending_slot_and_preserves_owners() {
    let c = Context::new();
    let escaped = {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        let f = FixBody::new(Db::Lam(DbBox::new(g("A")), DbBox::new(Db::Var(1))));
        assert!(f.facts().is_some());
        f
    };
    assert!(escaped.facts().is_none());
    let original = escaped.as_ref().clone();
    for after in [false, true] {
        let mut body = app(Db::Var(0), Db::Fix(escaped.clone()));
        if after {
            body = app(body, Db::Var(0));
        }
        let a = arg();
        let expected = beta_open(&body, &a);
        assert_eq!(beta_open_deferred(body, a), expected);
        assert_eq!(escaped.as_ref(), &original);
    }
}
#[test]
fn no_occurrences_do_not_traverse_or_copy_the_argument() {
    let body = large(Db::Var(2));
    let expected = large(Db::Var(1));
    let a = large(arg());
    BETA_WORK.with(|w| w.set(Some((0, 0))));
    let result = beta_open_deferred(body, a);
    let work = BETA_WORK.with(|w| w.replace(None).unwrap());
    assert_eq!(result, expected);
    assert_eq!(work, (601, 0));
}
