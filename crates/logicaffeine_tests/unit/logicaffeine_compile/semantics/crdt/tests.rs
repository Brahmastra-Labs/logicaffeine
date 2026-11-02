use super::*;

/// A tiny deterministic PRNG (SplitMix64) so the property tests are exhaustively random
/// yet perfectly reproducible — no `rand`, no flakiness.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn int(n: u64) -> RuntimeValue {
    RuntimeValue::Int(n as i64)
}

/// A random OR-Set: a fresh replica with a random add/remove op stream over a small
/// element domain (so collisions and removals actually happen).
fn rand_set(rng: &mut Rng, domain: u64, ops: usize) -> CrdtValue {
    let mut s = CrdtValue::new_set(next_replica_id());
    for _ in 0..ops {
        let e = int(rng.below(domain));
        if rng.below(3) == 0 {
            let _ = s.remove(&e);
        } else {
            let _ = s.insert(&e);
        }
    }
    s
}

/// A random RGA: a fresh replica appending random elements.
fn rand_seq(rng: &mut Rng, domain: u64, ops: usize) -> CrdtValue {
    let mut s = CrdtValue::new_seq(next_replica_id());
    for _ in 0..ops {
        let _ = s.append(&int(rng.below(domain)));
    }
    s
}

/// `a ⊔ b` as a fresh value (never aliases an operand).
fn join(a: &CrdtValue, b: &CrdtValue) -> CrdtValue {
    let mut out = a.clone();
    out.merge(b).unwrap();
    out
}

#[test]
fn orset_merge_obeys_the_crdt_laws() {
    let mut rng = Rng(0xDEAD_BEEF);
    for _ in 0..500 {
        let a = rand_set(&mut rng, 6, 8);
        let b = rand_set(&mut rng, 6, 8);
        let c = rand_set(&mut rng, 6, 8);
        assert!(
            crdt_values_equal(&join(&a, &b), &join(&b, &a)),
            "commutative"
        );
        assert!(
            crdt_values_equal(&join(&join(&a, &b), &c), &join(&a, &join(&b, &c))),
            "associative"
        );
        assert!(crdt_values_equal(&join(&a, &a), &a), "idempotent");
    }
}

#[test]
fn rga_merge_obeys_the_crdt_laws() {
    let mut rng = Rng(0x0123_4567);
    for _ in 0..500 {
        let a = rand_seq(&mut rng, 6, 6);
        let b = rand_seq(&mut rng, 6, 6);
        let c = rand_seq(&mut rng, 6, 6);
        assert!(
            crdt_values_equal(&join(&a, &b), &join(&b, &a)),
            "commutative"
        );
        assert!(
            crdt_values_equal(&join(&join(&a, &b), &c), &join(&a, &join(&b, &c))),
            "associative"
        );
        assert!(crdt_values_equal(&join(&a, &a), &a), "idempotent");
    }
}

/// The distinguishing OR-Set property, at the unit level: replica `a` observes-and-
/// removes "X", replica `b` adds "X" CONCURRENTLY; after the join "X" survives, because
/// `b`'s add carries a tag `a`'s remove never saw. A grow-set-with-one-tombstone fails.
#[test]
fn orset_add_wins_over_concurrent_remove() {
    let x = int(42);
    let mut a = CrdtValue::new_set(next_replica_id());
    let mut b = CrdtValue::new_set(next_replica_id());
    a.insert(&x).unwrap();
    b.insert(&x).unwrap();
    a.remove(&x).unwrap();
    let m = join(&a, &b);
    assert!(
        m.contains(&x).unwrap(),
        "the concurrent add must survive the remove"
    );
}

#[test]
fn scalar_conversions_round_trip() {
    for v in [
        RuntimeValue::Int(-7),
        RuntimeValue::Text(std::rc::Rc::new("héllo".to_string())),
        RuntimeValue::Bool(true),
    ] {
        let s = CrdtScalar::from_runtime(&v).unwrap();
        assert_eq!(s.to_runtime(), v);
    }
    // A non-scalar is refused at the seam, not silently coerced.
    assert!(CrdtScalar::from_runtime(&RuntimeValue::Float(1.5)).is_err());
}

/// THE property the whole fix exists for: a `ConvergentCount` (G-Counter) merge is IDEMPOTENT
/// and commutative. Two replicas increment disjointly; re-merging the same remote state never
/// double-counts — the exact failure a bare-`Int` op-based counter had (5+3+3 = 11 instead of 8).
#[test]
fn convergent_count_merge_is_idempotent_and_commutative() {
    let mut a = CrdtValue::new_counter(next_replica_id());
    a.bump(5).unwrap();
    let mut b = CrdtValue::new_counter(next_replica_id());
    b.bump(3).unwrap();

    let ab = join(&a, &b);
    assert_eq!(ab.counter_value(), Some(8), "disjoint replicas sum");
    assert_eq!(
        join(&ab, &b).counter_value(),
        Some(8),
        "REDELIVERY is a no-op — never double-counts"
    );
    assert_eq!(join(&b, &a).counter_value(), Some(8), "commutative");
    // Two increments on the same replica accumulate monotonically in its own slot.
    let mut c = CrdtValue::new_counter(next_replica_id());
    c.bump(5).unwrap();
    c.bump(2).unwrap();
    assert_eq!(c.counter_value(), Some(7));
}

/// A `Tally` (PN-Counter) is idempotent under redelivery too, and tracks decrements.
#[test]
fn tally_merge_is_idempotent_and_tracks_decrements() {
    let mut a = CrdtValue::new_pn_counter(next_replica_id());
    a.bump(10).unwrap();
    let mut b = CrdtValue::new_pn_counter(next_replica_id());
    b.bump(4).unwrap();
    b.bump(-1).unwrap();

    let ab = join(&a, &b);
    assert_eq!(ab.counter_value(), Some(13), "10 + (4−1)");
    assert_eq!(
        join(&ab, &b).counter_value(),
        Some(13),
        "redelivery is idempotent"
    );
}

/// A counter's full state ships as its delta and round-trips through `apply_delta_bytes`,
/// converging to the union — and a redelivered delta is idempotent (the wire-edge safety the
/// `Sync` path needs). This locks the counter branch of the δ-CRDT wire codec.
#[test]
fn counter_delta_round_trips_and_is_idempotent_on_redelivery() {
    let mut a = CrdtValue::new_counter(next_replica_id());
    a.bump(5).unwrap();
    let mut b = CrdtValue::new_counter(next_replica_id());
    b.bump(3).unwrap();

    let delta = a
        .delta_since_bytes(&VClock::new())
        .expect("a counter always yields a delta");
    assert!(
        b.apply_delta_bytes(&delta),
        "the delta applies to a same-kind counter"
    );
    assert_eq!(b.counter_value(), Some(8));
    assert!(
        b.apply_delta_bytes(&delta),
        "re-applying the same delta still succeeds"
    );
    assert_eq!(b.counter_value(), Some(8), "and never double-counts");

    // A foreign delta (wrong kind tag) is refused, leaving the counter unchanged.
    let mut garbage = delta.clone();
    garbage[0] = 0; // claim it's an OR-Set delta
    let before = b.counter_value();
    assert!(!b.apply_delta_bytes(&garbage));
    assert_eq!(
        b.counter_value(),
        before,
        "a mismatched delta never corrupts the counter"
    );
}

/// EVERY kind survives the full-state wire form (what crosses a task / `Sync` boundary) as the SAME
/// kind with the same observable contents. The rebuild once mapped a remove-wins set to a sequence
/// and a sequence to a register, so a synced replica of either silently arrived empty.
#[test]
fn every_kind_round_trips_through_its_full_state() {
    let text = |s: &str| RuntimeValue::Text(std::rc::Rc::new(s.to_string()));
    let mut kinds = Vec::new();
    let mut set = CrdtValue::new_set(next_replica_id());
    set.insert(&int(3)).unwrap();
    kinds.push(set);
    let mut rw = CrdtValue::new_set_remove_wins(next_replica_id());
    rw.insert(&int(4)).unwrap();
    kinds.push(rw);
    let mut seq = CrdtValue::new_seq(next_replica_id());
    seq.append(&int(5)).unwrap();
    seq.append(&int(6)).unwrap();
    kinds.push(seq);
    let mut reg = CrdtValue::new_register(next_replica_id());
    reg.resolve(&text("x")).unwrap();
    kinds.push(reg);
    let mut g = CrdtValue::new_counter(next_replica_id());
    g.bump(7).unwrap();
    kinds.push(g);
    let mut pn = CrdtValue::new_pn_counter(next_replica_id());
    pn.bump(-2).unwrap();
    kinds.push(pn);
    let mut map = CrdtValue::new_map(next_replica_id());
    map.map_put(&text("wood"), &int(50)).unwrap();
    kinds.push(map);
    for original in kinds {
        let rebuilt = CrdtValue::from_full_state_bytes(&original.to_full_state_bytes());
        assert_eq!(
            rebuilt.kind(),
            original.kind(),
            "{} came back as another kind",
            original.kind()
        );
        assert_eq!(
            rebuilt.render(),
            original.render(),
            "{} lost its contents",
            original.kind()
        );
        assert!(!rebuilt.render().is_empty());
    }
}

#[test]
fn a_map_puts_reads_removes_and_renders_by_key() {
    let text = |s: &str| RuntimeValue::Text(std::rc::Rc::new(s.to_string()));
    let mut m = CrdtValue::new_map(next_replica_id());
    m.map_put(&text("wood"), &int(50)).unwrap();
    m.map_put(&text("clay"), &int(7)).unwrap();
    assert_eq!(m.map_get(&text("wood")).unwrap(), int(50));
    assert_eq!(
        m.map_get(&text("iron")).unwrap_err(),
        "Key 'iron' not found in map"
    );
    assert_eq!(m.render(), "{clay: 7, wood: 50}");
    assert_eq!(m.len(), 2);
    m.remove(&text("clay")).unwrap();
    assert!(!m.contains(&text("clay")).unwrap());
    assert_eq!(m.render(), "{wood: 50}");
}

/// A `ConvergentCount` is grow-only (rejects a decrement); a `Tally` accepts both directions and
/// can go negative. This is the bias each counter type promises.
#[test]
fn convergent_count_is_grow_only_but_tally_is_signed() {
    let mut g = CrdtValue::new_counter(next_replica_id());
    assert!(
        g.bump(-1).is_err(),
        "a ConvergentCount must refuse a decrement"
    );
    g.bump(4).unwrap();
    assert_eq!(g.counter_value(), Some(4));

    let mut t = CrdtValue::new_pn_counter(next_replica_id());
    t.bump(-5).unwrap();
    assert_eq!(t.counter_value(), Some(-5), "a Tally can go negative");
}

/// A counter renders as its integer value and compares EQUAL by value across distinct replicas
/// (the internal per-replica slots differ, but the observed number is what matters).
#[test]
fn counter_renders_and_equals_by_value() {
    let mut a = CrdtValue::new_counter(next_replica_id());
    a.bump(7).unwrap();
    assert_eq!(
        a.render(),
        "7",
        "a counter shows its value, not set/brace notation"
    );

    let mut b = CrdtValue::new_counter(next_replica_id());
    b.bump(7).unwrap();
    assert!(
        crdt_values_equal(&a, &b),
        "two counters at the same value are equal despite distinct replica ids"
    );
    let mut c = CrdtValue::new_counter(next_replica_id());
    c.bump(8).unwrap();
    assert!(!crdt_values_equal(&a, &c), "different values are not equal");
}
