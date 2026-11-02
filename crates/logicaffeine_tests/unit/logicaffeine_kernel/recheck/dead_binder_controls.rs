//! Dead-binder beta controls: absence of the removed variable alone is insufficient.
use super::*;

fn owned(t: Db) -> DbBox {
    DbBox::owned(Box::new(t))
}
fn g(name: &str) -> Db {
    Db::Global(name.into())
}
fn app(a: Db, b: Db) -> Db {
    Db::App(owned(a), owned(b))
}
fn lam(a: Db, b: Db) -> Db {
    Db::Lam(owned(a), owned(b))
}
fn argument() -> Db {
    app(g(&"retained_argument".repeat(40)), Db::Var(4))
}
fn legacy(body: &Db, arg: &Db) -> Db {
    shift(&subst(body, 0, &shift(arg, 1, 0)), -1, 0)
}
fn lam_storage(t: &Db) -> (*const Db, *const Db, *const u8) {
    let Db::Lam(a, b) = t else { panic!("lambda") };
    let Db::Global(name) = a.as_ref() else {
        panic!("global domain")
    };
    (a.as_ref(), b.as_ref(), name.as_ptr())
}
fn app_storage(t: &Db) -> (*const Db, *const Db, *const u8) {
    let Db::App(a, b) = t else {
        panic!("application")
    };
    let Db::Global(name) = a.as_ref() else {
        panic!("global function")
    };
    (a.as_ref(), b.as_ref(), name.as_ptr())
}
struct Reset {
    depth: usize,
    fuel: usize,
    exhausted: bool,
    work: Option<(usize, usize)>,
}
impl Drop for Reset {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|s| s.set(self.depth));
        WHNF_FUEL.with(|s| s.set(self.fuel));
        WHNF_EXHAUSTED.with(|s| s.set(self.exhausted));
        BETA_WORK.with(|s| s.set(self.work));
    }
}
fn reset() -> Reset {
    Reset {
        depth: WHNF_DEPTH.with(|s| s.replace(0)),
        fuel: WHNF_FUEL.with(|s| s.replace(MAX_RECHECK_STEPS)),
        exhausted: WHNF_EXHAUSTED.with(|s| s.replace(false)),
        work: BETA_WORK.with(|s| s.replace(None)),
    }
}
fn measured(f: impl FnOnce() -> Db) -> (Db, (usize, usize)) {
    BETA_WORK.with(|s| s.set(Some((0, 0))));
    let result = f();
    let work = BETA_WORK.with(|s| s.replace(None).unwrap());
    (result, work)
}

#[test]
fn closed_owned_body_needs_only_its_three_scan_visits_and_retains_storage() {
    let _reset = reset();
    let body = lam(g(&"retained_domain".repeat(30)), Db::Var(0));
    let pointers = lam_storage(&body);
    let expected = body.clone();
    // This large unused argument is deliberately prepared outside measurement.
    let mut arg = argument();
    for _ in 0..300 {
        arg = app(g("unused"), arg);
    }
    let (result, work) = measured(|| beta_open_consuming(body, arg));
    assert_eq!(result, expected);
    assert_eq!(lam_storage(&result), pointers);
    assert_eq!(
        work,
        (3, 0),
        "complete dead-binder scan must avoid a second traversal"
    );
}

#[test]
fn unused_binder_still_rebases_outer_variables_and_preserves_argument_occurrences() {
    let _reset = reset();
    assert_eq!(beta_open_consuming(Db::Var(1), argument()), Db::Var(0));
    assert_eq!(
        beta_open_consuming(lam(g("A"), Db::Var(2)), argument()),
        lam(g("A"), Db::Var(1))
    );
    let arg = argument();
    let pointers = app_storage(&arg);
    let expected = arg.clone();
    let (result, work) = measured(|| beta_open_consuming(Db::Var(0), arg));
    assert_eq!(result, expected);
    assert_eq!(app_storage(&result), pointers);
    assert_eq!(
        work,
        (2, 0),
        "an actual occurrence still scans and rewrites"
    );
}

#[test]
fn every_binder_form_and_capture_position_matches_legacy_substitution() {
    let _reset = reset();
    for k in 0..=5 {
        let forms = vec![
            Db::Var(k),
            Db::Pi(owned(Db::Var(k)), owned(app(Db::Var(0), Db::Var(k)))),
            lam(Db::Var(k), app(Db::Var(0), Db::Var(k))),
            app(Db::Var(k), Db::Var(k)),
            Db::Let(
                owned(Db::Var(k)),
                owned(Db::Var(k)),
                owned(lam(Db::Var(0), Db::Var(k))),
            ),
            Db::Match {
                disc: owned(Db::Var(k)),
                motive: owned(lam(g("A"), Db::Var(k))),
                cases: vec![Db::Var(k), lam(Db::Var(0), Db::Var(k))],
            },
            Db::Fix(FixBody::owned(Box::new(app(Db::Var(0), Db::Var(k))))),
            Db::MutualFix {
                defs: vec![],
                index: 0,
            },
            Db::MutualFix {
                defs: vec![app(Db::Var(0), Db::Var(k))],
                index: 0,
            },
            Db::MutualFix {
                defs: vec![lam(Db::Var(k), Db::Var(2)), Db::Var(k)],
                index: 1,
            },
        ];
        for body in forms {
            let alias = body.clone();
            let arg = argument();
            let expected = legacy(&body, &arg);
            assert_eq!(
                beta_open_consuming(body.clone(), arg),
                expected,
                "capture {k}: {body:?}"
            );
            assert_eq!(body, alias);
        }
    }
}

#[test]
fn exact_scan_limit_closed_body_skips_all_rewriting() {
    let _reset = reset();
    // Root + discriminant + motive + 253 cases = exactly 256 inspected nodes.
    let body = Db::Match {
        disc: owned(g("disc")),
        motive: owned(g("motive")),
        cases: (0..253).map(|_| g("case")).collect(),
    };
    let Db::Match { cases, .. } = &body else {
        unreachable!()
    };
    let storage = cases.as_ptr();
    let expected = body.clone();
    let (result, work) = measured(|| beta_open_consuming(body, argument()));
    assert_eq!(result, expected);
    let Db::Match { cases, .. } = &result else {
        panic!("match")
    };
    assert_eq!(cases.as_ptr(), storage);
    assert_eq!(work, (256, 0));
}

#[test]
fn unvisited_capture_beyond_scan_limit_takes_real_substitution_fallback() {
    let _reset = reset();
    let mut cases: Vec<_> = (0..253).map(|_| g("case")).collect();
    cases.push(Db::Var(0)); // 257th node is the sole substitution occurrence.
    let body = Db::Match {
        disc: owned(g("disc")),
        motive: owned(g("motive")),
        cases,
    };
    let arg = argument();
    let pointers = app_storage(&arg);
    let expected = legacy(&body, &arg);
    let (result, work) = measured(|| beta_open_consuming(body, arg));
    assert_eq!(result, expected);
    let Db::Match { cases, .. } = &result else {
        panic!("match")
    };
    assert_eq!(app_storage(cases.last().unwrap()), pointers);
    assert_eq!(
        work,
        (513, 0),
        "256 incomplete scan visits plus 257 deferred visits"
    );
}

#[test]
fn shared_dead_subtrees_keep_aliases_and_expired_fix_facts_use_fallback() {
    let _reset = reset();
    let ctx = Context::new();
    let baseline = FIX_SHARED_LIVE.with(|s| s.get());
    let (escaped, alias) = {
        let _scope = PrefixScope::enter(&ctx);
        assert!(fix_factory_begin());
        let shared = DbBox::new(lam(g("A"), Db::Var(0)));
        assert!(shared.is_shared());
        let fixed = FixBody::new(lam(g("A"), Db::Var(1)));
        assert!(fixed.facts().is_some());
        let body = Db::App(shared, owned(Db::Fix(fixed)));
        let alias = body.clone();
        let (result, work) = measured(|| beta_open_consuming(body, argument()));
        assert_eq!(result, alias);
        let (Db::App(a, b), Db::App(c, d)) = (&result, &alias) else {
            panic!("application")
        };
        assert!(std::ptr::eq(a.as_ref(), c.as_ref()));
        let (Db::Fix(b), Db::Fix(d)) = (b.as_ref(), d.as_ref()) else {
            panic!("fix")
        };
        assert!(b.same_current(d));
        assert_eq!(
            work,
            (3, 0),
            "current closed facts eliminate rewriting without expanding owners"
        );
        (result, alias)
    };
    let Db::App(a, b) = &escaped else {
        panic!("application")
    };
    assert!(a.facts().is_none());
    let Db::Fix(fixed) = b.as_ref() else {
        panic!("fix")
    };
    assert!(fixed.facts().is_none());
    let expected = legacy(&escaped, &argument());
    let result = beta_open_consuming(escaped, argument());
    assert_eq!(result, expected);
    assert_eq!(alias, expected);
    drop(result);
    drop(alias);
    drop(expected);
    assert_eq!(FIX_SHARED_LIVE.with(|s| s.get()), baseline);
}

#[test]
fn lambda_application_and_let_reduction_reuse_the_same_dead_binder_fast_path() {
    let _reset = reset();
    let ctx = Context::new();
    for is_let in [false, true] {
        let body = lam(g("A"), Db::Var(0));
        let pointers = lam_storage(&body);
        let expected = body.clone();
        let (result, work) = measured(|| {
            let step = if is_let {
                whnf_step(
                    &ctx,
                    &[],
                    Db::Let(owned(g("A")), owned(argument()), owned(body)),
                )
            } else {
                resume_whnf(
                    &ctx,
                    &[],
                    WhnfFrame::Apply(owned(argument())),
                    lam(g("A"), body),
                )
            };
            let WhnfStep::Reduce(result) = step else {
                panic!("beta reduction")
            };
            result
        });
        assert_eq!(result, expected);
        assert_eq!(lam_storage(&result), pointers);
        assert_eq!(work, (3, 0));
    }
}
