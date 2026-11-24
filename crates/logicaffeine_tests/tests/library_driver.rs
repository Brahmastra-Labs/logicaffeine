//! The multi-theorem driver — the Euclid-graph engine. Theorems are discharged in
//! citation order, and each proved theorem becomes a citable lemma for later theorems:
//! what it states, its premises implying its goal. A lemma proved from the shared axioms
//! alone states its goal outright; one proved under premises of its own hands a citer
//! only the implication, so a citation never supplies a fact the citer cannot derive.

use logicaffeine_proof::verify::{prove_library, prove_library_with_axioms, LibraryTheorem};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn p(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(body),
    }
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}

/// `∀x. man(x) → mortal(x)`, `man(Socrates)` ⊢ `mortal(Socrates)`.
fn mortal_socrates() -> LibraryTheorem {
    LibraryTheorem {
        name: "mortal_socrates".to_string(),
        premises: vec![
            forall(
                "x",
                implies(p("man", vec![v("x")]), p("mortal", vec![v("x")])),
            ),
            p("man", vec![k("Socrates")]),
        ],
        goal: p("mortal", vec![k("Socrates")]),
        cites: vec![],
    }
}

/// `∀x. mortal(x) → dies(x)` ⊢ `dies(Socrates)` — needs `mortal(Socrates)`, supplied
/// by citing `mortal_socrates`.
fn socrates_dies(cites: &[&str]) -> LibraryTheorem {
    LibraryTheorem {
        name: "socrates_dies".to_string(),
        premises: vec![forall(
            "x",
            implies(p("mortal", vec![v("x")]), p("dies", vec![v("x")])),
        )],
        goal: p("dies", vec![k("Socrates")]),
        cites: cites.iter().map(|s| s.to_string()).collect(),
    }
}

/// `mortal_socrates`'s premises as the library's shared axioms, and the lemma itself with none of its own: proved
/// from the axioms alone, it states `mortal(Socrates)` outright.
fn mortal_socrates_axioms() -> Vec<ProofExpr> {
    mortal_socrates().premises
}

fn mortal_socrates_from_the_axioms() -> LibraryTheorem {
    LibraryTheorem { premises: vec![], ..mortal_socrates() }
}

#[test]
fn library_cites_earlier_theorem() {
    let r = prove_library_with_axioms(
        &mortal_socrates_axioms(),
        &[mortal_socrates_from_the_axioms(), socrates_dies(&["mortal_socrates"])],
    );
    assert!(
        r[0].verified,
        "mortal_socrates: {:?}",
        r[0].verification_error
    );
    assert!(
        r[1].verified,
        "socrates_dies (cites mortal_socrates): {:?}",
        r[1].verification_error
    );
}

#[test]
fn library_without_citation_fails() {
    // No citation → `mortal(Socrates)` is unavailable → `dies(Socrates)` is unprovable.
    let r = prove_library(&[socrates_dies(&[])]);
    assert!(
        !r[0].verified,
        "without citing the lemma, the dependent theorem must NOT prove"
    );
}

#[test]
fn library_topo_orders_out_of_sequence() {
    // The citer is listed FIRST; the driver must still prove `mortal_socrates` first.
    let r = prove_library_with_axioms(
        &mortal_socrates_axioms(),
        &[socrates_dies(&["mortal_socrates"]), mortal_socrates_from_the_axioms()],
    );
    // Results come back in INPUT order: [socrates_dies, mortal_socrates].
    assert!(
        r[0].verified,
        "citer listed first: {:?}",
        r[0].verification_error
    );
    assert!(r[1].verified, "cited: {:?}", r[1].verification_error);
}

#[test]
fn library_citation_of_a_failed_theorem_does_not_help() {
    // `bad` does not follow from its premise, so it fails; a theorem citing it
    // therefore cannot use its conclusion and fails too (no false lemma leaks in).
    let bad = LibraryTheorem {
        name: "bad".to_string(),
        premises: vec![p("man", vec![k("Socrates")])],
        goal: p("mortal", vec![k("Socrates")]), // does NOT follow from man(Socrates) alone
        cites: vec![],
    };
    let r = prove_library(&[bad, socrates_dies(&["bad"])]);
    assert!(!r[0].verified, "the unsound theorem must fail");
    assert!(
        !r[1].verified,
        "citing a FAILED theorem must not supply its (unproved) conclusion"
    );
}

/// `mortal_socrates` proves `mortal(Socrates)` from its own two premises, so what it states, and what a citation of
/// it hands over, is those premises implying `mortal(Socrates)`. `socrates_dies` holds neither premise, so citing the
/// lemma cannot give it `mortal(Socrates)` outright: a library that did would prove `dies(Socrates)` from nothing.
#[test]
fn a_cited_theorem_is_its_premises_implying_its_goal() {
    let r = prove_library(&[mortal_socrates(), socrates_dies(&["mortal_socrates"])]);
    assert!(r[0].verified, "mortal_socrates: {:?}", r[0].verification_error);
    assert!(
        !r[1].verified,
        "a lemma proved under premises the citer lacks must not hand over its bare conclusion"
    );
}

/// The smallest form of the hole: a theorem that assumes `P(A)` proves `P(A)`, and a theorem that cites it, with no
/// premise of its own, must not.
#[test]
fn citing_an_assumption_proves_nothing() {
    let p_a = p("P", vec![k("A")]);
    let assumes = LibraryTheorem { name: "assumes".to_string(), premises: vec![p_a.clone()], goal: p_a.clone(), cites: vec![] };
    let cites = LibraryTheorem { name: "cites".to_string(), premises: vec![], goal: p_a, cites: vec!["assumes".to_string()] };
    let r = prove_library(&[assumes, cites]);
    assert!(r[0].verified, "{:?}", r[0].verification_error);
    assert!(!r[1].verified, "P(A) follows from nothing the citer holds");
}

/// A citer that holds the cited theorem's premises proves through the citation, the implication's antecedent met.
#[test]
fn a_citer_holding_the_cited_premises_proves_through_the_citation() {
    let mut dies = socrates_dies(&["mortal_socrates"]);
    dies.premises.extend(mortal_socrates().premises);
    let r = prove_library(&[mortal_socrates(), dies]);
    assert!(r[1].verified, "{:?}", r[1].verification_error);
}
