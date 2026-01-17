//! Phase CRDT Concurrent: Multi-threaded CRDT operation tests
//!
//! Tests for concurrent operations from multiple threads/replicas.
//! Verifies CRDT properties hold under realistic contention.
//!
//! All tests use unique replica IDs to prevent cross-talk between parallel tests.

use logicaffeine_data::crdt::generate_replica_id;
use std::sync::{Arc, Mutex};
use std::thread;

/// Generate a unique base ID for this test run to avoid cross-talk
fn test_base_id() -> u64 {
    generate_replica_id()
}

// ===== GCOUNTER CONCURRENT TESTS =====

#[test]
fn test_gcounter_concurrent_increments() {
    use logicaffeine_data::crdt::{GCounter, Merge};

    let base = test_base_id();
    let num_threads = 10;
    let increments_per_thread = 1000;

    // Each thread gets its own counter with unique replica ID (base + offset)
    let counters: Vec<_> = (0..num_threads)
        .map(|i| Arc::new(Mutex::new(GCounter::with_replica_id(base + i as u64))))
        .collect();

    let handles: Vec<_> = counters
        .iter()
        .cloned()
        .map(|counter| {
            thread::spawn(move || {
                let mut c = counter.lock().unwrap();
                for _ in 0..increments_per_thread {
                    c.increment(1);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // Merge all counters into one
    let mut result = GCounter::with_replica_id(base + 999);
    for counter in &counters {
        result.merge(&counter.lock().unwrap());
    }

    // Total should be num_threads * increments_per_thread
    assert_eq!(
        result.value(),
        (num_threads * increments_per_thread) as u64
    );
}

#[test]
fn test_gcounter_concurrent_merge_order_independence() {
    use logicaffeine_data::crdt::{GCounter, Merge};

    let base = test_base_id();

    // Create 5 counters with different values
    let counters: Vec<GCounter> = (0..5)
        .map(|i| {
            let mut c = GCounter::with_replica_id(base + i as u64);
            c.increment((i + 1) as u64 * 10);
            c
        })
        .collect();

    // Expected: 10 + 20 + 30 + 40 + 50 = 150
    let expected = 150u64;

    // Merge in forward order
    let mut forward = counters[0].clone();
    for c in &counters[1..] {
        forward.merge(c);
    }

    // Merge in reverse order
    let mut reverse = counters[4].clone();
    for c in counters[..4].iter().rev() {
        reverse.merge(c);
    }

    // Merge in random order (3, 1, 4, 0, 2)
    let order = [3, 1, 4, 0, 2];
    let mut random = counters[order[0]].clone();
    for &i in &order[1..] {
        random.merge(&counters[i]);
    }

    assert_eq!(forward.value(), expected);
    assert_eq!(reverse.value(), expected);
    assert_eq!(random.value(), expected);
}

// ===== PNCOUNTER CONCURRENT TESTS =====

#[test]
fn test_pncounter_concurrent_inc_dec() {
    use logicaffeine_data::crdt::{Merge, PNCounter};

    let base = test_base_id();
    let num_threads = 10;
    let ops_per_thread = 500;

    let counters: Vec<_> = (0..num_threads)
        .map(|i| Arc::new(Mutex::new(PNCounter::with_replica_id(base + i as u64))))
        .collect();

    let handles: Vec<_> = counters
        .iter()
        .enumerate()
        .map(|(i, counter)| {
            let counter = counter.clone();
            thread::spawn(move || {
                let mut c = counter.lock().unwrap();
                for j in 0..ops_per_thread {
                    if (i + j) % 2 == 0 {
                        c.increment(1);
                    } else {
                        c.decrement(1);
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // Merge all counters
    let mut result = PNCounter::with_replica_id(base + 999);
    for counter in &counters {
        result.merge(&counter.lock().unwrap());
    }

    // Each thread does 500 ops alternating inc/dec based on (thread_id + op_idx) % 2
    // This is deterministic - we can calculate expected value
    let mut expected = 0i64;
    for i in 0..num_threads {
        for j in 0..ops_per_thread {
            if (i + j) % 2 == 0 {
                expected += 1;
            } else {
                expected -= 1;
            }
        }
    }
    assert_eq!(result.value(), expected);
}

#[test]
fn test_pncounter_concurrent_same_replica_merge() {
    use logicaffeine_data::crdt::{Merge, PNCounter};

    let id = test_base_id();

    // Simulate concurrent branches from same replica
    let mut base = PNCounter::with_replica_id(id);
    base.increment(100);

    // Branch A: more increments
    let mut branch_a = base.clone();
    branch_a.increment(50);

    // Branch B: decrements
    let mut branch_b = base.clone();
    branch_b.decrement(30);

    // After merge, should have max of increments and max of decrements
    // Base had 100 inc, 0 dec
    // A has 150 inc, 0 dec
    // B has 100 inc, 30 dec
    // Merge: max(150, 100) inc = 150, max(0, 30) dec = 30
    // Result: 150 - 30 = 120
    branch_a.merge(&branch_b);
    assert_eq!(branch_a.value(), 120);
}

// ===== ORSET CONCURRENT TESTS =====

#[test]
fn test_orset_concurrent_add_remove() {
    use logicaffeine_data::crdt::{Merge, ORSet};

    let base = test_base_id();

    // Replica 1: adds items
    let mut r1: ORSet<String> = ORSet::new(base);
    r1.add("a".to_string());
    r1.add("b".to_string());
    r1.add("c".to_string());

    // Replica 2: removes one item (after seeing r1's state)
    let mut r2: ORSet<String> = ORSet::new(base + 1);
    r2.merge(&r1);
    r2.remove(&"b".to_string());

    // Replica 3: adds the same item concurrently (without seeing r2's remove)
    let mut r3: ORSet<String> = ORSet::new(base + 2);
    r3.merge(&r1);
    r3.add("b".to_string()); // Concurrent add

    // Merge r2 and r3
    r2.merge(&r3);

    // With add-wins (default), "b" should be present
    assert!(r2.contains(&"a".to_string()));
    assert!(r2.contains(&"b".to_string())); // Add wins over concurrent remove
    assert!(r2.contains(&"c".to_string()));
}

#[test]
fn test_orset_interleaved_add_remove_add() {
    use logicaffeine_data::crdt::ORSet;

    let mut set: ORSet<String> = ORSet::new(test_base_id());

    // Add, remove, add same element
    set.add("x".to_string());
    assert!(set.contains(&"x".to_string()));

    set.remove(&"x".to_string());
    assert!(!set.contains(&"x".to_string()));

    set.add("x".to_string());
    assert!(set.contains(&"x".to_string()));

    // Element should be present with new dot
    assert_eq!(set.len(), 1);
}

#[test]
fn test_orset_concurrent_from_multiple_replicas() {
    use logicaffeine_data::crdt::{Merge, ORSet};

    let base = test_base_id();

    // 5 replicas, each adds unique elements
    let mut sets: Vec<ORSet<i64>> = (0..5).map(|i| ORSet::new(base + i as u64)).collect();

    for (i, set) in sets.iter_mut().enumerate() {
        for j in 0..10 {
            set.add((i * 10 + j) as i64);
        }
    }

    // Merge all into first
    let mut result = sets[0].clone();
    for set in &sets[1..] {
        result.merge(set);
    }

    // Should have 50 unique elements
    assert_eq!(result.len(), 50);

    // All elements should be present
    for i in 0..50 {
        assert!(result.contains(&(i as i64)));
    }
}

// ===== MVREGISTER CONCURRENT TESTS =====

#[test]
fn test_mvregister_concurrent_writes() {
    use logicaffeine_data::crdt::{MVRegister, Merge};

    let base = test_base_id();

    // N replicas write simultaneously (without seeing each other)
    let n = 5;
    let registers: Vec<_> = (0..n)
        .map(|i| {
            let mut reg: MVRegister<String> = MVRegister::new(base + i as u64);
            reg.set(format!("value-{}", i));
            reg
        })
        .collect();

    // Merge all
    let mut result: MVRegister<String> = MVRegister::new(base + 999);
    for reg in &registers {
        result.merge(reg);
    }

    // All values should be present (conflict)
    let values = result.values();
    assert_eq!(values.len(), n);
    assert!(result.has_conflict());

    // All original values should be in the result
    for i in 0..n {
        let expected = format!("value-{}", i);
        assert!(
            values.iter().any(|v| **v == expected),
            "Missing value-{}",
            i
        );
    }
}

#[test]
fn test_mvregister_conflict_then_resolve() {
    use logicaffeine_data::crdt::{MVRegister, Merge};

    let base = test_base_id();

    let mut a: MVRegister<String> = MVRegister::new(base);
    let mut b: MVRegister<String> = MVRegister::new(base + 1);
    let mut c: MVRegister<String> = MVRegister::new(base + 2);

    // All write concurrently
    a.set("A".to_string());
    b.set("B".to_string());
    c.set("C".to_string());

    // Merge all into a
    a.merge(&b);
    a.merge(&c);

    assert_eq!(a.values().len(), 3);
    assert!(a.has_conflict());

    // Resolve conflict
    a.resolve("Resolved".to_string());

    assert_eq!(a.values().len(), 1);
    assert!(!a.has_conflict());
    assert_eq!(a.values()[0], &"Resolved".to_string());

    // Merge resolved state back
    b.merge(&a);
    c.merge(&a);

    // All should now agree
    assert_eq!(b.values().len(), 1);
    assert_eq!(c.values().len(), 1);
}

#[test]
fn test_mvregister_causal_overwrite() {
    use logicaffeine_data::crdt::{MVRegister, Merge};

    let base = test_base_id();

    let mut a: MVRegister<i64> = MVRegister::new(base);
    a.set(1);

    // b sees a's value, then overwrites
    let mut b: MVRegister<i64> = MVRegister::new(base + 1);
    b.merge(&a);
    b.set(2);

    // Merge back - b's value should dominate (causal)
    a.merge(&b);

    assert_eq!(a.values().len(), 1);
    assert_eq!(*a.values()[0], 2);
}

// ===== LWWREGISTER CONCURRENT TESTS =====

#[test]
fn test_lww_concurrent_writes() {
    use logicaffeine_data::crdt::{LWWRegister, Merge};

    // Create registers with controlled timestamps (explicit, not SystemTime)
    let mut regs: Vec<LWWRegister<String>> = Vec::new();

    for i in 0..5 {
        // Each register gets a higher timestamp
        let reg = LWWRegister::new(format!("value-{}", i), (i + 1) as u64 * 100);
        regs.push(reg);
    }

    // Last one should have highest timestamp (500)
    let mut result = regs[0].clone();
    for reg in &regs[1..] {
        result.merge(reg);
    }

    // Last writer wins (highest timestamp)
    assert_eq!(result.get(), "value-4");
}

#[test]
fn test_lww_merge_order_independence() {
    use logicaffeine_data::crdt::{LWWRegister, Merge};

    // Explicit timestamps: r1=100, r2=200, r3=300
    let r1 = LWWRegister::new("first".to_string(), 100);
    let r2 = LWWRegister::new("second".to_string(), 200);
    let r3 = LWWRegister::new("third".to_string(), 300);

    // Merge in different orders
    let mut forward = r1.clone();
    forward.merge(&r2);
    forward.merge(&r3);

    let mut reverse = r3.clone();
    reverse.merge(&r2);
    reverse.merge(&r1);

    let mut middle = r2.clone();
    middle.merge(&r1);
    middle.merge(&r3);

    // All should have "third" (highest timestamp)
    assert_eq!(forward.get(), "third");
    assert_eq!(reverse.get(), "third");
    assert_eq!(middle.get(), "third");
}

// ===== VCLOCK CONCURRENT TESTS =====

#[test]
fn test_vclock_concurrent_increments() {
    use logicaffeine_data::crdt::VClock;

    let base = test_base_id();
    let mut clocks: Vec<VClock> = (0..10).map(|_| VClock::new()).collect();

    // Each clock increments different replicas (unique per test)
    for (i, clock) in clocks.iter_mut().enumerate() {
        for j in 0..100 {
            clock.increment(base + (i * 100 + j) as u64);
        }
    }

    // Merge all
    let mut result = VClock::new();
    for clock in &clocks {
        result.merge_vclock(clock);
    }

    // Verify all entries present
    for i in 0..10 {
        for j in 0..100 {
            assert_eq!(result.get(base + (i * 100 + j) as u64), 1);
        }
    }
}

#[test]
fn test_vclock_concurrent_detection() {
    use logicaffeine_data::crdt::VClock;

    let base = test_base_id();
    let mut a = VClock::new();
    let mut b = VClock::new();

    // a and b increment independently (unique replica IDs)
    a.increment(base);
    a.increment(base);
    b.increment(base + 1);
    b.increment(base + 1);

    // Neither dominates the other
    assert!(!a.dominates(&b));
    assert!(!b.dominates(&a));
    assert!(a.concurrent(&b));
}
