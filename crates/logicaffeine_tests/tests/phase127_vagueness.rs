//! Phase 127 — §8.5 Vagueness (MISSING_ENGLISH.md).
//!
//! Vague predicates (bald, tall, heap) have borderline cases and sorites behavior.
//! Beyond the degree-standard form (§7.2), a vague predicate carries a PENUMBRA —
//! a borderline region around the threshold:
//!   "John is bald." → ∃d(Bald(john,d) ∧ d > θ) ∧ Borderline(θ)
//! A merely-gradable-but-sharp predicate has a threshold but no penumbra marker.

use logicaffeine_language::compile_pragmatic as compile;

#[test]
fn vague_predicate_has_penumbra_marker() {
    let out = compile("John is bald.").unwrap();
    eprintln!("bald: {out}");
    assert!(out.contains("Bald"), "the vague predicate: {out}");
    assert!(out.contains('θ') || out.contains('Θ'), "a context threshold: {out}");
    // The penumbra / borderline region marks vagueness (sorites-susceptible).
    assert!(
        out.contains("Borderline") || out.contains("Penumbra"),
        "vague predicate carries a penumbra/borderline marker: {out}"
    );
}

#[test]
fn tall_is_vague() {
    let out = compile("John is tall.").unwrap();
    eprintln!("tall: {out}");
    assert!(out.contains("Tall"), "the predicate: {out}");
    assert!(out.contains("Borderline") || out.contains("Penumbra"), "tall is vague: {out}");
}

#[test]
fn non_vague_predicate_has_no_penumbra() {
    // A non-gradable predicate is crisp — no threshold and no penumbra.
    let out = compile("John is Canadian.").unwrap();
    eprintln!("canadian: {out}");
    assert!(!out.contains("Borderline") && !out.contains("Penumbra"), "crisp predicate has no penumbra: {out}");
}
