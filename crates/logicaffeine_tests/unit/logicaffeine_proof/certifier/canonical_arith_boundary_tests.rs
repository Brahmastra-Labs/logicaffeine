use super::*;

#[test]
fn kernel_arith_name_maps_canonical_to_ring() {
    // The shared IR is canonicalised to Add/Sub/Mul/Div; the kernel ring axioms
    // and LIA prover speak lowercase add/sub/mul/div. The boundary translates.
    assert_eq!(kernel_arith_name("Add"), "add");
    assert_eq!(kernel_arith_name("Sub"), "sub");
    assert_eq!(kernel_arith_name("Mul"), "mul");
    assert_eq!(kernel_arith_name("Div"), "div");
    // Everything else passes through verbatim (measure fns, relations, names).
    assert_eq!(kernel_arith_name("Score"), "Score");
    assert_eq!(kernel_arith_name("add"), "add");
}

#[test]
fn canonical_add_lowers_to_kernel_ring_head() {
    // ProofTerm::Function("Add", [x, y]) must lower to ((add x) y) so the
    // kernel arith prover (which matches Global("add")) can discharge it.
    let t = proof_term_to_kernel_term(&ProofTerm::Function(
        "Add".to_string(),
        vec![
            ProofTerm::Variable("x".to_string()),
            ProofTerm::Variable("y".to_string()),
        ],
    ))
    .expect("Add term converts to a kernel term");
    match t {
        Term::App(f, _) => match *f {
            Term::App(g, _) => match *g {
                Term::Global(name) => assert_eq!(
                    name, "add",
                    "canonical Add must lower to the kernel ring name 'add'; got {name}"
                ),
                other => panic!("expected a Global head, got {:?}", other),
            },
            other => panic!("expected a nested App, got {:?}", other),
        },
        other => panic!("expected an App, got {:?}", other),
    }
}

// A numeric constant is ambiguous: an arithmetic `Int` literal in an Int position,
// an opaque `Entity` LABEL (a grid year `2004`) in a relation position. The position
// decides — this is what keeps a monomorphic relation (`in : Entity → Entity → Prop`,
// shared by a logic grid's year and state columns) well-typed. These pin both arms;
// the regression they guard froze the Studio's Simon puzzle by making `in(Beta, 2002)`
// fail certification (`expected Entity, found Int`), which fell through to an unbounded
// backward-chainer grind.

/// The second argument of `in(Beta, 2002)` is the YEAR ENTITY `2002`, so it must lower
/// to `Global("2002")` (resolving to the `Entity` constant `verify.rs` registers),
/// never an `Int` literal — otherwise it clashes with `in : Entity → Entity → Prop`.
#[test]
fn numeric_predicate_argument_lowers_to_entity_label() {
    let in_beta_2002 = ProofExpr::Predicate {
        name: "in".to_string(),
        args: vec![
            ProofTerm::Constant("Beta".to_string()),
            ProofTerm::Constant("2002".to_string()),
        ],
        world: None,
    };
    let t = proof_expr_to_type(&in_beta_2002).expect("predicate lowers to a kernel type");
    // (in Beta) 2002 — the outer App's argument is the year term.
    match t {
        Term::App(_, year) => assert!(
            matches!(*year, Term::Global(ref n) if n == "2002"),
            "year `2002` in a relation must be an Entity Global, got {:?}",
            year
        ),
        other => panic!("expected `(in Beta) 2002` application, got {:?}", other),
    }
}

/// The dual: a numeric under an arithmetic builtin stays an `Int` literal so the
/// kernel's δ-rules fire (`le 2 5 ⇝ true`). The fix must not touch this path.
#[test]
fn numeric_arithmetic_operand_stays_int_literal() {
    use logicaffeine_kernel::Literal;
    let le_2_5 = ProofTerm::Function(
        "le".to_string(),
        vec![
            ProofTerm::Constant("2".to_string()),
            ProofTerm::Constant("5".to_string()),
        ],
    );
    let t = proof_term_to_kernel_term(&le_2_5).expect("le term converts");
    match t {
        Term::App(_, five) => assert!(
            matches!(*five, Term::Lit(Literal::Int(5))),
            "an arithmetic operand must stay an Int literal, got {:?}",
            five
        ),
        other => panic!("expected `(le 2) 5` application, got {:?}", other),
    }
}

/// Entity-position lowering keeps arithmetic builtins nested inside it on the `Int`
/// path (their operands still compute), while a bare numeric becomes an Entity label.
#[test]
fn entity_lowering_is_numeric_aware_but_arith_preserving() {
    use logicaffeine_kernel::Literal;
    assert!(
        matches!(
            proof_term_to_kernel_term_entity(&ProofTerm::Constant("2004".to_string())).unwrap(),
            Term::Global(ref n) if n == "2004"
        ),
        "a bare numeric in an entity position is an Entity label"
    );
    // add(2, 3) under an entity position still lowers its operands as Int literals.
    let add = ProofTerm::Function(
        "add".to_string(),
        vec![
            ProofTerm::Constant("2".to_string()),
            ProofTerm::Constant("3".to_string()),
        ],
    );
    match proof_term_to_kernel_term_entity(&add).unwrap() {
        Term::App(_, three) => assert!(
            matches!(*three, Term::Lit(Literal::Int(3))),
            "an arithmetic operand stays Int even inside an entity position, got {:?}",
            three
        ),
        other => panic!("expected `(add 2) 3`, got {:?}", other),
    }
}
