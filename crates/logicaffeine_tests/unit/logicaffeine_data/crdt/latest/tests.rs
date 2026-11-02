use super::*;

fn join(a: &LatestWrite<i64>, b: &LatestWrite<i64>) -> LatestWrite<i64> {
    let mut out = a.clone();
    out.merge(b);
    out
}

#[test]
fn the_later_stamp_wins_whatever_the_values() {
    let old = LatestWrite::new(99, 1);
    let new = LatestWrite::new(1, 2);
    assert_eq!(*join(&old, &new).get(), 1);
    assert_eq!(*join(&new, &old).get(), 1);
}

#[test]
fn a_tie_resolves_to_the_larger_value_from_either_side() {
    let a = LatestWrite::new(50, 3);
    let b = LatestWrite::new(99, 3);
    assert_eq!(join(&a, &b), join(&b, &a));
    assert_eq!(*join(&a, &b).get(), 99);
}

#[test]
fn merge_is_a_join() {
    let regs: Vec<LatestWrite<i64>> = (0..6)
        .flat_map(|s| (0..4).map(move |v| LatestWrite::new(v, s)))
        .collect();
    for a in &regs {
        assert_eq!(&join(a, a), a, "idempotent");
        for b in &regs {
            assert_eq!(join(a, b), join(b, a), "commutative");
            for c in &regs {
                assert_eq!(join(&join(a, b), c), join(a, &join(b, c)), "associative");
            }
        }
    }
}
