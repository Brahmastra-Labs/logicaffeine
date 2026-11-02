use super::*;
use crate::ast::{AspectOperator, ModalDomain, ModalVector, QuantifierKind, TemporalOperator};
use crate::token::TokenType;
use logicaffeine_base::Interner;

fn setup<'a>(
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    np_arena: &'a Arena<NounPhrase<'a>>,
    sym_arena: &'a Arena<Symbol>,
    role_arena: &'a Arena<(ThematicRole, Term<'a>)>,
    pp_arena: &'a Arena<&'a LogicExpr<'a>>,
) -> AstContext<'a> {
    AstContext::new(
        expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena,
    )
}

#[test]
fn binary_builder_creates_binary_op() {
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();
    let ctx = setup(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut interner = Interner::new();
    let p = interner.intern("P");
    let q = interner.intern("Q");

    let left = ctx.alloc_expr(LogicExpr::Atom(p));
    let right = ctx.alloc_expr(LogicExpr::Atom(q));
    let result = ctx.binary(left, TokenType::And, right);

    assert!(matches!(
        result,
        LogicExpr::BinaryOp {
            op: TokenType::And,
            ..
        }
    ));
}

#[test]
fn unary_builder_creates_unary_op() {
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();
    let ctx = setup(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut interner = Interner::new();
    let p = interner.intern("P");
    let operand = ctx.alloc_expr(LogicExpr::Atom(p));
    let result = ctx.unary(TokenType::Not, operand);

    assert!(matches!(
        result,
        LogicExpr::UnaryOp {
            op: TokenType::Not,
            ..
        }
    ));
}

#[test]
fn quantifier_builder_creates_quantifier() {
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();
    let ctx = setup(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut interner = Interner::new();
    let x = interner.intern("x");
    let p = interner.intern("P");
    let body = ctx.alloc_expr(LogicExpr::Atom(p));
    let result = ctx.quantifier(QuantifierKind::Universal, x, body, 0);

    assert!(matches!(
        result,
        LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            ..
        }
    ));
}

#[test]
fn temporal_builder_creates_temporal() {
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();
    let ctx = setup(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut interner = Interner::new();
    let p = interner.intern("P");
    let body = ctx.alloc_expr(LogicExpr::Atom(p));
    let result = ctx.temporal(TemporalOperator::Past, body);

    assert!(matches!(
        result,
        LogicExpr::Temporal {
            operator: TemporalOperator::Past,
            ..
        }
    ));
}

#[test]
fn aspectual_builder_creates_aspectual() {
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();
    let ctx = setup(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut interner = Interner::new();
    let p = interner.intern("P");
    let body = ctx.alloc_expr(LogicExpr::Atom(p));
    let result = ctx.aspectual(AspectOperator::Progressive, body);

    assert!(matches!(
        result,
        LogicExpr::Aspectual {
            operator: AspectOperator::Progressive,
            ..
        }
    ));
}

#[test]
fn modal_builder_creates_modal() {
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();
    let ctx = setup(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut interner = Interner::new();
    let p = interner.intern("P");
    let operand = ctx.alloc_expr(LogicExpr::Atom(p));
    let vector = ModalVector {
        domain: ModalDomain::Alethic,
        force: 1.0,
        flavor: crate::ast::ModalFlavor::Root,
        modal_base: None,
        ordering_source: None,
    };
    let result = ctx.modal(vector, operand);

    assert!(matches!(result, LogicExpr::Modal { .. }));
}
