//! # 𝔽_{p²} arithmetic and the supersingular isogeny graph
//!
//! The quadratic extension `𝔽_{p²} = 𝔽_p[i]/(i²+1)` (for `p ≡ 3 (mod 4)`, so `−1` is a non-residue and
//! `i² = −1` is irreducible) is where the supersingular curves live — and so where CSIDH, SIKE, and the
//! 2022 Castryck–Decru break happen. This module gives the field arithmetic and builds the **supersingular
//! ℓ=2 isogeny graph**: vertices are supersingular j-invariants in `𝔽_{p²}`, edges the 2-isogenies. We
//! build it not through Vélu but through the classical **level-2 modular polynomial** `Φ₂(X,Y)` — the three
//! roots of `Φ₂(j, ·)` are exactly the j-invariants 2-isogenous to `j`. So the whole graph is field
//! arithmetic + cubic root-finding + a breadth-first walk from a known supersingular vertex (`j = 1728`,
//! supersingular whenever `p ≡ 3 (mod 4)`). The result is a 3-regular **Ramanujan graph** whose size is the
//! supersingular mass `⌊p/12⌋ + ε`.

use crate::factor::mod_inverse;
use logicaffeine_base::BigInt;

/// An element `a + b·i` of `𝔽_{p²} = 𝔽_p[i]/(i²+1)`.
pub type Fp2 = (BigInt, BigInt);

#[inline]
fn ib(x: i64) -> BigInt {
    BigInt::from_i64(x)
}

#[inline]
fn rem_pos(a: &BigInt, p: &BigInt) -> BigInt {
    let r = a.div_rem(p).map(|(_, r)| r).unwrap_or_else(|| a.clone());
    if r.is_negative() {
        r.add(p)
    } else {
        r
    }
}

#[inline]
fn m(a: &BigInt, b: &BigInt, p: &BigInt) -> BigInt {
    rem_pos(&a.mul(b), p)
}
#[inline]
fn s(a: &BigInt, b: &BigInt, p: &BigInt) -> BigInt {
    rem_pos(&a.sub(b), p)
}
#[inline]
fn a_(a: &BigInt, b: &BigInt, p: &BigInt) -> BigInt {
    rem_pos(&a.add(b), p)
}

/// A rational constant `k` as an element of `𝔽_{p²}`.
pub fn fp2_const(k: i64, p: &BigInt) -> Fp2 {
    (rem_pos(&ib(k), p), ib(0))
}

pub fn fp2_add(x: &Fp2, y: &Fp2, p: &BigInt) -> Fp2 {
    (a_(&x.0, &y.0, p), a_(&x.1, &y.1, p))
}
pub fn fp2_sub(x: &Fp2, y: &Fp2, p: &BigInt) -> Fp2 {
    (s(&x.0, &y.0, p), s(&x.1, &y.1, p))
}
pub fn fp2_neg(x: &Fp2, p: &BigInt) -> Fp2 {
    (s(&ib(0), &x.0, p), s(&ib(0), &x.1, p))
}

/// `(a+bi)(c+di) = (ac − bd) + (ad + bc)i`.
pub fn fp2_mul(x: &Fp2, y: &Fp2, p: &BigInt) -> Fp2 {
    let ac = m(&x.0, &y.0, p);
    let bd = m(&x.1, &y.1, p);
    let ad = m(&x.0, &y.1, p);
    let bc = m(&x.1, &y.0, p);
    (s(&ac, &bd, p), a_(&ad, &bc, p))
}

/// `(a+bi)⁻¹ = (a − bi)/(a² + b²)`. `None` only for `0`.
pub fn fp2_inv(x: &Fp2, p: &BigInt) -> Option<Fp2> {
    let norm = a_(&m(&x.0, &x.0, p), &m(&x.1, &x.1, p), p);
    let ninv = mod_inverse(&norm, p)?;
    Some((m(&x.0, &ninv, p), m(&s(&ib(0), &x.1, p), &ninv, p)))
}

pub fn fp2_is_zero(x: &Fp2) -> bool {
    x.0.is_zero() && x.1.is_zero()
}

// ---- The level-2 modular polynomial and the supersingular graph -------------------------------------

/// `Φ₂(j, Y)` as the cubic `c₃Y³ + c₂Y² + c₁Y + c₀` over `𝔽_{p²}` (returned low-degree first). The classical
/// polynomial `Φ₂(X,Y) = X³ + Y³ − X²Y² + 1488(X²Y+XY²) − 162000(X²+Y²) + 40773375·XY + 8748000000(X+Y) −
/// 157464000000000`, with `X = j` fixed.
fn phi2_in_y(j: &Fp2, p: &BigInt) -> [Fp2; 4] {
    let j2 = fp2_mul(j, j, p);
    let j3 = fp2_mul(&j2, j, p);
    let k = |c: i64| fp2_const(c, p);
    let c3 = k(1);
    // c2 = −j² + 1488·j − 162000
    let c2 = fp2_sub(&fp2_add(&fp2_neg(&j2, p), &fp2_mul(&k(1488), j, p), p), &k(162000), p);
    // c1 = 1488·j² + 40773375·j + 8748000000
    let c1 = fp2_add(
        &fp2_add(&fp2_mul(&k(1488), &j2, p), &fp2_mul(&k(40773375), j, p), p),
        &k(8_748_000_000),
        p,
    );
    // c0 = j³ − 162000·j² + 8748000000·j − 157464000000000
    let c0 = fp2_sub(
        &fp2_add(&fp2_sub(&j3, &fp2_mul(&k(162000), &j2, p), p), &fp2_mul(&k(8_748_000_000), j, p), p),
        &k(157_464_000_000_000),
        p,
    );
    [c0, c1, c2, c3]
}

/// Evaluate a polynomial (low-degree-first coefficients) at `y` over `𝔽_{p²}` by Horner's rule.
fn poly_eval(coeffs: &[Fp2], y: &Fp2, p: &BigInt) -> Fp2 {
    let mut acc = fp2_const(0, p);
    for c in coeffs.iter().rev() {
        acc = fp2_add(&fp2_mul(&acc, y, p), c, p);
    }
    acc
}

/// Divide a polynomial by `(Y − r)` over `𝔽_{p²}` (synthetic division); returns `(quotient, remainder)`.
fn poly_div_linear(coeffs: &[Fp2], r: &Fp2, p: &BigInt) -> (Vec<Fp2>, Fp2) {
    let mut q = vec![fp2_const(0, p); coeffs.len().saturating_sub(1)];
    let mut carry = fp2_const(0, p);
    for k in (0..coeffs.len()).rev() {
        let cur = fp2_add(&coeffs[k], &fp2_mul(&carry, r, p), p);
        if k == 0 {
            return (q, cur); // remainder
        }
        q[k - 1] = cur.clone();
        carry = cur;
    }
    (q, fp2_const(0, p))
}

/// All elements of `𝔽_{p²}` (small `p` only).
fn fp2_elements(p: &BigInt) -> Vec<Fp2> {
    let pu = p.to_i64().expect("small prime") as i64;
    let mut v = Vec::new();
    for a in 0..pu {
        for b in 0..pu {
            v.push((ib(a), ib(b)));
        }
    }
    v
}

/// The roots of a cubic over `𝔽_{p²}`, each paired with its multiplicity (found by brute force over the
/// field, then divided out). For a supersingular `j`, `Φ₂(j,·)` splits completely, so the multiplicities
/// sum to 3.
fn cubic_roots(coeffs: &[Fp2; 4], p: &BigInt) -> Vec<(Fp2, usize)> {
    let mut out = Vec::new();
    for y in fp2_elements(p) {
        if fp2_is_zero(&poly_eval(coeffs, &y, p)) {
            // multiplicity by repeated division
            let mut poly: Vec<Fp2> = coeffs.to_vec();
            let mut mult = 0usize;
            loop {
                let (q, rem) = poly_div_linear(&poly, &y, p);
                if !fp2_is_zero(&rem) {
                    break;
                }
                mult += 1;
                poly = q;
                if poly.len() <= 1 {
                    break;
                }
            }
            out.push((y, mult));
        }
    }
    out
}

/// A vertex of the supersingular isogeny graph: a j-invariant and its 2-isogenous neighbours (as a
/// multiset — a double root of `Φ₂` is a double edge).
#[derive(Clone, Debug)]
pub struct SsVertex {
    pub j: Fp2,
    pub neighbors: Vec<Fp2>,
}

/// Build the **supersingular ℓ=2 isogeny graph** over `𝔽_{p²}` (requires `p ≡ 3 (mod 4)`). Breadth-first
/// from the supersingular vertex `j = 1728`, following the roots of `Φ₂(j,·)`. Because the graph is
/// connected, this reaches *every* supersingular j-invariant; its size is the supersingular mass
/// `⌊p/12⌋ + ε` and every vertex has out-degree 3. `None` if `p ≢ 3 (mod 4)`.
pub fn supersingular_graph(p: &BigInt) -> Option<Vec<SsVertex>> {
    if rem_pos(p, &ib(4)) != ib(3) {
        return None;
    }
    let start = fp2_const(1728, p);
    let mut visited: Vec<Fp2> = Vec::new();
    let mut queue: Vec<Fp2> = vec![start];
    let mut graph: Vec<SsVertex> = Vec::new();
    while let Some(j) = queue.pop() {
        if visited.iter().any(|v| v == &j) {
            continue;
        }
        visited.push(j.clone());
        let mut neighbors = Vec::new();
        for (root, mult) in cubic_roots(&phi2_in_y(&j, p), p) {
            for _ in 0..mult {
                neighbors.push(root.clone());
            }
            if !visited.iter().any(|v| v == &root) && !queue.iter().any(|v| v == &root) {
                queue.push(root);
            }
        }
        graph.push(SsVertex { j, neighbors });
    }
    Some(graph)
}

// ---- Elliptic curves over 𝔽_{p²} — SIKE scale ------------------------------------------------------
//
// Supersingular curves, and their FULL ℓ-torsion (`E[ℓ] ≅ (ℤ/ℓ)²`), are rational only over the quadratic
// extension: `#E(𝔽_{p²}) = (p ∓ 1)²`, so `E(𝔽_{p²}) ≅ (ℤ/(p∓1))²` carries the torsion SIKE's basis lives
// in. This is the concrete curve layer over 𝔽_{p²}: the same Weierstrass group law as over the prime
// field, with every coordinate an `Fp2`.

/// A Weierstrass curve `y² = x³ + ax + b` over `𝔽_{p²}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Curve2 {
    pub a: Fp2,
    pub b: Fp2,
    pub p: BigInt,
}

/// An affine point over `𝔽_{p²}`, or the identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Point2 {
    Infinity,
    Affine(Fp2, Fp2),
}

impl Curve2 {
    pub fn new(a: Fp2, b: Fp2, p: BigInt) -> Curve2 {
        Curve2 { a, b, p }
    }

    pub fn is_on_curve(&self, pt: &Point2) -> bool {
        match pt {
            Point2::Infinity => true,
            Point2::Affine(x, y) => {
                let lhs = fp2_mul(y, y, &self.p);
                let x3 = fp2_mul(&fp2_mul(x, x, &self.p), x, &self.p);
                let rhs = fp2_add(&fp2_add(&x3, &fp2_mul(&self.a, x, &self.p), &self.p), &self.b, &self.p);
                lhs == rhs
            }
        }
    }

    pub fn negate(&self, pt: &Point2) -> Point2 {
        match pt {
            Point2::Infinity => Point2::Infinity,
            Point2::Affine(x, y) => Point2::Affine(x.clone(), fp2_neg(y, &self.p)),
        }
    }

    pub fn add(&self, p1: &Point2, p2: &Point2) -> Point2 {
        match (p1, p2) {
            (Point2::Infinity, _) => p2.clone(),
            (_, Point2::Infinity) => p1.clone(),
            (Point2::Affine(x1, y1), Point2::Affine(x2, y2)) => {
                if x1 == x2 {
                    if fp2_is_zero(&fp2_add(y1, y2, &self.p)) {
                        return Point2::Infinity; // P = −Q
                    }
                    return self.double(p1); // P = Q
                }
                let den = fp2_inv(&fp2_sub(x2, x1, &self.p), &self.p).expect("distinct x invertible");
                let lam = fp2_mul(&fp2_sub(y2, y1, &self.p), &den, &self.p);
                let x3 = fp2_sub(&fp2_sub(&fp2_mul(&lam, &lam, &self.p), x1, &self.p), x2, &self.p);
                let y3 = fp2_sub(&fp2_mul(&lam, &fp2_sub(x1, &x3, &self.p), &self.p), y1, &self.p);
                Point2::Affine(x3, y3)
            }
        }
    }

    pub fn double(&self, pt: &Point2) -> Point2 {
        match pt {
            Point2::Infinity => Point2::Infinity,
            Point2::Affine(x, y) => {
                if fp2_is_zero(y) {
                    return Point2::Infinity;
                }
                let num = fp2_add(&fp2_mul(&fp2_const(3, &self.p), &fp2_mul(x, x, &self.p), &self.p), &self.a, &self.p);
                let den = fp2_inv(&fp2_mul(&fp2_const(2, &self.p), y, &self.p), &self.p).expect("2y invertible");
                let lam = fp2_mul(&num, &den, &self.p);
                let x3 = fp2_sub(&fp2_mul(&lam, &lam, &self.p), &fp2_mul(&fp2_const(2, &self.p), x, &self.p), &self.p);
                let y3 = fp2_sub(&fp2_mul(&lam, &fp2_sub(x, &x3, &self.p), &self.p), y, &self.p);
                Point2::Affine(x3, y3)
            }
        }
    }

    /// Scalar multiplication `k·P` by double-and-add (`k ≥ 0`).
    pub fn mul(&self, k: &BigInt, pt: &Point2) -> Point2 {
        let mut result = Point2::Infinity;
        let mut addend = pt.clone();
        let (_, bytes) = k.to_le_bytes();
        for byte in bytes {
            for bit in 0..8 {
                if (byte >> bit) & 1 == 1 {
                    result = self.add(&result, &addend);
                }
                addend = self.double(&addend);
            }
        }
        result
    }

    /// The j-invariant in `𝔽_{p²}` — the supersingular-graph vertex this curve sits at.
    pub fn j_invariant(&self) -> Option<Fp2> {
        let a3 = fp2_mul(&fp2_mul(&self.a, &self.a, &self.p), &self.a, &self.p);
        let four_a3 = fp2_mul(&fp2_const(4, &self.p), &a3, &self.p);
        let disc = fp2_add(&four_a3, &fp2_mul(&fp2_const(27, &self.p), &fp2_mul(&self.b, &self.b, &self.p), &self.p), &self.p);
        if fp2_is_zero(&disc) {
            return None;
        }
        Some(fp2_mul(&fp2_mul(&fp2_const(1728, &self.p), &four_a3, &self.p), &fp2_inv(&disc, &self.p)?, &self.p))
    }
}

// ---- Square roots, torsion, Vélu, and the Weil pairing OVER 𝔽_{p²} (SIKE scale) --------------------

/// `√a` in `𝔽_p` for `p ≡ 3 (mod 4)`: `a^{(p+1)/4}`; `None` for a non-residue.
fn fp_sqrt(a: &BigInt, p: &BigInt) -> Option<BigInt> {
    let ap = rem_pos(a, p);
    if ap.is_zero() {
        return Some(ib(0));
    }
    let exp = p.add(&ib(1)).div_rem(&ib(4))?.0;
    let r = crate::factor::modpow(&ap, &exp, p);
    (m(&r, &r, p) == ap).then_some(r)
}

/// `√α` in `𝔽_{p²}` (`p ≡ 3 mod 4`), or `None` if `α` is not a square. From `β² = α` with `β = x + yi`:
/// `x² − y² = a`, `2xy = b`, whence `x² = (a ± √(a²+b²))/2`; a final `β² = α` check makes it exact.
pub fn fp2_sqrt(alpha: &Fp2, p: &BigInt) -> Option<Fp2> {
    let (a, b) = (&alpha.0, &alpha.1);
    if b.is_zero() {
        if let Some(r) = fp_sqrt(a, p) {
            return Some((r, ib(0)));
        }
        return fp_sqrt(&s(&ib(0), a, p), p).map(|r| (ib(0), r)); // a non-residue ⟹ √a = √(−a)·i
    }
    let norm = a_(&m(a, a, p), &m(b, b, p), p);
    let sn = fp_sqrt(&norm, p)?;
    let inv2 = crate::factor::mod_inverse(&ib(2), p)?;
    for signed in [sn.clone(), s(&ib(0), &sn, p)] {
        let t = m(&a_(a, &signed, p), &inv2, p); // (a ± √N)/2
        if let Some(x) = fp_sqrt(&t, p) {
            if x.is_zero() {
                continue;
            }
            let y = m(b, &crate::factor::mod_inverse(&m(&ib(2), &x, p), p)?, p);
            let cand = (x, y);
            if fp2_mul(&cand, &cand, p) == *alpha {
                return Some(cand);
            }
        }
    }
    None
}

/// Every affine point of a curve over `𝔽_{p²}`, found by an `𝔽_{p²}` square root per x — `O(p²)`, not the
/// `O(p⁴)` of brute enumeration, so it reaches SIKE-shaped primes.
fn points_on_curve2(c: &Curve2) -> Vec<Point2> {
    let pu = c.p.to_i64().expect("small prime");
    let mut v = vec![Point2::Infinity];
    for xa in 0..pu {
        for xb in 0..pu {
            let x = (ib(xa), ib(xb));
            let x3 = fp2_mul(&fp2_mul(&x, &x, &c.p), &x, &c.p);
            let rhs = fp2_add(&fp2_add(&x3, &fp2_mul(&c.a, &x, &c.p), &c.p), &c.b, &c.p);
            if fp2_is_zero(&rhs) {
                v.push(Point2::Affine(x, fp2_const(0, &c.p)));
            } else if let Some(y) = fp2_sqrt(&rhs, &c.p) {
                v.push(Point2::Affine(x.clone(), fp2_neg(&y, &c.p)));
                v.push(Point2::Affine(x, y));
            }
        }
    }
    v
}

/// A point of prime order `ell` over `𝔽_{p²}` (a candidate isogeny kernel), or `None`.
pub fn point_of_order2(c: &Curve2, ell: u64) -> Option<Point2> {
    points_on_curve2(c).into_iter().find(|pt| *pt != Point2::Infinity && c.mul(&ib(ell as i64), pt) == Point2::Infinity)
}

/// A torsion basis of `E[n]` over `𝔽_{p²}` (`n` prime) — the full `n`-torsion is `𝔽_{p²}`-rational on a
/// supersingular curve when `n | p+1`. `None` if it is not fully rational.
pub fn torsion_basis2(c: &Curve2, n: u64) -> Option<(Point2, Point2)> {
    let onp: Vec<Point2> = points_on_curve2(c)
        .into_iter()
        .filter(|pt| *pt != Point2::Infinity && c.mul(&ib(n as i64), pt) == Point2::Infinity)
        .collect();
    for p in &onp {
        let span: Vec<Point2> = (0..n).map(|k| c.mul(&ib(k as i64), p)).collect();
        if let Some(q) = onp.iter().find(|q| !span.contains(q)) {
            return Some((p.clone(), q.clone()));
        }
    }
    None
}

/// A separable `ell`-isogeny over `𝔽_{p²}` by Vélu's formulas (the `Curve2` analogue of `elliptic::Isogeny`).
#[derive(Clone, Debug)]
pub struct Isogeny2 {
    pub domain: Curve2,
    pub codomain: Curve2,
    pub degree: u64,
    kernel: Vec<(Fp2, Fp2, Fp2)>, // (xQ, vQ, uQ)
}

impl Isogeny2 {
    pub fn from_kernel(curve: &Curve2, gen: &Point2, ell: u64) -> Option<Isogeny2> {
        if ell < 3 || ell % 2 == 0 {
            return None;
        }
        let p = &curve.p;
        let mut kernel = Vec::new();
        let mut cur = gen.clone();
        for _ in 0..((ell - 1) / 2) {
            match &cur {
                Point2::Affine(xq, yq) => {
                    let vq = fp2_add(
                        &fp2_mul(&fp2_const(6, p), &fp2_mul(xq, xq, p), p),
                        &fp2_mul(&fp2_const(2, p), &curve.a, p),
                        p,
                    );
                    let uq = fp2_mul(&fp2_const(4, p), &fp2_mul(yq, yq, p), p);
                    kernel.push((xq.clone(), vq, uq));
                }
                Point2::Infinity => return None,
            }
            cur = curve.add(&cur, gen);
        }
        if matches!(gen, Point2::Infinity) || curve.mul(&ib(ell as i64), gen) != Point2::Infinity {
            return None;
        }
        let (mut vsum, mut wsum) = (fp2_const(0, p), fp2_const(0, p));
        for (xq, vq, uq) in &kernel {
            vsum = fp2_add(&vsum, vq, p);
            wsum = fp2_add(&wsum, &fp2_add(uq, &fp2_mul(xq, vq, p), p), p);
        }
        let a2 = fp2_sub(&curve.a, &fp2_mul(&fp2_const(5, p), &vsum, p), p);
        let b2 = fp2_sub(&curve.b, &fp2_mul(&fp2_const(7, p), &wsum, p), p);
        Some(Isogeny2 { domain: curve.clone(), codomain: Curve2::new(a2, b2, p.clone()), degree: ell, kernel })
    }

    pub fn eval(&self, pt: &Point2) -> Point2 {
        let p = &self.domain.p;
        match pt {
            Point2::Infinity => Point2::Infinity,
            Point2::Affine(x, y) => {
                let mut xnew = x.clone();
                let mut yfac = fp2_const(1, p);
                for (xq, vq, uq) in &self.kernel {
                    let d = fp2_sub(x, xq, p);
                    if fp2_is_zero(&d) {
                        return Point2::Infinity;
                    }
                    let di = fp2_inv(&d, p).expect("nonzero");
                    let di2 = fp2_mul(&di, &di, p);
                    let di3 = fp2_mul(&di2, &di, p);
                    xnew = fp2_add(&xnew, &fp2_add(&fp2_mul(vq, &di, p), &fp2_mul(uq, &di2, p), p), p);
                    let tail = fp2_add(&fp2_mul(vq, &di2, p), &fp2_mul(&fp2_mul(&fp2_const(2, p), uq, p), &di3, p), p);
                    yfac = fp2_sub(&yfac, &tail, p);
                }
                Point2::Affine(xnew, fp2_mul(y, &yfac, p))
            }
        }
    }
}

fn mline_double2(c: &Curve2, t: &Point2, xq: &Fp2, yq: &Fp2) -> Option<(Fp2, Fp2)> {
    let p = &c.p;
    let (xt, yt) = match t {
        Point2::Affine(x, y) => (x, y),
        Point2::Infinity => return None,
    };
    if fp2_is_zero(yt) {
        return None;
    }
    let num = fp2_add(&fp2_mul(&fp2_const(3, p), &fp2_mul(xt, xt, p), p), &c.a, p);
    let lam = fp2_mul(&num, &fp2_inv(&fp2_mul(&fp2_const(2, p), yt, p), p)?, p);
    let ell = fp2_sub(&fp2_sub(yq, yt, p), &fp2_mul(&lam, &fp2_sub(xq, xt, p), p), p);
    let vert = match c.double(t) {
        Point2::Affine(x2, _) => fp2_sub(xq, &x2, p),
        Point2::Infinity => fp2_const(1, p),
    };
    Some((ell, vert))
}

fn mline_add2(c: &Curve2, t: &Point2, pp: &Point2, xq: &Fp2, yq: &Fp2) -> Option<(Fp2, Fp2)> {
    let p = &c.p;
    let ((xt, yt), (xpp, ypp)) = match (t, pp) {
        (Point2::Affine(a, b), Point2::Affine(cc, d)) => ((a, b), (cc, d)),
        _ => return None,
    };
    if xt == xpp {
        if yt == ypp {
            return mline_double2(c, t, xq, yq);
        }
        return Some((fp2_sub(xq, xt, p), fp2_const(1, p)));
    }
    let lam = fp2_mul(&fp2_sub(ypp, yt, p), &fp2_inv(&fp2_sub(xpp, xt, p), p)?, p);
    let ell = fp2_sub(&fp2_sub(yq, yt, p), &fp2_mul(&lam, &fp2_sub(xq, xt, p), p), p);
    let vert = match c.add(t, pp) {
        Point2::Affine(x3, _) => fp2_sub(xq, &x3, p),
        Point2::Infinity => fp2_const(1, p),
    };
    Some((ell, vert))
}

fn miller2(c: &Curve2, pp: &Point2, qq: &Point2, n: u64) -> Option<Fp2> {
    let p = &c.p;
    let (xq, yq) = match qq {
        Point2::Affine(x, y) => (x, y),
        Point2::Infinity => return None,
    };
    let (mut num, mut den) = (fp2_const(1, p), fp2_const(1, p));
    let mut t = pp.clone();
    for bit in (0..(64 - n.leading_zeros() - 1)).rev() {
        let (ell, vert) = mline_double2(c, &t, xq, yq)?;
        num = fp2_mul(&fp2_mul(&num, &num, p), &ell, p);
        den = fp2_mul(&fp2_mul(&den, &den, p), &vert, p);
        t = c.double(&t);
        if (n >> bit) & 1 == 1 {
            let (ell, vert) = mline_add2(c, &t, pp, xq, yq)?;
            num = fp2_mul(&num, &ell, p);
            den = fp2_mul(&den, &vert, p);
            t = c.add(&t, pp);
        }
    }
    if fp2_is_zero(&den) {
        return None;
    }
    Some(fp2_mul(&num, &fp2_inv(&den, p)?, p))
}

/// The **Weil pairing over `𝔽_{p²}`** — the `Curve2` analogue of `elliptic::weil_pairing`, via Miller.
pub fn weil_pairing2(c: &Curve2, pp: &Point2, qq: &Point2, n: u64) -> Option<Fp2> {
    if pp == qq || matches!(pp, Point2::Infinity) || matches!(qq, Point2::Infinity) {
        return Some(fp2_const(1, &c.p));
    }
    let fp = miller2(c, pp, qq, n)?;
    let fq = miller2(c, qq, pp, n)?;
    let ratio = fp2_mul(&fp, &fp2_inv(&fq, &c.p)?, &c.p);
    Some(if n % 2 == 1 { fp2_neg(&ratio, &c.p) } else { ratio })
}

/// The Weil pairing on a **product surface** `E₁ × E₂`: `e((P₁,P₂),(Q₁,Q₂)) = e_N(P₁,Q₁)·e_N(P₂,Q₂)`. A
/// subgroup on which this vanishes (`= 1`) is **isotropic**; a *maximal* isotropic (Lagrangian) subgroup is
/// the kernel of an `(N,N)`-isogeny of abelian surfaces — the gluing at the heart of Kani's theorem and the
/// Castryck–Decru attack.
pub fn product_weil_pairing(
    c1: &Curve2,
    c2: &Curve2,
    p: (&Point2, &Point2),
    q: (&Point2, &Point2),
    n: u64,
) -> Option<Fp2> {
    let e1 = weil_pairing2(c1, p.0, q.0, n)?;
    let e2 = weil_pairing2(c2, p.1, q.1, n)?;
    Some(fp2_mul(&e1, &e2, &c1.p))
}

/// `α^k` in `𝔽_{p²}` by square-and-multiply.
pub fn fp2_pow(alpha: &Fp2, k: u64, p: &BigInt) -> Fp2 {
    let mut result = fp2_const(1, p);
    let mut base = alpha.clone();
    let mut e = k;
    while e > 0 {
        if e & 1 == 1 {
            result = fp2_mul(&result, &base, p);
        }
        base = fp2_mul(&base, &base, p);
        e >>= 1;
    }
    result
}

// ===== SIDH keyspace over 𝔽_{p²}: unfolding, images → generator recovery, the ℓ-adic tree, Aut-orbits =====

/// The order of a point over `𝔽_{p²}` (least `k ≥ 1` with `k·pt = O`), searched up to `bound`.
pub fn point_order2(c: &Curve2, pt: &Point2, bound: u64) -> Option<u64> {
    let mut cur = pt.clone();
    for k in 1..=bound {
        if cur == Point2::Infinity {
            return Some(k);
        }
        cur = c.add(&cur, pt);
    }
    None
}

/// The SIDH kernel generator `P + [s]·Q` over `𝔽_{p²}`.
pub fn kernel_generator2(c: &Curve2, p_pt: &Point2, q_pt: &Point2, s: &BigInt) -> Point2 {
    c.add(p_pt, &c.mul(s, q_pt))
}

/// A rank-2 basis `(P, Q)` of `E[ℓᵃ] = (ℤ/ℓᵃ)²` — both of order exactly `ℓᵃ`, independent. The full
/// `ℓᵃ`-torsion is `𝔽_{p²}`-rational on a supersingular curve when `ℓᵃ | p+1`.
pub fn full_order_basis2(c: &Curve2, ell: u64, a: u32) -> Option<(Point2, Point2)> {
    let n = ell.pow(a);
    let full: Vec<Point2> = points_on_curve2(c)
        .into_iter()
        .filter(|pt| point_order2(c, pt, n + 1) == Some(n))
        .collect();
    for p in &full {
        let span: Vec<Point2> = (0..n).map(|k| c.mul(&ib(k as i64), p)).collect();
        if let Some(q) = full.iter().find(|q| !span.contains(q)) {
            return Some((p.clone(), q.clone()));
        }
    }
    None
}

/// A single `ℓ`-isogeny step over `𝔽_{p²}`: domain, the order-`ℓ` kernel, and codomain.
#[derive(Clone, Debug, PartialEq)]
pub struct IsogenyStep2 {
    pub domain: Curve2,
    pub kernel: Point2,
    pub codomain: Curve2,
}

/// Unfold one order-`ℓᵃ` kernel generator into the chain of `a` prime-degree `ℓ`-isogenies over `𝔽_{p²}` —
/// the `Curve2` analogue of [`crate::elliptic::derive_isogeny_path`]. `ℓ` an odd prime.
pub fn derive_isogeny_path2(c: &Curve2, gen: &Point2, ell: u64, a: u32) -> Option<Vec<IsogenyStep2>> {
    let mut steps = Vec::with_capacity(a as usize);
    let mut e = c.clone();
    let mut g = gen.clone();
    for step in 0..a {
        let mut mult = ib(1);
        for _ in 0..(a - 1 - step) {
            mult = mult.mul(&ib(ell as i64));
        }
        let k = e.mul(&mult, &g);
        let iso = Isogeny2::from_kernel(&e, &k, ell)?;
        steps.push(IsogenyStep2 { domain: e.clone(), kernel: k, codomain: iso.codomain.clone() });
        g = iso.eval(&g);
        e = iso.codomain;
    }
    Some(steps)
}

/// Push a point through an entire isogeny chain, rebuilding each step's Vélu map.
pub fn push_through2(steps: &[IsogenyStep2], ell: u64, pt: &Point2) -> Option<Point2> {
    let mut cur = pt.clone();
    for step in steps {
        cur = Isogeny2::from_kernel(&step.domain, &step.kernel, ell)?.eval(&cur);
    }
    Some(cur)
}

/// **Images → generator reconstruction (flat oracle).** The secret isogeny `φ: E₀ → E` of degree `ℓᵃ` has
/// kernel `⟨P_A + [s]Q_A⟩`; the published torsion images `φ(P_B), φ(Q_B)` pin `s` down. Enumerate the whole
/// keyspace and return the scalar whose unfolded chain reproduces both images — a genuine images → generator
/// inversion at SIDH scale. `O(ℓᵃ)`; the correctness oracle for the structured recovery below.
pub fn recover_secret2(
    e0: &Curve2,
    basis_a: (&Point2, &Point2),
    ell: u64,
    a: u32,
    basis_b: (&Point2, &Point2),
    images: (&Point2, &Point2),
) -> Option<(BigInt, Point2, Vec<IsogenyStep2>)> {
    let n = ell.pow(a);
    for s in 0..n {
        let sb = ib(s as i64);
        let gen = kernel_generator2(e0, basis_a.0, basis_a.1, &sb);
        if point_order2(e0, &gen, n + 1) != Some(n) {
            continue;
        }
        let Some(path) = derive_isogeny_path2(e0, &gen, ell, a) else { continue };
        match (push_through2(&path, ell, basis_b.0), push_through2(&path, ell, basis_b.1)) {
            (Some(ip), Some(iq)) if &ip == images.0 && &iq == images.1 => return Some((sb, gen, path)),
            _ => {}
        }
    }
    None
}

/// The digit-tree walk: `s = Σ dₖ·ℓᵏ`, so the secret's `ℓ`-adic digits `d₀,…,d_{a-1}` index a depth-`a`
/// `ℓ`-ary tree whose depth-`k` node fixes `s mod ℓᵏ`. All secrets under one node share their first `k`
/// isogeny steps — the auto-partition. DFS over the digits, testing each leaf's chain against the images.
#[allow(clippy::too_many_arguments)]
fn walk_keyspace_tree(
    e0: &Curve2,
    ba: (&Point2, &Point2),
    ell: u64,
    a: u32,
    n: u64,
    bb: (&Point2, &Point2),
    images: (&Point2, &Point2),
    digits: &mut Vec<u64>,
) -> Option<(BigInt, Point2)> {
    if digits.len() as u32 == a {
        let (mut s, mut place) = (ib(0), ib(1));
        for &d in digits.iter() {
            s = s.add(&place.mul(&ib(d as i64)));
            place = place.mul(&ib(ell as i64));
        }
        let gen = kernel_generator2(e0, ba.0, ba.1, &s);
        if point_order2(e0, &gen, n + 1) != Some(n) {
            return None;
        }
        let path = derive_isogeny_path2(e0, &gen, ell, a)?;
        let (ip, iq) = (push_through2(&path, ell, bb.0)?, push_through2(&path, ell, bb.1)?);
        return (&ip == images.0 && &iq == images.1).then_some((s, gen));
    }
    for d in 0..ell {
        digits.push(d);
        if let Some(found) = walk_keyspace_tree(e0, ba, ell, a, n, bb, images, digits) {
            return Some(found);
        }
        digits.pop();
    }
    None
}

/// **Structured images → generator recovery.** Walks the `ℓ`-adic keyspace *tree* (auto-partitioning by
/// digit) rather than the flat list, returning the recovered `ℓ`-adic digits, the secret `s = Σ dₖ·ℓᵏ`, and
/// its generator. Same answer as [`recover_secret2`] — but the recursion **is** the digit-by-digit structure
/// the Castryck–Decru Kani oracle prunes with a per-digit split-test.
pub fn recover_secret_recursive2(
    e0: &Curve2,
    basis_a: (&Point2, &Point2),
    ell: u64,
    a: u32,
    basis_b: (&Point2, &Point2),
    images: (&Point2, &Point2),
) -> Option<(Vec<u64>, BigInt, Point2)> {
    let n = ell.pow(a);
    let mut digits: Vec<u64> = Vec::with_capacity(a as usize);
    let (s, gen) = walk_keyspace_tree(e0, basis_a, ell, a, n, basis_b, images, &mut digits)?;
    Some((digits, s, gen))
}

/// The order-4 automorphism `ι:(x,y) ↦ (−x, i·y)` of the `j = 1728` curve `y² = x³ + x` (`i² = −1` lives in
/// `𝔽_{p²}` for `p ≡ 3 mod 4`). `ι² = [−1]`, so `⟨ι⟩ ≅ ℤ/4 = Aut(E₀)`. It permutes the keyspace, and
/// `E₀/⟨K⟩ ≅ E₀/⟨ι(K)⟩` — the symmetry that makes recovering one kernel recover its whole orbit.
pub fn aut_1728(p: &BigInt, pt: &Point2) -> Point2 {
    match pt {
        Point2::Infinity => Point2::Infinity,
        Point2::Affine(x, y) => {
            let i_unit = (ib(0), ib(1)); // i ∈ 𝔽_{p²}
            Point2::Affine(fp2_neg(x, p), fp2_mul(&i_unit, y, p))
        }
    }
}

/// Partition the keyspace by **codomain `j`-invariant**: secrets whose isogeny lands on the same curve (up to
/// isomorphism). These classes are exactly the `Aut(E₀)`-orbits — so if one secret in a class is invertible
/// (its codomain is the target `E`), every secret in the class maps to a curve `≅ E`. Returns
/// `(j, [secrets])` groups.
pub fn keyspace_codomain_classes(
    e0: &Curve2,
    basis_a: (&Point2, &Point2),
    ell: u64,
    a: u32,
) -> Vec<(Fp2, Vec<u64>)> {
    let n = ell.pow(a);
    let mut classes: Vec<(Fp2, Vec<u64>)> = Vec::new();
    for s in 0..n {
        let gen = kernel_generator2(e0, basis_a.0, basis_a.1, &ib(s as i64));
        if point_order2(e0, &gen, n + 1) != Some(n) {
            continue;
        }
        let Some(path) = derive_isogeny_path2(e0, &gen, ell, a) else { continue };
        let Some(j) = path.last().and_then(|st| st.codomain.j_invariant()) else { continue };
        match classes.iter().position(|(cj, _)| *cj == j) {
            Some(idx) => classes[idx].1.push(s),
            None => classes.push((j, vec![s])),
        }
    }
    classes
}

/// The `ℓ+1` `ℓ`-isogenous neighbours of a curve: the codomains of the `ℓ+1` order-`ℓ` subgroups of `E[ℓ]`.
fn ell_isogeny_neighbors(c: &Curve2, ell: u64) -> Vec<Curve2> {
    let Some((r, s)) = full_order_basis2(c, ell, 1) else { return Vec::new() };
    let mut gens: Vec<Point2> = (0..ell).map(|i| c.add(&r, &c.mul(&ib(i as i64), &s))).collect();
    gens.push(s);
    gens.into_iter().filter_map(|g| Isogeny2::from_kernel(c, &g, ell).map(|iso| iso.codomain)).collect()
}

/// Backward BFS on the supersingular `ℓ`-isogeny graph from `target`: the minimum `ℓ`-isogeny distance to each
/// reachable `j`-invariant, out to `max_depth`. This is a **sound per-digit oracle** — a partial curve farther
/// from the target than its remaining step budget cannot lie on the secret path, so its whole subtree is dead.
pub fn isogeny_graph_distances(target: &Curve2, ell: u64, max_depth: u32) -> Vec<(Fp2, u32)> {
    let Some(j0) = target.j_invariant() else { return Vec::new() };
    let mut dist: Vec<(Fp2, u32)> = vec![(j0, 0)];
    let mut frontier = vec![target.clone()];
    for d in 1..=max_depth {
        let mut next = Vec::new();
        for c in &frontier {
            for nbr in ell_isogeny_neighbors(c, ell) {
                if let Some(j) = nbr.j_invariant() {
                    if !dist.iter().any(|(jj, _)| *jj == j) {
                        dist.push((j, d));
                        next.push(nbr);
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    dist
}

/// Oracle-pruned digit-tree walk: at each depth-`k` node the partial curve `E_k` (the codomain after `k`
/// steps of the prefix) is checked against the reachability oracle `dists`; if it is not within `a−k` steps of
/// the target, the subtree is pruned. Sound (the true prefix always satisfies the budget), so the answer is
/// unchanged — but leaf image-tests are confined to curves that can still reach `E`, which is exactly the
/// target's `Aut`-orbit. `leaves` counts the image-tests actually performed.
#[allow(clippy::too_many_arguments)]
fn walk_pruned(
    e0: &Curve2,
    ba: (&Point2, &Point2),
    ell: u64,
    a: u32,
    n: u64,
    bb: (&Point2, &Point2),
    images: (&Point2, &Point2),
    dists: &[(Fp2, u32)],
    digits: &mut Vec<u64>,
    leaves: &mut usize,
) -> Option<(BigInt, Point2)> {
    let k = digits.len() as u32;
    let (mut p, mut place) = (ib(0), ib(1));
    for &d in digits.iter() {
        p = p.add(&place.mul(&ib(d as i64)));
        place = place.mul(&ib(ell as i64));
    }
    if k > 0 {
        let gen_p = kernel_generator2(e0, ba.0, ba.1, &p);
        let path = derive_isogeny_path2(e0, &gen_p, ell, a)?;
        let jk = path[(k - 1) as usize].codomain.j_invariant()?;
        match dists.iter().find(|(jj, _)| *jj == jk).map(|(_, d)| *d) {
            Some(dk) if dk <= a - k => {}    // still reachable within budget — keep going
            _ => return None,                // too far (or off-graph) ⟹ prune the whole subtree
        }
    }
    if k == a {
        *leaves += 1;
        let gen = kernel_generator2(e0, ba.0, ba.1, &p);
        if point_order2(e0, &gen, n + 1) != Some(n) {
            return None;
        }
        let path = derive_isogeny_path2(e0, &gen, ell, a)?;
        let (ip, iq) = (push_through2(&path, ell, bb.0)?, push_through2(&path, ell, bb.1)?);
        return (&ip == images.0 && &iq == images.1).then_some((p, gen));
    }
    for d in 0..ell {
        digits.push(d);
        if let Some(found) = walk_pruned(e0, ba, ell, a, n, bb, images, dists, digits, leaves) {
            return Some(found);
        }
        digits.pop();
    }
    None
}

/// **Oracle-pruned images → generator recovery.** Walks the `ℓ`-adic keyspace tree but consults the isogeny
/// graph reachability oracle (backward BFS from the *public* codomain `target`) at every node, pruning any
/// branch that can no longer reach the target. Returns the secret, its generator, and the number of leaf
/// image-tests performed (the pruned search visits only the target's `Aut`-orbit). Same answer as
/// [`recover_secret2`] — soundness guarantees the true path is never pruned.
pub fn recover_secret_pruned2(
    e0: &Curve2,
    basis_a: (&Point2, &Point2),
    ell: u64,
    a: u32,
    basis_b: (&Point2, &Point2),
    images: (&Point2, &Point2),
    target: &Curve2,
) -> Option<(BigInt, Point2, usize)> {
    let n = ell.pow(a);
    let dists = isogeny_graph_distances(target, ell, a);
    let mut digits: Vec<u64> = Vec::with_capacity(a as usize);
    let mut leaves = 0usize;
    let (s, gen) = walk_pruned(e0, basis_a, ell, a, n, basis_b, images, &dists, &mut digits, &mut leaves)?;
    Some((s, gen, leaves))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(s: &str) -> BigInt {
        BigInt::parse_decimal(s).unwrap()
    }

    #[test]
    fn fp2_is_a_field() {
        let p = big("103"); // 103 ≡ 3 (mod 4)
        let one = fp2_const(1, &p);
        let ii = (ib(0), ib(1)); // the element i
        // i² = −1.
        assert_eq!(fp2_mul(&ii, &ii, &p), fp2_neg(&one, &p), "i² = −1");
        // Commutativity, distributivity, and inverses over a sweep.
        let elts: Vec<Fp2> = (0..7)
            .flat_map(|a| (0..7).map(move |b| (ib(a * 13 + 1), ib(b * 11 + 2))))
            .collect();
        for x in &elts {
            for y in &elts {
                assert_eq!(fp2_mul(x, y, &p), fp2_mul(y, x, &p), "× commutes");
                assert_eq!(fp2_add(x, y, &p), fp2_add(y, x, &p), "+ commutes");
                // x·(y+1) = x·y + x
                let lhs = fp2_mul(x, &fp2_add(y, &one, &p), &p);
                let rhs = fp2_add(&fp2_mul(x, y, &p), x, &p);
                assert_eq!(lhs, rhs, "distributive");
            }
            if !fp2_is_zero(x) {
                assert_eq!(fp2_mul(x, &fp2_inv(x, &p).unwrap(), &p), one, "x·x⁻¹ = 1");
            }
        }
    }

    // The supersingular mass: #supersingular j-invariants = ⌊p/12⌋ + ε, ε from p mod 12.
    fn ss_mass(p: u64) -> u64 {
        p / 12
            + match p % 12 {
                1 => 0,
                5 | 7 => 1,
                11 => 2,
                _ => 0,
            }
    }

    #[test]
    fn supersingular_graph_has_the_right_mass_is_3_regular_and_connected() {
        for &pu in &[11u64, 19, 23, 31, 43, 47] {
            let p = big(&pu.to_string());
            let g = supersingular_graph(&p).expect("p ≡ 3 (mod 4)");
            // Mass: BFS reached every supersingular j-invariant (the graph is connected, so this proves
            // connectivity too — a disconnected graph would under-count).
            assert_eq!(g.len() as u64, ss_mass(pu), "supersingular mass for p = {pu}");
            // 3-regular: Φ₂ is a cubic and splits completely over 𝔽_{p²} at every supersingular vertex.
            for v in &g {
                assert_eq!(v.neighbors.len(), 3, "each vertex has out-degree 3 (p = {pu})");
            }
            // The graph is undirected/consistent: every neighbour is itself a vertex in the graph.
            let js: Vec<&Fp2> = g.iter().map(|v| &v.j).collect();
            for v in &g {
                for nb in &v.neighbors {
                    assert!(js.iter().any(|j| *j == nb), "neighbour is a supersingular vertex");
                }
            }
            // j = 1728 is in the graph (our supersingular starting point).
            assert!(js.iter().any(|j| **j == fp2_const(1728, &p)), "1728 is supersingular for p = {pu}");
        }
    }

    fn embed(pt: &crate::elliptic::Point) -> Point2 {
        match pt {
            crate::elliptic::Point::Infinity => Point2::Infinity,
            crate::elliptic::Point::Affine(x, y) => Point2::Affine((x.clone(), ib(0)), (y.clone(), ib(0))),
        }
    }

    #[test]
    fn curve2_agrees_with_the_prime_field_on_rational_points() {
        // The 𝔽_{p²} group law, restricted to 𝔽_p-rational points, must reproduce the prime-field layer.
        let p = big("43");
        let cfp = crate::elliptic::Curve::new(ib(2), ib(25), p.clone());
        let base = crate::elliptic::point_of_order(&cfp, 5).expect("a rational 5-torsion point");
        let c2 = Curve2::new(fp2_const(2, &p), fp2_const(25, &p), p.clone());
        let base2 = embed(&base);
        assert!(c2.is_on_curve(&base2), "the embedded point lies on the 𝔽_{{p²}} curve");
        for k in 1..=5i64 {
            let via_fp = embed(&cfp.mul(&ib(k), &base));
            let via_fp2 = c2.mul(&ib(k), &base2);
            assert_eq!(via_fp2, via_fp, "Curve2 matches Curve on {k}·P");
        }
    }

    // Every affine point of a curve over 𝔽_{p²} (brute force; small p only).
    fn all_points2(c: &Curve2) -> Vec<Point2> {
        let pu = c.p.to_i64().unwrap();
        let mut v = vec![Point2::Infinity];
        for xa in 0..pu {
            for xb in 0..pu {
                let x = (ib(xa), ib(xb));
                for ya in 0..pu {
                    for yb in 0..pu {
                        let pt = Point2::Affine(x.clone(), (ib(ya), ib(yb)));
                        if c.is_on_curve(&pt) {
                            v.push(pt);
                        }
                    }
                }
            }
        }
        v
    }

    #[test]
    fn supersingular_curve_over_fp2_carries_full_torsion() {
        // y² = x³ + x is supersingular for p ≡ 3 (mod 4); over 𝔽_{p²} it has #E = (p+1)² and E ≅ (ℤ/(p+1))²,
        // so its full ℓ-torsion (ℓ | p+1) is 𝔽_{p²}-rational — the property SIKE's torsion basis needs.
        let p = big("11");
        let c = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
        assert_eq!(c.j_invariant().unwrap(), fp2_const(1728, &p), "j(y²=x³+x) = 1728, a supersingular vertex");
        let pts = all_points2(&c);
        assert_eq!(pts.len(), 144, "#E(𝔽_{{p²}}) = (p+1)² = 12²");
        // Group law is a real abelian group.
        for a in pts.iter().take(6) {
            assert_eq!(c.add(a, &c.negate(a)), Point2::Infinity, "P + (−P) = O");
            for b in pts.iter().take(6) {
                for d in pts.iter().take(6) {
                    assert_eq!(c.add(&c.add(a, b), d), c.add(a, &c.add(b, d)), "associative");
                }
            }
        }
        // FULL 3-torsion is 𝔽_{p²}-rational: E[3] ≅ (ℤ/3)² has exactly 9 points (3 | p+1 = 12).
        let e3 = pts.iter().filter(|pt| c.mul(&ib(3), pt) == Point2::Infinity).count();
        assert_eq!(e3, 9, "E[3] ≅ (ℤ/3)² is fully rational over 𝔽_{{p²}}");
    }

    #[test]
    fn fp2_sqrt_round_trips() {
        let p = big("59");
        for (a, b) in [(3i64, 7), (1, 0), (0, 5), (11, 2), (23, 41)] {
            let x = (ib(a), ib(b));
            let sq = fp2_mul(&x, &x, &p);
            let r = fp2_sqrt(&sq, &p).expect("a square has a root");
            assert_eq!(fp2_mul(&r, &r, &p), sq, "(√α)² = α");
        }
    }

    #[test]
    fn torsion_image_certificate_lifts_to_fp2_at_sike_scale() {
        // p = 59 ≡ 3 (mod 4), p+1 = 60 = 2²·3·5 — a SIKE-shaped prime: E[3] and E[5] are fully
        // 𝔽_{p²}-rational on the supersingular curve y² = x³ + x. Build a 3-isogeny, take a 5-torsion basis,
        // and verify the torsion-image compatibility law e_5(φP,φQ) = e_5(P,Q)^{deg φ} OVER 𝔽_{p²}.
        let p = big("59");
        let c = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
        let kernel = point_of_order2(&c, 3).expect("3-torsion over 𝔽_{p²}");
        let iso = Isogeny2::from_kernel(&c, &kernel, 3).expect("Vélu over 𝔽_{p²}");
        let (pp, qq) = torsion_basis2(&c, 5).expect("full 5-torsion basis over 𝔽_{p²}");
        let ep = weil_pairing2(&c, &pp, &qq, 5).expect("basis pairing");
        assert_ne!(ep, fp2_const(1, &p), "independent basis ⟹ a nontrivial (primitive 5th root) pairing");
        let (fp_, fq_) = (iso.eval(&pp), iso.eval(&qq));
        assert!(iso.codomain.is_on_curve(&fp_) && iso.codomain.is_on_curve(&fq_), "images land on the codomain");
        let eq = weil_pairing2(&iso.codomain, &fp_, &fq_, 5).expect("image pairing");
        assert_eq!(eq, fp2_pow(&ep, iso.degree, &p), "e_5(φP,φQ) = e_5(P,Q)^{{deg φ}} over 𝔽_{{p²}}");
    }

    // A SIDH-scale supersingular curve over 𝔽_{107²}: y²=x³+x (j=1728), #E=(p+1)²=108², so E[3³] and E[2²]
    // are both rank-2 rational — a genuine 27-kernel keyspace with the order-4 automorphism ι acting on it.
    fn sidh_scale_setup() -> (BigInt, Curve2, (Point2, Point2), (Point2, Point2)) {
        let p = big("107");
        let e0 = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
        let basis_a = full_order_basis2(&e0, 3, 3).expect("rank-2 E[3³] basis");
        let basis_b = full_order_basis2(&e0, 2, 2).expect("rank-2 E[2²] basis");
        (p, e0, basis_a, basis_b)
    }

    #[test]
    fn images_reconstruct_the_secret_generator_at_sidh_scale() {
        let (_p, e0, (pa, qa), (pb, qb)) = sidh_scale_setup();
        // Secret scalar → kernel generator → 3³-isogeny → published torsion images.
        let secret = ib(7);
        let gen = kernel_generator2(&e0, &pa, &qa, &secret);
        assert_eq!(point_order2(&e0, &gen, 28), Some(27), "the secret kernel generates order 3³");
        let path = derive_isogeny_path2(&e0, &gen, 3, 3).expect("the secret 3³-isogeny");
        let images =
            (push_through2(&path, 3, &pb).expect("push P_B"), push_through2(&path, 3, &qb).expect("push Q_B"));
        // Invert: recover a generator from ONLY (E₀, the two bases, the images).
        let (rs, rgen, rpath) =
            recover_secret2(&e0, (&pa, &qa), 3, 3, (&pb, &qb), (&images.0, &images.1)).expect("recovery");
        // The recovered isogeny reproduces the published images and lands on the same codomain — a genuine
        // images → generator inversion. (The scalar may be symmetry-equivalent to the planted one, not
        // identical: that is exactly the Aut-orbit structure exercised below.)
        assert_eq!(push_through2(&rpath, 3, &pb).unwrap(), images.0, "recovered chain reproduces φ(P_B)");
        assert_eq!(push_through2(&rpath, 3, &qb).unwrap(), images.1, "recovered chain reproduces φ(Q_B)");
        assert_eq!(rpath.last().unwrap().codomain, path.last().unwrap().codomain, "and lands on the target E");
        assert_eq!(kernel_generator2(&e0, &pa, &qa, &rs), rgen, "the reported scalar yields the reported gen");
        let _ = gen; // the planted generator is one valid preimage among its orbit
    }

    #[test]
    fn the_structured_tree_recovery_matches_the_flat_oracle() {
        let (_p, e0, (pa, qa), (pb, qb)) = sidh_scale_setup();
        let secret = ib(11);
        let gen = kernel_generator2(&e0, &pa, &qa, &secret);
        let path = derive_isogeny_path2(&e0, &gen, 3, 3).unwrap();
        let images = (push_through2(&path, 3, &pb).unwrap(), push_through2(&path, 3, &qb).unwrap());
        // The ℓ-adic tree recursion recovers a generator digit by digit, walking the keyspace partition.
        let (digits, rs, rgen) =
            recover_secret_recursive2(&e0, (&pa, &qa), 3, 3, (&pb, &qb), (&images.0, &images.1)).expect("tree");
        assert_eq!(digits.len(), 3, "one ℓ-adic digit per isogeny step");
        assert!(digits.iter().all(|&d| d < 3), "each digit lies in 0..ℓ");
        // The reported scalar is exactly the digit reconstruction s = Σ dₖ·ℓᵏ.
        let s_check: i64 = digits.iter().enumerate().map(|(k, &d)| d as i64 * 3i64.pow(k as u32)).sum();
        assert_eq!(rs, ib(s_check), "the reported scalar is the ℓ-adic digit reconstruction");
        // And it reproduces the images on the same codomain — the tree walk inverts, like the flat oracle.
        let rpath = derive_isogeny_path2(&e0, &rgen, 3, 3).unwrap();
        assert_eq!(push_through2(&rpath, 3, &pb).unwrap(), images.0, "the recovered generator reproduces φ(P_B)");
        assert_eq!(push_through2(&rpath, 3, &qb).unwrap(), images.1, "and φ(Q_B)");
        assert_eq!(rpath.last().unwrap().codomain, path.last().unwrap().codomain, "and lands on the target E");
        let _ = secret; // planted secret is a valid solution; the tree may return a symmetry-equivalent one
    }

    #[test]
    fn the_keyspace_partitions_into_aut_orbits_so_one_recovery_yields_the_orbit() {
        let (p, e0, (pa, qa), _b) = sidh_scale_setup();
        // Partition the 27-kernel keyspace by codomain j-invariant.
        let classes = keyspace_codomain_classes(&e0, (&pa, &qa), 3, 3);
        let total: usize = classes.iter().map(|(_, ss)| ss.len()).sum();
        assert_eq!(total, 27, "the classes partition the whole keyspace");
        assert!(classes.len() < 27, "the keyspace COLLAPSES — kernels share codomains ⟹ nontrivial symmetry");
        // The rule, verified across the entire keyspace: E₀/⟨K⟩ ≅ E₀/⟨ι(K)⟩. So if ONE kernel is invertible
        // (its codomain is the target E), its whole ι-orbit maps to curves ≅ E — recovering one recovers the
        // orbit. This is symmetry forcing the answer out, the same lever as our SAT clause-orbit quotients.
        for s in 0..27u64 {
            let gen = kernel_generator2(&e0, &pa, &qa, &ib(s as i64));
            let j_gen = derive_isogeny_path2(&e0, &gen, 3, 3).unwrap().last().unwrap().codomain.j_invariant();
            let igen = aut_1728(&p, &gen);
            assert!(e0.is_on_curve(&igen), "ι(K) lands back on E₀");
            assert_eq!(point_order2(&e0, &igen, 28), Some(27), "ι preserves the order-27 kernel");
            let j_igen = derive_isogeny_path2(&e0, &igen, 3, 3).unwrap().last().unwrap().codomain.j_invariant();
            assert_eq!(j_gen, j_igen, "ι(K) shares K's codomain j — the orbit shares its target curve");
        }
    }

    #[test]
    fn the_graph_reachability_oracle_prunes_the_tree_to_the_aut_orbit() {
        let (_p, e0, (pa, qa), (pb, qb)) = sidh_scale_setup();
        let secret = ib(11);
        let gen = kernel_generator2(&e0, &pa, &qa, &secret);
        let path = derive_isogeny_path2(&e0, &gen, 3, 3).unwrap();
        let target = path.last().unwrap().codomain.clone(); // the PUBLIC codomain E
        let images = (push_through2(&path, 3, &pb).unwrap(), push_through2(&path, 3, &qb).unwrap());

        // Backward BFS maps the reachable neighbourhood of E in the supersingular ℓ-isogeny graph.
        let dists = isogeny_graph_distances(&target, 3, 3);
        assert!(dists.len() > 1, "the ℓ-isogeny graph around E has genuine structure (a Ramanujan expander)");
        assert!(dists.iter().any(|(_, d)| *d == 0), "E sits at distance 0 from itself");

        // Oracle-pruned recovery still inverts (sound: the true path is never pruned) — reproduces the images
        // and lands on E.
        let (_rs, rgen, leaves) =
            recover_secret_pruned2(&e0, (&pa, &qa), 3, 3, (&pb, &qb), (&images.0, &images.1), &target)
                .expect("pruned recovery");
        let rpath = derive_isogeny_path2(&e0, &rgen, 3, 3).unwrap();
        assert_eq!(push_through2(&rpath, 3, &pb).unwrap(), images.0, "pruned recovery reproduces φ(P_B)");
        assert_eq!(push_through2(&rpath, 3, &qb).unwrap(), images.1, "and φ(Q_B)");
        assert_eq!(rpath.last().unwrap().codomain, target, "and lands on the target E");

        // The oracle confines leaf image-tests to the target's Aut-orbit: the reachability prune and the
        // symmetry collapse are the SAME cut. Only orbit members (same codomain j as E) can survive to a leaf.
        let classes = keyspace_codomain_classes(&e0, (&pa, &qa), 3, 3);
        let tj = target.j_invariant().unwrap();
        let orbit_size = classes.iter().find(|(j, _)| *j == tj).map(|(_, ss)| ss.len()).unwrap();
        assert!(orbit_size < 27, "the keyspace collapses to a strictly smaller orbit");
        assert!(leaves <= orbit_size, "the graph oracle leaf-tests only the target's Aut-orbit, not all 27");
    }
}
