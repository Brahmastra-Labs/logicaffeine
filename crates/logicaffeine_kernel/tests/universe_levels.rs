//! R3 — the universe-level algebra, locked in by TDD.
//!
//! Universe polymorphism is only sound if the level decision procedure is. These tests
//! pin the algebra: definitional equality up to `max`/`succ`/commutativity, cumulative
//! `≤` that is correct for EVERY instantiation of the variables (so `Type 1 ≤ u` and
//! `u ≤ v` are rejected — they fail for some assignment), and variable substitution
//! (the mechanism that instantiates a polymorphic definition at a concrete level).

use std::collections::HashMap;

use logicaffeine_kernel::Universe;

fn u(name: &str) -> Universe {
    Universe::Var(name.to_string())
}
fn t(n: u32) -> Universe {
    Universe::Type(n)
}
fn prop() -> Universe {
    Universe::Prop
}
fn succ(l: Universe) -> Universe {
    l.succ()
}
fn max(a: Universe, b: Universe) -> Universe {
    a.max(&b)
}
fn subst(pairs: &[(&str, Universe)]) -> HashMap<String, Universe> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

// ===========================================================================
// Concrete levels — the existing behavior must be preserved exactly.
// ===========================================================================

#[test]
fn concrete_succ_and_max_unchanged() {
    assert!(succ(t(0)).equiv(&t(1)), "succ(Type 0) = Type 1");
    assert!(succ(prop()).equiv(&t(1)), "succ(Prop) = Type 1 (the kernel's convention)");
    assert!(max(t(1), t(2)).equiv(&t(2)));
    assert!(max(prop(), t(0)).equiv(&t(0)), "Prop is dominated by Type 0");
}

#[test]
fn concrete_cumulativity_unchanged() {
    assert!(prop().is_subtype_of(&t(0)), "Prop ≤ Type 0");
    assert!(prop().is_subtype_of(&prop()), "Prop ≤ Prop");
    assert!(t(0).is_subtype_of(&t(1)), "Type 0 ≤ Type 1");
    assert!(!t(1).is_subtype_of(&t(0)), "Type 1 ≰ Type 0");
    assert!(!t(0).is_subtype_of(&prop()), "Type 0 ≰ Prop");
}

// ===========================================================================
// Definitional equality of level EXPRESSIONS (not derived structural equality).
// ===========================================================================

#[test]
fn variable_equality_is_by_name() {
    assert!(u("a").equiv(&u("a")), "u ≡ u");
    assert!(!u("a").equiv(&u("b")), "u ≢ v");
    assert!(!u("a").equiv(&t(0)), "u ≢ Type 0 (a variable is not a concrete level)");
}

#[test]
fn the_algebra_collapses_in_normalization() {
    // max(u, u) ≡ u
    assert!(max(u("a"), u("a")).equiv(&u("a")), "max(u,u) ≡ u");
    // max is commutative
    assert!(max(u("a"), u("b")).equiv(&max(u("b"), u("a"))), "max(u,v) ≡ max(v,u)");
    // max(succ u, u) ≡ succ u  (the offset form: u+1 dominates u+0)
    assert!(max(succ(u("a")), u("a")).equiv(&succ(u("a"))), "max(u+1, u) ≡ u+1");
    // succ(succ(Type 0)) ≡ Type 2
    assert!(succ(succ(t(0))).equiv(&t(2)));
    // max(u, Type 0) ≢ u — a universe variable ranges over ALL levels including
    // Prop (the Lean model), so at u := Prop the two differ: max(Prop, Type 0) =
    // Type 0 ≠ Prop. The old "variable floors at Type 0" reading was the source
    // of the `Sort u := Nat` ⇒ `Nat : Prop` unsoundness that `imax` now closes.
    assert!(!max(u("a"), t(0)).equiv(&u("a")), "max(u, Type 0) ≢ u (differ at u = Prop)");
    // associativity-ish: max(max(u,v),w) ≡ max(u,max(v,w))
    assert!(
        max(max(u("a"), u("b")), u("c")).equiv(&max(u("a"), max(u("b"), u("c")))),
        "max is associative"
    );
}

// ===========================================================================
// Cumulative ≤ over variables — SOUND for every instantiation.
// ===========================================================================

#[test]
fn variable_cumulativity_holds_only_when_universally_true() {
    // Reflexive; Prop (the bottom) is below every variable; a variable is below
    // its own successor.
    assert!(u("a").is_subtype_of(&u("a")), "u ≤ u");
    assert!(prop().is_subtype_of(&u("a")), "Prop ≤ u (Prop is the bottom level)");
    assert!(u("a").is_subtype_of(&succ(u("a"))), "u ≤ u+1");

    // These would FAIL for some instantiation, so must be rejected. A universe
    // variable ranges over ALL levels including Prop, so `Type 0 ≤ u` fails at
    // u := Prop — the soundness-critical case that keeps `Sort u := Nat` from
    // being instantiated at Prop.
    assert!(!t(0).is_subtype_of(&u("a")), "Type 0 ≰ u (fails at u := Prop)");
    assert!(!u("a").is_subtype_of(&u("b")), "u ≰ v (distinct variables)");
    assert!(!t(1).is_subtype_of(&u("a")), "Type 1 ≰ u (fails at u := Prop)");
    assert!(!succ(u("a")).is_subtype_of(&u("a")), "u+1 ≰ u");
    assert!(!u("a").is_subtype_of(&prop()), "u ≰ Prop (u could be Type 0)");
}

#[test]
fn max_is_the_least_upper_bound() {
    // Each branch is ≤ the max, and the max of equals collapses.
    assert!(u("a").is_subtype_of(&max(u("a"), u("b"))), "u ≤ max(u,v)");
    assert!(u("b").is_subtype_of(&max(u("a"), u("b"))), "v ≤ max(u,v)");
    // max(u,v) ≤ max(v,u) both ways (commutativity under ≤).
    assert!(max(u("a"), u("b")).is_subtype_of(&max(u("b"), u("a"))));
    // max(u,v) ≰ u (it can exceed u when v is large).
    assert!(!max(u("a"), u("b")).is_subtype_of(&u("a")), "max(u,v) ≰ u");
}

// ===========================================================================
// Substitution — instantiating a universe-polymorphic level at a concrete one.
// ===========================================================================

#[test]
fn substitution_instantiates_variables() {
    assert!(u("a").substitute(&subst(&[("a", t(0))])).equiv(&t(0)), "u[a:=Type0] = Type 0");
    // succ(u)[u := Type 1] ≡ Type 2
    assert!(
        succ(u("a")).substitute(&subst(&[("a", t(1))])).equiv(&t(2)),
        "(u+1)[u:=Type1] = Type 2"
    );
    // max(u, Type 5)[u := Type 0] ≡ Type 5
    assert!(
        max(u("a"), t(5)).substitute(&subst(&[("a", t(0))])).equiv(&t(5)),
        "max(u,Type5)[u:=Type0] = Type 5"
    );
    // A variable not in the substitution is left alone.
    assert!(
        u("a").substitute(&subst(&[("b", t(0))])).equiv(&u("a")),
        "unrelated substitution is identity"
    );
    // Substituting one variable for another is honored by the algebra.
    assert!(
        max(u("a"), u("b")).substitute(&subst(&[("a", u("b"))])).equiv(&u("b")),
        "max(u,v)[u:=v] ≡ v"
    );
}
