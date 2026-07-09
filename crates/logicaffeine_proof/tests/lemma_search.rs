//! `exact?` / `apply?` — library search over the named lemma base, plus
//! premise selection. The discrimination tree indexes lemma conclusions;
//! one-sided matching instantiates the lemma at the goal and reports the
//! citation (and, for `apply?`, the antecedents you would still owe).

use logicaffeine_proof::lemma_index::LemmaIndex;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn pr(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn cong(a: &str, b: &str, c: &str, d: &str) -> ProofExpr {
    pr("Cong", vec![k(a), k(b), k(c), k(d)])
}
fn cong_v(a: &str, b: &str, c: &str, d: &str) -> ProofExpr {
    pr("Cong", vec![v(a), v(b), v(c), v(d)])
}

#[test]
fn exact_search_finds_instantiated_lemma() {
    // Axiom flip: ∀ a b. Cong(a, b, b, a).  Goal Cong(P, Q, Q, P) is an instance.
    let flip = forall("a", forall("b", cong_v("a", "b", "b", "a")));
    let idx = LemmaIndex::build(&[("flip".to_string(), flip)]);
    let hits = idx.find_exact(&cong("P", "Q", "Q", "P"));
    assert_eq!(hits.len(), 1, "flip should be the unique exact match");
    assert_eq!(hits[0].lemma, "flip");
    assert_eq!(hits[0].tactic_text, "exact flip");
    assert_eq!(hits[0].subst.get("a"), Some(&k("P")));
    assert_eq!(hits[0].subst.get("b"), Some(&k("Q")));
}

#[test]
fn exact_search_ignores_non_matching_lemmas() {
    let flip = forall("a", forall("b", cong_v("a", "b", "b", "a")));
    let refl = forall("a", forall("b", cong_v("a", "b", "a", "b")));
    let idx = LemmaIndex::build(&[("flip".to_string(), flip), ("refl".to_string(), refl)]);
    // Cong(P,Q,Q,P) matches flip only (refl would need c=a, d=b, but here c=Q≠P).
    let hits = idx.find_exact(&cong("P", "Q", "Q", "P"));
    let names: Vec<&str> = hits.iter().map(|h| h.lemma.as_str()).collect();
    assert_eq!(names, vec!["flip"]);
}

#[test]
fn apply_search_matches_consequent_and_lists_antecedents() {
    // inner_trans: ∀ a b c d e f. Cong(a,b,c,d) → Cong(a,b,e,f) → Cong(c,d,e,f).
    let inner_trans = forall(
        "a",
        forall(
            "b",
            forall(
                "c",
                forall(
                    "d",
                    forall(
                        "e",
                        forall(
                            "f",
                            implies(
                                cong_v("a", "b", "c", "d"),
                                implies(cong_v("a", "b", "e", "f"), cong_v("c", "d", "e", "f")),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    );
    let idx = LemmaIndex::build(&[("inner_trans".to_string(), inner_trans)]);
    // Goal Cong(C,D,E,F): matches the consequent with c=C,d=D,e=E,f=F. a,b stay free.
    let hits = idx.find_apply(&cong("C", "D", "E", "F"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].tactic_text, "apply inner_trans");
    // Two antecedents owed, with the determined bindings substituted.
    assert_eq!(hits[0].remaining.len(), 2, "two antecedents owed");
}

#[test]
fn search_ranks_specific_before_general() {
    // A ground lemma P(A) outranks the universal ∀x. P(x) for goal P(A):
    // the ground match binds nothing, the universal binds x — fewer bindings first.
    let specific = pr("P", vec![k("A")]);
    let general = forall("x", pr("P", vec![v("x")]));
    let idx = LemmaIndex::build(&[
        ("general".to_string(), general),
        ("specific".to_string(), specific),
    ]);
    let hits = idx.find_exact(&pr("P", vec![k("A")]));
    assert_eq!(hits.len(), 2, "both match");
    assert_eq!(hits[0].lemma, "specific", "the ground lemma ranks first");
}

#[test]
fn search_finds_nothing_cleanly() {
    let flip = forall("a", forall("b", cong_v("a", "b", "b", "a")));
    let idx = LemmaIndex::build(&[("flip".to_string(), flip)]);
    assert!(idx.find_exact(&pr("Unrelated", vec![k("X")])).is_empty());
}

#[test]
fn premise_selection_ranks_relevant_above_distractors() {
    // 150 distractor axioms over unrelated predicates + 3 relevant Cong lemmas.
    // select_premises must surface the relevant ones near the top.
    let mut lemmas: Vec<(String, ProofExpr)> = Vec::new();
    for i in 0..150 {
        lemmas.push((
            format!("distractor{i}"),
            forall("x", pr(&format!("D{i}"), vec![v("x")])),
        ));
    }
    lemmas.push(("flip".to_string(), forall("a", forall("b", cong_v("a", "b", "b", "a")))));
    lemmas.push(("cong_refl".to_string(), forall("a", forall("b", cong_v("a", "b", "a", "b")))));
    lemmas.push((
        "cong_id".to_string(),
        forall("a", pr("Cong", vec![v("a"), v("a"), v("a"), v("a")])),
    ));

    let idx = LemmaIndex::build(&lemmas);
    let selected = idx.select_premises(&cong("P", "Q", "Q", "P"), 10);
    assert!(selected.len() <= 10, "returns at most k");
    assert!(selected.contains(&"flip".to_string()), "the exact-matching lemma is selected");
    // No distractor (unrelated head symbol) should crowd out the relevant Cong lemmas.
    let distractors = selected.iter().filter(|n| n.starts_with("distractor")).count();
    assert!(distractors == 0, "unrelated distractors must not be selected: {selected:?}");
}
