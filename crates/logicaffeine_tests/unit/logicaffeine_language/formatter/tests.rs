use super::*;

#[test]
fn unicode_binary_operators() {
    let f = UnicodeFormatter;
    assert_eq!(f.binary_op(&TokenType::And, "P", "Q"), "(P ∧ Q)");
    assert_eq!(f.binary_op(&TokenType::Or, "P", "Q"), "(P ∨ Q)");
    assert_eq!(f.binary_op(&TokenType::If, "P", "Q"), "(P → Q)");
    assert_eq!(f.binary_op(&TokenType::Iff, "P", "Q"), "(P ↔ Q)");
}

#[test]
fn latex_binary_operators() {
    let f = LatexFormatter;
    assert_eq!(f.binary_op(&TokenType::And, "P", "Q"), r"(P \cdot Q)");
    assert_eq!(f.binary_op(&TokenType::Or, "P", "Q"), r"(P \vee Q)");
    assert_eq!(f.binary_op(&TokenType::If, "P", "Q"), r"(P \supset Q)");
    assert_eq!(f.binary_op(&TokenType::Iff, "P", "Q"), r"(P \equiv Q)");
}

#[test]
fn unicode_quantifiers() {
    let f = UnicodeFormatter;
    assert_eq!(
        f.quantifier(&QuantifierKind::Universal, "x", "P(x)"),
        "∀x(P(x))"
    );
    assert_eq!(
        f.quantifier(&QuantifierKind::Existential, "x", "P(x)"),
        "∃x(P(x))"
    );
    assert_eq!(
        f.quantifier(&QuantifierKind::Cardinal(3), "x", "P(x)"),
        "∃=3.x(P(x))"
    );
}

#[test]
fn latex_quantifiers() {
    let f = LatexFormatter;
    assert_eq!(
        f.quantifier(&QuantifierKind::Universal, "x", "P(x)"),
        "\\forall x(P(x))"
    );
    assert_eq!(
        f.quantifier(&QuantifierKind::Existential, "x", "P(x)"),
        "\\exists x(P(x))"
    );
}

#[test]
fn latex_sanitization() {
    let f = LatexFormatter;
    assert_eq!(f.sanitize("foo_bar"), r"foo\_bar");
    assert_eq!(f.sanitize("x^2"), r"x\^{}2");
    assert_eq!(f.sanitize("a&b"), r"a\&b");
}

#[test]
fn unicode_no_sanitization() {
    let f = UnicodeFormatter;
    assert_eq!(f.sanitize("foo_bar"), "foo_bar");
}

#[test]
fn unicode_lambda() {
    let f = UnicodeFormatter;
    assert_eq!(f.lambda("x", "P(x)"), "λx.P(x)");
}

#[test]
fn latex_lambda() {
    let f = LatexFormatter;
    assert_eq!(f.lambda("x", "P(x)"), "\\lambda x.P(x)");
}

#[test]
fn unicode_counterfactual() {
    let f = UnicodeFormatter;
    assert_eq!(f.counterfactual("P", "Q"), "(P □→ Q)");
}

#[test]
fn latex_counterfactual() {
    let f = LatexFormatter;
    assert_eq!(f.counterfactual("P", "Q"), r"(P \boxright Q)");
}

// RustFormatter tests
#[test]
fn rust_binary_operators() {
    let f = RustFormatter;
    assert_eq!(f.binary_op(&TokenType::And, "P", "Q"), "(P && Q)");
    assert_eq!(f.binary_op(&TokenType::Or, "P", "Q"), "(P || Q)");
    assert_eq!(f.binary_op(&TokenType::Iff, "P", "Q"), "(P == Q)");
}

#[test]
fn rust_implication_desugaring() {
    let f = RustFormatter;
    // A → B desugars to !A || B
    assert_eq!(f.binary_op(&TokenType::If, "P", "Q"), "(!(P) || (Q))");
}

#[test]
fn rust_lambda() {
    let f = RustFormatter;
    assert_eq!(f.lambda("x", "x > 0"), "|x| { x > 0 }");
}

#[test]
fn rust_quantifiers_as_comments() {
    let f = RustFormatter;
    assert_eq!(f.universal(), "/* ∀ */");
    assert_eq!(f.existential(), "/* ∃ */");
}
