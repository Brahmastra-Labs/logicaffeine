use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, Context, Term, Universe};

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn sort(level: Universe) -> Term {
    Term::Sort(level)
}
fn cref(name: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: name.into(),
        levels,
    }
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn identity() -> (Term, Term) {
    let carrier = sort(Universe::Var("u".into()));
    let ty = Term::Pi {
        param: "A".into(),
        param_type: Box::new(carrier.clone()),
        body_type: Box::new(Term::Pi {
            param: "x".into(),
            param_type: Box::new(Term::Var("A".into())),
            body_type: Box::new(Term::Var("A".into())),
        }),
    };
    let body = Term::Lambda {
        param: "A".into(),
        param_type: Box::new(carrier),
        body: Box::new(Term::Lambda {
            param: "x".into(),
            param_type: Box::new(Term::Var("A".into())),
            body: Box::new(Term::Var("x".into())),
        }),
    };
    (ty, body)
}

#[test]
fn polymorphic_registration_refuses_false_from_a_nat_body() {
    let mut ctx = context();
    ctx.add_universe_poly("bad", vec![], g("False"), g("Zero"));
    assert!(ctx.get_universe_poly("bad").is_none());
    assert!(infer_type(&ctx, &cref("bad", vec![])).is_err());
    assert!(ctx
        .refused_declarations()
        .iter()
        .any(|(name, _)| name == "bad"));
}

#[test]
fn a_checked_polymorphic_identity_computes_at_prop_and_type() {
    let mut ctx = context();
    let (ty, body) = identity();
    ctx.add_universe_poly("polyIdentity", vec!["u".into()], ty, body);
    for (level, carrier, value) in [
        (Universe::Prop, g("True"), g("I")),
        (Universe::Type(0), g("Nat"), g("Zero")),
    ] {
        let term = Term::App(
            Box::new(Term::App(
                Box::new(cref("polyIdentity", vec![level])),
                Box::new(carrier.clone()),
            )),
            Box::new(value.clone()),
        );
        assert_eq!(infer_type(&ctx, &term).unwrap(), carrier);
        assert_eq!(normalize(&ctx, &term), value);
    }
}

#[test]
fn replacing_either_definition_kind_leaves_only_one_meaning() {
    let mut ctx = context();
    ctx.add_definition("replace".into(), g("Nat"), g("Zero"));
    let one = Term::App(Box::new(g("Succ")), Box::new(g("Zero")));
    ctx.add_universe_poly("replace", vec![], g("Nat"), one.clone());
    assert!(ctx.get_definition_body("replace").is_none());
    assert_eq!(normalize(&ctx, &cref("replace", vec![])), one);
    ctx.add_definition("replace".into(), g("Nat"), g("Zero"));
    assert!(ctx.get_universe_poly("replace").is_none());
    assert_eq!(normalize(&ctx, &g("replace")), g("Zero"));
}

#[test]
fn a_polymorphic_edge_cannot_complete_a_delta_cycle() {
    let mut ctx = context();
    ctx.add_universe_poly("polyFirst", vec![], g("Nat"), cref("polySecond", vec![]));
    ctx.add_universe_poly("polySecond", vec![], g("Nat"), cref("polyFirst", vec![]));
    assert!(ctx.get_universe_poly("polySecond").is_none());
    assert!(ctx
        .refused_declarations()
        .iter()
        .any(|(name, _)| name == "polySecond"));
}

#[test]
fn a_monomorphic_edge_cannot_complete_a_mixed_delta_cycle() {
    let mut ctx = context();
    ctx.add_universe_poly("polyFirst", vec![], g("Nat"), g("monoSecond"));
    ctx.add_definition("monoSecond".into(), g("Nat"), cref("polyFirst", vec![]));
    assert!(ctx.get_definition_body("monoSecond").is_none());
}

#[test]
fn explicit_level_parameters_bind_the_type_and_body() {
    let mut ctx = context();
    let (ty, body) = identity();
    ctx.add_universe_poly("unboundLevel", vec![], ty.clone(), body.clone());
    assert!(ctx.get_universe_poly("unboundLevel").is_none());
    ctx.add_universe_poly("duplicateLevel", vec!["u".into(), "u".into()], ty, body);
    assert!(ctx.get_universe_poly("duplicateLevel").is_none());
}

#[test]
fn pending_bodies_are_not_available_as_proofs() {
    let mut ctx = context();
    ctx.add_definition("pendingFalse".into(), g("False"), g("laterValue"));
    assert!(ctx.is_definition("pendingFalse"));
    assert!(infer_type(&ctx, &g("pendingFalse")).is_err());
    assert!(ctx.get_definition_body("pendingFalse").is_none());
    ctx.add_definition("laterValue".into(), g("Nat"), g("Zero"));
    assert!(infer_type(&ctx, &g("pendingFalse")).is_err());
    assert!(!ctx.pending_obligations().is_empty());
}

#[test]
fn pending_polymorphic_bodies_are_not_available_as_proofs() {
    let mut ctx = context();
    ctx.add_universe_poly("pendingFalse", vec![], g("False"), g("laterValue"));
    assert!(infer_type(&ctx, &cref("pendingFalse", vec![])).is_err());
    ctx.add_definition("laterValue".into(), g("Nat"), g("Zero"));
    assert!(infer_type(&ctx, &cref("pendingFalse", vec![])).is_err());
}

#[test]
fn resolving_a_forward_chain_checks_every_definition_before_exposing_it() {
    let mut ctx = context();
    for i in 0..8 {
        ctx.add_definition(
            format!("forward{i}"),
            g("Nat"),
            g(&format!("forward{}", i + 1)),
        );
    }
    assert!(infer_type(&ctx, &g("forward0")).is_err());
    ctx.add_definition("forward8".into(), g("Nat"), g("Zero"));
    assert!(
        ctx.pending_obligations().is_empty(),
        "{:?}",
        ctx.pending_obligations()
    );
    assert_eq!(infer_type(&ctx, &g("forward0")).unwrap(), g("Nat"));
    assert_eq!(normalize(&ctx, &g("forward0")), g("Zero"));
}
