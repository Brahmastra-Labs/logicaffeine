use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, infer_type, recheck, Context,
    Literal, Term,
};

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().cloned().fold(f, app)
}
fn op(name: &str, a: Term, b: Term) -> Term {
    apps(g(name), &[a, b])
}
fn eq(a: Term, b: Term) -> Term {
    apps(g("Eq"), &[g("Int"), a, b])
}
fn pi(name: &str, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(g("Int")),
        body_type: Box::new(body),
    }
}
fn lam(name: &str, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(g("Int")),
        body: Box::new(body),
    }
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn expected(name: &str) -> Term {
    match name {
        "add_zero" => pi("a", eq(op("add", v("a"), lit(0)), v("a"))),
        "mul_zero" => pi("a", eq(op("mul", v("a"), lit(0)), lit(0))),
        "mul_one" => pi("a", eq(op("mul", v("a"), lit(1)), v("a"))),
        "add_comm" => pi(
            "a",
            pi(
                "b",
                eq(op("add", v("a"), v("b")), op("add", v("b"), v("a"))),
            ),
        ),
        "mul_comm" => pi(
            "a",
            pi(
                "b",
                eq(op("mul", v("a"), v("b")), op("mul", v("b"), v("a"))),
            ),
        ),
        _ => unreachable!(),
    }
}
fn actual_body_is_a_closed_proof(name: &str) {
    let ctx = context();
    let required = expected(name);
    assert!(
        defeq_for_test(&ctx, ctx.get_global(name).unwrap(), &required),
        "{name} must preserve its existing public theorem signature"
    );
    let body = ctx
        .get_definition_body(name)
        .expect("the ring law must have an actual checked proof body");
    assert_ne!(
        body,
        &g(name),
        "a theorem reference cannot substitute for its proof"
    );
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, body).unwrap(),
        &required
    ));
    assert!(
        defeq_for_test(&ctx, &recheck(&ctx, body).unwrap(), &required),
        "the independent kernel must check the actual body against the exact public law"
    );
    let body_usage = axiom_dependencies(&ctx, body);
    let named_usage = axiom_dependencies_of(&ctx, name).unwrap();
    assert!(
        body_usage.axioms.is_empty() && body_usage.unresolved.is_empty(),
        "{body_usage:?}"
    );
    assert!(
        named_usage.axioms.is_empty() && named_usage.unresolved.is_empty(),
        "{named_usage:?}"
    );
}
#[test]
fn standard_add_zero_has_a_constructor_derived_proof() {
    actual_body_is_a_closed_proof("add_zero");
}
#[test]
fn standard_mul_zero_has_a_constructor_derived_proof() {
    actual_body_is_a_closed_proof("mul_zero");
}
#[test]
fn standard_mul_one_has_a_constructor_derived_proof() {
    actual_body_is_a_closed_proof("mul_one");
}
#[test]
fn standard_add_comm_has_a_constructor_derived_proof() {
    actual_body_is_a_closed_proof("add_comm");
}
#[test]
fn standard_mul_comm_has_a_constructor_derived_proof() {
    actual_body_is_a_closed_proof("mul_comm");
}

#[test]
fn a_downstream_integer_theorem_inherits_no_ring_assumption() {
    let mut ctx = context();
    let required = pi(
        "a",
        eq(op("add", v("a"), lit(0)), op("add", lit(0), v("a"))),
    );
    let body = lam("a", apps(g("add_comm"), &[v("a"), lit(0)]));
    assert!(defeq_for_test(
        &ctx,
        &recheck(&ctx, &body).unwrap(),
        &required
    ));
    ctx.declare("derived_swap_zero", required, body).unwrap();
    let usage = axiom_dependencies_of(&ctx, "derived_swap_zero").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}

#[test]
fn integer_constructor_induction_does_not_certify_a_false_law() {
    let mut ctx = context();
    let false_type = pi("a", eq(op("add", v("a"), lit(1)), v("a")));
    let reflexivity = lam("a", apps(g("refl"), &[g("Int"), v("a")]));
    let main = infer_type(&ctx, &reflexivity).unwrap();
    let independent = recheck(&ctx, &reflexivity).unwrap();
    assert!(!defeq_for_test(&ctx, &main, &false_type));
    assert!(!defeq_for_test(&ctx, &independent, &false_type));
    assert!(ctx
        .declare("false_successor_law", false_type, reflexivity)
        .is_err());
    assert!(ctx.get_global("false_successor_law").is_none());
}

#[test]
fn derived_ring_laws_accept_both_signs_and_literal_boundaries() {
    let ctx = context();
    for n in [i64::MIN, -1, 0, 1, i64::MAX] {
        for name in ["add_zero", "mul_zero", "mul_one"] {
            assert!(
                ctx.is_definition(name),
                "boundary instance must refer to a proved law"
            );
            let proof = app(g(name), lit(n));
            let goal = match name {
                "add_zero" => eq(op("add", lit(n), lit(0)), lit(n)),
                "mul_zero" => eq(op("mul", lit(n), lit(0)), lit(0)),
                _ => eq(op("mul", lit(n), lit(1)), lit(n)),
            };
            assert!(defeq_for_test(
                &ctx,
                &infer_type(&ctx, &proof).unwrap(),
                &goal
            ));
            assert!(defeq_for_test(&ctx, &recheck(&ctx, &proof).unwrap(), &goal));
        }
    }
}

#[test]
fn supporting_nat_and_congruence_lemmas_have_independently_checked_actual_bodies() {
    let ctx = context();
    for name in [
        "IntLaw_congrArg",
        "Nat_add_zero",
        "Nat_zero_add",
        "Nat_add_succ_right",
        "Nat_add_succ_left",
        "Nat_add_comm",
        "Nat_mul_zero_right",
        "Nat_add_assoc",
        "Nat_mul_zero_left",
        "Nat_mul_succ_left",
        "Nat_mul_comm",
        "Nat_mul_one",
    ] {
        let body = ctx
            .get_definition_body(name)
            .expect("a supporting lemma must be an actual checked definition");
        let required = ctx.get_definition_type(name).unwrap();
        assert_ne!(body, &g(name));
        assert!(
            defeq_for_test(&ctx, &infer_type(&ctx, body).unwrap(), required),
            "{name}"
        );
        assert!(
            defeq_for_test(&ctx, &recheck(&ctx, body).unwrap(), required),
            "{name}"
        );
        let usage = axiom_dependencies(&ctx, body);
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}

#[test]
fn canonical_nat_and_int_induction_principles_have_the_expected_checked_signatures() {
    fn forall(name: &str, domain: Term, codomain: Term) -> Term {
        Term::Pi {
            param: name.into(),
            param_type: Box::new(domain),
            body_type: Box::new(codomain),
        }
    }
    let ctx = context();
    let p = |x: Term| app(v("P"), x);
    let type0 = Term::Sort(logicaffeine_kernel::Universe::Type(0));
    let nat_signature = forall(
        "P",
        forall("x", g("Nat"), type0.clone()),
        forall(
            "zero",
            p(g("Zero")),
            forall(
                "step",
                forall(
                    "n",
                    g("Nat"),
                    forall("ih", p(v("n")), p(app(g("Succ"), v("n")))),
                ),
                forall("n", g("Nat"), p(v("n"))),
            ),
        ),
    );
    let int_signature = forall(
        "P",
        forall("x", g("Int"), type0),
        forall(
            "ofNat",
            forall("n", g("Nat"), p(app(g("Int_ofNat"), v("n")))),
            forall(
                "negSucc",
                forall("n", g("Nat"), p(app(g("Int_negSucc"), v("n")))),
                forall("i", g("Int"), p(v("i"))),
            ),
        ),
    );
    for (name, required) in [("Nat_rec", nat_signature), ("Int_rec", int_signature)] {
        let body = ctx
            .get_definition_body(name)
            .expect("canonical induction must have a checked body");
        assert_ne!(body, &g(name));
        assert!(
            defeq_for_test(&ctx, ctx.get_global(name).unwrap(), &required),
            "{name}"
        );
        assert!(
            defeq_for_test(&ctx, &infer_type(&ctx, body).unwrap(), &required),
            "{name}"
        );
        assert!(
            defeq_for_test(&ctx, &recheck(&ctx, body).unwrap(), &required),
            "{name}"
        );
        let usage = axiom_dependencies(&ctx, body);
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}
