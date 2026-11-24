use super::*;

/// Dividends where truncating division turns: the extremes, zero, the signs' edges, and both sides of a multiple of
/// the divisor and of 2³².
fn dividends(d: i64) -> Vec<i64> {
    let mut xs = vec![i64::MIN, i64::MIN + 1, i64::MIN + 2, -(1 << 32) - 1, -(1 << 32), -2, -1, 0, 1, 2, 1 << 32, (1 << 32) + 1];
    xs.extend([i64::MAX - 2, i64::MAX - 1, i64::MAX]);
    for k in [-3i64, -2, -1, 1, 2, 3, 1000] {
        if let Some(m) = d.checked_mul(k) {
            xs.extend([m.saturating_sub(1), m, m.saturating_add(1)]);
        }
    }
    if let Some(top) = (i64::MAX / d).checked_mul(d) {
        xs.extend([top - 1, top, -top, -top + 1]);
    }
    xs
}

/// Divisors: every small one, every power of two, and the large and awkward ones compilers special-case.
fn divisors() -> Vec<i64> {
    let mut ds: Vec<i64> = (2..=1100).collect();
    ds.extend((1..63).map(|k| 1i64 << k));
    ds.extend([
        1_000_000_007,
        998_244_353,
        (1 << 31) - 1,
        (1 << 61) - 1,
        3 << 60,
        1_000_000_000_000_000_000,
        i64::MAX / 3,
        i64::MAX - 1,
        i64::MAX,
    ]);
    ds
}

fn check(d: i64, x: i64) {
    let r = SignedReciprocal::new(d).expect("every divisor of at least two has a reciprocal");
    assert_eq!(r.quotient(x), x / d, "quotient of {x} by {d}");
    assert_eq!(r.remainder(x), x % d, "remainder of {x} by {d}");
}

#[test]
fn the_quotient_and_remainder_are_truncating_division_on_every_corner() {
    for d in divisors() {
        for x in dividends(d) {
            check(d, x);
        }
    }
}

#[test]
fn the_quotient_and_remainder_are_truncating_division_on_random_pairs() {
    // SplitMix64, fixed seed: every run checks the same pairs.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for _ in 0..200_000 {
        // A divisor of a random bit length, so small and large divisors are equally likely.
        let bits = (next() % 62) as u32 + 2;
        let d = ((next() >> (64 - bits)) as i64).max(2);
        let x = next() as i64;
        check(d, x);
    }
}

#[test]
fn a_divisor_below_two_has_no_reciprocal() {
    for d in [i64::MIN, -7, -1, 0, 1] {
        assert!(SignedReciprocal::new(d).is_none(), "{d} has no reciprocal");
    }
}

#[test]
fn the_parts_describe_the_reciprocal() {
    for d in divisors() {
        let r = SignedReciprocal::new(d).unwrap();
        assert_eq!(r.divisor(), d);
        assert!(r.shift() <= 63, "shift {} for {d}", r.shift());
        assert_eq!(r.add(), r.magic() < 0, "the add step is taken exactly when the magic is negative, for {d}");
    }
}

/// The reciprocals the JIT computes at run time, for a divisor it observed, are each decided exact before it uses one.
/// (The reciprocals the compiler emits for a literal divisor are the unsigned `MagicDivU` ones, proven for every
/// dividend by `bytecode_tv_lock`'s `every_reciprocal_the_compiler_emits_is_exact_for_every_dividend`.)
#[test]
fn every_divisor_up_to_a_hundred_thousand_and_the_rims_has_an_exact_reciprocal() {
    let rims = [i64::MAX, i64::MAX - 1, (1 << 62) + 1, 1 << 62, (1 << 32) + 1, 1 << 32, 641, 6_700_417, 1_000_000_007];
    for d in (2..=100_000).chain(rims) {
        let r = SignedReciprocal::new(d).unwrap_or_else(|| panic!("{d} has a reciprocal"));
        assert_eq!(r.counterexample(), None, "the reciprocal of {d} is exact for every dividend");
    }
}

/// A reciprocal one off in its magic or its shift is decided inexact, and the dividend the decision names is one its
/// quotient really gets wrong: the decision is the code's behaviour, not a separate claim about it. A magic one lower
/// is below `2^L / d` and a shift one higher halves the quotient, so both are refused for every divisor; a corruption the
/// decision accepts divides every corner dividend exactly.
#[test]
fn a_corrupted_reciprocal_is_refused_with_a_dividend_it_gets_wrong() {
    let with_magic = |r: SignedReciprocal, magic: i64| SignedReciprocal { magic, add: magic < 0, ..r };
    for d in (2..=3000).chain([1_000_000_007, (1 << 62) + 1, i64::MAX]) {
        let good = SignedReciprocal::new(d).unwrap();
        let lower = with_magic(good, good.magic.wrapping_sub(1));
        let wider = SignedReciprocal { shift: good.shift + 1, ..good };
        for bad in [lower, wider] {
            let x = bad.counterexample().unwrap_or_else(|| panic!("{bad:?} is refused"));
            assert_ne!(bad.quotient(x), truncating(x, d), "{bad:?} gets the named dividend {x} wrong");
        }
        let higher = with_magic(good, good.magic.wrapping_add(1));
        let narrower = SignedReciprocal { shift: good.shift.wrapping_sub(1), ..good };
        for bad in [Some(higher), (good.shift > 0).then_some(narrower)].into_iter().flatten() {
            match bad.counterexample() {
                Some(x) => assert_ne!(bad.quotient(x), truncating(x, d), "{bad:?} gets the named dividend {x} wrong"),
                None => {
                    for x in dividends(d) {
                        assert_eq!(bad.quotient(x), truncating(x, d), "{bad:?}, decided exact, divides {x} exactly");
                    }
                }
            }
        }
    }
}

/// The bounds are decided at their edges. Three's reciprocal has no slack: `3·m − 2^64 = 2`, and `2^63` (three divides
/// `2^63 + 1`) puts the negative bound at equality, which is exact. A reciprocal of `2^62 + 1` that divides by `2^62`
/// instead is wrong at one dividend of `2^64`, `2^62`, where the positive bound reaches equality, and is refused there.
#[test]
fn the_decision_is_exact_at_the_edges_of_both_bounds() {
    let three = SignedReciprocal::new(3).unwrap();
    assert_eq!((three.magic, three.shift, three.add), (0x5555_5555_5555_5556, 0, false));
    let e = 3 * i128::from(three.magic) - (1i128 << 64);
    assert_eq!(e << 63, 1i128 << 64, "the negative bound of three's reciprocal is met with equality at −2^63");
    assert_eq!(three.counterexample(), None);
    assert_eq!(three.quotient(i64::MIN), i64::MIN / 3);

    let d = (1i64 << 62) + 1;
    let quarter = SignedReciprocal { magic: 4, shift: 0, add: false, divisor: d };
    assert_eq!(quarter.counterexample(), Some(1 << 62), "the one dividend it gets wrong");
    assert_eq!(quarter.quotient(1 << 62), 1);
    assert_eq!(truncating(1 << 62, d), 0);
    for x in dividends(d).into_iter().chain([(1 << 62) - 1, (1 << 62) + 1, -(1 << 62), (1 << 62) + 2]) {
        if x != 1 << 62 {
            assert_eq!(quarter.quotient(x), truncating(x, d), "{x} is not the dividend it gets wrong");
        }
    }
}

/// Reciprocals near the right one, for random divisors of every size: each one refused is refused with a dividend it
/// gets wrong, and each one accepted divides every corner dividend and a thousand random ones exactly.
#[test]
fn every_decision_on_a_nearby_reciprocal_is_witnessed() {
    let mut state: u64 = 0xD1B5_4A32_D192_ED03;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let (mut accepted, mut refused) = (0, 0);
    for _ in 0..4000 {
        let d = ((next() >> (next() % 63)) as i64).max(2);
        let good = SignedReciprocal::new(d).unwrap();
        let delta = (next() % 6) as i64 - 2;
        let magic = good.magic.wrapping_add(delta);
        let near = SignedReciprocal { magic, add: magic < 0, ..good };
        match near.counterexample() {
            Some(x) => {
                refused += 1;
                assert_ne!(near.quotient(x), truncating(x, d), "{near:?} gets the named dividend {x} wrong");
            }
            None => {
                accepted += 1;
                let random: Vec<i64> = (0..1000).map(|_| next() as i64).collect();
                for x in dividends(d).into_iter().chain(random) {
                    assert_eq!(near.quotient(x), truncating(x, d), "{near:?}, decided exact, divides {x} exactly");
                }
            }
        }
    }
    assert!(accepted > 400 && refused > 400, "both decisions are exercised: {accepted} accepted, {refused} refused");
}

fn truncating(x: i64, d: i64) -> i64 {
    (i128::from(x) / i128::from(d)) as i64
}
