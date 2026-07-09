#![cfg(feature = "verification")]
//! ============================================================================
//! PHASE 141: P5 LINK-LATTICE MEREOLOGY — THE PLURAL/MASS SPEC
//! ============================================================================
//!
//! Plurals, groups, and mass terms share ONE join-semilattice: a primitive
//! sum `sum(a,b)` (⊕) with `Part(x,y) ↔ sum(x,y) = y`, membership as atomic
//! parthood, CUM/DIV closure for mass-tagged predicates. `Distributive` and
//! `GroupQuantifier` must lower to real first-order forms (no more
//! `unverifiable()`), so:
//!
//!   - distributive predication ⊢ each member instance
//!   - collective predication ⊬ member instances (the load-bearing split)
//!   - mass predicates are cumulative: Water(a) ∧ Water(b) ⊢ Water(a⊕b)
//!
//! Raw lattice claims go through `oracle_entails`; English claims through the
//! theorem doors (`check_theorem_smt` for Z3, `verify_theorem` for the plain
//! FOL the kernel can certify).

use logicaffeine_compile::{
    check_theorem_premises_consistent, check_theorem_smt, verify_theorem,
};
use logicaffeine_proof::oracle::{
    oracle_consistent, oracle_entails, SmtConsistency, SmtVerdict,
};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn c(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.to_string())
}

fn sum(a: ProofTerm, b: ProofTerm) -> ProofTerm {
    ProofTerm::Function("sum".to_string(), vec![a, b])
}

fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}

fn ident(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}

fn theorem(premises: &[&str], goal: &str) -> String {
    let givens: String = premises
        .iter()
        .map(|p| format!("Given: {p}\n"))
        .collect();
    format!("## Theorem: Phase141\n{givens}Prove: {goal}\nProof: Auto.\n")
}

// ============================================================================
// A. The lattice itself (axioms fire when `sum` appears)
// ============================================================================

#[test]
fn sum_is_idempotent() {
    assert_eq!(
        oracle_entails(&[], &ident(sum(c("A"), c("A")), c("A"))),
        SmtVerdict::Entailed,
        "⊕ idempotence: a⊕a = a"
    );
}

#[test]
fn sum_is_commutative() {
    assert_eq!(
        oracle_entails(&[], &ident(sum(c("A"), c("B")), sum(c("B"), c("A")))),
        SmtVerdict::Entailed,
        "⊕ commutativity: a⊕b = b⊕a"
    );
}

#[test]
fn sum_is_associative() {
    assert_eq!(
        oracle_entails(
            &[],
            &ident(
                sum(sum(c("A"), c("B")), c("C")),
                sum(c("A"), sum(c("B"), c("C")))
            )
        ),
        SmtVerdict::Entailed,
        "⊕ associativity"
    );
}

#[test]
fn parts_are_below_the_sum() {
    assert_eq!(
        oracle_entails(&[], &pred("Part", vec![c("A"), sum(c("A"), c("B"))])),
        SmtVerdict::Entailed,
        "a is part of a⊕b"
    );
}

#[test]
fn lattice_axioms_do_not_make_distinct_atoms_collapse() {
    // Soundness guard: the axiom pack must not force a = b.
    let premises = vec![pred("Atom", vec![c("A")]), pred("Atom", vec![c("B")])];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "two atoms are consistent"
    );
    assert_eq!(
        oracle_entails(&premises, &ident(c("A"), c("B"))),
        SmtVerdict::NotEntailed,
        "the lattice must not collapse distinct constants"
    );
}

// ============================================================================
// B. Distributive vs collective — the load-bearing split (§5.2, §5.4)
// ============================================================================

#[test]
fn distributive_entails_member_instance() {
    // "all" forces distribution over atomic members; the instance follows in
    // plain FOL, so the KERNEL door must certify it.
    let src = theorem(
        &["The boys all left.", "Tom is one of the boys."],
        "Tom left.",
    );
    assert!(
        verify_theorem(&src).is_ok(),
        "distributive plural ⊢ member instance (kernel-certifiable)"
    );
}

#[test]
fn each_distributes_too() {
    let src = theorem(
        &["The boys each lifted the piano.", "Tom is one of the boys."],
        "Tom lifted the piano.",
    );
    assert_eq!(
        check_theorem_smt(&src).expect("must parse"),
        SmtVerdict::Entailed,
        "floated 'each' ⊢ member instance"
    );
}

#[test]
fn collective_does_not_distribute() {
    let src = theorem(
        &["The boys lifted the piano.", "Tom is one of the boys."],
        "Tom lifted the piano.",
    );
    assert_eq!(
        check_theorem_premises_consistent(&src).expect("must parse"),
        SmtConsistency::Consistent,
        "collective premise set consistent"
    );
    assert_eq!(
        check_theorem_smt(&src).expect("must parse"),
        SmtVerdict::NotEntailed,
        "collective lifting must NOT entail that Tom lifted the piano alone"
    );
}

// ============================================================================
// C. Mass cumulativity and portions (§6.2)
// ============================================================================

#[test]
fn mass_predicates_are_cumulative() {
    // Water is mass ⇒ CUM(Water): the sum of two water portions is water.
    // (Subject coordination in English distributes — "Alpha and Beta are
    // water" predicates each conjunct — so the ⊕ claim itself is stated
    // directly, with the mass tag supplied as the lexicon would.)
    use logicaffeine_proof::oracle::{oracle_entails_with_theory, SmtTheory};
    let theory = SmtTheory {
        cumulative_predicates: vec!["water".to_string()],
    };
    let premises = vec![
        pred("water", vec![c("A")]),
        pred("water", vec![c("B")]),
    ];
    assert_eq!(
        oracle_entails_with_theory(&premises, &pred("water", vec![sum(c("A"), c("B"))]), &theory),
        SmtVerdict::Entailed,
        "CUM: Water(a) ∧ Water(b) ⊢ Water(a⊕b)"
    );
}

#[test]
fn count_predicates_are_not_cumulative() {
    // Boy is count (not in the cumulative theory): two boys do not sum to a
    // boy. No CUM leak onto untagged predicates.
    let premises = vec![pred("boy", vec![c("A")]), pred("boy", vec![c("B")])];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "two boys are a consistent premise set"
    );
    assert_eq!(
        oracle_entails(&premises, &pred("boy", vec![sum(c("A"), c("B"))])),
        SmtVerdict::NotEntailed,
        "CUM must be scoped to mass-tagged predicates only"
    );
}

#[test]
fn drinking_a_portion_is_drinking_water() {
    // Portion coercion (§6.2): ∃x(Portion(x) ∧ Water(x) ∧ Drink(j,x)) must
    // entail that John drank water.
    let src = theorem(&["John drank a water."], "John drank water.");
    assert_eq!(
        check_theorem_smt(&src).expect("portion coercion must parse"),
        SmtVerdict::Entailed,
        "a portion of water is water (DIV/portion axiom)"
    );
}
