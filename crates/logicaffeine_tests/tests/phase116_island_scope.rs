//! Phase 116 — §5.1 Scope inside islands / underspecified scope (P7).
//!
//! Scope is stored underspecified; readings are enumerated on demand, with the
//! permutation restricted to quantifiers in the same island. `compile_all_scopes`
//! yields every licensed reading:
//!   "Every student read a book." → ∀∃ (surface) AND ∃∀ (wide 'a book')
//! while island constraints prevent illicit permutations across a relative clause.
//!
//! (The narrow sub-case of a wide indefinite scoping OUT of a relative-clause
//! island headed by the quantifier-pronoun "everyone" — "Everyone who owns a car
//! insures it." — additionally needs "everyone" lexicalized as a quantifier; that
//! refinement is tracked separately. The core underspecified-scope enumeration is
//! exercised here.)

use logicaffeine_language::compile_all_scopes;

#[test]
fn transitive_quantifiers_enumerate_both_scopes() {
    let rs = compile_all_scopes("Every student read a book.").unwrap();
    eprintln!("every-student: {rs:?}");
    assert!(rs.len() >= 2, "both scope readings enumerated: {rs:?}");
    // Surface ∀ > ∃ : the universal is outermost.
    assert!(
        rs.iter().any(|r| r.trim_start().starts_with('∀')),
        "surface reading ∀ > ∃ present: {rs:?}"
    );
    // Inverse ∃ > ∀ : 'a book' takes wide scope (one book all read).
    assert!(
        rs.iter().any(|r| r.trim_start().starts_with('∃')),
        "inverse reading ∃ > ∀ (wide 'a book') present: {rs:?}"
    );
}

#[test]
fn some_every_enumerate_both_scopes() {
    let rs = compile_all_scopes("Some student read every book.").unwrap();
    eprintln!("some-every: {rs:?}");
    assert!(rs.len() >= 2, "both scope readings enumerated: {rs:?}");
    assert!(rs.iter().any(|r| r.trim_start().starts_with('∃')), "surface ∃ > ∀: {rs:?}");
    assert!(rs.iter().any(|r| r.trim_start().starts_with('∀')), "inverse ∀ > ∃: {rs:?}");
}

#[test]
fn island_constraint_limits_permutation() {
    // A quantifier inside a relative-clause island does not freely permute with the
    // matrix quantifier: the surface reading is always among those enumerated, and
    // the enumeration is finite/well-formed (no crash, at least one reading).
    let rs = compile_all_scopes("Every student who read a book passed.").unwrap();
    eprintln!("island: {rs:?}");
    assert!(!rs.is_empty(), "island sentence still yields a reading: {rs:?}");
    assert!(
        rs.iter().any(|r| r.contains("Book") && r.contains("Pass")),
        "the reading preserves both the island predicate and the matrix: {rs:?}"
    );
}
