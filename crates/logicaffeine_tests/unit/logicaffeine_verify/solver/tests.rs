use super::*;

#[test]
fn test_tautology() {
    let verifier = Verifier::new();
    assert!(verifier.check_bool(true).is_ok());
}

#[test]
fn test_contradiction() {
    let verifier = Verifier::new();
    assert!(verifier.check_bool(false).is_err());
}

#[test]
fn test_int_greater_than_valid() {
    let verifier = Verifier::new();
    assert!(verifier.check_int_greater_than(10, 5).is_ok());
}

#[test]
fn test_int_greater_than_invalid() {
    let verifier = Verifier::new();
    assert!(verifier.check_int_greater_than(3, 5).is_err());
}

#[test]
fn test_int_equals_valid() {
    let verifier = Verifier::new();
    assert!(verifier.check_int_equals(42, 42).is_ok());
}

#[test]
fn test_int_equals_invalid() {
    let verifier = Verifier::new();
    assert!(verifier.check_int_equals(1, 2).is_err());
}

#[test]
fn test_context_api() {
    let verifier = Verifier::new();
    let vctx = verifier.context();
    let solver = vctx.solver();

    // P ∨ ¬P is a tautology
    let p = vctx.bool_var("p");
    let tautology = Bool::or(&[&p, &p.not()]);

    assert!(vctx.check_valid(&solver, &tautology).is_ok());
}

#[test]
fn test_context_contradiction() {
    let verifier = Verifier::new();
    let vctx = verifier.context();
    let solver = vctx.solver();

    // P ∧ ¬P is a contradiction (not valid)
    let p = vctx.bool_var("p");
    let contradiction = Bool::and(&[&p, &p.not()]);

    assert!(vctx.check_valid(&solver, &contradiction).is_err());
}

// ============================================================
// VerificationSession Tests
// ============================================================

#[test]
fn test_session_integer_bounds() {
    let mut session = VerificationSession::new();

    // Declare x as Int
    session.declare("x", VerifyType::Int);

    // Assume: x = 10
    session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10)));

    // Verify: x > 5 (should pass)
    let result = session.verify(&VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5)));
    assert!(result.is_ok(), "10 > 5 should be provable");
}

#[test]
fn test_session_integer_contradiction() {
    let mut session = VerificationSession::new();

    // Declare x as Int
    session.declare("x", VerifyType::Int);

    // Assume: x = 10
    session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10)));

    // Verify: x < 5 (should FAIL)
    let result = session.verify(&VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(5)));
    assert!(result.is_err(), "10 < 5 should not be provable");
}

#[test]
fn test_session_uninterpreted_functions() {
    let mut session = VerificationSession::new();

    // Declare x as Object
    session.declare("x", VerifyType::Object);

    // Assume: Mortal(x) -> Human(x)
    session.assume(&VerifyExpr::implies(
        VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]),
        VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]),
    ));

    // Assume: Mortal(x)
    session.assume(&VerifyExpr::apply("Mortal", vec![VerifyExpr::var("x")]));

    // Verify: Human(x) - Z3 should deduce this structurally
    let result = session.verify(&VerifyExpr::apply("Human", vec![VerifyExpr::var("x")]));
    assert!(
        result.is_ok(),
        "Should deduce Human(x) from Mortal(x) and Mortal(x)->Human(x)"
    );
}

#[test]
fn test_session_modal_structural_reasoning() {
    let mut session = VerificationSession::new();

    // Declare A and B as Objects (representing propositions)
    session.declare("A", VerifyType::Object);
    session.declare("B", VerifyType::Object);

    // Assume: Possible(A) -> Possible(B)
    session.assume(&VerifyExpr::implies(
        VerifyExpr::apply("Possible", vec![VerifyExpr::var("A")]),
        VerifyExpr::apply("Possible", vec![VerifyExpr::var("B")]),
    ));

    // Assume: Possible(A)
    session.assume(&VerifyExpr::apply("Possible", vec![VerifyExpr::var("A")]));

    // Verify: Possible(B)
    let result = session.verify(&VerifyExpr::apply("Possible", vec![VerifyExpr::var("B")]));
    assert!(
        result.is_ok(),
        "Should deduce Possible(B) from modus ponens"
    );
}

#[test]
fn test_session_arithmetic() {
    let mut session = VerificationSession::new();

    // Declare x and y
    session.declare("x", VerifyType::Int);
    session.declare("y", VerifyType::Int);

    // Assume: x = 5, y = 3
    session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(5)));
    session.assume(&VerifyExpr::eq(VerifyExpr::var("y"), VerifyExpr::int(3)));

    // Verify: x + y > 7 (5 + 3 = 8 > 7)
    let sum = VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::var("y"));
    let result = session.verify(&VerifyExpr::gt(sum, VerifyExpr::int(7)));
    assert!(result.is_ok(), "5 + 3 > 7 should be provable");
}

#[test]
fn test_session_logic_and_or() {
    let mut session = VerificationSession::new();

    // Declare p and q as Bool
    session.declare("p", VerifyType::Bool);
    session.declare("q", VerifyType::Bool);

    // Assume: p = true, q = false
    session.assume(&VerifyExpr::eq(
        VerifyExpr::var("p"),
        VerifyExpr::bool(true),
    ));
    session.assume(&VerifyExpr::eq(
        VerifyExpr::var("q"),
        VerifyExpr::bool(false),
    ));

    // Verify: p || q (true || false = true)
    let result = session.verify(&VerifyExpr::or(VerifyExpr::var("p"), VerifyExpr::var("q")));
    assert!(result.is_ok(), "true || false should be provable");

    // Verify: !(p && q) (!(true && false) = true)
    let result = session.verify(&VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::var("p"),
        VerifyExpr::var("q"),
    )));
    assert!(result.is_ok(), "!(true && false) should be provable");
}
