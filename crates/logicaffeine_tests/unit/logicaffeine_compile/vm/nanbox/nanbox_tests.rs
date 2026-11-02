use super::*;
use crate::interpreter::{ListRepr, MapStorage};
use std::cell::RefCell;
use std::rc::Rc;

// The i64 edge grid mirrors value.rs's `fast_path_kernel_differential`.
// Note: i64::MIN/MAX and the ±2^47 boundary values straddle the inline range
// on purpose — the round-trip must be lossless for ALL of them.
const INT_EDGES: [i64; 16] = [
    i64::MIN,
    i64::MIN + 1,
    -4611686018427387904, // i64::MIN / 2
    INT_INLINE_MIN - 1,   // just below inline range → boxes
    INT_INLINE_MIN,       // inline boundary
    -1000003,
    -2,
    -1,
    0,
    1,
    2,
    1000003,
    INT_INLINE_MAX,      // inline boundary
    INT_INLINE_MAX + 1,  // just above inline range → boxes
    4611686018427387903, // i64::MAX / 2
    i64::MAX,
];

// The subset of INT_EDGES that stays inline — the only values the inline
// arith fast path applies to.
fn inline_edges() -> Vec<i64> {
    INT_EDGES
        .iter()
        .copied()
        .filter(|&n| (INT_INLINE_MIN..=INT_INLINE_MAX).contains(&n))
        .collect()
}

fn float_edges() -> Vec<f64> {
    vec![
        f64::NAN,
        -f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        -0.0,
        f64::MIN_POSITIVE,       // smallest normal
        f64::MIN_POSITIVE / 2.0, // a subnormal
        f64::MAX,
        f64::MIN,
        1.0,
        -1.0,
        0.1,
        std::f64::consts::PI,
        -2.5,
    ]
}

/// Round-trip equality. RuntimeValue's PartialEq has no arm for the
/// collection variants (they fall to `_ => false` — see interpreter.rs:376),
/// so we compare scalars by value (floats by bits, since NaN != NaN) and
/// collections/structs by their canonical display string.
fn assert_runtime_eq(a: &RuntimeValue, b: &RuntimeValue, ctx: &str) {
    match (a, b) {
        (RuntimeValue::Float(x), RuntimeValue::Float(y)) => assert_eq!(
            x.to_bits(),
            y.to_bits(),
            "{ctx}: float bits diverged ({x} vs {y})"
        ),
        (RuntimeValue::Int(_), _)
        | (RuntimeValue::Bool(_), _)
        | (RuntimeValue::Nothing, _)
        | (RuntimeValue::Char(_), _)
        | (RuntimeValue::Duration(_), _)
        | (RuntimeValue::Date(_), _)
        | (RuntimeValue::Moment(_), _)
        | (RuntimeValue::Span { .. }, _)
        | (RuntimeValue::Time(_), _)
        | (RuntimeValue::Text(_), _) => assert_eq!(a, b, "{ctx}: round-trip diverged"),
        // Collections/structs: PartialEq is structurally absent, so compare
        // type + display (deterministic for the VM/tree-walker).
        _ => {
            assert_eq!(a.type_name(), b.type_name(), "{ctx}: type diverged");
            assert_eq!(
                a.to_display_string(),
                b.to_display_string(),
                "{ctx}: display diverged"
            );
        }
    }
}

// ---- Test 4: size, the headline claim ---------------------------------

#[test]
fn nanbox_runtime_value_is_16_bytes() {
    // Documents WHY a 16-byte tagged union would be pointless: the enum it
    // would replace is ALREADY 16 bytes. The win must come from 8.
    assert_eq!(
        std::mem::size_of::<RuntimeValue>(),
        16,
        "RuntimeValue is expected to be 16 bytes"
    );
}

#[test]
fn nanbox_narrow_is_8_bytes() {
    // The whole point of WS-F: a register-file value HALF the width of the
    // fat enum. 8 = one machine word, the NaN-boxed u64.
    assert_eq!(std::mem::size_of::<Narrow>(), 8, "Narrow must be 8 bytes");
    assert_eq!(
        std::mem::align_of::<Narrow>(),
        8,
        "Narrow must be 8-aligned"
    );
    assert!(
        std::mem::size_of::<Narrow>() < std::mem::size_of::<RuntimeValue>(),
        "Narrow ({}) must be smaller than RuntimeValue ({})",
        std::mem::size_of::<Narrow>(),
        std::mem::size_of::<RuntimeValue>(),
    );
}

// ---- Test 1: exhaustive round-trip over every variant -----------------

#[test]
fn nanbox_int_round_trips_over_edge_grid() {
    for &n in &INT_EDGES {
        let rv = RuntimeValue::Int(n);
        let narrow = Narrow::from_runtime(rv.clone());
        // Lossless for ALL i64, inline or boxed.
        assert_eq!(narrow.as_int(), Some(n), "as_int({n})");
        let back = narrow.into_runtime();
        assert_runtime_eq(&rv, &back, &format!("Int({n})"));
        // The boundary values land where we claim.
        let inline = (INT_INLINE_MIN..=INT_INLINE_MAX).contains(&n);
        assert_eq!(
            Narrow::int(n).is_inline_int(),
            inline,
            "Int({n}) inline-classification"
        );
    }
}

#[test]
fn nanbox_float_round_trips_bit_exact_incl_nan_zero_inf_subnormal() {
    for f in float_edges() {
        let rv = RuntimeValue::Float(f);
        let n = Narrow::from_runtime(rv.clone());
        assert!(n.as_float().is_some(), "as_float present for {f}");
        // Bit-exactness (NaN canonicalises to a NaN; ±0.0, subnormal exact).
        let got = n.as_float().unwrap();
        if f.is_nan() {
            assert!(got.is_nan(), "NaN must round-trip as a NaN ({f})");
        } else {
            assert_eq!(got.to_bits(), f.to_bits(), "float bits for {f}");
        }
        let back = n.into_runtime();
        match (&rv, &back) {
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) if a.is_nan() => {
                assert!(b.is_nan(), "NaN round-trip")
            }
            _ => assert_runtime_eq(&rv, &back, &format!("Float({f})")),
        }
    }
}

#[test]
fn nanbox_bool_and_nothing_round_trip() {
    for b in [true, false] {
        let rv = RuntimeValue::Bool(b);
        let back = Narrow::from_runtime(rv.clone()).into_runtime();
        assert_runtime_eq(&rv, &back, &format!("Bool({b})"));
        assert_eq!(Narrow::bool(b).as_bool(), Some(b));
    }
    let back = Narrow::from_runtime(RuntimeValue::Nothing).into_runtime();
    assert_runtime_eq(&RuntimeValue::Nothing, &back, "Nothing");
    assert!(Narrow::nothing().is_nothing());
}

#[test]
fn nanbox_heap_types_round_trip_and_preserve_identity() {
    // Text.
    let text = RuntimeValue::Text(Rc::new("hello world".to_string()));
    let back = Narrow::from_runtime(text.clone()).into_runtime();
    assert_runtime_eq(&text, &back, "Text");

    // List (Ints repr) — reference identity preserved through the box.
    let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(vec![1, 2, 3]))));
    let narrow = Narrow::from_runtime(list.clone());
    assert!(narrow.is_heap());
    if let RuntimeValue::List(rc) = &list {
        rc.borrow_mut().push(RuntimeValue::Int(4));
    }
    if let Some(RuntimeValue::List(rc)) = narrow.as_heap() {
        assert_eq!(rc.borrow().len(), 4, "Narrow shares the original List's Rc");
    } else {
        panic!("heap payload was not a List");
    }
    let back = narrow.into_runtime();
    if let RuntimeValue::List(rc) = back {
        assert_eq!(rc.borrow().len(), 4);
    } else {
        panic!("List did not round-trip");
    }

    // Map.
    let mut storage = MapStorage::default();
    storage.insert(
        RuntimeValue::Int(7),
        RuntimeValue::Text(Rc::new("v".into())),
    );
    let map = RuntimeValue::Map(Rc::new(RefCell::new(storage)));
    let back = Narrow::from_runtime(map.clone()).into_runtime();
    if let (RuntimeValue::Map(a), RuntimeValue::Map(b)) = (&map, &back) {
        assert_eq!(a.borrow().len(), b.borrow().len());
    } else {
        panic!("Map did not round-trip");
    }

    // Char, the temporal scalars, and the other collections all live on the
    // heap path here.
    for rv in [
        RuntimeValue::Char('x'),
        RuntimeValue::Duration(42),
        RuntimeValue::Date(20260620),
        RuntimeValue::Moment(123456789),
        RuntimeValue::Span {
            months: 3,
            days: 14,
        },
        RuntimeValue::Time(987654321),
        RuntimeValue::Tuple(Rc::new(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Bool(true),
        ])),
        RuntimeValue::Set(Rc::new(RefCell::new(vec![RuntimeValue::Int(9)]))),
    ] {
        let back = Narrow::from_runtime(rv.clone()).into_runtime();
        assert_runtime_eq(&rv, &back, "heap-path scalar/collection");
    }
}

#[test]
fn nanbox_to_runtime_matches_into_runtime_without_consuming() {
    let cases = [
        RuntimeValue::Int(-99),
        RuntimeValue::Int(i64::MAX), // boxed large int
        RuntimeValue::Float(std::f64::consts::E),
        RuntimeValue::Bool(true),
        RuntimeValue::Nothing,
        RuntimeValue::Text(Rc::new("abc".into())),
    ];
    for rv in cases {
        let n = Narrow::from_runtime(rv.clone());
        let borrowed = n.to_runtime();
        let consumed = n.into_runtime();
        assert_runtime_eq(&borrowed, &consumed, "to_runtime vs into_runtime");
        assert_runtime_eq(&borrowed, &rv, "to_runtime vs original");
    }
}

// ---- Test 2: arithmetic parity with the kernel ------------------------

#[test]
fn nanbox_int_arith_is_bit_identical_to_kernel() {
    use crate::ast::stmt::BinaryOpKind;
    use crate::semantics::{arith, compare};
    let edges = inline_edges();
    for &a in &edges {
        for &b in &edges {
            let (na, nb) = (Narrow::int(a), Narrow::int(b));

            let add = na.int_add(&nb).unwrap().into_runtime();
            assert_eq!(
                add,
                arith::add(RuntimeValue::Int(a), RuntimeValue::Int(b)).unwrap(),
                "add({a},{b})"
            );
            let sub = na.int_sub(&nb).unwrap().into_runtime();
            assert_eq!(
                sub,
                arith::subtract(RuntimeValue::Int(a), RuntimeValue::Int(b)).unwrap(),
                "sub({a},{b})"
            );
            let mul = na.int_mul(&nb).unwrap().into_runtime();
            assert_eq!(
                mul,
                arith::multiply(RuntimeValue::Int(a), RuntimeValue::Int(b)).unwrap(),
                "mul({a},{b})"
            );

            let lt = na.int_lt(&nb).unwrap();
            let lt_kernel = compare::compare(
                BinaryOpKind::Lt,
                &RuntimeValue::Int(a),
                &RuntimeValue::Int(b),
            )
            .unwrap();
            assert_eq!(RuntimeValue::Bool(lt), lt_kernel, "lt({a},{b})");

            let eq = na.int_eq(&nb).unwrap();
            assert_eq!(
                eq,
                compare::values_equal(&RuntimeValue::Int(a), &RuntimeValue::Int(b)),
                "eq({a},{b})"
            );
        }
    }
}

#[test]
fn nanbox_arith_wrap_past_inline_range_stays_lossless() {
    // INT_INLINE_MAX + INT_INLINE_MAX overflows the inline range; the result
    // must re-box and still equal the kernel's wrapping-i64 answer.
    use crate::semantics::arith;
    let big = INT_INLINE_MAX;
    let n = Narrow::int(big).int_add(&Narrow::int(big)).unwrap();
    assert!(!n.is_inline_int(), "out-of-range sum must box");
    assert_eq!(
        n.into_runtime(),
        arith::add(RuntimeValue::Int(big), RuntimeValue::Int(big)).unwrap(),
        "wrapped sum equals the kernel"
    );
}

#[test]
fn nanbox_non_inline_int_operands_do_not_take_the_int_fast_path() {
    let i = Narrow::int(2);
    let f = Narrow::float(0.5);
    let t = Narrow::from_runtime(RuntimeValue::Text(Rc::new("x".into())));
    let big = Narrow::int(i64::MAX); // boxed int — must NOT take inline path
    assert!(i.int_add(&f).is_none(), "Int+Float must not take int path");
    assert!(f.int_add(&i).is_none());
    assert!(i.int_mul(&t).is_none(), "Int*Text must not take int path");
    assert!(
        i.int_add(&big).is_none(),
        "inline+boxed must not take inline path"
    );
    assert_eq!(i.int_lt(&f), None);
    assert_eq!(i.int_eq(&f), None);
}

// ---- Test 3: the NaN-boxing hazard, designed out ----------------------

#[test]
fn nanbox_nan_payload_is_not_aliased_with_heap() {
    // The classic NaN-boxing trap: a genuine f64::NAN must be a float, never
    // mistaken for a boxed/tagged value. We canonicalise NaN to a pattern
    // outside the tag region, so:
    let real_nan = Narrow::float(f64::NAN);
    assert!(
        real_nan.as_float().unwrap().is_nan(),
        "real NaN reads as a NaN float"
    );
    assert!(
        !real_nan.is_heap(),
        "a real NaN must not look like a heap value"
    );
    assert!(
        real_nan.as_int().is_none(),
        "a real NaN must not read back as Int"
    );
    assert!(real_nan.as_bool().is_none());
    assert!(!real_nan.is_nothing());
    assert!(real_nan.as_heap().is_none());

    // A boxed value is unambiguously Heap and never reads as a float.
    let heap = Narrow::from_runtime(RuntimeValue::Text(Rc::new("nan".into())));
    assert!(heap.is_heap());
    assert!(
        heap.as_float().is_none(),
        "a heap value must never read as Float"
    );
    assert!(heap.as_heap().is_some());

    // Distinct NaN bit-patterns (signaling, negative) all canonicalise to a
    // NaN that round-trips AS a NaN — and crucially never collide with the
    // tag region (which would make them read as Int/Bool/Heap).
    for bits in [
        0x7FF0_0000_0000_0001u64,
        0xFFF0_0000_0000_0001,
        0x7FF8_0000_0000_0000,
    ] {
        let f = f64::from_bits(bits);
        assert!(f.is_nan());
        let n = Narrow::float(f);
        assert!(
            n.as_float().unwrap().is_nan(),
            "{bits:#x} round-trips as NaN"
        );
        assert!(n.as_int().is_none() && n.as_bool().is_none() && !n.is_heap());
    }

    // +inf / -inf are NOT NaN and live on the plain float path untouched.
    assert_eq!(Narrow::float(f64::INFINITY).as_float(), Some(f64::INFINITY));
    assert_eq!(
        Narrow::float(f64::NEG_INFINITY).as_float(),
        Some(f64::NEG_INFINITY)
    );
    assert!(!Narrow::float(f64::INFINITY).is_heap());

    // The all-bits-set integer (Int(-1)) is the worst case for collision:
    // its 48-bit payload is all-ones. It must NOT alias CANONICAL_NAN — it
    // is `0xFFF8_FFFF_FFFF_FFFF` (high nibble 0xFFF8), an inline Int, and
    // reads back as -1, not as a NaN float.
    let minus_one = Narrow::int(-1);
    assert_eq!(minus_one.0, 0xFFF8_FFFF_FFFF_FFFF, "Int(-1) bit pattern");
    assert_ne!(minus_one.0, CANONICAL_NAN, "Int(-1) must not alias the NaN");
    assert_eq!(minus_one.as_int(), Some(-1));
    assert!(minus_one.as_float().is_none());

    // Exhaustively: no inline scalar of any tag, with any 48-bit payload
    // extreme, can reach CANONICAL_NAN — the high nibble is bounded by tag.
    for tag in 0u64..=3 {
        let hi = QNAN | (tag << TAG_SHIFT) | PAYLOAD_MASK;
        assert!(
            hi < CANONICAL_NAN,
            "tag {tag} max value must stay below the NaN"
        );
    }
}

// ---- Memory-safety stress: Drop/Clone over the heap path --------------

#[test]
fn nanbox_clone_and_drop_are_memory_safe_for_heap() {
    let rc = Rc::new(RefCell::new(ListRepr::Ints(vec![1, 2, 3])));
    let outer = Rc::clone(&rc); // strong = 2
    let n1 = Narrow::from_runtime(RuntimeValue::List(Rc::clone(&rc))); // strong = 3
    assert_eq!(Rc::strong_count(&rc), 3);

    let n2 = n1.clone(); // RuntimeValue::List clone bumps the Rc → strong = 4
    assert_eq!(Rc::strong_count(&rc), 4);

    drop(n2); // → 3
    assert_eq!(Rc::strong_count(&rc), 3);

    let _ = n1.into_runtime(); // moves box out, drops the List → 2
    assert_eq!(Rc::strong_count(&rc), 2);

    drop(outer); // → 1
    assert_eq!(Rc::strong_count(&rc), 1);
}

#[test]
fn nanbox_boxed_large_int_is_memory_safe_through_clone_and_drop() {
    // A large int boxes; clone allocates a fresh box, drop frees exactly
    // once. Miri-style discipline; here we just exercise the path heavily.
    let n = Narrow::int(i64::MIN);
    assert!(!n.is_inline_int());
    let c = n.clone();
    assert_eq!(c.as_int(), Some(i64::MIN));
    assert_eq!(n.as_int(), Some(i64::MIN));
    drop(c);
    assert_eq!(n.into_runtime(), RuntimeValue::Int(i64::MIN));
}

#[test]
fn nanbox_scalar_clone_is_a_value_copy() {
    let a = Narrow::int(123);
    let b = a.clone();
    assert_eq!(a.as_int(), b.as_int());
    let c = Narrow::float(f64::NAN);
    let d = c.clone();
    assert!(c.as_float().unwrap().is_nan() && d.as_float().unwrap().is_nan());
    let e = Narrow::nothing();
    assert!(e.clone().is_nothing());
}
