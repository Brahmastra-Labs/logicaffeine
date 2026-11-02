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
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/period/tests.rs"]
mod tests;
