use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn peano(n: usize, tail: Term) -> Term {
    (0..n).fold(tail, |t, _| app(g("Succ"), t))
}
fn base() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    c.add_constructor("false", "Bool", g("Bool"));
    c.add_constructor("true", "Bool", g("Bool"));
    c
}
#[test]
fn deep_peano_typing_inside_a_shallow_kernel_call_does_not_recurse_on_the_prefix() {
    let c = base();
    let shallow = g("Zero");
    let _outer = enter_kernel(&shallow, "peano control").unwrap();
    for n in [256, 512] {
        assert_eq!(infer_type(&c, &peano(n, g("Zero"))).unwrap(), g("Nat"));
    }
}
#[test]
fn peano_typing_checks_foreign_hole_and_open_tails() {
    let mut c = base();
    c.add_declaration("open_natural", g("Nat"));
    let shallow = g("Zero");
    let _outer = enter_kernel(&shallow, "peano tail control").unwrap();
    assert_eq!(
        infer_type(&c, &peano(256, g("open_natural"))).unwrap(),
        g("Nat")
    );
    assert!(infer_type(&c, &peano(256, g("true"))).is_err());
    assert!(infer_type(&c, &peano(256, Term::Hole)).is_err());
    assert!(infer_type(&c, &peano(256, v("unbound"))).is_err());
}
#[test]
fn rebound_successor_uses_its_actual_declared_function_type() {
    let mut c = base();
    c.declare(
        "Succ",
        pi("n", g("Nat"), g("Bool")),
        lam("n", g("Nat"), g("false")),
    )
    .unwrap();
    assert_eq!(
        infer_type(&c, &app(g("Succ"), g("Zero"))).unwrap(),
        g("Bool")
    );
    assert!(infer_type(&c, &app(g("Succ"), g("true"))).is_err());
    let mut c = base();
    c.add_constructor("Succ", "Nat", pi("b", g("Bool"), g("Nat")));
    assert_eq!(
        infer_type(&c, &app(g("Succ"), g("true"))).unwrap(),
        g("Nat")
    );
    assert!(infer_type(&c, &app(g("Succ"), g("Zero"))).is_err());
}
#[test]
fn registered_rewrite_contexts_keep_actual_tail_checking() {
    let mut c = base();
    c.add_declaration(
        "Eq",
        pi(
            "A",
            Term::Sort(Universe::Type(0)),
            pi("x", v("A"), pi("y", v("A"), Term::Sort(Universe::Prop))),
        ),
    );
    c.add_declaration("opaque", pi("n", g("Nat"), g("Nat")));
    let eq = app(
        app(app(g("Eq"), g("Nat")), app(g("opaque"), g("Zero"))),
        g("Zero"),
    );
    c.add_declaration("rewrite_zero", eq);
    c.add_simp_lemma("rewrite_zero").unwrap();
    assert_eq!(infer_type(&c, &peano(8, g("Zero"))).unwrap(), g("Nat"));
    assert!(infer_type(&c, &peano(8, g("true"))).is_err());
}
#[test]
fn computed_peano_prefixes_enforce_the_existing_whole_term_depth_ceiling() {
    let c = base();
    let shallow = g("Zero");
    let _outer = enter_kernel(&shallow, "computed depth control").unwrap();
    let excessive = peano(MAX_TERM_DEPTH + 1, g("Zero"));
    assert!(matches!(
        infer_type(&c, &excessive),
        Err(KernelError::CertificationError(_))
    ));
}

#[test]
fn successive_peano_prefixes_use_constant_inference_entries() {
    let c = base();
    let shallow = g("Zero");
    let _outer = enter_kernel(&shallow, "peano work control").unwrap();
    for n in [16, 32, 256, 512] {
        PEANO_INFERENCE_ENTRIES.with(|count| count.set(0));
        assert_eq!(infer_type(&c, &peano(n, g("Zero"))).unwrap(), g("Nat"));
        let entries = PEANO_INFERENCE_ENTRIES.with(|count| count.get());
        assert!(entries <= 4, "prefix {n} used {entries} inference entries");
    }
}

#[test]
fn peano_prefix_checks_locally_bound_natural_and_foreign_tails() {
    let c = base()
        .extend("natural", g("Nat"))
        .extend("boolean", g("Bool"));
    let shallow = g("Zero");
    let _outer = enter_kernel(&shallow, "local tail control").unwrap();
    assert_eq!(infer_type(&c, &peano(256, v("natural"))).unwrap(), g("Nat"));
    assert!(infer_type(&c, &peano(256, v("boolean"))).is_err());
}
#[test]
fn peano_depth_ceiling_counts_the_complete_nonpeano_tail() {
    let c = base();
    let shallow = g("Zero");
    let _outer = enter_kernel(&shallow, "whole computed depth control").unwrap();
    let tail = (0..801).fold(g("Zero"), |body, _| lam("x", g("Nat"), body));
    let excessive = peano(800, tail);
    assert!(matches!(
        infer_type(&c, &excessive),
        Err(KernelError::CertificationError(_))
    ));
    let shallow_tail = lam("x", g("Nat"), g("Zero"));
    assert!(matches!(
        infer_type(&c, &peano(8, shallow_tail)),
        Err(KernelError::TypeMismatch { .. })
    ));
}
