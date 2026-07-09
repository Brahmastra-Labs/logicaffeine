//! ════════════════════════════════════════════════════════════════════════════════════════════
//! WIRE Rc-DEDUP (G8) — a subtree the SAME `Rc` reaches more than once ships ONCE, and the decoder
//! rebuilds the SHARING (one `Rc`, aliased), not N independent copies. Off by default (every existing
//! byte-stream is untouched); a value with no actual sharing is byte-identical even with the knob on.
//! This is the "all-types completeness" tail: capnp/protobuf/bincode all explode a shared subtree
//! into N copies; we ship the generator of the structure — the reference graph itself.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_compile::concurrency::marshal::{
    message_from_wire, message_to_wire_with, with_dedup, WireCodec, WireIntegrity,
};
use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue};

fn enc(v: &RuntimeValue) -> Vec<u8> {
    message_to_wire_with("", v, WireCodec::Native, WireIntegrity::Raw).unwrap()
}

#[test]
fn dedup_ships_a_shared_subtree_once_and_rebuilds_the_sharing() {
    // Three references to the SAME 500-int list. Without dedup the list ships 3×; with dedup it ships
    // once + two backrefs — far smaller — and the decoded value re-aliases ONE Rc (sharing preserved).
    let inner = Rc::new(RefCell::new(ListRepr::Ints((0..500).collect())));
    let outer = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
        RuntimeValue::List(inner.clone()),
        RuntimeValue::List(inner.clone()),
        RuntimeValue::List(inner.clone()),
    ]))));

    let plain = enc(&outer);
    let deduped = with_dedup(true, || enc(&outer));
    assert!(
        deduped.len() * 2 < plain.len(),
        "dedup ships the shared 500-int list once + backrefs: {} vs {} bytes",
        deduped.len(),
        plain.len()
    );

    let (_f, back) = message_from_wire(&deduped).expect("deduped message decodes");
    let RuntimeValue::List(l) = back else { panic!("expected a List") };
    let items = match &*l.borrow() {
        ListRepr::Boxed(v) => v.clone(),
        other => panic!("expected Boxed, got {other:?}"),
    };
    assert_eq!(items.len(), 3, "all three references survive");
    for it in &items {
        match it {
            RuntimeValue::List(li) => match &*li.borrow() {
                ListRepr::Ints(g) => assert_eq!(g.len(), 500, "each is the 500-int list"),
                other => panic!("expected Ints, got {other:?}"),
            },
            other => panic!("expected List, got {other:?}"),
        }
    }
    // The decoded elements ALIAS one Rc — the sharing survived the wire (not three heap copies).
    let (RuntimeValue::List(ra), RuntimeValue::List(rb), RuntimeValue::List(rc)) =
        (&items[0], &items[1], &items[2])
    else {
        panic!("expected three Lists");
    };
    assert!(Rc::ptr_eq(ra, rb) && Rc::ptr_eq(rb, rc), "dedup rebuilt the aliasing into ONE Rc");
}

#[test]
fn dedup_is_byte_identical_when_there_is_no_sharing() {
    // Distinct Rcs (no aliasing) → dedup emits no def/ref tags → byte-for-byte the same as the default.
    let outer = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(vec![1, 2, 3])))),
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(vec![4, 5, 6])))),
        RuntimeValue::Text(Rc::new("hello".to_string())),
        RuntimeValue::Int(42),
    ]))));
    let plain = enc(&outer);
    let deduped = with_dedup(true, || enc(&outer));
    assert_eq!(deduped, plain, "no sharing → dedup changes nothing on the wire");
    assert!(message_from_wire(&deduped).is_some(), "and it still round-trips");
}

#[test]
fn dedup_shares_a_repeated_value_in_a_heterogeneous_list() {
    // A 2000-char string referenced several times inside a HETEROGENEOUS list — so it flows through
    // the general per-element encoder, NOT the string-column dictionary (which already dedups a
    // homogeneous string column). This is the case Rc-dedup uniquely owns: a shared value spread
    // through mixed structure ships ONCE, and the decoded copies re-alias one Rc.
    let s = Rc::new("x".repeat(2000));
    let outer = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
        RuntimeValue::Text(s.clone()),
        RuntimeValue::Int(0),
        RuntimeValue::Text(s.clone()),
        RuntimeValue::Int(1),
        RuntimeValue::Text(s.clone()),
    ]))));
    let plain = enc(&outer);
    let deduped = with_dedup(true, || enc(&outer));
    assert!(
        deduped.len() * 2 < plain.len(),
        "the shared 2000-char string ships once: {} vs {} bytes",
        deduped.len(),
        plain.len()
    );
    let (_f, back) = message_from_wire(&deduped).expect("decodes");
    let RuntimeValue::List(l) = back else { panic!("expected List") };
    let items = match &*l.borrow() {
        ListRepr::Boxed(v) => v.clone(),
        other => panic!("expected Boxed, got {other:?}"),
    };
    assert_eq!(items.len(), 5);
    let (RuntimeValue::Text(a), RuntimeValue::Text(b)) = (&items[0], &items[4]) else {
        panic!("expected Texts at positions 0 and 4");
    };
    assert_eq!(a.len(), 2000, "the full 2000-char string survives");
    assert!(Rc::ptr_eq(a, b), "the shared string rebuilt as ONE Rc");
}

#[test]
fn dedup_decode_is_panic_safe_on_truncation() {
    // Every truncation of a deduped message decodes to a clean `None` (or a value) — never a panic,
    // never a hang. A dangling backref resolves to `None` by construction (no entry in the memo).
    let inner = Rc::new(RefCell::new(ListRepr::Ints((0..200).collect())));
    let outer = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
        RuntimeValue::List(inner.clone()),
        RuntimeValue::List(inner.clone()),
        RuntimeValue::List(inner.clone()),
    ]))));
    let deduped = with_dedup(true, || enc(&outer));
    for cut in 0..deduped.len() {
        let _ = message_from_wire(&deduped[..cut]); // must not panic
    }
    // Every single-byte mutation likewise stays panic-safe (covers a corrupted ref id / def id).
    for i in 0..deduped.len() {
        let mut m = deduped.clone();
        m[i] ^= 0xFF;
        let _ = message_from_wire(&m);
    }
}

#[test]
fn best_and_auto_fold_in_dedup_when_sharing_is_present() {
    // The no-brainer paths (`best`/`smallest`/Auto) now CONSIDER dedup automatically whenever the value
    // aliases a subtree — you don't have to ask for the knob. Two guarantees:
    //   1. `best` is never worse than the dedup knob alone (dedup is a candidate in the search);
    //   2. when compression is off (a peer that can't inflate), dedup IS the crush — `best` uses it and
    //      rebuilds the SHARING (proving dedup, not compression, did the work).
    use logicaffeine_compile::concurrency::marshal::{
        message_to_wire, message_to_wire_best, message_to_wire_negotiated, with_dedup, Negotiated,
        WireCompression, WireGoal, WireTypeRegistry,
    };

    let inner = Rc::new(RefCell::new(ListRepr::Ints((0..500).collect())));
    let outer = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..8).map(|_| RuntimeValue::List(inner.clone())).collect(),
    ))));
    let plain = message_to_wire("p", &outer).unwrap();
    let dedup_only = with_dedup(true, || message_to_wire("p", &outer)).unwrap();

    let best = message_to_wire_best("p", &outer, WireGoal::Smallest).unwrap();
    assert!(
        best.len() <= dedup_only.len(),
        "best ({}) folds dedup into the search, so it's never worse than the dedup knob alone ({})",
        best.len(),
        dedup_only.len()
    );
    assert!(message_from_wire(&best).is_some(), "best round-trips");

    // Compression off → dedup is the only way to crush the 8 aliased lists.
    let neg = Negotiated {
        use_type_id: false,
        may_send_computed: false,
        compression: WireCompression::None,
        peer_max_bytes: 1 << 20,
    };
    let nbytes =
        message_to_wire_negotiated("p", &outer, &neg, WireTypeRegistry::new(Vec::new())).unwrap();
    assert!(
        nbytes.len() * 2 < plain.len(),
        "no-compression `best` auto-dedups the shared list: {} vs plain {}",
        nbytes.len(),
        plain.len()
    );
    let (_f, back) = message_from_wire(&nbytes).expect("decodes");
    let RuntimeValue::List(l) = back else { panic!("expected List") };
    let items = match &*l.borrow() {
        ListRepr::Boxed(v) => v.clone(),
        other => panic!("expected Boxed, got {other:?}"),
    };
    let (RuntimeValue::List(a), RuntimeValue::List(b)) = (&items[0], &items[7]) else {
        panic!("expected Lists");
    };
    assert!(Rc::ptr_eq(a, b), "best auto-dedup rebuilt the aliasing under no compression");
}
