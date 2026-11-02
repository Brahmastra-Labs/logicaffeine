use super::*;
use crate::ast::ModalDomain;
use logicaffeine_base::Arena;

#[test]
fn resolve_expr_predicate() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let mortal = interner.intern("Mortal");
    let x = interner.intern("x");
    let expr = LogicExpr::Predicate {
        name: mortal,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::Predicate {
            name: "Mortal",
            args: vec![TermView::Variable("x")],
        }
    );
}

#[test]
fn resolve_expr_identity() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let clark = interner.intern("Clark");
    let superman = interner.intern("Superman");
    let expr = LogicExpr::Identity {
        left: term_arena.alloc(Term::Constant(clark)),
        right: term_arena.alloc(Term::Constant(superman)),
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::Identity {
            left: TermView::Constant("Clark"),
            right: TermView::Constant("Superman"),
        }
    );
}

#[test]
fn resolve_expr_quantifier() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let x = interner.intern("x");
    let mortal = interner.intern("Mortal");

    let body = expr_arena.alloc(LogicExpr::Predicate {
        name: mortal,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let expr = LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: x,
        body,
        island_id: 0,
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::Quantifier {
            kind: QuantifierKind::Universal,
            variable: "x",
            body: Box::new(ExprView::Predicate {
                name: "Mortal",
                args: vec![TermView::Variable("x")],
            }),
        }
    );
}

#[test]
fn resolve_expr_atom() {
    let mut interner = Interner::new();
    let p = interner.intern("P");
    let expr = LogicExpr::Atom(p);

    assert_eq!(expr.resolve(&interner), ExprView::Atom("P"));
}

#[test]
fn resolve_expr_binary_op() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let p = interner.intern("P");
    let q = interner.intern("Q");
    let expr = LogicExpr::BinaryOp {
        left: expr_arena.alloc(LogicExpr::Atom(p)),
        op: TokenType::And,
        right: expr_arena.alloc(LogicExpr::Atom(q)),
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::BinaryOp {
            left: Box::new(ExprView::Atom("P")),
            op: TokenType::And,
            right: Box::new(ExprView::Atom("Q")),
        }
    );
}

#[test]
fn resolve_expr_lambda() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let x = interner.intern("x");
    let p = interner.intern("P");
    let expr = LogicExpr::Lambda {
        variable: x,
        body: expr_arena.alloc(LogicExpr::Atom(p)),
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::Lambda {
            variable: "x",
            body: Box::new(ExprView::Atom("P")),
        }
    );
}

#[test]
fn resolve_expr_temporal() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let run = interner.intern("Run");
    let expr = LogicExpr::Temporal {
        operator: TemporalOperator::Past,
        body: expr_arena.alloc(LogicExpr::Atom(run)),
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::Temporal {
            operator: TemporalOperator::Past,
            body: Box::new(ExprView::Atom("Run")),
        }
    );
}

#[test]
fn resolve_expr_modal() {
    use crate::ast::ModalFlavor;
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let rain = interner.intern("Rain");
    let expr = LogicExpr::Modal {
        vector: ModalVector {
            domain: ModalDomain::Alethic,
            force: 1.0,
            flavor: ModalFlavor::Root,
            modal_base: None,
            ordering_source: None,
        },
        operand: expr_arena.alloc(LogicExpr::Atom(rain)),
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::Modal {
            vector: ModalVector {
                domain: ModalDomain::Alethic,
                force: 1.0,
                flavor: ModalFlavor::Root,
                modal_base: None,
                ordering_source: None
            },
            operand: Box::new(ExprView::Atom("Rain")),
        }
    );
}

#[test]
fn modal_vector_equality_is_bit_exact() {
    use crate::ast::ModalFlavor;
    let v1 = ModalVector {
        domain: ModalDomain::Alethic,
        force: 0.5,
        flavor: ModalFlavor::Root,
        modal_base: None,
        ordering_source: None,
    };
    let v2 = ModalVector {
        domain: ModalDomain::Alethic,
        force: 0.5,
        flavor: ModalFlavor::Root,
        modal_base: None,
        ordering_source: None,
    };
    let v3 = ModalVector {
        domain: ModalDomain::Alethic,
        force: 0.51,
        flavor: ModalFlavor::Root,
        modal_base: None,
        ordering_source: None,
    };

    assert_eq!(v1, v2);
    assert_ne!(v1, v3);
}

#[test]
fn resolve_expr_unary_op() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let p = interner.intern("P");
    let expr = LogicExpr::UnaryOp {
        op: TokenType::Not,
        operand: expr_arena.alloc(LogicExpr::Atom(p)),
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::UnaryOp {
            op: TokenType::Not,
            operand: Box::new(ExprView::Atom("P")),
        }
    );
}

#[test]
fn resolve_expr_app() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let f = interner.intern("f");
    let x = interner.intern("x");
    let expr = LogicExpr::App {
        function: expr_arena.alloc(LogicExpr::Atom(f)),
        argument: expr_arena.alloc(LogicExpr::Atom(x)),
    };

    assert_eq!(
        expr.resolve(&interner),
        ExprView::App {
            function: Box::new(ExprView::Atom("f")),
            argument: Box::new(ExprView::Atom("x")),
        }
    );
}

#[test]
fn expr_view_equality_complex() {
    let a = ExprView::Quantifier {
        kind: QuantifierKind::Universal,
        variable: "x",
        body: Box::new(ExprView::Predicate {
            name: "P",
            args: vec![TermView::Variable("x")],
        }),
    };
    let b = ExprView::Quantifier {
        kind: QuantifierKind::Universal,
        variable: "x",
        body: Box::new(ExprView::Predicate {
            name: "P",
            args: vec![TermView::Variable("x")],
        }),
    };
    assert_eq!(a, b);
}
