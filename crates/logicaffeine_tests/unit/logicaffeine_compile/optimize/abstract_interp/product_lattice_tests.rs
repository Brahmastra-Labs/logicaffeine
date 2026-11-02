use super::*;

fn av(iv: (i64, i64), t: TypeTag, sh: CollectionShape, n: Nullability) -> AbstractValue {
    AbstractValue {
        interval: Interval {
            lo: Bound::Finite(iv.0),
            hi: Bound::Finite(iv.1),
        },
        ty: TypeAbstraction::Concrete(t),
        shape: sh,
        nullability: n,
        alias: AliasInfo::Unique,
    }
}

#[test]
fn product_top_is_all_top() {
    let t = <AbstractValue as AbstractDomain>::top();
    assert_eq!(t.ty, TypeAbstraction::Top);
    assert_eq!(t.shape, CollectionShape::Top);
    assert_eq!(t.nullability, Nullability::Maybe);
    assert_eq!(t.alias, AliasInfo::Top);
    assert!(t.interval.lo == Bound::NegInf && t.interval.hi == Bound::PosInf);
}

#[test]
fn product_join_is_componentwise() {
    let a = av(
        (1, 1),
        TypeTag::Int,
        CollectionShape::Singleton,
        Nullability::Definite,
    );
    let b = av(
        (3, 3),
        TypeTag::Int,
        CollectionShape::KnownSize(3),
        Nullability::Definite,
    );
    let j = a.join(&b);
    // interval joins to [1,3]
    assert_eq!(j.interval.lo, Bound::Finite(1));
    assert_eq!(j.interval.hi, Bound::Finite(3));
    // same type stays Concrete(Int)
    assert_eq!(j.ty, TypeAbstraction::Concrete(TypeTag::Int));
    // shapes join to a size range
    assert_eq!(j.shape, CollectionShape::SizeRange(1, 3));
    // both Definite stays Definite
    assert_eq!(j.nullability, Nullability::Definite);
}

#[test]
fn product_join_mixed_types_widens_each_component() {
    let a = av(
        (0, 0),
        TypeTag::Int,
        CollectionShape::Empty,
        Nullability::Null,
    );
    let b = av(
        (5, 5),
        TypeTag::Bool,
        CollectionShape::Singleton,
        Nullability::Definite,
    );
    let j = a.join(&b);
    assert_eq!(j.interval.lo, Bound::Finite(0));
    assert_eq!(j.interval.hi, Bound::Finite(5));
    // distinct types → union
    assert_eq!(
        j.ty,
        TypeAbstraction::Union([TypeTag::Int, TypeTag::Bool].into_iter().collect())
    );
    assert_eq!(j.shape, CollectionShape::SizeRange(0, 1));
    // Null ⊔ Definite → Maybe
    assert_eq!(j.nullability, Nullability::Maybe);
}

#[test]
fn product_leq_and_widen_componentwise() {
    let a = av(
        (1, 1),
        TypeTag::Int,
        CollectionShape::Singleton,
        Nullability::Definite,
    );
    let top = <AbstractValue as AbstractDomain>::top();
    assert!(a.leq(&top));
    assert!(!top.leq(&a));
    // widen each component: the rising interval bound snaps to the
    // next THRESHOLD rung (EXODIA M9 ladder), not straight to +∞ —
    let b = av(
        (1, 9),
        TypeTag::Int,
        CollectionShape::KnownSize(9),
        Nullability::Definite,
    );
    let w = a.widen(&b);
    assert_eq!(w.interval.hi, Bound::Finite(10));
    assert_eq!(w.ty, TypeAbstraction::Concrete(TypeTag::Int));
    // — and falls off to +∞ only past the last rung.
    let c = av(
        (1, 5000),
        TypeTag::Int,
        CollectionShape::KnownSize(9),
        Nullability::Definite,
    );
    let w2 = a.widen(&c);
    assert!(matches!(w2.interval.hi, Bound::PosInf));
}
