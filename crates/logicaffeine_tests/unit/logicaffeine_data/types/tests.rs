use super::*;

/// `div_floor` rounds toward negative infinity across every sign combination,
/// stays exact past `i64` (BigInt promotion), and raises on a zero divisor —
/// differentially checked against an `i128` floor oracle over an exhaustive
/// small grid so no sign case escapes.
#[test]
fn div_floor_rounds_toward_negative_infinity_exhaustively() {
    let small = |x: i64| LogosInt::Small(x);
    // Exhaustive small grid including both signs and zero dividend.
    for a in -20i64..=20 {
        for b in -20i64..=20 {
            if b == 0 {
                assert!(
                    small(a).div_floor(&small(b)).is_err(),
                    "{a} // 0 must error"
                );
                continue;
            }
            let oracle = (a as i128).div_euclid(b as i128); // euclid == floor for these
                                                            // div_euclid is NOT floor for a>=0,b<0 — recompute a true floor oracle.
            let q = (a as i128) / (b as i128);
            let r = (a as i128) % (b as i128);
            let floor = if r != 0 && (r < 0) != (b < 0) {
                q - 1
            } else {
                q
            };
            let _ = oracle;
            let got = small(a).div_floor(&small(b)).unwrap();
            assert_eq!(got, LogosInt::Small(floor as i64), "{a} // {b}");
        }
    }
    // The canonical distinguishing case: truncation gives -3, floor gives -4.
    assert_eq!(small(-7).div_floor(&small(2)).unwrap(), LogosInt::Small(-4));
    assert_eq!(small(-7).div(&small(2)).unwrap(), LogosInt::Small(-3));
    // Exact past i64: (10^30) // 7 stays exact and floored.
    let ten30 = LogosInt::Small(10).pow(&LogosInt::Small(30)).unwrap();
    let q = ten30.div_floor(&small(7)).unwrap();
    assert_eq!(q.to_string(), "142857142857142857142857142857");
    // Negative BigInt numerator floors down, not toward zero.
    let neg = LogosInt::Small(-1).pow(&LogosInt::Small(1)).unwrap(); // -1
    let big_neg = ten30.mul(&neg);
    // -(10^30) // 7 = floor(-1.428...e29) = -(10^30)/7 rounded down.
    let qn = big_neg.div_floor(&small(7)).unwrap();
    assert_eq!(qn.to_string(), "-142857142857142857142857142858");
}

/// A deterministic LCG so the `LogosI64Map` fuzz is reproducible without a
/// `rand` dependency (same seed → same op stream every run).
struct Lcg(u64);
impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

/// Differential fuzz: a long random stream of inserts/gets/contains on a
/// `LogosI64Map` must agree with `std::collections::HashMap` (the oracle) on
/// every read, across many resizes and over a key space that forces both
/// overwrites (small range → collisions) and the `0` sentinel path (the real
/// key `0`, tracked outside the table) as well as `i64::MIN` as an ordinary
/// in-table key.
#[test]
fn i64map_matches_hashmap_oracle() {
    use std::collections::HashMap;
    // The key pool deliberately includes `0` (the sentinel value) and both
    // `i64` extremes so the empty-slot encoding and the side-tracked zero key
    // are exercised under fuzz, not just in isolation.
    let pool: Vec<i64> = {
        let mut v: Vec<i64> = (-40..40).collect();
        v.extend_from_slice(&[i64::MIN, i64::MAX, i64::MIN + 1, i64::MAX - 1, 0]);
        v
    };
    for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
        let mut rng = Lcg(seed);
        let mut map = LogosI64Map::new();
        let mut oracle: HashMap<i64, i64> = HashMap::new();
        for _ in 0..50_000 {
            let key = pool[(rng.next_u64() as usize) % pool.len()];
            match rng.next_u64() % 3 {
                0 => {
                    let val = rng.next_u64() as i64;
                    map.insert(key, val);
                    oracle.insert(key, val);
                }
                1 => {
                    assert_eq!(
                        map.get(&key),
                        oracle.get(&key).copied(),
                        "get disagreed for key {key} (seed {seed})"
                    );
                }
                _ => {
                    assert_eq!(
                        map.contains_key(&key),
                        oracle.contains_key(&key),
                        "contains disagreed for key {key} (seed {seed})"
                    );
                }
            }
            assert_eq!(map.len(), oracle.len(), "len diverged (seed {seed})");
        }
        // Final sweep: every pooled key must read identically.
        for &k in &pool {
            assert_eq!(map.get(&k), oracle.get(&k).copied(), "final get key {k}");
        }
    }
}

/// `LogosI64Set` agrees with `std::HashSet` under a 50k-op fuzz across seeds,
/// with `0` (the sentinel value) and both `i64` extremes in the key pool. The
/// set's `insert(key, _value)` mirrors the map call shape; the value is ignored.
#[test]
fn i64set_matches_hashset_oracle() {
    use std::collections::HashSet;
    let pool: Vec<i64> = {
        let mut v: Vec<i64> = (-40..40).collect();
        v.extend_from_slice(&[i64::MIN, i64::MAX, i64::MIN + 1, i64::MAX - 1, 0]);
        v
    };
    for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
        let mut rng = Lcg(seed);
        let mut set = LogosI64Set::new();
        let mut oracle: HashSet<i64> = HashSet::new();
        for _ in 0..50_000 {
            let key = pool[(rng.next_u64() as usize) % pool.len()];
            if rng.next_u64() % 2 == 0 {
                set.insert(key, 1);
                oracle.insert(key);
            } else {
                assert_eq!(
                    set.contains_key(&key),
                    oracle.contains(&key),
                    "contains disagreed for key {key} (seed {seed})"
                );
            }
            assert_eq!(set.len(), oracle.len(), "len diverged (seed {seed})");
        }
        for &k in &pool {
            assert_eq!(
                set.contains_key(&k),
                oracle.contains(&k),
                "final contains key {k}"
            );
        }
    }
}

/// `LogosDivU64` (loop-invariant libdivide) must agree with the hardware
/// `/` and `%` for EVERY (numerator, divisor) pair — exhaustively over small
/// values (every divisor 1..=512 against numerators 0..=4096, which forces
/// the power-of-two fast path, the 64-bit-magic path, AND the ADD_MARKER
/// 65-bit-magic path), and under a wide fuzz that hammers the full `u64`
/// range including `u64::MAX`, `2^63`, and the exact benchmark divisors. Any
/// disagreement here is a miscompiled division — the test is the spec.
#[test]
fn divu64_matches_hardware_div_and_rem() {
    // Exhaustive small grid — divisor 1..=512, numerator 0..=4096.
    for d in 1u64..=512 {
        let m = LogosDivU64::new(d);
        for x in 0u64..=4096 {
            assert_eq!(m.div(x), x / d, "div({x}, {d})");
            assert_eq!(m.rem(x), x % d, "rem({x}, {d})");
        }
    }

    // Boundary divisors (powers of two, primes, near-2^k, benchmark sizes)
    // crossed with boundary + fuzzed numerators across the full u64 range.
    let divisors: Vec<u64> = vec![
        1,
        2,
        3,
        4,
        5,
        7,
        8,
        9,
        10,
        16,
        31,
        37,
        41,
        43,
        47,
        64,
        100,
        127,
        128,
        1000,
        1024,
        65535,
        65536,
        65537,
        1_000_000,
        2_147_483_648,
        3_000_000,
        5_000_000,
        1_000_000_007,
        (1u64 << 62) - 1,
        1u64 << 62,
        (1u64 << 63) - 1,
        1u64 << 63,
        u64::MAX - 1,
        u64::MAX,
    ];
    let mut rng = Lcg(0xDEAD_BEEF_CAFE_F00D);
    for &d in &divisors {
        let m = LogosDivU64::new(d);
        let mut numerators: Vec<u64> = vec![
            0,
            1,
            2,
            d.wrapping_sub(1),
            d,
            d.wrapping_add(1),
            d.wrapping_mul(2),
            i64::MAX as u64,
            1u64 << 63,
            u64::MAX - 1,
            u64::MAX,
        ];
        for _ in 0..2000 {
            numerators.push(rng.next_u64());
        }
        for &x in &numerators {
            assert_eq!(m.div(x), x / d, "div({x}, {d}) [fuzz]");
            assert_eq!(m.rem(x), x % d, "rem({x}, {d}) [fuzz]");
        }
    }
}

/// `i64::MIN` is a genuine, distinguishable key — present after insert,
/// absent before, and independent of the rest of the table. (It is now an
/// ordinary in-table key; the empty-slot sentinel is `0`.)
#[test]
fn i64map_sentinel_key_is_a_real_key() {
    let mut m = LogosI64Map::new();
    assert_eq!(m.get(&i64::MIN), None);
    assert!(!m.contains_key(&i64::MIN));

    // Fill enough ordinary keys to force several resizes around it.
    for k in 0..1000i64 {
        m.insert(k, k * 3);
    }
    assert_eq!(m.get(&i64::MIN), None, "sentinel must stay absent");

    m.insert(i64::MIN, 999);
    assert_eq!(m.get(&i64::MIN), Some(999));
    assert!(m.contains_key(&i64::MIN));
    // It does not perturb ordinary keys, nor they it.
    for k in 0..1000i64 {
        assert_eq!(m.get(&k), Some(k * 3));
    }
    m.insert(i64::MIN, -1);
    assert_eq!(m.get(&i64::MIN), Some(-1), "sentinel value overwrites");
    assert_eq!(m.len(), 1001, "sentinel counts once toward len");
}

/// Overwriting an existing key replaces its value and never grows `len`.
#[test]
fn i64map_overwrite_preserves_len() {
    let mut m = LogosI64Map::new();
    for k in 0..100i64 {
        m.insert(k, 1);
    }
    assert_eq!(m.len(), 100);
    for k in 0..100i64 {
        m.insert(k, k * 10);
    }
    assert_eq!(m.len(), 100, "overwrites must not change len");
    for k in 0..100i64 {
        assert_eq!(m.get(&k), Some(k * 10));
    }
}

/// `with_capacity` pre-sized then filled exactly to capacity reads back
/// every entry (the headroom math must not under-allocate).
#[test]
fn i64map_with_capacity_fills_correctly() {
    for cap in [0usize, 1, 7, 8, 100, 1000] {
        let mut m = LogosI64Map::with_capacity(cap);
        let n = cap.max(1) as i64;
        for k in 0..n {
            m.insert(k, k + 7);
        }
        assert_eq!(m.len(), n as usize, "cap {cap}");
        for k in 0..n {
            assert_eq!(m.get(&k), Some(k + 7), "cap {cap} key {k}");
        }
        assert_eq!(m.get(&(n + 1)), None);
    }
}

/// `Clone` has value semantics — mutating the original leaves the clone
/// untouched (the whole point of selecting this map only when non-aliased).
#[test]
fn i64map_clone_is_independent() {
    let mut a = LogosI64Map::new();
    for k in 0..50i64 {
        a.insert(k, k);
    }
    let b = a.clone();
    for k in 0..50i64 {
        a.insert(k, k + 1000);
    }
    a.insert(100, 100);
    for k in 0..50i64 {
        assert_eq!(b.get(&k), Some(k), "clone must not see later mutations");
    }
    assert_eq!(b.get(&100), None, "clone must not gain new keys");
    assert_eq!(b.len(), 50);
}

/// The key `0` is a genuine, distinguishable key — never confused with an
/// empty slot — present after insert, absent before, and independent of the
/// rest of the table across many resizes. With the `0` empty-slot sentinel
/// the real key `0` lives outside the probe table; this pins that side path.
#[test]
fn i64map_zero_key_is_a_real_key() {
    let mut m = LogosI64Map::new();
    assert_eq!(m.get(&0), None);
    assert!(!m.contains_key(&0));

    // Ordinary non-zero keys force several resizes around the zero key.
    for k in 1..=1000i64 {
        m.insert(k, k * 3);
    }
    assert_eq!(m.get(&0), None, "zero key must stay absent");

    m.insert(0, 999);
    assert_eq!(m.get(&0), Some(999));
    assert!(m.contains_key(&0));
    for k in 1..=1000i64 {
        assert_eq!(
            m.get(&k),
            Some(k * 3),
            "ordinary keys undisturbed by zero key"
        );
    }
    m.insert(0, -1);
    assert_eq!(m.get(&0), Some(-1), "zero key value overwrites");
    assert_eq!(m.len(), 1001, "zero key counts once toward len");
}

/// The set analogue: `0` is a real member, distinct from empty slots, stable
/// across resizes driven by non-zero members.
#[test]
fn i64set_zero_key_is_a_real_key() {
    let mut s = LogosI64Set::new();
    assert!(!s.contains_key(&0));
    for k in 1..=1000i64 {
        s.insert(k, 1);
    }
    assert!(!s.contains_key(&0), "zero key must stay absent");
    s.insert(0, 1);
    assert!(s.contains_key(&0));
    for k in 1..=1000i64 {
        assert!(s.contains_key(&k), "members undisturbed by zero key");
    }
    s.insert(0, 1);
    assert_eq!(s.len(), 1001, "zero key counts once toward len");
}

/// A present key whose VALUE is `0` must read back as `Some(0)`, never `None`
/// — emptiness is decided by the key lane, never the value lane (the classic
/// open-addressing trap once the value array is zero-initialized via calloc).
#[test]
fn i64map_value_zero_distinct_from_absent() {
    let mut m = LogosI64Map::new();
    m.insert(5, 0);
    m.insert(7, 0);
    assert_eq!(m.get(&5), Some(0), "present key with value 0 is not absent");
    assert_eq!(m.get(&7), Some(0));
    assert_eq!(m.get(&6), None, "truly absent key is None");

    // Same, with a pre-sized (calloc-zeroed) table and the zero key itself.
    let mut m2 = LogosI64Map::with_capacity(64);
    m2.insert(0, 0);
    m2.insert(3, 0);
    assert_eq!(m2.get(&0), Some(0), "zero key with zero value is present");
    assert_eq!(m2.get(&3), Some(0));
    assert_eq!(m2.get(&1), None);
}

/// `i64::MIN` is an ORDINARY in-table key (no longer a sentinel): it coexists
/// with `i64::MAX` and ordinary keys, each with a distinct value, across
/// resizes, and overwrites like any other key.
#[test]
fn i64map_min_key_is_ordinary() {
    let mut m = LogosI64Map::new();
    m.insert(i64::MIN, 11);
    m.insert(i64::MAX, 22);
    for k in 1..=1000i64 {
        m.insert(k, k);
    }
    assert_eq!(m.get(&i64::MIN), Some(11));
    assert_eq!(m.get(&i64::MAX), Some(22));
    m.insert(i64::MIN, 33);
    assert_eq!(
        m.get(&i64::MIN),
        Some(33),
        "i64::MIN overwrites like any key"
    );
    assert_eq!(m.get(&i64::MAX), Some(22), "and does not perturb i64::MAX");
    assert_eq!(m.len(), 1002, "i64::MIN + i64::MAX + 1000 ordinary keys");
}

#[test]
fn value_int_arithmetic() {
    assert_eq!(Value::Int(10) + Value::Int(3), Value::Int(13));
    assert_eq!(Value::Int(10) - Value::Int(3), Value::Int(7));
    assert_eq!(Value::Int(10) * Value::Int(3), Value::Int(30));
    assert_eq!(Value::Int(10) / Value::Int(3), Value::Int(3));
}

#[test]
fn value_float_arithmetic() {
    assert_eq!(Value::Float(2.5) + Value::Float(1.5), Value::Float(4.0));
    assert_eq!(Value::Float(5.0) - Value::Float(1.5), Value::Float(3.5));
    assert_eq!(Value::Float(2.0) * Value::Float(3.0), Value::Float(6.0));
    assert_eq!(Value::Float(7.0) / Value::Float(2.0), Value::Float(3.5));
}

#[test]
fn value_cross_type_promotion() {
    assert_eq!(Value::Int(2) + Value::Float(1.5), Value::Float(3.5));
    assert_eq!(Value::Float(2.5) + Value::Int(2), Value::Float(4.5));
    assert_eq!(Value::Int(3) * Value::Float(2.0), Value::Float(6.0));
    assert_eq!(Value::Float(6.0) / Value::Int(2), Value::Float(3.0));
}

#[test]
fn value_text_concat() {
    assert_eq!(
        Value::Text("hello".to_string()) + Value::Text(" world".to_string()),
        Value::Text("hello world".to_string())
    );
}

#[test]
#[should_panic(expected = "divide by zero")]
fn value_div_by_zero_panics() {
    let _ = Value::Int(1) / Value::Int(0);
}

#[test]
#[should_panic(expected = "Cannot add")]
fn value_incompatible_types_panic() {
    let _ = Value::Bool(true) + Value::Int(1);
}

#[test]
fn value_display() {
    assert_eq!(format!("{}", Value::Int(42)), "42");
    assert_eq!(format!("{}", Value::Float(3.14)), "3.14");
    assert_eq!(format!("{}", Value::Bool(true)), "true");
    assert_eq!(format!("{}", Value::Text("hi".to_string())), "hi");
    assert_eq!(format!("{}", Value::Char('a')), "a");
    assert_eq!(format!("{}", Value::Nothing), "nothing");
}

#[test]
fn value_from_conversions() {
    assert_eq!(Value::from(42i64), Value::Int(42));
    assert_eq!(Value::from(3.14f64), Value::Float(3.14));
    assert_eq!(Value::from(true), Value::Bool(true));
    assert_eq!(Value::from("hello"), Value::Text("hello".to_string()));
    assert_eq!(
        Value::from("hello".to_string()),
        Value::Text("hello".to_string())
    );
    assert_eq!(Value::from('x'), Value::Char('x'));
}

/// Differential fuzz: a `LogosDenseI64Map` built over a proven window
/// `[lo, lo+cap)` must agree with `std::collections::HashMap` on every
/// `get`/`contains`/`len` — for keys IN the window (the only keys the
/// soundness gate ever lets reach this representation), including in-window
/// keys that are never inserted (which must read `None`), overwrites, and a
/// negative offset `lo`. The window is the contract; we never index outside it.
#[test]
fn dense_i64map_matches_hashmap_oracle() {
    use std::collections::HashMap;
    // (lo, cap) windows: 0-based, 1-based (the collect shape), and negative.
    for &(lo, cap) in &[(0i64, 64usize), (1, 100), (-40, 80), (-1, 9), (1000, 50)] {
        for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
            let mut rng = Lcg(seed);
            let mut map = LogosDenseI64Map::with_bounds(lo, cap);
            let mut oracle: HashMap<i64, i64> = HashMap::new();
            for _ in 0..40_000 {
                // Draw a key strictly inside the window [lo, lo+cap).
                let key = lo + (rng.next_u64() % cap as u64) as i64;
                match rng.next_u64() % 3 {
                    0 => {
                        let val = rng.next_u64() as i64;
                        map.insert(key, val);
                        oracle.insert(key, val);
                    }
                    1 => assert_eq!(
                        map.get(&key),
                        oracle.get(&key).copied(),
                        "get disagreed for key {key} (lo {lo} cap {cap} seed {seed})"
                    ),
                    _ => assert_eq!(
                        map.contains_key(&key),
                        oracle.contains_key(&key),
                        "contains disagreed for key {key} (lo {lo} cap {cap} seed {seed})"
                    ),
                }
                assert_eq!(
                    map.len(),
                    oracle.len(),
                    "len diverged (lo {lo} cap {cap} seed {seed})"
                );
            }
            // Final sweep: every in-window key reads identically, present or absent.
            for k in lo..lo + cap as i64 {
                assert_eq!(
                    map.get(&k),
                    oracle.get(&k).copied(),
                    "final get key {k} (lo {lo} cap {cap})"
                );
            }
        }
    }
}

/// A never-inserted but in-range key reads `None` (the presence bitset is
/// what distinguishes "stored 0" from "absent"), and an offset `lo` rebases
/// correctly with no aliasing between neighbouring keys.
#[test]
fn dense_i64map_absent_in_range_key_is_none() {
    let mut m = LogosDenseI64Map::with_bounds(-5, 20); // window [-5, 15)
    assert_eq!(m.get(&3), None);
    m.insert(-5, 100);
    m.insert(14, 200);
    m.insert(0, 0); // value 0 is a real stored value, NOT "absent"
    assert_eq!(m.get(&-5), Some(100));
    assert_eq!(m.get(&14), Some(200));
    assert_eq!(m.get(&0), Some(0), "stored 0 must read as present");
    assert_eq!(m.get(&3), None, "never-inserted in-range key stays absent");
    assert_eq!(m.get(&-1), None);
    assert_eq!(m.len(), 3);
    // Overwrite preserves len; neighbours untouched.
    m.insert(0, 42);
    assert_eq!(m.get(&0), Some(42));
    assert_eq!(m.get(&-5), Some(100));
    assert_eq!(m.len(), 3, "overwrite must not change len");
}

/// The presence-elided `LogosDenseI64MapNoPresence` is selected ONLY when the
/// compiler proves every queried key was inserted (contiguous full coverage),
/// so its `get` is a pure `Some(data[k-lo])` load. Under that regime it must
/// agree with a `HashMap` for every covered key — including a negative `lo`.
#[test]
fn dense_i64map_nopresence_full_coverage() {
    for &(lo, cap) in &[(0i64, 64usize), (1, 1000), (-40, 80)] {
        let mut m = LogosDenseI64MapNoPresence::with_bounds(lo, cap);
        // Fully cover the window (the proven precondition for this type).
        for k in lo..lo + cap as i64 {
            m.insert(k, k.wrapping_mul(3).wrapping_add(7));
        }
        for k in lo..lo + cap as i64 {
            assert_eq!(
                m.get(&k),
                Some(k.wrapping_mul(3).wrapping_add(7)),
                "nopresence get key {k} (lo {lo} cap {cap})"
            );
        }
        assert_eq!(m.len(), cap, "nopresence len (lo {lo} cap {cap})");
    }
}

/// Differential fuzz for the dense SET sibling against `std::HashSet`.
#[test]
fn dense_i64set_matches_hashset_oracle() {
    use std::collections::HashSet;
    for &(lo, cap) in &[(0i64, 64usize), (1, 100), (-40, 80), (5, 5)] {
        for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
            let mut rng = Lcg(seed);
            let mut set = LogosDenseI64Set::with_bounds(lo, cap);
            let mut oracle: HashSet<i64> = HashSet::new();
            for _ in 0..40_000 {
                let key = lo + (rng.next_u64() % cap as u64) as i64;
                if rng.next_u64() % 2 == 0 {
                    set.insert(key, 1);
                    oracle.insert(key);
                } else {
                    assert_eq!(
                        set.contains_key(&key),
                        oracle.contains(&key),
                        "contains disagreed for key {key} (lo {lo} cap {cap} seed {seed})"
                    );
                }
                assert_eq!(
                    set.len(),
                    oracle.len(),
                    "len diverged (lo {lo} cap {cap} seed {seed})"
                );
            }
            for k in lo..lo + cap as i64 {
                assert_eq!(
                    set.contains_key(&k),
                    oracle.contains(&k),
                    "final contains key {k}"
                );
            }
        }
    }
}

/// The exact `collect` benchmark shape: a 1-based window `[1, n]` allocated to
/// capacity `n`, insert `i -> i*2`, then look every key up. The bound is
/// `lo = 1`, so key `n` maps to index `n-1 < n` — the off-by-one that a naive
/// `lo = 0` would blow. Both dense flavours must report all `n` found.
#[test]
fn dense_i64map_collect_benchmark_shape() {
    let n: i64 = 5000;
    // Presence-tracking flavour.
    let mut m = LogosDenseI64Map::with_bounds(1, n as usize);
    for i in 1..=n {
        m.insert(i, i << 1);
    }
    let found = (1..=n).filter(|&i| m.get(&i) == Some(i << 1)).count();
    assert_eq!(found as i64, n, "presence flavour must find every key");
    // Presence-elided flavour (proven full coverage).
    let mut mp = LogosDenseI64MapNoPresence::with_bounds(1, n as usize);
    for i in 1..=n {
        mp.insert(i, i << 1);
    }
    let found_np = (1..=n).filter(|&i| mp.get(&i) == Some(i << 1)).count();
    assert_eq!(found_np as i64, n, "nopresence flavour must find every key");
}

/// Differential fuzz: a `LogosI32Map` driven by keys AND values inside `i32`
/// range (the only inputs the narrowing gate ever sends it) must agree with a
/// `HashMap` on every `get`/`contains`/`len` — across resizes, overwrites, the
/// `0` sentinel path (the real key `0`), and `i32::MIN`/`i32::MAX` as ordinary
/// in-table keys.
#[test]
fn i32map_matches_hashmap_oracle() {
    use std::collections::HashMap;
    let pool: Vec<i64> = {
        let mut v: Vec<i64> = (-40..40).collect();
        v.extend_from_slice(&[
            i32::MIN as i64,
            i32::MAX as i64,
            (i32::MIN + 1) as i64,
            (i32::MAX - 1) as i64,
            0,
        ]);
        v
    };
    for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
        let mut rng = Lcg(seed);
        let mut map = LogosI32Map::new();
        let mut oracle: HashMap<i64, i64> = HashMap::new();
        for _ in 0..50_000 {
            let key = pool[(rng.next_u64() as usize) % pool.len()];
            match rng.next_u64() % 3 {
                0 => {
                    // A value inside i32 range (what the narrowing proof guarantees).
                    let val = (rng.next_u64() as i32) as i64;
                    map.insert(key, val);
                    oracle.insert(key, val);
                }
                1 => assert_eq!(
                    map.get(&key),
                    oracle.get(&key).copied(),
                    "get disagreed for key {key} (seed {seed})"
                ),
                _ => assert_eq!(
                    map.contains_key(&key),
                    oracle.contains_key(&key),
                    "contains disagreed for key {key} (seed {seed})"
                ),
            }
            assert_eq!(map.len(), oracle.len(), "len diverged (seed {seed})");
        }
        for &k in &pool {
            assert_eq!(map.get(&k), oracle.get(&k).copied(), "final get key {k}");
        }
    }
}

/// `LogosI32Set` agrees with `std::HashSet` under fuzz over `i32`-range keys.
#[test]
fn i32set_matches_hashset_oracle() {
    use std::collections::HashSet;
    let pool: Vec<i64> = {
        let mut v: Vec<i64> = (-40..40).collect();
        v.extend_from_slice(&[
            i32::MIN as i64,
            i32::MAX as i64,
            (i32::MIN + 1) as i64,
            (i32::MAX - 1) as i64,
            0,
        ]);
        v
    };
    for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
        let mut rng = Lcg(seed);
        let mut set = LogosI32Set::new();
        let mut oracle: HashSet<i64> = HashSet::new();
        for _ in 0..50_000 {
            let key = pool[(rng.next_u64() as usize) % pool.len()];
            if rng.next_u64() % 2 == 0 {
                set.insert(key, 1);
                oracle.insert(key);
            } else {
                assert_eq!(
                    set.contains_key(&key),
                    oracle.contains(&key),
                    "contains disagreed for key {key} (seed {seed})"
                );
            }
            assert_eq!(set.len(), oracle.len(), "len diverged (seed {seed})");
        }
        for &k in &pool {
            assert_eq!(
                set.contains_key(&k),
                oracle.contains(&k),
                "final contains key {k}"
            );
        }
    }
}

/// Every open-addressed Int table stays at most half full, presized or grown one insert at a time: a linear-probing
/// miss walks (1 + 1/(1 - α)²) / 2 slots on average, and past half full that walk, a cache miss a slot on a table
/// larger than the cache, is what two_sum's set spent its time on.
#[test]
fn int_tables_stay_at_most_half_full() {
    fn check(name: &str, len: usize, slots: usize) {
        assert!(2 * len <= slots, "{name}: {len} keys in {slots} slots is past half full");
    }
    for n in [1usize, 5, 6, 7, 100, 1_000, 65_537] {
        let mut a = LogosI64Map::new();
        let mut b = LogosI64Set::new();
        let mut c = LogosI32Map::new();
        let mut d = LogosI32Set::new();
        for k in 1..=n as i64 {
            a.insert(k, k);
            b.insert(k, 1);
            c.insert(k, k);
            d.insert(k, 1);
            check("LogosI64Map grown", a.table_len(), a.mask + 1);
            check("LogosI64Set grown", b.table_len(), b.mask + 1);
            check("LogosI32Map grown", c.table_len(), c.mask + 1);
            check("LogosI32Set grown", d.table_len(), d.mask + 1);
        }
        let (mut a, mut b) = (LogosI64Map::with_capacity(n), LogosI64Set::with_capacity(n));
        let (mut c, mut d) = (LogosI32Map::with_capacity(n), LogosI32Set::with_capacity(n));
        let slots = [a.mask + 1, b.mask + 1, c.mask + 1, d.mask + 1];
        for k in 1..=n as i64 {
            a.insert(k, k);
            b.insert(k, 1);
            c.insert(k, k);
            d.insert(k, 1);
        }
        assert_eq!(slots, [a.mask + 1, b.mask + 1, c.mask + 1, d.mask + 1], "n = {n}: a presized table never grows");
        for (name, len, slots) in [
            ("LogosI64Map presized", a.table_len(), a.mask + 1),
            ("LogosI64Set presized", b.table_len(), b.mask + 1),
            ("LogosI32Map presized", c.table_len(), c.mask + 1),
            ("LogosI32Set presized", d.table_len(), d.mask + 1),
        ] {
            check(name, len, slots);
        }
    }
}
