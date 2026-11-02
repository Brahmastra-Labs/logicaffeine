use super::*;

#[test]
fn identical_structure_is_one_id() {
    let mut s = Store::new();
    let p = s.prop();
    let b0 = s.bvar(0);
    let a = s.lam(p, b0);
    let b = s.lam(p, b0);
    assert_eq!(a, b);
    assert_eq!(s.loose_bvar_range(a), 0);
    assert_eq!(s.loose_bvar_range(b0), 1);
}

#[test]
fn instantiate_abstract_round_trip() {
    let mut s = Store::new();
    let p = s.prop();
    let x = s.fvar(7);
    let y = s.fvar(8);
    let body = s.app(x, y);
    let lam_body = s.abstract_fvars(body, &[7, 8]);
    assert_eq!(
        s.node(lam_body),
        Node::App(ExprId(s.bvar(1).0), ExprId(s.bvar(0).0))
    );
    let back = s.instantiate_rev(lam_body, &[x, y]);
    assert_eq!(back, body);
    let pi = s.pi(p, lam_body);
    assert_eq!(s.loose_bvar_range(pi), 1);
}

#[test]
fn instantiate_under_binder_lifts_the_value() {
    let mut s = Store::new();
    let p = s.prop();
    let b0 = s.bvar(0);
    let b1 = s.bvar(1);
    let inner = s.app(b1, b0);
    let lam = s.lam(p, inner);
    let v = s.bvar(3);
    let r = s.instantiate1(lam, v);
    let expect_v = s.bvar(4);
    let expect_inner = s.app(expect_v, b0);
    let expect = s.lam(p, expect_inner);
    assert_eq!(r, expect);
}

#[test]
fn a_structure_has_one_id_across_regions() {
    let mut s = Store::new();
    let p = s.prop();
    s.begin_scratch();
    let b0 = s.bvar(0);
    let lam_scratch = s.lam(p, b0);
    assert!(lam_scratch.is_scratch());
    let (r, persistent) = s.with_persistent(|s| {
        let b = s.bvar(0);
        s.lam(p, b)
    });
    assert_eq!(
        r, lam_scratch,
        "a persistent computation reuses the scratch node, never duplicates it"
    );
    assert!(!persistent, "and reports its result as not cacheable");
    let (q, persistent) = s.with_persistent(|s| {
        let b = s.bvar(7);
        s.pi(p, b)
    });
    assert!(persistent && !q.is_scratch());
    s.end_scratch();
    let b7 = s.bvar(7);
    assert_eq!(
        s.pi(p, b7),
        q,
        "a persistent result survives the declaration"
    );
    s.begin_scratch();
    let again = s.pi(p, b7);
    assert_eq!(
        again, q,
        "scratch interning finds the persistent node first"
    );
    s.end_scratch();
}

#[test]
fn scratch_nodes_are_dropped() {
    let mut s = Store::new();
    let p = s.prop();
    let before = s.expr_count();
    s.begin_scratch();
    let b0 = s.bvar(0);
    let _ = s.lam(p, b0);
    assert!(s.expr_count() > before);
    s.end_scratch();
    assert_eq!(s.expr_count(), before);
    let b0 = s.bvar(0);
    let l1 = s.lam(p, b0);
    let l2 = s.lam(p, b0);
    assert_eq!(l1, l2);
}
