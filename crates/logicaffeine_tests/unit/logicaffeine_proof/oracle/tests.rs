use super::*;

#[test]
fn test_convert_atom() {
    let expr = ProofExpr::Atom("P".into());
    let result = proof_expr_to_verify_expr(&expr);
    assert!(matches!(result, Some(VerifyExpr::Var(s)) if s == "P"));
}

#[test]
fn test_convert_gt_predicate() {
    let expr = ProofExpr::Predicate {
        name: "Gt".into(),
        args: vec![
            ProofTerm::Variable("x".into()),
            ProofTerm::Constant("10".into()),
        ],
        world: None,
    };
    let result = proof_expr_to_verify_expr(&expr);
    assert!(matches!(
        result,
        Some(VerifyExpr::Binary {
            op: VerifyOp::Gt,
            ..
        })
    ));
}

#[test]
fn test_convert_implication() {
    let expr = ProofExpr::Implies(
        Box::new(ProofExpr::Atom("P".into())),
        Box::new(ProofExpr::Atom("Q".into())),
    );
    let result = proof_expr_to_verify_expr(&expr);
    assert!(matches!(
        result,
        Some(VerifyExpr::Binary {
            op: VerifyOp::Implies,
            ..
        })
    ));
}

#[test]
fn test_convert_arithmetic_function() {
    let term = ProofTerm::Function(
        "Add".into(),
        vec![
            ProofTerm::Variable("x".into()),
            ProofTerm::Constant("5".into()),
        ],
    );
    let result = proof_term_to_verify_expr(&term);
    assert!(matches!(
        result,
        Some(VerifyExpr::Binary {
            op: VerifyOp::Add,
            ..
        })
    ));
}
