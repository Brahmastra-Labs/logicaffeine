//! Differential checks against the legacy, forced-Owned substitution path.
use super::*;

fn setup() {
    WHNF_DEPTH.with(|s| s.set(0));
    WHNF_FUEL.with(|s| s.set(MAX_RECHECK_STEPS));
    WHNF_EXHAUSTED.with(|s| s.set(false));
    assert!(!FIX_FACTORY.with(|s| s.get().started));
}
fn owned(t: Db) -> DbBox {
    DbBox::owned(Box::new(t))
}
fn app(a: Db, b: Db) -> Db {
    Db::App(owned(a), owned(b))
}
fn lam(a: Db, b: Db) -> Db {
    Db::Lam(owned(a), owned(b))
}
fn fix(t: Db) -> Db {
    Db::Fix(FixBody::owned(Box::new(t)))
}
fn levels() -> Db {
    Db::Const {
        name: "poly".into(),
        levels: vec![
            Universe::Var("u".into()),
            Universe::Max(
                Box::new(Universe::Type(0)),
                Box::new(Universe::Succ(Box::new(Universe::Var("v".into())))),
            ),
            Universe::IMax(Box::new(Universe::Prop), Box::new(Universe::Type(1))),
        ],
    }
}
fn argument() -> Db {
    app(
        Db::Var(0),
        lam(Db::Sort(Universe::Type(0)), app(Db::Var(1), Db::Var(5))),
    )
}
fn fixtures(k: usize) -> Vec<(&'static str, Db)> {
    let forms = vec![
        (
            "pi",
            Db::Pi(owned(Db::Var(k)), owned(app(Db::Var(0), Db::Var(k)))),
        ),
        ("lambda", lam(Db::Var(k), app(Db::Var(0), Db::Var(k)))),
        ("application", app(Db::Var(k), Db::Var(k))),
        (
            "let",
            Db::Let(
                owned(Db::Var(k)),
                owned(Db::Var(k)),
                owned(app(Db::Var(0), Db::Var(k))),
            ),
        ),
        (
            "match",
            Db::Match {
                disc: owned(Db::Var(k)),
                motive: owned(lam(Db::Var(k), app(Db::Var(0), Db::Var(k)))),
                cases: vec![Db::Var(k), lam(Db::Var(k), app(Db::Var(0), Db::Var(k)))],
            },
        ),
        ("fix", fix(app(Db::Var(0), Db::Var(k)))),
        (
            "mutual-empty",
            Db::MutualFix {
                defs: vec![],
                index: 0,
            },
        ),
        (
            "mutual-single",
            Db::MutualFix {
                defs: vec![app(Db::Var(0), Db::Var(k))],
                index: 0,
            },
        ),
        (
            "mutual-first",
            Db::MutualFix {
                defs: vec![
                    lam(Db::Var(k), app(Db::Var(2), Db::Var(k))),
                    app(Db::Var(1), Db::Var(k)),
                ],
                index: 0,
            },
        ),
        (
            "mutual-last",
            Db::MutualFix {
                defs: vec![
                    app(Db::Var(0), Db::Var(k)),
                    lam(Db::Var(k), app(Db::Var(2), Db::Var(k))),
                ],
                index: 1,
            },
        ),
        (
            "nested-binders",
            lam(
                Db::Var(k),
                Db::Let(
                    owned(Db::Var(k)),
                    owned(Db::Var(0)),
                    owned(fix(Db::MutualFix {
                        defs: vec![
                            app(Db::Var(4), Db::Var(k)),
                            lam(Db::Var(k), app(Db::Var(5), Db::Var(k))),
                        ],
                        index: 1,
                    })),
                ),
            ),
        ),
    ];
    // This ordinary child boundary also makes admission observable for an empty
    // mutual block, which has no child edges of its own. It binds no variables.
    forms
        .into_iter()
        .map(|(name, t)| (name, app(t, levels())))
        .collect()
}

fn promote(t: &Db) -> Db {
    fn edge(t: &Db) -> DbBox {
        DbBox::new(promote(t))
    }
    match t {
        Db::Pi(a, b) => Db::Pi(edge(a), edge(b)),
        Db::Lam(a, b) => Db::Lam(edge(a), edge(b)),
        Db::App(a, b) => Db::App(edge(a), edge(b)),
        Db::Let(a, b, c) => Db::Let(edge(a), edge(b), edge(c)),
        Db::Match {
            disc,
            motive,
            cases,
        } => Db::Match {
            disc: edge(disc),
            motive: edge(motive),
            cases: cases.iter().map(promote).collect(),
        },
        Db::Fix(body) => Db::Fix(FixBody::new(promote(body))),
        Db::MutualFix { defs, index } => Db::MutualFix {
            defs: defs.iter().map(promote).collect(),
            index: *index,
        },
        Db::Sort(_) | Db::Var(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => t.clone(),
    }
}

// Count physical shared boundaries by occurrences; do not use pointer identity
// to replace any syntactic comparison or conversion result.
fn storage_counts(t: &Db) -> (usize, usize, usize) {
    fn edge(t: &DbBox, counts: &mut (usize, usize, usize)) {
        counts.0 += usize::from(t.is_shared());
        counts.1 += usize::from(t.facts().is_some());
        walk(t, counts);
    }
    fn walk(t: &Db, counts: &mut (usize, usize, usize)) {
        match t {
            Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
                edge(a, counts);
                edge(b, counts);
            }
            Db::Let(a, b, c) => {
                edge(a, counts);
                edge(b, counts);
                edge(c, counts);
            }
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                edge(disc, counts);
                edge(motive, counts);
                for case in cases {
                    walk(case, counts);
                }
            }
            Db::Fix(body) => {
                counts.2 += usize::from(matches!(body, FixBody::Shared(_)));
                walk(body, counts);
            }
            Db::MutualFix { defs, .. } => {
                for body in defs {
                    walk(body, counts);
                }
            }
            Db::Sort(_) | Db::Var(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => {}
        }
    }
    let mut counts = (0, 0, 0);
    walk(t, &mut counts);
    counts
}

fn legacy_beta(body: &Db, arg: &Db) -> Db {
    assert!(!FIX_FACTORY.with(|s| s.get().started));
    assert_eq!(storage_counts(body), (0, 0, 0));
    assert_eq!(storage_counts(arg), (0, 0, 0));
    shift(&subst(body, 0, &shift(arg, 1, 0)), -1, 0)
}

fn assert_storage(t: &Db, active: bool, label: &str) {
    let (shared, current, _) = storage_counts(t);
    assert!(
        shared > 0,
        "{label}: fixture must exercise ordinary shared edges"
    );
    assert_eq!(
        current,
        if active { shared } else { 0 },
        "{label}: storage metadata generation"
    );
}

#[test]
fn borrowed_and_owned_beta_match_legacy_for_active_and_expired_storage() {
    setup();
    let c = Context::new();
    let mut saw_shared_fix = false;
    for k in 0..=5 {
        for (name, body) in fixtures(k) {
            let arg = argument();
            let expected = legacy_beta(&body, &arg);
            let (shared, shared_arg) = {
                let _scope = PrefixScope::enter(&c);
                assert!(fix_factory_begin());
                let shared = promote(&body);
                let shared_arg = promote(&arg);
                assert_storage(&shared, true, name);
                assert_storage(&shared_arg, true, name);
                saw_shared_fix |= storage_counts(&shared).2 > 0;
                let alias = shared.clone();
                assert_eq!(
                    beta_open(&shared, &shared_arg),
                    expected,
                    "{name}, index {k}, borrowed active"
                );
                assert_eq!(
                    beta_open_owned(shared.clone(), &shared_arg),
                    expected,
                    "{name}, index {k}, owned active"
                );
                assert_eq!(shared, body, "{name}: active source changed");
                assert_eq!(alias, body, "{name}: active alias changed");
                assert_eq!(shared_arg, arg, "{name}: active argument changed");
                (shared, shared_arg)
            };
            assert_storage(&shared, false, name);
            assert_storage(&shared_arg, false, name);
            let alias = shared.clone();
            assert_eq!(
                beta_open(&shared, &shared_arg),
                expected,
                "{name}, index {k}, borrowed expired"
            );
            assert_eq!(
                beta_open_owned(shared.clone(), &shared_arg),
                expected,
                "{name}, index {k}, owned expired"
            );
            assert_eq!(shared, body, "{name}: expired source changed");
            assert_eq!(alias, body, "{name}: expired alias changed");
            assert_eq!(shared_arg, arg, "{name}: expired argument changed");
        }
    }
    assert!(
        saw_shared_fix,
        "also exercise admitted recursive-body storage"
    );
}

#[test]
fn consuming_and_deferred_beta_match_legacy_for_active_and_expired_storage() {
    setup();
    let c = Context::new();
    for k in 0..=5 {
        for (name, body) in fixtures(k) {
            let arg = argument();
            let expected = legacy_beta(&body, &arg);
            let (shared, shared_arg) = {
                let _scope = PrefixScope::enter(&c);
                assert!(fix_factory_begin());
                let shared = promote(&body);
                let shared_arg = promote(&arg);
                assert_storage(&shared, true, name);
                assert_storage(&shared_arg, true, name);
                let alias = shared.clone();
                let arg_alias = shared_arg.clone();
                assert_eq!(
                    beta_open_consuming(shared.clone(), shared_arg.clone()),
                    expected,
                    "{name}, index {k}, consuming active"
                );
                assert_eq!(
                    beta_open_deferred(shared.clone(), shared_arg.clone()),
                    expected,
                    "{name}, index {k}, deferred active"
                );
                assert_eq!(shared, body, "{name}: active source changed");
                assert_eq!(alias, body, "{name}: active alias changed");
                assert_eq!(shared_arg, arg, "{name}: active argument changed");
                assert_eq!(arg_alias, arg, "{name}: active argument alias changed");
                (shared, shared_arg)
            };
            assert_storage(&shared, false, name);
            assert_storage(&shared_arg, false, name);
            let alias = shared.clone();
            let arg_alias = shared_arg.clone();
            assert_eq!(
                beta_open_consuming(shared.clone(), shared_arg.clone()),
                expected,
                "{name}, index {k}, consuming expired"
            );
            assert_eq!(
                beta_open_deferred(shared.clone(), shared_arg.clone()),
                expected,
                "{name}, index {k}, deferred expired"
            );
            assert_eq!(shared, body, "{name}: expired source changed");
            assert_eq!(alias, body, "{name}: expired alias changed");
            assert_eq!(shared_arg, arg, "{name}: expired argument changed");
            assert_eq!(arg_alias, arg, "{name}: expired argument alias changed");
        }
    }
}

#[test]
fn borrowed_and_owned_shifts_match_unshared_reference_across_all_binders() {
    setup();
    let c = Context::new();
    // The negative shift starts above index zero, including beneath binders,
    // so this exercises removal without manufacturing underflowed indices.
    let shifts = [(2, 0), (1, 2), (-1, 1)];
    for k in 0..=5 {
        for (name, body) in fixtures(k) {
            assert_eq!(storage_counts(&body), (0, 0, 0));
            assert!(!FIX_FACTORY.with(|s| s.get().started));
            let expected: Vec<_> = shifts
                .iter()
                .map(|&(delta, cutoff)| shift(&body, delta, cutoff))
                .collect();
            let shared = {
                let _scope = PrefixScope::enter(&c);
                assert!(fix_factory_begin());
                let shared = promote(&body);
                assert_storage(&shared, true, name);
                let alias = shared.clone();
                for (&(delta, cutoff), expected) in shifts.iter().zip(&expected) {
                    assert_eq!(
                        &shift(&shared, delta, cutoff),
                        expected,
                        "{name}, index {k}, delta {delta}, cutoff {cutoff}, borrowed active"
                    );
                    assert_eq!(
                        &shift_owned(shared.clone(), delta, cutoff),
                        expected,
                        "{name}, index {k}, delta {delta}, cutoff {cutoff}, owned active"
                    );
                    assert_eq!(shared, body, "{name}: active source changed");
                    assert_eq!(alias, body, "{name}: active alias changed");
                }
                shared
            };
            assert_storage(&shared, false, name);
            let alias = shared.clone();
            for (&(delta, cutoff), expected) in shifts.iter().zip(&expected) {
                assert_eq!(
                    &shift(&shared, delta, cutoff),
                    expected,
                    "{name}, index {k}, delta {delta}, cutoff {cutoff}, borrowed expired"
                );
                assert_eq!(
                    &shift_owned(shared.clone(), delta, cutoff),
                    expected,
                    "{name}, index {k}, delta {delta}, cutoff {cutoff}, owned expired"
                );
                assert_eq!(shared, body, "{name}: expired source changed");
                assert_eq!(alias, body, "{name}: expired alias changed");
            }
        }
    }
}
