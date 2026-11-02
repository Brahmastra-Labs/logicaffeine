use super::*;

fn big(s: &str) -> BigInt {
    BigInt::parse_decimal(s).unwrap()
}

// Brute-force order: least r>0 with aʳ ≡ 1 (mod n), or None.
fn brute_order(a: u64, n: u64) -> Option<u64> {
    if gcd(&i(a as i64), &i(n as i64)) != i(1) {
        return None;
    }
    let mut cur = 1u64 % n;
    for r in 1..=n {
        cur = (cur * a) % n;
        if cur == 1 {
            return Some(r);
        }
    }
    None
}

#[test]
fn order_matches_brute_force_exhaustively() {
    // Tested to absurdity: every base against every modulus in a range, BSGS vs the definition.
    for n in 2..=200u64 {
        for a in 2..n {
            let bsgs = multiplicative_order(&i(a as i64), &i(n as i64), 400);
            assert_eq!(bsgs, brute_order(a, n), "ord_{n}({a}) mismatch");
        }
    }
}

#[test]
fn factor_from_order_splits_a_semiprime() {
    // N = 15, a = 2: 2,4,8,16≡1 so r=4 (even); 2² = 4, gcd(4−1,15)=3, gcd(4+1,15)=5.
    let n = i(15);
    let r = multiplicative_order(&i(2), &n, 100).unwrap();
    assert_eq!(r, 4, "ord_15(2) = 4");
    let f = factor_from_order(&i(2), r, &n).expect("even order with a nontrivial root splits N");
    assert!(f == i(3) || f == i(5), "recovers a real factor, got {f:?}");
}

#[test]
fn factor_via_order_is_the_classical_shor_shell() {
    // End-to-end: recover a factor of a semiprime purely through period-finding.
    for (p, q) in [(11u64, 13u64), (17, 19), (101, 103), (211, 223)] {
        let n = i((p * q) as i64);
        let f = factor_via_order(&n, 50, (p * q) as u64, 42).expect("order route finds a factor");
        assert!(f != i(1) && f != n, "nontrivial");
        assert!(
            n.div_rem(&f).unwrap().1.is_zero(),
            "and it divides N = {}·{}",
            p,
            q
        );
    }
}

#[test]
fn order_declines_when_base_shares_a_factor() {
    // gcd(a,n) ≠ 1 ⟹ a is not a unit ⟹ no multiplicative order.
    assert_eq!(
        multiplicative_order(&i(6), &i(15), 100),
        None,
        "6 shares 3 with 15"
    );
    assert_eq!(
        multiplicative_order(&i(10), &i(15), 100),
        None,
        "10 shares 5 with 15"
    );
}
