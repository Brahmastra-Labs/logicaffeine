//! The multi-theorem driver — the Euclid-graph engine. Theorems are discharged in
//! citation order, and each proved conclusion becomes a citable lemma for later
//! theorems. Here `socrates_dies` can only be proved because `mortal_socrates`
//! proves `mortal(Socrates)` first.

use logicaffeine_proof::verify::{prove_library, LibraryTheorem};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn p(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}

/// `∀x. man(x) → mortal(x)`, `man(Socrates)` ⊢ `mortal(Socrates)`.
fn mortal_socrates() -> LibraryTheorem {
    LibraryTheorem {
        name: "mortal_socrates".to_string(),
        premises: vec![
            forall("x", implies(p("man", vec![v("x")]), p("mortal", vec![v("x")]))),
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

#[test]
fn library_cites_earlier_theorem() {
    let r = prove_library(&[mortal_socrates(), socrates_dies(&["mortal_socrates"])]);
    assert!(r[0].verified, "mortal_socrates: {:?}", r[0].verification_error);
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
    let r = prove_library(&[socrates_dies(&["mortal_socrates"]), mortal_socrates()]);
    // Results come back in INPUT order: [socrates_dies, mortal_socrates].
    assert!(r[0].verified, "citer listed first: {:?}", r[0].verification_error);
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
