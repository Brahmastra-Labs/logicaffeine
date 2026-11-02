use super::*;

/// Deterministic splitmix64 — reproducible fuzzing, no external RNG.
struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn upto(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn fills_to_capacity_without_dropping_anything() {
    let mut r = BoundedRing::new(4);
    for i in 0..4 {
        assert_eq!(r.push(i), None, "no eviction below capacity");
    }
    assert_eq!(r.len(), 4);
    assert!(r.is_full());
    assert_eq!(r.dropped(), 0);
}

#[test]
fn a_push_into_a_full_ring_evicts_and_counts_the_oldest() {
    let mut r = BoundedRing::new(3);
    r.push(10);
    r.push(11);
    r.push(12); // full: [10, 11, 12]
    let evicted = r.push(13); // evict oldest (10) → [11, 12, 13]
    assert_eq!(
        evicted,
        Some(10),
        "the OLDEST item is shed, never the freshest"
    );
    assert_eq!(r.len(), 3, "length stays clamped at the capacity");
    assert_eq!(r.dropped(), 1);
    // FIFO order of the survivors: 11, 12, 13.
    assert_eq!(r.pop(), Some(11));
    assert_eq!(r.pop(), Some(12));
    assert_eq!(r.pop(), Some(13));
    assert_eq!(r.pop(), None);
}

#[test]
fn under_sustained_overflow_it_retains_exactly_the_last_capacity_items() {
    let cap = 5usize;
    let n = 100usize;
    let mut r = BoundedRing::new(cap);
    for i in 0..n {
        r.push(i);
    }
    assert_eq!(r.len(), cap, "never exceeds capacity (no unbounded growth)");
    assert_eq!(r.dropped() as usize, n - cap, "drop count is exact");
    // The survivors are the freshest `cap` items, in order.
    let survivors: Vec<usize> = std::iter::from_fn(|| r.pop()).collect();
    assert_eq!(
        survivors,
        (n - cap..n).collect::<Vec<_>>(),
        "freshest-data-wins, FIFO preserved"
    );
}

#[test]
fn a_capacity_one_ring_always_holds_only_the_freshest() {
    let mut r = BoundedRing::new(1);
    assert_eq!(r.push('a'), None);
    assert_eq!(r.push('b'), Some('a'));
    assert_eq!(r.push('c'), Some('b'));
    assert_eq!(r.len(), 1);
    assert_eq!(r.dropped(), 2);
    assert_eq!(r.pop(), Some('c'));
}

#[test]
fn interleaved_push_and_pop_keeps_fifo_order_and_never_overflows() {
    let mut r = BoundedRing::new(3);
    r.push(1);
    r.push(2);
    assert_eq!(r.pop(), Some(1)); // [2]
    r.push(3);
    r.push(4); // [2,3,4] full, no drop yet
    assert_eq!(r.dropped(), 0);
    let evicted = r.push(5); // evict 2 → [3,4,5]
    assert_eq!(evicted, Some(2));
    assert_eq!(r.dropped(), 1);
    assert_eq!(r.pop(), Some(3));
    assert_eq!(r.pop(), Some(4));
    assert_eq!(r.pop(), Some(5));
    assert!(r.is_empty());
}

#[test]
fn draining_empties_and_the_ring_refills_cleanly() {
    let mut r = BoundedRing::new(2);
    r.push('x');
    r.push('y');
    while r.pop().is_some() {}
    assert!(r.is_empty() && !r.is_full());
    // Refill after a full drain behaves like a fresh ring (drop-count is cumulative telemetry).
    r.push('z');
    assert_eq!(r.len(), 1);
    assert_eq!(r.pop(), Some('z'));
}

#[test]
fn a_clone_is_an_independent_snapshot() {
    let mut r = BoundedRing::new(2);
    r.push(1);
    let mut snap = r.clone();
    r.push(2);
    r.push(3); // r drops 1 → [2,3], dropped=1
               // The clone is untouched by later mutations of the original.
    assert_eq!(snap.dropped(), 0);
    assert_eq!(snap.pop(), Some(1));
    assert_eq!(r.dropped(), 1);
}

#[test]
fn the_length_invariant_and_drop_accounting_hold_under_fuzz() {
    let mut rng = R(0x00C0_FFEE_1234_5678);
    for _ in 0..2000 {
        let cap = 1 + rng.upto(16);
        let mut r: BoundedRing<u64> = BoundedRing::new(cap);
        let mut pushed = 0u64;
        let mut popped = 0u64;
        for _ in 0..200 {
            if rng.upto(3) == 0 && !r.is_empty() {
                r.pop();
                popped += 1;
            } else {
                r.push(rng.next());
                pushed += 1;
            }
            // The core safety invariant: memory is bounded, ALWAYS.
            assert!(r.len() <= cap, "len must never exceed capacity");
        }
        // Conservation: everything pushed is either still buffered, popped, or dropped.
        assert_eq!(
            pushed,
            r.len() as u64 + popped + r.dropped(),
            "no item is unaccounted for"
        );
    }
}
