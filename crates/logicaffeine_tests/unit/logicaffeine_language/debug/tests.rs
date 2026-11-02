use super::*;
use logicaffeine_base::Arena;

#[test]
fn symbol_display_with_interner() {
    let mut interner = Interner::new();
    let sym = interner.intern("Socrates");
    assert_eq!(sym.with(&interner).to_string(), "Socrates");
}

#[test]
fn symbol_empty_displays_empty() {
    let interner = Interner::new();
    assert_eq!(Symbol::EMPTY.with(&interner).to_string(), "");
}

#[test]
fn term_constant_display() {
    let mut interner = Interner::new();
    let sym = interner.intern("John");
    let term = Term::Constant(sym);
    assert_eq!(term.with(&interner).to_string(), "John");
}

#[test]
fn term_variable_display() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let term = Term::Variable(x);
    assert_eq!(term.with(&interner).to_string(), "x");
}

#[test]
fn term_function_display() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let f = interner.intern("father");
    let j = interner.intern("John");
    let term = Term::Function(f, term_arena.alloc_slice([Term::Constant(j)]));
    assert_eq!(term.with(&interner).to_string(), "father(John)");
}

#[test]
fn term_group_display() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let j = interner.intern("John");
    let m = interner.intern("Mary");
    let term = Term::Group(term_arena.alloc_slice([Term::Constant(j), Term::Constant(m)]));
    assert_eq!(term.with(&interner).to_string(), "[John ⊕ Mary]");
}

#[test]
fn term_possessed_display() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let j = interner.intern("John");
    let dog = interner.intern("dog");
    let term = Term::Possessed {
        possessor: term_arena.alloc(Term::Constant(j)),
        possessed: dog,
    };
    assert_eq!(term.with(&interner).to_string(), "John.dog");
}

#[test]
fn expr_predicate_display() {
    let mut interner = Interner::new();
    let term_arena: Arena<Term> = Arena::new();
    let mortal = interner.intern("Mortal");
    let x = interner.intern("x");
    let expr = LogicExpr::Predicate {
        name: mortal,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    };
    assert_eq!(expr.with(&interner).to_string(), "Mortal(x)");
}

#[test]
fn expr_quantifier_display() {
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
    assert_eq!(expr.with(&interner).to_string(), "∀x.Mortal(x)");
}

#[test]
fn expr_atom_display() {
    let mut interner = Interner::new();
    let p = interner.intern("P");
    let expr = LogicExpr::Atom(p);
    assert_eq!(expr.with(&interner).to_string(), "P");
}

#[test]
fn expr_binary_op_display() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let p = interner.intern("P");
    let q = interner.intern("Q");
    let expr = LogicExpr::BinaryOp {
        left: expr_arena.alloc(LogicExpr::Atom(p)),
        op: TokenType::And,
        right: expr_arena.alloc(LogicExpr::Atom(q)),
    };
    assert_eq!(expr.with(&interner).to_string(), "(P ∧ Q)");
}

#[test]
fn expr_lambda_display() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let x = interner.intern("x");
    let p = interner.intern("P");
    let expr = LogicExpr::Lambda {
        variable: x,
        body: expr_arena.alloc(LogicExpr::Atom(p)),
    };
    assert_eq!(expr.with(&interner).to_string(), "λx.P");
}

#[test]
fn debug_world_works_with_dbg_pattern() {
    let mut interner = Interner::new();
    let sym = interner.intern("test");
    let term = Term::Constant(sym);
    let debug_str = format!(
        "{:?}",
        DebugWorld {
            target: &term,
            interner: &interner
        }
    );
    assert!(debug_str.contains("test"));
}

#[test]
fn expr_temporal_display() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let p = interner.intern("Run");
    let expr = LogicExpr::Temporal {
        operator: TemporalOperator::Past,
        body: expr_arena.alloc(LogicExpr::Atom(p)),
    };
    assert_eq!(expr.with(&interner).to_string(), "P(Run)");
}

#[test]
fn expr_modal_display() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let p = interner.intern("Rain");
    let expr = LogicExpr::Modal {
        vector: crate::ast::ModalVector {
            domain: crate::ast::ModalDomain::Alethic,
            force: 1.0,
            flavor: crate::ast::ModalFlavor::Root,
            modal_base: None,
            ordering_source: None,
        },
        operand: expr_arena.alloc(LogicExpr::Atom(p)),
    };
    assert_eq!(expr.with(&interner).to_string(), "□(Rain)");
}

/// BUG-033: force == 0.5 (ability/epistemic can/could/may) is the
/// possibility side (◇), as the formatter and Kripke lowering both treat it.
/// The debug display must agree, not render □.
#[test]
fn expr_modal_display_force_half_is_possibility_not_necessity() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let fly = interner.intern("Fly");
    let expr = LogicExpr::Modal {
        vector: crate::ast::ModalVector {
            domain: crate::ast::ModalDomain::Alethic,
            force: 0.5,
            flavor: crate::ast::ModalFlavor::Root,
            modal_base: None,
            ordering_source: None,
        },
        operand: expr_arena.alloc(LogicExpr::Atom(fly)),
    };
    assert_eq!(expr.with(&interner).to_string(), "◇(Fly)");
}
