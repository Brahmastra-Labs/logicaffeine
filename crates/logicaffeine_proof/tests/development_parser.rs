//! The formal-development parser + driver: a `## Theory` body (formal `Axiom`/`Theorem`
//! declarations) parsed and discharged through the kernel-certified multi-theorem driver.
//! The headline test is the opening Tarski congruence development, proved entirely from
//! surface text.

use logicaffeine_proof::development::{parse_development, prove_development};
use logicaffeine_proof::{ProofExpr, ProofTerm};

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

#[test]
fn parses_axioms_and_theorems_with_names_premises_and_cites() {
    let dev = parse_development(
        "Axiom flip: for all a b, Cong(a, b, b, a). \
         Theorem symmetry cites flip: prove for all a b c d, if Cong(a,b,c,d) then Cong(c,d,a,b). \
         Theorem null_seg: given Cong(P, Q, R, R); prove P = Q.",
    )
    .expect("development should parse");

    assert_eq!(dev.axioms.len(), 1);
    assert_eq!(dev.axioms[0].0, "flip");
    assert_eq!(dev.axioms[0].1, forall(&["a", "b"], cong(v("a"), v("b"), v("b"), v("a"))));

    assert_eq!(dev.theorems.len(), 2);
    assert_eq!(dev.theorems[0].name, "symmetry");
    assert_eq!(dev.theorems[0].cites, vec!["flip".to_string()]);
    assert!(dev.theorems[0].premises.is_empty());

    assert_eq!(dev.theorems[1].name, "null_seg");
    assert_eq!(dev.theorems[1].premises.len(), 1, "null_seg has one 'given'");
    assert_eq!(
        dev.theorems[1].goal,
        ProofExpr::Identity(ProofTerm::Constant("P".to_string()), ProofTerm::Constant("Q".to_string()))
    );
}

#[test]
fn tarski_congruence_development_proved_from_surface_text_kernel_certified() {
    // The opening Tarski development — the same one hand-built in `tarski_geometry.rs`,
    // now written as formal surface text and discharged end to end:
    //   A1 + A2 are the shared base; reflexivity, then symmetry (cites reflexivity), then
    //   transitivity (cites symmetry) are proved in citation order, each kernel-certified.
    let body = "
        Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
        Axiom inner_transitivity: for all a b c d e f,
            if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).

        Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
        Theorem symmetry cites reflexivity:
            prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
        Theorem transitivity cites symmetry:
            prove for all a b c d e f,
            if Cong(a, b, c, d) and Cong(c, d, e, f) then Cong(a, b, e, f).
    ";

    let results = prove_development(body).expect("development should parse");
    assert_eq!(results.len(), 3, "three theorems");
    for (name, result) in &results {
        assert!(
            result.verified,
            "Tarski theorem '{name}' must be kernel-certified: {:?}",
            result.verification_error
        );
    }
}

#[test]
fn tarski_identity_theorem_with_a_given_premise() {
    // A3 (identity of congruence) + a theorem with a 'given' premise whose goal is an
    // EQUALITY — `Cong(P,Q,R,R)` forces `P = Q`. Proven from surface text, kernel-certified.
    let body = "
        Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
        Axiom inner_transitivity: for all a b c d e f,
            if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).
        Axiom identity: for all a b c, if Cong(a, b, c, c) then a = b.

        Theorem null_segment: given Cong(P, Q, R, R); prove P = Q.
    ";
    let results = prove_development(body).expect("development should parse");
    assert_eq!(results.len(), 1);
    assert!(
        results[0].1.verified,
        "null_segment identity (equality goal): {:?}",
        results[0].1.verification_error
    );
}

#[test]
fn n_ary_conjunctive_antecedent_certifies() {
    // Regression lock for the n-ary `ConjunctionIntro` fix: an axiom with a FOUR-conjunct
    // antecedent, discharged against four hypotheses. The engine must fold the four proofs
    // into a binary-nested ConjunctionIntro tree (not one illegal flat node) for the
    // certifier to accept it. Kernel-certified.
    let body = "
        Axiom quad: for all a b c d e f g h,
            if R(a, b) and R(c, d) and R(e, f) and R(g, h) then Goal(a, h).
        Theorem use_quad:
            given R(P, Q); given R(Q, S); given R(S, T); given R(T, U);
            prove Goal(P, U).
    ";
    let results = prove_development(body).expect("development should parse");
    assert_eq!(results.len(), 1);
    assert!(
        results[0].1.verified,
        "four-conjunct antecedent must certify: {:?}",
        results[0].1.verification_error
    );
}
