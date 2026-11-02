// Proposals only: not executed in this local review.
fn var(name: &str) -> crate::Term {
    crate::Term::Var(name.into())
}
fn sort() -> crate::Term {
    crate::Term::Sort(crate::Universe::Prop)
}
fn app(a: crate::Term, b: crate::Term) -> crate::Term {
    crate::Term::App(Box::new(a), Box::new(b))
}
fn lam(name: &str, body: crate::Term) -> crate::Term {
    crate::Term::Lambda {
        param: name.into(),
        param_type: Box::new(sort()),
        body: Box::new(body),
    }
}
fn equivalent(body: crate::Term, pairs: Vec<(&str, crate::Term)>) {
    let mut regular = super::TermStore::new();
    let mut bounded = super::TermStore::new();
    let a = regular.intern(&body);
    let b = bounded.intern(&body);
    let mut ap = Vec::new();
    let mut bp = Vec::new();
    for (name, value) in &pairs {
        let ar = regular.intern(value);
        let br = bounded.intern(value);
        ap.push((regular.symbol(name), ar));
        bp.push((bounded.symbol(name), br));
    }
    let expected = regular.instantiate(a, &ap);
    let actual = bounded
        .instantiate_bounded(b, &bp, &mut u64::MAX, 100_000, &mut 1_000_000)
        .unwrap();
    let expected = regular.extract_bounded(expected, 100_000).unwrap();
    let actual = bounded.extract_bounded(actual, 100_000).unwrap();
    // Separate stores avoid the ordinary substitution hitting bounded substitution's memo.
    let mut comparison = super::TermStore::new();
    let a = comparison.intern(&expected);
    let b = comparison.intern(&actual);
    assert!(comparison.alpha_same(a, b));
}

#[test]
fn nested_shadowing_and_simultaneous_ranges_match_existing_substitution() {
    equivalent(
        lam("y", app(var("x"), lam("x", app(var("x"), var("z"))))),
        vec![("x", var("y")), ("z", var("x")), ("y", var("z"))],
    );
}

#[test]
fn let_domain_and_value_are_outside_the_shadowed_body() {
    equivalent(
        crate::Term::Let {
            name: "x".into(),
            ty: Box::new(var("x")),
            value: Box::new(var("x")),
            body: Box::new(app(var("x"), var("y"))),
        },
        vec![("x", var("replacement")), ("y", var("x"))],
    );
}

#[test]
fn mutual_fix_freshening_uses_whole_block_and_growing_substitution_range() {
    equivalent(
        crate::Term::MutualFix {
            defs: vec![
                ("f".into(), app(var("x"), var("g"))),
                ("g".into(), app(var("f✦1"), app(var("f"), var("x")))),
            ],
            index: 0,
        },
        vec![("x", app(var("f"), var("g")))],
    );
}

#[test]
fn fresh_name_collision_preserves_every_original_free_name() {
    let body = lam("y", app(var("x"), var("y✦1")));
    equivalent(body.clone(), vec![("x", var("y"))]);
    let mut store = super::TermStore::new();
    let body = store.intern(&body);
    let replacement = store.mk_var("y");
    let x = store.symbol("x");
    let result = store
        .instantiate_bounded(body, &[(x, replacement)], &mut u64::MAX, 1000, &mut 100_000)
        .unwrap();
    let free = store.free_vars(result);
    assert!(free.contains(&"y") && free.contains(&"y✦1"));
    assert!(!free.contains(&"x"));
}

#[test]
fn work_and_allocation_exhaustion_return_no_root_before_initial_mutation() {
    for no_work in [true, false] {
        let mut store = super::TermStore::new();
        let x = store.mk_var("x");
        let y = store.mk_var("y");
        let name = store.symbol("x");
        let before = (
            store.nodes.len(),
            store.sets.len(),
            store.substs.len(),
            store.subst_cache.len(),
        );
        let mut work = if no_work { 0 } else { u64::MAX };
        let mut allocation = if no_work { 100_000 } else { 0 };
        let result = store.instantiate_bounded(x, &[(name, y)], &mut work, 1000, &mut allocation);
        assert_eq!(
            result,
            Err(if no_work {
                super::StoreOperationLimit::Work
            } else {
                super::StoreOperationLimit::Allocation
            })
        );
        assert_eq!(
            before,
            (
                store.nodes.len(),
                store.sets.len(),
                store.substs.len(),
                store.subst_cache.len()
            )
        );
    }
}

#[test]
fn node_exhaustion_returns_no_partial_root_and_never_exceeds_node_cap() {
    let mut store = super::TermStore::new();
    let body = store.intern(&app(var("x"), sort()));
    let y = store.mk_var("y");
    let x = store.symbol("x");
    let limit = store.len();
    let result = store.instantiate_bounded(body, &[(x, y)], &mut u64::MAX, limit, &mut 100_000);
    assert_eq!(result, Err(super::StoreOperationLimit::Nodes));
    assert_eq!(store.len(), limit);
}

#[test]
fn tiny_original_node_is_charged_for_large_replacement_metadata_before_rebuild() {
    let mut store = super::TermStore::new();
    let x = store.mk_var("x");
    let f = store.mk_var("f");
    let body = store.mk_app(f, x);
    let mut range = store.mk_var("free0");
    for i in 1..64 {
        let next = store.mk_var(&format!("free{i}"));
        range = store.mk_app(range, next);
    }
    let before = (store.nodes.len(), store.sets.len());
    // Old charge was original tree_size=3. This must fail from actual replacement metadata.
    assert_eq!(
        store.admit_node_metadata(&super::Node::App(f, range), &mut 64, &mut 1_000_000),
        Err(super::StoreOperationLimit::Work)
    );
    assert_eq!(before, (store.nodes.len(), store.sets.len()));
    assert_eq!(
        store.admit_node_metadata(&super::Node::App(f, range), &mut u64::MAX, &mut 64),
        Err(super::StoreOperationLimit::Allocation)
    );
    assert_eq!(before, (store.nodes.len(), store.sets.len()));
}

#[test]
fn wide_shared_children_are_refused_before_worklist_expansion() {
    let mut store = super::TermStore::new();
    let x = store.mk_var("x");
    let y = store.mk_var("y");
    let body = store.mk_match(x, x, &vec![x; 256]);
    let name = store.symbol("x");
    let before = store.len();
    let result = store.instantiate_bounded(body, &[(name, y)], &mut u64::MAX, 32, &mut 1_000_000);
    assert_eq!(result, Err(super::StoreOperationLimit::Nodes));
    assert_eq!(store.len(), before);
}

#[test]
fn mid_traversal_work_stop_exposes_no_root_even_with_partial_memo_entries() {
    let mut witnessed = false;
    for fuel in 1..2000 {
        let mut store = super::TermStore::new();
        let body = store.intern(&app(var("x"), sort()));
        let y = store.mk_var("y");
        let x = store.symbol("x");
        let mut work = fuel;
        let result = store.instantiate_bounded(body, &[(x, y)], &mut work, 1000, &mut 100_000);
        if result == Err(super::StoreOperationLimit::Work) && !store.subst_cache.is_empty() {
            assert!(store.len() <= 1000);
            witnessed = true;
            break;
        }
    }
    assert!(
        witnessed,
        "exercise an admitted child followed by refusal before the rebuilt root"
    );
}
