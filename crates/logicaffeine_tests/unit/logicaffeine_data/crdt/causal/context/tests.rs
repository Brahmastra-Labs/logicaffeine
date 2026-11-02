use super::*;

#[test]
fn test_context_new() {
    let ctx = DotContext::new();
    assert!(!ctx.has_seen(&Dot::new(1, 1)));
}

#[test]
fn test_context_next() {
    let mut ctx = DotContext::new();
    let d1 = ctx.next(42);
    let d2 = ctx.next(42);
    assert_eq!(d1.counter, 1);
    assert_eq!(d2.counter, 2);
    assert!(ctx.has_seen(&d1));
    assert!(ctx.has_seen(&d2));
}

#[test]
fn test_context_add_contiguous() {
    let mut ctx = DotContext::new();
    ctx.add(Dot::new(1, 1));
    ctx.add(Dot::new(1, 2));
    ctx.add(Dot::new(1, 3));

    assert!(ctx.has_seen(&Dot::new(1, 1)));
    assert!(ctx.has_seen(&Dot::new(1, 2)));
    assert!(ctx.has_seen(&Dot::new(1, 3)));
    assert!(!ctx.has_seen(&Dot::new(1, 4)));
}

#[test]
fn test_context_add_out_of_order() {
    let mut ctx = DotContext::new();
    ctx.add(Dot::new(1, 3)); // Into cloud
    ctx.add(Dot::new(1, 1)); // Into clock
    ctx.add(Dot::new(1, 2)); // Should trigger compaction

    assert!(ctx.has_seen(&Dot::new(1, 1)));
    assert!(ctx.has_seen(&Dot::new(1, 2)));
    assert!(ctx.has_seen(&Dot::new(1, 3)));
    // Cloud should be empty after compaction
    assert!(ctx.cloud.is_empty());
}

#[test]
fn test_context_merge() {
    let mut a = DotContext::new();
    let mut b = DotContext::new();

    a.next(1);
    a.next(1);
    b.next(2);

    a.merge(&b);

    assert!(a.has_seen(&Dot::new(1, 1)));
    assert!(a.has_seen(&Dot::new(1, 2)));
    assert!(a.has_seen(&Dot::new(2, 1)));
}
