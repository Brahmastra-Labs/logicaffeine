//! Regression pins for Bug Report #1 — parser semantics (BUG-016, ...).
//!
//! Parses via the public `Lexer`/`Parser` API (same setup as the e2e harness's
//! `parse_to_view`) and inspects the resolved `ExprView`, so these run under
//! `-p logicaffeine-language` without the e2e/forge toolchain.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{LogicExpr, NounPhrase, Term, ThematicRole};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::token::TokenType;
use logicaffeine_language::view::{ExprView, Resolve};
use logicaffeine_language::{Lexer, Parser};

fn parse_to_view(input: &str) -> ExprView<'static> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));

    let ctx = AstContext::new(expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena);

    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();
    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    let ast = parser.parse().unwrap();
    ast.resolve(interner)
}

/// BUG-016: "P if and only if Q or R" is standardly P ↔ (Q ∨ R) — the
/// biconditional binds LOOSER than disjunction. The top connective must be Iff
/// with the whole disjunction as its right operand, not (P ↔ Q) ∨ R.
#[test]
fn biconditional_outscopes_following_disjunction() {
    let view = parse_to_view("John stays if and only if Mary leaves or Bob leaves.");
    match view {
        ExprView::BinaryOp { op: TokenType::Iff, left, right } => {
            assert!(
                matches!(*right, ExprView::BinaryOp { op: TokenType::Or, .. }),
                "Expected (Mary leaves OR Bob leaves) as the right operand of Iff, got {:?}",
                right
            );
            assert!(
                !matches!(*left, ExprView::BinaryOp { op: TokenType::Iff, .. }),
                "Left operand of the top Iff must be the simple clause, got {:?}",
                left
            );
        }
        _ => panic!(
            "Top connective must be Iff for 'P if and only if Q or R' (P <-> (Q v R)), got {:?}",
            view
        ),
    }
}

/// BUG-017: "No N will/did V" must keep its negation — ∀x(N(x) → ¬V(x)). The
/// auxiliary (will=Future, did=Past) branch of `parse_quantified_core` dropped
/// the `No` case, emitting the negation-free ∀x(N(x) ∧ V(x)).
#[test]
fn no_quantifier_with_future_auxiliary_keeps_negation() {
    let out = logicaffeine_language::compile("No bird will fly.").expect("should compile");
    assert!(
        out.contains('¬') || out.contains("Not"),
        "\"No bird will fly\" lost its negation (auxiliary branch dropped \"No\"): {out}"
    );
}

#[test]
fn no_quantifier_with_did_auxiliary_keeps_negation() {
    let out = logicaffeine_language::compile("No dog did bark.").expect("should compile");
    assert!(
        out.contains('¬') || out.contains("Not"),
        "\"No dog did bark\" lost its negation (auxiliary branch dropped \"No\"): {out}"
    );
}

/// Structural: the "No" quantifier must be Universal with a negation in its body.
#[test]
fn no_quantifier_aux_is_negated_universal() {
    use logicaffeine_language::ast::QuantifierKind;
    fn contains_not(v: &ExprView) -> bool {
        match v {
            ExprView::UnaryOp { .. } => true,
            ExprView::BinaryOp { left, right, .. } => contains_not(left) || contains_not(right),
            ExprView::Quantifier { body, .. } => contains_not(body),
            _ => false,
        }
    }
    match parse_to_view("No bird will fly.") {
        ExprView::Quantifier { kind, body, .. } => {
            assert_eq!(kind, QuantifierKind::Universal, "\"No\" should be Universal, got {:?}", kind);
            assert!(contains_not(&body), "\"No bird will fly\" body must contain a negation, got {:?}", body);
        }
        other => panic!("Expected Quantifier for \"No bird will fly\", got {:?}", other),
    }
}

/// BUG-018: a donkey indefinite in a subject relative clause must stay bound
/// even when the main verb has a QUANTIFIED object. That path returned without
/// running the donkey-binding closure, leaving the donkey variable `y` free.
#[test]
fn donkey_in_relative_clause_bound_even_with_quantified_object() {
    let baseline = logicaffeine_language::compile("Every farmer who owns a donkey beats it.")
        .expect("baseline compiles");
    assert!(
        baseline.contains("∀y(") || baseline.contains("∃y("),
        "baseline donkey var must be bound, got: {baseline}"
    );

    let out = logicaffeine_language::compile("Every farmer who owns a donkey feeds every animal.")
        .expect("should compile");
    assert!(
        out.contains("Donkey(y)"),
        "expected the donkey predicate Donkey(y) in the output, got: {out}"
    );
    // The bug left `y` free — no binder over the donkey variable. (Do NOT check
    // for a bare ∃: every neo-Davidsonian event emits ∃e, which is always present.)
    assert!(
        out.contains("∀y(") || out.contains("∃y("),
        "donkey variable y is FREE (unbound) with a quantified object: no ∀y(/∃y( binder. got: {out}"
    );
}

/// BUG-019: an embedded copula+participle passive must bind the `by`-phrase NP
/// as the AGENT (first slot) — `See(Mary, John)` for "...seen by Mary" — not
/// demote it into a locative `by(...)` adjunct leaving the theme in the agent slot.
#[test]
fn embedded_passive_binds_by_phrase_agent() {
    let fol = logicaffeine_language::compile("It was John who was seen by Mary.").unwrap();
    assert!(
        fol.contains("See(Mary"),
        "by-phrase agent must fill the predicate agent slot, got: {fol}"
    );
    assert!(
        !fol.contains("See(John)") && !fol.contains("See(John,"),
        "theme John must not occupy the agent slot of the passive predicate, got: {fol}"
    );
}
