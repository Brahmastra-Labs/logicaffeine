//! Tarski BETWEENNESS on the surface development machinery — extending the congruence
//! base (Cong, A1–A3) with the betweenness primitive `Bet(a,b,c)` ("b is between a and c")
//! and its axioms (A4 segment construction, A7 Pasch, A6 identity of betweenness). Every
//! axiom and theorem is written as formal surface text and discharged, kernel-certified,
//! by the multi-theorem driver — the Tarski half of "sit Euclid on Tarski".
//!
//! This file maps what the directed-cut prover reaches with betweenness: the reachable
//! moves are DIRECT INSTANTIATION of an axiom (its ∀-variables pinned by the goal, its
//! existential body or antecedents discharged against hypotheses).

use logicaffeine_proof::development::prove_development;

fn assert_all_verified(body: &str) {
    let results = prove_development(body).expect("development should parse");
    assert!(!results.is_empty(), "no theorems found");
    for (name, r) in &results {
        assert!(r.verified, "theorem '{name}' must be kernel-certified: {:?}", r.verification_error);
    }
}

#[test]
fn segment_construction_lays_off_a_betweenness_and_congruence() {
    // A4 (segment construction): from any q, a, lay off a segment congruent to bc beyond a
    // — ∀q a b c. ∃x. Bet(q,a,x) ∧ Cong(a,x,b,c). Proving the instance ∃x. Bet(Q,A,x) ∧
    // Cong(A,x,B,C) is direct ∀-instantiation of an axiom with a conjunctive existential
    // body. The move behind every Euclidean "extend the segment / construct the point".
    assert_all_verified(
        "
        Axiom segment_construction: for all q a b c,
            there exists x, Bet(q, a, x) and Cong(a, x, b, c).

        Theorem construct_point: prove there exists x, Bet(Q, A, x) and Cong(A, x, B, C).
        ",
    );
}

#[test]
fn pasch_inner_form_instantiated_against_hypotheses() {
    // A7 (inner Pasch): two cevians of a triangle meet — Bet(a,p,c) ∧ Bet(b,q,c) →
    // ∃x. Bet(p,x,b) ∧ Bet(q,x,a). With both antecedents as hypotheses, proving the
    // existential conclusion is direct instantiation + antecedent discharge.
    assert_all_verified(
        "
        Axiom pasch: for all a p c b q,
            if Bet(a, p, c) and Bet(b, q, c) then there exists x, Bet(p, x, b) and Bet(q, x, a).

        Theorem cevians_meet:
            given Bet(A, P, C);
            given Bet(B, Q, C);
            prove there exists x, Bet(P, x, B) and Bet(Q, x, A).
        ",
    );
}

#[test]
fn identity_of_betweenness_collapses_a_degenerate_triple() {
    // A6 (identity of betweenness): Bet(a,b,a) → a = b. A degenerate "between" forces the
    // endpoints to coincide. Backward-chaining a rule whose CONSEQUENT is an equality,
    // antecedent discharged against the hypothesis.
    assert_all_verified(
        "
        Axiom identity_of_betweenness: for all a b, if Bet(a, b, a) then a = b.

        Theorem degenerate_is_point: given Bet(P, Q, P); prove P = Q.
        ",
    );
}

#[test]
fn mixed_congruence_and_betweenness_base_in_one_theory() {
    // A full opening Tarski base — congruence (A1, A2) AND betweenness (A4 construction,
    // A6 identity) in one development — discharging both a congruence theorem and a
    // betweenness theorem against the shared base. Proves the two primitives coexist.
    assert_all_verified(
        "
        Axiom cong_pseudo_reflexivity: for all a b, Cong(a, b, b, a).
        Axiom cong_inner_transitivity: for all a b c d e f,
            if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).
        Axiom segment_construction: for all q a b c,
            there exists x, Bet(q, a, x) and Cong(a, x, b, c).
        Axiom identity_of_betweenness: for all a b, if Bet(a, b, a) then a = b.

        Theorem cong_reflexivity: prove for all a b, Cong(a, b, a, b).
        Theorem construct_point: prove there exists x, Bet(Q, A, x) and Cong(A, x, B, C).
        Theorem degenerate_is_point: given Bet(P, Q, P); prove P = Q.
        ",
    );
}

#[test]
fn five_segment_axiom_instantiated_against_hypotheses() {
    // A5 (the five-segment axiom): the congruence-preservation engine of Tarski geometry.
    // If two 5-point configurations agree on the four congruences of an "outer" frame and
    // share a betweenness with a non-degenerate base, their fifth segments are congruent —
    // ∀…. (¬(a=b) ∧ Bet(a,b,c) ∧ Bet(a1,b1,c1) ∧ Cong(a,b,a1,b1) ∧ Cong(b,c,b1,c1) ∧
    // Cong(a,d,a1,d1) ∧ Cong(b,d,b1,d1)) → Cong(c,d,c1,d1). With every antecedent as a
    // hypothesis (incl. the inequality ¬(A=B)), the conclusion is direct instantiation.
    // (Primed points use a `p` suffix — `B1`/`B0` are reserved as the kernel's `Bit`
    // constructors, so a point literally named `B1` would type as a Bit, not a point.)
    assert_all_verified(
        "
        Axiom five_segment: for all a b c d ap bp cp dp,
            if not (a = b) and Bet(a, b, c) and Bet(ap, bp, cp)
               and Cong(a, b, ap, bp) and Cong(b, c, bp, cp)
               and Cong(a, d, ap, dp) and Cong(b, d, bp, dp)
            then Cong(c, d, cp, dp).

        Theorem outer_five_segment:
            given not (A = B);
            given Bet(A, B, C);
            given Bet(Ap, Bp, Cp);
            given Cong(A, B, Ap, Bp);
            given Cong(B, C, Bp, Cp);
            given Cong(A, D, Ap, Dp);
            given Cong(B, D, Bp, Dp);
            prove Cong(C, D, Cp, Dp).
        ",
    );
}

#[test]
fn five_segment_proves_against_the_full_seven_axiom_base() {
    // The scenario that HUNG before the relevance heuristic: proving a `Cong` goal with the
    // whole 7-axiom Tarski base in scope. Naive KB-order search tried the recursive inner-
    // transitivity axiom first and exploded. With antecedent-pinnability ordering, the prover
    // picks five-segment (all seven antecedents are hypotheses) first → a direct proof, FAST.
    assert_all_verified(
        "
        Axiom a1: for all a b, Cong(a, b, b, a).
        Axiom a2: for all a b c d e f, if Cong(a,b,c,d) and Cong(a,b,e,f) then Cong(c,d,e,f).
        Axiom a3: for all a b c, if Cong(a,b,c,c) then a = b.
        Axiom a4: for all q a b c, there exists x, Bet(q,a,x) and Cong(a,x,b,c).
        Axiom a6: for all a b, if Bet(a,b,a) then a = b.
        Axiom a7: for all a p c b q, if Bet(a,p,c) and Bet(b,q,c) then there exists x, Bet(p,x,b) and Bet(q,x,a).
        Axiom a5: for all a b c d ap bp cp dp, if not (a=b) and Bet(a,b,c) and Bet(ap,bp,cp) and Cong(a,b,ap,bp) and Cong(b,c,bp,cp) and Cong(a,d,ap,dp) and Cong(b,d,bp,dp) then Cong(c,d,cp,dp).
        Theorem t:
            given not (A=B); given Bet(A,B,C); given Bet(Ap,Bp,Cp);
            given Cong(A,B,Ap,Bp); given Cong(B,C,Bp,Cp); given Cong(A,D,Ap,Dp); given Cong(B,D,Bp,Dp);
            prove Cong(C,D,Cp,Dp).
        ",
    );
}
