use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn app(a: Db, b: Db) -> Db {
    Db::App(DbBox::new(a), DbBox::new(b))
}
fn fixed() -> Db {
    Db::Fix(FixBody::owned(Box::new(Db::Lam(
        DbBox::new(g("Nat")),
        DbBox::new(Db::Var(0)),
    ))))
}
fn copies<T>(f: impl FnOnce() -> T) -> (T, usize) {
    OWNED_FIX_CLONES.with(|c| c.set(Some(0)));
    let result = f();
    (result, OWNED_FIX_CLONES.with(|c| c.replace(None).unwrap()))
}
#[test]
fn structural_column_inspection_borrows_the_existing_recursive_body() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    for name in ["Nat_add", "Nat_mul"] {
        let term = to_db(ctx.get_definition_body(name).unwrap(), &mut Vec::new()).unwrap();
        assert!(matches!(term, Db::Fix(FixBody::Owned(_))));
        let partial = app(term.clone(), g("Zero"));
        let full = app(partial.clone(), Db::Var(0));
        let (a, ac) = copies(|| fix_structural_column(&ctx, &partial));
        let (b, bc) = copies(|| fix_structural_column(&ctx, &full));
        assert_eq!(a, None);
        assert_eq!(b, Some(1));
        assert_eq!((ac, bc), (0, 0));
    }
}
#[test]
fn ineligible_singleton_probe_does_not_copy_inferred_type_arguments() {
    let ctx = Context::new();
    let locals = [app(g("OpaqueFamily"), fixed())];
    let (result, count) = copies(|| k_constructor_db(&ctx, &locals, &Db::Var(0)));
    assert_eq!(result, None);
    assert_eq!(count, 0);
}
#[test]
fn ineligible_structure_eta_probe_borrows_its_application_spine() {
    let ctx = Context::new();
    let left = app(g("not_a_structure_constructor"), fixed());
    let (result, count) = copies(|| try_struct_eta_db(&ctx, &[], &left, &g("other")));
    assert_eq!(result, None);
    assert_eq!(count, 0);
}
#[test]
fn stuck_mutual_fix_guard_does_not_copy_the_definition_block() {
    let ctx = Context::new();
    let defs = vec![
        fixed(),
        Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(0))),
    ];
    let original = defs.clone();
    let storage = defs.as_ptr();
    let (result, count) = copies(|| {
        resume_whnf(
            &ctx,
            &[],
            WhnfFrame::Apply(DbBox::owned(Box::new(Db::Var(0)))),
            Db::MutualFix { defs, index: 1 },
        )
    });
    let WhnfStep::Ready(Db::App(head, argument)) = result else {
        panic!("neutral guard unfolded")
    };
    let Db::MutualFix { defs, index } = head.into_db() else {
        panic!("lost mutual block")
    };
    assert_eq!(index, 1);
    assert_eq!(defs, original);
    assert_eq!(defs.as_ptr(), storage);
    assert_eq!(*argument, Db::Var(0));
    assert_eq!(count, 0);
}
#[test]
fn constructor_guard_preserves_selected_mutual_body_and_argument() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    let defs = vec![
        Db::Lam(DbBox::new(g("Nat")), DbBox::new(g("Zero"))),
        Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(0))),
    ];
    for index in 0..defs.len() {
        let term = app(
            Db::MutualFix {
                defs: defs.clone(),
                index,
            },
            app(g("Succ"), g("Zero")),
        );
        let expected = if index == 0 {
            g("Zero")
        } else {
            app(g("Succ"), g("Zero"))
        };
        take_budget_exhausted();
        assert_eq!(whnf(&ctx, &term), expected);
        assert!(!take_budget_exhausted());
    }
}
