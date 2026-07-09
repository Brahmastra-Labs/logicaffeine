//! The formal-formula parser (`logicaffeine_proof::formula`) — text → `ProofExpr`.
//!
//! The decisive tests assert the parser reproduces the EXACT `ProofExpr` structures of
//! the kernel-certified Tarski axioms in `tarski_geometry.rs`: if `parse_formula` of the
//! surface text equals the hand-built axiom, the surface seam is faithful to the proven
//! development.

use logicaffeine_proof::formula::{parse_formula, FormulaError};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn cong(a: ProofTerm, b: ProofTerm, c: ProofTerm, d: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: "Cong".to_string(), args: vec![a, b, c, d], world: None }
}
fn forall(vars: &[&str], body: ProofExpr) -> ProofExpr {
    vars.iter().rev().fold(body, |acc, var| ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(acc),
    })
}
fn exists(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists { variable: var.to_string(), body: Box::new(body) }
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn eq(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}

// ---------------------------------------------------------------------------
// The Tarski axioms, parsed from surface text, must EQUAL the proven structures.
// ---------------------------------------------------------------------------

#[test]
fn tarski_a1_pseudo_reflexivity() {
    // A1: ∀a b. Cong(a,b,b,a)
    let parsed = parse_formula("for all a b, Cong(a, b, b, a)").unwrap();
    let expected = forall(&["a", "b"], cong(v("a"), v("b"), v("b"), v("a")));
    assert_eq!(parsed, expected);
}

#[test]
fn tarski_a2_inner_transitivity() {
    // A2: ∀a b c d e f. (Cong(a,b,c,d) ∧ Cong(a,b,e,f)) → Cong(c,d,e,f)
    let parsed = parse_formula(
        "for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f)",
    )
    .unwrap();
    let expected = forall(
        &["a", "b", "c", "d", "e", "f"],
        implies(
            and(
                cong(v("a"), v("b"), v("c"), v("d")),
                cong(v("a"), v("b"), v("e"), v("f")),
            ),
            cong(v("c"), v("d"), v("e"), v("f")),
        ),
    );
    assert_eq!(parsed, expected);
}

#[test]
fn tarski_a3_identity_of_congruence() {
    // A3: ∀a b c. Cong(a,b,c,c) → a = b
    let parsed =
        parse_formula("for all a b c, if Cong(a, b, c, c) then a = b").unwrap();
    let expected = forall(
        &["a", "b", "c"],
        implies(cong(v("a"), v("b"), v("c"), v("c")), eq(v("a"), v("b"))),
    );
    assert_eq!(parsed, expected);
}

#[test]
fn tarski_a4_segment_construction_existential_body() {
    // A4 (simplified): ∀b c d. ∃x. Cong(b,x,c,d)
    let parsed =
        parse_formula("for all b c d, there exists x, Cong(b, x, c, d)").unwrap();
    let expected = forall(
        &["b", "c", "d"],
        exists("x", cong(v("b"), v("x"), v("c"), v("d"))),
    );
    assert_eq!(parsed, expected);
}

// ---------------------------------------------------------------------------
// Notation: symbolic spellings, the `implies` infix, constants vs variables.
// ---------------------------------------------------------------------------

#[test]
fn symbolic_and_english_spellings_agree() {
    let english =
        parse_formula("for all a b c d, if Cong(a,b,c,d) and Cong(a,b,c,d) then Cong(a,b,c,d)")
            .unwrap();
    let symbolic =
        parse_formula("∀ a b c d, Cong(a,b,c,d) ∧ Cong(a,b,c,d) → Cong(a,b,c,d)").unwrap();
    assert_eq!(english, symbolic);
}

#[test]
fn uppercase_is_constant_lowercase_is_variable() {
    // A goal as a real theorem states it: Cong over fixed points P, Q (constants).
    let parsed = parse_formula("Cong(P, Q, P, Q)").unwrap();
    let expected = cong(k("P"), k("Q"), k("P"), k("Q"));
    assert_eq!(parsed, expected);
}

#[test]
fn implication_is_right_associative() {
    // a -> b -> c parses as a -> (b -> c)
    let parsed = parse_formula("P implies Q implies R").unwrap();
    let expected = implies(
        ProofExpr::Atom("P".to_string()),
        implies(ProofExpr::Atom("Q".to_string()), ProofExpr::Atom("R".to_string())),
    );
    assert_eq!(parsed, expected);
}

#[test]
fn and_binds_tighter_than_implies() {
    // a and b implies c  ==  (a and b) implies c
    let parsed = parse_formula("P and Q implies R").unwrap();
    let expected = implies(
        and(ProofExpr::Atom("P".to_string()), ProofExpr::Atom("Q".to_string())),
        ProofExpr::Atom("R".to_string()),
    );
    assert_eq!(parsed, expected);
}

#[test]
fn period_terminator_is_accepted() {
    let with_period = parse_formula("for all a b, Cong(a, b, b, a).").unwrap();
    let without = parse_formula("for all a b, Cong(a, b, b, a)").unwrap();
    assert_eq!(with_period, without);
}

#[test]
fn malformed_input_is_an_error() {
    assert!(matches!(parse_formula("for all"), Err(FormulaError { .. })));
    assert!(matches!(parse_formula("Cong(a, b,"), Err(FormulaError { .. })));
    assert!(matches!(parse_formula("if P"), Err(FormulaError { .. })));
    assert!(matches!(parse_formula(""), Err(FormulaError { .. })));
}

// ---------------------------------------------------------------------------
// END-TO-END: axioms + goal PARSED FROM TEXT drive the kernel-certified prover.
// ---------------------------------------------------------------------------

#[test]
fn parsed_tarski_axioms_prove_reflexivity_kernel_certified() {
    use logicaffeine_proof::verify::{prove_library_with_axioms, LibraryTheorem};
    // The Tarski congruence base — A1 + A2 — parsed from surface text, not hand-built.
    let axioms = vec![
        parse_formula("for all a b, Cong(a, b, b, a)").unwrap(),
        parse_formula(
            "for all a b c d e f, if Cong(a,b,c,d) and Cong(a,b,e,f) then Cong(c,d,e,f)",
        )
        .unwrap(),
    ];
    let reflexivity = LibraryTheorem {
        name: "cong_reflexivity".to_string(),
        premises: vec![],
        goal: parse_formula("Cong(P, Q, P, Q)").unwrap(),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[reflexivity]);
    assert!(
        r[0].verified,
        "parsed Tarski A1+A2 ⊢ Cong(P,Q,P,Q): {:?}",
        r[0].verification_error
    );
}

#[test]
fn parsed_tarski_universal_symmetry_cites_reflexivity_kernel_certified() {
    use logicaffeine_proof::verify::{prove_library_with_axioms, LibraryTheorem};
    let axioms = vec![
        parse_formula("for all a b, Cong(a, b, b, a)").unwrap(),
        parse_formula(
            "for all a b c d e f, if Cong(a,b,c,d) and Cong(a,b,e,f) then Cong(c,d,e,f)",
        )
        .unwrap(),
    ];
    let reflexivity = LibraryTheorem {
        name: "reflexivity".to_string(),
        premises: vec![],
        goal: parse_formula("for all a b, Cong(a, b, a, b)").unwrap(),
        cites: vec![],
    };
    let symmetry = LibraryTheorem {
        name: "symmetry".to_string(),
        premises: vec![],
        goal: parse_formula("for all a b c d, if Cong(a,b,c,d) then Cong(c,d,a,b)").unwrap(),
        cites: vec!["reflexivity".to_string()],
    };
    let r = prove_library_with_axioms(&axioms, &[reflexivity, symmetry]);
    assert!(r[0].verified, "parsed reflexivity: {:?}", r[0].verification_error);
    assert!(
        r[1].verified,
        "parsed universal symmetry (cites reflexivity): {:?}",
        r[1].verification_error
    );
}

#[test]
fn a_quantifier_may_be_an_operand_not_only_top_level() {
    // FOL `P → ∃x. Q`: the consequent of an implication may START with a quantifier (the
    // shape of Tarski's segment-construction and Pasch axioms). The quantifier extends
    // rightward over the rest of the consequent.
    let parsed = parse_formula("for all a, if R(a) then there exists b, S(a, b)").unwrap();
    let s = |args: Vec<ProofTerm>| ProofExpr::Predicate { name: "S".to_string(), args, world: None };
    let expected = ProofExpr::ForAll {
        variable: "a".to_string(),
        body: Box::new(implies(
            ProofExpr::Predicate { name: "R".to_string(), args: vec![v("a")], world: None },
            ProofExpr::Exists { variable: "b".to_string(), body: Box::new(s(vec![v("a"), v("b")])) },
        )),
    };
    assert_eq!(parsed, expected);

    // A quantifier after `and` also binds rightward: `P and ∃x. Q(x)`.
    let p2 = parse_formula("Flag and there exists x, Q(x)").unwrap();
    let expected2 = and(
        ProofExpr::Atom("Flag".to_string()),
        ProofExpr::Exists {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::Predicate { name: "Q".to_string(), args: vec![v("x")], world: None }),
        },
    );
    assert_eq!(p2, expected2);
}
