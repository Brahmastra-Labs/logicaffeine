//! # The period / order seam — the classical shell of Shor's algorithm
//!
//! The multiplicative structure of `ℤ/N` hides a Fourier object: the map `x ↦ aˣ mod N` is **periodic**,
//! with period `r = ord_N(a)` (the multiplicative order). That period is the additive shadow of the
//! multiplicative group, and it *reveals the factorization*: if `r` is even and `a^{r/2} ≢ ±1 (mod N)`,
//! then `a^{r/2}` is a nontrivial square root of `1`, so `gcd(a^{r/2} − 1, N)` and `gcd(a^{r/2} + 1, N)`
//! split `N`. Factoring reduces to order-finding.
//!
//! The catch — and the whole point — is that **classically, finding the order is not easier than
//! factoring.** Baby-step-giant-step does it in `O(√r)`, exponential in the bit length. Shor's quantum
//! algorithm replaces this scan with a *quantum Fourier transform* that reads the period off in polynomial
//! time. So this module is the honest classical primitive: the reduction is exact and the order-finder is
//! correct, but the speed lives on the quantum side of the seam. It is also a first-class number-theory
//! primitive in its own right (order-finding underlies discrete-log and group-structure computations).

use crate::factor::{gcd, modpow};
use logicaffeine_base::BigInt;
use std::collections::HashMap;

#[inline]
fn i(x: i64) -> BigInt {
    BigInt::from_i64(x)
}

#[inline]
fn rem_pos(a: &BigInt, n: &BigInt) -> BigInt {
    let r = a.div_rem(n).map(|(_, r)| r).unwrap_or_else(|| a.clone());
    if r.is_negative() {
        r.add(n)
    } else {
        r
    }
}

#[inline]
fn mulmod(a: &BigInt, b: &BigInt, n: &BigInt) -> BigInt {
    rem_pos(&a.mul(b), n)
}

/// A stable map key for a residue in `[0, n)`.
#[inline]
fn key(x: &BigInt) -> Vec<u8> {
    x.to_le_bytes().1
}

/// The distinct prime factors of a `u64` (trial division — used only on small orders).
fn distinct_primes(mut r: u64) -> Vec<u64> {
    let mut ps = Vec::new();
    let mut d = 2u64;
    while d * d <= r {
        if r % d == 0 {
            ps.push(d);
            while r % d == 0 {
                r /= d;
            }
        }
        d += 1;
    }
    if r > 1 {
        ps.push(r);
    }
    ps
}

/// Given a multiple `r` of `ord_N(a)`, reduce it to the exact order by stripping prime factors while the
/// smaller exponent still annihilates `a`.
fn reduce_to_order(a: &BigInt, mut r: u64, n: &BigInt) -> u64 {
    let one = i(1);
    for p in distinct_primes(r) {
        while r % p == 0 && modpow(a, &i((r / p) as i64), n) == one {
            r /= p;
        }
    }
    r
}

/// The **multiplicative order** of `a` modulo `n` — the least `r > 0` with `aʳ ≡ 1 (mod n)` — by
/// baby-step-giant-step, searching orders up to `bound`. `None` if `gcd(a, n) ≠ 1` (no order) or the order
/// exceeds `bound`. Runs in `O(√bound)` field multiplications.
pub fn multiplicative_order(a: &BigInt, n: &BigInt, bound: u64) -> Option<u64> {
    let one = i(1);
    if gcd(a, n) != one {
        return None;
    }
    let a = rem_pos(a, n);
    if a == one {
        return Some(1);
    }
    // m = ⌈√bound⌉.
    let m = (bound as f64).sqrt() as u64 + 1;

    // Baby steps a⁰, a¹, …, a^{m−1}. A hit at aʲ = 1 (j > 0) IS the order (a small one).
    let mut baby: HashMap<Vec<u8>, u64> = HashMap::new();
    let mut cur = one.clone();
    for j in 0..m {
        if j > 0 && cur == one {
            return Some(j);
        }
        baby.entry(key(&cur)).or_insert(j);
        cur = mulmod(&cur, &a, n);
    }
    let giant = cur; // a^m
    // Giant steps: find k with a^{km} = aⁱ (i < m), so a^{km − i} = 1 is a multiple of the order.
    let mut gk = giant.clone();
    for k in 1..=m {
        if let Some(&idx) = baby.get(&key(&gk)) {
            let cand = k * m - idx;
            if cand > 0 {
                return Some(reduce_to_order(&a, cand, n));
            }
        }
        gk = mulmod(&gk, &giant, n);
    }
    None
}

/// Split `n` from a known order `r` of `a`: if `r` is even and `a^{r/2}` is a nontrivial square root of `1`
/// (`≢ ±1`), then `gcd(a^{r/2} ± 1, n)` is a proper factor. `None` when the order is odd or the root is
/// trivial (`±1`) — the ~50% of the time a fresh `a` is needed.
pub fn factor_from_order(a: &BigInt, r: u64, n: &BigInt) -> Option<BigInt> {
    if r % 2 != 0 {
        return None;
    }
    let one = i(1);
    let root = modpow(a, &i((r / 2) as i64), n); // a^{r/2} mod n
    if root == one || root == n.sub(&one) {
        return None; // a^{r/2} ≡ ±1 — no information
    }
    for cand in [root.sub(&one), root.add(&one)] {
        let g = gcd(&cand, n);
        if g != one && g != *n {
            return Some(g);
        }
    }
    None
}

/// **Factoring via order-finding** — the classical shell of Shor's algorithm. Draw bases `a`, find each
/// order (bounded by `bound`), and split `n` when the order is even with a nontrivial root. `None` if no
/// trial succeeded within `tries`/`bound`. Classically `O(√order)` per base (exponential in bit length);
/// this is exactly the step Shor's quantum Fourier transform makes polynomial.
pub fn factor_via_order(n: &BigInt, tries: usize, bound: u64, seed: u64) -> Option<BigInt> {
    let one = i(1);
    if !n.is_odd() {
        return Some(i(2));
    }
    for t in 0..tries {
        let a_u = 2 + seed.wrapping_add(t as u64).wrapping_mul(2_654_435_761) % 1_000_000;
        let a = rem_pos(&i(a_u as i64), n);
        if a == one || a.is_zero() {
            continue;
        }
        // A base sharing a factor with n hands it over directly (the lucky case).
        let g = gcd(&a, n);
        if g != one && g != *n {
            return Some(g);
        }
        if let Some(r) = multiplicative_order(&a, n, bound) {
            if let Some(f) = factor_from_order(&a, r, n) {
                return Some(f);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
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
            assert!(n.div_rem(&f).unwrap().1.is_zero(), "and it divides N = {}·{}", p, q);
        }
    }

    #[test]
    fn order_declines_when_base_shares_a_factor() {
        // gcd(a,n) ≠ 1 ⟹ a is not a unit ⟹ no multiplicative order.
        assert_eq!(multiplicative_order(&i(6), &i(15), 100), None, "6 shares 3 with 15");
        assert_eq!(multiplicative_order(&i(10), &i(15), 100), None, "10 shares 5 with 15");
    }
}
