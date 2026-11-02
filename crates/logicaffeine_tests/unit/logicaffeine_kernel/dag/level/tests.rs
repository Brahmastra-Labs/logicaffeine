use super::*;
use crate::dag::name::Names;

fn setup() -> (Levels, LevelId, LevelId) {
    let mut names = Names::new();
    let (u, v) = (names.dotted("u"), names.dotted("v"));
    let mut ls = Levels::new();
    let (pu, pv) = (ls.param(u), ls.param(v));
    (ls, pu, pv)
}

#[test]
fn max_is_commutative_associative_idempotent() {
    let (mut ls, u, v) = setup();
    let (a, b) = (ls.max_raw(u, v), ls.max_raw(v, u));
    assert!(ls.equiv(a, b));
    let one = ls.of_nat(1);
    let x = ls.max_raw(a, one);
    let w = ls.max_raw(v, one);
    let z = ls.max_raw(u, w);
    assert!(ls.equiv(x, z));
    let uu = ls.max_raw(u, u);
    assert!(ls.equiv(uu, u));
}

#[test]
fn imax_vanishes_only_with_its_right_side() {
    let (mut ls, u, v) = setup();
    let i = ls.imax_raw(u, v);
    let m = ls.max_raw(u, v);
    assert!(ls.leq(i, m));
    assert!(!ls.leq(m, i), "max u v ≰ imax u v at v = 0, u = 1");
    let sv = ls.succ(v);
    let i1 = ls.imax_raw(u, sv);
    let m1 = ls.max_raw(u, sv);
    assert!(ls.equiv(i1, m1));
    let iz = ls.imax_raw(u, LevelId::ZERO);
    assert!(ls.is_always_zero(iz));
    let iuu = ls.imax_raw(u, u);
    assert!(ls.equiv(iuu, u));
}

#[test]
fn offsets_compare_pointwise() {
    let (mut ls, u, v) = setup();
    let su = ls.succ(u);
    assert!(ls.leq(u, su));
    assert!(!ls.leq(su, u));
    assert!(!ls.leq(u, v));
    let one = ls.of_nat(1);
    let m = ls.max_raw(one, u);
    assert!(!ls.leq(m, u));
    assert!(ls.leq(m, su));
}
