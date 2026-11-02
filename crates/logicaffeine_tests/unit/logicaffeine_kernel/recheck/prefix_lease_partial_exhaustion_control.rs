use super::*;
#[test]
fn successful_leased_projection_followed_by_exhaustion_cannot_publish_partial_prefix() {
    fn g(n: &str) -> Db {
        Db::Global(n.into())
    }
    fn n(x: i64) -> Db {
        Db::Lit(Literal::Nat(x.into()))
    }
    fn app(f: Db, a: Db) -> Db {
        Db::App(DbBox::new(f), DbBox::new(a))
    }
    struct Restore(usize, usize, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            WHNF_DEPTH.with(|v| v.set(self.0));
            WHNF_FUEL.with(|v| v.set(self.1));
            WHNF_EXHAUSTED.with(|v| v.set(self.2));
        }
    }
    let _meter = Restore(
        WHNF_DEPTH.with(|v| v.replace(1)),
        WHNF_FUEL.with(|v| v.replace(MAX_RECHECK_STEPS)),
        WHNF_EXHAUSTED.with(|v| v.replace(false)),
    );
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let scope = PrefixScope::enter(&c);
    let mut unary = g("Zero");
    for _ in 0..8 {
        unary = app(g("Succ"), unary);
    }
    let bound = app(Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(0))), unary);
    let warm = app(app(g("natLe"), n(1)), bound.clone());
    assert_eq!(try_nat_order_db_in(&c, &[], &warm), Some(g("true")));
    let (seed, cursor) = prefix_lookup(&c, &bound).unwrap();
    let mut seed = Some(seed);
    let mut cursor = cursor.unwrap();
    assert_eq!(seed.as_ref().unwrap().observed, 1);
    cursor.count = 0.into(); // preceding cancellation consumed cached prefix
    assert!(cursor.expose(&c, &[]).is_some());
    prefix_observe(&mut seed, &cursor.count);
    assert_eq!(seed.as_ref().unwrap().observed, 2);
    assert!(cursor.tail.is_some(), "a later demanded step remains");
    WHNF_FUEL.with(|v| v.set(0));
    cursor.count = 0.into();
    assert!(cursor.expose(&c, &[]).is_none());
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
    prefix_publish(&c, &mut seed, &mut cursor);
    drop(cursor);
    drop(seed);
    assert!(
        PREFIX_TABLE
            .with(|v| v.borrow().entries[0].as_ref().unwrap().status.get() == PrefixStatus::Vacant),
        "completed earlier step cannot authorize exhausted publication"
    );
    WHNF_FUEL.with(|v| v.set(MAX_RECHECK_STEPS));
    WHNF_EXHAUSTED.with(|v| v.set(false));
    let misses = PREFIX_STATS.with(|v| v.get()[1]);
    let demand = app(app(g("natLe"), n(9)), bound);
    assert_eq!(try_nat_order_db_in(&c, &[], &demand), Some(g("false")));
    assert_eq!(
        PREFIX_STATS.with(|v| v.get()[1]),
        misses,
        "must recompute vacant value"
    );
    drop(scope);
    assert_eq!(PREFIX_MEMORY.with(|v| v.get().live), 0);
}
