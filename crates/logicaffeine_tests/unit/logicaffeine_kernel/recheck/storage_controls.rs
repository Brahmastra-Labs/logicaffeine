use super::*;

fn setup() {
    WHNF_DEPTH.with(|s| s.set(0));
    WHNF_FUEL.with(|s| s.set(MAX_RECHECK_STEPS));
    WHNF_EXHAUSTED.with(|s| s.set(false));
}

#[test]
fn lowered_lambda_clone_retains_immutable_child_storage() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let term = Term::Lambda {
        param: "n".into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(Term::Var("n".into())),
    };
    let original = to_db(&term, &mut Vec::new()).unwrap();
    let copy = original.clone();
    let (Db::Lam(a, b), Db::Lam(c, d)) = (&original, &copy) else {
        panic!("lambda lowering")
    };
    assert!(
        std::ptr::eq(a.as_ref(), c.as_ref()),
        "cloning must retain the immutable domain"
    );
    assert!(
        std::ptr::eq(b.as_ref(), d.as_ref()),
        "cloning must retain the immutable open body"
    );
}

#[test]
fn owned_opening_and_shifting_retain_unaffected_shared_domains() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let original = Db::Lam(DbBox::new(Db::Global("Nat".into())), DbBox::new(Db::Var(1)));
    let Db::Lam(domain, _) = &original else {
        panic!()
    };
    let ptr = domain.as_ref() as *const Db;
    let argument = Db::Global("zero".into());
    let results = [
        beta_open_owned(original.clone(), &argument),
        beta_open_consuming(original.clone(), argument.clone()),
        beta_open_deferred(original.clone(), argument.clone()),
        shift_owned(original.clone(), 1, 0),
    ];
    for (i, result) in results.iter().enumerate() {
        let Db::Lam(domain, body) = result else {
            panic!()
        };
        assert_eq!(
            domain.as_ref() as *const Db,
            ptr,
            "unchanged shared domain in operation {i}"
        );
        assert_eq!(body.as_ref(), if i == 3 { &Db::Var(2) } else { &argument });
    }
    assert!(matches!(&original,Db::Lam(_,body) if body.as_ref()==&Db::Var(1)));
}
