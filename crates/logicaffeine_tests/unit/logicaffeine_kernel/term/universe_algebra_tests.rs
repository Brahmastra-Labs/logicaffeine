//! Lib-unit coverage for the universe algebra — `succ`/`max`/`imax`/`is_subtype_of`/`equiv`.
//! Universe consistency is what keeps the kernel from Girard's paradox, so these lock the
//! exact level arithmetic (impredicative `imax` collapse to Prop, cumulative `≤` boundary).
use super::*;

#[test]
fn succ_steps_every_level() {
    assert_eq!(Universe::Type(0).succ(), Universe::Type(1));
    assert_eq!(Universe::Type(5).succ(), Universe::Type(6));
    // `Prop`/`SProp` sit at level 0 in `to_lnat`'s model, so their successor is level 1,
    // which is spelled `Type 0`. Asserting `Type 1` here pinned the off-by-one that made
    // `succ` disagree with `level_leq` about `Prop` alone.
    assert_eq!(Universe::Prop.succ(), Universe::Type(0));
    assert_eq!(Universe::SProp.succ(), Universe::Type(0));
}

#[test]
fn max_takes_the_larger_and_absorbs_prop() {
    assert_eq!(Universe::Type(2).max(&Universe::Type(5)), Universe::Type(5));
    assert_eq!(Universe::Type(5).max(&Universe::Type(2)), Universe::Type(5));
    assert_eq!(Universe::Prop.max(&Universe::Type(3)), Universe::Type(3));
    assert_eq!(Universe::Type(3).max(&Universe::Prop), Universe::Type(3));
}

#[test]
fn imax_is_impredicative_in_its_codomain() {
    // A Π into (S)Prop is (S)Prop regardless of domain — impredicativity.
    assert_eq!(Universe::Type(7).imax(&Universe::Prop), Universe::Prop);
    assert_eq!(Universe::Type(7).imax(&Universe::SProp), Universe::SProp);
    // A Π into Type n is predicative: max(domain, codomain).
    assert_eq!(
        Universe::Type(2).imax(&Universe::Type(5)),
        Universe::Type(5)
    );
}

#[test]
fn cumulative_subtyping_respects_level_order() {
    assert!(
        Universe::Type(0).is_subtype_of(&Universe::Type(1)),
        "Type0 ≤ Type1"
    );
    assert!(
        Universe::Type(3).is_subtype_of(&Universe::Type(3)),
        "reflexive"
    );
    assert!(
        !Universe::Type(1).is_subtype_of(&Universe::Type(0)),
        "Type1 ⋠ Type0"
    );
    assert!(!Universe::Type(5).is_subtype_of(&Universe::Type(2)));
}

#[test]
fn equiv_is_exact_level_equality() {
    assert!(Universe::Type(2).equiv(&Universe::Type(2)));
    assert!(!Universe::Type(1).equiv(&Universe::Type(2)));
    assert!(Universe::Prop.equiv(&Universe::Prop));
    assert!(!Universe::Prop.equiv(&Universe::Type(0)), "Prop ≠ Type0");
}

#[test]
fn int_lit_and_lit_bigint_round_trip() {
    use logicaffeine_base::BigInt;
    let small = int_lit(BigInt::from_u64(42));
    assert_eq!(lit_bigint(&small), Some(BigInt::from_u64(42)));
    // u64::MAX exceeds i64::MAX, so it must round-trip via the BigInt literal form.
    let big = BigInt::from_u64(u64::MAX);
    assert_eq!(lit_bigint(&int_lit(big.clone())), Some(big));
}

#[test]
fn variable_levels_are_decided_soundly_over_all_instantiations() {
    // The symbolic level engine (to_lnat → simp_lnat → lnat_leq: imax_pivot / subst_lnat /
    // leq_linear / lnat_atoms) decides `≤` SOUNDLY over every instantiation of the universe
    // variables (each ranges over ℕ, Prop = 0). A variable universe is what closes the
    // `Sort u := Nat` unsoundness, so these are consistency-critical.
    let u = Universe::Var("u".to_string());
    let vv = Universe::Var("v".to_string());
    let succ_u = Universe::Succ(Box::new(u.clone()));
    let max_uv = Universe::Max(Box::new(u.clone()), Box::new(vv.clone()));
    let imax_uv = Universe::IMax(Box::new(u.clone()), Box::new(vv.clone()));

    // leq_linear atom domination (var+offset, and the constant floor).
    assert!(u.is_subtype_of(&u), "u ≤ u");
    assert!(u.is_subtype_of(&succ_u), "u ≤ succ u");
    assert!(!succ_u.is_subtype_of(&u), "succ u ⋠ u");
    assert!(Universe::Prop.is_subtype_of(&u), "Prop (0) ≤ u");
    assert!(
        !Universe::Type(0).is_subtype_of(&u),
        "Type 0 ⋠ u — u may instantiate to Prop"
    );
    assert!(
        !u.is_subtype_of(&vv),
        "u ⋠ v — distinct variables are incomparable"
    );

    // max: an atom is dominated iff it appears in the max; SProp is absorbed.
    assert!(u.is_subtype_of(&max_uv), "u ≤ max(u,v)");
    assert!(vv.is_subtype_of(&max_uv), "v ≤ max(u,v)");
    assert!(!max_uv.is_subtype_of(&u), "max(u,v) ⋠ u");
    assert!(
        Universe::Max(Box::new(u.clone()), Box::new(u.clone())).equiv(&u),
        "max(u,u) ≡ u",
    );
    assert_eq!(Universe::SProp.max(&u), u, "SProp is absorbed by max");
    assert_eq!(
        u.max(&Universe::SProp),
        u,
        "SProp is absorbed by max (either side)"
    );

    // imax with a VARIABLE codomain stays symbolic ⇒ the pivot case-split (v := 0 vs v ≥ 1).
    assert!(imax_uv.is_subtype_of(&imax_uv), "imax(u,v) ≤ imax(u,v)");
    assert!(
        imax_uv.is_subtype_of(&max_uv),
        "imax(u,v) ≤ max(u,v) always"
    );
    assert!(
        !max_uv.is_subtype_of(&imax_uv),
        "max(u,v) ⋠ imax(u,v) — fails at v = Prop, where imax collapses to 0",
    );
}
