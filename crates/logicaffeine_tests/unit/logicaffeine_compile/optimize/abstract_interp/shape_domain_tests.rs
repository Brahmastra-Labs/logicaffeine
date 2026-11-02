use super::*;

#[test]
fn new_collection_is_empty_and_push_pop_track_size() {
    assert_eq!(CollectionShape::empty_collection(), CollectionShape::Empty);
    // Push grows the size precisely.
    assert_eq!(CollectionShape::Empty.pushed(), CollectionShape::Singleton);
    assert_eq!(
        CollectionShape::Singleton.pushed(),
        CollectionShape::KnownSize(2)
    );
    assert_eq!(
        CollectionShape::KnownSize(2).pushed(),
        CollectionShape::KnownSize(3)
    );
    // Pop shrinks it.
    assert_eq!(
        CollectionShape::KnownSize(3).popped(),
        CollectionShape::KnownSize(2)
    );
    assert_eq!(CollectionShape::Singleton.popped(), CollectionShape::Empty);
    // Pop on a definitely-empty collection stays Empty (saturating).
    assert_eq!(CollectionShape::Empty.popped(), CollectionShape::Empty);
    // Pushing an unknown (Top) collection proves it non-empty.
    assert_eq!(CollectionShape::Top.pushed(), CollectionShape::NonEmpty);
}

#[test]
fn nonempty_and_empty_predicates() {
    assert!(CollectionShape::Singleton.is_definitely_nonempty());
    assert!(CollectionShape::KnownSize(5).is_definitely_nonempty());
    assert!(CollectionShape::NonEmpty.is_definitely_nonempty());
    assert!(!CollectionShape::Empty.is_definitely_nonempty());
    assert!(!CollectionShape::Top.is_definitely_nonempty());
    assert!(!CollectionShape::KnownSize(0).is_definitely_nonempty());

    assert!(CollectionShape::Empty.is_definitely_empty());
    assert!(CollectionShape::KnownSize(0).is_definitely_empty());
    assert!(!CollectionShape::Singleton.is_definitely_empty());
    assert!(!CollectionShape::Top.is_definitely_empty());
}

#[test]
fn shape_canonical_equality() {
    // Named variants that denote the same size set compare equal.
    assert_eq!(CollectionShape::KnownSize(0), CollectionShape::Empty);
    assert_eq!(CollectionShape::KnownSize(1), CollectionShape::Singleton);
    assert_eq!(CollectionShape::SizeRange(1, 1), CollectionShape::Singleton);
}

#[test]
fn shape_lattice_join_meet_leq() {
    // join is union of size ranges
    assert_eq!(
        CollectionShape::Empty.join(&CollectionShape::Singleton),
        CollectionShape::SizeRange(0, 1)
    );
    assert_eq!(
        CollectionShape::Singleton.join(&CollectionShape::KnownSize(3)),
        CollectionShape::SizeRange(1, 3)
    );
    assert_eq!(
        CollectionShape::Singleton.join(&CollectionShape::Top),
        CollectionShape::Top
    );
    // meet is intersection; disjoint sizes collapse to Bottom
    assert_eq!(
        CollectionShape::Empty.meet(&CollectionShape::Singleton),
        CollectionShape::Bottom
    );
    assert_eq!(
        CollectionShape::Top.meet(&CollectionShape::Singleton),
        CollectionShape::Singleton
    );
    // order
    assert!(CollectionShape::Singleton.leq(&CollectionShape::NonEmpty));
    assert!(CollectionShape::KnownSize(2).leq(&CollectionShape::SizeRange(0, 5)));
    assert!(CollectionShape::Empty.leq(&CollectionShape::Top));
    assert!(CollectionShape::Bottom.leq(&CollectionShape::Empty));
    assert!(!CollectionShape::Top.leq(&CollectionShape::NonEmpty));
}

#[test]
fn shape_widening_converges_growing_loops() {
    // A push-only loop keeps growing the upper bound → +∞, stays NonEmpty.
    let w = CollectionShape::KnownSize(1).widen(&CollectionShape::KnownSize(2));
    assert_eq!(w, CollectionShape::NonEmpty);
    // A collection that might shrink widens its lower bound toward 0.
    let w2 = CollectionShape::KnownSize(2).widen(&CollectionShape::KnownSize(1));
    assert!(w2.leq(&CollectionShape::Top));
}
