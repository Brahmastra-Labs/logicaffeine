use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn materialize(c: &dyn LocalTypes) -> Vec<Db> {
    (0..c.len())
        .map(|i| c.binding(i).unwrap().clone())
        .collect()
}
#[test]
fn extending_a_scope_retains_predecessor_type_storage() {
    let mut root = vec![g("Nat"), Db::Var(0)];
    root.push(Db::Pi(DbBox::new(g("Nat")), DbBox::new(Db::Var(1))));
    let mut outer = extend_local(&root);
    outer.push(Db::Var(1));
    let mut inner = extend_local(&outer);
    inner.push(Db::Var(0));
    for i in 0..root.len() {
        assert!(std::ptr::eq(
            root.binding(i).unwrap(),
            outer.binding(i).unwrap()
        ));
        assert!(std::ptr::eq(
            root.binding(i).unwrap(),
            inner.binding(i).unwrap()
        ));
    }
    assert!(std::ptr::eq(
        outer.binding(3).unwrap(),
        inner.binding(3).unwrap()
    ));
    assert_eq!(root.len(), 3);
    assert_eq!(outer.len(), 4);
    assert_eq!(inner.len(), 5);
    assert!(inner.binding(5).is_none());
    assert!(inner.binding(usize::MAX).is_none());
}
#[test]
fn entering_scope_does_not_materialize_ancestor_bindings() {
    let root = vec![g("Nat"); 128];
    let mut child = extend_local(&root);
    assert_eq!(child.bindings.len(), 0);
    assert_eq!(child.bindings.capacity(), 0);
    child.push(g("Bool"));
    assert_eq!(child.bindings.len(), 1);
    assert_eq!(child.len(), 129);
    assert_eq!(child.binding(0), root.binding(0));
    assert_eq!(child.binding(128), Some(&g("Bool")));
}
#[test]
fn dependent_lookup_matches_flat_telescope_at_every_index() {
    let root = vec![Db::Sort(Universe::Type(0)), Db::Var(0)];
    let mut first = extend_local(&root);
    first.push(Db::Pi(DbBox::new(Db::Var(1)), DbBox::new(Db::Var(2))));
    first.push(Db::App(DbBox::new(Db::Var(0)), DbBox::new(Db::Var(1))));
    let mut second = extend_local(&first);
    second.push(Db::Var(3));
    let flat = materialize(&second);
    for k in 0..=flat.len() + 1 {
        assert_eq!(
            format!("{:?}", var_type(&second, k)),
            format!("{:?}", var_type(&flat, k))
        );
    }
    assert_eq!(var_type(&second, 0).unwrap(), Db::Var(4));
    assert_eq!(var_type(&second, 3).unwrap(), Db::Var(4));
}
#[test]
fn empty_segments_and_sibling_scopes_preserve_binding_order() {
    let root = [g("A"), g("B")];
    let empty = extend_local(&root);
    let mut left = extend_local(&empty);
    left.push(g("L"));
    let mut right = extend_local(&empty);
    right.push(g("R"));
    right.push(Db::Var(0));
    assert_eq!(materialize(&left), vec![g("A"), g("B"), g("L")]);
    assert_eq!(
        materialize(&right),
        vec![g("A"), g("B"), g("R"), Db::Var(0)]
    );
    assert_eq!(materialize(&empty), root);
}
#[test]
fn nested_binder_inference_and_conversion_match_flat_scopes() {
    let c = Context::new();
    let root = [Db::Sort(Universe::Type(0))];
    let mut ext = extend_local(&root);
    ext.push(Db::Var(0));
    let flat = materialize(&ext);
    let terms = [
        Db::Var(0),
        Db::Var(1),
        Db::Lam(DbBox::new(Db::Var(1)), DbBox::new(Db::Var(0))),
        Db::Pi(DbBox::new(Db::Var(1)), DbBox::new(Db::Var(2))),
        Db::Let(
            DbBox::new(Db::Var(1)),
            DbBox::new(Db::Var(0)),
            DbBox::new(Db::Var(0)),
        ),
    ];
    for a in &terms {
        assert_eq!(
            format!("{:?}", infer(&c, &ext, a)),
            format!("{:?}", infer(&c, &flat, a))
        );
        for b in &terms {
            assert_eq!(def_eq(&c, &ext, a, b), def_eq(&c, &flat, a, b));
        }
    }
}
#[test]
fn failed_branch_check_does_not_change_parent_context() {
    let c = Context::new();
    let root = [Db::Sort(Universe::Type(0))];
    let original = root.clone();
    let ext = extend_local(&root);
    let case = Db::Lam(DbBox::new(Db::Var(0)), DbBox::new(Db::Var(99)));
    let ty = Db::Pi(DbBox::new(Db::Var(0)), DbBox::new(Db::Var(1)));
    assert!(check_case(&c, &ext, &case, &ty).is_err());
    assert_eq!(root, original);
    assert_eq!(materialize(&ext), original);
}
