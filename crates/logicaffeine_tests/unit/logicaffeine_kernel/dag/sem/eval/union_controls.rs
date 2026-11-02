use super::*;
use std::rc::Rc;

fn reference(a: &[u32], b: &[u32], shift: u32) -> Vec<u32> {
    let mut v: Vec<u32> = a
        .iter()
        .copied()
        .chain(b.iter().filter(|&&i| i >= shift).map(|&i| i - shift))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// A small deterministic generator: sets of indices, ascending and distinct.
fn sets(seed: u64) -> impl Iterator<Item = Vec<u32>> {
    let mut x = seed;
    std::iter::from_fn(move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let len = (x % 12) as usize;
        let mut v: Vec<u32> = (0..len).map(|i| ((x >> (i * 5)) % 140) as u32).collect();
        v.sort_unstable();
        v.dedup();
        Some(v)
    })
}

#[test]
fn the_one_walk_union_is_the_union() {
    let mut pairs = sets(0x9E37_79B9_7F4A_7C15).zip(sets(0xD6E8_FEB8_6659_FD93));
    for _ in 0..4000 {
        let (a, b) = pairs.next().expect("endless");
        for shift in [0, 1, 3] {
            let (ra, rb): (Rc<[u32]>, Rc<[u32]>) = (a.clone().into(), b.clone().into());
            assert_eq!(
                &*union_shifted(&ra, &rb, shift),
                reference(&a, &b, shift).as_slice(),
                "{a:?} ∪ ({b:?} - {shift})"
            );
        }
    }
}

#[test]
fn a_union_that_adds_nothing_shares_its_allocation() {
    let a: Rc<[u32]> = vec![1, 4, 9].into();
    let inside: Rc<[u32]> = vec![2, 5, 10].into();
    assert!(
        Rc::ptr_eq(&union_shifted(&a, &inside, 1), &a),
        "{{2,5,10}} - 1 = {{1,4,9}} adds nothing"
    );
    let below: Rc<[u32]> = vec![0].into();
    assert!(
        Rc::ptr_eq(&union_shifted(&a, &below, 1), &a),
        "a variable the binder binds leaves the union"
    );
    let empty: Rc<[u32]> = Vec::new().into();
    assert!(
        Rc::ptr_eq(&union_shifted(&empty, &a, 0), &a),
        "the empty set adds nothing to the other side"
    );
}
