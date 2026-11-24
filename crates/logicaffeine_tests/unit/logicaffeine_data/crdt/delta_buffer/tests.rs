use super::*;

#[test]
fn test_buffer_new() {
    let buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
    assert!(buf.is_empty());
}

#[test]
fn test_buffer_push_and_retrieve() {
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
    let mut clock = VClock::new();
    clock.increment(1);
    buf.push(clock, 42);

    let empty = VClock::new();
    let deltas = buf.deltas_since(&empty).unwrap();
    assert_eq!(deltas, vec![42]);
}

#[test]
fn test_buffer_multiple_deltas() {
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
fn test_buffer_since_version() {
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
fn test_buffer_overflow() {
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(2);
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
fn test_buffer_overflow_partial() {
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

// A peer on another replica's line has seen none of the evicted deltas, so the buffer cannot catch
// it up: it must fall back to full state rather than receive part of the history.
#[test]
fn a_concurrent_peer_is_not_served_after_an_eviction() {
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(1);
    let mut a1 = VClock::new();
    a1.increment(1);
    let mut a2 = a1.clone();
    a2.increment(1);
    buf.push(a1, 1);
    buf.push(a2, 2); // evicts delta 1
    let mut b3 = VClock::new();
    for _ in 0..3 {
        b3.increment(2);
    }
    assert_eq!(buf.deltas_since(&b3), None, "a peer that never saw delta 1 was served without it");
    assert!(!buf.can_serve(&b3));
}

#[test]
fn a_peer_past_the_evicted_delta_is_still_served() {
    let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(1);
    let mut a1 = VClock::new();
    a1.increment(1);
    let mut a2 = a1.clone();
    a2.increment(1);
    buf.push(a1.clone(), 1);
    buf.push(a2, 2);
    let mut peer = a1;
    peer.increment(2); // saw delta 1, and wrote on its own line
    assert_eq!(buf.deltas_since(&peer), Some(vec![2]));
}
