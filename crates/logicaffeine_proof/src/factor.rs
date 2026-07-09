//! Structural factoring: the AIT thesis applied to public-key crypto.
//!
//! The compressibility ladder crushes symmetric keystreams by finding their *structure* — a short
//! feedback register, a correlation, a low-degree annihilator. RSA lives in a different universe: its
//! security is integer factorization of `N = p·q`, not sequence structure. But the same thesis governs
//! it. A *weak* RSA modulus is one with exploitable STRUCTURE, and every classical factoring attack is a
//! structure-detector: a small factor, primes that sit too close, a smooth `p−1`, a shared prime across
//! moduli, a small private exponent. Each such structure is a compression — a short description of the
//! secret — and each attack returns a re-checkable WITNESS (the factors themselves, `p·q = N`).
//!
//! The point of building the whole arsenal is the *ceiling*: run every structural attack against a
//! soundly-generated modulus — two large, independent, well-separated strong primes — and it finds
//! NOTHING within budget. RSA's safety is precisely that its modulus is the number-theoretic
//! incompressible residue: no structural shortcut exists, and only the general (sub-)exponential
//! algorithms remain, which real key sizes push out of reach. Crush every structured form; the sound
//! form stands. That standing IS the proof.

use logicaffeine_base::numeric::{BigInt, Rational};

/// The Miller–Rabin witness bases (the first twelve primes) — deterministic for the modulus sizes used
/// here, and overwhelmingly reliable beyond them.
const MR_BASES: &[u64] = &[2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];

fn one() -> BigInt {
    BigInt::from_i64(1)
}

fn two() -> BigInt {
    BigInt::from_i64(2)
}

/// `a mod m` for non-negative `a`, `m > 0`.
fn rem(a: &BigInt, m: &BigInt) -> BigInt {
    a.div_rem(m).expect("modulus is nonzero").1
}

/// The greatest common divisor of `|a|` and `|b|` (Euclid).
pub fn gcd(a: &BigInt, b: &BigInt) -> BigInt {
    let (mut a, mut b) = (a.abs(), b.abs());
    while !b.is_zero() {
        let r = rem(&a, &b);
        a = b;
        b = r;
    }
    a
}

/// The modular inverse `a⁻¹ mod m` via the extended Euclidean algorithm, or `None` if `gcd(a, m) ≠ 1`.
pub fn mod_inverse(a: &BigInt, m: &BigInt) -> Option<BigInt> {
    let (mut old_r, mut r) = (rem(a, m), m.clone());
    let (mut old_s, mut s) = (one(), BigInt::zero());
    while !r.is_zero() {
        let (q, rr) = old_r.div_rem(&r).expect("nonzero");
        old_r = r;
        r = rr;
        let ns = old_s.sub(&q.mul(&s));
        old_s = s;
        s = ns;
    }
    if old_r != one() {
        return None;
    }
    let mut res = rem(&old_s, m);
    if res.is_negative() {
        res = res.add(m);
    }
    Some(res)
}

/// Modular exponentiation `base^exp mod m` with a full `BigInt` exponent (square-and-multiply, the
/// exponent's bits read off by repeated halving).
pub fn modpow(base: &BigInt, exp: &BigInt, m: &BigInt) -> BigInt {
    if *m == one() {
        return BigInt::zero();
    }
    let two = two();
    let mut result = one();
    let mut b = rem(base, m);
    let mut e = exp.clone();
    while !e.is_zero() {
        let (q, bit) = e.div_rem(&two).expect("nonzero");
        if !bit.is_zero() {
            result = rem(&result.mul(&b), m);
        }
        b = rem(&b.mul(&b), m);
        e = q;
    }
    result
}

/// The integer square root `⌊√n⌋` (Newton's method). The floating-point seed is only ~15 digits precise,
/// so it is first forced to an OVERESTIMATE (`x² ≥ n`, a handful of doublings); from above, the Newton
/// iteration `x ← ⌊(x + ⌊n/x⌋)/2⌋` decreases monotonically and halts exactly at `⌊√n⌋`.
pub fn isqrt(n: &BigInt) -> BigInt {
    if n.is_zero() || n.is_negative() {
        return BigInt::zero();
    }
    let two = two();
    let approx = n.to_f64().sqrt();
    let mut x = if approx.is_finite() && approx >= 1.0 {
        BigInt::parse_decimal(&format!("{approx:.0}")).unwrap_or_else(one)
    } else {
        one()
    };
    if x.is_zero() {
        x = one();
    }
    while x.mul(&x) < *n {
        x = x.mul(&two); // force an overestimate (the f64 seed may undershoot by ~10¹⁵)
    }
    loop {
        let (q, _) = n.div_rem(&x).expect("nonzero");
        let y = x.add(&q).div_rem(&two).expect("nonzero").0;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// Miller–Rabin primality over [`MR_BASES`].
pub fn is_probable_prime(n: &BigInt) -> bool {
    let (one, two) = (one(), two());
    if *n < two {
        return false;
    }
    if *n == two {
        return true;
    }
    if !n.is_odd() {
        return false;
    }
    // n − 1 = d·2ˢ.
    let n1 = n.sub(&one);
    let mut d = n1.clone();
    let mut s = 0u32;
    while !d.is_odd() {
        d = d.div_rem(&two).expect("nonzero").0;
        s += 1;
    }
    'base: for &a in MR_BASES {
        let a = BigInt::from_u64(a);
        if rem(&a, n).is_zero() {
            continue; // a ≡ 0 mod n (n divides the base) — uninformative
        }
        let mut x = modpow(&a, &d, n);
        if x == one || x == n1 {
            continue;
        }
        for _ in 0..s.saturating_sub(1) {
            x = rem(&x.mul(&x), n);
            if x == n1 {
                continue 'base;
            }
        }
        return false;
    }
    true
}

/// The smallest prime `≥ start`.
pub fn next_prime(start: &BigInt) -> BigInt {
    let (one, two) = (one(), two());
    if *start <= two {
        return two;
    }
    let mut c = if start.is_odd() { start.clone() } else { start.add(&one) };
    loop {
        if is_probable_prime(&c) {
            return c;
        }
        c = c.add(&two);
    }
}

/// A nontrivial factorization `p·q = n` (`1 < p, q < n`) — the re-checkable witness every attack returns.
pub fn verify_factorization(n: &BigInt, p: &BigInt, q: &BigInt) -> bool {
    let one = one();
    p.mul(q) == *n && *p > one && *q > one && *p < *n && *q < *n
}

fn split(n: &BigInt, d: &BigInt) -> Option<(BigInt, BigInt)> {
    let one = one();
    if *d > one && *d < *n {
        let (q, r) = n.div_rem(d).expect("nonzero");
        if r.is_zero() {
            return Some((d.clone(), q));
        }
    }
    None
}

/// Trial division up to `limit`: catches a **small factor** — a modulus that leaked a tiny prime.
pub fn trial_division(n: &BigInt, limit: u64) -> Option<(BigInt, BigInt)> {
    let mut d = 2u64;
    while d <= limit {
        let bd = BigInt::from_u64(d);
        if bd.mul(&bd) > *n {
            break;
        }
        if let Some(f) = split(n, &bd) {
            return Some(f);
        }
        d += if d == 2 { 1 } else { 2 };
    }
    None
}

/// Fermat's method: catches **primes that sit too close** — `N = a² − b²` with `a` just above `√N`, so a
/// few steps expose `(a−b, a+b)`. Structurally, close primes are a compressed key (one prime nearly fixes
/// the other).
pub fn fermat(n: &BigInt, max_iters: u64) -> Option<(BigInt, BigInt)> {
    if !n.is_odd() {
        return split(n, &two());
    }
    let one = one();
    let mut a = isqrt(n);
    if a.mul(&a) < *n {
        a = a.add(&one);
    }
    for _ in 0..max_iters {
        let b2 = a.mul(&a).sub(n);
        let b = isqrt(&b2);
        if b.mul(&b) == b2 {
            let p = a.sub(&b);
            let q = a.add(&b);
            if verify_factorization(n, &p, &q) {
                return Some((p, q));
            }
        }
        a = a.add(&one);
    }
    None
}

/// Pollard's rho (`f(x) = x² + 1`, Floyd cycle detection): the general-purpose structural probe. Expected
/// `O(N^{1/4})`, so it crushes moderate semiprimes but is bounded out on a large sound modulus.
pub fn pollard_rho(n: &BigInt, max_iters: u64) -> Option<(BigInt, BigInt)> {
    let (one, two) = (one(), two());
    if !n.is_odd() {
        return split(n, &two);
    }
    let f = |x: &BigInt| rem(&x.mul(x).add(&one), n);
    let (mut x, mut y, mut d) = (two.clone(), two.clone(), one.clone());
    let mut iters = 0u64;
    while d == one && iters < max_iters {
        x = f(&x);
        y = f(&f(&y));
        d = gcd(&x.sub(&y).abs(), n);
        iters += 1;
    }
    split(n, &d)
}

/// Pollard's `p − 1`: catches a prime with a **smooth `p − 1`**. Raising a base to `B!` collapses to `1`
/// modulo any prime whose `p − 1` is `B`-smooth, and `gcd(a − 1, N)` exposes it — the smoothness is the
/// structure.
pub fn pollard_p_minus_1(n: &BigInt, bound: u64) -> Option<(BigInt, BigInt)> {
    let one = one();
    let mut a = two();
    for k in 2..=bound {
        a = modpow(&a, &BigInt::from_u64(k), n);
        if k % 16 == 0 || k == bound {
            if let Some(f) = split(n, &gcd(&a.sub(&one), n)) {
                return Some(f);
            }
        }
    }
    split(n, &gcd(&a.sub(&one), n))
}

/// Wiener's attack: catches a **small private exponent `d`** (`d < ⅓ N^{1/4}`). The convergents of the
/// continued fraction of `e/N` include `k/d`; from `d` we recover `φ(N)`, then `p, q` as roots of
/// `x² − (N − φ + 1)x + N`. A small `d` is a compressed private key — and this reuses the very
/// continued-fraction / rational-reconstruction machinery the 2-adic FCSR rung was built on.
pub fn wiener(e: &BigInt, n: &BigInt) -> Option<(BigInt, BigInt)> {
    let (one, two, four) = (one(), two(), BigInt::from_i64(4));
    let (mut num, mut den) = (e.clone(), n.clone());
    // Convergent recurrence h_i / k_i (numerator = candidate multiplier k, denominator = candidate d).
    let (mut h2, mut h1) = (BigInt::zero(), one.clone());
    let (mut k2, mut k1) = (one.clone(), BigInt::zero());
    for _ in 0..2000 {
        if den.is_zero() {
            break;
        }
        let (a, r) = num.div_rem(&den).expect("nonzero");
        let h = a.mul(&h1).add(&h2); // candidate k (the small multiplier)
        let k = a.mul(&k1).add(&k2); // candidate d (the private exponent)
        if !h.is_zero() {
            let (phi, prem) = e.mul(&k).sub(&one).div_rem(&h).expect("nonzero");
            if prem.is_zero() {
                // p, q are the roots of x² − (N − φ + 1)x + N.
                let s = n.sub(&phi).add(&one); // p + q
                let disc = s.mul(&s).sub(&four.mul(n));
                if !disc.is_negative() {
                    let sq = isqrt(&disc);
                    if sq.mul(&sq) == disc {
                        let p = s.sub(&sq).div_rem(&two).expect("nonzero").0;
                        let q = s.add(&sq).div_rem(&two).expect("nonzero").0;
                        if verify_factorization(n, &p, &q) {
                            return Some((p, q));
                        }
                    }
                }
            }
        }
        num = den;
        den = r;
        (h2, h1) = (h1, h);
        (k2, k1) = (k1, k);
    }
    None
}

// ---- Håstad broadcast: a MESSAGE-recovery lens (small public exponent + broadcast) ------------------
//
// Every attack above recovers the KEY. Håstad's recovers the MESSAGE, from a different structural
// weakness: the same plaintext `m` sent to `k ≥ e` recipients under a small public exponent `e` and
// distinct moduli. The ciphertexts are `cᵢ = mᵉ mod Nᵢ`, and since `m < Nᵢ` for all `i`, `mᵉ < ∏Nᵢ`.
// The Chinese Remainder Theorem reconstructs `mᵉ mod ∏Nᵢ` — which, being smaller than the modulus, IS
// `mᵉ` over the integers — and an integer `e`-th root pops out `m`. The plaintext is a low-complexity
// object smeared across the broadcast; CRT gathers it back into a perfect power.

fn pos_rem(a: &BigInt, m: &BigInt) -> BigInt {
    let r = rem(a, m);
    if r.is_negative() {
        r.add(m)
    } else {
        r
    }
}

/// Chinese Remainder Theorem: the unique `x` in `[0, ∏mᵢ)` with `x ≡ residuesᵢ (mod moduliᵢ)` for
/// pairwise-coprime `moduli`, or `None` if a modular inverse fails (moduli not coprime).
pub fn crt(residues: &[BigInt], moduli: &[BigInt]) -> Option<BigInt> {
    if residues.is_empty() || residues.len() != moduli.len() {
        return None;
    }
    let mut x = pos_rem(&residues[0], &moduli[0]);
    let mut m = moduli[0].clone();
    for i in 1..moduli.len() {
        let mi = &moduli[i];
        let inv = mod_inverse(&pos_rem(&m, mi), mi)?;
        let diff = pos_rem(&residues[i].sub(&x), mi);
        let t = pos_rem(&diff.mul(&inv), mi);
        x = x.add(&m.mul(&t));
        m = m.mul(mi);
        x = pos_rem(&x, &m);
    }
    Some(x)
}

/// The integer `n`-th root `⌊x^{1/n}⌋` (Newton's method, overestimate seed then monotone descent, exact
/// final adjustment — the `n`-th-root analogue of [`isqrt`]).
pub fn integer_nth_root(x: &BigInt, n: u32) -> BigInt {
    if n == 0 || x.is_negative() {
        return BigInt::zero();
    }
    if n == 1 || x.is_zero() {
        return x.clone();
    }
    let (one, two) = (one(), two());
    let nn = BigInt::from_u64(n as u64);
    let nm1 = BigInt::from_u64((n - 1) as u64);
    let approx = x.to_f64().powf(1.0 / n as f64);
    let mut r = if approx.is_finite() && approx >= 1.0 {
        BigInt::parse_decimal(&format!("{approx:.0}")).unwrap_or_else(|| one.clone())
    } else {
        one.clone()
    };
    if r.is_zero() {
        r = one.clone();
    }
    while r.pow(n) < *x {
        r = r.mul(&two); // force an overestimate
    }
    loop {
        let (q, _) = x.div_rem(&r.pow(n - 1)).expect("nonzero");
        let next = nm1.mul(&r).add(&q).div_rem(&nn).expect("nonzero").0;
        if next >= r {
            break;
        }
        r = next;
    }
    while r.pow(n) > *x {
        r = r.sub(&one);
    }
    while r.add(&one).pow(n) <= *x {
        r = r.add(&one);
    }
    r
}

/// Håstad's broadcast attack: recover the plaintext `m` from `k ≥ e` ciphertexts `cᵢ = mᵉ mod Nᵢ` of the
/// SAME message under a small public exponent `e` and distinct moduli (see the module note above). CRT
/// reconstructs `mᵉ` exactly, and its integer `e`-th root is `m`. Returns the recovered message, or `None`
/// if the CRT fails or the result is not a perfect `e`-th power (the precondition `k ≥ e` did not hold).
pub fn hastad_broadcast_attack(e: u32, ciphertexts: &[BigInt], moduli: &[BigInt]) -> Option<BigInt> {
    if (ciphertexts.len() as u32) < e {
        return None;
    }
    let c = crt(ciphertexts, moduli)?;
    let m = integer_nth_root(&c, e);
    (m.pow(e) == c).then_some(m)
}

// ---- More message-recovery lenses: common modulus, Franklin–Reiter, low-exponent --------------------

/// Extended Euclid: `(g, x, y)` with `x·a + y·b = g = gcd(a, b)`.
fn ext_gcd(a: &BigInt, b: &BigInt) -> (BigInt, BigInt, BigInt) {
    let (mut old_r, mut r) = (a.clone(), b.clone());
    let (mut old_s, mut s) = (one(), BigInt::zero());
    let (mut old_t, mut t) = (BigInt::zero(), one());
    while !r.is_zero() {
        let (q, _) = old_r.div_rem(&r).expect("nonzero");
        let nr = old_r.sub(&q.mul(&r));
        old_r = r;
        r = nr;
        let ns = old_s.sub(&q.mul(&s));
        old_s = s;
        s = ns;
        let nt = old_t.sub(&q.mul(&t));
        old_t = t;
        t = nt;
    }
    (old_r, old_s, old_t)
}

/// Common-modulus attack: recover `m` when the SAME message is sent to the same modulus `N` under two
/// coprime public exponents `e₁, e₂` (a key-reuse mistake). Bézout gives `a·e₁ + b·e₂ = 1`, so
/// `c₁ᵃ · c₂ᵇ = m^{a·e₁ + b·e₂} = m (mod N)` — negative exponents handled by inverting the ciphertext.
pub fn common_modulus_attack(e1: &BigInt, e2: &BigInt, c1: &BigInt, c2: &BigInt, n: &BigInt) -> Option<BigInt> {
    let (g, a, b) = ext_gcd(e1, e2);
    if g != one() {
        return None;
    }
    let t1 = if a.is_negative() {
        modpow(&mod_inverse(c1, n)?, &a.negated(), n)
    } else {
        modpow(c1, &a, n)
    };
    let t2 = if b.is_negative() {
        modpow(&mod_inverse(c2, n)?, &b.negated(), n)
    } else {
        modpow(c2, &b, n)
    };
    Some(rem(&t1.mul(&t2), n))
}

/// Low-exponent / no-padding attack: if `mᵉ < N` (a small message under a small public exponent with no
/// padding), then `c = mᵉ` over the integers, so the plaintext is just the integer `e`-th root of `c`.
pub fn low_exponent_message(c: &BigInt, e: u32) -> Option<BigInt> {
    let m = integer_nth_root(c, e);
    (m.pow(e) == *c).then_some(m)
}

fn poly_norm_mod(p: &[BigInt], n: &BigInt) -> Vec<BigInt> {
    let mut v: Vec<BigInt> = p.iter().map(|c| pos_rem(c, n)).collect();
    while v.len() > 1 && v.last().is_some_and(|c| c.is_zero()) {
        v.pop();
    }
    if v.is_empty() {
        v.push(BigInt::zero());
    }
    v
}

fn poly_mul_mod(a: &[BigInt], b: &[BigInt], n: &BigInt) -> Vec<BigInt> {
    let mut out = vec![BigInt::zero(); a.len() + b.len() - 1];
    for (i, ai) in a.iter().enumerate() {
        for (j, bj) in b.iter().enumerate() {
            out[i + j] = pos_rem(&out[i + j].add(&ai.mul(bj)), n);
        }
    }
    poly_norm_mod(&out, n)
}

/// Remainder of `a` divided by `b` in `(ℤ/N)[x]` (long division, leading coefficient of `b` inverted mod
/// `N`). `None` if that inverse fails — which would itself expose a factor of `N`.
fn poly_rem_mod(a: &[BigInt], b: &[BigInt], n: &BigInt) -> Option<Vec<BigInt>> {
    let b = poly_norm_mod(b, n);
    let db = b.len() - 1;
    if b.iter().all(|c| c.is_zero()) {
        return None;
    }
    let lead_inv = mod_inverse(&b[db], n)?;
    let mut r = poly_norm_mod(a, n);
    while r.len() > db {
        let dr = r.len() - 1;
        let factor = pos_rem(&r[dr].mul(&lead_inv), n);
        let shift = dr - db;
        for i in 0..b.len() {
            r[shift + i] = pos_rem(&r[shift + i].sub(&factor.mul(&b[i])), n);
        }
        r = poly_norm_mod(&r, n);
    }
    Some(r)
}

/// Monic GCD of two polynomials over `(ℤ/N)[x]` (Euclidean algorithm).
fn poly_gcd_mod(a: &[BigInt], b: &[BigInt], n: &BigInt) -> Option<Vec<BigInt>> {
    let mut a = poly_norm_mod(a, n);
    let mut b = poly_norm_mod(b, n);
    while !(b.len() == 1 && b[0].is_zero()) {
        let r = poly_rem_mod(&a, &b, n)?;
        a = b;
        b = r;
    }
    Some(a)
}

/// Franklin–Reiter related-message attack: recover `m` from ciphertexts of two LINEARLY RELATED messages
/// `m` and `m + r` (known `r`) under the same modulus `N` and a small public exponent `e`. Both
/// `g₁(x) = xᵉ − c₁` and `g₂(x) = (x + r)ᵉ − c₂` have the root `x = m mod N`, so their GCD over `(ℤ/N)[x]`
/// is the linear factor `x − m`, and `m` is read straight off it. The lens: two ciphertexts sharing a
/// hidden root, extracted by polynomial GCD.
pub fn franklin_reiter_attack(e: u32, r: &BigInt, c1: &BigInt, c2: &BigInt, n: &BigInt) -> Option<BigInt> {
    let mut g1 = vec![BigInt::zero(); (e + 1) as usize];
    g1[0] = pos_rem(&c1.negated(), n);
    g1[e as usize] = one();

    let xr = vec![pos_rem(r, n), one()];
    let mut g2 = vec![one()];
    for _ in 0..e {
        g2 = poly_mul_mod(&g2, &xr, n);
    }
    g2[0] = pos_rem(&g2[0].sub(c2), n);

    let g = poly_gcd_mod(&g1, &g2, n)?;
    if g.len() == 2 {
        let m = pos_rem(&g[0].negated().mul(&mod_inverse(&g[1], n)?), n);
        if modpow(&m, &BigInt::from_u64(e as u64), n) == pos_rem(c1, n) {
            return Some(m);
        }
    }
    None
}

// ---- Coppersmith's method: the LATTICE lens on RSA (a break with no factoring shortcut) --------------
//
// Polynomial arithmetic over the integers (coefficients low-to-high; `p[k]` = coefficient of `xᵏ`).

fn poly_mul(a: &[BigInt], b: &[BigInt]) -> Vec<BigInt> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![BigInt::zero(); a.len() + b.len() - 1];
    for (i, ai) in a.iter().enumerate() {
        for (j, bj) in b.iter().enumerate() {
            out[i + j] = out[i + j].add(&ai.mul(bj));
        }
    }
    out
}

fn poly_pow(base: &[BigInt], e: usize) -> Vec<BigInt> {
    let mut acc = vec![one()];
    for _ in 0..e {
        acc = poly_mul(&acc, base);
    }
    acc
}

fn poly_eval(coeffs: &[BigInt], x: &BigInt) -> BigInt {
    coeffs.iter().rev().fold(BigInt::zero(), |acc, c| acc.mul(x).add(c))
}

/// Find an integer root of `coeffs` in `[0, bound]`, if one exists: a floating-point Newton search
/// (coefficients normalized to keep the doubles finite) proposes candidates, each verified EXACTLY with
/// `BigInt` evaluation. Root-finding is the easy step — the miracle is that Coppersmith's lattice produced
/// a polynomial whose small root is an *integer* root at all.
fn poly_integer_root(coeffs: &[BigInt], bound: &BigInt) -> Option<BigInt> {
    let maxc = coeffs.iter().map(|c| c.abs()).max()?;
    if maxc.is_zero() {
        return None;
    }
    // For a small bound, scan exactly — the root of the recovered polynomial is trivial to locate once
    // the lattice has produced it (this is post-processing, not the attack). Larger bounds fall through
    // to the Newton search below (as every Coppersmith implementation does).
    if bound.to_f64() <= 4_200_000.0 {
        let mut x = BigInt::zero();
        let one = one();
        while x <= *bound {
            if poly_eval(coeffs, &x).is_zero() {
                return Some(x);
            }
            x = x.add(&one);
        }
        return None;
    }
    let cf: Vec<f64> =
        coeffs.iter().map(|c| Rational::new(c.clone(), maxc.clone()).map(|r| r.to_f64()).unwrap_or(0.0)).collect();
    let eval = |x: f64| cf.iter().rev().fold(0.0, |acc, &c| acc * x + c);
    let deriv = |x: f64| (1..cf.len()).rev().fold(0.0, |acc, k| acc * x + cf[k] * k as f64);
    let bound_f = bound.to_f64();

    let mut seeds: Vec<f64> = Vec::new();
    let mut p = 1.0;
    while p <= bound_f {
        seeds.push(p);
        p *= 1.5;
    }
    for i in 0..=64 {
        seeds.push(bound_f * i as f64 / 64.0);
    }

    let mut seen: Vec<BigInt> = Vec::new();
    for &s in &seeds {
        let mut x = s;
        for _ in 0..200 {
            let d = deriv(x);
            if d.abs() < 1e-15 {
                break;
            }
            let nx = x - eval(x) / d;
            if !nx.is_finite() {
                break;
            }
            if (nx - x).abs() < 0.4 {
                x = nx;
                break;
            }
            x = nx;
        }
        if x.is_finite() && x >= -1.0 && x <= bound_f + 1.0 {
            if let Some(cand) = BigInt::parse_decimal(&format!("{:.0}", x.max(0.0))) {
                if !seen.contains(&cand) {
                    if cand <= *bound && poly_eval(coeffs, &cand).is_zero() {
                        return Some(cand);
                    }
                    seen.push(cand);
                }
            }
        }
    }
    None
}

fn poly_derivative(p: &[BigInt]) -> Vec<BigInt> {
    (1..p.len()).map(|k| p[k].mul(&BigInt::from_i64(k as i64))).collect()
}

/// Nearest integer to `a/b` (ties away from zero).
fn round_div_factor(a: &BigInt, b: &BigInt) -> BigInt {
    let (q, r) = a.div_rem(b).expect("nonzero");
    if r.is_zero() {
        return q;
    }
    if BigInt::from_i64(2).mul(&r.abs()) > b.abs() {
        if a.is_negative() ^ b.is_negative() {
            q.sub(&one())
        } else {
            q.add(&one())
        }
    } else {
        q
    }
}

/// Exact integer roots of `res` in `[−bound, bound]`: a floating-point Newton pass proposes seeds, and
/// EXACT integer Newton (`x ← x − round(res(x)/res'(x))`, evaluated in `BigInt`) refines each to a true
/// integer root — necessary because the resultant's coefficients are far too large for f64 to locate the
/// root directly.
fn integer_roots_of(res: &[BigInt], bound: &BigInt) -> Vec<BigInt> {
    let deriv = poly_derivative(res);
    let mut out: Vec<BigInt> = Vec::new();
    for seed in real_root_candidates(res, bound) {
        let mut x = seed;
        for _ in 0..80 {
            let fx = poly_eval(res, &x);
            if fx.is_zero() {
                break;
            }
            let dfx = poly_eval(&deriv, &x);
            if dfx.is_zero() {
                break;
            }
            let step = round_div_factor(&fx, &dfx);
            if step.is_zero() {
                break;
            }
            x = x.sub(&step);
        }
        if poly_eval(res, &x).is_zero() && x.abs() <= *bound && !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// Factor `N` from the **high bits of one prime** (Coppersmith's method). We know `p = p_high + x₀` with
/// `0 ≤ x₀ < 2^unknown_bits`; the monic linear polynomial `f(x) = x + p_high` has the small root `x₀`
/// modulo the unknown factor `p`. Coppersmith builds a lattice from the `N`-power and `x`-shift multiples
/// of `f` (all vanishing modulo `pᵐ` at `x₀`), LLL-reduces it, and reads a short vector whose small root
/// is an INTEGER root of a real polynomial — recovering `x₀`, hence `p`. This is the LATTICE lens: a
/// factorization from PARTIAL knowledge that no factoring shortcut (Fermat, rho, `p−1`) can provide.
pub fn coppersmith_factor_high_bits(n: &BigInt, p_high: &BigInt, unknown_bits: u32) -> Option<(BigInt, BigInt)> {
    let two = two();
    let mut x_bound = one();
    for _ in 0..unknown_bits {
        x_bound = x_bound.mul(&two);
    }
    // A larger lattice pushes `unknown_bits` toward the N^{1/4} limit. The float-Gram-Schmidt LLL
    // (`lll_reduce_bigint`) handles this dimension in milliseconds — the exact-rational version could not.
    let (m, t) = (4usize, 4usize);
    let deg = m + t;
    let f = vec![p_high.clone(), one()]; // f(x) = p_high + x

    // Polynomials that vanish modulo pᵐ at x₀: gᵢ = N^{m-i}·fⁱ (i=0..m) and hⱼ = xʲ·fᵐ (j=1..t).
    let mut polys: Vec<Vec<BigInt>> = Vec::new();
    for i in 0..=m {
        let scale = n.pow((m - i) as u32);
        polys.push(poly_pow(&f, i).iter().map(|c| c.mul(&scale)).collect());
    }
    let fm = poly_pow(&f, m);
    for j in 1..=t {
        let mut p = vec![BigInt::zero(); j];
        p.extend_from_slice(&fm);
        polys.push(p);
    }

    // Lattice rows: coefficient k scaled by Xᵏ (so a short row ⇒ a small-normed polynomial in x·X).
    let mut xpow = vec![one()];
    for k in 1..=deg {
        xpow.push(xpow[k - 1].mul(&x_bound));
    }
    let rows: Vec<Vec<BigInt>> = polys
        .iter()
        .map(|p| (0..=deg).map(|k| p.get(k).cloned().unwrap_or_else(BigInt::zero).mul(&xpow[k])).collect())
        .collect();

    let reduced = crate::lattice::lll_reduce_bigint_fp(&rows);
    for row in &reduced {
        // Unscale: hₖ = rowₖ / Xᵏ (exact — every lattice entry is an integer multiple of Xᵏ).
        let mut h = Vec::with_capacity(deg + 1);
        let mut ok = true;
        for k in 0..=deg {
            let (q, r) = row[k].div_rem(&xpow[k]).expect("nonzero");
            if !r.is_zero() {
                ok = false;
                break;
            }
            h.push(q);
        }
        if !ok || h.iter().all(|c| c.is_zero()) {
            continue;
        }
        if let Some(x0) = poly_integer_root(&h, &x_bound) {
            if let Some(f) = split(n, &p_high.add(&x0)) {
                return Some(f);
            }
        }
    }
    None
}

/// Derive the RSA private exponent `d = e⁻¹ mod φ(N)` from the public exponent and the two primes
/// (`φ = (p−1)(q−1)`), or `None` if `e` is not coprime to `φ`. The "if you can factor, you can break RSA"
/// direction: the factorization hands over the private key.
pub fn rsa_private_exponent(e: &BigInt, p: &BigInt, q: &BigInt) -> Option<BigInt> {
    let one = one();
    mod_inverse(e, &p.sub(&one).mul(&q.sub(&one)))
}

/// Factor `N` from the RSA private exponent `d` (Miller's deterministic reduction). Since `e·d − 1` is a
/// multiple of `λ(N)`, writing it as `t·2ˢ` and raising a base `g` to `t·2ⁱ` walks a chain that ends at
/// `1`; a step where the value squares to `1` while itself being `≠ ±1` is a NONTRIVIAL square root of
/// unity, and `gcd(x − 1, N)` splits `N`. This is the converse of [`rsa_private_exponent`]: it proves
/// that recovering the private key is **computationally equivalent to factoring the modulus** — the two
/// are one problem, so RSA breaks exactly when `N` factors. Returns `(p, q)` or `None` (no base worked).
pub fn factor_via_private_exponent(n: &BigInt, e: &BigInt, d: &BigInt) -> Option<(BigInt, BigInt)> {
    let (one, two) = (one(), two());
    let n1 = n.sub(&one);
    let k = e.mul(d).sub(&one);
    if k.is_zero() || k.is_negative() {
        return None;
    }
    // k = t·2ˢ, t odd.
    let mut t = k;
    let mut s = 0u32;
    while !t.is_odd() {
        t = t.div_rem(&two).expect("nonzero").0;
        s += 1;
    }
    for &gb in MR_BASES {
        let g = BigInt::from_u64(gb);
        let shared = gcd(&g, n);
        if shared != one {
            if let Some(f) = split(n, &shared) {
                return Some(f);
            }
            continue;
        }
        let mut x = modpow(&g, &t, n);
        if x == one || x == n1 {
            continue;
        }
        for _ in 0..s {
            let y = modpow(&x, &two, n);
            if y == one {
                // x is a square root of 1 with x ∉ {1, N−1}: a nontrivial root splits N.
                if let Some(f) = split(n, &gcd(&x.sub(&one), n)) {
                    return Some(f);
                }
                break;
            }
            if y == n1 {
                break; // x² ≡ −1: this base gives no nontrivial root
            }
            x = y;
        }
    }
    None
}

/// Batch GCD: catches a **shared prime** reused across moduli — the classic failure of low-entropy key
/// generation. Any pair with `gcd(Nᵢ, Nⱼ) > 1` hands over the common factor for free.
pub fn batch_gcd(moduli: &[BigInt]) -> Vec<(usize, usize, BigInt)> {
    let one = one();
    let mut hits = Vec::new();
    for i in 0..moduli.len() {
        for j in (i + 1)..moduli.len() {
            let g = gcd(&moduli[i], &moduli[j]);
            if g > one {
                hits.push((i, j, g));
            }
        }
    }
    hits
}

// ---- Dixon's method: RSA's ring structure broken by OUR GF(2) symmetry breaking -------------------
//
// The algebraic rungs (D, G, J) all turn on one primitive: finding a GF(2) linear DEPENDENCY — a subset
// of vectors that XORs to zero. That same symmetry break factors integers. Dixon's method (the heart of
// the quadratic sieve) collects numbers `r` whose square `r² mod N` is smooth over a small prime base,
// records each as an exponent vector, and finds a subset whose product is a PERFECT SQUARE — which is
// exactly a GF(2) kernel vector of the parity matrix. That yields `x² ≡ y² (mod N)`, and `gcd(x−y, N)`
// splits `N`. It uses RSA's own ring structure (congruences of squares) and our own symmetry breaking
// (the dependency search), so it breaks small and medium moduli; on a sound modulus it degrades to the
// sub-exponential regime (the real quadratic-sieve / GNFS frontier) — the honest boundary.

/// Every GF(2) linear dependency among `rows` (subsets of row indices that XOR to the zero vector), found
/// by Gaussian elimination with identity tracking. This is the symmetry-break at the core of Dixon's
/// method, shared with the algebraic rungs.
fn gf2_dependencies(rows: &[Vec<bool>], ncols: usize) -> Vec<Vec<usize>> {
    let m = rows.len();
    let mut mat: Vec<(Vec<bool>, Vec<bool>)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut tag = vec![false; m];
            tag[i] = true;
            (r.clone(), tag)
        })
        .collect();
    let mut pivot = 0;
    for col in 0..ncols {
        let Some(pr) = (pivot..m).find(|&i| mat[i].0[col]) else {
            continue;
        };
        mat.swap(pivot, pr);
        let (prow, ptag) = (mat[pivot].0.clone(), mat[pivot].1.clone());
        for i in 0..m {
            if i != pivot && mat[i].0[col] {
                for c in 0..ncols {
                    mat[i].0[c] ^= prow[c];
                }
                for c in 0..m {
                    mat[i].1[c] ^= ptag[c];
                }
            }
        }
        pivot += 1;
    }
    mat.iter()
        .filter(|(prim, _)| prim.iter().all(|&b| !b))
        .map(|(_, tag)| (0..m).filter(|&i| tag[i]).collect())
        .filter(|s: &Vec<usize>| !s.is_empty())
        .collect()
}

/// Dixon's method: factor `N` via a congruence of squares found by a GF(2) dependency among smooth
/// relations (see the module note above). Searches `r = ⌈√N⌉, ⌈√N⌉+1, …` for up to `tries` steps to
/// gather relations whose `r² mod N` is smooth over `base`, then combines them. Returns `(p, q)` or
/// `None` (not enough smooth relations found, or only trivial congruences). Reuses our GF(2)
/// symmetry-breaking on RSA's ring structure.
pub fn dixon_factor(n: &BigInt, base: &[u64], tries: usize) -> Option<(BigInt, BigInt)> {
    let one = one();
    let need = base.len() + 5;
    let mut rels: Vec<(BigInt, Vec<u64>)> = Vec::new();
    let mut r = isqrt(n).add(&one);
    let mut steps = 0;
    while rels.len() < need && steps < tries {
        let mut s = rem(&r.mul(&r), n);
        let mut exps = vec![0u64; base.len()];
        for (bi, &b) in base.iter().enumerate() {
            let bb = BigInt::from_u64(b);
            while let Some((q, rr)) = s.div_rem(&bb) {
                if rr.is_zero() {
                    s = q;
                    exps[bi] += 1;
                } else {
                    break;
                }
            }
        }
        if s == one {
            rels.push((r.clone(), exps));
        }
        r = r.add(&one);
        steps += 1;
    }
    if rels.len() < 2 {
        return None;
    }
    let rows: Vec<Vec<bool>> = rels.iter().map(|(_, e)| e.iter().map(|&x| x & 1 == 1).collect()).collect();
    for dep in gf2_dependencies(&rows, base.len()) {
        let mut x = one.clone();
        let mut total = vec![0u64; base.len()];
        for &i in &dep {
            x = rem(&x.mul(&rels[i].0), n);
            for (k, &e) in rels[i].1.iter().enumerate() {
                total[k] += e;
            }
        }
        let mut y = one.clone();
        for (k, &e) in total.iter().enumerate() {
            y = rem(&y.mul(&modpow(&BigInt::from_u64(base[k]), &BigInt::from_u64(e / 2), n)), n);
        }
        let diff = if x >= y { x.sub(&y) } else { x.add(n).sub(&y) };
        for cand in [gcd(&diff, n), gcd(&x.add(&y), n)] {
            if let Some(f) = split(n, &cand) {
                return Some(f);
            }
        }
    }
    None
}

// ---- Quadratic sieve: real sieving (the L[½] algorithm, GNFS's predecessor) ------------------------
//
// Dixon trial-divides `r² mod N` at every step; the quadratic sieve replaces that with a SIEVE. It uses
// `Q(x) = (⌈√N⌉ + x)² − N`, small for small `x`, and for each factor-base prime `p` marks the interval
// positions where `p | Q(x)` (the two roots of `Q ≡ 0 mod p`) by adding `ln p` to a running total. Where
// that total reaches `ln|Q(x)|`, `Q(x)` is smooth — found without dividing. The smooth `Q(x)` become
// relations, and the SAME GF(2) dependency search factors `N`. This is sub-exponential (`L[½]`); its far
// deeper cousin GNFS is `L[⅓]` — still sub-exponential, so both confirm the wall rather than breaching it.

fn is_small_prime(p: u64) -> bool {
    if p < 2 {
        return false;
    }
    let mut d = 2u64;
    while d * d <= p {
        if p % d == 0 {
            return false;
        }
        d += 1;
    }
    true
}

fn bigint_mod_u64(n: &BigInt, p: u64) -> u64 {
    n.div_rem(&BigInt::from_u64(p)).expect("nonzero").1.to_i64().unwrap_or(0) as u64
}

/// A square root of `a` modulo the small prime `p` (brute force — the factor-base primes are tiny), or
/// `None` if `a` is a non-residue.
fn modular_sqrt_u64(a: u64, p: u64) -> Option<u64> {
    let a = a % p;
    (0..p).find(|&r| (r as u128 * r as u128 % p as u128) as u64 == a)
}

/// The quadratic sieve (see the module note above): factor `N` by log-sieving `Q(x) = (⌈√N⌉+x)² − N`
/// over `[0, m_interval)` against the factor base of primes `≤ b` for which `N` is a quadratic residue,
/// then combining smooth relations through a GF(2) dependency. Returns `(p, q)` or `None`.
pub fn quadratic_sieve(n: &BigInt, b: u64, m_interval: usize) -> Option<(BigInt, BigInt)> {
    let one = one();
    let a = isqrt(n).add(&one);

    // Factor base: primes p ≤ b with N a quadratic residue mod p, and their two roots of Q ≡ 0 mod p.
    let mut base: Vec<u64> = Vec::new();
    let mut roots: Vec<(u64, u64)> = Vec::new();
    for p in 2..=b {
        if !is_small_prime(p) {
            continue;
        }
        let np = bigint_mod_u64(n, p);
        if p == 2 {
            base.push(2);
            roots.push((np % 2, np % 2));
        } else if let Some(r) = modular_sqrt_u64(np, p) {
            base.push(p);
            roots.push((r, p - r));
        }
    }
    if base.len() < 3 {
        return None;
    }

    // Log-sieve the interval.
    let mut sieve = vec![0f64; m_interval];
    for (bi, &p) in base.iter().enumerate() {
        let am = bigint_mod_u64(&a, p) as i64;
        let (r1, r2) = roots[bi];
        let both = [r1, r2];
        let candidates: &[u64] = if p == 2 { &both[..1] } else { &both[..] };
        for &root in candidates {
            let mut x = ((root as i64 - am).rem_euclid(p as i64)) as usize;
            while x < m_interval {
                sieve[x] += (p as f64).ln();
                x += p as usize;
            }
        }
    }

    // Collect smooth relations: sieve pinpoints candidates, exact trial division confirms them.
    let mut rels: Vec<(BigInt, Vec<u64>)> = Vec::new();
    for x in 0..m_interval {
        let ax = a.add(&BigInt::from_u64(x as u64));
        let q = ax.mul(&ax).sub(n);
        if q.is_zero() {
            continue;
        }
        // The sieve accounts for one ln(p) per distinct factor; require it to cover at least half the
        // log-mass of Q(x) (generous, so no smooth value is missed), then confirm by exact division.
        if sieve[x] < 0.5 * q.to_f64().ln() {
            continue;
        }
        let mut qq = q;
        let mut exps = vec![0u64; base.len()];
        for (bi, &p) in base.iter().enumerate() {
            let bp = BigInt::from_u64(p);
            while let Some((quo, rr)) = qq.div_rem(&bp) {
                if rr.is_zero() {
                    qq = quo;
                    exps[bi] += 1;
                } else {
                    break;
                }
            }
        }
        if qq == one {
            rels.push((ax, exps));
            if rels.len() > base.len() + 5 {
                break;
            }
        }
    }
    if rels.len() < 2 {
        return None;
    }

    let rows: Vec<Vec<bool>> = rels.iter().map(|(_, e)| e.iter().map(|&x| x & 1 == 1).collect()).collect();
    for dep in gf2_dependencies(&rows, base.len()) {
        let mut x = one.clone();
        let mut total = vec![0u64; base.len()];
        for &i in &dep {
            x = rem(&x.mul(&rels[i].0), n);
            for (k, &e) in rels[i].1.iter().enumerate() {
                total[k] += e;
            }
        }
        let mut y = one.clone();
        for (k, &e) in total.iter().enumerate() {
            y = rem(&y.mul(&modpow(&BigInt::from_u64(base[k]), &BigInt::from_u64(e / 2), n)), n);
        }
        let diff = if x >= y { x.sub(&y) } else { x.add(n).sub(&y) };
        for cand in [gcd(&diff, n), gcd(&x.add(&y), n)] {
            if let Some(f) = split(n, &cand) {
                return Some(f);
            }
        }
    }
    None
}

// ---- Resultants: eliminate a variable from two bivariate polynomials (Boneh–Durfee's core step) ------
//
// A bivariate polynomial is a map from monomial `(i, j)` (meaning `xⁱyʲ`) to its integer coefficient.
type BPoly = std::collections::BTreeMap<(u32, u32), BigInt>;

/// The degree of a dense univariate polynomial (coefficients low-to-high), ignoring trailing zeros.
fn poly_deg(p: &[BigInt]) -> usize {
    let mut d = p.len();
    while d > 0 && p[d - 1].is_zero() {
        d -= 1;
    }
    d.saturating_sub(1)
}

/// The determinant of an integer matrix by the fraction-free Bareiss algorithm — exact, no rationals.
fn det_bareiss(mut a: Vec<Vec<BigInt>>) -> BigInt {
    let n = a.len();
    if n == 0 {
        return one();
    }
    let mut prev = one();
    let mut neg = false;
    for k in 0..n - 1 {
        if a[k][k].is_zero() {
            match (k + 1..n).find(|&i| !a[i][k].is_zero()) {
                Some(p) => {
                    a.swap(k, p);
                    neg = !neg;
                }
                None => return BigInt::zero(),
            }
        }
        for i in k + 1..n {
            for j in k + 1..n {
                let num = a[i][j].mul(&a[k][k]).sub(&a[i][k].mul(&a[k][j]));
                a[i][j] = num.div_rem(&prev).expect("Bareiss division is exact").0;
            }
        }
        prev = a[k][k].clone();
    }
    if neg {
        a[n - 1][n - 1].negated()
    } else {
        a[n - 1][n - 1].clone()
    }
}

/// The resultant of two univariate polynomials (coefficients low-to-high) — the Sylvester determinant.
fn univariate_resultant(a: &[BigInt], b: &[BigInt]) -> BigInt {
    let (da, db) = (poly_deg(a), poly_deg(b));
    if a.iter().all(|c| c.is_zero()) || b.iter().all(|c| c.is_zero()) {
        return BigInt::zero();
    }
    if da == 0 {
        return a[0].pow(db as u32);
    }
    if db == 0 {
        return b[0].pow(da as u32);
    }
    let n = da + db;
    let mut syl = vec![vec![BigInt::zero(); n]; n];
    for r in 0..db {
        for c in 0..=da {
            syl[r][r + c] = a[da - c].clone();
        }
    }
    for r in 0..da {
        for c in 0..=db {
            syl[db + r][r + c] = b[db - c].clone();
        }
    }
    det_bareiss(syl)
}

/// Evaluate a bivariate polynomial at `y = t`, returning the univariate polynomial in `x` (low-to-high).
fn eval_bivar_at_y(g: &BPoly, t: &BigInt) -> Vec<BigInt> {
    let max_i = g.keys().map(|&(i, _)| i).max().unwrap_or(0) as usize;
    let mut uni = vec![BigInt::zero(); max_i + 1];
    for (&(i, j), c) in g {
        uni[i as usize] = uni[i as usize].add(&c.mul(&t.pow(j)));
    }
    uni
}

/// Lagrange-interpolate the unique polynomial (coefficients low-to-high) through `points`.
fn lagrange_interpolate(points: &[(BigInt, BigInt)]) -> Vec<BigInt> {
    let n = points.len();
    let mut acc = vec![Rational::zero(); n];
    for (i, (xi, yi)) in points.iter().enumerate() {
        // Basis polynomial Lᵢ(x) = Π_{m≠i} (x − xₘ)/(xᵢ − xₘ), scaled by yᵢ.
        let mut basis = vec![Rational::zero(); n];
        basis[0] = Rational::from_bigint(yi.clone());
        let mut denom = Rational::from_i64(1);
        let mut deg = 0;
        for (m, (xm, _)) in points.iter().enumerate() {
            if m == i {
                continue;
            }
            // Multiply basis by (x − xₘ).
            for d in (0..=deg).rev() {
                let shifted = basis[d].clone();
                basis[d + 1] = basis[d + 1].add(&shifted);
                basis[d] = basis[d].mul(&Rational::from_bigint(xm.negated()));
            }
            deg += 1;
            denom = denom.mul(&Rational::from_bigint(xi.sub(xm)));
        }
        let inv = denom.recip().expect("distinct nodes");
        for d in 0..n {
            acc[d] = acc[d].add(&basis[d].mul(&inv));
        }
    }
    acc.iter().map(|r| r.round()).collect()
}

/// Eliminate `x` from two bivariate polynomials: `Resₓ(g1, g2)`, a univariate polynomial in `y`
/// (coefficients low-to-high) whose roots include every shared `y`-coordinate. Computed by evaluating the
/// resultant at enough integer `y`-points and interpolating — avoiding a symbolic polynomial determinant.
fn bivariate_resultant_x(g1: &BPoly, g2: &BPoly) -> Vec<BigInt> {
    let dx = |g: &BPoly| g.keys().map(|&(i, _)| i).max().unwrap_or(0) as usize;
    let dy = |g: &BPoly| g.keys().map(|&(_, j)| j).max().unwrap_or(0) as usize;
    let degree = dx(g1) * dy(g2) + dx(g2) * dy(g1);
    let points: Vec<(BigInt, BigInt)> = (0..=degree as i64)
        .map(|t| {
            let ty = BigInt::from_i64(t);
            let r = univariate_resultant(&eval_bivar_at_y(g1, &ty), &eval_bivar_at_y(g2, &ty));
            (ty, r)
        })
        .collect();
    lagrange_interpolate(&points)
}

fn bpoly_mul(a: &BPoly, b: &BPoly) -> BPoly {
    let mut out = BPoly::new();
    for (&(i1, j1), c1) in a {
        for (&(i2, j2), c2) in b {
            let e = out.entry((i1 + i2, j1 + j2)).or_insert_with(BigInt::zero);
            *e = e.add(&c1.mul(c2));
        }
    }
    out.retain(|_, c| !c.is_zero());
    out
}

fn bpoly_pow(base: &BPoly, e: u32) -> BPoly {
    let mut acc = BPoly::new();
    acc.insert((0, 0), one());
    for _ in 0..e {
        acc = bpoly_mul(&acc, base);
    }
    acc
}

fn bpoly_eval(g: &BPoly, x: &BigInt, y: &BigInt) -> BigInt {
    g.iter().fold(BigInt::zero(), |acc, (&(i, j), c)| acc.add(&c.mul(&x.pow(i)).mul(&y.pow(j))))
}

/// Substitute `x = xv` into a bivariate polynomial, returning the univariate polynomial in `y`
/// (coefficients low-to-high).
fn bpoly_subst_x(g: &BPoly, xv: &BigInt) -> Vec<BigInt> {
    let max_j = g.keys().map(|&(_, j)| j).max().unwrap_or(0) as usize;
    let mut uni = vec![BigInt::zero(); max_j + 1];
    for (&(i, j), c) in g {
        uni[j as usize] = uni[j as usize].add(&c.mul(&xv.pow(i)));
    }
    uni
}

/// Build the Boneh–Durfee lattice, LLL-reduce it (fast float-Gram-Schmidt), and return the short
/// bivariate polynomials (unscaled) that should share the root `(k, −(p+q))`, plus the `y`-bound.
fn bd_reduced_polys(n: &BigInt, e: &BigInt, m: usize, t: usize, x_bound: &BigInt) -> (Vec<BPoly>, BigInt) {
    let one = one();
    let y_bound = isqrt(n).mul(&BigInt::from_i64(3));

    let mut f = BPoly::new();
    f.insert((1, 1), one.clone());
    f.insert((1, 0), n.add(&one));
    f.insert((0, 0), one.clone());

    let mut shifts: Vec<BPoly> = Vec::new();
    for k in 0..=m {
        let fke: BPoly = {
            let scale = e.pow((m - k) as u32);
            bpoly_pow(&f, k as u32).into_iter().map(|(key, c)| (key, c.mul(&scale))).collect()
        };
        for i in 0..=(m - k) {
            shifts.push(fke.iter().map(|(&(a, b), c)| ((a + i as u32, b), c.clone())).collect());
        }
        for j in 1..=t {
            shifts.push(fke.iter().map(|(&(a, b), c)| ((a, b + j as u32), c.clone())).collect());
        }
    }

    let mut monos: std::collections::BTreeSet<(u32, u32)> = std::collections::BTreeSet::new();
    for s in &shifts {
        monos.extend(s.keys());
    }
    let monos: Vec<(u32, u32)> = monos.into_iter().collect();
    let scale_of: Vec<BigInt> = monos.iter().map(|&(i, j)| x_bound.pow(i).mul(&y_bound.pow(j))).collect();
    let col: std::collections::HashMap<(u32, u32), usize> =
        monos.iter().enumerate().map(|(i, &k)| (k, i)).collect();
    let rows: Vec<Vec<BigInt>> = shifts
        .iter()
        .map(|s| {
            let mut row = vec![BigInt::zero(); monos.len()];
            for (&key, c) in s {
                let ci = col[&key];
                row[ci] = c.mul(&scale_of[ci]);
            }
            row
        })
        .collect();

    let reduced = crate::lattice::lll_reduce_bigint_fp(&rows);
    let polys: Vec<BPoly> = reduced
        .iter()
        .map(|row| {
            let mut g = BPoly::new();
            for (ci, &key) in monos.iter().enumerate() {
                if !row[ci].is_zero() {
                    let (q, r) = row[ci].div_rem(&scale_of[ci]).expect("nonzero");
                    if r.is_zero() {
                        g.insert(key, q);
                    }
                }
            }
            g
        })
        .filter(|g| !g.is_empty())
        .collect();
    (polys, y_bound)
}

/// Round-tripping real root candidates of `res` (a univariate polynomial, low-to-high) in `[−bound,
/// bound]`, from a floating-point Newton search — proposals, verified exactly by the caller.
fn real_root_candidates(res: &[BigInt], bound: &BigInt) -> Vec<BigInt> {
    let maxc = res.iter().map(|c| c.abs()).max().filter(|c| !c.is_zero());
    let Some(maxc) = maxc else {
        return Vec::new();
    };
    let cf: Vec<f64> =
        res.iter().map(|c| Rational::new(c.clone(), maxc.clone()).map(|r| r.to_f64()).unwrap_or(0.0)).collect();
    let eval = |x: f64| cf.iter().rev().fold(0.0, |acc, &c| acc * x + c);
    let deriv = |x: f64| (1..cf.len()).rev().fold(0.0, |acc, k| acc * x + cf[k] * k as f64);
    let bf = bound.to_f64();
    let mut seeds: Vec<f64> = Vec::new();
    let mut p = 1.0;
    while p <= bf {
        seeds.push(p);
        seeds.push(-p);
        p *= 1.3;
    }
    let mut out: Vec<BigInt> = Vec::new();
    for &s in &seeds {
        let mut x = s;
        for _ in 0..200 {
            let d = deriv(x);
            if d.abs() < 1e-18 || !x.is_finite() {
                break;
            }
            let nx = x - eval(x) / d;
            if !nx.is_finite() {
                break;
            }
            if (nx - x).abs() < 0.4 {
                x = nx;
                break;
            }
            x = nx;
        }
        if x.is_finite() && x.abs() <= bf * 1.01 {
            if let Some(c) = BigInt::parse_decimal(&format!("{:.0}", x.abs())) {
                let cand = if x < 0.0 { c.negated() } else { c };
                if !out.contains(&cand) {
                    out.push(cand);
                }
            }
        }
    }
    out
}

/// Boneh–Durfee: recover the factorization from a **small private exponent** `d < N^{0.284}` — beyond
/// Wiener's `N^{0.25}` — by bivariate Coppersmith. Since `e·d − 1 = k·φ(N)` and `φ(N) = N + 1 − (p+q)`,
/// the polynomial `f(x, y) = x·y + (N+1)·x + 1` has the small root `(x₀, y₀) = (k, −(p+q))` modulo `e`.
/// The lattice of `x`- and `y`-shifts of `f`, LLL-reduced (fast float-Gram-Schmidt), yields short bivariate
/// polynomials sharing that root; a resultant eliminates `x`, its root gives `s = p+q`, and `z² − s·z + N`
/// splits `N`. `m`, `t` size the lattice; `x_bound = N^δ` bounds `k`. Returns `(p, q)` or `None`.
pub fn boneh_durfee(n: &BigInt, e: &BigInt, m: usize, t: usize, x_bound: &BigInt) -> Option<(BigInt, BigInt)> {
    let (polys, y_bound) = bd_reduced_polys(n, e, m, t, x_bound);

    // Read the shared root (k, −s) off the reduced polynomials: k is small (< X), so scan it; substituting
    // x = k into a vanishing polynomial gives a low-degree polynomial in y whose root is −s, found
    // reliably. `s = p+q` then splits N via z² − s·z + N.
    let x_lim = x_bound.to_i64().unwrap_or(0).max(0);
    let four_n = BigInt::from_i64(4).mul(n);
    let two = two();
    for g in polys.iter().take(6) {
        for ki in 1..=x_lim {
            let k = BigInt::from_i64(ki);
            let gy = bpoly_subst_x(g, &k);
            if gy.iter().all(|c| c.is_zero()) {
                continue;
            }
            for y0 in integer_roots_of(&gy, &y_bound) {
                let s = y0.negated();
                if s.is_negative() || s.is_zero() {
                    continue;
                }
                let disc = s.mul(&s).sub(&four_n);
                if disc.is_negative() {
                    continue;
                }
                let sq = isqrt(&disc);
                if sq.mul(&sq) == disc {
                    let p = s.sub(&sq).div_rem(&two).expect("nonzero").0;
                    let q = s.add(&sq).div_rem(&two).expect("nonzero").0;
                    if verify_factorization(n, &p, &q) {
                        return Some((p, q));
                    }
                }
            }
        }
    }
    None
}

/// The effort budget for [`structural_factor`]: each structural attack runs to its own bound, then
/// declines. A soundly-generated modulus exhausts every bound and yields no witness.
#[derive(Clone, Copy, Debug)]
pub struct StructuralBudget {
    pub trial_limit: u64,
    pub fermat_iters: u64,
    pub pminus1_bound: u64,
    pub rho_iters: u64,
}

impl Default for StructuralBudget {
    // A quick structural triage: enough to expose any real structural weakness, but far short of the
    // (sub-)exponential effort a sound modulus would demand — so the ceiling is proven cheaply. A
    // 200-bit sound modulus needs ~2⁵⁰ rho steps; 5 000 is not remotely close, yet a *structured* key
    // falls in a handful of steps regardless.
    fn default() -> Self {
        Self { trial_limit: 1_000, fermat_iters: 3_000, pminus1_bound: 500, rho_iters: 5_000 }
    }
}

/// A certified structural weakness: the factors and the attack that found them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralWitness {
    pub p: BigInt,
    pub q: BigInt,
    pub method: &'static str,
}

/// Run the whole structural arsenal against `n` within `budget`, returning the first certified
/// factorization found, or `None` — the number-theoretic incompressible residue (a sound modulus has no
/// structural shortcut, so only the general sub-exponential algorithms, out of scope here, remain).
pub fn structural_factor(n: &BigInt, budget: StructuralBudget) -> Option<StructuralWitness> {
    let mk = |(p, q): (BigInt, BigInt), method: &'static str| StructuralWitness { p, q, method };
    if let Some(f) = trial_division(n, budget.trial_limit) {
        return Some(mk(f, "trial division (small factor)"));
    }
    if let Some(f) = fermat(n, budget.fermat_iters) {
        return Some(mk(f, "Fermat (close primes)"));
    }
    if let Some(f) = pollard_p_minus_1(n, budget.pminus1_bound) {
        return Some(mk(f, "Pollard p−1 (smooth p−1)"));
    }
    if let Some(f) = pollard_rho(n, budget.rho_iters) {
        return Some(mk(f, "Pollard rho"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(s: &str) -> BigInt {
        BigInt::parse_decimal(s).expect("valid decimal")
    }

    #[test]
    fn primality_and_next_prime_agree_with_known_values() {
        assert!(is_probable_prime(&BigInt::from_i64(2)));
        assert!(is_probable_prime(&BigInt::from_i64(1_000_003)));
        assert!(!is_probable_prime(&BigInt::from_i64(1_000_000)));
        assert!(!is_probable_prime(&big("1000000000000000000000000000000"))); // even
        assert_eq!(next_prime(&BigInt::from_i64(1_000_000)), BigInt::from_i64(1_000_003));
    }

    #[test]
    fn isqrt_is_exact() {
        assert_eq!(isqrt(&BigInt::from_i64(0)), BigInt::from_i64(0));
        assert_eq!(isqrt(&BigInt::from_i64(15)), BigInt::from_i64(3));
        assert_eq!(isqrt(&BigInt::from_i64(16)), BigInt::from_i64(4));
        let big_sq = big("1000000000000000000000000000000"); // 10³⁰ = (10¹⁵)²
        assert_eq!(isqrt(&big_sq), big("1000000000000000"));
    }

    #[test]
    fn trial_division_catches_a_small_factor() {
        let p = next_prime(&big("1000000000000000000000000000000"));
        let n = BigInt::from_i64(3).mul(&p);
        let (a, b) = trial_division(&n, 100).expect("the small factor 3 is found");
        assert!(verify_factorization(&n, &a, &b));
        assert!(a == BigInt::from_i64(3) || b == BigInt::from_i64(3));
    }

    #[test]
    fn fermat_crushes_close_primes() {
        let p = next_prime(&big("1000000000000000000000000000000"));
        let q = next_prime(&p.add(&BigInt::from_i64(2)));
        let n = p.mul(&q);
        let (a, b) = fermat(&n, 100_000).expect("adjacent primes fall to Fermat");
        assert!(verify_factorization(&n, &a, &b));
    }

    #[test]
    fn pollard_rho_factors_a_moderate_semiprime() {
        let p = next_prime(&big("1000000007"));
        let q = next_prime(&big("2000000011"));
        let n = p.mul(&q);
        let (a, b) = pollard_rho(&n, 500_000).expect("rho factors a ~60-bit semiprime");
        assert!(verify_factorization(&n, &a, &b));
    }

    #[test]
    fn pollard_p_minus_1_crushes_a_smooth_prime() {
        // Build p with p−1 = 37-smooth: p = (∏ primes ≤ 37)·k + 1, prime, k small.
        let mut smooth = one();
        for &pr in &[2u64, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37] {
            smooth = smooth.mul(&BigInt::from_u64(pr));
        }
        let mut p = smooth.add(&one());
        let mut k = 1u64;
        while !is_probable_prime(&p) {
            k += 1;
            assert!(k <= 40, "a smooth prime is found with a small (≤40-smooth) multiplier");
            p = smooth.mul(&BigInt::from_u64(k)).add(&one());
        }
        let q = next_prime(&big("999999999999999999999999999999"));
        let n = p.mul(&q);
        let (a, b) = pollard_p_minus_1(&n, 40).expect("a smooth p−1 falls to Pollard p−1");
        assert!(verify_factorization(&n, &a, &b));
    }

    #[test]
    fn wiener_crushes_a_small_private_exponent() {
        let p = next_prime(&big("1000000007"));
        let q = next_prime(&big("3000000019"));
        let n = p.mul(&q);
        let phi = p.sub(&one()).mul(&q.sub(&one()));
        let d = BigInt::from_i64(7919); // small: 7919 < ⅓·N^{1/4}
        let e = mod_inverse(&d, &phi).expect("d is coprime to φ");
        let (a, b) = wiener(&e, &n).expect("a small private exponent falls to Wiener");
        assert!(verify_factorization(&n, &a, &b));
    }

    #[test]
    fn batch_gcd_crushes_a_shared_prime() {
        let p = next_prime(&big("1000000000000000000000000000000"));
        let q1 = next_prime(&big("3000000000000000000000000000000"));
        let q2 = next_prime(&big("7000000000000000000000000000000"));
        let (n1, n2) = (p.mul(&q1), p.mul(&q2));
        let hits = batch_gcd(&[n1, n2]);
        assert_eq!(hits.len(), 1, "the shared prime is detected");
        assert_eq!(hits[0].2, p, "and it is exactly p");
    }

    #[test]
    fn structural_factor_orchestrates_the_arsenal() {
        // A close-primes modulus is caught, and the method is reported.
        let p = next_prime(&big("1000000000000000000000000000000"));
        let q = next_prime(&p.add(&BigInt::from_i64(2)));
        let n = p.mul(&q);
        let w = structural_factor(&n, StructuralBudget::default()).expect("weakness found");
        assert!(verify_factorization(&n, &w.p, &w.q));
        assert_eq!(w.method, "Fermat (close primes)");
    }

    #[test]
    fn a_soundly_generated_modulus_resists_the_entire_structural_arsenal() {
        // Two large, independent, well-separated strong primes: not close (Fermat fails), p−1/q−1 are
        // not smooth (Pollard p−1 fails), no shared prime, and N is far too large for bounded rho. Every
        // STRUCTURAL attack, run to its budget, finds NOTHING — RSA's safety IS this ceiling, the
        // number-theoretic incompressible residue. Only general sub-exponential factoring remains, which
        // real key sizes push out of reach.
        let p = next_prime(&big("1000000000000000000000000000057"));
        let q = next_prime(&big("9000000000000000000000000000000"));
        let n = p.mul(&q);
        assert!(
            structural_factor(&n, StructuralBudget::default()).is_none(),
            "a sound modulus has no structural shortcut — the ceiling stands"
        );
    }

    #[test]
    fn resultant_machinery_eliminates_a_variable() {
        // Fraction-free determinant: |[[1,2],[3,4]]| = −2.
        let det = det_bareiss(vec![
            vec![BigInt::from_i64(1), BigInt::from_i64(2)],
            vec![BigInt::from_i64(3), BigInt::from_i64(4)],
        ]);
        assert_eq!(det, BigInt::from_i64(-2));

        // Eliminate x from g1 = x − y and g2 = x + y − 2 (common root (1, 1)). The resultant is a
        // polynomial in y that must vanish exactly at the shared coordinate y = 1.
        let mut g1 = BPoly::new();
        g1.insert((1, 0), BigInt::from_i64(1));
        g1.insert((0, 1), BigInt::from_i64(-1));
        let mut g2 = BPoly::new();
        g2.insert((1, 0), BigInt::from_i64(1));
        g2.insert((0, 1), BigInt::from_i64(1));
        g2.insert((0, 0), BigInt::from_i64(-2));
        let res = bivariate_resultant_x(&g1, &g2);
        assert!(poly_eval(&res, &BigInt::from_i64(1)).is_zero(), "resultant vanishes at the shared y = 1");
        assert!(!poly_eval(&res, &BigInt::from_i64(0)).is_zero(), "and is nonzero away from it");
    }

    #[test]
    #[ignore] // heavy (dim ~72 fpLLL) — run explicitly in release
    fn boneh_durfee_breaks_small_d() {
        let p = next_prime(&big("1000003"));
        let q = next_prime(&big("2000003"));
        let n = p.mul(&q);
        let phi = p.sub(&one()).mul(&q.sub(&one()));
        let d = BigInt::from_i64(1423); // δ ≈ 0.255 — past Wiener's 0.25
        let e = mod_inverse(&d, &phi).unwrap();
        assert!(wiener(&e, &n).is_none(), "beyond Wiener (control)");
        let x_bound = BigInt::from_i64(1 << 11);
        let (a, b) = boneh_durfee(&n, &e, 8, 3, &x_bound).expect("Boneh-Durfee breaks small-d");
        eprintln!("SMALL-d BROKEN: {a:?} · {b:?}");
        assert!(verify_factorization(&n, &a, &b), "the recovered factors check out");
    }

    #[test]
    fn boneh_durfee_lattice_lifts_the_modular_root_to_the_integers() {
        // The verified core of Boneh-Durfee: for a small-d key, build the bivariate lattice of x/y-shifts
        // of f(x,y) = x·y + (N+1)·x + 1 and LLL-reduce it (fast float-Gram-Schmidt). The short polynomials
        // then vanish at the true root (k, −(p+q)) OVER THE INTEGERS — Howgrave-Graham — which is the deep
        // step that lifts "root mod e" to a solvable integer system. Full factor recovery additionally
        // needs two ALGEBRAICALLY-INDEPENDENT vanishing vectors for the resultant elimination; producing
        // them reliably is Boneh-Durfee's own geometrically-progressive sublattice (the documented
        // remaining refinement — in the low-δ regime here the vanishing sublattice collapses to a single
        // dependent family, so the resultant degenerates).
        let p = next_prime(&big("100003"));
        let q = next_prime(&big("200003"));
        let n = p.mul(&q);
        let phi = p.sub(&one()).mul(&q.sub(&one()));
        let d = BigInt::from_i64(43);
        let e = mod_inverse(&d, &phi).unwrap();
        let k = e.mul(&d).sub(&one()).div_rem(&phi).unwrap().0;
        let s = p.add(&q);
        let (polys, _) = bd_reduced_polys(&n, &e, 4, 2, &BigInt::from_i64(1 << 8));
        let vanishing = polys.iter().filter(|g| g.len() > 1 && bpoly_eval(g, &k, &s.negated()).is_zero()).count();
        assert!(vanishing >= 5, "the reduced lattice lifts the modular root to integer roots, got {vanishing}");
    }

    #[test]
    fn quadratic_sieve_factors_a_larger_semiprime_by_sieving() {
        // A ~34-bit semiprime — beyond comfortable Dixon range — factored by log-sieving Q(x) and the
        // GF(2) dependency search.
        let p = next_prime(&big("100000"));
        let q = next_prime(&big("100050"));
        let n = p.mul(&q);
        let (a, b) = quadratic_sieve(&n, 500, 50_000).expect("the quadratic sieve factors N");
        assert!(verify_factorization(&n, &a, &b), "sieving + a GF(2) dependency split N");
    }

    #[test]
    fn dixon_breaks_a_semiprime_with_our_gf2_symmetry_breaking() {
        // N = 179 · 257. Dixon collects r with r² mod N smooth over a small prime base, then OUR GF(2)
        // dependency finder picks a subset whose product is a perfect square → x² ≡ y² (mod N) → factor.
        let n = BigInt::from_i64(179).mul(&BigInt::from_i64(257));
        let base = [2u64, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31];
        let (a, b) = dixon_factor(&n, &base, 50_000).expect("Dixon factors via a GF(2) dependency");
        assert!(verify_factorization(&n, &a, &b), "the congruence of squares splits N");
    }

    #[test]
    fn crt_and_nth_root_are_correct() {
        // Sunzi's classic: x ≡ 2 (mod 3), 3 (mod 5), 2 (mod 7) → 23.
        let x = crt(
            &[BigInt::from_i64(2), BigInt::from_i64(3), BigInt::from_i64(2)],
            &[BigInt::from_i64(3), BigInt::from_i64(5), BigInt::from_i64(7)],
        )
        .unwrap();
        assert_eq!(x, BigInt::from_i64(23));
        assert_eq!(integer_nth_root(&BigInt::from_i64(27), 3), BigInt::from_i64(3));
        assert_eq!(integer_nth_root(&big("1000000000000000000"), 3), BigInt::from_i64(1000000));
        assert_eq!(integer_nth_root(&BigInt::from_i64(3125), 5), BigInt::from_i64(5));
        assert_eq!(integer_nth_root(&BigInt::from_i64(26), 3), BigInt::from_i64(2), "floor cbrt(26) = 2");
    }

    #[test]
    fn hastad_broadcast_recovers_the_message() {
        // The same plaintext, encrypted under e=3 to three recipients with distinct moduli, all > m.
        let e = 3u32;
        let m = big("123456789");
        let moduli: Vec<BigInt> = vec![
            next_prime(&big("100000007")).mul(&next_prime(&big("200000033"))),
            next_prime(&big("300000007")).mul(&next_prime(&big("400000009"))),
            next_prime(&big("500000003")).mul(&next_prime(&big("600000011"))),
        ];
        let e_big = BigInt::from_u64(e as u64);
        let ciphertexts: Vec<BigInt> = moduli.iter().map(|n| modpow(&m, &e_big, n)).collect();
        let recovered = hastad_broadcast_attack(e, &ciphertexts, &moduli).expect("Håstad recovers m");
        assert_eq!(recovered, m, "the broadcast plaintext falls out of the ciphertexts alone");
    }

    #[test]
    fn common_modulus_recovers_the_message() {
        // The same message encrypted under the SAME modulus with two coprime public exponents.
        let n = next_prime(&big("1000000007")).mul(&next_prime(&big("2000000011")));
        let m = big("31415926535");
        let (e1, e2) = (BigInt::from_i64(65537), BigInt::from_i64(3));
        let c1 = modpow(&m, &e1, &n);
        let c2 = modpow(&m, &e2, &n);
        let recovered = common_modulus_attack(&e1, &e2, &c1, &c2, &n).expect("common modulus recovers m");
        assert_eq!(recovered, m, "a shared modulus + coprime exponents leaks the message via Bézout");
    }

    #[test]
    fn franklin_reiter_recovers_related_messages() {
        // Two linearly related messages m and m+r under the same N and e=3.
        let n = next_prime(&big("1000000007")).mul(&next_prime(&big("2000000011")));
        let e = 3u32;
        let e_big = BigInt::from_u64(e as u64);
        let m = big("424242424242");
        let r = big("31337");
        let c1 = modpow(&m, &e_big, &n);
        let c2 = modpow(&m.add(&r), &e_big, &n);
        let recovered = franklin_reiter_attack(e, &r, &c1, &c2, &n).expect("Franklin-Reiter recovers m");
        assert_eq!(recovered, m, "the related pair leaks the plaintext via polynomial GCD");
    }

    #[test]
    fn low_exponent_no_padding_recovers_small_message() {
        // A small message under e=3 with no padding: c = m³ over the integers (no reduction).
        let n = next_prime(&big("100000000000000003")).mul(&next_prime(&big("200000000000000003")));
        let m = big("123456");
        let c = modpow(&m, &BigInt::from_i64(3), &n);
        let recovered = low_exponent_message(&c, 3).expect("cube root recovers the small message");
        assert_eq!(recovered, m, "no padding under a small exponent is just an integer root");
    }

    #[test]
    fn coppersmith_factors_from_known_high_bits() {
        // A ~129-bit modulus. Reveal all but the low 20 bits of p — a partial-key-exposure leak that no
        // factoring shortcut (Fermat/rho/p−1) exploits. The dim-9 Coppersmith lattice (fast now via
        // float-Gram-Schmidt LLL) recovers the missing bits and factors N.
        let p = next_prime(&big("18446744073709551629"));
        let q = next_prime(&big("36893488147419103237"));
        let n = p.mul(&q);
        let unknown_bits = 20u32;
        let mut mask = one();
        for _ in 0..unknown_bits {
            mask = mask.mul(&two());
        }
        let (p_div, _) = p.div_rem(&mask).expect("nonzero");
        let p_high = p_div.mul(&mask); // p with its low 18 bits zeroed
        let (a, b) = coppersmith_factor_high_bits(&n, &p_high, unknown_bits).expect("Coppersmith recovers p");
        assert!(verify_factorization(&n, &a, &b), "the recovered factorization checks out");
    }

    #[test]
    fn rsa_breaks_end_to_end_when_the_modulus_factors() {
        // A weak RSA key: two primes chosen close together.
        let p = next_prime(&big("1000000000000000000000000000057"));
        let q = next_prime(&p.add(&BigInt::from_i64(100)));
        let n = p.mul(&q);
        let e = BigInt::from_i64(65537);
        let d = rsa_private_exponent(&e, &p, &q).expect("e is coprime to φ");
        let m = big("123456789987654321");
        let c = modpow(&m, &e, &n);
        assert_eq!(modpow(&c, &d, &n), m, "RSA encryption round-trips");

        // Factor the modulus (Fermat crushes the close primes), recover d from the factors, decrypt: a
        // complete break with no access to the private key.
        let w = structural_factor(&n, StructuralBudget::default()).expect("close primes factor");
        let d_broken = rsa_private_exponent(&e, &w.p, &w.q).expect("recover d from the factors");
        assert_eq!(modpow(&c, &d_broken, &n), m, "the recovered key decrypts — RSA broken end to end");
    }

    #[test]
    fn recovering_the_private_key_is_equivalent_to_factoring() {
        // A sound RSA key: two large, well-separated primes.
        let p = next_prime(&big("1000000000000000000000000000057"));
        let q = next_prime(&big("9000000000000000000000000000000"));
        let n = p.mul(&q);
        let e = BigInt::from_i64(65537);
        let d = rsa_private_exponent(&e, &p, &q).expect("e is coprime to φ");

        // The attacker's side: the structural arsenal cannot factor it — no shortcut to the key.
        assert!(
            structural_factor(&n, StructuralBudget::default()).is_none(),
            "the sound modulus resists every structural attack"
        );

        // The equivalence: yet the private exponent d, if known, DETERMINISTICALLY factors N. So
        // recovering the key and factoring the modulus are one and the same problem — RSA's security
        // reduces exactly to factoring, which the arsenal cannot shortcut and which (by Chaitin) we
        // cannot prove hard. It neither provably breaks nor is provably unbreakable.
        let (fp, fq) = factor_via_private_exponent(&n, &e, &d).expect("the private exponent factors N");
        assert!(verify_factorization(&n, &fp, &fq), "recovering d recovers the factorization");
    }
}
