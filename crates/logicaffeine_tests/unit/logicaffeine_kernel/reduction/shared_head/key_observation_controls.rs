use super::*;
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn graph(r: &mut HeadReducerWithWork<'_, true>, count: usize) -> (Lazy, Lazy) {
    let nat = Term::Global("Nat".into());
    let term = Term::App(
        Box::new(Term::Lambda {
            param: "n".into(),
            param_type: Box::new(nat.clone()),
            body: Box::new(Term::Var("n".into())),
        }),
        Box::new(Term::Global("Zero".into())),
    );
    let expression = r.store.intern(&term);
    let leaf = r.delay(expression, &None).unwrap();
    let name = r.store.symbol("captured");
    let param = r.store.symbol("unused");
    let body = r.store.intern(&Term::Var("captured".into()));
    let param_type = r.store.intern(&nat);
    let mut root = leaf.clone();
    for _ in 0..count {
        root = r
            .thunk(Thunk::Done(Value::Lambda {
                param,
                param_type,
                body,
                env: bind(None, name, root),
            }))
            .unwrap();
    }
    (root, leaf)
}
#[test]
fn unchanged_unsettled_capture_graph_is_not_retraversed() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, _) = graph(&mut r, 16);
    let mut remaining = 256;
    let first = r.binding_key_state(&root, 32, &mut remaining);
    assert!(!first.1);
    let work = r.work.as_ref().unwrap().capture_key_entries;
    assert!(work >= 16);
    for _ in 0..64 {
        let mut again = 256;
        assert_eq!(r.binding_key_state(&root, 32, &mut again), first);
        assert_eq!(
            again, remaining,
            "memo hits must preserve traversal-budget accounting"
        );
    }
    assert_eq!(
        r.work.as_ref().unwrap().capture_key_entries,
        work,
        "unchanged delayed captures must not rebuild the same semantic key"
    );
}
#[test]
fn forcing_a_capture_invalidates_unsettled_key_observations() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, leaf) = graph(&mut r, 8);
    let before = r.binding_key_state(&root, 32, &mut 256);
    assert!(!before.1);
    assert!(r.force(leaf).is_some());
    let after = r.binding_key_state(&root, 32, &mut 256);
    assert!(
        after.1,
        "the enclosing closure must observe its settled capture"
    );
    assert_ne!(before.0, after.0);
}
#[test]
fn different_key_probe_budgets_remain_distinct_and_do_not_gain_fuel() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, _) = graph(&mut r, 8);
    let mut small = 1;
    let shallow = r.binding_key_state(&root, 1, &mut small);
    assert_eq!(small, 0);
    let mut large = 256;
    let deep = r.binding_key_state(&root, 32, &mut large);
    assert_ne!(shallow.0, deep.0, "more budget may refine an unsettled key");
    let mut empty = 0;
    assert_eq!(r.binding_key_state(&root, 0, &mut empty), (root.id, false));
    assert_eq!(empty, 0);
    let mut repeat = 256;
    assert_eq!(r.binding_key_state(&root, 32, &mut repeat), deep);
    assert_eq!(repeat, large);
}

#[test]
fn revision_overflow_disables_unsettled_observation_reuse() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, leaf) = graph(&mut r, 8);
    let first = r.binding_key_state(&root, 32, &mut 256);
    assert!(!first.1);
    r.key_revision = u64::MAX;
    let work = r.work.as_ref().unwrap().capture_key_entries;
    assert_eq!(r.binding_key_state(&root, 32, &mut 256), first);
    assert_eq!(r.binding_key_state(&root, 32, &mut 256), first);
    assert!(r.work.as_ref().unwrap().capture_key_entries >= work + 16);
    assert!(r.force(leaf).is_some());
    assert_eq!(
        r.key_revision,
        u64::MAX,
        "generation must not wrap into an old observation"
    );
    assert!(r.binding_key_state(&root, 32, &mut 256).1);
}
#[test]
fn distinct_capture_values_do_not_share_an_observation() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (first, zero) = graph(&mut r, 1);
    let one_term = Term::App(
        Box::new(Term::Global("Succ".into())),
        Box::new(Term::Global("Zero".into())),
    );
    let one_term = r.store.intern(&one_term);
    let one = r.delay(one_term, &None).unwrap();
    let (param, param_type, body) = match &*first.state.borrow() {
        Thunk::Done(Value::Lambda {
            param,
            param_type,
            body,
            ..
        }) => (*param, *param_type, *body),
        _ => unreachable!(),
    };
    let captured = r.store.symbol("captured");
    let second = r
        .thunk(Thunk::Done(Value::Lambda {
            param,
            param_type,
            body,
            env: bind(None, captured, one.clone()),
        }))
        .unwrap();
    for _ in 0..2 {
        assert_ne!(
            r.binding_key_state(&first, 32, &mut 256).0,
            r.binding_key_state(&second, 32, &mut 256).0
        );
    }
    assert!(r.force(zero).is_some());
    assert!(r.force(one).is_some());
    let a = r.binding_key_state(&first, 32, &mut 256);
    let b = r.binding_key_state(&second, 32, &mut 256);
    assert!(a.1 && b.1);
    assert_ne!(a.0, b.0);
}

#[test]
fn sufficient_probe_allowances_reuse_completed_observation_work() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, _) = graph(&mut r, 16);
    let mut first_budget = 256;
    let first = r.binding_key_state(&root, 32, &mut first_budget);
    assert!(!first.1);
    let spent = 256 - first_budget;
    let work = r.work.as_ref().unwrap().capture_key_entries;
    assert!(work >= 16 && spent > 0 && spent < 256);
    for allowance in [512, spent, spent + 1, 128, 1024, spent, 257] {
        let mut remaining = allowance;
        assert_eq!(r.binding_key_state(&root, 32, &mut remaining), first);
        assert_eq!(
            remaining,
            allowance - spent,
            "reused observations retain every charged node"
        );
    }
    assert_eq!(
        r.work.as_ref().unwrap().capture_key_entries,
        work,
        "nonbinding allowance differences must not reconstruct identical capture keys"
    );
    let mut insufficient = spent - 1;
    assert_ne!(r.binding_key_state(&root, 32, &mut insufficient).0, first.0);
    assert_eq!(insufficient, 0);
}

#[test]
fn exhausted_allowances_and_different_depths_recompute_their_own_keys() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, _) = graph(&mut r, 16);
    let mut small = 4;
    let truncated = r.binding_key_state(&root, 32, &mut small);
    assert_eq!(small, 0);
    let complete = r.binding_key_state(&root, 32, &mut 256);
    assert_ne!(
        truncated.0, complete.0,
        "an exhausted observation cannot answer a larger probe"
    );
    let shallower = r.binding_key_state(&root, 8, &mut 256);
    assert_ne!(
        shallower.0, complete.0,
        "equal allowances cannot erase the depth boundary"
    );
    assert_eq!(r.binding_key_state(&root, 32, &mut 256), complete);
}

#[test]
fn unrelated_evaluation_does_not_rewalk_the_capture_graph() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, _leaf) = graph(&mut r, 16);
    let mut remaining = 256;
    let initial = r.binding_key_state(&root, 32, &mut remaining);
    assert!(!initial.1);
    let work = r.work.as_ref().unwrap().capture_key_entries;
    let unrelated = r.store.intern(&Term::Sort(crate::Universe::Type(0)));
    for _ in 0..64 {
        let thunk = r.thunk(Thunk::Suspended(unrelated, None)).unwrap();
        assert!(r.force(thunk).is_some());
        let mut budget = 256;
        assert_eq!(r.binding_key_state(&root, 32, &mut budget), initial);
        assert_eq!(budget, remaining);
    }
    assert!(
        r.work.as_ref().unwrap().capture_key_entries <= work + 16 * 8,
        "unrelated evaluation must preserve most completed capture-key observations: {} entries",
        r.work.as_ref().unwrap().capture_key_entries
    );
}

#[test]
fn cached_child_observations_still_contribute_their_dependencies() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, leaf) = graph(&mut r, 8);
    let child = match &*root.state.borrow() {
        Thunk::Done(Value::Lambda { env, .. }) => lookup(env, r.store.symbol("captured")).unwrap(),
        _ => unreachable!(),
    };
    assert!(!r.binding_key_state(&child, 31, &mut 255).1);
    let work = r.work.as_ref().unwrap().capture_key_entries;
    let before = r.binding_key_state(&root, 32, &mut 256);
    assert!(!before.1);
    assert_eq!(
        r.work.as_ref().unwrap().capture_key_entries,
        work + 1,
        "the outer observation must actually consume the child's cached key"
    );
    assert!(r.force(leaf).is_some());
    let after = r.binding_key_state(&root, 32, &mut 256);
    assert!(after.1);
    assert_ne!(after.0, before.0);
}
#[test]
fn publishing_a_stable_key_invalidates_an_earlier_depth_cutoff() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, leaf) = graph(&mut r, 1);
    assert!(r.force(leaf.clone()).is_some());
    let before = r.binding_key_state(&root, 1, &mut 256);
    assert!(!before.1);
    assert!(r.binding_key_state(&leaf, 32, &mut 256).1);
    let after = r.binding_key_state(&root, 1, &mut 256);
    assert!(after.1);
    assert_ne!(after.0, before.0);
}
#[test]
fn dependency_bucket_collisions_only_discard_optional_reuse() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, leaf) = graph(&mut r, 8);
    let before = r.binding_key_state(&root, 32, &mut 256);
    let unrelated = r.store.intern(&Term::Sort(crate::Universe::Type(0)));
    let collision = (0..4096)
        .find_map(|_| {
            let t = r.thunk(Thunk::Suspended(unrelated, None)).unwrap();
            (dependency_bucket(t.id) == dependency_bucket(leaf.id)).then_some(t)
        })
        .expect("find a different identity sharing the conservative bucket");
    assert_ne!(collision.id, leaf.id);
    let work = r.work.as_ref().unwrap().capture_key_entries;
    assert!(r.force(collision).is_some());
    assert_eq!(r.binding_key_state(&root, 32, &mut 256), before);
    assert!(
        r.work.as_ref().unwrap().capture_key_entries > work,
        "a collision must recompute rather than discard a possible dependency"
    );
    assert!(r.force(leaf).is_some());
    assert!(r.binding_key_state(&root, 32, &mut 256).1);
}

#[test]
fn sufficient_depth_allowances_reuse_a_completed_capture_walk() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, _) = graph(&mut r, 16);
    let mut remaining = 256;
    let initial = r.binding_key_state(&root, 32, &mut remaining);
    let work = r.work.as_ref().unwrap().capture_key_entries;
    for depth in [64, 17, 32, 31, 18, 128, 17] {
        let mut budget = 256;
        assert_eq!(r.binding_key_state(&root, depth, &mut budget), initial);
        assert_eq!(budget, remaining);
    }
    assert_eq!(
        r.work.as_ref().unwrap().capture_key_entries,
        work,
        "sufficient depth allowances must not reconstruct the same observed graph"
    );
    assert_ne!(
        r.binding_key_state(&root, 16, &mut 256).0,
        initial.0,
        "a shallower allowance must keep its own cutoff identity"
    );
}

#[test]
fn depth_reuse_cannot_launder_an_exhausted_node_allowance() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let (root, _) = graph(&mut r, 16);
    let mut small = 4;
    let truncated = r.binding_key_state(&root, 32, &mut small);
    assert_eq!(small, 0);
    let mut shallow = 4;
    assert_eq!(r.binding_key_state(&root, 4, &mut shallow), truncated);
    assert_eq!(shallow, 0);
    let mut another = 5;
    assert_eq!(r.binding_key_state(&root, 4, &mut another), truncated);
    assert_eq!(another, 1);
    let mut larger = 5;
    assert_ne!(r.binding_key_state(&root, 32, &mut larger).0, truncated.0);
    assert_eq!(larger, 0);
    let mut zero_depth_budget = 256;
    assert_eq!(
        r.binding_key_state(&root, 0, &mut zero_depth_budget),
        (root.id, false)
    );
    assert_eq!(zero_depth_budget, 256);
}
