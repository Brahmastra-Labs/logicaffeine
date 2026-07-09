//! ════════════════════════════════════════════════════════════════════════════════════════════
//! WIRE SPECTRUM — the codec proven across the FULL spectrum of real data, from perfectly ordered
//! (constant / affine / geometric / polynomial / periodic) through structured (clustered / runs /
//! low-cardinality / shared-prefix) all the way to LITERALLY RANDOM (uniform noise, mixed magnitude,
//! random floats / strings / records). Each shape is a payload people actually transmit — timestamps,
//! status codes, latencies, prices, geo coordinates, sensor walks, feature flags, categorical labels,
//! counters, record tables, id→record maps — NOT a single synthetic combo.
//!
//! Every payload is exercised THREE ways, so a bug can't hide behind one lucky dial:
//!   1. round-trips bit-exact under `Auto` (the no-brainer "smallest" path),
//!   2. round-trips bit-exact across a MATRIX of dial combinations (numerics × structure × floats ×
//!      compression) — knobs must compose, never corrupt,
//!   3. `Auto` is never larger than the plain-varint baseline (the menu never loses to doing nothing),
//! and the ORDERED shapes additionally crush far below a same-shape RANDOM column — proving the
//! generators (affine / geometric / polynomial / periodic / FOR / RLE / dict) actually fire across
//! the whole spectrum, not just on one hand-picked input.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use logicaffeine_compile::concurrency::marshal::{
    message_from_wire, message_to_wire_with, with_compression_codec, with_floats, with_numerics,
    with_structure, WireCodec, WireCompression, WireFloats, WireIntegrity, WireNumerics, WireStructure,
};
use logicaffeine_compile::interpreter::{ListRepr, MapStorage, RuntimeValue, StructValue};

// ── deterministic RNG (so every "random" payload is reproducible) ──────────────────────────────
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
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

// ── value constructors ─────────────────────────────────────────────────────────────────────────
fn rv_ints(v: Vec<i64>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
}
fn rv_floats(v: Vec<f64>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
}
fn rv_from(values: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values))))
}
fn rv_bools(v: Vec<bool>) -> RuntimeValue {
    rv_from(v.into_iter().map(RuntimeValue::Bool).collect())
}
fn rv_strings(v: Vec<String>) -> RuntimeValue {
    rv_from(v.into_iter().map(|s| RuntimeValue::Text(Rc::new(s))).collect())
}
fn point(x: i64, y: i64) -> RuntimeValue {
    let mut fields = HashMap::new();
    fields.insert("x".to_string(), RuntimeValue::Int(x));
    fields.insert("y".to_string(), RuntimeValue::Int(y));
    RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields }))
}
fn record(id: i64, name: &str, active: bool) -> RuntimeValue {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), RuntimeValue::Int(id));
    fields.insert("name".to_string(), RuntimeValue::Text(Rc::new(name.to_string())));
    fields.insert("active".to_string(), RuntimeValue::Bool(active));
    RuntimeValue::Struct(Box::new(StructValue { type_name: "Record".to_string(), fields }))
}
fn rv_map(pairs: Vec<(RuntimeValue, RuntimeValue)>) -> RuntimeValue {
    let m: MapStorage = pairs.into_iter().collect();
    RuntimeValue::Map(Rc::new(RefCell::new(m)))
}

/// The whole spectrum, labelled. Ordered → structured → random, plus the common real workflows.
fn spectrum() -> Vec<(&'static str, RuntimeValue)> {
    let n = 1000usize;
    let mut rng = Rng::new(0x5EED_1234_ABCD_0001);
    let mut out: Vec<(&'static str, RuntimeValue)> = Vec::new();

    // ── perfectly ordered integer columns (the generators) ──
    out.push(("int/constant", rv_ints(vec![42; n])));
    out.push(("int/affine-up", rv_ints((0..n as i64).collect())));
    out.push(("int/affine-down", rv_ints((0..n as i64).map(|i| 1_000_000 - 7 * i).collect())));
    out.push(("int/affine-bigstride", rv_ints((0..n as i64).map(|i| -500 + 65_537 * i).collect())));
    out.push(("int/geometric-2", rv_ints((0..50i64).map(|i| 3 * (1i64 << i)).collect())));
    out.push(("int/geometric-neg", rv_ints({
        let mut c = 1i64;
        (0..40).map(|_| { let v = c; c = c.wrapping_mul(-2); v }).collect()
    })));
    out.push(("int/quadratic", rv_ints((0..n as i64).map(|i| 2 * i * i - 5 * i + 7).collect())));
    out.push(("int/cubic", rv_ints((0..500i64).map(|i| i * i * i - 3 * i + 1).collect())));
    out.push(("int/periodic-small", rv_ints((0..n).map(|i| [10i64, 20, 30, 40][i % 4]).collect())));
    out.push(("int/periodic-block", rv_ints((0..n).map(|i| [-7i64, 0, 999_999, -3, 42, 42, -1][i % 7]).collect())));
    out.push(("int/sawtooth-modaffine", rv_ints((0..n as i64).map(|i| 100 + 9 * (i % 13)).collect())));

    // ── structured columns (the G5 menu: FOR / RLE / DoD / dict / bytes) ──
    out.push(("int/runs-rle", rv_ints({
        let mut v = Vec::new();
        for k in 0..50 { v.extend(std::iter::repeat(k as i64 * 3).take(20)); }
        v
    })));
    out.push(("int/clustered-for", rv_ints((0..n).map(|_| 100_000 + rng.below(64) as i64).collect())));
    out.push(("int/low-cardinality-dict", rv_ints((0..n).map(|_| [3i64, 3, 3, 7, 11, 200][rng.below(6) as usize]).collect())));
    out.push(("int/byte-range", rv_ints((0..n).map(|_| rng.below(256) as i64).collect())));
    out.push(("int/near-linear-dod", rv_ints({
        let mut t = 0i64;
        (0..n).map(|_| { t += 1000 + rng.below(3) as i64; t }).collect()
    })));

    // ── literally random integer columns ──
    out.push(("int/uniform-noise", rv_ints((0..n).map(|_| rng.next() as i64).collect())));
    out.push(("int/random-signed", rv_ints((0..n).map(|_| (rng.next() as i64).wrapping_sub(i64::MAX / 2)).collect())));
    out.push(("int/mixed-magnitude", rv_ints((0..n).map(|_| {
        let bits = (rng.below(40) + 1) as u32;
        (rng.next() & ((1u64 << bits) - 1)) as i64
    }).collect())));
    out.push(("int/extremes", rv_ints((0..n).map(|_| if rng.next() & 1 == 0 { i64::MIN } else { i64::MAX }).collect())));

    // ── common integer WORKFLOWS ──
    out.push(("workflow/timestamps", rv_ints({
        let mut t = 1_700_000_000_000i64;
        (0..n).map(|_| { t += rng.below(1000) as i64; t }).collect()
    })));
    out.push(("workflow/http-status", rv_ints((0..n).map(|_| [200i64, 200, 200, 301, 404, 500, 204, 200][rng.below(8) as usize]).collect())));
    out.push(("workflow/latencies-ms", rv_ints((0..n).map(|_| 1 + rng.below(500) as i64).collect())));
    out.push(("workflow/prices-cents", rv_ints((0..n).map(|_| 99 + rng.below(90_000) as i64).collect())));
    out.push(("workflow/counters", rv_ints((0..n).map(|_| {
        let bits = (rng.below(40) + 1) as u32;
        (rng.next() & ((1u64 << bits) - 1)) as i64
    }).collect())));

    // ── float columns: ordered → random + workflows ──
    out.push(("float/constant", rv_floats(vec![3.14159; n])));
    out.push(("float/monotone", rv_floats((0..n).map(|i| i as f64 * 0.5).collect())));
    out.push(("float/geo-lat", rv_floats((0..n).map(|_| rng.below(180_000_000) as f64 / 1_000_000.0 - 90.0).collect())));
    out.push(("float/sensor-walk", rv_floats({
        let mut s = 20.0f64;
        (0..n).map(|_| { s += (rng.below(200) as f64 - 100.0) / 100.0; (s * 100.0).round() / 100.0 }).collect()
    })));
    out.push(("float/random", rv_floats((0..n).map(|_| rng.next() as f64 / 7.0).collect())));

    // ── string columns: ordered → random + workflows ──
    out.push(("string/constant", rv_strings(vec!["OK".to_string(); n])));
    out.push(("string/categorical", rv_strings((0..n).map(|_| ["GET", "POST", "GET", "PUT", "DELETE", "GET"][rng.below(6) as usize].to_string()).collect())));
    out.push(("string/shared-prefix", rv_strings((0..n).map(|i| format!("https://api.example.com/v1/items/{i}")).collect())));
    out.push(("string/sorted", rv_strings({
        let mut v: Vec<String> = (0..n).map(|_| format!("user_{:06}", rng.below(1_000_000))).collect();
        v.sort();
        v
    })));
    out.push(("string/random", rv_strings((0..n).map(|_| {
        let len = 4 + rng.below(12) as usize;
        (0..len).map(|_| (b'a' + rng.below(26) as u8) as char).collect()
    }).collect())));

    // ── bool columns ──
    out.push(("bool/all-true", rv_bools(vec![true; n])));
    out.push(("bool/alternating", rv_bools((0..n).map(|i| i % 2 == 0).collect())));
    out.push(("bool/random-flags", rv_bools((0..n).map(|_| rng.next() & 1 == 0).collect())));

    // ── struct lists (record tables) ──
    out.push(("struct/points-affine", rv_from((0..200i64).map(|i| point(i, 2 * i + 1)).collect())));
    out.push(("struct/records-structured", rv_from((0..200i64).map(|i| record(i, "alpha", i % 3 == 0)).collect())));
    out.push(("struct/records-random", rv_from((0..200).map(|_| {
        let id = rng.next() as i64;
        let name = format!("n{}", rng.below(1000));
        record(id, &name, rng.next() & 1 == 0)
    }).collect())));

    // ── maps (id → value tables) ──
    out.push(("map/int-int-affine", rv_map((0..500i64).map(|k| (RuntimeValue::Int(k), RuntimeValue::Int(2 * k))).collect())));
    out.push(("map/int-int-random", rv_map((0..500i64).map(|k| (RuntimeValue::Int(k), RuntimeValue::Int(rng.next() as i64))).collect())));
    out.push(("map/int-url", rv_map((0..300i64).map(|k| (RuntimeValue::Int(k), RuntimeValue::Text(Rc::new(format!("https://cdn/x/{k}"))))).collect())));
    out.push(("map/int-record", rv_map((0..200i64).map(|k| (RuntimeValue::Int(k), record(k, "row", k % 2 == 0))).collect())));
    out.push(("map/string-int", rv_map((0..200i64).map(|k| (RuntimeValue::Text(Rc::new(format!("k{k}"))), RuntimeValue::Int(k * k))).collect())));

    // ── scalars & edges ──
    out.push(("edge/single-int", RuntimeValue::Int(-123456789)));
    out.push(("edge/single-struct", record(7, "solo", true)));
    out.push(("edge/empty-list", rv_ints(Vec::new())));
    out.push(("edge/one-element", rv_ints(vec![999])));
    out.push(("edge/two-element", rv_ints(vec![5, 9])));
    out.push(("edge/nested-list", rv_from(vec![rv_ints(vec![1, 2, 3]), rv_ints(vec![4, 5, 6])])));

    out
}

/// The canonical encoding (default dials) — a deterministic fingerprint of a value: equal values
/// encode to equal bytes, so a decode that re-encodes to the canonical form is value-exact even for
/// containers whose `PartialEq` is shallow (structs / maps).
fn canonical(v: &RuntimeValue) -> Vec<u8> {
    message_to_wire_with("", v, WireCodec::Native, WireIntegrity::Raw).unwrap()
}

#[test]
fn every_spectrum_payload_roundtrips_under_auto() {
    for (label, v) in spectrum() {
        let want = canonical(&v);
        let enc = with_structure(WireStructure::Auto, || canonical(&v));
        let back = message_from_wire(&enc)
            .unwrap_or_else(|| panic!("[{label}] Auto encoding failed to decode"))
            .1;
        assert_eq!(canonical(&back), want, "[{label}] Auto round-trip is not value-exact");
    }
}

#[test]
fn every_spectrum_payload_roundtrips_across_dial_combos() {
    // Not one lucky combo: the cross product of numerics × structure × floats × compression must ALL
    // decode back to the same value. (integrity stays Raw so the canonical fingerprint is comparable.)
    let nums = [WireNumerics::Varint, WireNumerics::Fixed, WireNumerics::GroupVarint];
    let structs = [WireStructure::Off, WireStructure::Affine, WireStructure::Auto];
    let floats = [WireFloats::Memcpy, WireFloats::XorDelta];
    let comps = [WireCompression::None, WireCompression::Deflate, WireCompression::Zstd];

    for (label, v) in spectrum() {
        let want = canonical(&v);
        let mut combos = 0usize;
        for &nu in &nums {
            for &st in &structs {
                for &fl in &floats {
                    for &co in &comps {
                        let enc = with_numerics(nu, || {
                            with_structure(st, || {
                                with_floats(fl, || with_compression_codec(co, || canonical(&v)))
                            })
                        });
                        let back = message_from_wire(&enc).unwrap_or_else(|| {
                            panic!("[{label}] combo num={nu:?} st={st:?} fl={fl:?} co={co:?} failed to decode")
                        });
                        assert_eq!(
                            canonical(&back.1),
                            want,
                            "[{label}] combo num={nu:?} st={st:?} fl={fl:?} co={co:?} corrupted the value"
                        );
                        combos += 1;
                    }
                }
            }
        }
        assert_eq!(combos, 54, "[{label}] every dial combo must run");
    }
}

#[test]
fn auto_is_never_larger_than_plain_varint_across_the_spectrum() {
    for (label, v) in spectrum() {
        let varint = with_structure(WireStructure::Off, || with_numerics(WireNumerics::Varint, || canonical(&v)));
        let auto = with_structure(WireStructure::Auto, || canonical(&v));
        assert!(
            auto.len() <= varint.len(),
            "[{label}] Auto ({}) must never be larger than plain varint ({})",
            auto.len(),
            varint.len()
        );
    }
}

#[test]
fn ordered_columns_crush_far_below_their_random_peer() {
    // The same length+nature of column, ordered vs random, must differ by orders of magnitude — that
    // gap IS the generators firing. A 1000-row affine/geometric/polynomial/periodic column collapses
    // to a handful of bytes; its random sibling cannot.
    let n = 1000usize;
    let mut rng = Rng::new(0xC0DE_F00D);
    let random = with_structure(WireStructure::Auto, || {
        canonical(&rv_ints((0..n).map(|_| rng.next() as i64).collect()))
    });

    let auto = |v: RuntimeValue| with_structure(WireStructure::Auto, || canonical(&v));
    let affine = auto(rv_ints((0..n as i64).collect()));
    let constant = auto(rv_ints(vec![7; n]));
    let quadratic = auto(rv_ints((0..n as i64).map(|i| i * i + i + 1).collect()));
    let periodic = auto(rv_ints((0..n).map(|i| [9i64, 8, 7, 100, -4][i % 5]).collect()));
    let geometric = auto(rv_ints((0..60i64).map(|i| 1i64.wrapping_shl(i as u32 % 62)).collect()));

    for (name, bytes) in [
        ("affine", &affine),
        ("constant", &constant),
        ("quadratic", &quadratic),
        ("periodic", &periodic),
    ] {
        assert!(
            bytes.len() * 20 < random.len(),
            "{name} ({}) must crush far below random ({}) — the generator must fire",
            bytes.len(),
            random.len()
        );
    }
    // geometric is only 60 values; compare against its own random peer of the same length.
    let mut rng2 = Rng::new(0xBEEF_1234);
    let random60 = auto(rv_ints((0..60).map(|_| rng2.next() as i64).collect()));
    assert!(
        geometric.len() * 4 < random60.len(),
        "geometric ({}) must crush below a random 60-column ({})",
        geometric.len(),
        random60.len()
    );
}
