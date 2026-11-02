use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{double_check, infer_type, Context, DoubleCheck, Term};
use logicaffeine_proof::tactic_cert::{
    syn_apply, syn_binary, syn_typed_eq, term_of_syn, CertifiablePrimitive, Syn,
};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn name(n: &str) -> Syn {
    Syn::Name(n.into())
}
fn equality_witness(carrier: &str, left: Term, right: Term) -> Term {
    let ty = app(app(app(g("Eq"), g(carrier)), left), right.clone());
    app(
        Term::Lambda {
            param: "proof".into(),
            param_type: Box::new(ty),
            body: Box::new(Term::Var("proof".into())),
        },
        app(app(g("refl"), g(carrier)), right),
    )
}
fn check_primitive(p: CertifiablePrimitive, goal: Syn) {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let goal = term_of_syn(&goal);
    let computation = app(g(p.rule_name()), goal.clone());
    let derivation = app(g(p.success_constructor()), goal.clone());
    for proof in [
        equality_witness("Derivation", computation.clone(), derivation.clone()),
        equality_witness("Syntax", app(g("concludes"), derivation), goal.clone()),
        equality_witness("Syntax", app(g("concludes"), computation), goal),
    ] {
        assert!(
            infer_type(&ctx, &proof).is_ok(),
            "main: {:?}",
            infer_type(&ctx, &proof)
        );
        assert_eq!(double_check(&ctx, &proof), DoubleCheck::Agreed);
    }
}
#[test]
fn ring_computation_and_conclusion_are_independently_checked() {
    check_primitive(
        CertifiablePrimitive::Ring,
        syn_typed_eq(
            "Int",
            syn_binary("add", name("x"), name("x")),
            syn_binary("mul", Syn::Lit(2), name("x")),
        ),
    );
}
#[test]
fn linear_computation_and_conclusion_are_independently_checked() {
    check_primitive(
        CertifiablePrimitive::Lia,
        syn_binary("le", name("x"), syn_binary("add", name("x"), Syn::Lit(1))),
    );
}
#[test]
fn congruence_computation_and_conclusion_are_independently_checked() {
    check_primitive(
        CertifiablePrimitive::Cc,
        syn_binary(
            "implies",
            syn_binary("Eq", name("x"), name("y")),
            syn_binary(
                "Eq",
                syn_apply(name("f"), name("x")),
                syn_apply(name("f"), name("y")),
            ),
        ),
    );
}
#[test]
fn forged_primitive_derivations_cannot_acquire_a_conclusion() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for (p, goal) in [
        (
            CertifiablePrimitive::Ring,
            syn_typed_eq("Int", Syn::Lit(0), Syn::Lit(1)),
        ),
        (
            CertifiablePrimitive::Lia,
            syn_binary("le", Syn::Lit(2), Syn::Lit(1)),
        ),
        (
            CertifiablePrimitive::Cc,
            syn_binary("Eq", name("x"), name("y")),
        ),
    ] {
        let goal = term_of_syn(&goal);
        let forged = app(g(p.success_constructor()), goal.clone());
        let proof = equality_witness("Syntax", app(g("concludes"), forged), goal);
        assert!(infer_type(&ctx, &proof).is_err());
        assert!(logicaffeine_kernel::recheck::recheck(&ctx, &proof).is_err());
    }
}

#[test]
fn redefining_a_primitive_keeps_the_definition_authoritative() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.declare(
        "try_ring",
        Term::Pi {
            param: "goal".into(),
            param_type: Box::new(g("Syntax")),
            body_type: Box::new(g("Derivation")),
        },
        Term::Lambda {
            param: "goal".into(),
            param_type: Box::new(g("Syntax")),
            body: Box::new(app(g("DAxiom"), Term::Var("goal".into()))),
        },
    )
    .unwrap();
    let goal = term_of_syn(&syn_typed_eq("Int", name("x"), name("x")));
    let proof = equality_witness(
        "Derivation",
        app(g("try_ring"), goal.clone()),
        app(g("DAxiom"), goal),
    );
    assert!(infer_type(&ctx, &proof).is_ok());
    assert_eq!(double_check(&ctx, &proof), DoubleCheck::Agreed);
}

#[test]
fn unimplemented_derivation_checks_remain_explicitly_incomplete() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let goal = term_of_syn(&name("aHypothesis"));
    let proof = equality_witness(
        "Syntax",
        app(g("concludes"), app(g("DAxiom"), goal.clone())),
        goal,
    );
    assert!(infer_type(&ctx, &proof).is_ok());
    assert!(matches!(
        double_check(&ctx, &proof),
        DoubleCheck::MainOnlyReCheckerIncomplete(_)
    ));
}

#[test]
fn independent_conclusions_do_not_turn_near_misses_into_proofs() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for n in -4..=4 {
        for delta in [-1, 0, 1] {
            let lhs = syn_binary("add", name("x"), Syn::Lit(n));
            let rhs = syn_binary("add", name("x"), Syn::Lit(n + delta));
            for (p, goal, valid) in [
                (
                    CertifiablePrimitive::Ring,
                    syn_typed_eq("Int", lhs.clone(), rhs.clone()),
                    delta == 0,
                ),
                (
                    CertifiablePrimitive::Lia,
                    syn_binary("le", lhs.clone(), rhs.clone()),
                    delta >= 0,
                ),
            ] {
                let goal = term_of_syn(&goal);
                let proof = equality_witness(
                    "Syntax",
                    app(
                        g("concludes"),
                        app(g(p.success_constructor()), goal.clone()),
                    ),
                    goal,
                );
                assert_eq!(infer_type(&ctx, &proof).is_ok(), valid);
                let second = logicaffeine_kernel::recheck::recheck(&ctx, &proof);
                assert_eq!(
                    second.is_ok(),
                    valid,
                    "{p:?}, n={n}, delta={delta}: {second:?}"
                );
            }
        }
    }
}
