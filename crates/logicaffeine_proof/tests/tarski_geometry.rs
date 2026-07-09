//! Tarski geometry on the multi-theorem driver — the Euclid path in miniature. A
//! shared axiom base (Tarski's congruence axioms) is in scope for the whole library,
//! and theorems are discharged in citation order: `cong_symmetry` is proved from
//! `cong_reflexivity`, which is itself proved from the axioms. All kernel-certified.
//!
//! `Cong(a,b,c,d)` means "segment ab is congruent to segment cd".
//!   A1 (pseudo-reflexivity):   ∀a b.         Cong(a,b,b,a)
//!   A2 (inner transitivity):   ∀a b c d e f. (Cong(a,b,c,d) ∧ Cong(a,b,e,f)) → Cong(c,d,e,f)

use logicaffeine_proof::verify::{
    prove_certify_check_bounded, prove_library_with_axioms, LibraryTheorem,
};
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
fn eq(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}

/// A3: ∀a b c. Cong(a,b,c,c) → a = b  (identity of congruence: a null right segment
/// forces the left segment to be null too — the axiom whose consequent is an equality)
fn tarski_a3() -> ProofExpr {
    forall(
        &["a", "b", "c"],
        implies(cong(v("a"), v("b"), v("c"), v("c")), eq(v("a"), v("b"))),
    )
}
fn exists(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists { variable: var.to_string(), body: Box::new(body) }
}
/// A4 (segment construction, simplified): ∀b c d. ∃x. Cong(b,x,c,d) — any segment
/// `cd` can be laid off from point `b`. An axiom whose body is itself existential.
/// (Every `∀`-variable occurs in the body, so instantiation is fully determined.)
fn tarski_a4() -> ProofExpr {
    forall(
        &["b", "c", "d"],
        exists("x", cong(v("b"), v("x"), v("c"), v("d"))),
    )
}

/// A1: ∀a b. Cong(a,b,b,a)  (pseudo-reflexivity)
fn tarski_a1() -> ProofExpr {
    forall(&["a", "b"], cong(v("a"), v("b"), v("b"), v("a")))
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

#[test]
fn recursive_axiom_terminates_and_is_certified() {
    // Tarski A1 is a universal FACT and A2 is RECURSIVE (its conclusion `Cong` is
    // also its antecedent's predicate). A goal that re-enters A2 used to STACK
    // OVERFLOW; the antecedent solver is now depth-bounded, so the search terminates
    // — and `cong_RSTU` (proved above from A2 + hypotheses) certifies cleanly. The
    // fuller `cong_reflexivity` (the existential cut through A1) is proved in its own
    // test below.
    let axioms = vec![tarski_a1(), tarski_a2()];
    let cong_a = LibraryTheorem {
        name: "cong_RSTU".to_string(),
        premises: vec![
            cong(k("P"), k("Q"), k("R"), k("S")),
            cong(k("P"), k("Q"), k("T"), k("U")),
        ],
        goal: cong(k("R"), k("S"), k("T"), k("U")),
        cites: vec![],
    };
    // Terminates (no overflow) AND certifies — even with the recursive A2 + the
    // universal fact A1 both in scope.
    let r = prove_library_with_axioms(&axioms, &[cong_a]);
    assert!(
        r[0].verified,
        "recursive A2 (with A1 in scope) must terminate and certify: {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_congruence_reflexivity_from_a1_a2() {
    // ⊢ Cong(P,Q,P,Q), the classic first Tarski lemma, from A1 + A2 — needs the
    // EXISTENTIAL CUT: instantiate A1 at (Q,P) to get Cong(Q,P,P,Q), then A2 at
    // (Q,P,P,Q,P,Q) to conclude Cong(P,Q,P,Q). The middle terms a,b are determined by
    // the antecedent solve, not the goal, and their witnesses are only transitively
    // bound — fixpoint witness resolution now grounds them. Kernel-certified.
    let axioms = vec![tarski_a1(), tarski_a2()];
    let reflexivity = LibraryTheorem {
        name: "cong_reflexivity".to_string(),
        premises: vec![],
        goal: cong(k("P"), k("Q"), k("P"), k("Q")),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[reflexivity]);
    assert!(
        r[0].verified,
        "cong_reflexivity from Tarski A1 + A2 (existential cut): {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_congruence_symmetry_from_a1_a2() {
    // Cong(A,B,C,D) ⊢ Cong(C,D,A,B) — Tarski's Satz 2.2 (symmetry of congruence),
    // proved from A1 + A2 with the hypothesis in scope. A2 at (A,B,C,D,A,B) gives
    // (Cong(A,B,C,D) ∧ Cong(A,B,A,B)) → Cong(C,D,A,B): the first antecedent is the
    // hypothesis, the second is reflexivity (its own existential cut through A1). The
    // shared middle (a,b) is pinned by the hypothesis, then the reflexivity subgoal is
    // discharged — a nested cut on top of a cut. Kernel-certified.
    let axioms = vec![tarski_a1(), tarski_a2()];
    let symmetry = LibraryTheorem {
        name: "cong_symmetry".to_string(),
        premises: vec![cong(k("A"), k("B"), k("C"), k("D"))],
        goal: cong(k("C"), k("D"), k("A"), k("B")),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[symmetry]);
    assert!(
        r[0].verified,
        "cong_symmetry from Tarski A1 + A2 (nested cut): {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_congruence_transitivity_from_a1_a2() {
    // Cong(A,B,C,D), Cong(C,D,E,F) ⊢ Cong(A,B,E,F) — Tarski's Satz 2.3 (transitivity
    // of congruence), proved standalone from A1 + A2. A2 at (C,D,A,B,E,F) needs
    // Cong(C,D,A,B) ∧ Cong(C,D,E,F): the second is a hypothesis, the first is the
    // SYMMETRY of the other hypothesis — itself a nested cut, which in turn needs
    // reflexivity, the cut through A1. A triple-deep existential cut, kernel-certified.
    let axioms = vec![tarski_a1(), tarski_a2()];
    let transitivity = LibraryTheorem {
        name: "cong_transitivity".to_string(),
        premises: vec![
            cong(k("A"), k("B"), k("C"), k("D")),
            cong(k("C"), k("D"), k("E"), k("F")),
        ],
        goal: cong(k("A"), k("B"), k("E"), k("F")),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[transitivity]);
    assert!(
        r[0].verified,
        "cong_transitivity from Tarski A1 + A2 (triple cut): {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_universal_congruence_symmetry() {
    // ⊢ ∀a b c d. Cong(a,b,c,d) → Cong(c,d,a,b) — symmetry stated UNIVERSALLY, with
    // NO hypotheses, the way a real theorem (and a citable Euclid-graph lemma) reads.
    // This drives ∀-introduction (the bound vars are opaque eigenvariables the proof
    // may not instantiate) + →-introduction (assume the antecedent) + the existential
    // cut through A1/A2. The closed universal form is what lets later theorems CITE
    // and instantiate it. Kernel-certified.
    let axioms = vec![tarski_a1(), tarski_a2()];
    let symmetry = LibraryTheorem {
        name: "cong_symmetry_forall".to_string(),
        premises: vec![],
        goal: forall(
            &["a", "b", "c", "d"],
            implies(
                cong(v("a"), v("b"), v("c"), v("d")),
                cong(v("c"), v("d"), v("a"), v("b")),
            ),
        ),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[symmetry]);
    assert!(
        r[0].verified,
        "universal cong_symmetry (∀I + →I + cut): {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_universal_development_cited_library() {
    // The Euclid-graph engine on real geometry: the opening Tarski development as a
    // dependency-ordered library of UNIVERSAL lemmas, each citing the previous.
    //   reflexivity  : ∀a b.         Cong(a,b,a,b)
    //   symmetry     : ∀a b c d.     Cong(a,b,c,d) → Cong(c,d,a,b)        cites reflexivity
    //   transitivity : ∀a b c d e f. (Cong(a,b,c,d) ∧ Cong(c,d,e,f)) → Cong(a,b,e,f)  cites symmetry
    // Each proved lemma's universal conclusion enters the next proof as a citable
    // premise, instantiated on demand — so transitivity discharges its hard
    // antecedent by INSTANTIATING the symmetry lemma, not by re-deriving the cut.
    // Every theorem is independently kernel-certified.
    let axioms = vec![tarski_a1(), tarski_a2()];

    let reflexivity = LibraryTheorem {
        name: "reflexivity".to_string(),
        premises: vec![],
        goal: forall(&["a", "b"], cong(v("a"), v("b"), v("a"), v("b"))),
        cites: vec![],
    };
    let symmetry = LibraryTheorem {
        name: "symmetry".to_string(),
        premises: vec![],
        goal: forall(
            &["a", "b", "c", "d"],
            implies(
                cong(v("a"), v("b"), v("c"), v("d")),
                cong(v("c"), v("d"), v("a"), v("b")),
            ),
        ),
        cites: vec!["reflexivity".to_string()],
    };
    // Stated with a CONJUNCTIVE antecedent — A2's own shape, the natural Satz 2.3:
    // the single `→I` assumes `Cong ∧ Cong`, and forward ∧-elimination lets the cut's
    // existential middle pin against a conjunct (cheapest-first subgoal selection puts
    // that pinning sibling before the open one). Certifies in milliseconds.
    let transitivity = LibraryTheorem {
        name: "transitivity".to_string(),
        premises: vec![],
        goal: forall(
            &["a", "b", "c", "d", "e", "f"],
            implies(
                and(
                    cong(v("a"), v("b"), v("c"), v("d")),
                    cong(v("c"), v("d"), v("e"), v("f")),
                ),
                cong(v("a"), v("b"), v("e"), v("f")),
            ),
        ),
        cites: vec!["symmetry".to_string()],
    };

    let r = prove_library_with_axioms(&axioms, &[reflexivity, symmetry, transitivity]);
    assert!(r[0].verified, "reflexivity: {:?}", r[0].verification_error);
    assert!(r[1].verified, "symmetry (cites reflexivity): {:?}", r[1].verification_error);
    assert!(
        r[2].verified,
        "transitivity (cites symmetry): {:?}",
        r[2].verification_error
    );
}

#[test]
fn tarski_identity_axiom_derives_equality() {
    // A3: Cong(a,b,c,c) → a = b. From a single null-segment congruence, conclude the
    // two endpoints coincide. The prover must backward-chain a rule whose CONSEQUENT is
    // an EQUALITY (not a predicate) and discharge its antecedent against the hypothesis
    // — the move behind every "therefore X = Y" step in Euclid. Kernel-certified.
    let axioms = vec![tarski_a1(), tarski_a2(), tarski_a3()];
    let thm = LibraryTheorem {
        name: "null_segment_identity".to_string(),
        premises: vec![cong(k("P"), k("Q"), k("R"), k("R"))],
        goal: eq(k("P"), k("Q")),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[thm]);
    assert!(
        r[0].verified,
        "A3 identity (equality consequent): {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_identity_through_transitivity() {
    // Cong(A,B,C,D), Cong(C,D,E,E) ⊢ A = B. Two moves composed: transitivity gives
    // Cong(A,B,E,E), then A3 collapses it to A=B. The prover backward-chains A3 (an
    // equality consequent), leaves its null-segment witness `c` open, and pins it by
    // chaining transitivity through the premises — an existential cut feeding an
    // equality axiom, the texture of a real Euclid step. Kernel-certified.
    let axioms = vec![tarski_a1(), tarski_a2(), tarski_a3()];
    let thm = LibraryTheorem {
        name: "identity_via_transitivity".to_string(),
        premises: vec![
            cong(k("A"), k("B"), k("C"), k("D")),
            cong(k("C"), k("D"), k("E"), k("E")),
        ],
        goal: eq(k("A"), k("B")),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[thm]);
    assert!(
        r[0].verified,
        "identity via transitivity (cut into equality axiom): {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_existential_construction_axiom() {
    // A4 (segment construction, simplified): ∀a b c d. ∃x. Cong(b,x,c,d). Prove
    // ∃x. Cong(P,x,Q,R) — "there is a point x with Px ≅ QR" — by INSTANTIATING the
    // universal part of an axiom whose body is itself existential. This is the move
    // behind every Euclidean "construct the point such that …". Kernel-certified.
    let axioms = vec![tarski_a4()];
    let thm = LibraryTheorem {
        name: "construct_segment".to_string(),
        premises: vec![],
        goal: exists("x", cong(k("P"), v("x"), k("Q"), k("R"))),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[thm]);
    assert!(
        r[0].verified,
        "existential construction axiom (∀…∃ instantiation): {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_lemma_reuse_chain_supercrush() {
    // Lemma reuse is the math trick that breaks the re-derive-from-axioms depth ceiling.
    // With the transitivity LEMMA in hand (itself proved from A1+A2 elsewhere), a length-k
    // congruence chain is k SHALLOW applications — one instantiation per step, the cut's
    // existential middle pinned against a premise by cheapest-first selection — so we scale
    // far past k≈16. Z3 on the same chain runs into the tens of seconds (k=16 alone ≈ 7s);
    // we certify k=24 in milliseconds.
    let transitivity = forall(
        &["a", "b", "c", "d", "e", "f"],
        implies(
            and(
                cong(v("a"), v("b"), v("c"), v("d")),
                cong(v("c"), v("d"), v("e"), v("f")),
            ),
            cong(v("a"), v("b"), v("e"), v("f")),
        ),
    );
    let pt = |i: usize| ProofTerm::Constant(format!("P{i}"));
    // ~3 search levels per step, so a depth-200 budget covers k=40 (the global default
    // stays 100 — only this genuinely-deep proof opts in, riding the verify big-stack).
    let k = 40usize;
    let mut premises = vec![transitivity];
    premises.extend((0..k).map(|i| cong(pt(2 * i), pt(2 * i + 1), pt(2 * i + 2), pt(2 * i + 3))));
    let goal = cong(pt(0), pt(1), pt(2 * k), pt(2 * k + 1));
    let r = prove_certify_check_bounded(&premises, &goal, 200);
    assert!(
        r.verified,
        "k=40 chain via the transitivity lemma (Z3 from axioms times out >60s): {:?}",
        r.verification_error
    );
}

#[test]
fn tarski_existential_construction_with_vacuous_var() {
    // The FULL-arity construction axiom shape ∀a b c d. ∃x. Cong(b,x,c,d): the point
    // `a` is a genuine ∀-variable but does NOT occur in the body (in real A4 it sits in
    // a betweenness atom B(a,b,x); here it is vacuous). Proving ∃x. Cong(P,x,Q,R) must
    // still succeed — a ∀-variable absent from the matrix is instantiated by ANY entity,
    // so it is defaulted rather than rejected as "unpinned". Kernel-certified.
    let axioms = vec![forall(
        &["a", "b", "c", "d"],
        exists("x", cong(v("b"), v("x"), v("c"), v("d"))),
    )];
    let thm = LibraryTheorem {
        name: "construct_segment_vacuous".to_string(),
        premises: vec![],
        goal: exists("x", cong(k("P"), v("x"), k("Q"), k("R"))),
        cites: vec![],
    };
    let r = prove_library_with_axioms(&axioms, &[thm]);
    assert!(
        r[0].verified,
        "construction axiom with a vacuous ∀-variable: {:?}",
        r[0].verification_error
    );
}

#[test]
fn tarski_deep_transitivity_chain_beats_z3_scaling() {
    // A k-step congruence chain — Cong(p0,p1,p2,p3), Cong(p2,p3,p4,p5), …, Cong(p_{2k-2},
    // p_{2k-1}, p_{2k}, p_{2k+1}) ⊢ Cong(p0,p1,p_{2k},p_{2k+1}) — which is k chained
    // applications of transitivity, from A1+A2 alone. On the SAME problem Z3's MBQI
    // instantiation count explodes with depth (k=2→43, k=3→72, k=4→241, k=5→265); our
    // directed cut search chains it in milliseconds and certifies it. Each k proved.
    let axioms = vec![tarski_a1(), tarski_a2()];
    // Uppercase-leading names: the verifier reads a lowercase-leading constant as a
    // variable (standard FOL convention), so geometry points are `P0`, `P1`, ….
    let pt = |i: usize| ProofTerm::Constant(format!("P{i}"));
    for k in [2usize, 3, 4, 5, 8, 12, 16] {
        let premises: Vec<ProofExpr> = (0..k)
            .map(|i| cong(pt(2 * i), pt(2 * i + 1), pt(2 * i + 2), pt(2 * i + 3)))
            .collect();
        let goal = cong(pt(0), pt(1), pt(2 * k), pt(2 * k + 1));
        let thm = LibraryTheorem {
            name: format!("chain_{k}"),
            premises,
            goal,
            cites: vec![],
        };
        let r = prove_library_with_axioms(&axioms, &[thm]);
        assert!(
            r[0].verified,
            "deep transitivity chain k={k}: {:?}",
            r[0].verification_error
        );
    }
}

#[test]
fn tarski_universal_conjunctive_transitivity_is_fast() {
    // Transitivity stated with a CONJUNCTIVE antecedent — A2's own shape, the natural
    // FOL statement: ∀a..f. (Cong(a,b,c,d) ∧ Cong(c,d,e,f)) → Cong(a,b,e,f). The single
    // `→I` assumes the whole conjunction, so discharging the cut's open existential
    // middle requires FORWARD ∧-ELIMINATION: a conjunct of the assumed `A ∧ B` must be
    // usable as a standalone fact (and re-certified by projecting it back out). Without
    // it the search cannot pin the existential and explodes; with it this certifies in
    // milliseconds. Cites the universal symmetry lemma.
    let axioms = vec![tarski_a1(), tarski_a2()];
    let symmetry = LibraryTheorem {
        name: "symmetry".to_string(),
        premises: vec![],
        goal: forall(
            &["a", "b", "c", "d"],
            implies(
                cong(v("a"), v("b"), v("c"), v("d")),
                cong(v("c"), v("d"), v("a"), v("b")),
            ),
        ),
        cites: vec![],
    };
    let transitivity = LibraryTheorem {
        name: "transitivity_conj".to_string(),
        premises: vec![],
        goal: forall(
            &["a", "b", "c", "d", "e", "f"],
            implies(
                and(
                    cong(v("a"), v("b"), v("c"), v("d")),
                    cong(v("c"), v("d"), v("e"), v("f")),
                ),
                cong(v("a"), v("b"), v("e"), v("f")),
            ),
        ),
        cites: vec!["symmetry".to_string()],
    };
    let r = prove_library_with_axioms(&axioms, &[symmetry, transitivity]);
    assert!(r[0].verified, "symmetry: {:?}", r[0].verification_error);
    assert!(
        r[1].verified,
        "conjunctive transitivity (forward ∧-elim): {:?}",
        r[1].verification_error
    );
}
