use super::*;
use crate::arena::Arena;

#[test]
fn test_convert_predicate() {
    let mut interner = Interner::new();
    let name = interner.intern("Man");
    let arg = interner.intern("socrates");

    let arena: Arena<Term> = Arena::new();
    let args = arena.alloc_slice([Term::Constant(arg)]);

    let expr = LogicExpr::Predicate {
        name,
        args,
        world: None,
    };

    let result = logic_expr_to_proof_expr(&expr, &interner);

    match result {
        ProofExpr::Predicate { name, args, world } => {
            // Predicate names are normalized to lowercase
            assert_eq!(name, "man");
            assert_eq!(args.len(), 1);
            // Terms (constants) preserve their case
            assert!(matches!(&args[0], ProofTerm::Constant(s) if s == "socrates"));
            assert!(world.is_none());
        }
        _ => panic!("Expected Predicate, got {:?}", result),
    }
}

#[test]
fn test_convert_universal() {
    let mut interner = Interner::new();
    let var = interner.intern("x");
    let pred = interner.intern("P");

    let arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let body = arena.alloc(LogicExpr::Predicate {
        name: pred,
        args: term_arena.alloc_slice([Term::Variable(var)]),
        world: None,
    });

    let expr = LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: var,
        body,
        island_id: 0,
    };

    let result = logic_expr_to_proof_expr(&expr, &interner);

    match result {
        ProofExpr::ForAll { variable, body } => {
            assert_eq!(variable, "x");
            assert!(matches!(*body, ProofExpr::Predicate { .. }));
        }
        _ => panic!("Expected ForAll, got {:?}", result),
    }
}

#[test]
fn test_convert_implication() {
    let mut interner = Interner::new();
    let p = interner.intern("P");
    let q = interner.intern("Q");

    let arena: Arena<LogicExpr> = Arena::new();

    let left = arena.alloc(LogicExpr::Atom(p));
    let right = arena.alloc(LogicExpr::Atom(q));

    let expr = LogicExpr::BinaryOp {
        left,
        op: TokenType::If,
        right,
    };

    let result = logic_expr_to_proof_expr(&expr, &interner);

    match result {
        ProofExpr::Implies(l, r) => {
            assert!(matches!(*l, ProofExpr::Atom(ref s) if s == "P"));
            assert!(matches!(*r, ProofExpr::Atom(ref s) if s == "Q"));
        }
        _ => panic!("Expected Implies, got {:?}", result),
    }
}

// ---- Solver-canonical arithmetic vocabulary (the parser↔oracle bridge) ----
//
// The parser names comparison directions "Greater"/"Less"/… and arithmetic
// offsets "add"/"sub", but the proof oracle recognises ONLY the canonical
// "Gt"/"Lt"/"Gte"/"Lte"/"Eq"/"Neq" predicates and "Add"/"Sub"/"Mul"/"Div"
// functions, matched case-sensitively (oracle.rs:879, 1034;
// modal_translation.rs:124). If the bridge does not translate to the canonical
// vocabulary, every arithmetic/comparison clue silently degrades to an
// uninterpreted function and never constrains the model — a parse that compiles
// to FOL the prover cannot use. These pin the translation.

#[test]
fn arithmetic_offset_uses_canonical_add() {
    // "Tara scored 3 points higher than Bessie." → Score(tara) = add(Score(bessie), 3)
    let mut interner = Interner::new();
    let score = interner.intern("Score");
    let add = interner.intern("add");
    let tara = interner.intern("tara");
    let bessie = interner.intern("bessie");

    let terms: Arena<Term> = Arena::new();

    let score_tara = Term::Function(score, terms.alloc_slice([Term::Constant(tara)]));
    let score_bessie = Term::Function(score, terms.alloc_slice([Term::Constant(bessie)]));
    let offset = Term::Value {
        kind: crate::ast::logic::NumberKind::Integer(3),
        unit: None,
        dimension: None,
    };
    let rhs = Term::Function(add, terms.alloc_slice([score_bessie, offset]));
    let expr = LogicExpr::Identity {
        left: terms.alloc(score_tara),
        right: terms.alloc(rhs),
    };

    match logic_expr_to_proof_expr(&expr, &interner) {
        ProofExpr::Identity(_, ProofTerm::Function(name, args)) => {
            assert_eq!(
                name, "Add",
                "offset function must be canonical Add; got {name}"
            );
            assert!(
                matches!(&args[1], ProofTerm::Constant(s) if s == "3"),
                "offset constant must be the bare integer 3; got {:?}",
                args[1]
            );
        }
        other => panic!("expected Identity with an Add rhs, got {:?}", other),
    }
}

#[test]
fn arithmetic_equality_predicate_lowers_to_identity() {
    // Eq(Mul(a, b), Mul(b, a)) — an equality between ARITHMETIC expressions — lowers to a ring
    // IDENTITY (so the oracle can decide it), not an uninterpreted `Eq` predicate.
    let mut interner = Interner::new();
    let eq = interner.intern("Eq");
    let mul = interner.intern("mul");
    let a = interner.intern("a");
    let b = interner.intern("b");
    let terms: Arena<Term> = Arena::new();
    let mul_ab = Term::Function(
        mul,
        terms.alloc_slice([Term::Constant(a), Term::Constant(b)]),
    );
    let mul_ba = Term::Function(
        mul,
        terms.alloc_slice([Term::Constant(b), Term::Constant(a)]),
    );
    let expr = LogicExpr::Predicate {
        name: eq,
        args: terms.alloc_slice([mul_ab, mul_ba]),
        world: None,
    };
    match logic_expr_to_proof_expr(&expr, &interner) {
        ProofExpr::Identity(ProofTerm::Function(ln, _), ProofTerm::Function(rn, _)) => {
            assert_eq!(ln, "Mul", "left side is canonical Mul");
            assert_eq!(rn, "Mul", "right side is canonical Mul");
        }
        other => panic!(
            "arithmetic Eq must lower to Identity(Mul, Mul), got {:?}",
            other
        ),
    }
}

#[test]
fn non_arithmetic_equality_predicate_stays_a_predicate() {
    // Eq(x, y) over plain entities is NOT arithmetic — it must stay a Predicate so existing FOL
    // equality handling is untouched (the arithmetic→Identity lowering is gated on an arith side).
    let mut interner = Interner::new();
    let eq = interner.intern("Eq");
    let x = interner.intern("x");
    let y = interner.intern("y");
    let terms: Arena<Term> = Arena::new();
    let expr = LogicExpr::Predicate {
        name: eq,
        args: terms.alloc_slice([Term::Constant(x), Term::Constant(y)]),
        world: None,
    };
    match logic_expr_to_proof_expr(&expr, &interner) {
        ProofExpr::Predicate { name, .. } => assert_eq!(name, "Eq"),
        other => panic!("non-arithmetic Eq must stay a Predicate, got {:?}", other),
    }
}

#[test]
fn arithmetic_offset_uses_canonical_sub() {
    // "… 2 years before …" / "lower than" → ord(a) = sub(ord(b), 2)
    let mut interner = Interner::new();
    let ord = interner.intern("Ord");
    let sub = interner.intern("sub");
    let a = interner.intern("a");
    let b = interner.intern("b");
    let terms: Arena<Term> = Arena::new();
    let ord_a = Term::Function(ord, terms.alloc_slice([Term::Constant(a)]));
    let ord_b = Term::Function(ord, terms.alloc_slice([Term::Constant(b)]));
    let offset = Term::Value {
        kind: crate::ast::logic::NumberKind::Integer(2),
        unit: None,
        dimension: None,
    };
    let rhs = Term::Function(sub, terms.alloc_slice([ord_b, offset]));
    let expr = LogicExpr::Identity {
        left: terms.alloc(ord_a),
        right: terms.alloc(rhs),
    };
    match logic_expr_to_proof_expr(&expr, &interner) {
        ProofExpr::Identity(_, ProofTerm::Function(name, _)) => {
            assert_eq!(
                name, "Sub",
                "offset function must be canonical Sub; got {name}"
            );
        }
        other => panic!("expected Identity with a Sub rhs, got {:?}", other),
    }
}

fn convert_binary_predicate(raw_name: &str) -> String {
    let mut interner = Interner::new();
    let name = interner.intern(raw_name);
    let a = interner.intern("a");
    let b = interner.intern("b");
    let terms: Arena<Term> = Arena::new();
    let args = terms.alloc_slice([Term::Constant(a), Term::Constant(b)]);
    let expr = LogicExpr::Predicate {
        name,
        args,
        world: None,
    };
    match logic_expr_to_proof_expr(&expr, &interner) {
        ProofExpr::Predicate { name, .. } => name,
        other => panic!("expected Predicate, got {:?}", other),
    }
}

#[test]
fn comparison_predicates_use_canonical_names() {
    // The parser's comparison vocabulary must reach the oracle's exact names,
    // and must NOT be lowercased into oblivion ("greater" ≠ "Gt").
    assert_eq!(convert_binary_predicate("Greater"), "Gt");
    assert_eq!(convert_binary_predicate("Less"), "Lt");
    assert_eq!(convert_binary_predicate("GreaterEqual"), "Gte");
    assert_eq!(convert_binary_predicate("LessEqual"), "Lte");
    assert_eq!(convert_binary_predicate("Equal"), "Eq");
    assert_eq!(convert_binary_predicate("NotEqual"), "Neq");
}

#[test]
fn ordinary_binary_predicate_is_not_remapped_to_a_comparison() {
    // Regression: a genuine relational predicate is unaffected by the
    // comparison-name mapping — it keeps the noun-normalised lowercase form
    // and is never hijacked into an arithmetic comparison symbol.
    let name = convert_binary_predicate("Loves");
    assert!(
        !["Gt", "Lt", "Gte", "Lte", "Eq", "Neq"].contains(&name.as_str()),
        "ordinary predicate must not become a comparison op; got {name}"
    );
    assert_eq!(
        name,
        name.to_lowercase(),
        "ordinary predicate keeps the noun-normalised lowercase form; got {name}"
    );
}
