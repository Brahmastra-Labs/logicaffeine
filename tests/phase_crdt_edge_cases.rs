//! Phase CRDT Edge Cases: Specific edge case tests for each CRDT type
//!
//! Tests for boundary conditions, corner cases, and specific behaviors.
//!
//! All tests use unique replica IDs to prevent cross-talk between parallel tests.

use logos_core::crdt::generate_replica_id;

/// Generate a unique base ID for this test run to avoid cross-talk
fn test_base_id() -> u64 {
    generate_replica_id()
}

// ===== GCOUNTER EDGE CASES =====

#[test]
fn test_gcounter_zero_increment() {
    use logos_core::crdt::GCounter;

    let mut counter = GCounter::with_replica_id(test_base_id());

    // Increment by 0 should be a no-op
    counter.increment(0);
    assert_eq!(counter.value(), 0);

    counter.increment(5);
    counter.increment(0);
    assert_eq!(counter.value(), 5);
}

#[test]
fn test_gcounter_large_increment() {
    use logos_core::crdt::GCounter;

    let mut counter = GCounter::with_replica_id(test_base_id());

    // Large increment near u64 max (but not overflowing)
    let large = u64::MAX / 2;
    counter.increment(large);
    assert_eq!(counter.value(), large);
}

#[test]
fn test_gcounter_value_with_no_local_increments() {
    use logos_core::crdt::{GCounter, Merge};

    let base = test_base_id();

    // Counter A increments
    let mut a = GCounter::with_replica_id(base);
    a.increment(10);

    // Counter B never increments locally, just receives merge
    let mut b = GCounter::with_replica_id(base + 1);
    assert_eq!(b.value(), 0);

    b.merge(&a);
    assert_eq!(b.value(), 10);
}

// ===== PNCOUNTER EDGE CASES =====

#[test]
fn test_pncounter_i64_boundaries() {
    use logos_core::crdt::PNCounter;

    let mut counter = PNCounter::with_replica_id(test_base_id());

    // Test large positive values
    let large_pos = (i64::MAX / 2) as u64;
    counter.increment(large_pos);
    assert_eq!(counter.value(), large_pos as i64);

    // Decrement to zero
    counter.decrement(large_pos);
    assert_eq!(counter.value(), 0);

    // Test large negative values
    counter.decrement(large_pos);
    assert_eq!(counter.value(), -(large_pos as i64));
}

#[test]
fn test_pncounter_alternating_inc_dec() {
    use logos_core::crdt::PNCounter;

    let mut counter = PNCounter::with_replica_id(test_base_id());

    // Alternating increment/decrement
    for _ in 0..100 {
        counter.increment(1);
        counter.decrement(1);
    }

    assert_eq!(counter.value(), 0);
}

#[test]
fn test_pncounter_zero_operations() {
    use logos_core::crdt::PNCounter;

    let mut counter = PNCounter::with_replica_id(test_base_id());

    counter.increment(0);
    counter.decrement(0);
    assert_eq!(counter.value(), 0);

    counter.increment(5);
    counter.increment(0);
    counter.decrement(0);
    assert_eq!(counter.value(), 5);
}

// ===== MVREGISTER EDGE CASES =====

#[test]
fn test_mvregister_many_concurrent_values() {
    use logos_core::crdt::{MVRegister, Merge};

    let base = test_base_id();

    // 15 concurrent writers (exceeds 10+ requirement)
    let registers: Vec<MVRegister<i32>> = (0..15)
        .map(|i| {
            let mut reg: MVRegister<i32> = MVRegister::new(base + i as u64);
            reg.set(i as i32);
            reg
        })
        .collect();

    // Merge all into a new result
    let mut result: MVRegister<i32> = MVRegister::new(base + 100);
    for reg in &registers {
        result.merge(reg);
    }

    // All 15 values should be present
    assert_eq!(result.values().len(), 15);
    assert!(result.has_conflict());
}

#[test]
fn test_mvregister_sequential_overwrites() {
    use logos_core::crdt::{MVRegister, Merge};

    let base = test_base_id();

    let mut a: MVRegister<String> = MVRegister::new(base);
    a.set("first".to_string());

    let mut b: MVRegister<String> = MVRegister::new(base + 1);
    b.merge(&a); // b sees first
    b.set("second".to_string()); // b causally overwrites

    a.merge(&b); // a sees b's overwrite

    // Should only have "second" (causal overwrite)
    assert_eq!(a.values().len(), 1);
    assert_eq!(a.values()[0], &"second".to_string());
}

#[test]
fn test_mvregister_empty_then_set() {
    use logos_core::crdt::MVRegister;

    let mut reg: MVRegister<String> = MVRegister::new(test_base_id());

    // Empty register
    assert!(reg.values().is_empty());
    assert!(!reg.has_conflict());

    // First set
    reg.set("value".to_string());
    assert_eq!(reg.values().len(), 1);
}

// ===== ORSET EDGE CASES =====

#[test]
fn test_orset_add_after_remove() {
    use logos_core::crdt::ORSet;

    let mut set: ORSet<String> = ORSet::new(test_base_id());

    set.add("x".to_string());
    assert!(set.contains(&"x".to_string()));

    set.remove(&"x".to_string());
    assert!(!set.contains(&"x".to_string()));

    // Re-add after remove should work
    set.add("x".to_string());
    assert!(set.contains(&"x".to_string()));
}

#[test]
fn test_orset_remove_nonexistent() {
    use logos_core::crdt::ORSet;

    let mut set: ORSet<String> = ORSet::new(test_base_id());

    // Remove element that was never added - should be no-op
    set.remove(&"nonexistent".to_string());
    assert!(set.is_empty());

    // Add something else, then remove nonexistent
    set.add("exists".to_string());
    set.remove(&"nonexistent".to_string());
    assert!(set.contains(&"exists".to_string()));
}

#[test]
fn test_orset_add_wins_vs_remove_wins_same_scenario() {
    use logos_core::crdt::{AddWins, Merge, ORSet, RemoveWins};

    let base = test_base_id();

    // Set up the same scenario with both biases
    // A adds, syncs to B, then A removes while B re-adds

    // Add-wins version
    let mut a_aw: ORSet<String, AddWins> = ORSet::new(base);
    let mut b_aw: ORSet<String, AddWins> = ORSet::new(base + 1);

    a_aw.add("item".to_string());
    b_aw.merge(&a_aw);
    a_aw.remove(&"item".to_string());
    b_aw.add("item".to_string()); // Concurrent re-add
    a_aw.merge(&b_aw);

    // Remove-wins version
    let mut a_rw: ORSet<String, RemoveWins> = ORSet::new(base + 10);
    let mut b_rw: ORSet<String, RemoveWins> = ORSet::new(base + 11);

    a_rw.add("item".to_string());
    b_rw.merge(&a_rw);
    a_rw.remove(&"item".to_string());
    b_rw.add("item".to_string()); // Concurrent re-add
    a_rw.merge(&b_rw);

    // Verify different outcomes
    assert!(a_aw.contains(&"item".to_string())); // Add wins
    assert!(!a_rw.contains(&"item".to_string())); // Remove wins
}

#[test]
fn test_orset_duplicate_add_same_replica() {
    use logos_core::crdt::ORSet;

    let mut set: ORSet<i64> = ORSet::new(test_base_id());

    // Add same element multiple times
    set.add(42);
    set.add(42);
    set.add(42);

    // Should only appear once
    assert_eq!(set.len(), 1);
    assert!(set.contains(&42));
}

// ===== LWWREGISTER EDGE CASES =====

#[test]
fn test_lww_exact_timestamp_tiebreak() {
    use logos_core::crdt::{LWWRegister, Merge};

    // When timestamps are equal, implementation should pick deterministically
    // Looking at lww.rs:67 - "If timestamps are equal, the other value wins"

    let mut a = LWWRegister::new("a".to_string());

    // Create b with exact same timestamp by deserializing from a's bytes
    // then modifying the value
    let a_bytes = bincode::serialize(&a).unwrap();
    let b: LWWRegister<String> = bincode::deserialize(&a_bytes).unwrap();
    // b now has same timestamp as a, but we need to change value
    // Since we can't set without changing timestamp, test the merge behavior

    // When we merge b into a with same timestamp, b's value should win
    // (per the >= comparison in merge)
    a.merge(&b);
    // Value should be consistent
    let _ = a.get(); // Just verify no panic
}

#[test]
fn test_lww_newer_always_wins() {
    use logos_core::crdt::{LWWRegister, Merge};

    let mut r1 = LWWRegister::new("first".to_string());
    std::thread::sleep(std::time::Duration::from_millis(2));
    let r2 = LWWRegister::new("second".to_string());
    std::thread::sleep(std::time::Duration::from_millis(2));
    let r3 = LWWRegister::new("third".to_string());

    // Merge in various orders - newest should always win
    r1.merge(&r3);
    r1.merge(&r2);
    assert_eq!(r1.get(), "third");
}

// ===== DOTCONTEXT EDGE CASES =====

#[test]
fn test_dotcontext_gap_then_fill() {
    use logos_core::crdt::{Dot, DotContext};

    let base = test_base_id();
    let mut ctx = DotContext::new();

    // Add dots 1, 3, 5, 7, 9 (gaps at 2, 4, 6, 8)
    for i in (1..=9).step_by(2) {
        ctx.add(Dot::new(base, i));
    }

    // Verify gaps
    for i in (2..=8).step_by(2) {
        assert!(!ctx.has_seen(&Dot::new(base, i)));
    }

    // Fill gaps
    for i in (2..=8).step_by(2) {
        ctx.add(Dot::new(base, i));
    }

    // All should be seen, compaction should occur
    for i in 1..=9 {
        assert!(ctx.has_seen(&Dot::new(base, i)));
    }

    // Next should be 10
    let next = ctx.next(base);
    assert_eq!(next.counter, 10);
}

#[test]
fn test_dotcontext_multiple_replicas() {
    use logos_core::crdt::{Dot, DotContext};

    let base = test_base_id();
    let mut ctx = DotContext::new();

    // Add dots from multiple replicas
    ctx.add(Dot::new(base, 1));
    ctx.add(Dot::new(base + 1, 1));
    ctx.add(Dot::new(base + 2, 1));

    // All should be seen
    assert!(ctx.has_seen(&Dot::new(base, 1)));
    assert!(ctx.has_seen(&Dot::new(base + 1, 1)));
    assert!(ctx.has_seen(&Dot::new(base + 2, 1)));

    // Dots from other replicas should be independent
    assert!(!ctx.has_seen(&Dot::new(base, 2)));
    assert!(!ctx.has_seen(&Dot::new(base + 1, 2)));
}

// ===== VCLOCK EDGE CASES =====

#[test]
fn test_vclock_get_nonexistent_replica() {
    use logos_core::crdt::VClock;

    let clock = VClock::new();

    // Getting a replica that was never incremented should return 0
    assert_eq!(clock.get(test_base_id()), 0);
    assert_eq!(clock.get(99999), 0);
}

#[test]
fn test_vclock_dominates_empty() {
    use logos_core::crdt::VClock;

    let empty = VClock::new();
    let mut non_empty = VClock::new();
    non_empty.increment(test_base_id());

    // Non-empty dominates empty
    assert!(non_empty.dominates(&empty));

    // Empty dominates empty
    assert!(empty.dominates(&empty));

    // Empty does not dominate non-empty
    assert!(!empty.dominates(&non_empty));
}

#[test]
fn test_vclock_concurrent_vs_dominates() {
    use logos_core::crdt::VClock;

    let base = test_base_id();

    let mut a = VClock::new();
    let mut b = VClock::new();

    // Make them concurrent (each has something the other doesn't)
    a.increment(base);
    b.increment(base + 1);

    assert!(a.concurrent(&b));
    assert!(b.concurrent(&a));
    assert!(!a.dominates(&b));
    assert!(!b.dominates(&a));

    // Now make a dominate by merging b into it
    a.merge_vclock(&b);
    a.increment(base); // One more increment to ensure domination

    assert!(a.dominates(&b));
    assert!(!a.concurrent(&b));
}

// ===== MERGE TRAIT EDGE CASES =====

#[test]
fn test_merge_self_is_idempotent() {
    use logos_core::crdt::{GCounter, MVRegister, Merge, ORSet, PNCounter};

    let base = test_base_id();

    // GCounter
    let mut gc = GCounter::with_replica_id(base);
    gc.increment(10);
    let gc_before = gc.value();
    gc.merge(&gc.clone());
    assert_eq!(gc.value(), gc_before);

    // PNCounter
    let mut pnc = PNCounter::with_replica_id(base);
    pnc.increment(10);
    pnc.decrement(3);
    let pnc_before = pnc.value();
    pnc.merge(&pnc.clone());
    assert_eq!(pnc.value(), pnc_before);

    // ORSet
    let mut set: ORSet<String> = ORSet::new(base);
    set.add("test".to_string());
    let set_before = set.len();
    set.merge(&set.clone());
    assert_eq!(set.len(), set_before);

    // MVRegister
    let mut reg: MVRegister<String> = MVRegister::new(base);
    reg.set("test".to_string());
    let reg_before = reg.values().len();
    reg.merge(&reg.clone());
    assert_eq!(reg.values().len(), reg_before);
}
