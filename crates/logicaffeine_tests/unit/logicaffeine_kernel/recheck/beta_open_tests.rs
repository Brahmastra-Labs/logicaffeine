use super::*;

fn legacy(body: &Db, arg: &Db) -> Db {
    shift(&subst(body, 0, &shift(arg, 1, 0)), -1, 0)
}

fn measured(f: impl FnOnce() -> Db) -> (Db, (usize, usize)) {
    BETA_WORK.with(|work| work.set(Some((0, 0))));
    let result = f();
    let count = BETA_WORK.with(|work| work.replace(None).unwrap());
    (result, count)
}

fn constructors(seed: Db) -> Vec<Db> {
    let b = || DbBox::new(seed.clone());
    vec![
        Db::Sort(Universe::Type(0)),
        Db::Global("opaque".into()),
        Db::Const {
            name: "poly".into(),
            levels: vec![Universe::Var("u".into())],
        },
        Db::Lit(Literal::Nat(7.into())),
        seed.clone(),
        Db::Pi(b(), b()),
        Db::Lam(b(), b()),
        Db::App(b(), b()),
        Db::Match {
            disc: b(),
            motive: b(),
            cases: vec![seed.clone(), Db::Var(0)],
        },
        Db::Fix(FixBody::owned(b().into_box())),
        Db::MutualFix {
            defs: vec![],
            index: 0,
        },
        Db::MutualFix {
            defs: vec![seed.clone()],
            index: 0,
        },
        Db::MutualFix {
            defs: vec![seed.clone(), Db::Var(1), Db::Var(3)],
            index: 2,
        },
        Db::Let(b(), b(), b()),
    ]
}

#[test]
fn fused_beta_matches_legacy_for_every_constructor_and_binding_position() {
    let mut comparisons = 0;
    for k in 0..7 {
        for inner in constructors(Db::Var(k)) {
            for body in constructors(inner) {
                for arg in constructors(Db::App(DbBox::new(Db::Var(0)), DbBox::new(Db::Var(2)))) {
                    assert_eq!(
                        beta_open(&body, &arg),
                        legacy(&body, &arg),
                        "body={body:?}, arg={arg:?}"
                    );
                    comparisons += 1;
                }
            }
        }
    }
    assert_eq!(comparisons, 19208);
}

fn deep_fixture(used: bool) -> (Db, Db) {
    let mut arg = Db::Var(0);
    for _ in 0..128 {
        arg = Db::App(DbBox::new(Db::Global("f".into())), DbBox::new(arg));
    }
    let mut body = if used {
        Db::Var(40)
    } else {
        Db::Global("unused".into())
    };
    for _ in 0..40 {
        body = Db::Lam(DbBox::new(Db::Sort(Universe::Type(0))), DbBox::new(body));
    }
    (body, arg)
}

#[test]
fn opening_unused_argument_does_not_traverse_or_copy_it() {
    let (body, arg) = deep_fixture(false);
    let (expected, old_work) = measured(|| legacy(&body, &arg));
    let (actual, new_work) = measured(|| beta_open(&body, &arg));
    eprintln!("unused: legacy visits/copies={old_work:?}, current={new_work:?}");
    assert_eq!(actual, expected);
    assert_eq!(new_work, (db_node_count(&body), db_node_count(&body)));
}

#[test]
fn opening_used_argument_shifts_it_once_at_its_occurrence() {
    let (body, arg) = deep_fixture(true);
    let (expected, old_work) = measured(|| legacy(&body, &arg));
    let (actual, new_work) = measured(|| beta_open(&body, &arg));
    eprintln!("used: legacy visits/copies={old_work:?}, current={new_work:?}");
    assert_eq!(actual, expected);
    assert_eq!(new_work.0, db_node_count(&body) + db_node_count(&arg));
    assert_eq!(new_work.1, db_node_count(&body) - 1 + db_node_count(&arg));
}
