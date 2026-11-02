use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn pi(depth: usize) -> Db {
    let mut body = Db::Sort(Universe::Type(0));
    for _ in 0..depth {
        body = Db::Pi(DbBox::new(Db::Sort(Universe::Type(0))), DbBox::new(body));
    }
    body
}
fn measured(f: impl FnOnce() -> Db) -> (Db, usize) {
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    let result = f();
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    (result, copies)
}
struct Restore(usize, usize, bool);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|v| v.set(self.0));
        WHNF_FUEL.with(|v| v.set(self.1));
        WHNF_EXHAUSTED.with(|v| v.set(self.2));
    }
}
fn state() -> Restore {
    Restore(
        WHNF_DEPTH.with(|v| v.get()),
        WHNF_FUEL.with(|v| v.get()),
        WHNF_EXHAUSTED.with(|v| v.get()),
    )
}
#[test]
fn owned_head_keeps_a_large_completed_type_without_copying() {
    let c = Context::new();
    let value = pi(64);
    let expected = value.clone();
    let (result, copies) = measured(|| whnf_in_owned(&c, &[], value));
    assert_eq!(result, expected);
    assert_eq!(copies, 0, "freshly owned result must not be copied");
}
#[test]
fn borrowed_head_still_returns_an_independent_owned_result() {
    let c = Context::new();
    let value = pi(64);
    let (result, copies) = measured(|| whnf_in(&c, &[], &value));
    assert_eq!(result, value);
    assert_eq!(copies, 129);
}
#[test]
fn owned_and_borrowed_beta_zeta_preserve_outer_indices() {
    let c = Context::new();
    let values = [
        Db::App(
            DbBox::new(Db::Lam(
                DbBox::new(g("Nat")),
                DbBox::new(Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(1)))),
            )),
            DbBox::new(Db::Var(0)),
        ),
        Db::Let(
            DbBox::new(g("Nat")),
            DbBox::new(Db::Var(0)),
            DbBox::new(Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(1)))),
        ),
    ];
    for value in values {
        let borrowed = whnf_in(&c, &[g("Nat")], &value);
        let owned = whnf_in_owned(&c, &[g("Nat")], value);
        assert_eq!(owned, borrowed);
        assert_eq!(owned, Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(1))));
    }
}
#[test]
fn owned_nested_exhaustion_preserves_fuel_and_original_value() {
    let _restore = state();
    let c = Context::new();
    let value = pi(32);
    for owned in [false, true] {
        WHNF_DEPTH.with(|v| v.set(1));
        WHNF_FUEL.with(|v| v.set(0));
        WHNF_EXHAUSTED.with(|v| v.set(false));
        let input = value.clone();
        let (result, copies) = measured(|| {
            if owned {
                whnf_in_owned(&c, &[], input)
            } else {
                whnf_in(&c, &[], &input)
            }
        });
        assert_eq!(result, value);
        assert!(WHNF_EXHAUSTED.with(|v| v.get()));
        assert_eq!(WHNF_FUEL.with(|v| v.get()), 0);
        assert_eq!(WHNF_DEPTH.with(|v| v.get()), 1);
        if owned {
            assert_eq!(copies, 0);
        }
    }
}
#[test]
fn owned_depth_refusal_restores_depth_without_copying() {
    let _restore = state();
    let c = Context::new();
    let value = pi(32);
    let expected = value.clone();
    WHNF_DEPTH.with(|v| v.set(MAX_RECHECK_WHNF_DEPTH));
    WHNF_EXHAUSTED.with(|v| v.set(false));
    let (result, copies) = measured(|| whnf_in_owned(&c, &[], value));
    assert_eq!(result, expected);
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
    assert_eq!(WHNF_DEPTH.with(|v| v.get()), MAX_RECHECK_WHNF_DEPTH);
    assert_eq!(copies, 0);
}
#[test]
fn inferred_sort_moves_its_fresh_type_into_whnf() {
    let c = Context::new();
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    let result = infer_sort(&c, &[], &Db::Sort(Universe::Type(0))).unwrap();
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    assert_eq!(result, Universe::Type(1));
    assert_eq!(copies, 0);
}
