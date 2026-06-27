//! Tarski geometry on the multi-theorem driver — the Euclid path in miniature. A
//! shared axiom base (Tarski's congruence axioms) is in scope for the whole library,
//! and theorems are discharged in citation order: `cong_symmetry` is proved from
//! `cong_reflexivity`, which is itself proved from the axioms. All kernel-certified.
//!
//! `Cong(a,b,c,d)` means "segment ab is congruent to segment cd".
//!   A1 (pseudo-reflexivity):   ∀a b.         Cong(a,b,b,a)
//!   A2 (inner transitivity):   ∀a b c d e f. (Cong(a,b,c,d) ∧ Cong(a,b,e,f)) → Cong(c,d,e,f)

use logicaffeine_proof::verify::{prove_library_with_axioms, LibraryTheorem};
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
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}

/// A2: ∀a b c d e f. (Cong(a,b,c,d) ∧ Cong(a,b,e,f)) → Cong(c,d,e,f)
fn tarski_a2() -> ProofExpr {
    forall(
        &["a", "b", "c", "d", "e", "f"],
        implies(
            and(
                cong(v("a"), v("b"), v("c"), v("d")),
                cong(v("a"), v("b"), v("e"), v("f")),
            ),
            cong(v("c"), v("d"), v("e"), v("f")),
        ),
    )
}

#[test]
fn tarski_inner_transitivity_chain_and_citation() {
    // The shared base is Tarski's inner-transitivity axiom A2.
    let axioms = vec![tarski_a2()];

    // cong_a: Cong(P,Q,R,S), Cong(P,Q,T,U) ⊢ Cong(R,S,T,U)
    //   — A2 instantiated at (P,Q,R,S,T,U); both antecedents are hypotheses.
    let cong_a = LibraryTheorem {
        name: "cong_RSTU".to_string(),
        premises: vec![
            cong(k("P"), k("Q"), k("R"), k("S")),
            cong(k("P"), k("Q"), k("T"), k("U")),
        ],
        goal: cong(k("R"), k("S"), k("T"), k("U")),
        cites: vec![],
    };

    // cong_b (cites cong_RSTU): Cong(R,S,V,W) ⊢ Cong(T,U,V,W)
    //   — A2 at (R,S,T,U,V,W), where the first antecedent Cong(R,S,T,U) is the
    //   conclusion of cong_RSTU, supplied by citation.
    let cong_b = LibraryTheorem {
        name: "cong_TUVW".to_string(),
        premises: vec![cong(k("R"), k("S"), k("V"), k("W"))],
        goal: cong(k("T"), k("U"), k("V"), k("W")),
        cites: vec!["cong_RSTU".to_string()],
    };

    let r = prove_library_with_axioms(&axioms, &[cong_a, cong_b]);
    assert!(
        r[0].verified,
        "cong_RSTU from A2 + hypotheses: {:?}",
        r[0].verification_error
    );
    assert!(
        r[1].verified,
        "cong_TUVW (cites cong_RSTU): {:?}",
        r[1].verification_error
    );
}
