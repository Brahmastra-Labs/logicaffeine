use super::*;

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Operands that stress Algorithm D: random limbs, all-ones limbs (the `q̂` overestimate),
/// divisors with a top limb just past a power of two (the add-back step), and lengths from 1
/// to 12 limbs on each side.
fn division_corpus() -> Vec<Vec<u64>> {
    let mut st = 0x2545_f491_4f6c_dd1d;
    let mut out = Vec::new();
    for len in 1..=12usize {
        out.push((0..len).map(|_| xorshift(&mut st)).collect());
        out.push(vec![u64::MAX; len]);
        let mut v = vec![0u64; len];
        v[len - 1] = 1 << 63;
        out.push(v.clone());
        v[0] = 1;
        out.push(v);
        let mut w: Vec<u64> = (0..len).map(|_| xorshift(&mut st) >> 40).collect();
        w[len - 1] |= 1;
        out.push(w);
    }
    out.into_iter()
        .map(normalize)
        .filter(|v| !v.is_empty())
        .collect()
}

#[test]
fn algorithm_d_agrees_with_bitwise_long_division() {
    let corpus = division_corpus();
    for a in &corpus {
        for b in &corpus {
            let fast = mag_divrem(a, b);
            let slow = mag_divrem_bitwise(a, b);
            assert_eq!(fast, slow, "a = {a:x?}, b = {b:x?}");
            let (q, r) = fast;
            assert_eq!(mag_add(&mag_mul(&q, b), &r), a.clone());
            assert_eq!(mag_cmp(&r, b), Ordering::Less);
        }
    }
}

#[test]
fn natural_number_operations() {
    let n = |s: &str| BigInt::parse_decimal(s).unwrap();
    assert_eq!(n("12").gcd(&n("18")), n("6"));
    assert_eq!(n("0").gcd(&n("5")), n("5"));
    assert_eq!(n("2").pow(300).gcd(&n("6").pow(100)), n("2").pow(100));
    assert_eq!(n("1").shl_bits(200).shr_bits(199), Some(n("2")));
    assert_eq!(n("-3").shl_bits(2), n("-12"));
    assert_eq!(n("-3").shr_bits(1), None);
    assert_eq!(n("1024").bit_len(), 11);
    assert_eq!(n("0").bit_len(), 0);
    let (a, b) = (
        n("340282366920938463463374607431768211455"),
        n("18446744073709551616"),
    );
    assert_eq!(a.bitand_natural(&b), Some(b.clone()));
    assert_eq!(a.bitor_natural(&b), Some(a.clone()));
    assert_eq!(a.bitxor_natural(&a), Some(n("0")));
    assert_eq!(n("-1").bitand_natural(&a), None);
    assert_eq!(n("18446744073709551615").to_u64(), Some(u64::MAX));
    assert_eq!(n("18446744073709551616").to_u64(), None);
    assert_eq!(n("-1").to_u64(), None);
}

/// The in-place scalar multiply must be byte-identical to the functional `mul` across every
/// magnitude width, sign combination, and scalar edge (0, ±1, i64::MIN/MAX, primes, random) —
/// in-place is only ever a buffer optimization, never a value change.
#[test]
fn mul_assign_i64_matches_functional_mul() {
    let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut scalars: Vec<i64> = vec![
        0,
        1,
        -1,
        2,
        -2,
        7,
        -7,
        i64::MIN,
        i64::MAX,
        i64::MIN + 1,
        i64::MAX - 1,
        1_000_000_007,
        -999_999_937,
        4_294_967_296,
        -4_294_967_296,
    ];
    for _ in 0..40 {
        scalars.push(next() as i64);
    }
    for &limbs in &[0usize, 1, 2, 3, 5, 8, 16, 40] {
        for &neg in &[false, true] {
            for _ in 0..8 {
                let v: Vec<u64> = (0..limbs).map(|_| next()).collect();
                let base = BigInt::from_sign_mag(neg, v);
                for &rhs in &scalars {
                    let mut inplace = base.clone();
                    inplace.mul_assign_i64(rhs);
                    let functional = base.mul(&BigInt::from_i64(rhs));
                    assert!(
                        inplace == functional,
                        "mul_assign_i64 differs from functional mul: limbs={limbs} neg={neg} rhs={rhs}"
                    );
                }
            }
        }
    }
}

fn b(x: i64) -> BigInt {
    BigInt::from_i64(x)
}

/// Karatsuba must be BIT-IDENTICAL to schoolbook on every operand shape — the whole point of a
/// fast multiply is that it changes only the cost, never the value. Fuzzed across sizes that span
/// `KARATSUBA_THRESHOLD` (so both the schoolbook fallback and deep Karatsuba recursion are hit),
/// including badly-unbalanced operands and zero/one-limb edges. A miscompiled bignum multiply is
/// catastrophic and silent, so this is deliberately heavy.
#[test]
fn karatsuba_is_bit_identical_to_schoolbook() {
    // SplitMix64 — deterministic, reproducible.
    let mut state: u64 = 0x0123_4567_89AB_CDEF;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    // Explicit edges first: zero, one-limb, exactly at/around the threshold, wildly unbalanced.
    let edges: &[(usize, usize)] = &[
        (0, 5),
        (5, 0),
        (1, 1),
        (1, 200),
        (200, 1),
        (KARATSUBA_THRESHOLD - 1, KARATSUBA_THRESHOLD - 1),
        (KARATSUBA_THRESHOLD, KARATSUBA_THRESHOLD),
        (KARATSUBA_THRESHOLD + 1, KARATSUBA_THRESHOLD),
        (300, 7),
        (256, 256),
    ];
    let check = |a: &[u64], bb: &[u64]| {
        let a = normalize(a.to_vec());
        let bb = normalize(bb.to_vec());
        assert_eq!(
            mag_mul(&a, &bb),
            mag_mul_schoolbook(&a, &bb),
            "Karatsuba diverged from schoolbook for lengths {} x {}",
            a.len(),
            bb.len()
        );
    };
    for &(la, lb) in edges {
        let a: Vec<u64> = (0..la).map(|_| next()).collect();
        let bb: Vec<u64> = (0..lb).map(|_| next()).collect();
        check(&a, &bb);
    }
    // Random spread across the threshold.
    for _ in 0..3000 {
        let la = (next() % 160) as usize;
        let lb = (next() % 160) as usize;
        let a: Vec<u64> = (0..la).map(|_| next()).collect();
        let bb: Vec<u64> = (0..lb).map(|_| next()).collect();
        check(&a, &bb);
    }
}

#[test]
fn from_to_i64_round_trips_the_extremes() {
    for x in [
        0i64,
        1,
        -1,
        42,
        -42,
        i64::MAX,
        i64::MIN,
        i64::MAX - 1,
        i64::MIN + 1,
    ] {
        assert_eq!(BigInt::from_i64(x).to_i64(), Some(x), "round trip {x}");
    }
}

#[test]
fn to_i64_is_none_just_past_the_boundary() {
    // i64::MAX + 1 and i64::MIN - 1 must NOT fit i64 (this is the whole point —
    // the value survives instead of wrapping, unlike a JSON double).
    let over = b(i64::MAX).add(&b(1));
    let under = b(i64::MIN).sub(&b(1));
    assert_eq!(over.to_i64(), None);
    assert_eq!(under.to_i64(), None);
    // …but the values are still exact and printable.
    assert_eq!(over.to_string(), "9223372036854775808");
    assert_eq!(under.to_string(), "-9223372036854775809");
}

#[test]
fn add_sub_mul_match_i128_on_a_dense_grid() {
    // Differential oracle: for every pair that fits i128, our wide arithmetic must
    // equal the machine's. Includes carry/borrow/sign corners.
    let xs: [i64; 11] = [
        0,
        1,
        -1,
        2,
        -2,
        1000,
        -1000,
        i32::MAX as i64,
        i32::MIN as i64,
        i64::MAX,
        i64::MIN,
    ];
    for &x in &xs {
        for &y in &xs {
            let (bx, by) = (b(x), b(y));
            assert_eq!(
                bx.add(&by).to_string(),
                (x as i128 + y as i128).to_string(),
                "{x}+{y}"
            );
            assert_eq!(
                bx.sub(&by).to_string(),
                (x as i128 - y as i128).to_string(),
                "{x}-{y}"
            );
            assert_eq!(
                bx.mul(&by).to_string(),
                (x as i128 * y as i128).to_string(),
                "{x}*{y}"
            );
        }
    }
}

#[test]
fn div_rem_matches_i64_truncation_including_signs() {
    let xs = [
        0i64,
        1,
        -1,
        7,
        -7,
        100,
        -100,
        9_999_999,
        -9_999_999,
        i64::MAX,
        i64::MIN,
    ];
    let ys = [1i64, -1, 2, -2, 3, -3, 7, -7, 1000, -1000];
    for &x in &xs {
        for &y in &ys {
            let (q, r) = b(x).div_rem(&b(y)).expect("nonzero divisor");
            // i64::MIN / -1 overflows i64; compare in i128 there.
            let (eq, er) = ((x as i128) / (y as i128), (x as i128) % (y as i128));
            assert_eq!(q.to_string(), eq.to_string(), "{x}/{y} quotient");
            assert_eq!(r.to_string(), er.to_string(), "{x}%{y} remainder");
            // The defining identity must hold exactly.
            assert_eq!(b(x), q.mul(&b(y)).add(&r), "x = q*y + r for {x},{y}");
        }
    }
}

#[test]
fn division_by_zero_is_none_not_a_panic() {
    assert!(b(5).div_rem(&BigInt::zero()).is_none());
    assert!(BigInt::zero().div_rem(&BigInt::zero()).is_none());
}

#[test]
fn huge_factorial_is_exact() {
    // 50! has 65 digits — far beyond any fixed-width integer or f64.
    let mut acc = BigInt::from_u64(1);
    for k in 1..=50u64 {
        acc = acc.mul(&BigInt::from_u64(k));
    }
    assert_eq!(
        acc.to_string(),
        "30414093201713378043612608166064768844377641568960512000000000000"
    );
    // Dividing back down the chain returns exactly 1 (exercises big/big division).
    for k in 1..=50u64 {
        let (q, r) = acc.div_rem(&BigInt::from_u64(k)).unwrap();
        assert!(r.is_zero(), "{k}! divides cleanly");
        acc = q;
    }
    assert_eq!(acc, BigInt::from_u64(1));
}

#[test]
fn parse_and_display_round_trip_big_decimals() {
    for s in [
        "0",
        "-0",
        "7",
        "-7",
        "9223372036854775808",
        "-9223372036854775809",
        "123456789012345678901234567890",
        "-100000000000000000000000000000000000000000",
    ] {
        let parsed = BigInt::parse_decimal(s).expect("parse");
        // "-0" canonicalizes to "0".
        let expected = if s == "-0" { "0" } else { s };
        assert_eq!(parsed.to_string(), expected, "round trip {s}");
    }
    assert!(BigInt::parse_decimal("12x3").is_none());
    assert!(BigInt::parse_decimal("").is_none());
    assert!(BigInt::parse_decimal("-").is_none());
}

/// A tiny deterministic RNG (SplitMix64) so the fuzz is reproducible with no
/// external dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A random BigInt of 0..=3 limbs and random sign — spans single- and
    /// multi-limb magnitudes (and zero).
    fn big(&mut self) -> BigInt {
        let limbs = (self.next() % 4) as usize;
        let mut bytes = Vec::new();
        for _ in 0..limbs {
            bytes.extend_from_slice(&self.next().to_le_bytes());
        }
        BigInt::from_le_bytes(self.next() & 1 == 1, &bytes)
    }
}

#[test]
fn fuzz_algebraic_laws_hold_for_random_bigints() {
    let mut r = Rng(0x0BAD_F00D_DEAD_BEEF);
    for _ in 0..3000 {
        let (a, b, c) = (r.big(), r.big(), r.big());
        // Commutativity.
        assert_eq!(a.add(&b), b.add(&a), "add commutes");
        assert_eq!(a.mul(&b), b.mul(&a), "mul commutes");
        // Associativity.
        assert_eq!(a.add(&b).add(&c), a.add(&b.add(&c)), "add associates");
        assert_eq!(a.mul(&b).mul(&c), a.mul(&b.mul(&c)), "mul associates");
        // Distributivity.
        assert_eq!(
            a.mul(&b.add(&c)),
            a.mul(&b).add(&a.mul(&c)),
            "mul distributes over add"
        );
        // Identities and inverses.
        assert_eq!(a.add(&BigInt::zero()), a, "0 is the additive identity");
        assert_eq!(
            a.mul(&BigInt::from_u64(1)),
            a,
            "1 is the multiplicative identity"
        );
        assert!(a.mul(&BigInt::zero()).is_zero(), "x*0 = 0");
        assert_eq!(a.sub(&a), BigInt::zero(), "x - x = 0");
        assert_eq!(a.add(&a.negated()), BigInt::zero(), "x + (-x) = 0");
        assert_eq!(a.negated().negated(), a, "double negation");
        // abs is non-negative; sign of a product.
        assert!(!a.abs().is_negative(), "|x| >= 0");
        assert_eq!(a.negated().mul(&b), a.mul(&b).negated(), "(-a)*b = -(a*b)");
        // Division identity and remainder bound.
        if !b.is_zero() {
            let (q, rem) = a.div_rem(&b).unwrap();
            assert_eq!(q.mul(&b).add(&rem), a, "a = q*b + r exactly");
            assert!(rem.is_zero() || rem.abs() < b.abs(), "|r| < |b|");
        }
        // Serialization and decimal round-trips.
        let (neg, bytes) = a.to_le_bytes();
        assert_eq!(BigInt::from_le_bytes(neg, &bytes), a, "byte round-trip");
        assert_eq!(
            BigInt::parse_decimal(&a.to_string()).unwrap(),
            a,
            "decimal round-trip"
        );
        // Ordering is a total, antisymmetric, transitive relation.
        assert_eq!(a < b, b > a, "antisymmetry");
        if a < b && b < c {
            assert!(a < c, "transitivity");
        }
    }
}

#[test]
fn fuzz_differential_against_i128() {
    // For random i64 operands, our (promoting) arithmetic must equal i128 exactly.
    let mut r = Rng(0xC0FF_EE00_1234_5678);
    for _ in 0..5000 {
        let x = r.next() as i64;
        let y = r.next() as i64;
        assert_eq!(
            b(x).add(&b(y)).to_string(),
            (x as i128 + y as i128).to_string(),
            "{x}+{y}"
        );
        assert_eq!(
            b(x).sub(&b(y)).to_string(),
            (x as i128 - y as i128).to_string(),
            "{x}-{y}"
        );
        assert_eq!(
            b(x).mul(&b(y)).to_string(),
            (x as i128 * y as i128).to_string(),
            "{x}*{y}"
        );
        if y != 0 {
            let (q, rem) = b(x).div_rem(&b(y)).unwrap();
            assert_eq!(
                q.to_string(),
                (x as i128 / y as i128).to_string(),
                "{x}/{y}"
            );
            assert_eq!(
                rem.to_string(),
                (x as i128 % y as i128).to_string(),
                "{x}%{y}"
            );
        }
    }
}

#[test]
fn limb_boundary_edge_cases_are_exact() {
    let two64 = BigInt::parse_decimal("18446744073709551616").unwrap(); // 2^64
                                                                        // 2^64 - 1 = u64::MAX (a borrow that empties the high limb).
    assert_eq!(
        two64.sub(&BigInt::from_u64(1)).to_string(),
        "18446744073709551615"
    );
    // 2^64 * 2^64 = 2^128 (exact two-limb product).
    let two128 = two64.mul(&two64);
    assert_eq!(
        two128.to_string(),
        "340282366920938463463374607431768211456"
    );
    // 2^128 - 1 (a borrow chain across every limb).
    assert_eq!(
        two128.sub(&BigInt::from_u64(1)).to_string(),
        "340282366920938463463374607431768211455"
    );
    // u64::MAX + 1 = 2^64 (a carry that grows a limb).
    assert_eq!(BigInt::from_u64(u64::MAX).add(&BigInt::from_u64(1)), two64);
    // Division with a multi-limb divisor: 2^128 / 2^64 = 2^64, remainder 0.
    let (q, rem) = two128.div_rem(&two64).unwrap();
    assert_eq!(q, two64);
    assert!(rem.is_zero());
}

#[test]
fn pow_is_exact_and_unbounded() {
    assert_eq!(b(2).pow(63).to_string(), "9223372036854775808"); // the value i64 can't hold
    assert_eq!(b(2).pow(100).to_string(), "1267650600228229401496703205376");
    assert_eq!(b(10).pow(0).to_string(), "1");
    assert_eq!(b(0).pow(0).to_string(), "1");
    assert_eq!(b(-3).pow(3).to_string(), "-27");
    assert_eq!(b(-2).pow(10).to_string(), "1024");
    // 3^50 cross-checked against repeated multiplication.
    let mut acc = BigInt::from_u64(1);
    for _ in 0..50 {
        acc = acc.mul(&b(3));
    }
    assert_eq!(b(3).pow(50), acc);
}

#[test]
fn ordering_is_total_and_sign_aware() {
    let mut v = vec![
        b(3),
        b(-5),
        BigInt::zero(),
        b(i64::MAX),
        b(i64::MIN),
        b(-5).mul(&b(i64::MAX)),
    ];
    v.sort();
    let as_str: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    assert_eq!(
        as_str,
        vec![
            "-46116860184273879035", // -5 * i64::MAX
            "-9223372036854775808",  // i64::MIN
            "-5",
            "0",
            "3",
            "9223372036854775807", // i64::MAX
        ]
    );
}

// -----------------------------------------------------------------
// Rational
// -----------------------------------------------------------------

fn r(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).expect("nonzero denominator in test")
}

#[test]
fn rational_reduces_to_lowest_terms_on_construction() {
    assert_eq!(r(6, 8).to_string(), "3/4");
    assert_eq!(r(10, 5).to_string(), "2");
    assert_eq!(r(0, 7).to_string(), "0");
    assert_eq!(r(100, 1000).to_string(), "1/10");
    // The reduced form is canonical, so equal values are structurally equal.
    assert_eq!(r(6, 8), r(3, 4));
    assert_eq!(r(0, 7), r(0, 1));
}

#[test]
fn rational_normalizes_sign_onto_a_positive_denominator() {
    assert_eq!(r(1, -2).to_string(), "-1/2");
    assert_eq!(r(-1, -2).to_string(), "1/2");
    assert_eq!(r(-3, 4), r(3, -4));
    assert!(r(1, -2).is_negative());
    assert!(!r(-1, -2).is_negative());
    // The denominator accessor is always positive.
    assert!(!r(1, -2).denominator().is_negative());
}

#[test]
fn rational_zero_denominator_is_none() {
    assert!(Rational::from_ratio_i64(5, 0).is_none());
    assert!(Rational::new(BigInt::from_i64(1), BigInt::zero()).is_none());
    assert!(r(3, 4).div(&Rational::zero()).is_none());
    assert!(Rational::zero().recip().is_none());
    assert!(Rational::parse("7/0").is_none());
}

#[test]
fn rational_arithmetic_is_exact() {
    assert_eq!(r(1, 3).add(&r(1, 6)).to_string(), "1/2");
    assert_eq!(r(1, 2).sub(&r(1, 3)).to_string(), "1/6");
    assert_eq!(r(2, 3).mul(&r(3, 4)).to_string(), "1/2");
    assert_eq!(r(1, 2).div(&r(3, 4)).unwrap().to_string(), "2/3");
    // a + (-a) == 0, a * (1/a) == 1.
    assert!(r(5, 7).add(&r(5, 7).negated()).is_zero());
    assert_eq!(r(5, 7).mul(&r(5, 7).recip().unwrap()), Rational::one());
}

#[test]
fn one_third_stays_exact_where_a_json_double_would_round() {
    // The whole point: 1/3 is EXACT, not 0.3333…; three of them are exactly 1.
    let third = r(1, 3);
    let sum = third.add(&third).add(&third);
    assert_eq!(sum, Rational::one());
    assert_eq!(third.to_string(), "1/3");
    // The classic f64 trap 0.1 + 0.2 != 0.3 — Rationals don't have it.
    assert_ne!(0.1_f64 + 0.2, 0.3);
    assert_eq!(r(1, 10).add(&r(2, 10)), r(3, 10));
}

#[test]
fn rational_ordering_cross_multiplies_without_rounding() {
    assert!(r(1, 3) < r(1, 2));
    assert!(r(-1, 2) < r(1, 3));
    assert!(r(2, 4) == r(1, 2));
    let mut v = vec![
        r(1, 2),
        r(1, 3),
        r(-1, 4),
        r(2, 3),
        Rational::zero(),
        r(1, 2),
    ];
    v.sort();
    let s: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    assert_eq!(s, ["-1/4", "0", "1/3", "1/2", "1/2", "2/3"]);
}

#[test]
fn rational_terms_overflow_i64_into_bigint() {
    // (1/i64::MAX) + (1/i64::MAX) = 2/i64::MAX — denominator past i64 stays exact.
    let big = Rational::new(BigInt::from_i64(1), BigInt::from_i64(i64::MAX)).unwrap();
    let twice = big.add(&big);
    assert_eq!(twice.numerator().to_string(), "2");
    assert_eq!(twice.denominator().to_string(), i64::MAX.to_string());
    // A product whose numerator escapes i64 is still exact.
    let p = r(i64::MAX, 1).mul(&r(3, 1));
    assert_eq!(p.numerator().to_i64(), None);
    assert_eq!(
        p.numerator().to_string(),
        (i64::MAX as i128 * 3).to_string()
    );
}

#[test]
fn rational_pow_handles_negative_and_zero_exponents() {
    assert_eq!(r(2, 3).pow(3).unwrap().to_string(), "8/27");
    assert_eq!(r(2, 3).pow(0).unwrap(), Rational::one());
    assert_eq!(r(2, 3).pow(-2).unwrap().to_string(), "9/4");
    assert_eq!(r(-2, 3).pow(-3).unwrap().to_string(), "-27/8");
    assert!(Rational::zero().pow(-1).is_none());
    assert_eq!(Rational::zero().pow(3).unwrap(), Rational::zero());
}

#[test]
fn rational_parse_round_trips_and_rejects_garbage() {
    assert_eq!(Rational::parse("3/4").unwrap().to_string(), "3/4");
    assert_eq!(Rational::parse("-3/4").unwrap().to_string(), "-3/4");
    assert_eq!(Rational::parse("6/8").unwrap().to_string(), "3/4");
    assert_eq!(Rational::parse("5").unwrap().to_string(), "5");
    assert_eq!(Rational::parse("  7 / 14 ").unwrap().to_string(), "1/2");
    assert!(Rational::parse("abc").is_none());
    assert!(Rational::parse("1/2/3").is_none());
}

#[test]
fn rational_integer_predicate_and_narrowing() {
    assert!(r(10, 2).is_integer());
    assert_eq!(r(10, 2).to_i64(), Some(5));
    assert_eq!(r(10, 2).to_bigint().unwrap().to_string(), "5");
    assert!(!r(3, 4).is_integer());
    assert_eq!(r(3, 4).to_i64(), None);
    assert!(r(3, 4).to_bigint().is_none());
}

#[test]
fn rational_floor_and_ceil_round_toward_neg_and_pos_infinity() {
    assert_eq!(r(7, 2).floor().to_string(), "3");
    assert_eq!(r(7, 2).ceil().to_string(), "4");
    assert_eq!(r(-7, 2).floor().to_string(), "-4");
    assert_eq!(r(-7, 2).ceil().to_string(), "-3");
    // Whole values floor/ceil to themselves.
    assert_eq!(r(6, 2).floor().to_string(), "3");
    assert_eq!(r(6, 2).ceil().to_string(), "3");
    // round ties away from zero, matching f64::round.
    assert_eq!(r(7, 2).round().to_string(), "4");
    assert_eq!(r(-7, 2).round().to_string(), "-4");
    assert_eq!(r(1, 3).round().to_string(), "0");
    assert_eq!(r(2, 3).round().to_string(), "1");
    // Differential vs f64 on a dense grid (small terms are exact in f64).
    for n in -9i64..=9 {
        for d in 1i64..=9 {
            let q = r(n, d);
            assert_eq!(
                q.floor().to_i64(),
                Some((n as f64 / d as f64).floor() as i64),
                "{n}/{d} floor"
            );
            assert_eq!(
                q.ceil().to_i64(),
                Some((n as f64 / d as f64).ceil() as i64),
                "{n}/{d} ceil"
            );
            assert_eq!(
                q.round().to_i64(),
                Some((n as f64 / d as f64).round() as i64),
                "{n}/{d} round"
            );
        }
    }
}

#[test]
fn rational_to_f64_matches_the_division_on_small_terms() {
    for n in -8i64..=8 {
        for d in 1i64..=8 {
            let approx = r(n, d).to_f64();
            assert!((approx - (n as f64 / d as f64)).abs() < 1e-12, "{n}/{d}");
        }
    }
}

#[test]
fn rational_obeys_the_field_laws_over_random_fractions() {
    // Differential/property fuzz: build fractions from random i64 terms and
    // check the field axioms exactly (no rounding, unlike f64).
    let mut rng = Rng(0x5A7F_104E_2C19_8B63);
    for _ in 0..2000 {
        let pick = |rng: &mut Rng| -> i64 { (rng.next() % 4001) as i64 - 2000 };
        let (a, b, c, d, e, g) = (
            pick(&mut rng),
            pick(&mut rng),
            pick(&mut rng),
            pick(&mut rng),
            pick(&mut rng),
            pick(&mut rng),
        );
        let (Some(x), Some(y), Some(z)) = (
            Rational::from_ratio_i64(a, b.max(1)),
            Rational::from_ratio_i64(c, d.max(1)),
            Rational::from_ratio_i64(e, g.max(1)),
        ) else {
            continue;
        };
        // commutativity
        assert_eq!(x.add(&y), y.add(&x));
        assert_eq!(x.mul(&y), y.mul(&x));
        // associativity
        assert_eq!(x.add(&y).add(&z), x.add(&y.add(&z)));
        assert_eq!(x.mul(&y).mul(&z), x.mul(&y.mul(&z)));
        // distributivity
        assert_eq!(x.mul(&y.add(&z)), x.mul(&y).add(&x.mul(&z)));
        // additive inverse + subtraction agreement
        assert!(x.add(&x.negated()).is_zero());
        assert_eq!(x.sub(&y), x.add(&y.negated()));
        // multiplicative inverse (when nonzero)
        if !x.is_zero() {
            assert_eq!(x.mul(&x.recip().unwrap()), Rational::one());
            assert_eq!(y.div(&x).unwrap(), y.mul(&x.recip().unwrap()));
        }
    }
}

// -----------------------------------------------------------------
// Decimal — exact base-10 fixed-point
// -----------------------------------------------------------------

fn dec(s: &str) -> Decimal {
    Decimal::parse(s).unwrap_or_else(|| panic!("parse decimal {s:?}"))
}

#[test]
fn decimal_parse_and_display_round_trip() {
    for s in [
        "0", "19.99", "0.05", "-0.005", "1999", "100.00", "-7", "0.10", "3.14159",
    ] {
        assert_eq!(dec(s).to_string(), s, "round trip {s}");
    }
    // "-0" and "+5" normalize on display.
    assert_eq!(dec("-0").to_string(), "0");
    assert_eq!(dec("+5").to_string(), "5");
    // leading/trailing forms.
    assert_eq!(dec(".5").to_string(), "0.5");
    assert_eq!(dec("5.").to_string(), "5");
    // garbage rejected.
    assert!(Decimal::parse("1.2.3").is_none());
    assert!(Decimal::parse("abc").is_none());
    assert!(Decimal::parse("").is_none());
    assert!(Decimal::parse("-").is_none());
    assert!(Decimal::parse(".").is_none());
}

#[test]
fn decimal_has_no_binary_float_drift() {
    // The headline: 0.1 + 0.2 == 0.3 EXACTLY (the classic f64 trap), and is shown so.
    assert_ne!(0.1_f64 + 0.2, 0.3);
    assert_eq!(dec("0.1").add(&dec("0.2")), dec("0.3"));
    assert_eq!(dec("0.1").add(&dec("0.2")).to_string(), "0.3");
}

#[test]
fn decimal_add_sub_align_scales() {
    assert_eq!(dec("19.99").add(&dec("0.01")).to_string(), "20.00");
    assert_eq!(dec("1.1").add(&dec("2.22")).to_string(), "3.32");
    assert_eq!(dec("5").add(&dec("0.5")).to_string(), "5.5");
    assert_eq!(dec("20.00").sub(&dec("0.01")).to_string(), "19.99");
    assert_eq!(dec("0").sub(&dec("0.005")).to_string(), "-0.005");
}

#[test]
fn decimal_mul_adds_scales_and_is_exact() {
    assert_eq!(dec("1.1").mul(&dec("1.1")).to_string(), "1.21");
    assert_eq!(dec("0.05").mul(&dec("0.05")).to_string(), "0.0025");
    assert_eq!(dec("19.99").mul(&Decimal::from_i64(3)).to_string(), "59.97");
    assert_eq!(dec("-2.5").mul(&dec("4")).to_string(), "-10.0");
}

#[test]
fn decimal_div_rounds_to_scale() {
    // 10/3 = 3.333… → 3.33 at scale 2.
    assert_eq!(
        dec("10")
            .div(&dec("3"), 2, RoundingMode::HalfEven)
            .unwrap()
            .to_string(),
        "3.33"
    );
    // 1/8 = 0.125 exact at scale 3.
    assert_eq!(
        dec("1")
            .div(&dec("8"), 3, RoundingMode::HalfEven)
            .unwrap()
            .to_string(),
        "0.125"
    );
    // At scale 2 the tie 0.125 splits by mode: HalfEven → 0.12, HalfUp → 0.13.
    assert_eq!(
        dec("1")
            .div(&dec("8"), 2, RoundingMode::HalfEven)
            .unwrap()
            .to_string(),
        "0.12"
    );
    assert_eq!(
        dec("1")
            .div(&dec("8"), 2, RoundingMode::HalfUp)
            .unwrap()
            .to_string(),
        "0.13"
    );
    // Divide by zero is None, never a panic.
    assert!(dec("1").div(&dec("0"), 2, RoundingMode::HalfEven).is_none());
}

#[test]
fn decimal_rescale_quantizes_with_rounding() {
    assert_eq!(
        dec("19.999").rescale(2, RoundingMode::HalfEven).to_string(),
        "20.00"
    );
    // Banker's rounding: 2.5 → 2, 3.5 → 4, -2.5 → -2 (ties to even).
    assert_eq!(
        dec("2.5").rescale(0, RoundingMode::HalfEven).to_string(),
        "2"
    );
    assert_eq!(
        dec("3.5").rescale(0, RoundingMode::HalfEven).to_string(),
        "4"
    );
    assert_eq!(
        dec("-2.5").rescale(0, RoundingMode::HalfEven).to_string(),
        "-2"
    );
    // Widening just appends zeros (exact).
    assert_eq!(
        dec("1.5").rescale(4, RoundingMode::HalfEven).to_string(),
        "1.5000"
    );
}

#[test]
fn decimal_rounding_modes_on_a_tie_and_a_non_tie() {
    // 2.5 (an exact tie) under every mode.
    let tie = dec("2.5");
    for (mode, want) in [
        (RoundingMode::Down, "2"),
        (RoundingMode::Up, "3"),
        (RoundingMode::Floor, "2"),
        (RoundingMode::Ceiling, "3"),
        (RoundingMode::HalfUp, "3"),
        (RoundingMode::HalfDown, "2"),
        (RoundingMode::HalfEven, "2"),
    ] {
        assert_eq!(tie.rescale(0, mode).to_string(), want, "2.5 under {mode:?}");
    }
    // -2.5 (negative tie): Floor → -3, Ceiling → -2, HalfUp → -3.
    let ntie = dec("-2.5");
    assert_eq!(ntie.rescale(0, RoundingMode::Floor).to_string(), "-3");
    assert_eq!(ntie.rescale(0, RoundingMode::Ceiling).to_string(), "-2");
    assert_eq!(ntie.rescale(0, RoundingMode::HalfUp).to_string(), "-3");
    assert_eq!(ntie.rescale(0, RoundingMode::Down).to_string(), "-2");
}

#[test]
fn decimal_equality_is_value_based_and_hash_agrees() {
    use std::collections::HashSet;
    // 1.0 == 1.00 == 1 by VALUE; scale is only a display hint.
    assert_eq!(dec("1.0"), dec("1.00"));
    assert_eq!(dec("1.0"), dec("1"));
    assert_eq!(dec("0.0"), dec("0"));
    // Equal values must hash equal (the Eq/Hash contract).
    let mut set = HashSet::new();
    set.insert(dec("1.00"));
    assert!(set.contains(&dec("1")));
    assert!(set.contains(&dec("1.0")));
    // Different values are not equal.
    assert_ne!(dec("1.0"), dec("1.01"));
}

#[test]
fn decimal_ordering_compares_across_scales_without_rounding() {
    assert!(dec("0.1") < dec("0.11"));
    assert!(dec("1.5") < dec("2"));
    assert!(dec("-0.005") < dec("0"));
    assert!(dec("1.00") == dec("1.0"));
    let mut v = vec![dec("0.5"), dec("0.05"), dec("-0.1"), dec("2"), dec("0.50")];
    v.sort();
    let s: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    // 0.5 and 0.50 are equal; sort is stable so input order among equals is kept.
    assert_eq!(s, ["-0.1", "0.05", "0.5", "0.50", "2"]);
}

#[test]
fn decimal_to_and_from_rational_is_exact() {
    assert_eq!(
        dec("19.99").to_rational(),
        Rational::from_ratio_i64(1999, 100).unwrap()
    );
    assert_eq!(
        dec("0.125").to_rational(),
        Rational::from_ratio_i64(1, 8).unwrap()
    );
    assert_eq!(
        dec("-0.005").to_rational(),
        Rational::from_ratio_i64(-1, 200).unwrap()
    );
    // from_rational rounds to the requested scale.
    let third = Rational::from_ratio_i64(1, 3).unwrap();
    assert_eq!(
        Decimal::from_rational(&third, 4, RoundingMode::HalfEven).to_string(),
        "0.3333"
    );
}

#[test]
fn decimal_wire_components_round_trip() {
    for s in ["0", "19.99", "-0.005", "100.00", "123456789.000001", "-7"] {
        let d = dec(s);
        let (neg, bytes, scale) = d.to_le_bytes();
        let back = Decimal::from_le_bytes(neg, &bytes, scale);
        assert_eq!(back, d, "value round-trip {s}");
        assert_eq!(back.to_string(), d.to_string(), "display round-trip {s}");
    }
}

#[test]
fn decimal_money_scenario_is_exact() {
    // 3 items at $19.99, plus 8% tax rounded to cents (banker's).
    let subtotal = dec("19.99").mul(&Decimal::from_i64(3)); // 59.97
    assert_eq!(subtotal.to_string(), "59.97");
    let tax = subtotal
        .mul(&dec("0.08"))
        .rescale(2, RoundingMode::HalfEven); // 4.7976 → 4.80
    assert_eq!(tax.to_string(), "4.80");
    assert_eq!(subtotal.add(&tax).to_string(), "64.77");
}

#[test]
fn decimal_add_sub_mul_match_the_rational_oracle_under_fuzz() {
    // Differential: build decimals from random coeff/scale and check that +,-,*
    // agree EXACTLY with the Rational tower (the exactness oracle).
    let mut rng = Rng(0xDEC1_2A1B_0000_FACE);
    for _ in 0..3000 {
        let ca = (rng.next() % 2_000_001) as i64 - 1_000_000;
        let cb = (rng.next() % 2_000_001) as i64 - 1_000_000;
        let sa = (rng.next() % 6) as u32;
        let sb = (rng.next() % 6) as u32;
        let a = Decimal::from_coeff_scale(BigInt::from_i64(ca), sa);
        let b = Decimal::from_coeff_scale(BigInt::from_i64(cb), sb);
        let (ra, rb) = (a.to_rational(), b.to_rational());
        assert_eq!(a.add(&b).to_rational(), ra.add(&rb), "add {a} {b}");
        assert_eq!(a.sub(&b).to_rational(), ra.sub(&rb), "sub {a} {b}");
        assert_eq!(a.mul(&b).to_rational(), ra.mul(&rb), "mul {a} {b}");
        // div is rounded: its result must be within one ULP of the requested scale.
        if !b.is_zero() {
            let scale = 8u32;
            let q = a.div(&b, scale, RoundingMode::HalfEven).unwrap();
            let exact = ra.div(&rb).unwrap();
            let ulp = Rational::new(BigInt::from_i64(1), BigInt::from_u64(10).pow(scale)).unwrap();
            let err = q.to_rational().sub(&exact).abs();
            // |rounded - exact| <= ulp/2 for nearest-rounding.
            assert!(
                err.mul(&Rational::from_i64(2)) <= ulp,
                "div within half-ulp {a}/{b}"
            );
        }
    }
}

// -----------------------------------------------------------------
// Complex — exact Gaussian rationals
// -----------------------------------------------------------------

fn c(re: i64, im: i64) -> Complex {
    Complex::new(Rational::from_i64(re), Rational::from_i64(im))
}

fn cq(rn: i64, rd: i64, in_: i64, id: i64) -> Complex {
    Complex::new(
        Rational::from_ratio_i64(rn, rd).unwrap(),
        Rational::from_ratio_i64(in_, id).unwrap(),
    )
}

#[test]
fn complex_i_squared_is_minus_one() {
    // The headline: i·i == −1, EXACTLY.
    assert_eq!(Complex::i().mul(&Complex::i()), c(-1, 0));
    assert_eq!(Complex::i().mul(&Complex::i()), Complex::from_i64(-1));
}

#[test]
fn complex_construction_and_accessors() {
    let z = c(3, 4);
    assert_eq!(z.re(), &Rational::from_i64(3));
    assert_eq!(z.im(), &Rational::from_i64(4));
    assert!(!z.is_real());
    assert!(c(5, 0).is_real());
    assert!(Complex::zero().is_zero());
    assert!(!Complex::i().is_zero());
    assert_eq!(Complex::from_i64(7), c(7, 0));
}

#[test]
fn complex_add_sub_mul_are_exact() {
    assert_eq!(c(2, 3).add(&c(1, -1)), c(3, 2));
    assert_eq!(c(2, 3).sub(&c(1, -1)), c(1, 4));
    // (1+i)(1−i) = 1 − i² = 2 — the classic exact identity.
    assert_eq!(c(1, 1).mul(&c(1, -1)), c(2, 0));
    // (2+3i)(4+5i) = 8 + 10i + 12i + 15i² = (8−15) + 22i = −7 + 22i.
    assert_eq!(c(2, 3).mul(&c(4, 5)), c(-7, 22));
}

#[test]
fn complex_div_recip_and_zero_guard() {
    // z/z == 1 for any nonzero z.
    assert_eq!(c(2, 3).div(&c(2, 3)).unwrap(), Complex::one());
    // 1/i == −i.
    assert_eq!(Complex::i().recip().unwrap(), c(0, -1));
    // (3+4i)/(1+2i) = (3+4i)(1−2i)/5 = (11−2i)/5.
    assert_eq!(c(3, 4).div(&c(1, 2)).unwrap(), cq(11, 5, -2, 5));
    // Division/reciprocal of zero is None, never a panic.
    assert!(c(1, 1).div(&Complex::zero()).is_none());
    assert!(Complex::zero().recip().is_none());
}

#[test]
fn complex_conjugate_and_norm_are_exact() {
    assert_eq!(c(3, 4).conjugate(), c(3, -4));
    assert_eq!(c(3, 4).norm_sqr(), Rational::from_i64(25));
    // |3+4i| = 5 exactly (a Pythagorean magnitude).
    assert!((c(3, 4).abs_f64() - 5.0).abs() < 1e-12);
    // z · conj(z) == |z|² (a real number).
    let z = c(2, -5);
    assert_eq!(z.mul(&z.conjugate()), Complex::from_rational(z.norm_sqr()));
}

#[test]
fn complex_pow_handles_the_cycle_and_negatives() {
    assert_eq!(Complex::i().pow(2).unwrap(), c(-1, 0));
    assert_eq!(Complex::i().pow(3).unwrap(), c(0, -1));
    assert_eq!(Complex::i().pow(4).unwrap(), c(1, 0));
    assert_eq!(Complex::i().pow(0).unwrap(), Complex::one());
    assert_eq!(Complex::i().pow(-1).unwrap(), c(0, -1)); // 1/i = −i
                                                         // (1+i)^2 = 2i.
    assert_eq!(c(1, 1).pow(2).unwrap(), c(0, 2));
    // 0^0 == 1; 0 to a negative power is undefined.
    assert_eq!(Complex::zero().pow(0).unwrap(), Complex::one());
    assert!(Complex::zero().pow(-1).is_none());
}

#[test]
fn complex_display_round_trips_every_form() {
    for (z, want) in [
        (c(0, 0), "0"),
        (c(3, 0), "3"),
        (c(0, 1), "i"),
        (c(0, -1), "-i"),
        (c(0, 4), "4i"),
        (c(0, -4), "-4i"),
        (c(3, 4), "3+4i"),
        (c(3, -4), "3-4i"),
        (c(3, 1), "3+i"),
        (c(3, -1), "3-i"),
        (c(-2, 5), "-2+5i"),
    ] {
        assert_eq!(z.to_string(), want, "display");
        assert_eq!(
            Complex::parse(want).unwrap(),
            z,
            "parse round-trip of {want}"
        );
    }
    // Fractional parts round-trip too.
    let z = cq(1, 2, -3, 4);
    assert_eq!(z.to_string(), "1/2-3/4i");
    assert_eq!(Complex::parse("1/2-3/4i").unwrap(), z);
    // Garbage is rejected.
    assert!(Complex::parse("").is_none());
    assert!(Complex::parse("3+xi").is_none());
}

#[test]
fn complex_equality_and_hash_are_structural() {
    use std::collections::HashSet;
    assert_eq!(c(3, 4), c(3, 4));
    assert_ne!(c(3, 4), c(3, -4));
    assert_eq!(cq(2, 4, 6, 8), cq(1, 2, 3, 4)); // reduced parts compare equal
    let mut set = HashSet::new();
    set.insert(cq(2, 4, 6, 8));
    assert!(set.contains(&cq(1, 2, 3, 4)));
}

#[test]
fn complex_obeys_the_field_laws_under_fuzz() {
    // Property fuzz: random Gaussian rationals satisfy the field axioms EXACTLY,
    // plus the conjugate/norm homomorphisms.
    let mut rng = Rng(0xC011_9EE5_A11C_E5ED);
    let pick = |rng: &mut Rng| -> Rational {
        let n = (rng.next() % 41) as i64 - 20;
        let d = ((rng.next() % 9) as i64) + 1;
        Rational::from_ratio_i64(n, d).unwrap()
    };
    let pick_c = |rng: &mut Rng| -> Complex { Complex::new(pick(rng), pick(rng)) };
    for _ in 0..2000 {
        let (x, y, z) = (pick_c(&mut rng), pick_c(&mut rng), pick_c(&mut rng));
        // commutativity
        assert_eq!(x.add(&y), y.add(&x));
        assert_eq!(x.mul(&y), y.mul(&x));
        // associativity
        assert_eq!(x.add(&y).add(&z), x.add(&y.add(&z)));
        assert_eq!(x.mul(&y).mul(&z), x.mul(&y.mul(&z)));
        // distributivity
        assert_eq!(x.mul(&y.add(&z)), x.mul(&y).add(&x.mul(&z)));
        // additive inverse + subtraction agreement
        assert!(x.add(&x.negated()).is_zero());
        assert_eq!(x.sub(&y), x.add(&y.negated()));
        // conjugate is an involution and a ring homomorphism
        assert_eq!(x.conjugate().conjugate(), x);
        assert_eq!(x.mul(&y).conjugate(), x.conjugate().mul(&y.conjugate()));
        // the norm is multiplicative: |xy|² = |x|²·|y|²
        assert_eq!(x.mul(&y).norm_sqr(), x.norm_sqr().mul(&y.norm_sqr()));
        // multiplicative inverse (when nonzero)
        if !x.is_zero() {
            assert_eq!(x.mul(&x.recip().unwrap()), Complex::one());
            assert_eq!(y.div(&x).unwrap(), y.mul(&x.recip().unwrap()));
        }
    }
}

// -----------------------------------------------------------------
// Modular — the ring ℤ/nℤ
// -----------------------------------------------------------------

fn m(v: i64, n: i64) -> Modular {
    Modular::from_i64(v, n).unwrap_or_else(|| panic!("bad modulus {n}"))
}

#[test]
fn modular_reduces_into_range_on_construction() {
    assert_eq!(m(10, 7).value().to_i64(), Some(3));
    assert_eq!(m(7, 7).value().to_i64(), Some(0));
    // Negatives become non-negative residues (Euclidean): −1 mod 7 = 6.
    assert_eq!(m(-1, 7).value().to_i64(), Some(6));
    assert_eq!(m(-10, 7).value().to_i64(), Some(4));
    // Modulus 1 collapses everything to 0 (the trivial ring).
    assert_eq!(m(5, 1).value().to_i64(), Some(0));
    // A non-positive modulus is rejected.
    assert!(Modular::from_i64(3, 0).is_none());
    assert!(Modular::from_i64(3, -7).is_none());
}

#[test]
fn modular_arithmetic_wraps_in_the_ring() {
    // Add/sub/mul stay in [0, n) — overflow IS the point (cyclic group).
    assert_eq!(m(5, 7).add(&m(4, 7)).unwrap(), m(2, 7)); // 9 ≡ 2
    assert_eq!(m(3, 7).sub(&m(5, 7)).unwrap(), m(5, 7)); // −2 ≡ 5
    assert_eq!(m(4, 7).mul(&m(5, 7)).unwrap(), m(6, 7)); // 20 ≡ 6
                                                         // −x is the additive inverse.
    assert_eq!(m(3, 7).negated(), m(4, 7));
    assert!(m(3, 7).add(&m(3, 7).negated()).unwrap().is_zero());
    // A modulus mismatch is refused (a ring mismatch, like a Word width mismatch).
    assert!(m(3, 7).add(&m(3, 5)).is_none());
    assert!(m(3, 7).mul(&m(3, 5)).is_none());
}

#[test]
fn modular_exponentiation_is_fast_and_exact() {
    assert_eq!(m(3, 5).pow(4), m(1, 5)); // 81 ≡ 1 (mod 5)
    assert_eq!(m(2, 7).pow(3), m(1, 7)); // 8 ≡ 1 (mod 7)
    assert_eq!(m(5, 13).pow(0), m(1, 13)); // x^0 = 1
                                           // A large exponent that fast-exp makes tractable: 7^100 (mod 13).
    let mut acc = m(1, 13);
    for _ in 0..100 {
        acc = acc.mul(&m(7, 13)).unwrap();
    }
    assert_eq!(m(7, 13).pow(100), acc);
}

#[test]
fn modular_inverse_and_division() {
    // 3⁻¹ ≡ 5 (mod 7) since 3·5 = 15 ≡ 1.
    assert_eq!(m(3, 7).inverse().unwrap(), m(5, 7));
    assert_eq!(m(3, 7).mul(&m(3, 7).inverse().unwrap()).unwrap(), m(1, 7));
    // Division is multiplication by the inverse: 1/3 ≡ 5 (mod 7).
    assert_eq!(m(1, 7).div(&m(3, 7)).unwrap(), m(5, 7));
    assert_eq!(m(4, 7).div(&m(2, 7)).unwrap(), m(2, 7));
    // A non-unit (gcd ≠ 1) has NO inverse: 2 is not invertible mod 4.
    assert!(m(2, 4).inverse().is_none());
    assert!(m(1, 4).div(&m(2, 4)).is_none());
    // Division across different moduli is refused.
    assert!(m(1, 7).div(&m(3, 5)).is_none());
}

#[test]
fn modular_generalizes_the_word_wrapping_ring() {
    // ℤ/2³²ℤ IS the Word32 ring: add/mul agree with the fixed-width wrapping ops.
    let modulus = 1i64 << 32; // 2^32 = 4_294_967_296
    let (a, b) = (4_000_000_000u32, 1_000_000_000u32); // a + b overflows u32
    let word_add = (crate::Word32(a) + crate::Word32(b)).0 as i64;
    let mod_add = m(a as i64, modulus).add(&m(b as i64, modulus)).unwrap();
    assert_eq!(
        mod_add.value().to_i64(),
        Some(word_add),
        "ℤ/2³² add == Word32 add"
    );
    let word_mul = (crate::Word32(a) * crate::Word32(b)).0 as i64;
    let mod_mul = m(a as i64, modulus).mul(&m(b as i64, modulus)).unwrap();
    assert_eq!(
        mod_mul.value().to_i64(),
        Some(word_mul),
        "ℤ/2³² mul == Word32 mul"
    );
}

#[test]
fn modular_equality_is_per_ring_and_displays_the_modulus() {
    // Same residue in DIFFERENT rings is not equal (the modulus is part of the value).
    assert_ne!(m(3, 7), m(3, 5));
    assert_eq!(m(10, 7), m(3, 7));
    assert_eq!(m(3, 7).to_string(), "3 (mod 7)");
    assert_eq!(m(-1, 7).to_string(), "6 (mod 7)");
}

#[test]
fn modular_obeys_fermats_little_theorem() {
    // For a prime p and a not divisible by p: a^(p−1) ≡ 1 (mod p).
    for &p in &[2i64, 3, 5, 7, 13, 97, 101] {
        for a in 1..p.min(40) {
            assert_eq!(
                m(a, p).pow((p - 1) as u64),
                m(1, p),
                "{a}^({p}-1) ≡ 1 (mod {p})"
            );
        }
    }
}

#[test]
fn modular_obeys_the_ring_laws_and_inverse_under_fuzz() {
    // Property fuzz: random residues over random moduli satisfy the commutative-ring
    // axioms exactly, and every unit times its inverse is 1.
    let mut rng = Rng(0x_0DDF_ACE5_ABCD_1234);
    for _ in 0..3000 {
        let n = ((rng.next() % 50) as i64) + 2; // modulus in [2, 51]
        let pick = |rng: &mut Rng| (rng.next() % 1_000_000) as i64 - 500_000;
        let (a, b, c) = (
            m(pick(&mut rng), n),
            m(pick(&mut rng), n),
            m(pick(&mut rng), n),
        );
        // Commutativity.
        assert_eq!(a.add(&b).unwrap(), b.add(&a).unwrap());
        assert_eq!(a.mul(&b).unwrap(), b.mul(&a).unwrap());
        // Associativity.
        assert_eq!(
            a.add(&b).unwrap().add(&c).unwrap(),
            a.add(&b.add(&c).unwrap()).unwrap()
        );
        assert_eq!(
            a.mul(&b).unwrap().mul(&c).unwrap(),
            a.mul(&b.mul(&c).unwrap()).unwrap()
        );
        // Distributivity.
        assert_eq!(
            a.mul(&b.add(&c).unwrap()).unwrap(),
            a.mul(&b).unwrap().add(&a.mul(&c).unwrap()).unwrap()
        );
        // Additive inverse.
        assert!(a.add(&a.negated()).unwrap().is_zero());
        // Multiplicative inverse for units (gcd(a, n) == 1).
        if let Some(inv) = a.inverse() {
            assert_eq!(a.mul(&inv).unwrap(), m(1, n), "a·a⁻¹ = 1 in ℤ/{n}ℤ");
        }
    }
}

// ─── Exact float interop ────────────────────────────────────────────

#[test]
fn exact_rational_from_f64_values() {
    assert_eq!(
        rational_from_f64_exact(0.5).unwrap(),
        Rational::from_ratio_i64(1, 2).unwrap()
    );
    assert_eq!(rational_from_f64_exact(3.0).unwrap(), Rational::from_i64(3));
    assert_eq!(
        rational_from_f64_exact(-2.25).unwrap(),
        Rational::from_ratio_i64(-9, 4).unwrap()
    );
    assert_eq!(rational_from_f64_exact(0.0).unwrap(), Rational::zero());
    assert_eq!(rational_from_f64_exact(-0.0).unwrap(), Rational::zero());
    assert!(rational_from_f64_exact(f64::NAN).is_none());
    assert!(rational_from_f64_exact(f64::INFINITY).is_none());
    // 0.1 is NOT 1/10 — it is the nearest double, an exact dyadic rational.
    assert_ne!(
        rational_from_f64_exact(0.1).unwrap(),
        Rational::from_ratio_i64(1, 10).unwrap()
    );
}

/// The dyadic rational ROUND-TRIPS: `rational_from_f64_exact(x).to_f64() == x` bit-for-bit. This is
/// the invariant the native-tier float-complex `re`/`im` extraction rides on (C `cplx_re_f64`, WASM
/// `logos_rt_complex_re_f64` = `rat_to_f64` of the dyadic part), so `re(complex(x, y)) == x` exactly.
#[test]
fn exact_rational_from_f64_round_trips() {
    // The exact parts the complex float tiers process: inverse-trig results, cConj/log10 shapes.
    for x in [
        0.0_f64,
        1.0,
        -1.0,
        2.0,
        0.5,
        0.125,
        0.25,
        3.0,
        -4.0,
        0.1,
        -0.1,
        0.5306375,
        -0.4522784,
        1.7320508075688772,
        3.141592653589793,
        100.0,
        -359.134,
    ] {
        assert_eq!(
            rational_from_f64_exact(x).unwrap().to_f64(),
            x,
            "dyadic rational round-trip failed for {x}"
        );
    }
}

#[test]
fn exact_i64_f64_cmp_at_the_representability_boundary() {
    let big = 9_007_199_254_740_993_i64; // 2^53 + 1 — not representable
    let f = 9_007_199_254_740_992.0_f64; // 2^53 — what the literal rounds to
                                         // A lossy `as f64` compare would say Equal; the exact one says Greater.
    assert_eq!(cmp_i64_f64_exact(big, f), Some(Ordering::Greater));
    assert_eq!(cmp_i64_f64_exact(big - 1, f), Some(Ordering::Equal));
    assert_eq!(cmp_i64_f64_exact(3, 3.5), Some(Ordering::Less));
    assert_eq!(cmp_i64_f64_exact(4, 3.5), Some(Ordering::Greater));
    assert_eq!(cmp_i64_f64_exact(1, 1.0), Some(Ordering::Equal));
    assert_eq!(cmp_i64_f64_exact(0, f64::NAN), None);
    assert_eq!(
        cmp_i64_f64_exact(i64::MAX, f64::INFINITY),
        Some(Ordering::Less)
    );
    assert_eq!(
        cmp_i64_f64_exact(i64::MIN, f64::NEG_INFINITY),
        Some(Ordering::Greater)
    );
}

#[test]
fn unified_numeric_hash_coherence() {
    // hash(Int k) == hash(Float k) for every exactly-representable k.
    for k in [
        0i64,
        1,
        -1,
        2,
        42,
        -42,
        1_000_000_007,
        -(1i64 << 53),
        1i64 << 53,
    ] {
        assert_eq!(
            numeric_hash_i64(k),
            numeric_hash_f64(k as f64),
            "int/float hash mismatch at {k}"
        );
        assert_eq!(
            numeric_hash_i64(k),
            numeric_hash_bigint(&BigInt::from_i64(k)),
            "int/bigint hash mismatch at {k}"
        );
        assert_eq!(
            numeric_hash_i64(k),
            numeric_hash_rational(&Rational::from_i64(k)),
            "int/rational hash mismatch at {k}"
        );
    }
    // Non-integral coherence: 0.5 == 1/2 exactly, so hashes agree.
    assert_eq!(
        numeric_hash_f64(0.5),
        numeric_hash_rational(&Rational::from_ratio_i64(1, 2).unwrap())
    );
    assert_eq!(
        numeric_hash_f64(-2.25),
        numeric_hash_rational(&Rational::from_ratio_i64(-9, 4).unwrap())
    );
    // -0.0 and 0.0 are equal, so they hash equal.
    assert_eq!(numeric_hash_f64(-0.0), numeric_hash_f64(0.0));
    // And a float hashes as its EXACT value: 0.1's hash equals the hash
    // of its exact dyadic rational, not of 1/10.
    assert_eq!(
        numeric_hash_f64(0.1),
        numeric_hash_rational(&rational_from_f64_exact(0.1).unwrap())
    );
}

// ---- the one-limb boundary (2^64): every op must cross it exactly ----

#[test]
fn arithmetic_is_exact_across_the_one_limb_boundary() {
    let u64max = BigInt::from_u64(u64::MAX);
    let two64 = u64max.add(&b(1)); // 2^64
    assert_eq!(two64.to_string(), "18446744073709551616");
    // Back below the boundary.
    assert_eq!(two64.sub(&b(1)), u64max);
    // Multiplication crossing: (2^32)² = 2^64; 2^62·4 = 2^64.
    let two32 = b(1i64 << 32);
    assert_eq!(two32.mul(&two32), two64);
    assert_eq!(b(1i64 << 62).mul(&b(4)), two64);
    // Division re-crossing: 2^64 / 2 = 2^63 (one limb again).
    let (q, r) = two64.div_rem(&b(2)).expect("nonzero divisor");
    assert!(r.is_zero());
    assert_eq!(q.to_string(), "9223372036854775808");
    // Negative mirror.
    let neg = two64.negated();
    assert_eq!(neg.add(&two64), BigInt::zero());
    assert_eq!(neg.to_string(), "-18446744073709551616");
}

#[test]
fn ordering_and_hash_are_canonical_across_the_limb_boundary() {
    let two64 = BigInt::from_u64(u64::MAX).add(&b(1));
    let big = two64.mul(&two64); // 2^128
    assert!(b(1) < two64);
    assert!(two64 < big);
    assert!(big.negated() < b(-1));
    assert!(two64.negated() < two64);
    // Equal values reached by different construction routes must be equal
    // AND hash equal — the representation is canonical, not path-dependent.
    let via_mul = b(1i64 << 32).mul(&b(1i64 << 32));
    let via_add = BigInt::from_u64(u64::MAX).add(&b(1));
    assert_eq!(via_mul, via_add);
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let h = |v: &BigInt| {
        let mut s = DefaultHasher::new();
        v.hash(&mut s);
        s.finish()
    };
    assert_eq!(h(&via_mul), h(&via_add));
    // A value shrunk back under the boundary equals its native construction.
    let shrunk = two64.sub(&b(1));
    assert_eq!(shrunk, BigInt::from_u64(u64::MAX));
    assert_eq!(h(&shrunk), h(&BigInt::from_u64(u64::MAX)));
}

#[test]
fn le_bytes_round_trip_across_the_limb_boundary() {
    for s in [
        "0",
        "1",
        "-1",
        "18446744073709551615",
        "18446744073709551616",
        "-18446744073709551616",
        "340282366920938463463374607431768211456",
    ] {
        let v = BigInt::parse_decimal(s).expect("valid decimal");
        let (neg, bytes) = v.to_le_bytes();
        assert_eq!(BigInt::from_le_bytes(neg, &bytes), v, "round trip {s}");
        assert_eq!(v.to_string(), s, "display {s}");
    }
}

#[test]
fn div_rem_identity_holds_for_multi_limb_values() {
    // a = q·b + r with |r| < |b| and r carrying the dividend's sign, at
    // sizes where single-limb arithmetic cannot apply.
    let a = b(1_000_003).pow(7); // ~2^140
    for dividend in [a.clone(), a.negated()] {
        for divisor in [b(97), b(-97), b(1i64 << 40), a.sub(&b(1))] {
            let (q, r) = dividend.div_rem(&divisor).expect("nonzero divisor");
            assert_eq!(dividend, q.mul(&divisor).add(&r));
            assert!(r.abs() < divisor.abs());
            assert!(r.is_zero() || r.is_negative() == dividend.is_negative());
        }
    }
}

/// The nearest binary64 to an integer, ties to even: the text of the integer read by the standard
/// library's decimal parser, which rounds exactly once.
fn nearest_f64(n: &BigInt) -> f64 {
    n.to_string()
        .parse::<f64>()
        .expect("an integer's decimal text parses as f64")
}

fn pow2(k: u64) -> BigInt {
    BigInt::from_u64(1).shl_bits(k)
}

#[test]
fn to_f64_rounds_once_where_a_limb_at_a_time_rounds_twice() {
    // (2^53 + 1)·2^64 + 1 lies just above the halfway point between 2^117 and 2^117 + 2^65. Rounding the
    // top limb first loses the +1 that decides the tie.
    let n = BigInt::from_u64((1u64 << 53) + 1)
        .shl_bits(64)
        .add(&BigInt::from_u64(1));
    assert_eq!(n.to_f64(), 2f64.powi(117) + 2f64.powi(65));
    assert_eq!(n.negated().to_f64(), -(2f64.powi(117) + 2f64.powi(65)));
}

#[test]
fn to_f64_breaks_an_exact_tie_toward_the_even_significand() {
    // (2^53 + 1)·2^64 is exactly halfway between 2^117 (even significand) and 2^117 + 2^65 (odd).
    let tie_down = BigInt::from_u64((1u64 << 53) + 1).shl_bits(64);
    assert_eq!(tie_down.to_f64(), 2f64.powi(117));
    // (2^53 + 3)·2^64 is halfway between 2^117 + 2^65 (odd) and 2^117 + 2^66 (even).
    let tie_up = BigInt::from_u64((1u64 << 53) + 3).shl_bits(64);
    assert_eq!(tie_up.to_f64(), 2f64.powi(117) + 2f64.powi(66));
}

#[test]
fn to_f64_is_exact_on_every_machine_integer_boundary() {
    for x in [
        0i64,
        1,
        -1,
        (1 << 53) - 1,
        1 << 53,
        (1 << 53) + 1,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
    ] {
        assert_eq!(BigInt::from_i64(x).to_f64(), x as f64, "{x}");
    }
    for x in [u64::MAX, u64::MAX - 1, 1u64 << 63, (1u64 << 63) + 1025] {
        assert_eq!(BigInt::from_u64(x).to_f64(), x as f64, "{x}");
    }
}

#[test]
fn to_f64_overflows_to_infinity_exactly_where_rounding_reaches_two_to_the_1024() {
    // f64::MAX = (2^53 − 1)·2^971. Halfway to 2^1024 is 2^1024 − 2^970; the tie goes to the even
    // significand, which is 2^1024 itself, so the halfway point is already infinite.
    let halfway = pow2(1024).sub(&pow2(970));
    assert_eq!(halfway.to_f64(), f64::INFINITY);
    assert_eq!(halfway.sub(&BigInt::from_u64(1)).to_f64(), f64::MAX);
    assert_eq!(halfway.negated().to_f64(), f64::NEG_INFINITY);
    assert_eq!(pow2(4096).to_f64(), f64::INFINITY);
    assert_eq!(pow2(4096).negated().to_f64(), f64::NEG_INFINITY);
}

#[test]
fn to_f64_is_the_nearest_float_for_random_integers_of_64_to_4096_bits() {
    let mut st = 0x9E37_79B9_7F4A_7C15u64;
    for round in 0..20_000u64 {
        let bits = 64 + xorshift(&mut st) % (4096 - 64 + 1);
        let limbs = ((bits + 63) / 64) as usize;
        let mut bytes = Vec::with_capacity(limbs * 8);
        for _ in 0..limbs {
            bytes.extend_from_slice(&xorshift(&mut st).to_le_bytes());
        }
        let magnitude = BigInt::from_le_bytes(false, &bytes)
            .shr_bits(limbs as u64 * 64 - bits)
            .expect("non-negative");
        let n = if round & 1 == 1 {
            magnitude.negated()
        } else {
            magnitude
        };
        assert_eq!(n.to_f64().to_bits(), nearest_f64(&n).to_bits(), "{n}");
    }
}

#[test]
fn to_f64_is_the_nearest_float_on_exact_halfway_cases_and_their_neighbours() {
    let mut st = 0xD1B5_4A32_D192_ED03u64;
    for _ in 0..5_000 {
        let significand = (xorshift(&mut st) >> 11) | (1u64 << 52);
        let shift = 1 + xorshift(&mut st) % 900;
        // significand·2^(shift+1) + 2^shift is exactly halfway between two neighbouring floats.
        let halfway = BigInt::from_u64(significand)
            .shl_bits(shift + 1)
            .add(&pow2(shift));
        for n in [
            halfway.clone(),
            halfway.add(&BigInt::from_u64(1)),
            halfway.sub(&BigInt::from_u64(1)),
        ] {
            for n in [n.clone(), n.negated()] {
                assert_eq!(n.to_f64().to_bits(), nearest_f64(&n).to_bits(), "{n}");
            }
        }
    }
}

fn next_toward(f: f64, up: bool) -> f64 {
    if f == 0.0 {
        let tiny = f64::from_bits(1);
        return if up { tiny } else { -tiny };
    }
    let bits = f.to_bits();
    let away = (f > 0.0) == up;
    f64::from_bits(if away { bits + 1 } else { bits - 1 })
}

fn assert_nearest(r: &Rational) {
    let f = rational_to_f64_nearest(r);
    assert!(f.is_finite(), "{r} is in range");
    let dist = |g: f64| r.sub(&rational_from_f64_exact(g).unwrap()).abs();
    let d = dist(f);
    for g in [next_toward(f, true), next_toward(f, false)] {
        let dg = dist(g);
        assert!(d <= dg, "{r} rounds to {f:e}, but {g:e} is nearer");
        if d == dg {
            assert_eq!(f.to_bits() & 1, 0, "{r} is a tie and must round to the even neighbour, not {f:e}");
        }
    }
}

#[test]
fn rational_to_f64_nearest_is_correctly_rounded() {
    let mut st = 0x0123_4567_89ab_cdefu64;
    for _ in 0..4_000 {
        let n = BigInt::from_u64(xorshift(&mut st) >> (xorshift(&mut st) % 64))
            .shl_bits(xorshift(&mut st) % 90);
        let d = BigInt::from_u64((xorshift(&mut st) >> (xorshift(&mut st) % 64)) | 1)
            .shl_bits(xorshift(&mut st) % 90);
        let r = Rational::new(n, d).unwrap();
        assert_nearest(&r);
        assert_nearest(&r.negated());
    }
    for (n, d) in [(1i64, 3i64), (2, 3), (1, 10), (7, 2), (-22, 7), (355, 113), (1, 1 << 40)] {
        assert_nearest(&Rational::from_ratio_i64(n, d).unwrap());
    }
    assert_eq!(rational_to_f64_nearest(&Rational::zero()), 0.0);
    assert_eq!(rational_to_f64_nearest(&Rational::from_ratio_i64(1, 10).unwrap()), 0.1);
    assert_eq!(rational_to_f64_nearest(&Rational::from_ratio_i64(-1, 3).unwrap()), -1.0 / 3.0);
}

#[test]
fn rational_to_f64_nearest_round_trips_every_float() {
    let mut st = 0xfeed_face_dead_beefu64;
    for _ in 0..4_000 {
        let f = f64::from_bits(xorshift(&mut st) & !(0x7ff << 52) | ((1 + xorshift(&mut st) % 2000) << 52));
        if !f.is_finite() {
            continue;
        }
        assert_eq!(rational_to_f64_nearest(&rational_from_f64_exact(f).unwrap()).to_bits(), f.to_bits());
    }
}

#[test]
fn rational_to_f64_nearest_breaks_exact_ties_toward_even() {
    let one = BigInt::from_i64(1);
    let halfway_above_one = Rational::new(one.shl_bits(53).add(&one), one.shl_bits(53)).unwrap();
    assert_eq!(rational_to_f64_nearest(&halfway_above_one), 1.0);
    let three_halves_ulp = Rational::new(one.shl_bits(53).add(&BigInt::from_i64(3)), one.shl_bits(53)).unwrap();
    assert_eq!(rational_to_f64_nearest(&three_halves_ulp), 1.0 + 2.0 * f64::EPSILON);
}

#[test]
fn rational_to_f64_nearest_handles_subnormals_underflow_and_overflow() {
    let one = BigInt::from_i64(1);
    let tiny = |k: u64| Rational::new(one.clone(), one.shl_bits(k)).unwrap();
    assert_eq!(rational_to_f64_nearest(&tiny(1074)).to_bits(), 1, "the least subnormal");
    assert_eq!(rational_to_f64_nearest(&tiny(1022)), f64::MIN_POSITIVE);
    assert_eq!(rational_to_f64_nearest(&tiny(1075)), 0.0, "half the least subnormal ties to even zero");
    let just_over_half = Rational::new(one.shl_bits(40).add(&one), one.shl_bits(1115)).unwrap();
    assert_eq!(rational_to_f64_nearest(&just_over_half).to_bits(), 1);
    assert_eq!(rational_to_f64_nearest(&tiny(1075).negated()).to_bits(), (-0.0f64).to_bits());
    let mut st = 0x5eed_0f5b_0a11_c0deu64;
    for _ in 0..2_000 {
        let n = BigInt::from_u64((xorshift(&mut st) >> (xorshift(&mut st) % 60)) | 1);
        let d = one.shl_bits(1000 + xorshift(&mut st) % 140);
        assert_nearest(&Rational::new(n.clone(), d.clone()).unwrap());
        assert_nearest(&Rational::new(n.negated(), d).unwrap());
    }
    let huge = Rational::from_bigint(one.shl_bits(1024));
    assert_eq!(rational_to_f64_nearest(&huge), f64::INFINITY);
    assert_eq!(rational_to_f64_nearest(&huge.negated()), f64::NEG_INFINITY);
    assert_eq!(rational_to_f64_nearest(&rational_from_f64_exact(f64::MAX).unwrap()), f64::MAX);
}

#[test]
fn min_and_max_of_reals_order_the_zeros_and_let_a_nan_give_way() {
    let bits = |f: f64| f.to_bits();
    assert_eq!(bits(float_min(0.0, -0.0)), bits(-0.0));
    assert_eq!(bits(float_min(-0.0, 0.0)), bits(-0.0));
    assert_eq!(bits(float_max(0.0, -0.0)), bits(0.0));
    assert_eq!(bits(float_max(-0.0, 0.0)), bits(0.0));
    assert_eq!(bits(float_min(-0.0, -0.0)), bits(-0.0));
    assert_eq!(bits(float_max(0.0, 0.0)), bits(0.0));
    assert_eq!(float_min(f64::NAN, 2.0), 2.0);
    assert_eq!(float_min(2.0, f64::NAN), 2.0);
    assert_eq!(float_max(f64::NAN, -2.0), -2.0);
    assert_eq!(float_max(-2.0, f64::NAN), -2.0);
    assert!(float_min(f64::NAN, f64::NAN).is_nan());
    assert!(float_max(f64::NAN, f64::NAN).is_nan());
    assert_eq!(float_min(1.5, -3.0), -3.0);
    assert_eq!(float_max(1.5, -3.0), 1.5);
    assert_eq!(float_min(f64::NEG_INFINITY, f64::INFINITY), f64::NEG_INFINITY);
    assert_eq!(float_max(f64::NEG_INFINITY, f64::INFINITY), f64::INFINITY);
}
