use super::*;

#[test]
fn test_verify_expr_construction() {
    // Test that we can construct expressions
    let x = VerifyExpr::var("x");
    let five = VerifyExpr::int(5);
    let ten = VerifyExpr::int(10);

    // x > 5
    let gt = VerifyExpr::gt(x.clone(), five);
    assert!(matches!(
        gt,
        VerifyExpr::Binary {
            op: VerifyOp::Gt,
            ..
        }
    ));

    // x == 10
    let eq = VerifyExpr::eq(x.clone(), ten);
    assert!(matches!(
        eq,
        VerifyExpr::Binary {
            op: VerifyOp::Eq,
            ..
        }
    ));
}

#[test]
fn test_uninterpreted_function() {
    // Mortal(x)
    let mortal_x = VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]);
    assert!(
        matches!(mortal_x, VerifyExpr::Apply { name, args } if name == "Mortal" && args.len() == 1)
    );
}

#[test]
fn test_implication() {
    // Mortal(x) -> Human(x)
    let mortal = VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]);
    let human = VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]);
    let impl_expr = VerifyExpr::implies(mortal, human);

    assert!(matches!(
        impl_expr,
        VerifyExpr::Binary {
            op: VerifyOp::Implies,
            ..
        }
    ));
}

#[test]
fn test_quantifier() {
    // forall x: Object. Mortal(x) -> Human(x)
    let body = VerifyExpr::implies(
        VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]),
        VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]),
    );
    let forall = VerifyExpr::forall(vec![("x".to_string(), VerifyType::Object)], body);

    assert!(matches!(forall, VerifyExpr::ForAll { vars, .. } if vars.len() == 1));
}
