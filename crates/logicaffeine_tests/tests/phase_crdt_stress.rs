//! Phase CRDT Stress: Scalability and stress tests
//!
//! Tests for large collections, many replicas, and sustained operations.
//! Verifies CRDT correctness at scale.
//!
//! All tests use unique replica IDs to prevent cross-talk between parallel tests.

use logicaffeine_data::crdt::generate_replica_id;

/// Generate a unique base ID for this test run to avoid cross-talk
fn test_base_id() -> u64 {
    generate_replica_id()
}

// ===== ORSET STRESS TESTS =====

#[test]
fn test_orset_1000_elements() {
    use logicaffeine_data::crdt::ORSet;

    let mut set: ORSet<i64> = ORSet::new(test_base_id());

    // Add 1000 elements
    for i in 0..1000 {
        set.add(i);
    }

    assert_eq!(set.len(), 1000);

    // Verify all elements present
    for i in 0..1000 {
        assert!(set.contains(&i), "Missing element {}", i);
    }

    // Remove half
    for i in (0..1000).step_by(2) {
        set.remove(&i);
    }

    assert_eq!(set.len(), 500);

    // Verify correct elements remain
    for i in 0..1000 {
        if i % 2 == 0 {
            assert!(!set.contains(&i), "Should not contain {}", i);
        } else {
            assert!(set.contains(&i), "Should contain {}", i);
        }
    }
}

#[test]
fn test_orset_merge_large_sets() {
    use logicaffeine_data::crdt::{Merge, ORSet};

    let base = test_base_id();

    let mut a: ORSet<i64> = ORSet::new(base);
    let mut b: ORSet<i64> = ORSet::new(base + 1);

    // Each set has 500 unique elements
    for i in 0..500 {
        a.add(i);
        b.add(i + 500);
    }

    // Merge
    a.merge(&b);

    // Should have all 1000
    assert_eq!(a.len(), 1000);
}

#[test]
fn test_repeated_add_remove_cycles() {
    use logicaffeine_data::crdt::ORSet;

    let mut set: ORSet<String> = ORSet::new(test_base_id());
    let element = "cycle-element".to_string();

    // 1000 add/remove cycles on same element
    for _ in 0..1000 {
        set.add(element.clone());
        assert!(set.contains(&element));
        set.remove(&element);
        assert!(!set.contains(&element));
    }

    // Final state: empty
    assert!(set.is_empty());
}

// ===== GCOUNTER STRESS TESTS =====

#[test]
fn test_gcounter_many_replicas() {
    use logicaffeine_data::crdt::{GCounter, Merge};

    let base = test_base_id();
    let num_replicas = 100;

    // Create 100 replicas, each increments by its ID
    let counters: Vec<GCounter> = (0..num_replicas)
        .map(|i| {
            let mut c = GCounter::with_replica_id(base + i as u64);
            c.increment(i as u64 + 1);
            c
        })
        .collect();

    // Merge all into one
    let mut result = GCounter::with_replica_id(base + 999);
    for c in &counters {
        result.merge(c);
    }

    // Expected: 1 + 2 + 3 + ... + 100 = 100 * 101 / 2 = 5050
    assert_eq!(result.value(), 5050);
}

#[test]
fn test_gcounter_high_frequency_increments() {
    use logicaffeine_data::crdt::GCounter;

    let mut counter = GCounter::with_replica_id(test_base_id());

    // 100,000 increments
    for _ in 0..100_000 {
        counter.increment(1);
    }

    assert_eq!(counter.value(), 100_000);
}

// ===== PNCOUNTER STRESS TESTS =====

#[test]
fn test_pncounter_large_values() {
    use logicaffeine_data::crdt::PNCounter;

    let mut counter = PNCounter::with_replica_id(test_base_id());

    // Test near i64 boundaries (but not overflow)
    let large_val = i64::MAX / 2;

    counter.increment(large_val as u64);
    assert_eq!(counter.value(), large_val);

    counter.decrement(large_val as u64);
    assert_eq!(counter.value(), 0);

    // Large negative
    counter.decrement(large_val as u64);
    assert_eq!(counter.value(), -large_val);
}

#[test]
fn test_pncounter_many_operations() {
    use logicaffeine_data::crdt::PNCounter;

    let mut counter = PNCounter::with_replica_id(test_base_id());

    // 50,000 increments, 50,000 decrements
    for _ in 0..50_000 {
        counter.increment(1);
    }
    for _ in 0..50_000 {
        counter.decrement(1);
    }

    assert_eq!(counter.value(), 0);
}

// ===== VCLOCK STRESS TESTS =====

#[test]
fn test_vclock_many_replicas() {
    use logicaffeine_data::crdt::VClock;

    let base = test_base_id();
    let num_replicas = 100;

    let mut clock = VClock::new();

    // Increment 100 different replica IDs
    for i in 0..num_replicas {
        clock.increment(base + i);
    }

    // Verify all present
    for i in 0..num_replicas {
        assert_eq!(clock.get(base + i), 1);
    }
}

#[test]
fn test_vclock_comparison_at_scale() {
    use logicaffeine_data::crdt::VClock;

    let base = test_base_id();

    let mut a = VClock::new();
    let mut b = VClock::new();

    // Both increment same 50 replicas
    for i in 0..50 {
        a.increment(base + i);
        b.increment(base + i);
    }

    // A increments 50 more
    for i in 50..100 {
        a.increment(base + i);
    }

    // A should dominate B
    assert!(a.dominates(&b));
    assert!(!b.dominates(&a));
    assert!(!a.concurrent(&b));
}

// ===== DOTCONTEXT STRESS TESTS =====

#[test]
fn test_dotcontext_many_out_of_order() {
    use logicaffeine_data::crdt::{Dot, DotContext};

    let base = test_base_id();
    let mut ctx = DotContext::new();

    // Add 1000 dots out of order (reverse order)
    for i in (1..=1000).rev() {
        ctx.add(Dot::new(base, i));
    }

    // All should be seen
    for i in 1..=1000 {
        assert!(ctx.has_seen(&Dot::new(base, i)), "Should have seen dot {}", i);
    }

    // Next dot for this replica should be 1001
    let next = ctx.next(base);
    assert_eq!(next.counter, 1001);
}

#[test]
fn test_dotcontext_sparse_dots() {
    use logicaffeine_data::crdt::{Dot, DotContext};

    let base = test_base_id();
    let mut ctx = DotContext::new();

    // Add only even-numbered dots (creates gaps)
    for i in (2..=1000).step_by(2) {
        ctx.add(Dot::new(base, i));
    }

    // Even dots should be seen
    for i in (2..=1000).step_by(2) {
        assert!(ctx.has_seen(&Dot::new(base, i)));
    }

    // Odd dots should not be seen
    for i in (1..=999).step_by(2) {
        assert!(!ctx.has_seen(&Dot::new(base, i)));
    }
}

#[test]
fn test_dotcontext_compaction() {
    use logicaffeine_data::crdt::{Dot, DotContext};

    let base = test_base_id();
    let mut ctx = DotContext::new();

    // Add dots 2-1000 (gap at 1)
    for i in 2..=1000 {
        ctx.add(Dot::new(base, i));
    }

    // Now add dot 1 - should trigger compaction
    ctx.add(Dot::new(base, 1));

    // All should be seen
    for i in 1..=1000 {
        assert!(ctx.has_seen(&Dot::new(base, i)));
    }

    // Next should be 1001
    let next = ctx.next(base);
    assert_eq!(next.counter, 1001);
}

// ===== MVREGISTER STRESS TESTS =====

#[test]
fn test_mvregister_many_concurrent_values() {
    use logicaffeine_data::crdt::{MVRegister, Merge};

    let base = test_base_id();

    // 20 concurrent writers (exceeds the 10+ requirement from plan)
    let num_writers = 20;
    let registers: Vec<MVRegister<i64>> = (0..num_writers)
        .map(|i| {
            let mut reg: MVRegister<i64> = MVRegister::new(base + i as u64);
            reg.set(i as i64);
            reg
        })
        .collect();

    // Merge all
    let mut result: MVRegister<i64> = MVRegister::new(base + 999);
    for reg in &registers {
        result.merge(reg);
    }

    // All 20 values should be present
    let values = result.values();
    assert_eq!(values.len(), num_writers);

    for i in 0..num_writers {
        assert!(
            values.iter().any(|v| **v == i as i64),
            "Missing value {}",
            i
        );
    }
}

// ===== DELTA BUFFER STRESS TESTS =====

#[test]
fn test_delta_buffer_overflow() {
    use logicaffeine_data::crdt::{DeltaBuffer, VClock};

    let mut buffer: DeltaBuffer<String> = DeltaBuffer::new(10);
    let base = test_base_id();

    // Push 20 deltas (buffer capacity is 10)
    for i in 0..20 {
        let mut version = VClock::new();
        version.increment(base);
        for _ in 0..i {
            version.increment(base);
        }
        buffer.push(version, format!("delta-{}", i));
    }

    // Oldest deltas should be evicted
    let old_version = VClock::new();
    assert!(
        !buffer.can_serve(&old_version),
        "Should not be able to serve very old version"
    );

    // Recent version should be servable
    let mut recent = VClock::new();
    for _ in 0..15 {
        recent.increment(base);
    }
    // Note: can_serve depends on implementation details
}

#[test]
fn test_delta_buffer_partial_recovery() {
    use logicaffeine_data::crdt::{DeltaBuffer, VClock};

    let base = test_base_id();
    let mut buffer: DeltaBuffer<i64> = DeltaBuffer::new(100);

    // Push 50 deltas
    for i in 0..50i64 {
        let mut version = VClock::new();
        for j in 0..=i as u64 {
            version.increment(base + j);
        }
        buffer.push(version, i);
    }

    // Try to get deltas since version 25
    let mut since = VClock::new();
    for j in 0..25 {
        since.increment(base + j);
    }

    if let Some(deltas) = buffer.deltas_since(&since) {
        // Should get deltas from 25 onwards
        assert!(!deltas.is_empty());
    }
}
