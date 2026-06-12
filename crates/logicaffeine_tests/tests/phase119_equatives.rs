//! Phase 119 — §7.1 Equatives (as…as) (MISSING_ENGLISH.md).
//!
//! An equative is an at-least degree comparison (≥), distinct from the strict
//! comparative (>):
//!   "John is as tall as Mary."  → max{d:Tall(john,d)} ≥ max{d:Tall(mary,d)}
//!   "John is taller than Mary." → max{d:Tall(john,d)} > max{d:Tall(mary,d)}
//! The `Comparative` node gains a relation (GT|GE|EQ); `write_comparative` emits ≥
//! for the equative `as ADJ as` frame.

use logicaffeine_language::compile;

#[test]
fn equative_uses_at_least_relation() {
    let out = compile("John is as tall as Mary.").unwrap();
    eprintln!("equative: {out}");
    // Subject/object render via the symbol registry (John→J, Mary→M).
    assert!(out.contains('J') && out.contains('M'), "both arguments present: {out}");
    assert!(
        out.contains('≥') || out.contains(">="),
        "equative is an at-least (≥) comparison, not strict >: {out}"
    );
    assert!(out.contains("Tall") || out.contains("tall"), "the gradable dimension: {out}");
    assert!(out.contains("max"), "max-degree comparison form: {out}");
}

#[test]
fn equative_distinct_from_strict_comparative() {
    let equative = compile("John is as tall as Mary.").unwrap();
    let comparative = compile("John is taller than Mary.").unwrap();
    eprintln!("eq: {equative}\ncomp: {comparative}");
    // The equative must NOT render the same strict '>' the comparative does.
    assert!(equative.contains('≥') || equative.contains(">="), "equative ≥: {equative}");
    assert!(
        !equative.contains('≥') || equative != comparative,
        "equative and comparative are distinct: {equative} vs {comparative}"
    );
}
