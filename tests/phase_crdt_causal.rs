//! Phase CRDT Causal: Tests for causal infrastructure (VClock, Dot, DotContext)
//!
//! Wave 1 of CRDT Expansion: Foundation for delta-state CRDTs.
//!
//! TDD: These are RED tests - they define the spec before implementation.

// ===== WAVE 1.1: REPLICA ID MIGRATION =====

#[test]
fn test_replica_id_is_u64() {
    let id = logos_core::crdt::generate_replica_id();
    let _: u64 = id; // Must compile as u64
    assert!(id > 0);
}

#[test]
fn test_replica_id_unique() {
    let id1 = logos_core::crdt::generate_replica_id();
    let id2 = logos_core::crdt::generate_replica_id();
    assert_ne!(id1, id2);
}

#[test]
fn test_gcounter_uses_u64_replica() {
    use logos_core::crdt::GCounter;
    let counter = GCounter::new();
    // Must return u64 (this will fail to compile if still String)
    let id: u64 = counter.replica_id();
    let _ = id;
}

#[test]
fn test_gcounter_with_u64_replica_id() {
    use logos_core::crdt::GCounter;
    let counter = GCounter::with_replica_id(42u64);
    assert_eq!(counter.replica_id(), 42);
}

// ===== WAVE 1.2: VCLOCK =====

#[test]
fn test_vclock_new() {
    use logos_core::crdt::VClock;
    let clock = VClock::new();
    assert_eq!(clock.get(42), 0);
    assert_eq!(clock.get(999), 0);
}

#[test]
fn test_vclock_increment() {
    use logos_core::crdt::VClock;
    let mut clock = VClock::new();
    assert_eq!(clock.get(42), 0);
    let count = clock.increment(42);
    assert_eq!(count, 1);
    assert_eq!(clock.get(42), 1);

    let count2 = clock.increment(42);
    assert_eq!(count2, 2);
    assert_eq!(clock.get(42), 2);
}

#[test]
fn test_vclock_merge_commutative() {
    use logos_core::crdt::{Merge, VClock};
    let mut a = VClock::new();
    let mut b = VClock::new();
    a.increment(1);
    a.increment(1);
    b.increment(2);

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);
    assert_eq!(a1, b1); // Commutative
}

#[test]
fn test_vclock_merge_associative() {
    use logos_core::crdt::{Merge, VClock};
    let mut a = VClock::new();
    let mut b = VClock::new();
    let mut c = VClock::new();
    a.increment(1);
    b.increment(2);
    c.increment(3);

    // (a merge b) merge c
    let mut ab = a.clone();
    ab.merge(&b);
    let mut abc1 = ab.clone();
    abc1.merge(&c);

    // a merge (b merge c)
    let mut bc = b.clone();
    bc.merge(&c);
    let mut abc2 = a.clone();
    abc2.merge(&bc);

    assert_eq!(abc1, abc2); // Associative
}

#[test]
fn test_vclock_merge_idempotent() {
    use logos_core::crdt::{Merge, VClock};
    let mut a = VClock::new();
    a.increment(1);
    a.increment(1);

    let before = a.clone();
    a.merge(&before);
    assert_eq!(a, before); // Idempotent
}

#[test]
fn test_vclock_dominates() {
    use logos_core::crdt::VClock;
    let mut a = VClock::new();
    let mut b = VClock::new();
    a.increment(1);
    a.increment(1); // a[1] = 2
    b.increment(1); // b[1] = 1

    assert!(a.dominates(&b)); // a >= b for all replicas
    assert!(!b.dominates(&a)); // b < a for replica 1
}

#[test]
fn test_vclock_dominates_empty() {
    use logos_core::crdt::VClock;
    let a = VClock::new();
    let b = VClock::new();

    // Empty clocks dominate each other (equal)
    assert!(a.dominates(&b));
    assert!(b.dominates(&a));
}

#[test]
fn test_vclock_dominates_partial() {
    use logos_core::crdt::VClock;
    let mut a = VClock::new();
    let mut b = VClock::new();
    a.increment(1);
    a.increment(1); // a[1] = 2
    b.increment(1);
    b.increment(2); // b[1] = 1, b[2] = 1

    // Neither dominates: a has higher 1, b has 2 which a lacks
    assert!(!a.dominates(&b));
    assert!(!b.dominates(&a));
}

#[test]
fn test_vclock_concurrent() {
    use logos_core::crdt::VClock;
    let mut a = VClock::new();
    let mut b = VClock::new();
    a.increment(1);
    b.increment(2);

    assert!(a.concurrent(&b)); // Neither dominates
    assert!(b.concurrent(&a));
}

#[test]
fn test_vclock_not_concurrent() {
    use logos_core::crdt::VClock;
    let mut a = VClock::new();
    let mut b = VClock::new();
    a.increment(1);
    a.increment(1);
    b.increment(1);

    assert!(!a.concurrent(&b)); // a dominates b
    assert!(!b.concurrent(&a));
}

#[test]
fn test_vclock_serialization() {
    use logos_core::crdt::VClock;
    let mut clock = VClock::new();
    clock.increment(42);
    clock.increment(99);

    let bytes = bincode::serialize(&clock).unwrap();
    let decoded: VClock = bincode::deserialize(&bytes).unwrap();

    assert_eq!(clock, decoded);
}

// ===== WAVE 1.3: DOT =====

#[test]
fn test_dot_creation() {
    use logos_core::crdt::Dot;
    let dot = Dot::new(42, 1);
    assert_eq!(dot.replica, 42);
    assert_eq!(dot.counter, 1);
}

#[test]
fn test_dot_equality() {
    use logos_core::crdt::Dot;
    let a = Dot::new(1, 5);
    let b = Dot::new(1, 5);
    let c = Dot::new(1, 6);
    let d = Dot::new(2, 5);

    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
}

#[test]
fn test_dot_hash() {
    use logos_core::crdt::Dot;
    use std::collections::HashSet;

    let mut set = HashSet::new();
    set.insert(Dot::new(1, 1));
    set.insert(Dot::new(1, 2));
    set.insert(Dot::new(1, 1)); // Duplicate

    assert_eq!(set.len(), 2);
}

#[test]
fn test_dot_serialization() {
    use logos_core::crdt::Dot;
    let dot = Dot::new(42, 99);

    let bytes = bincode::serialize(&dot).unwrap();
    let decoded: Dot = bincode::deserialize(&bytes).unwrap();

    assert_eq!(dot, decoded);
}

// ===== WAVE 1.4: DOT CONTEXT =====

#[test]
fn test_dot_context_new() {
    use logos_core::crdt::DotContext;
    let ctx = DotContext::new();
    // Empty context
    assert!(!ctx.has_seen(&logos_core::crdt::Dot::new(1, 1)));
}

#[test]
fn test_dot_context_next() {
    use logos_core::crdt::DotContext;
    let mut ctx = DotContext::new();
    let d1 = ctx.next(42);
    let d2 = ctx.next(42);
    let d3 = ctx.next(99);

    assert_eq!(d1.replica, 42);
    assert_eq!(d1.counter, 1);
    assert_eq!(d2.replica, 42);
    assert_eq!(d2.counter, 2);
    assert_eq!(d3.replica, 99);
    assert_eq!(d3.counter, 1);
}

#[test]
fn test_dot_context_has_seen_after_next() {
    use logos_core::crdt::DotContext;
    let mut ctx = DotContext::new();
    let dot = ctx.next(42);

    assert!(ctx.has_seen(&dot));
    assert!(!ctx.has_seen(&logos_core::crdt::Dot::new(42, 99)));
}

#[test]
fn test_dot_context_add_contiguous() {
    use logos_core::crdt::{Dot, DotContext};
    let mut ctx = DotContext::new();

    // Add dots in order - should compact into clock
    ctx.add(Dot::new(1, 1));
    ctx.add(Dot::new(1, 2));
    ctx.add(Dot::new(1, 3));

    assert!(ctx.has_seen(&Dot::new(1, 1)));
    assert!(ctx.has_seen(&Dot::new(1, 2)));
    assert!(ctx.has_seen(&Dot::new(1, 3)));
    assert!(!ctx.has_seen(&Dot::new(1, 4)));
}

#[test]
fn test_dot_context_add_out_of_order() {
    use logos_core::crdt::{Dot, DotContext};
    let mut ctx = DotContext::new();

    // Add dots out of order - should go into cloud, then compact
    ctx.add(Dot::new(1, 3)); // Into cloud
    ctx.add(Dot::new(1, 1)); // Into clock
    ctx.add(Dot::new(1, 2)); // Should trigger compaction of 3

    assert!(ctx.has_seen(&Dot::new(1, 1)));
    assert!(ctx.has_seen(&Dot::new(1, 2)));
    assert!(ctx.has_seen(&Dot::new(1, 3)));
}

#[test]
fn test_dot_context_merge() {
    use logos_core::crdt::{Dot, DotContext};
    let mut a = DotContext::new();
    let mut b = DotContext::new();

    a.next(1);
    a.next(1); // a has seen (1,1), (1,2)
    b.next(2); // b has seen (2,1)

    a.merge(&b);

    assert!(a.has_seen(&Dot::new(1, 1)));
    assert!(a.has_seen(&Dot::new(1, 2)));
    assert!(a.has_seen(&Dot::new(2, 1)));
}

#[test]
fn test_dot_context_merge_commutative() {
    use logos_core::crdt::DotContext;
    let mut a = DotContext::new();
    let mut b = DotContext::new();

    a.next(1);
    a.next(1);
    b.next(2);

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);

    // Both should have seen the same dots
    assert!(a1.has_seen(&logos_core::crdt::Dot::new(1, 1)));
    assert!(a1.has_seen(&logos_core::crdt::Dot::new(1, 2)));
    assert!(a1.has_seen(&logos_core::crdt::Dot::new(2, 1)));
    assert!(b1.has_seen(&logos_core::crdt::Dot::new(1, 1)));
    assert!(b1.has_seen(&logos_core::crdt::Dot::new(1, 2)));
    assert!(b1.has_seen(&logos_core::crdt::Dot::new(2, 1)));
}

#[test]
fn test_dot_context_serialization() {
    use logos_core::crdt::DotContext;
    let mut ctx = DotContext::new();
    ctx.next(1);
    ctx.next(2);

    let bytes = bincode::serialize(&ctx).unwrap();
    let decoded: DotContext = bincode::deserialize(&bytes).unwrap();

    assert!(decoded.has_seen(&logos_core::crdt::Dot::new(1, 1)));
    assert!(decoded.has_seen(&logos_core::crdt::Dot::new(2, 1)));
}

// ===== WAVE 1.5: DELTA CRDT TRAIT =====

#[test]
fn test_delta_crdt_trait_exists() {
    use logos_core::crdt::DeltaCrdt;
    // Trait exists and can be used as bound
    fn assert_delta_crdt<T: DeltaCrdt>() {}
    // Will fail until trait is defined
}

// ===== WAVE 1.6: DELTA BUFFER =====

#[test]
fn test_delta_buffer_new() {
    use logos_core::crdt::{DeltaBuffer, VClock};
    let buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
    let empty = VClock::new();
    let deltas = buf.deltas_since(&empty);
    assert!(deltas.is_some());
    assert!(deltas.unwrap().is_empty());
}

#[test]
fn test_delta_buffer_push_and_retrieve() {
    use logos_core::crdt::{DeltaBuffer, VClock};
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
    let mut clock = VClock::new();
    clock.increment(1);
    buf.push(clock.clone(), 42);

    let empty = VClock::new();
    let deltas = buf.deltas_since(&empty).unwrap();
    assert_eq!(deltas, vec![42]);
}

#[test]
fn test_delta_buffer_multiple_deltas() {
    use logos_core::crdt::{DeltaBuffer, VClock};
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
    let mut clock = VClock::new();

    clock.increment(1);
    buf.push(clock.clone(), 1);
    clock.increment(1);
    buf.push(clock.clone(), 2);
    clock.increment(1);
    buf.push(clock.clone(), 3);

    let empty = VClock::new();
    let deltas = buf.deltas_since(&empty).unwrap();
    assert_eq!(deltas, vec![1, 2, 3]);
}

#[test]
fn test_delta_buffer_since_version() {
    use logos_core::crdt::{DeltaBuffer, VClock};
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
    let mut clock = VClock::new();

    clock.increment(1);
    let v1 = clock.clone();
    buf.push(clock.clone(), 1);

    clock.increment(1);
    buf.push(clock.clone(), 2);

    clock.increment(1);
    buf.push(clock.clone(), 3);

    // Get deltas since v1 - should only get 2 and 3
    let deltas = buf.deltas_since(&v1).unwrap();
    assert_eq!(deltas, vec![2, 3]);
}

#[test]
fn test_delta_buffer_overflow() {
    use logos_core::crdt::{DeltaBuffer, VClock};
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(2); // Only holds 2
    let mut clock = VClock::new();

    clock.increment(1);
    buf.push(clock.clone(), 1);
    clock.increment(1);
    buf.push(clock.clone(), 2);
    clock.increment(1);
    buf.push(clock.clone(), 3); // Evicts 1

    let empty = VClock::new();
    // Gap too large - oldest delta we have is after empty
    assert!(buf.deltas_since(&empty).is_none());
}

#[test]
fn test_delta_buffer_overflow_partial() {
    use logos_core::crdt::{DeltaBuffer, VClock};
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(2);
    let mut clock = VClock::new();

    clock.increment(1);
    let v1 = clock.clone();
    buf.push(clock.clone(), 1);

    clock.increment(1);
    let v2 = clock.clone();
    buf.push(clock.clone(), 2);

    clock.increment(1);
    buf.push(clock.clone(), 3); // Evicts 1

    // v1 is too old - can't provide deltas
    assert!(buf.deltas_since(&v1).is_none());

    // v2 is still in buffer - can provide 3
    let deltas = buf.deltas_since(&v2).unwrap();
    assert_eq!(deltas, vec![3]);
}
