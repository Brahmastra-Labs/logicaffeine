use super::*;
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}

#[test]
fn repeated_identity_hit_does_not_rewalk_unsettled_capture_graph() {
    let ctx = Context::new();
    let mut r = HeadReducer::new(&ctx, 10000);
    let x = r.store.symbol("x");
    let y = r.store.symbol("y");
    let outer = r.store.symbol("outer");
    let body = r.store.intern(&v("x"));
    let ty = r.store.intern(&g("Nat"));
    let suspended = r.store.intern(&g("opaque"));
    let mut capture = r.thunk(Thunk::Suspended(suspended, None)).unwrap();
    for _ in 0..16 {
        capture = r
            .thunk(Thunk::Done(Value::Lambda {
                param: y,
                param_type: ty,
                body,
                env: bind(None, x, capture),
            }))
            .unwrap();
    }
    let env = bind(None, outer, capture);
    let root = r.store.intern(&app(v("outer"), g("Zero")));
    let first = r.delay(root, &env).unwrap();
    ENV_LOOKUP_STEPS.with(|n| n.set(0));
    let fuel = r.fuel;
    for _ in 0..64 {
        let hit = r.delay(root, &env).unwrap();
        assert!(Rc::ptr_eq(&hit, &first));
    }
    assert_eq!(r.fuel, fuel);
    let lookups = ENV_LOOKUP_STEPS.with(Cell::get);
    assert_eq!(
        lookups, 64,
        "exact cache hits rewalked nested captures: {lookups}"
    );
}

#[test]
fn distinct_identity_captures_still_share_equal_semantic_keys() {
    let ctx = Context::new();
    let mut r = HeadReducer::new(&ctx, 10000);
    let x = r.store.symbol("x");
    let root = r.store.intern(&app(g("Succ"), v("x")));
    let seven = crate::BigInt::from_i64(7);
    let a = r.thunk(Thunk::Done(Value::Nat(seven.clone()))).unwrap();
    let b = r.thunk(Thunk::Done(Value::Nat(seven))).unwrap();
    let c = r
        .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(8))))
        .unwrap();
    assert_ne!(a.id, b.id);
    let first = r.delay(root, &bind(None, x, a)).unwrap();
    let equal = r.delay(root, &bind(None, x, b)).unwrap();
    let different = r.delay(root, &bind(None, x, c)).unwrap();
    assert!(Rc::ptr_eq(&first, &equal));
    assert!(!Rc::ptr_eq(&first, &different));
}

#[test]
fn identity_hit_preserves_running_suspension_and_fuel() {
    let ctx = Context::new();
    let mut r = HeadReducer::new(&ctx, 10000);
    let x = r.store.symbol("x");
    let input = r.thunk(Thunk::Unknown).unwrap();
    let env = bind(None, x, input);
    let root = r.store.intern(&app(g("Succ"), v("x")));
    let (first, fresh) = r.prepare_delay::<true>(root, &env).unwrap();
    assert!(fresh);
    let fuel = r.fuel;
    let (hit, fresh) = r.prepare_delay::<true>(root, &env).unwrap();
    assert!(!fresh);
    assert!(Rc::ptr_eq(&first, &hit));
    assert!(matches!(&*hit.state.borrow(), Thunk::Running));
    assert_eq!(r.fuel, fuel);
    assert!(r.force(hit).is_none());
}

#[test]
fn evicted_identity_entries_cannot_alias_new_capture_values() {
    let ctx = Context::new();
    let mut r = HeadReducer::new(&ctx, 10000);
    let x = r.store.symbol("x");
    let root = r.store.intern(&app(g("Succ"), v("x")));
    let a = r
        .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(7))))
        .unwrap();
    let first = r.delay(root, &bind(None, x, a.clone())).unwrap();
    r.closures.recent.clear();
    r.closures.previous.clear();
    let b = r
        .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(8))))
        .unwrap();
    let different = r.delay(root, &bind(None, x, b)).unwrap();
    let repeated = r.delay(root, &bind(None, x, a)).unwrap();
    assert!(!Rc::ptr_eq(&first, &different));
    assert!(!Rc::ptr_eq(&repeated, &different));
    assert_ne!(first.id, repeated.id);
}

#[test]
fn running_identity_does_not_hide_a_completed_semantic_alias() {
    let ctx = Context::new();
    let mut r = HeadReducer::new(&ctx, 10000);
    let x = r.store.symbol("x");
    let root = r.store.intern(&app(g("Succ"), v("x")));
    let capture = r
        .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(7))))
        .unwrap();
    let key = r.binding_key(&capture, 32, &mut 256);
    assert_ne!(key, capture.id);
    let running = r.thunk(Thunk::Running).unwrap();
    let completed = r
        .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(8))))
        .unwrap();
    r.closures.insert((root, vec![capture.id]), running);
    r.closures.insert((root, vec![key]), completed.clone());
    let (hit, fresh) = r
        .prepare_delay::<true>(root, &bind(None, x, capture))
        .unwrap();
    assert!(!fresh);
    assert!(Rc::ptr_eq(&hit, &completed));
    assert!(matches!(r.force(hit), Some(Value::Nat(n)) if n == crate::BigInt::from_i64(8)));
}
