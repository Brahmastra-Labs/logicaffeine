//! Phase R1: `RuntimeValue::Rational` as a first-class exact value. Arithmetic on
//! Rational operands is exact (an integer-valued result downsizes back to `Int`),
//! a Float operand makes the expression Float, and eq/compare/display are exact.
//! (The `Int / Int → Rational` flip itself is Phase R2 — here `/` still truncates,
//! so these tests construct Rationals directly via `from_rational`.)
use super::*;
use crate::interpreter::StructValue;
use logicaffeine_base::Rational;
use std::collections::HashMap;

fn rat(n: i64, d: i64) -> RuntimeValue {
    RuntimeValue::from_rational(Rational::from_ratio_i64(n, d).unwrap())
}
fn int(n: i64) -> RuntimeValue {
    RuntimeValue::Int(n)
}
fn is_rat(v: &RuntimeValue) -> bool {
    matches!(v, RuntimeValue::Rational(_))
}

#[test]
fn from_rational_downsizes_whole_values_to_int() {
    // 6/2 reduces to the whole number 3 → Int, NOT a Rational.
    assert_eq!(rat(6, 2), int(3));
    assert!(!is_rat(&rat(6, 2)));
    assert_eq!(rat(0, 5), int(0));
    // 7/2 is not whole → stays a Rational, displayed as a fraction.
    assert!(is_rat(&rat(7, 2)));
    assert_eq!(rat(7, 2).to_display_string(), "7/2");
    assert_eq!(rat(7, 2).type_name(), "Rational");
}

#[test]
fn rational_arithmetic_is_exact_and_downsizes() {
    // 1/2 + 1/2 = 1 (downsizes to Int).
    assert_eq!(add(rat(1, 2), rat(1, 2)).unwrap(), int(1));
    // 1/3 + 1/6 = 1/2 (stays exact).
    assert_eq!(
        add(rat(1, 3), rat(1, 6)).unwrap().to_display_string(),
        "1/2"
    );
    // 1/2 - 1/3 = 1/6 ; 2/3 * 3/4 = 1/2 ; (1/2)/(3/4) = 2/3.
    assert_eq!(
        subtract(rat(1, 2), rat(1, 3)).unwrap().to_display_string(),
        "1/6"
    );
    assert_eq!(
        multiply(rat(2, 3), rat(3, 4)).unwrap().to_display_string(),
        "1/2"
    );
    assert_eq!(
        divide(rat(1, 2), rat(3, 4)).unwrap().to_display_string(),
        "2/3"
    );
}

#[test]
fn rational_mixes_with_int_and_bigint_exactly() {
    // 1/2 + 1 = 3/2 ; 3 * (1/2) = 3/2 ; (3/2) / 3 = 1/2.
    assert_eq!(add(rat(1, 2), int(1)).unwrap().to_display_string(), "3/2");
    assert_eq!(
        multiply(int(3), rat(1, 2)).unwrap().to_display_string(),
        "3/2"
    );
    assert_eq!(divide(rat(3, 2), int(3)).unwrap(), rat(1, 2));
    // 1/3 + 2/3 = 1 (Int).
    assert_eq!(add(rat(1, 3), rat(2, 3)).unwrap(), int(1));
}

#[test]
fn rational_with_a_float_operand_becomes_float() {
    // 1/2 + 0.5 → Float 1.0 (a Float operand opts the whole expression into floats).
    assert!(
        matches!(add(rat(1, 2), RuntimeValue::Float(0.5)).unwrap(), RuntimeValue::Float(f) if (f - 1.0).abs() < 1e-12)
    );
    assert!(
        matches!(divide(rat(1, 2), RuntimeValue::Float(2.0)).unwrap(), RuntimeValue::Float(f) if (f - 0.25).abs() < 1e-12)
    );
}

#[test]
fn rational_equality_and_ordering_are_exact() {
    // Never equal to an Int; equal to a structurally-equal Rational.
    assert!(!values_equal(&rat(7, 2), &int(3)));
    assert!(values_equal(&rat(7, 2), &rat(7, 2)));
    // 1/3 < 1/2 < 2/3 by exact cross-multiplication (no rounding).
    let lt = |a, b| {
        matches!(
            compare(BinaryOpKind::Lt, &a, &b).unwrap(),
            RuntimeValue::Bool(true)
        )
    };
    assert!(lt(rat(1, 3), rat(1, 2)));
    assert!(lt(rat(1, 2), rat(2, 3)));
    // 1/2 < 1 (Int) and 0 < 1/2.
    assert!(lt(rat(1, 2), int(1)));
    assert!(lt(int(0), rat(1, 2)));
}

#[test]
fn dividing_a_rational_by_zero_errors() {
    assert_eq!(divide(rat(1, 2), int(0)).unwrap_err(), "Division by zero");
    assert_eq!(
        divide(rat(1, 2), rat(0, 1)).unwrap_err(),
        "Division by zero"
    );
}

// ===== G7: the CRDT laws, fuzzed to absurdity =====
// A state-based CRDT (CvRDT) converges iff its merge is COMMUTATIVE, ASSOCIATIVE, and
// IDEMPOTENT. We prove all three for set-union and the map-of-sets join over thousands of
// random states, then prove the corollary that matters on a real network: replicas reach
// the same value no matter the ORDER or DUPLICATION of merges (a gossip/lossy link).

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn upto(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Structural CRDT equality: sets compared AS SETS (order-independent), maps AS MAPS,
/// scalars directly — the right notion of "converged to the same state".
fn crdt_eq(a: &RuntimeValue, b: &RuntimeValue) -> bool {
    match (a, b) {
        (RuntimeValue::Int(x), RuntimeValue::Int(y)) => x == y,
        (RuntimeValue::Text(x), RuntimeValue::Text(y)) => x == y,
        (RuntimeValue::Bool(x), RuntimeValue::Bool(y)) => x == y,
        (RuntimeValue::Nothing, RuntimeValue::Nothing) => true,
        (RuntimeValue::Set(x), RuntimeValue::Set(y)) => {
            let (xb, yb) = (x.borrow(), y.borrow());
            xb.len() == yb.len() && xb.iter().all(|e| yb.iter().any(|f| crdt_eq(e, f)))
        }
        (RuntimeValue::Map(x), RuntimeValue::Map(y)) => {
            let (xb, yb) = (x.borrow(), y.borrow());
            xb.len() == yb.len()
                && xb
                    .iter()
                    .all(|(k, v)| yb.get(k).map_or(false, |w| crdt_eq(v, w)))
        }
        (RuntimeValue::Struct(x), RuntimeValue::Struct(y)) => {
            x.type_name == y.type_name
                && x.fields.len() == y.fields.len()
                && x.fields
                    .iter()
                    .all(|(k, v)| y.fields.get(k).map_or(false, |w| crdt_eq(v, w)))
        }
        _ => false,
    }
}

fn rand_gcounter(rng: &mut Rng) -> RuntimeValue {
    let mut fields: HashMap<String, RuntimeValue> = HashMap::new();
    for _ in 0..rng.upto(4) {
        let r = format!("r{}", rng.upto(4));
        let c = rng.upto(20) as i64;
        let cur = if let Some(RuntimeValue::Int(x)) = fields.get(&r) {
            *x
        } else {
            0
        };
        fields.insert(r, RuntimeValue::Int(cur.max(c)));
    }
    RuntimeValue::Struct(Box::new(StructValue {
        type_name: GCOUNTER_TAG.to_string(),
        fields,
    }))
}

fn gcounter(pairs: &[(&str, i64)]) -> RuntimeValue {
    let mut fields = HashMap::new();
    for (r, c) in pairs {
        fields.insert(r.to_string(), RuntimeValue::Int(*c));
    }
    RuntimeValue::Struct(Box::new(StructValue {
        type_name: GCOUNTER_TAG.to_string(),
        fields,
    }))
}

#[test]
fn gcounter_is_gossip_safe_max_per_replica_and_idempotent_under_redelivery() {
    // The fix for the #1 gap: per-replica MAX, not op-based add. A REDELIVERED counter
    // state must not double-count — the crux the old `add` counter fails on a lossy link.
    let a = gcounter(&[("alice", 3), ("bob", 1)]);
    let b = gcounter(&[("alice", 2), ("carol", 5)]);
    let ab = crdt_merge_field(&a, b.clone());
    assert_eq!(
        gcounter_value(&ab),
        Some(3 + 1 + 5),
        "MAX per replica → alice=3, bob=1, carol=5"
    );
    let abb = crdt_merge_field(&ab, b.clone());
    assert_eq!(
        gcounter_value(&abb),
        Some(9),
        "REDELIVERY is a no-op — never double-counts"
    );
    assert!(
        crdt_eq(&ab, &abb),
        "the state is unchanged under redelivery (idempotent)"
    );
}

#[test]
fn gcounter_merge_obeys_the_crdt_laws() {
    assert_crdt_laws(&mut Rng::new(0x06C0_0117), rand_gcounter, "g-counter");
}

#[test]
fn gcounter_replicas_converge_under_any_order_and_duplication() {
    // Replicas increment independently, gossip in shuffled+duplicated order, and every
    // one ends at the same total — a genuinely gossip-safe distributed counter.
    let mut rng = Rng::new(0x06C0_F1A6);
    for _ in 0..500 {
        let states: Vec<RuntimeValue> = (0..4).map(|_| rand_gcounter(&mut rng)).collect();
        let mut truth = states[0].clone();
        for s in &states[1..] {
            truth = crdt_merge_field(&truth, s.clone());
        }
        let mut deliveries: Vec<usize> = (0..states.len()).chain(0..states.len()).collect();
        for i in (1..deliveries.len()).rev() {
            let j = rng.upto((i + 1) as u64) as usize;
            deliveries.swap(i, j);
        }
        let mut replica = states[rng.upto(states.len() as u64) as usize].clone();
        for &d in &deliveries {
            replica = crdt_merge_field(&replica, states[d].clone());
        }
        assert!(
            crdt_eq(&replica, &truth),
            "g-counter replicas converge despite order/duplication"
        );
        assert_eq!(
            gcounter_value(&replica),
            gcounter_value(&truth),
            "and agree on the total"
        );
    }
}

fn set_of(items: &[i64]) -> RuntimeValue {
    RuntimeValue::Set(std::rc::Rc::new(std::cell::RefCell::new(
        items.iter().map(|&i| RuntimeValue::Int(i)).collect(),
    )))
}

fn rand_set(rng: &mut Rng) -> RuntimeValue {
    let mut s = set_of(&[]);
    for _ in 0..rng.upto(6) {
        s = crate::semantics::collections::union(&s, &set_of(&[rng.upto(8) as i64])).unwrap();
    }
    s
}

fn rand_map_of_sets(rng: &mut Rng) -> RuntimeValue {
    let mut m = crate::interpreter::MapStorage::default();
    for _ in 0..rng.upto(5) {
        m.insert(RuntimeValue::Int(rng.upto(5) as i64), rand_set(rng));
    }
    RuntimeValue::Map(std::rc::Rc::new(std::cell::RefCell::new(m)))
}

fn assert_crdt_laws(rng: &mut Rng, gen: impl Fn(&mut Rng) -> RuntimeValue, what: &str) {
    for _ in 0..2000 {
        let (a, b, c) = (gen(rng), gen(rng), gen(rng));
        assert!(
            crdt_eq(
                &crdt_merge_field(&a, b.clone()),
                &crdt_merge_field(&b, a.clone())
            ),
            "{what} merge must be COMMUTATIVE"
        );
        assert!(
            crdt_eq(&crdt_merge_field(&a, a.clone()), &a),
            "{what} merge must be IDEMPOTENT"
        );
        let ab = crdt_merge_field(&a, b.clone());
        assert!(
            crdt_eq(&crdt_merge_field(&ab, b.clone()), &ab),
            "{what}: re-merging a seen value is a no-op"
        );
        let l = crdt_merge_field(&crdt_merge_field(&a, b.clone()), c.clone());
        let r = crdt_merge_field(&a, crdt_merge_field(&b, c.clone()));
        assert!(crdt_eq(&l, &r), "{what} merge must be ASSOCIATIVE");
    }
}

#[test]
fn crdt_set_merge_obeys_the_crdt_laws() {
    assert_crdt_laws(&mut Rng::new(0x00C0_FFEE), rand_set, "set");
}

#[test]
fn crdt_map_of_sets_merge_obeys_the_crdt_laws() {
    // A CRDT map of CRDT sets — "shared memory" over the network — is itself a CRDT.
    assert_crdt_laws(&mut Rng::new(0x0000_BEEF), rand_map_of_sets, "map-of-sets");
}

#[test]
fn crdt_replicas_converge_under_any_order_and_duplication() {
    // The property that matters on a real gossip/lossy network: a replica that receives
    // every state in a SHUFFLED order, some delivered TWICE, still ends at the exact LUB.
    let mut rng = Rng::new(0x0000_5EED);
    for _ in 0..500 {
        let states: Vec<RuntimeValue> = (0..4).map(|_| rand_map_of_sets(&mut rng)).collect();
        let mut truth = states[0].clone();
        for s in &states[1..] {
            truth = crdt_merge_field(&truth, s.clone());
        }
        // Deliver each state once, then a duplicate of each, in a shuffled order.
        let mut deliveries: Vec<usize> = (0..states.len()).chain(0..states.len()).collect();
        for i in (1..deliveries.len()).rev() {
            let j = rng.upto((i + 1) as u64) as usize;
            deliveries.swap(i, j);
        }
        let mut replica = states[rng.upto(states.len() as u64) as usize].clone();
        for &d in &deliveries {
            replica = crdt_merge_field(&replica, states[d].clone());
        }
        assert!(
            crdt_eq(&replica, &truth),
            "every replica must converge to the LUB despite order/duplication"
        );
    }
}

#[test]
fn crdt_map_round_trips_and_merges_through_the_wire() {
    // The CRDT map survives the JSON relay wire (the tagged `__map` form) and the wire
    // merge equals the in-memory merge — sync-over-network for shared memory.
    let mut rng = Rng::new(0x0057_17E0);
    for _ in 0..400 {
        let local = rand_map_of_sets(&mut rng);
        let incoming = rand_map_of_sets(&mut rng);
        let expected = crdt_merge_field(&local, incoming.clone());
        let bytes = crdt_to_wire(&incoming).expect("a map publishes to the wire");
        let merged = crdt_merge_wire(local.clone(), &bytes);
        assert!(
            crdt_eq(&merged, &expected),
            "wire merge must equal the in-memory merge"
        );
    }
}
