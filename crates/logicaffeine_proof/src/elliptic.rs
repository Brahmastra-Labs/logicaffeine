//! # Elliptic-curve primitives over ℤ/N — the additive→multiplicative lift
//!
//! Pollard's `p−1` gets a single shot at a smooth group order: the multiplicative group `(ℤ/p)*` has the
//! *fixed* order `p−1`, so if that one number isn't smooth, you lose. Lenstra's **Elliptic Curve Method**
//! lifts the problem onto `E(𝔽_p)`, whose order — by Hasse — lies in `[p+1−2√p, p+1+2√p]` and, crucially,
//! **changes when you change the curve.** Every curve is a fresh smoothness lottery ticket, and the cost
//! scales with the size of the *smallest* factor, not `N` — which makes ECM the champion for prying a
//! small-to-medium prime out of an otherwise-huge modulus.
//!
//! We work in **Montgomery form** `B·y² = x³ + A·x² + x` with **x-only `(X:Z)` projective coordinates**.
//! Two payoffs: point arithmetic never needs a modular inverse (only `+`, `−`, `×` mod `N`), and the ONE
//! place an inverse *would* be forced — when a point becomes the identity mod `p` but not mod `q`, so its
//! `Z` is `≡ 0 (mod p)` but `≢ 0 (mod N)` — is exactly where `gcd(Z, N)` hands us the factor. The "failure"
//! of the group law *is* the discovery.
//!
//! This module is the reusable substrate: [`xdbl`], [`xadd`], and [`ladder`] are ordinary elliptic-curve
//! group operations mod `N` and underpin ECDLP / pairing / isogeny work later; [`ecm_factor`] is the
//! flagship application.

use crate::factor::{gcd, mod_inverse, modpow};
use logicaffeine_base::BigInt;
use std::collections::HashMap;

#[inline]
fn i(x: i64) -> BigInt {
    BigInt::from_i64(x)
}

/// The non-negative residue `a mod n` (handles a negative `a`).
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
#[inline]
fn addmod(a: &BigInt, b: &BigInt, n: &BigInt) -> BigInt {
    rem_pos(&a.add(b), n)
}
#[inline]
fn submod(a: &BigInt, b: &BigInt, n: &BigInt) -> BigInt {
    rem_pos(&a.sub(b), n)
}

/// x-only **doubling** on a Montgomery curve mod `n`, given `a24 = (A+2)/4`. Maps `(X:Z)` to `2·(X:Z)`
/// using only field `+ − ×` (no inversion).
pub fn xdbl(x: &BigInt, z: &BigInt, a24: &BigInt, n: &BigInt) -> (BigInt, BigInt) {
    let xpz = addmod(x, z, n);
    let xmz = submod(x, z, n);
    let t1 = mulmod(&xpz, &xpz, n); // (X+Z)²
    let t2 = mulmod(&xmz, &xmz, n); // (X−Z)²
    let x2 = mulmod(&t1, &t2, n); // (X+Z)²(X−Z)²
    let t3 = submod(&t1, &t2, n); // 4XZ = (X+Z)² − (X−Z)²
    let inner = addmod(&t2, &mulmod(a24, &t3, n), n); // (X−Z)² + a24·4XZ
    let z2 = mulmod(&t3, &inner, n);
    (x2, z2)
}

/// x-only **differential addition** on a Montgomery curve mod `n`: given `P=(Xp:Zp)`, `Q=(Xq:Zq)` and their
/// known difference `P−Q = (Xd:Zd)`, return `P+Q`. Again inversion-free.
pub fn xadd(
    xp: &BigInt,
    zp: &BigInt,
    xq: &BigInt,
    zq: &BigInt,
    xd: &BigInt,
    zd: &BigInt,
    n: &BigInt,
) -> (BigInt, BigInt) {
    let a = mulmod(&submod(xp, zp, n), &addmod(xq, zq, n), n); // (Xp−Zp)(Xq+Zq)
    let b = mulmod(&addmod(xp, zp, n), &submod(xq, zq, n), n); // (Xp+Zp)(Xq−Zq)
    let s = addmod(&a, &b, n);
    let d = submod(&a, &b, n);
    let xr = mulmod(zd, &mulmod(&s, &s, n), n); // Zd·(a+b)²
    let zr = mulmod(xd, &mulmod(&d, &d, n), n); // Xd·(a−b)²
    (xr, zr)
}

/// Scalar multiplication `k·(X:Z)` on a Montgomery curve mod `n` by the **Montgomery ladder** — a uniform
/// sequence of one double and one differential-add per bit, maintaining the invariant `R1 − R0 = P`.
pub fn ladder(k: u64, x: &BigInt, z: &BigInt, a24: &BigInt, n: &BigInt) -> (BigInt, BigInt) {
    if k == 0 {
        return (i(0), i(0)); // the identity (a point with Z = 0)
    }
    if k == 1 {
        return (x.clone(), z.clone());
    }
    let mut r0 = (x.clone(), z.clone()); // P
    let mut r1 = xdbl(x, z, a24, n); // 2P  (R1 − R0 = P)
    for bit in (0..(64 - k.leading_zeros() - 1)).rev() {
        if (k >> bit) & 1 == 1 {
            r0 = xadd(&r0.0, &r0.1, &r1.0, &r1.1, x, z, n);
            r1 = xdbl(&r1.0, &r1.1, a24, n);
        } else {
            r1 = xadd(&r0.0, &r0.1, &r1.0, &r1.1, x, z, n);
            r0 = xdbl(&r0.0, &r0.1, a24, n);
        }
    }
    r0
}

/// Primes `≤ b` by the sieve of Eratosthenes.
fn primes_up_to(b: u64) -> Vec<u64> {
    if b < 2 {
        return Vec::new();
    }
    let mut sieve = vec![true; (b as usize) + 1];
    sieve[0] = false;
    sieve[1] = false;
    let mut p = 2usize;
    while p * p <= b as usize {
        if sieve[p] {
            let mut m = p * p;
            while m <= b as usize {
                sieve[m] = false;
                m += p;
            }
        }
        p += 1;
    }
    (2..=b).filter(|&x| sieve[x as usize]).collect()
}

/// **ECM stage 2** — the baby-step/giant-step continuation. After stage 1 has multiplied the base point by
/// every prime power `≤ b1` (leaving `q = (qx:qz)`), stage 2 catches a factor `p` whose group order
/// `#E(𝔽_p)` is `b1`-smooth EXCEPT for a *single* prime `ℓ ∈ (b1, b2]` — such an `ℓ` satisfies
/// `ℓ·q ≡ O (mod p)`. Writing each such prime as `ℓ = i·d ± j`, that is `x((i·d)·q) = x(j·q) (mod p)`; we
/// accumulate the product of all cross-differences `X(T_i)·Z(S_j) − X(S_j)·Z(T_i)` mod `n` and take ONE gcd,
/// so the whole interval costs a single inversion. This is the standard, large power boost over stage 1.
fn ecm_stage2(
    qx: &BigInt,
    qz: &BigInt,
    a24: &BigInt,
    b1: u64,
    b2: u64,
    n: &BigInt,
) -> Option<BigInt> {
    let one = i(1);
    let d = 210u64; // 2·3·5·7 — primes cluster as i·d ± j with |j| ≤ 105
    let half = (d / 2) as usize;

    // Baby steps S[j] = j·q for j = 1..=half (built by the differential-addition chain).
    let mut sx = vec![i(0); half + 1];
    let mut sz = vec![i(0); half + 1];
    sx[1] = qx.clone();
    sz[1] = qz.clone();
    if half >= 2 {
        let s2 = xdbl(qx, qz, a24, n);
        sx[2] = s2.0;
        sz[2] = s2.1;
        for j in 3..=half {
            let s = xadd(&sx[j - 1], &sz[j - 1], qx, qz, &sx[j - 2], &sz[j - 2], n);
            sx[j] = s.0;
            sz[j] = s.1;
        }
    }

    // Bin each prime in (b1, b2] by its nearest giant index i and offset j: ℓ = i·d ± j.
    let mut bins: HashMap<u64, Vec<usize>> = HashMap::new();
    let (mut i_lo, mut i_hi) = (u64::MAX, 0u64);
    for &ell in primes_up_to(b2).iter().filter(|&&p| p > b1) {
        let ii = (ell + d / 2) / d;
        let j = (ell as i64 - (ii * d) as i64).unsigned_abs() as usize;
        if (1..=half).contains(&j) {
            bins.entry(ii).or_default().push(j);
            i_lo = i_lo.min(ii);
            i_hi = i_hi.max(ii);
        }
    }
    if i_lo > i_hi {
        return None; // no primes in range
    }

    // Giant steps T_i = (i·d)·q, advanced by the differential addition T_{i+1} = T_i + d·q (diff T_{i-1}).
    let dq = ladder(d, qx, qz, a24, n);
    let mut t_prev = ladder(i_lo.saturating_sub(1) * d, qx, qz, a24, n);
    let mut t_cur = ladder(i_lo * d, qx, qz, a24, n);
    let mut accum = one.clone();
    for ii in i_lo..=i_hi {
        if ii > i_lo {
            let t_next = xadd(&t_cur.0, &t_cur.1, &dq.0, &dq.1, &t_prev.0, &t_prev.1, n);
            t_prev = t_cur;
            t_cur = t_next;
        }
        if let Some(js) = bins.get(&ii) {
            for &j in js {
                let cross = submod(
                    &mulmod(&t_cur.0, &sz[j], n),
                    &mulmod(&sx[j], &t_cur.1, n),
                    n,
                );
                if !cross.is_zero() {
                    accum = mulmod(&accum, &cross, n);
                }
            }
        }
    }
    let g = gcd(&accum, n);
    (g != one && g != *n).then_some(g)
}

/// Run stage 1 (and stage 2 when `b2 > b1`) on ONE Suyama curve `σ`. Returns a factor or `None`.
fn ecm_one_curve(n: &BigInt, sigma_u: u64, b1: u64, b2: u64, primes1: &[u64]) -> Option<BigInt> {
    let one = i(1);
    let sigma = i(sigma_u as i64);
    let u = submod(&mulmod(&sigma, &sigma, n), &i(5), n); // σ² − 5
    let v = mulmod(&i(4), &sigma, n); // 4σ
    let u3 = mulmod(&mulmod(&u, &u, n), &u, n);
    let v3 = mulmod(&mulmod(&v, &v, n), &v, n);
    let vmu = submod(&v, &u, n);
    let vmu3 = mulmod(&mulmod(&vmu, &vmu, n), &vmu, n);
    let num = mulmod(&vmu3, &addmod(&mulmod(&i(3), &u, n), &v, n), n); // (v−u)³(3u+v)
    let den = mulmod(&mulmod(&i(4), &u3, n), &v, n); // 4u³v

    // The one forced inverse; a nontrivial gcd(den, n) IS a factor (bonus).
    let g = gcd(&den, n);
    if g != one && g != *n {
        return Some(g);
    }
    let a24 = mulmod(&num, &mod_inverse(&den, n)?, n);
    let mut pt = (u3, v3);

    // Stage 1: multiply by ∏_{p ≤ b1} p^{⌊log_p b1⌋}, one prime power at a time.
    for &p in primes1 {
        let mut q = p;
        while q <= b1 {
            pt = ladder(p, &pt.0, &pt.1, &a24, n);
            q = q.saturating_mul(p);
        }
    }
    let g = gcd(&pt.1, n);
    if g != one && g != *n {
        return Some(g);
    }
    if b2 > b1 {
        return ecm_stage2(&pt.0, &pt.1, &a24, b1, b2, n);
    }
    None
}

/// The per-curve Suyama seed: `σ ≥ 6`, varied by curve index and `seed`.
#[inline]
fn suyama_sigma(seed: u64, c: usize) -> u64 {
    6 + seed.wrapping_add(c as u64).wrapping_mul(2_654_435_761) % 1_000_000_000
}

/// **Lenstra's ECM, stage 1 only.** Try `curves` Suyama curves at smoothness bound `b1`; a nontrivial
/// factor of `n`, or `None`. Cost tracks the *smallest* factor, not `n`. `None` for `n ≤ 3`.
pub fn ecm_factor(n: &BigInt, b1: u64, curves: usize, seed: u64) -> Option<BigInt> {
    if n.to_i64().is_some_and(|v| v <= 3) {
        return None;
    }
    if !n.is_odd() {
        return Some(i(2));
    }
    let primes = primes_up_to(b1);
    (0..curves).find_map(|c| ecm_one_curve(n, suyama_sigma(seed, c), b1, b1, &primes))
}

/// **ECM with stage 1 + stage 2.** Stage 2 (bound `b2 > b1`) extends each curve's reach to a group order
/// that is `b1`-smooth apart from one prime `≤ b2` — a large boost per curve for a small extra cost.
pub fn ecm_two_stage(n: &BigInt, b1: u64, b2: u64, curves: usize, seed: u64) -> Option<BigInt> {
    if n.to_i64().is_some_and(|v| v <= 3) {
        return None;
    }
    if !n.is_odd() {
        return Some(i(2));
    }
    let primes = primes_up_to(b1);
    (0..curves).find_map(|c| ecm_one_curve(n, suyama_sigma(seed, c), b1, b2, &primes))
}

/// **ECM stage 3 — the escalating driver.** Stages 1 and 2 are the algorithm; this is the orchestration that
/// makes ECM a complete tool (à la GMP-ECM): run the two-stage method at a schedule of increasing bounds
/// `(b1, b2 = 100·b1)` with growing curve counts, so a factor of *any* size is found at the cheapest level
/// that reaches it — small factors fall almost immediately, larger ones as the bounds climb, without ever
/// paying the big-bound cost up front. `budget` caps the curves per level. `None` if the whole schedule
/// finishes without a factor.
pub fn ecm(n: &BigInt, budget: usize, seed: u64) -> Option<BigInt> {
    const SCHEDULE: &[(u64, usize)] = &[(2_000, 25), (11_000, 90), (50_000, 300), (250_000, 700)];
    for (level, &(b1, curves)) in SCHEDULE.iter().enumerate() {
        let curves = curves.min(budget.max(1));
        if let Some(f) = ecm_two_stage(
            n,
            b1,
            b1.saturating_mul(100),
            curves,
            seed.wrapping_add(level as u64),
        ) {
            return Some(f);
        }
    }
    None
}

// ---- Full Weierstrass arithmetic over 𝔽_p and the ECDLP ---------------------------------------------
//
// ECM *factors*; the elliptic-curve DISCRETE LOG is the other, deeper problem — the one ECC's security
// rests on. Here we build full affine point arithmetic over a prime field and (a) solve the ECDLP by
// baby-step/giant-step, which runs in O(√n) and so *proves the generic wall* (exponential in bit length —
// why sound ECC resists and gets away with small keys), and (b) audit a curve for the exact structural
// symmetries that DO break it: anomalous (Smart, polynomial), supersingular (MOV), smooth order (Pohlig–
// Hellman). The generic ECDLP has NO sub-exponential attack — unlike factoring/DLP — which is the honest
// reason ECC is, per bit, the strongest of the classical assumptions.

/// A short Weierstrass curve `y² = x³ + a·x + b` over the prime field `𝔽_p`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Curve {
    pub a: BigInt,
    pub b: BigInt,
    pub p: BigInt,
}

/// An affine point on a curve, or the point at infinity (the group identity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Point {
    Infinity,
    Affine(BigInt, BigInt),
}

impl Curve {
    pub fn new(a: BigInt, b: BigInt, p: BigInt) -> Curve {
        Curve { a, b, p }
    }

    /// Whether `pt` satisfies the curve equation.
    pub fn is_on_curve(&self, pt: &Point) -> bool {
        match pt {
            Point::Infinity => true,
            Point::Affine(x, y) => {
                let lhs = mulmod(y, y, &self.p);
                let x3 = mulmod(&mulmod(x, x, &self.p), x, &self.p);
                let rhs = addmod(
                    &addmod(&x3, &mulmod(&self.a, x, &self.p), &self.p),
                    &self.b,
                    &self.p,
                );
                lhs == rhs
            }
        }
    }

    /// The group inverse `−P`.
    pub fn negate(&self, pt: &Point) -> Point {
        match pt {
            Point::Infinity => Point::Infinity,
            Point::Affine(x, y) => Point::Affine(x.clone(), submod(&i(0), y, &self.p)),
        }
    }

    /// The group law `P + Q`.
    pub fn add(&self, p1: &Point, p2: &Point) -> Point {
        match (p1, p2) {
            (Point::Infinity, _) => p2.clone(),
            (_, Point::Infinity) => p1.clone(),
            (Point::Affine(x1, y1), Point::Affine(x2, y2)) => {
                if x1 == x2 {
                    if addmod(y1, y2, &self.p).is_zero() {
                        return Point::Infinity; // P = −Q
                    }
                    return self.double(p1); // P = Q
                }
                let den = mod_inverse(&submod(x2, x1, &self.p), &self.p)
                    .expect("distinct x invertible over 𝔽_p");
                let lam = mulmod(&submod(y2, y1, &self.p), &den, &self.p);
                let x3 = submod(
                    &submod(&mulmod(&lam, &lam, &self.p), x1, &self.p),
                    x2,
                    &self.p,
                );
                let y3 = submod(
                    &mulmod(&lam, &submod(x1, &x3, &self.p), &self.p),
                    y1,
                    &self.p,
                );
                Point::Affine(x3, y3)
            }
        }
    }

    /// Point doubling `2P`.
    pub fn double(&self, pt: &Point) -> Point {
        match pt {
            Point::Infinity => Point::Infinity,
            Point::Affine(x, y) => {
                if y.is_zero() {
                    return Point::Infinity; // vertical tangent
                }
                let num = addmod(
                    &mulmod(&i(3), &mulmod(x, x, &self.p), &self.p),
                    &self.a,
                    &self.p,
                );
                let den = mod_inverse(&mulmod(&i(2), y, &self.p), &self.p)
                    .expect("2y invertible over 𝔽_p");
                let lam = mulmod(&num, &den, &self.p);
                let x3 = submod(
                    &mulmod(&lam, &lam, &self.p),
                    &mulmod(&i(2), x, &self.p),
                    &self.p,
                );
                let y3 = submod(&mulmod(&lam, &submod(x, &x3, &self.p), &self.p), y, &self.p);
                Point::Affine(x3, y3)
            }
        }
    }

    /// Scalar multiplication `k·P` by double-and-add (`k ≥ 0`).
    pub fn mul(&self, k: &BigInt, pt: &Point) -> Point {
        let mut result = Point::Infinity;
        let mut addend = pt.clone();
        let (_, bytes) = k.to_le_bytes();
        for byte in bytes {
            for b in 0..8 {
                if (byte >> b) & 1 == 1 {
                    result = self.add(&result, &addend);
                }
                addend = self.double(&addend);
            }
        }
        result
    }

    /// `#E(𝔽_p)` by direct point counting (includes the identity). `O(p)` — small `p` only.
    pub fn count_points(&self) -> u64 {
        let p_u = self.p.to_i64().expect("small prime") as u64;
        let exp = self.p.sub(&i(1)).div_rem(&i(2)).unwrap().0; // (p−1)/2 — the Legendre symbol exponent
        let one = i(1);
        let mut count = 1u64; // infinity
        for xu in 0..p_u {
            let x = i(xu as i64);
            let x3 = mulmod(&mulmod(&x, &x, &self.p), &x, &self.p);
            let rhs = addmod(
                &addmod(&x3, &mulmod(&self.a, &x, &self.p), &self.p),
                &self.b,
                &self.p,
            );
            if rhs.is_zero() {
                count += 1;
            } else if modpow(&rhs, &exp, &self.p) == one {
                count += 2; // a quadratic residue has two square roots
            }
        }
        count
    }

    /// The order of `pt` — least `k > 0` with `k·pt = O`, searched up to `bound`; `None` if larger.
    pub fn point_order(&self, pt: &Point, bound: u64) -> Option<u64> {
        let mut cur = pt.clone();
        for k in 1..=bound {
            if cur == Point::Infinity {
                return Some(k);
            }
            cur = self.add(&cur, pt);
        }
        None
    }
}

fn point_key(pt: &Point) -> Vec<u8> {
    match pt {
        Point::Infinity => vec![0xff],
        Point::Affine(x, y) => {
            let (_, xb) = x.to_le_bytes();
            let (_, yb) = y.to_le_bytes();
            let mut k = vec![0x00, xb.len() as u8];
            k.extend(xb);
            k.extend(yb);
            k
        }
    }
}

/// Solve the **elliptic-curve discrete log** `Q = k·P` by baby-step/giant-step, given the order `n` of `P`.
/// `O(√n)` group operations — the best generic attack, and exponential in the bit length. This is exactly
/// why a sound (large prime-order) curve resists, and why ECC uses far smaller keys than RSA. `None` if `Q`
/// is not a multiple of `P`.
pub fn ecdlp_bsgs(curve: &Curve, base: &Point, target: &Point, n: u64) -> Option<BigInt> {
    let m = (n as f64).sqrt() as u64 + 1;
    let mut baby: HashMap<Vec<u8>, u64> = HashMap::new();
    let mut cur = Point::Infinity;
    for j in 0..m {
        baby.entry(point_key(&cur)).or_insert(j);
        cur = curve.add(&cur, base);
    }
    let neg_mp = curve.negate(&curve.mul(&i(m as i64), base)); // −(m·P)
    let mut gamma = target.clone();
    for step in 0..=m {
        if let Some(&j) = baby.get(&point_key(&gamma)) {
            return Some(i((step * m + j) as i64));
        }
        gamma = curve.add(&gamma, &neg_mp);
    }
    None
}

/// The known STRUCTURAL weaknesses that make an elliptic curve's ECDLP breakable — the symmetries a sound
/// curve must avoid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CurveWeakness {
    /// `#E(𝔽_p) = p` (trace 1): **anomalous** — Smart's attack solves the ECDLP in *polynomial* time.
    Anomalous,
    /// `#E(𝔽_p) = p + 1` (trace 0): **supersingular** — small embedding degree; MOV maps to a sub-exp DLP.
    Supersingular,
    /// The order's largest prime factor is small: **Pohlig–Hellman** shrinks the ECDLP to that subgroup.
    SmoothOrder { largest_prime_factor: u64 },
}

/// Audit a curve of known order `#E` over `𝔽_p` for the structural ECDLP weaknesses. `None` means the only
/// attacks that apply are the generic `O(√·)` ones — the honest ceiling, not a proof of security.
pub fn curve_security(p: u64, order: u64) -> Option<CurveWeakness> {
    if order == p {
        return Some(CurveWeakness::Anomalous);
    }
    if order == p + 1 {
        return Some(CurveWeakness::Supersingular);
    }
    let lpf = largest_prime_factor(order);
    if lpf.saturating_mul(lpf) < order {
        return Some(CurveWeakness::SmoothOrder {
            largest_prime_factor: lpf,
        });
    }
    None
}

fn largest_prime_factor(mut m: u64) -> u64 {
    let mut largest = 1u64;
    let mut d = 2u64;
    while d * d <= m {
        while m % d == 0 {
            largest = d;
            m /= d;
        }
        d += 1;
    }
    largest.max(m)
}

// ---- Isogenies: the maps BETWEEN elliptic curves — where the new mathematics lives -------------------
//
// An isogeny φ: E → E' is a nonconstant morphism that is also a group homomorphism; its kernel is a finite
// subgroup, and its degree is the kernel size. Isogenies are the edges of the isogeny graph, the object
// underneath CSIDH and SIKE — and the place a symmetry break (Castryck–Decru, 2022) collapsed a whole
// post-quantum finalist by exploiting hidden torsion/endomorphism structure. Two curves are the same
// vertex iff they share a **j-invariant** (the isomorphism symmetry). Given a kernel, **Vélu's formulas**
// build φ and the codomain E'. A deep invariant we exploit as a test oracle: isogenous curves have the
// SAME number of points (Tate) — the group order is an isogeny invariant.

impl Curve {
    /// The **j-invariant** `1728 · 4a³ / (4a³ + 27b²)` — the complete isomorphism invariant of the curve
    /// (two curves are isomorphic over the algebraic closure iff their j-invariants agree). `None` if the
    /// curve is singular (`4a³ + 27b² ≡ 0`).
    pub fn j_invariant(&self) -> Option<BigInt> {
        let p = &self.p;
        let a3 = mulmod(&mulmod(&self.a, &self.a, p), &self.a, p);
        let four_a3 = mulmod(&i(4), &a3, p);
        let disc = addmod(
            &four_a3,
            &mulmod(&i(27), &mulmod(&self.b, &self.b, p), p),
            p,
        );
        if disc.is_zero() {
            return None;
        }
        Some(mulmod(
            &mulmod(&i(1728), &four_a3, p),
            &mod_inverse(&disc, p)?,
            p,
        ))
    }
}

/// A separable isogeny `φ: E → E'` of odd prime degree `ℓ`, built from a kernel generator by Vélu's
/// formulas. Stores the codomain and, for each kernel representative `Q` (one per `±` pair), the Vélu
/// quantities `(xQ, vQ = 6xQ²+2a, uQ = 4yQ²)` used to evaluate `φ`.
#[derive(Clone, Debug)]
pub struct Isogeny {
    pub domain: Curve,
    pub codomain: Curve,
    pub degree: u64,
    kernel: Vec<(BigInt, BigInt, BigInt)>, // (xQ, vQ, uQ)
}

impl Isogeny {
    /// Build the `ℓ`-isogeny with kernel `⟨gen⟩` by **Vélu's formulas** (`ℓ` an odd prime, `gen` of order
    /// exactly `ℓ`). The codomain is `y² = x³ + (a − 5v)x + (b − 7w)` where `v = Σ vQ`, `w = Σ (uQ + xQ·vQ)`
    /// over the `(ℓ−1)/2` kernel representatives. `None` unless `ℓ` is an odd prime and `gen` has order `ℓ`.
    pub fn from_kernel(curve: &Curve, gen: &Point, ell: u64) -> Option<Isogeny> {
        if ell < 3 || ell % 2 == 0 {
            return None;
        }
        let p = &curve.p;
        let mut kernel = Vec::with_capacity(((ell - 1) / 2) as usize);
        let mut cur = gen.clone();
        for _ in 0..((ell - 1) / 2) {
            match &cur {
                Point::Affine(xq, yq) => {
                    let vq = addmod(
                        &mulmod(&i(6), &mulmod(xq, xq, p), p),
                        &mulmod(&i(2), &curve.a, p),
                        p,
                    );
                    let uq = mulmod(&i(4), &mulmod(yq, yq, p), p);
                    kernel.push((xq.clone(), vq, uq));
                }
                Point::Infinity => return None, // gen's order is smaller than ℓ
            }
            cur = curve.add(&cur, gen);
        }
        // Confirm the order is exactly ℓ (odd prime): ℓ·gen = O and gen ≠ O.
        let _ = cur;
        if matches!(gen, Point::Infinity) || curve.mul(&i(ell as i64), gen) != Point::Infinity {
            return None;
        }
        let (mut vsum, mut wsum) = (i(0), i(0));
        for (xq, vq, uq) in &kernel {
            vsum = addmod(&vsum, vq, p);
            wsum = addmod(&wsum, &addmod(uq, &mulmod(xq, vq, p), p), p);
        }
        let a2 = submod(&curve.a, &mulmod(&i(5), &vsum, p), p);
        let b2 = submod(&curve.b, &mulmod(&i(7), &wsum, p), p);
        Some(Isogeny {
            domain: curve.clone(),
            codomain: Curve::new(a2, b2, p.clone()),
            degree: ell,
            kernel,
        })
    }

    /// Evaluate `φ` at a point. Kernel points map to the identity. For `P = (x, y) ∉ ker`, the x-map is
    /// `X = x + Σ [ vQ/(x−xQ) + uQ/(x−xQ)² ]` and — since a normalized isogeny pulls back the invariant
    /// differential (`φ*(dX/Y) = dx/y`) — the y-coordinate is `Y = y · X'(x) = y·[1 − Σ (vQ/(x−xQ)² +
    /// 2uQ/(x−xQ)³)]`.
    pub fn eval(&self, pt: &Point) -> Point {
        let p = &self.domain.p;
        match pt {
            Point::Infinity => Point::Infinity,
            Point::Affine(x, y) => {
                let mut xnew = x.clone();
                let mut yfac = i(1);
                for (xq, vq, uq) in &self.kernel {
                    let d = submod(x, xq, p);
                    if d.is_zero() {
                        return Point::Infinity; // P = ±Q ∈ ker
                    }
                    let di = mod_inverse(&d, p).expect("nonzero over 𝔽_p");
                    let di2 = mulmod(&di, &di, p);
                    let di3 = mulmod(&di2, &di, p);
                    xnew = addmod(
                        &xnew,
                        &addmod(&mulmod(vq, &di, p), &mulmod(uq, &di2, p), p),
                        p,
                    );
                    yfac = submod(
                        &yfac,
                        &addmod(
                            &mulmod(vq, &di2, p),
                            &mulmod(&mulmod(&i(2), uq, p), &di3, p),
                            p,
                        ),
                        p,
                    );
                }
                Point::Affine(xnew, mulmod(y, &yfac, p))
            }
        }
    }
}

// ---- The Weil pairing: the ultimate symmetry on torsion --------------------------------------------
//
// `e_N: E[N] × E[N] → μ_N` is the canonical pairing on the N-torsion — bilinear, alternating
// (`e(P,P)=1`), non-degenerate, and Galois/isogeny-compatible: `e_N(φP, φQ) = e_N(P,Q)^{deg φ}`. That last
// law is the symmetry the torsion images in SIKE are FORCED to obey, and the lever the Castryck–Decru
// break pulls. We compute it by Miller's algorithm (a double-and-add accumulating line-function values).

/// The pair `(ℓ(Q), v(Q))` for the DOUBLING step: the tangent line at `T` and the vertical at `2T`,
/// evaluated at `Q = (xq, yq)`.
fn miller_double_lines(c: &Curve, t: &Point, xq: &BigInt, yq: &BigInt) -> Option<(BigInt, BigInt)> {
    let p = &c.p;
    let (xt, yt) = match t {
        Point::Affine(x, y) => (x, y),
        Point::Infinity => return None,
    };
    if yt.is_zero() {
        return None; // 2-torsion; avoided for odd order
    }
    let lam = mulmod(
        &addmod(&mulmod(&i(3), &mulmod(xt, xt, p), p), &c.a, p),
        &mod_inverse(&mulmod(&i(2), yt, p), p)?,
        p,
    );
    let ell = submod(&submod(yq, yt, p), &mulmod(&lam, &submod(xq, xt, p), p), p);
    let t2 = c.double(t);
    let vert = match &t2 {
        Point::Affine(x2, _) => submod(xq, x2, p),
        Point::Infinity => i(1), // 2T = O: no vertical factor
    };
    Some((ell, vert))
}

/// The pair `(ℓ(Q), v(Q))` for the ADDITION step: the chord through `T` and `P`, and the vertical at
/// `T + P`, evaluated at `Q`.
fn miller_add_lines(
    c: &Curve,
    t: &Point,
    pp: &Point,
    xq: &BigInt,
    yq: &BigInt,
) -> Option<(BigInt, BigInt)> {
    let p = &c.p;
    let ((xt, yt), (xpp, ypp)) = match (t, pp) {
        (Point::Affine(a, b), Point::Affine(cc, d)) => ((a, b), (cc, d)),
        _ => return None,
    };
    if xt == xpp {
        if yt == ypp {
            return miller_double_lines(c, t, xq, yq); // T = P
        }
        // T = −P ⟹ T + P = O: the line is the vertical through xt.
        return Some((submod(xq, xt, p), i(1)));
    }
    let lam = mulmod(
        &submod(ypp, yt, p),
        &mod_inverse(&submod(xpp, xt, p), p)?,
        p,
    );
    let ell = submod(&submod(yq, yt, p), &mulmod(&lam, &submod(xq, xt, p), p), p);
    let sum = c.add(t, pp);
    let vert = match &sum {
        Point::Affine(x3, _) => submod(xq, x3, p),
        Point::Infinity => i(1),
    };
    Some((ell, vert))
}

/// Miller's function `f_{n,P}(Q)` as a field element (numerator·denominator⁻¹, batched to one inversion).
fn miller(c: &Curve, pp: &Point, qq: &Point, n: u64) -> Option<BigInt> {
    let p = &c.p;
    let (xq, yq) = match qq {
        Point::Affine(x, y) => (x, y),
        Point::Infinity => return None,
    };
    let (mut num, mut den) = (i(1), i(1));
    let mut t = pp.clone();
    for bit in (0..(64 - n.leading_zeros() - 1)).rev() {
        let (ell, vert) = miller_double_lines(c, &t, xq, yq)?;
        num = mulmod(&mulmod(&num, &num, p), &ell, p);
        den = mulmod(&mulmod(&den, &den, p), &vert, p);
        t = c.double(&t);
        if (n >> bit) & 1 == 1 {
            let (ell, vert) = miller_add_lines(c, &t, pp, xq, yq)?;
            num = mulmod(&num, &ell, p);
            den = mulmod(&den, &vert, p);
            t = c.add(&t, pp);
        }
    }
    if den.is_zero() {
        return None;
    }
    Some(mulmod(&num, &mod_inverse(&den, p)?, p))
}

/// The **Weil pairing** `e_N(P, Q) ∈ μ_N` for independent `N`-torsion points `P, Q`, via Miller:
/// `e_N(P,Q) = (−1)ᴺ · f_{N,P}(Q) / f_{N,Q}(P)`. Bilinear, alternating, non-degenerate, and (the SIKE-
/// relevant law) isogeny-compatible. `None` if a Miller evaluation degenerates (e.g. dependent points).
pub fn weil_pairing(c: &Curve, pp: &Point, qq: &Point, n: u64) -> Option<BigInt> {
    if pp == qq || matches!(pp, Point::Infinity) || matches!(qq, Point::Infinity) {
        return Some(i(1)); // e(P,P)=1 and the degenerate cases
    }
    let fp = miller(c, pp, qq, n)?;
    let fq = miller(c, qq, pp, n)?;
    let ratio = mulmod(&fp, &mod_inverse(&fq, &c.p)?, &c.p);
    let e = if n % 2 == 1 {
        submod(&i(0), &ratio, &c.p)
    } else {
        ratio
    }; // (−1)ᴺ
    Some(e)
}

/// A square root of `a` mod `p` for `p ≡ 3 (mod 4)` (the SIDH regime): `a^{(p+1)/4}`; `None` for a
/// non-residue.
fn sqrt_fp(a: &BigInt, p: &BigInt) -> Option<BigInt> {
    let exp = p.add(&i(1)).div_rem(&i(4))?.0;
    let r = modpow(a, &exp, p);
    (mulmod(&r, &r, p) == rem_pos(a, p)).then_some(r)
}

/// Every affine point of the curve (requires `p ≡ 3 (mod 4)`). `O(p)`.
fn all_affine_points(curve: &Curve) -> Vec<Point> {
    let pu = curve.p.to_i64().expect("small prime") as u64;
    let mut v = Vec::new();
    for xu in 0..pu {
        let x = i(xu as i64);
        let x3 = mulmod(&mulmod(&x, &x, &curve.p), &x, &curve.p);
        let rhs = addmod(
            &addmod(&x3, &mulmod(&curve.a, &x, &curve.p), &curve.p),
            &curve.b,
            &curve.p,
        );
        if rhs.is_zero() {
            v.push(Point::Affine(x, i(0)));
        } else if let Some(y) = sqrt_fp(&rhs, &curve.p) {
            v.push(Point::Affine(x.clone(), submod(&i(0), &y, &curve.p)));
            v.push(Point::Affine(x, y));
        }
    }
    v
}

/// A **torsion basis** of `E[n]` (`n` prime): two independent points of order `n` generating the full
/// `n`-torsion `(ℤ/n)²`, or `None` if the `n`-torsion is not fully rational. Requires `p ≡ 3 (mod 4)`. This
/// is the public torsion basis whose images an SIDH/SIKE key publishes.
pub fn torsion_basis(curve: &Curve, n: u64) -> Option<(Point, Point)> {
    let order_n: Vec<Point> = all_affine_points(curve)
        .into_iter()
        .filter(|pt| curve.mul(&i(n as i64), pt) == Point::Infinity)
        .collect();
    for p in &order_n {
        let span: Vec<Point> = (0..n).map(|k| curve.mul(&i(k as i64), p)).collect();
        if let Some(q) = order_n.iter().find(|q| !span.contains(q)) {
            return Some((p.clone(), q.clone()));
        }
    }
    None
}

/// A single point of prime order `ell` on the curve (a candidate isogeny-kernel generator), or `None` if
/// there is no rational `ell`-torsion. Requires `p ≡ 3 (mod 4)`.
pub fn point_of_order(curve: &Curve, ell: u64) -> Option<Point> {
    all_affine_points(curve)
        .into_iter()
        .find(|pt| curve.mul(&i(ell as i64), pt) == Point::Infinity)
}

/// The SIDH kernel generator `P + [s]·Q` from a torsion basis `(P, Q)` and the secret scalar `s`. The secret
/// isogeny is the quotient by `⟨P + [s]Q⟩`; Kani's gluing is what lets an attacker recover `s` from the
/// published torsion images, after which [`derive_isogeny_path`] unfolds the whole chain.
pub fn kernel_generator(curve: &Curve, p_pt: &Point, q_pt: &Point, s: &BigInt) -> Point {
    curve.add(p_pt, &curve.mul(s, q_pt))
}

/// A single `ℓ`-isogeny step in a chain: the `domain` curve, the order-`ℓ` `kernel` point quotiented at this
/// step, and the resulting `codomain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsogenyStep {
    pub domain: Curve,
    pub kernel: Point,
    pub codomain: Curve,
}

/// **Torsion-image → path derivation.** A degree-`ℓᵃ` isogeny is pinned down by a *single* kernel generator
/// `gen` of order `ℓᵃ` — the datum the SIDH/SIKE torsion images reconstruct (via Kani's gluing). This unfolds
/// that one generator into the explicit chain of `a` prime-degree `ℓ`-isogenies: at step `i` the kernel is
/// the order-`ℓ` point `[ℓ^{a−1−i}]·genᵢ`, and `gen` is pushed forward through each step so its order descends
/// `ℓᵃ → ℓᵃ⁻¹ → … → 1`. The entire secret path pops out of the one meta-datum — the generator is the rule
/// that emits the per-step rules. Requires `ℓ` an odd prime and `gen` of order exactly `ℓᵃ`.
pub fn derive_isogeny_path(
    curve: &Curve,
    gen: &Point,
    ell: u64,
    a: u32,
) -> Option<Vec<IsogenyStep>> {
    let mut steps = Vec::with_capacity(a as usize);
    let mut e = curve.clone();
    let mut g = gen.clone();
    for step in 0..a {
        let mut mult = i(1);
        for _ in 0..(a - 1 - step) {
            mult = mult.mul(&i(ell as i64));
        }
        let k = e.mul(&mult, &g); // [ℓ^{a−1−step}]·g — order exactly ℓ
        let iso = Isogeny::from_kernel(&e, &k, ell)?;
        steps.push(IsogenyStep {
            domain: e.clone(),
            kernel: k,
            codomain: iso.codomain.clone(),
        });
        g = iso.eval(&g);
        e = iso.codomain;
    }
    Some(steps)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/elliptic/tests.rs"]
mod tests;
