use super::*;

#[test]
fn top_and_bottom() {
    assert_eq!(<Nullability as AbstractDomain>::top(), Nullability::Maybe);
    assert_eq!(
        <Nullability as AbstractDomain>::bottom(),
        Nullability::Bottom
    );
}

#[test]
fn literal_nullability() {
    // `nothing` is definitely absent; everything else is definitely present.
    assert_eq!(
        Nullability::for_literal(&Literal::Nothing),
        Nullability::Null
    );
    assert_eq!(
        Nullability::for_literal(&Literal::Number(5)),
        Nullability::Definite
    );
    assert_eq!(
        Nullability::for_literal(&Literal::Boolean(false)),
        Nullability::Definite
    );
}

#[test]
fn matched_variant_binding_is_definite() {
    // `Inspect x: When Some(v): ...` ⇒ inside the arm, the match is present.
    // This is the fact that lets the unwrap guard be eliminated.
    assert_eq!(Nullability::for_matched_variant(), Nullability::Definite);
}

#[test]
fn nullability_join_meet() {
    // join merges Definite and Null into Maybe (the diamond top)
    assert_eq!(
        Nullability::Definite.join(&Nullability::Null),
        Nullability::Maybe
    );
    assert_eq!(
        Nullability::Definite.join(&Nullability::Definite),
        Nullability::Definite
    );
    assert_eq!(
        Nullability::Bottom.join(&Nullability::Null),
        Nullability::Null
    );
    assert_eq!(
        Nullability::Maybe.join(&Nullability::Definite),
        Nullability::Maybe
    );
    // meet refines; Definite ⊓ Null is unreachable
    assert_eq!(
        Nullability::Definite.meet(&Nullability::Null),
        Nullability::Bottom
    );
    assert_eq!(
        Nullability::Maybe.meet(&Nullability::Definite),
        Nullability::Definite
    );
    assert_eq!(
        Nullability::Definite.meet(&Nullability::Definite),
        Nullability::Definite
    );
}

#[test]
fn nullability_order_and_widen() {
    assert!(Nullability::Bottom.leq(&Nullability::Definite));
    assert!(Nullability::Definite.leq(&Nullability::Maybe));
    assert!(Nullability::Null.leq(&Nullability::Maybe));
    assert!(Nullability::Definite.leq(&Nullability::Definite));
    assert!(!Nullability::Definite.leq(&Nullability::Null));
    assert!(!Nullability::Maybe.leq(&Nullability::Definite));
    // finite lattice → widen is join
    assert_eq!(
        Nullability::Definite.widen(&Nullability::Null),
        Nullability::Definite.join(&Nullability::Null)
    );
}
