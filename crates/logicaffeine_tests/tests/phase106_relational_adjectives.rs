//! Phase 106 — §9.1 Relational / pertainymic adjectives (work/MISSING_ENGLISH.md).
//!
//! A relational adjective ("dental" ← tooth, "coastal" ← coast) is denominal and
//! NON-predicating: "a dental procedure" is not {dental things} ∩ {procedures};
//! the adjective relates the procedure to teeth.
//!
//! Authoritative model (work/MISSING_ENGLISH.md §9.1, McNally & Boleda 2004 —
//! relational adjectives are predicates of KINDS):
//!   - kind-level (DEFAULT):  Noun(x) ∧ Rel(x, ^Base)     — no ∃y
//!   - instance-level (override): Noun(x) ∧ ∃y(Base(y) ∧ Rel(x, y))
//! `relation` defaults to `Pertains`; `level` defaults to `Kind`.
//!   - dental  → { base: Tooth }                         (kind, Pertains)
//!   - coastal → { base: Coast, relation: Near, level: Instance }
//!
//! This file also guards the pre-existing copular adjective-drop bug: copular
//! subjects ("A red car is fast.") must NOT lose their adjectives.

use logicaffeine_language::{compile, compile_kripke, compile_simple};

// ───────────────────────────────────────────────────────────────────────────
// Group A — kind-level relational (DEFAULT): Pertains(x, ^Base), no ∃y
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn dental_is_kind_relational_no_existential() {
    let out = compile("Every dental procedure is expensive.").unwrap();
    eprintln!("dental: {out}");
    assert!(out.contains("Procedure("), "must keep the head noun: {out}");
    assert!(out.contains("Pertains("), "relational ⇒ Pertains relation: {out}");
    assert!(
        out.contains("^Tooth") || out.contains("^tooth"),
        "kind-level base noun renders as ^Tooth: {out}"
    );
    assert!(
        !out.contains('∃'),
        "kind-level relational introduces NO existential: {out}"
    );
    assert!(
        !out.contains("Dental("),
        "relational adjective must NOT survive as a flat predicate Dental(x): {out}"
    );
}

#[test]
fn nuclear_is_kind_relational() {
    let out = compile("Every nuclear reactor is dangerous.").unwrap();
    eprintln!("nuclear: {out}");
    assert!(out.contains("Reactor("), "head noun kept: {out}");
    assert!(out.contains("Pertains("), "default relation Pertains: {out}");
    assert!(
        out.contains("^Nucleus") || out.contains("^nucleus"),
        "base noun ^Nucleus: {out}"
    );
    assert!(!out.contains('∃'), "kind-level ⇒ no ∃: {out}");
}

// ───────────────────────────────────────────────────────────────────────────
// Group B — instance-level relational (override): ∃y(Base(y) ∧ Near(x, y))
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn coastal_is_instance_relational_with_existential() {
    let out = compile("Every coastal region is wet.").unwrap();
    eprintln!("coastal: {out}");
    assert!(out.contains("Region("), "head noun kept: {out}");
    assert!(
        out.contains("Coast(") || out.contains("coast("),
        "base noun Coast appears: {out}"
    );
    assert!(out.contains("Near("), "coastal overrides relation to Near: {out}");
    assert!(
        out.contains('∃'),
        "instance-level relational introduces an existential ∃y: {out}"
    );
    assert!(
        !out.contains("Pertains("),
        "the per-adjective override replaces the default Pertains: {out}"
    );
    assert!(
        !out.contains("Coastal("),
        "relational adjective must NOT survive as a flat predicate Coastal(x): {out}"
    );
}

// ───────────────────────────────────────────────────────────────────────────
// Group C — regression guards: intersective + subsective UNCHANGED
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn intersective_red_unchanged_in_universal() {
    let out = compile("Every red car is fast.").unwrap();
    eprintln!("red(universal): {out}");
    assert!(
        out.contains("Red(x)") || out.contains("R(x)"),
        "intersective 'red' stays a 1-arg predicate: {out}"
    );
    assert!(!out.contains("Pertains("), "intersective is not relational: {out}");
    assert!(!out.contains('∃'), "no existential introduced: {out}");
}

#[test]
fn subsective_large_unchanged_in_universal() {
    let out = compile("Every large mouse is quiet.").unwrap();
    eprintln!("large(universal): {out}");
    assert!(
        out.contains("^Mouse") || out.contains("^mouse"),
        "subsective 'large' keeps its 2-arg ^Mouse comparison class: {out}"
    );
    assert!(!out.contains("Pertains("), "subsective is not relational: {out}");
}

// ───────────────────────────────────────────────────────────────────────────
// Group D — copular adjective-drop bug fix (prerequisite for the shared helper)
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn copular_subject_keeps_intersective_adjective() {
    let out = compile("A red car is fast.").unwrap();
    eprintln!("copular red: {out}");
    assert!(
        out.contains("Red(") || out.contains("R("),
        "copular subject must NOT drop the adjective 'red': {out}"
    );
    assert!(out.contains("Car(") || out.contains("C("), "head noun kept: {out}");
    assert!(out.contains("Fast(") || out.contains("F("), "predicate kept: {out}");
}

#[test]
fn copular_subject_keeps_relational_adjective() {
    let out = compile("A coastal region is wet.").unwrap();
    eprintln!("copular coastal: {out}");
    assert!(out.contains("Region("), "head noun kept: {out}");
    assert!(out.contains("Near("), "relational expansion survives in ∃ context: {out}");
    assert!(out.contains("Coast(") || out.contains("coast("), "base noun: {out}");
}

// ───────────────────────────────────────────────────────────────────────────
// Format coverage — the relational expansion renders in every printer
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn relational_renders_in_simple_and_kripke() {
    let simple = compile_simple("Every dental procedure is expensive.").unwrap();
    eprintln!("dental(simple): {simple}");
    assert!(simple.contains("Pertains"), "SimpleFOL keeps Pertains: {simple}");

    let kripke = compile_kripke("Every dental procedure is expensive.").unwrap();
    eprintln!("dental(kripke): {kripke}");
    assert!(kripke.contains("Pertains"), "Kripke keeps Pertains: {kripke}");
}
