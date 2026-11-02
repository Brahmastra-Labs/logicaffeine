use super::*;
#[test]
fn independently_reduces_large_power_with_a_shared_work_budget() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let lit = |n| Db::Lit(Literal::Nat(crate::BigInt::from_u64(n)));
    let t = Db::App(
        DbBox::new(Db::App(
            DbBox::new(Db::Global("Nat_pow".into())),
            DbBox::new(lit(2)),
        )),
        DbBox::new(lit(32)),
    );
    struct Restore(usize, usize, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            WHNF_DEPTH.with(|x| x.set(self.0));
            WHNF_FUEL.with(|x| x.set(self.1));
            WHNF_EXHAUSTED.with(|x| x.set(self.2));
        }
    }
    let _restore = Restore(
        WHNF_DEPTH.with(|x| x.replace(1)),
        WHNF_FUEL.with(|x| x.replace(20_000)),
        WHNF_EXHAUSTED.with(|x| x.replace(false)),
    );
    assert!(def_eq(&c, &[], &t, &lit(4_294_967_296)));
    assert!(!WHNF_EXHAUSTED.with(|x| x.get()));
    assert!(WHNF_FUEL.with(|x| x.get()) > 0);
}
