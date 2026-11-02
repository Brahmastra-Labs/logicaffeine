use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{axiom_dependencies_of, infer_type, Context, Term};
use logicaffeine_proof::verify::check_derivation;
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};
fn le(a: &str, b: &str) -> ProofExpr {
    ProofExpr::Identity(
        ProofTerm::Function(
            "le".into(),
            vec![ProofTerm::Constant(a.into()), ProofTerm::Constant(b.into())],
        ),
        ProofTerm::Constant("true".into()),
    )
}
fn cites(t: &Term, name: &str) -> bool {
    match t {
        Term::Global(n) => n == name,
        Term::App(f, a) => cites(f, name) || cites(a, name),
        Term::Lambda {
            param_type, body, ..
        } => cites(param_type, name) || cites(body, name),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => cites(param_type, name) || cites(body_type, name),
        _ => false,
    }
}
#[test]
fn certified_transitivity_cites_a_real_definition_with_an_empty_trusted_footprint() {
    let premises = vec![le("2", "5"), le("5", "9")];
    let goal = le("2", "9");
    let tree = DerivationTree::new(
        goal.clone(),
        InferenceRule::LeTrans,
        vec![
            DerivationTree::leaf(le("2", "5"), InferenceRule::Axiom),
            DerivationTree::leaf(le("5", "9"), InferenceRule::Axiom),
        ],
    );
    let r = check_derivation(&premises, &goal, tree);
    assert!(r.verified, "{:?}", r.verification_error);
    let term = r.proof_term.as_ref().expect("verified proof term");
    assert!(
        cites(term, "le_trans"),
        "the certificate must actually cite the derived theorem"
    );
    assert!(r.kernel_ctx.get_definition_body("le_trans").is_some());
    let usage = axiom_dependencies_of(&r.kernel_ctx, "le_trans").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
    assert!(r.axiom_footprint().trusted.is_empty());
}
#[test]
fn a_checked_wrapper_still_reports_an_actual_transitive_assumption() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let ty = ctx.get_global("propext").unwrap().clone();
    ctx.declare(
        "wrapped_propext",
        ty.clone(),
        Term::Global("propext".into()),
    )
    .unwrap();
    assert_eq!(
        infer_type(&ctx, &Term::Global("wrapped_propext".into())).unwrap(),
        ty
    );
    let usage = axiom_dependencies_of(&ctx, "wrapped_propext").unwrap();
    assert_eq!(
        usage.axioms.into_iter().collect::<Vec<_>>(),
        vec!["propext".to_string()]
    );
    assert!(usage.unresolved.is_empty());
}
