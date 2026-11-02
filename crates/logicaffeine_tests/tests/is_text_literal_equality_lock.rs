//! LOCK — `is <text literal>` and `is not <text literal>` are EQUALITY, exactly like `is 5`, in
//! plain statements and inside an LGX template hole. Before this pin, `is` disambiguated a bare
//! value only for a number or an already-bound name; a text literal fell into none of those cases
//! and the word `is` was left unconsumed, so `If s is "a":` and `{if name is "":}` were refused as
//! though the sentence never named a comparison at all.

#[path = "language_common/mod.rs"]
mod common;

use common::accepted;
use logicaffeine_language::ast::{BinaryOpKind, Expr, Literal, Stmt};

/// The lone top-level `If`'s condition, unwrapped to its `Expr::BinaryOp`.
fn if_cond<'a>(stmts: &'a [Stmt<'static>]) -> (BinaryOpKind, &'a Expr<'static>, &'a Expr<'static>) {
    let cond = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::If { cond, .. } => Some(*cond),
            _ => None,
        })
        .expect("the program must contain an `If`");
    match cond {
        Expr::BinaryOp { op, left, right } => (*op, left, right),
        other => panic!("`If`'s condition must be a binary comparison, was {other:?}"),
    }
}

#[test]
fn a_bare_is_against_a_text_literal_is_equality_in_a_statement() {
    let parsed = accepted("## Main\n\nLet s be \"a\".\nIf s is \"a\":\n    Let x be 1.\n");
    let (op, left, right) = if_cond(&parsed.stmts);
    assert_eq!(op, BinaryOpKind::Eq);
    assert!(matches!(left, Expr::Identifier(_)));
    assert!(
        matches!(right, Expr::Literal(Literal::Text(_))),
        "right operand must be the text literal, was {right:?}"
    );
}

#[test]
fn is_not_against_a_text_literal_is_inequality_in_a_statement() {
    let parsed = accepted("## Main\n\nLet s be \"a\".\nIf s is not \"b\":\n    Let x be 1.\n");
    let (op, left, right) = if_cond(&parsed.stmts);
    assert_eq!(op, BinaryOpKind::NotEq);
    assert!(matches!(left, Expr::Identifier(_)));
    assert!(
        matches!(right, Expr::Literal(Literal::Text(_))),
        "right operand must be the text literal, was {right:?}"
    );
}

#[test]
fn an_empty_text_literal_is_a_legal_right_operand() {
    accepted("## Main\n\nLet s be \"\".\nIf s is \"\":\n    Let x be 1.\n");
}

#[test]
fn is_against_a_text_literal_still_parses_one_expression_when_negated_first() {
    // `is not` already read any right operand generically; the positive form is the one this pin
    // adds, and it must not disturb `is not`'s existing, wider acceptance.
    accepted("## Main\n\nLet s be \"a\".\nIf s is not \"a\":\n    Let x be 1.\n");
}

fn page(view: &str) -> String {
    format!(
        "## Component App\nState:\n    Let name be \"\".\nView:\n    {view}\n\n## Main\nMount App.\n"
    )
}

#[test]
fn a_bare_is_against_a_text_literal_is_equality_in_a_template_hole() {
    accepted(&page("{if name is \"\":}<p>empty</p>{end}"));
}

#[test]
fn is_not_against_a_text_literal_in_a_template_hole() {
    accepted(&page("{if name is not \"\":}<p>has a name</p>{end}"));
}

#[test]
fn the_worded_comparisons_are_unaffected() {
    // The worded `is …` comparisons (`is between`, `is even`, `is greater than`, …) keep parsing
    // exactly as before — nothing about disambiguating a text-literal right operand touches them.
    let parsed = accepted("## Main\n\nLet n be 3.\nIf n is between 1 and 5:\n    Let x be 1.\n");
    let (op, ..) = if_cond(&parsed.stmts);
    assert_eq!(op, BinaryOpKind::And);
}
