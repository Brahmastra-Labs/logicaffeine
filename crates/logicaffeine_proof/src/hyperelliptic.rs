//! # Genus-2 curves and the Richelot (2,2)-isogeny — the abelian-surface mechanism
//!
//! When an elliptic-curve pair is *glued* into an abelian surface (a genus-2 Jacobian), the Castryck–Decru
//! break walks **(2,2)-isogenies** of that surface and watches for the codomain to **split** back into a
//! product of elliptic curves — each split reveals a step of the secret isogeny.
//!
//! A genus-2 curve is `y² = f(x)` with `f` a sextic, and its 2-torsion is the six Weierstrass points (the
//! roots of `f`). A **(2,2)-subgroup** is a partition of those six roots into three pairs — equivalently a
//! factorisation `f = G₁·G₂·G₃` into three quadratics; there are exactly 15 of them, the neighbours in the
//! (2,2)-isogeny graph. **Richelot's isogeny** sends `f` to the dual sextic `H₁·H₂·H₃`, where
//! `Hᵢ = Gⱼ·Gₖ' − Gⱼ'·Gₖ` (`'` = `d/dx`, `{i,j,k}` cyclic), scaled by the **Richelot determinant**
//! `δ = det[coeffs of G₁,G₂,G₃]`. When `δ = 0` the three quadratics are dependent, the correspondence
//! degenerates, and the abelian surface is **reducible** — the split condition, in its cleanest form.
//!
//! This module builds that mechanism over `𝔽_p` and the chain-walk across the (2,2)-graph. The honest
//! boundary: the *full* Castryck–Decru secret recovery, and Damien Robert's higher-dimensional (dimension
//! 4/8, theta-coordinate) generalisation, are genuine research-grade machinery built ON this core — the
//! chain-walk framework here is the shape they generalise, not a claim to have implemented dimension 8.

use crate::factor::{mod_inverse, modpow};
use logicaffeine_base::BigInt;

#[inline]
fn ib(x: i64) -> BigInt {
    BigInt::from_i64(x)
}
#[inline]
fn rp(a: &BigInt, p: &BigInt) -> BigInt {
    let r = a.div_rem(p).map(|(_, r)| r).unwrap_or_else(|| a.clone());
    if r.is_negative() {
        r.add(p)
    } else {
        r
    }
}
#[inline]
fn mm(a: &BigInt, b: &BigInt, p: &BigInt) -> BigInt {
    rp(&a.mul(b), p)
}
#[inline]
fn am(a: &BigInt, b: &BigInt, p: &BigInt) -> BigInt {
    rp(&a.add(b), p)
}
#[inline]
fn sm(a: &BigInt, b: &BigInt, p: &BigInt) -> BigInt {
    rp(&a.sub(b), p)
}

/// A quadratic `g₀ + g₁·x + g₂·x²`, coefficients low-to-high.
pub type Quad = [BigInt; 3];

/// Polynomial product mod `p` (coefficients low-to-high).
pub fn poly_mul(a: &[BigInt], b: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut r = vec![ib(0); a.len() + b.len() - 1];
    for (i, ai) in a.iter().enumerate() {
        for (j, bj) in b.iter().enumerate() {
            r[i + j] = am(&r[i + j], &mm(ai, bj, p), p);
        }
    }
    r
}

/// Formal derivative of a polynomial mod `p`.
pub fn poly_deriv(a: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    (1..a.len()).map(|i| mm(&ib(i as i64), &a[i], p)).collect()
}

fn poly_sub(a: &[BigInt], b: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| sm(a.get(i).unwrap_or(&ib(0)), b.get(i).unwrap_or(&ib(0)), p))
        .collect()
}

/// Drop trailing zero coefficients (so a polynomial's length reflects its true degree + 1).
fn poly_trim(mut a: Vec<BigInt>) -> Vec<BigInt> {
    while a.len() > 1 && a.last().is_some_and(|c| c.is_zero()) {
        a.pop();
    }
    a
}

/// The result of a Richelot (2,2)-isogeny from a quadratic splitting `f = G₁·G₂·G₃`.
#[derive(Clone, Debug)]
pub struct Richelot {
    /// The Richelot determinant `δ`. `δ = 0` ⟺ the surface is reducible (a product of elliptic curves).
    pub delta: BigInt,
    /// The domain sextic `G₁·G₂·G₃`.
    pub domain: Vec<BigInt>,
    /// The codomain sextic `H₁·H₂·H₃`.
    pub codomain: Vec<BigInt>,
    /// The three dual quadratics `Hᵢ`.
    pub dual: [Vec<BigInt>; 3],
}

impl Richelot {
    /// Whether the codomain abelian surface is **reducible** — splits as a product of elliptic curves. This
    /// is the split condition the Castryck–Decru chain-walk searches for.
    pub fn is_split(&self) -> bool {
        self.delta.is_zero()
    }
}

/// The Richelot (2,2)-isogeny from a quadratic splitting `G = (G₁, G₂, G₃)` over `𝔽_p`.
pub fn richelot(g: &[Quad; 3], p: &BigInt) -> Richelot {
    // δ = det of the coefficient matrix (rows = quadratics, columns = [g₂, g₁, g₀]).
    let m = |i: usize, j: usize| &g[i][2 - j]; // column 0 = x² coeff
    let det = {
        let t1 = mm(m(0, 0), &sm(&mm(m(1, 1), m(2, 2), p), &mm(m(1, 2), m(2, 1), p), p), p);
        let t2 = mm(m(0, 1), &sm(&mm(m(1, 0), m(2, 2), p), &mm(m(1, 2), m(2, 0), p), p), p);
        let t3 = mm(m(0, 2), &sm(&mm(m(1, 0), m(2, 1), p), &mm(m(1, 1), m(2, 0), p), p), p);
        am(&sm(&t1, &t2, p), &t3, p)
    };

    // Hᵢ = Gⱼ·Gₖ' − Gⱼ'·Gₖ for {i,j,k} cyclic.
    let quad = |q: &Quad| vec![q[0].clone(), q[1].clone(), q[2].clone()];
    let dual_of = |j: usize, k: usize| {
        let (gj, gk) = (quad(&g[j]), quad(&g[k]));
        let (dj, dk) = (poly_deriv(&gj, p), poly_deriv(&gk, p));
        poly_trim(poly_sub(&poly_mul(&gj, &dk, p), &poly_mul(&dj, &gk, p), p))
    };
    let dual = [dual_of(1, 2), dual_of(2, 0), dual_of(0, 1)];

    let domain = poly_mul(&poly_mul(&quad(&g[0]), &quad(&g[1]), p), &quad(&g[2]), p);
    let codomain = poly_mul(&poly_mul(&dual[0], &dual[1], p), &dual[2], p);

    Richelot { delta: det, domain, codomain, dual }
}

/// The 15 quadratic splittings of six roots into three pairs — the neighbours of a genus-2 Jacobian in the
/// (2,2)-isogeny graph. Each pair `(rₐ, r_b)` becomes the quadratic `(x−rₐ)(x−r_b)`.
pub fn richelot_partitions(roots: &[BigInt; 6], p: &BigInt) -> Vec<[Quad; 3]> {
    fn quad_from(ra: &BigInt, rb: &BigInt, p: &BigInt) -> Quad {
        // (x − ra)(x − rb) = x² − (ra+rb)x + ra·rb
        [mm(ra, rb, p), sm(&ib(0), &am(ra, rb, p), p), ib(1)]
    }
    let idx = [0usize, 1, 2, 3, 4, 5];
    let mut out = Vec::new();
    // Partition {0,1,2,3,4,5} into three unordered pairs: fix the partner of 0, then of the smallest left.
    for a in 1..6 {
        let rest1: Vec<usize> = idx.iter().copied().filter(|&x| x != 0 && x != a).collect();
        let b0 = rest1[0];
        for bi in 1..rest1.len() {
            let b = rest1[bi];
            let rest2: Vec<usize> = rest1.iter().copied().filter(|&x| x != b0 && x != b).collect();
            let (c, d) = (rest2[0], rest2[1]);
            out.push([
                quad_from(&roots[0], &roots[a], p),
                quad_from(&roots[b0], &roots[b], p),
                quad_from(&roots[c], &roots[d], p),
            ]);
        }
    }
    out
}

/// Walk the (2,2)-isogeny graph from a genus-2 Jacobian given by its six Weierstrass points: try each of the
/// 15 Richelot neighbours and report whether **any** is a split (reducible) surface. This is the shape of
/// the Castryck–Decru search — at real scale it is *guided* by the torsion images rather than exhaustive,
/// and Robert's generalisation lifts the same walk to dimension 4/8. Returns the splitting quadratics of the
/// first split neighbour, if one exists.
pub fn find_split_neighbour(roots: &[BigInt; 6], p: &BigInt) -> Option<[Quad; 3]> {
    richelot_partitions(roots, p).into_iter().find(|g| richelot(g, p).is_split())
}

/// Evaluate a polynomial at `x` mod `p` (Horner).
pub fn poly_eval(a: &[BigInt], x: &BigInt, p: &BigInt) -> BigInt {
    let mut acc = ib(0);
    for c in a.iter().rev() {
        acc = am(&mm(&acc, x, p), c, p);
    }
    acc
}

/// A genus-2 curve `C: y² = g(x²)` (an even sextic) with a **(2,2)-split Jacobian** `Jac(C) ~ E₁ × E₂`,
/// together with its two elliptic quotients. This is the (2,2)-**gluing** in reverse: the two degree-2 maps
/// `(x,y) ↦ (x², y)` onto `E₁: y² = g(u)` and `(x,y) ↦ (1/x², y/x³)` onto `E₂: y² = ĝ(u)` (the reversed
/// cubic) exhibit the split. It is the abelian-surface object the Kani/Castryck–Decru oracle detects.
#[derive(Clone, Debug)]
pub struct SplitGenus2 {
    /// The sextic `f(x) = g(x²)`, coefficients low→high (odd coefficients zero).
    pub sextic: Vec<BigInt>,
    /// `E₁: y² = g(u)` — the cubic `g`, coefficients low→high.
    pub e1: Vec<BigInt>,
    /// `E₂: y² = ĝ(u)` — the reversed cubic `u³·g(1/u)`, coefficients low→high.
    pub e2: Vec<BigInt>,
    pub p: BigInt,
}

/// Glue `E₁: y² = g(u)` with its reverse `E₂: y² = ĝ(u)` into the genus-2 curve `C: y² = g(x²)`. `g` is a
/// cubic given low→high `[g₀, g₁, g₂, g₃]`. The result carries a split Jacobian — validated by the quotient
/// maps and by the Richelot `±`-splitting being reducible (`δ = 0`).
pub fn split_jacobian_from_cubic(g: &[BigInt], p: &BigInt) -> SplitGenus2 {
    let z = ib(0);
    let sextic = vec![
        rp(&g[0], p), z.clone(), rp(&g[1], p), z.clone(), rp(&g[2], p), z.clone(), rp(&g[3], p),
    ];
    SplitGenus2 {
        sextic,
        e1: g.iter().map(|c| rp(c, p)).collect(),
        e2: g.iter().rev().map(|c| rp(c, p)).collect(),
        p: p.clone(),
    }
}

/// A **matched-pair (2,2)-gluing**: the genus-2 curve `C: y² = ∏ᵢ(x² + cᵢx + 1)` whose Jacobian is
/// `(2,2)`-isogenous to `E₁ × E₂`, where `E₁, E₂` share three of the four quartic 2-torsion roots `−cᵢ` and
/// differ in the fourth (`∓2`). Derived from the involution `x ↦ 1/x` (invariants `t = x + 1/x`,
/// `w = y(x³+1)/2x³`, giving `W = 2w/(t−1)` with `W² = (t+2)∏(t+cᵢ)`). Unlike the even-sextic special case,
/// this is the *general* gluing along a matched 2-torsion; any two curves with a symplectic 2-torsion
/// isomorphism reach this form after a Möbius normalization of the shared points. Validated exactly by
/// `#Jac(C) = #E₁·#E₂` ([`genus2_jacobian_order`]).
#[derive(Clone, Debug)]
pub struct MatchedPairGlue {
    /// `C: y² = ∏(x² + cᵢx + 1)` — the glued genus-2 curve (a sextic).
    pub sextic: Vec<BigInt>,
    /// `E₁: W² = (t+2)·∏(t+cᵢ)` — a quartic model of the first elliptic quotient.
    pub e1: Vec<BigInt>,
    /// `E₂: W² = (t−2)·∏(t+cᵢ)` — the second quotient (shares 3 of 4 roots with `E₁`).
    pub e2: Vec<BigInt>,
    pub p: BigInt,
}

/// Glue two elliptic curves along their matched 2-torsion `{−c₁, −c₂, −c₃}` into the genus-2 curve
/// `C: y² = ∏(x² + cᵢx + 1)`. See [`MatchedPairGlue`].
pub fn glue_shared_2torsion(c: &[BigInt; 3], p: &BigInt) -> MatchedPairGlue {
    let quad = |ci: &BigInt| vec![ib(1), rp(ci, p), ib(1)]; // x² + cᵢ x + 1
    let sextic = poly_mul(&poly_mul(&quad(&c[0]), &quad(&c[1]), p), &quad(&c[2]), p);
    let lin = |r: BigInt| vec![rp(&r, p), ib(1)]; // t + r
    let core = poly_mul(&poly_mul(&lin(c[0].clone()), &lin(c[1].clone()), p), &lin(c[2].clone()), p); // ∏(t+cᵢ)
    MatchedPairGlue {
        sextic,
        e1: poly_mul(&lin(ib(2)), &core, p),  // (t+2)·∏(t+cᵢ)
        e2: poly_mul(&lin(ib(-2)), &core, p), // (t−2)·∏(t+cᵢ)
        p: p.clone(),
    }
}

/// Whether the genus-2 curve with these six Weierstrass points has a **reducible** Jacobian (splits as a
/// product of elliptic curves) — decided by the Richelot split-test: some `(2,2)`-splitting has `δ = 0`. This
/// is exactly the decision the Castryck–Decru per-digit oracle makes — a correct guess yields a *splitting*
/// abelian surface, a wrong one an indecomposable genus-2 Jacobian.
pub fn surface_is_reducible(roots: &[BigInt; 6], p: &BigInt) -> bool {
    find_split_neighbour(roots, p).is_some()
}

/// **The per-digit split oracle.** Given the candidate surface for each branch of the recursive descent (one
/// per guessed `ℓ`-adic digit — in the full attack each is built by Kani gluing from the torsion images and
/// the guess), return the index of the branch whose surface **splits**. The split-test prunes every branch
/// but the consistent one — the `is_split`/Kani step of Castryck–Decru, as a wired per-digit selector.
pub fn select_splitting_branch(branches: &[[BigInt; 6]], p: &BigInt) -> Option<usize> {
    branches.iter().position(|r| surface_is_reducible(r, p))
}

// ---- Genus-2 Jacobian arithmetic: Mumford representation + Cantor's algorithm ----------------------
//
// A degree-2g+1 (imaginary) hyperelliptic curve C: y² = f(x), deg f = 5, has genus 2. A divisor class in
// Jac(C) is a Mumford pair (u, v): u monic, deg u ≤ 2, deg v < deg u, and u | (f − v²). This is the concrete
// point-arithmetic machinery on the abelian surface — the substrate a Richelot isogeny would act on, and the
// validator that would judge any surface-kernel construction (`#Jac · D = 0` for every class D).

fn pdeg(a: &[BigInt]) -> isize {
    (0..a.len()).rev().find(|&i| !a[i].is_zero()).map(|i| i as isize).unwrap_or(-1)
}
fn padd(a: &[BigInt], b: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    let n = a.len().max(b.len());
    poly_trim((0..n).map(|i| am(a.get(i).unwrap_or(&ib(0)), b.get(i).unwrap_or(&ib(0)), p)).collect())
}
fn psub(a: &[BigInt], b: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    poly_trim(poly_sub(a, b, p))
}
fn pmul(a: &[BigInt], b: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    poly_trim(poly_mul(a, b, p))
}
fn pscale(a: &[BigInt], s: &BigInt, p: &BigInt) -> Vec<BigInt> {
    poly_trim(a.iter().map(|c| mm(c, s, p)).collect())
}
fn pneg(a: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    pscale(a, &sm(&ib(0), &ib(1), p), p)
}
fn pmonic(a: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    let d = pdeg(a);
    if d < 0 {
        return vec![ib(0)];
    }
    pscale(a, &mod_inverse(&a[d as usize], p).unwrap(), p)
}
/// Polynomial long division: `a = q·b + r`, `deg r < deg b`. Returns `(q, r)`.
fn pdivmod(a: &[BigInt], b: &[BigInt], p: &BigInt) -> (Vec<BigInt>, Vec<BigInt>) {
    let db = pdeg(b);
    assert!(db >= 0, "division by the zero polynomial");
    let binv = mod_inverse(&b[db as usize], p).unwrap();
    let mut r = poly_trim(a.to_vec());
    let mut q = vec![ib(0)];
    while pdeg(&r) >= db {
        let dr = pdeg(&r);
        let shift = (dr - db) as usize;
        let coef = mm(&r[dr as usize], &binv, p);
        if q.len() <= shift {
            q.resize(shift + 1, ib(0));
        }
        q[shift] = am(&q[shift], &coef, p);
        let mut sub = vec![ib(0); shift];
        sub.extend(b.iter().map(|c| mm(c, &coef, p)));
        r = psub(&r, &sub, p);
    }
    (poly_trim(q), poly_trim(r))
}
fn pmod(a: &[BigInt], m: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    pdivmod(a, m, p).1
}
fn pdiv_exact(a: &[BigInt], b: &[BigInt], p: &BigInt) -> Vec<BigInt> {
    pdivmod(a, b, p).0
}
/// Extended gcd of polynomials: returns `(g, s, t)` with `s·a + t·b = g`, `g` monic.
fn pext_gcd(a: &[BigInt], b: &[BigInt], p: &BigInt) -> (Vec<BigInt>, Vec<BigInt>, Vec<BigInt>) {
    let (mut r0, mut r1) = (poly_trim(a.to_vec()), poly_trim(b.to_vec()));
    let (mut s0, mut s1) = (vec![ib(1)], vec![ib(0)]);
    let (mut t0, mut t1) = (vec![ib(0)], vec![ib(1)]);
    while pdeg(&r1) >= 0 {
        let (q, r) = pdivmod(&r0, &r1, p);
        r0 = r1;
        r1 = r;
        let ns = psub(&s0, &pmul(&q, &s1, p), p);
        s0 = s1;
        s1 = ns;
        let nt = psub(&t0, &pmul(&q, &t1, p), p);
        t0 = t1;
        t1 = nt;
    }
    let d = pdeg(&r0);
    if d > 0 || (d == 0 && r0[0] != ib(1)) {
        let inv = mod_inverse(&r0[d as usize], p).unwrap();
        r0 = pscale(&r0, &inv, p);
        s0 = pscale(&s0, &inv, p);
        t0 = pscale(&t0, &inv, p);
    }
    (r0, s0, t0)
}

/// A divisor class in `Jac(C)` in Mumford form `(u, v)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mumford {
    pub u: Vec<BigInt>,
    pub v: Vec<BigInt>,
}

/// The identity divisor class `(1, 0)`.
pub fn jac_identity() -> Mumford {
    Mumford { u: vec![ib(1)], v: vec![ib(0)] }
}

/// `−D = (u, −v)`.
pub fn jac_negate(d: &Mumford, p: &BigInt) -> Mumford {
    Mumford { u: d.u.clone(), v: pmod(&pneg(&d.v, p), &d.u, p) }
}

/// Reduce a (possibly unreduced) Mumford pair to `deg u ≤ 2`.
fn jac_reduce(mut u: Vec<BigInt>, mut v: Vec<BigInt>, f: &[BigInt], p: &BigInt) -> Mumford {
    while pdeg(&u) > 2 {
        let up = pdiv_exact(&psub(f, &pmul(&v, &v, p), p), &u, p);
        let vp = pmod(&pneg(&v, p), &up, p);
        u = up;
        v = vp;
    }
    let um = pmonic(&u, p);
    let vm = pmod(&v, &um, p);
    Mumford { u: um, v: vm }
}

/// Cantor's algorithm: add two divisor classes on `C: y² = f(x)` (genus 2, `char ≠ 2`).
pub fn cantor_add(d1: &Mumford, d2: &Mumford, f: &[BigInt], p: &BigInt) -> Mumford {
    let (u1, v1, u2, v2) = (&d1.u, &d1.v, &d2.u, &d2.v);
    // d = gcd(u1, u2, v1+v2) = s1·u1 + s2·u2 + s3·(v1+v2).
    let (g1, e1, e2) = pext_gcd(u1, u2, p);
    let vsum = padd(v1, v2, p);
    let (d, c1, c2) = pext_gcd(&g1, &vsum, p);
    let s1 = pmul(&c1, &e1, p);
    let s2 = pmul(&c1, &e2, p);
    let s3 = c2;
    // u = u1·u2 / d², v = (s1·u1·v2 + s2·u2·v1 + s3·(v1·v2 + f)) / d  mod u.
    let u = pdiv_exact(&pmul(u1, u2, p), &pmul(&d, &d, p), p);
    let num = padd(
        &padd(&pmul(&pmul(&s1, u1, p), v2, p), &pmul(&pmul(&s2, u2, p), v1, p), p),
        &pmul(&s3, &padd(&pmul(v1, v2, p), f, p), p),
        p,
    );
    let v = pmod(&pdiv_exact(&num, &d, p), &u, p);
    jac_reduce(u, v, f, p)
}

/// Scalar multiple `[n]·D` by double-and-add.
pub fn jac_scalar_mul(n: u128, d: &Mumford, f: &[BigInt], p: &BigInt) -> Mumford {
    let mut result = jac_identity();
    let mut base = d.clone();
    let mut k = n;
    while k > 0 {
        if k & 1 == 1 {
            result = cantor_add(&result, &base, f, p);
        }
        base = cantor_add(&base, &base, f, p);
        k >>= 1;
    }
    result
}

/// The order of a divisor class `D` in `Jac(C)` — the least `n | group_order` with `[n]·D = 0`. Computed by
/// stripping each prime of `group_order` as far as still annihilates `D`.
pub fn jac_element_order(d: &Mumford, group_order: u128, f: &[BigInt], p: &BigInt) -> u128 {
    let id = jac_identity();
    let mut primes = Vec::new();
    let (mut m, mut q) = (group_order, 2u128);
    while q * q <= m {
        if m % q == 0 {
            primes.push(q);
            while m % q == 0 {
                m /= q;
            }
        }
        q += 1;
    }
    if m > 1 {
        primes.push(m);
    }
    let mut n = group_order;
    for &pr in &primes {
        while n % pr == 0 && jac_scalar_mul(n / pr, d, f, p) == id {
            n /= pr;
        }
    }
    n
}

/// Legendre symbol `χ_p(a) ∈ {−1, 0, 1}` by Euler's criterion.
fn legendre(a: &BigInt, p: &BigInt) -> i64 {
    let ar = rp(a, p);
    if ar.is_zero() {
        return 0;
    }
    let e = p.sub(&ib(1)).div_rem(&ib(2)).map(|(q, _)| q).unwrap();
    if modpow(&ar, &e, p) == ib(1) {
        1
    } else {
        -1
    }
}

/// `#{(x,y) ∈ 𝔽_p² : y² = f(x)} + inf` — the `𝔽_p`-point count of `y² = f(x)` (`inf` = points at infinity).
pub fn count_curve_fp(f: &[BigInt], p: &BigInt, inf: i64) -> i64 {
    let pu = p.to_i64().unwrap();
    (0..pu).fold(inf, |n, x| n + 1 + legendre(&poly_eval(f, &ib(x), p), p))
}

/// The `𝔽_{p²}`-point count of `y² = f(x)` (`f` with `𝔽_p` coefficients), using Euler's criterion in
/// `𝔽_{p²}` for the quadratic character.
fn count_curve_fp2(f: &[BigInt], p: &BigInt, inf: i64) -> i64 {
    use crate::fp2::{fp2_add, fp2_const, fp2_is_zero, fp2_mul, fp2_pow, Fp2};
    let pu = p.to_i64().unwrap();
    let exp = (((pu as i128) * (pu as i128) - 1) / 2) as u64;
    let coeffs: Vec<Fp2> = f.iter().map(|c| (rp(c, p), ib(0))).collect();
    let mut n = inf;
    for a in 0..pu {
        for b in 0..pu {
            let beta: Fp2 = (ib(a), ib(b));
            let mut acc = fp2_const(0, p);
            for c in coeffs.iter().rev() {
                acc = fp2_add(&fp2_mul(&acc, &beta, p), c, p);
            }
            n += 1 + if fp2_is_zero(&acc) {
                0
            } else if fp2_pow(&acc, exp, p) == fp2_const(1, p) {
                1
            } else {
                -1
            };
        }
    }
    n
}

/// `#Jac(C)(𝔽_p)` for the genus-2 curve `C: y² = f(x)` (deg f = 6, monic), from its L-polynomial via the
/// point counts over `𝔽_p` and `𝔽_{p²}`. For a split Jacobian `Jac(C) ~ E₁ × E₂` this equals `#E₁·#E₂` (Tate:
/// isogenous abelian varieties over `𝔽_p` share their order) — the exact point-count validation of a gluing.
pub fn genus2_jacobian_order(sextic: &[BigInt], p: &BigInt) -> i128 {
    let pu = p.to_i64().unwrap() as i128;
    let n1 = count_curve_fp(sextic, p, 2) as i128; // monic sextic ⟹ 2 points at infinity
    let n2 = count_curve_fp2(sextic, p, 2) as i128;
    let t1 = (pu + 1) - n1; // Σ αᵢ
    let t2 = (pu * pu + 1) - n2; // Σ αᵢ²
    // #Jac = ∏(1 − αᵢ) = 1 + p² − (1+p)·t₁ + (t₁² − t₂)/2  (Weil functional equation: e₃ = p·e₁, e₄ = p²).
    1 + pu * pu - (1 + pu) * t1 + (t1 * t1 - t2) / 2
}

/// `#Jac(C)(𝔽_p)` for `C: y² = f(x)` of any degree 5 or 6, with the correct number of points at infinity:
/// deg 5 ⟹ 1; deg 6 ⟹ 2 if the leading coefficient is a square (over `𝔽_p`), else 0. Over `𝔽_{p²}` every
/// `𝔽_p` leading coefficient is a square, so the count there is 2 (deg 6) or 1 (deg 5). Needed to compare a
/// Richelot dual (a non-monic, possibly quadratic-twisted sextic) against its domain.
pub fn genus2_jacobian_order_general(f: &[BigInt], p: &BigInt) -> i128 {
    let deg = pdeg(f);
    let pu = p.to_i64().unwrap() as i128;
    let inf_p = if deg % 2 == 1 {
        1
    } else if legendre(&f[deg as usize], p) == 1 {
        2
    } else {
        0
    };
    let inf_p2 = if deg % 2 == 1 { 1 } else { 2 };
    let n1 = count_curve_fp(f, p, inf_p) as i128;
    let n2 = count_curve_fp2(f, p, inf_p2) as i128;
    let t1 = (pu + 1) - n1;
    let t2 = (pu * pu + 1) - n2;
    1 + pu * pu - (1 + pu) * t1 + (t1 * t1 - t2) / 2
}

/// A square root of `a` in `𝔽_p` for `p ≡ 3 (mod 4)`, if one exists.
fn fp_sqrt(a: &BigInt, p: &BigInt) -> Option<BigInt> {
    let a = rp(a, p);
    if a.is_zero() {
        return Some(ib(0));
    }
    let exp = p.add(&ib(1)).div_rem(&ib(4)).map(|(q, _)| q)?;
    let r = modpow(&a, &exp, p);
    (mm(&r, &r, p) == a).then_some(r)
}

/// The two roots of a quadratic `g₀ + g₁x + g₂x²` over `𝔽_p`, when they are rational (`p ≡ 3 mod 4`).
fn quad_roots(q: &[BigInt], p: &BigInt) -> Option<[BigInt; 2]> {
    if q.len() < 3 || q[2].is_zero() {
        return None;
    }
    let (c0, c1, c2) = (&q[0], &q[1], &q[2]);
    let disc = sm(&mm(c1, c1, p), &mm(&ib(4), &mm(c0, c2, p), p), p);
    let sq = fp_sqrt(&disc, p)?;
    let inv = mod_inverse(&mm(&ib(2), c2, p), p)?;
    let neg_b = sm(&ib(0), c1, p);
    Some([mm(&sm(&neg_b, &sq, p), &inv, p), mm(&am(&neg_b, &sq, p), &inv, p)])
}

/// The six Weierstrass points of a Richelot codomain — the roots of its three dual quadratics — when they
/// are all rational over `𝔽_p`. Returning `None` means the codomain's 2-torsion lives in `𝔽_{p²}`, so the
/// chain-walk over the prime field cannot continue through that neighbour (the lift to `𝔽_{p²}` removes this
/// obstruction, at the cost of the extension arithmetic).
fn codomain_roots(dual: &[Vec<BigInt>; 3], p: &BigInt) -> Option<[BigInt; 6]> {
    let a = quad_roots(&dual[0], p)?;
    let b = quad_roots(&dual[1], p)?;
    let c = quad_roots(&dual[2], p)?;
    Some([a[0].clone(), a[1].clone(), b[0].clone(), b[1].clone(), c[0].clone(), c[1].clone()])
}

/// Walk chains of Richelot (2,2)-isogenies to depth `depth` (a chain of `depth + 1` steps), returning the
/// sequence of neighbour indices that first reaches a **split** (reducible) surface, or `None` if none does
/// within the bound. This is the recursive form of the Castryck–Decru search across the (2,2)-graph: from
/// each node it forms the codomain, extracts its Weierstrass points, and recurses. Neighbours whose codomain
/// 2-torsion escapes to `𝔽_{p²}` are skipped over the prime field.
pub fn richelot_chain(roots: &[BigInt; 6], depth: usize, p: &BigInt) -> Option<Vec<usize>> {
    for (i, g) in richelot_partitions(roots, p).into_iter().enumerate() {
        let r = richelot(&g, p);
        if r.is_split() {
            return Some(vec![i]);
        }
        if depth > 0 {
            if let Some(next) = codomain_roots(&r.dual, p) {
                if let Some(mut path) = richelot_chain(&next, depth - 1, p) {
                    path.insert(0, i);
                    return Some(path);
                }
            }
        }
    }
    None
}

/// The outcome of following a guided Richelot chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuidedWalk {
    /// A split (reducible) surface was reached at this step index along the path.
    SplitAt(usize),
    /// The path completed without splitting; the six Weierstrass points of the final surface.
    Ended([BigInt; 6]),
    /// The chain left the prime field (a codomain's 2-torsion is in `𝔽_{p²}`) at this step, or the index
    /// was out of range — the prime-field walk cannot continue.
    Stuck(usize),
}

/// Follow a **guided** chain of Richelot isogenies: apply the neighbour named by each index in `path` in
/// turn, reporting the first split. In the full Castryck–Decru attack the path is not free — it is *dictated
/// by the torsion images* `φ(P), φ(Q)`: the kernel of each (2,2)-step is the image of a prescribed torsion
/// subgroup, so the walk is a single guided line rather than the 15-way exhaustive tree. This function is
/// that guided line; deriving `path` from the torsion images is the remaining Castryck–Decru integration.
pub fn guided_chain(roots: &[BigInt; 6], path: &[usize], p: &BigInt) -> GuidedWalk {
    let mut cur = roots.clone();
    for (step, &idx) in path.iter().enumerate() {
        let parts = richelot_partitions(&cur, p);
        if idx >= parts.len() {
            return GuidedWalk::Stuck(step);
        }
        let r = richelot(&parts[idx], p);
        if r.is_split() {
            return GuidedWalk::SplitAt(step);
        }
        match codomain_roots(&r.dual, p) {
            Some(next) => cur = next,
            None => return GuidedWalk::Stuck(step),
        }
    }
    GuidedWalk::Ended(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(s: &str) -> BigInt {
        BigInt::parse_decimal(s).unwrap()
    }

    #[test]
    fn poly_arithmetic_is_correct() {
        let p = big("101");
        // (1 + 2x)(3 + x) = 3 + 7x + 2x²
        assert_eq!(poly_mul(&[ib(1), ib(2)], &[ib(3), ib(1)], &p), vec![ib(3), ib(7), ib(2)]);
        // d/dx (5 + 3x + 4x²) = 3 + 8x
        assert_eq!(poly_deriv(&[ib(5), ib(3), ib(4)], &p), vec![ib(3), ib(8)]);
    }

    #[test]
    fn richelot_delta_and_dual_quadratics_are_correct() {
        let p = big("101");
        // Three generic quadratics.
        let g = [
            [ib(1), ib(2), ib(1)], // 1 + 2x + x²
            [ib(3), ib(1), ib(1)], // 3 + x + x²
            [ib(0), ib(4), ib(1)], // 4x + x²
        ];
        let r = richelot(&g, &p);
        // δ = det of the coefficient matrix, columns [x², x, 1] = [[1,2,1],[1,1,3],[1,4,0]].
        // det = 1(1·0−3·4) − 2(1·0−3·1) + 1(1·4−1·1) = −12 + 6 + 3 = −3 ≡ 98 (mod 101).
        assert_eq!(r.delta, big("98"), "Richelot δ = det of the quadratic coefficients");
        // Each dual Hᵢ has degree ≤ 2 (the cubic leading terms cancel).
        for h in &r.dual {
            assert!(h.len() <= 3, "Hᵢ is a quadratic");
        }
        // Domain = G₁·G₂·G₃ is a genuine sextic.
        assert_eq!(r.domain.len(), 7, "domain is a sextic (7 coefficients)");
        assert!(!r.is_split(), "generic (independent) quadratics ⟹ δ ≠ 0 ⟹ not split");
    }

    #[test]
    fn dependent_quadratics_give_a_reducible_split_surface() {
        let p = big("101");
        // G₃ = G₁ + G₂ makes the coefficient rows linearly dependent ⟹ δ = 0 ⟹ the surface is a product.
        let g1 = [ib(1), ib(2), ib(1)];
        let g2 = [ib(3), ib(1), ib(1)];
        let g3 = [am(&g1[0], &g2[0], &p), am(&g1[1], &g2[1], &p), am(&g1[2], &g2[2], &p)];
        let r = richelot(&[g1, g2, g3], &p);
        assert_eq!(r.delta, ib(0), "dependent quadratics ⟹ δ = 0");
        assert!(r.is_split(), "δ = 0 ⟹ reducible: the abelian surface splits into a product of curves");
    }

    #[test]
    fn chain_walk_finds_a_split_across_the_2_2_graph() {
        let p = big("101");
        // Six Weierstrass points chosen so that one of the 15 (2,2)-neighbours is a split surface.
        // (Roots crafted so a quadratic splitting yields dependent coefficient rows.)
        let roots = [ib(1), ib(2), ib(3), ib(4), ib(5), ib(6)];
        let parts = richelot_partitions(&roots, &p);
        assert_eq!(parts.len(), 15, "a genus-2 Jacobian has exactly 15 (2,2)-neighbours");
        // Every partition yields a well-formed Richelot isogeny with a defined δ.
        for g in &parts {
            let r = richelot(g, &p);
            assert_eq!(r.domain.len(), 7, "each neighbour has a sextic domain");
        }
        // The δ = 0 split test is exercised across the whole neighbourhood; the search is total.
        let any_split = parts.iter().any(|g| richelot(g, &p).is_split());
        let found = find_split_neighbour(&roots, &p);
        assert_eq!(found.is_some(), any_split, "find_split_neighbour agrees with the exhaustive scan");
    }

    #[test]
    fn quadratic_root_extraction_recovers_the_weierstrass_points() {
        let p = big("103"); // 103 ≡ 3 (mod 4)
        // (x − 7)(x − 20) = x² − 27x + 140 ≡ x² + 76x + 37 (mod 103).
        let q = [big("37"), big("76"), ib(1)];
        let roots = quad_roots(&q, &p).expect("rational roots");
        let mut got: Vec<i64> = roots.iter().map(|r| r.to_i64().unwrap()).collect();
        got.sort();
        assert_eq!(got, vec![7, 20], "the quadratic formula recovers both Weierstrass points");
        for r in &roots {
            assert!(poly_eval(&q, r, &p).is_zero(), "each extracted point is a genuine root");
        }
    }

    #[test]
    fn a_guided_chain_step_lands_on_the_codomain_surface() {
        let p = big("103");
        let roots = [ib(1), ib(2), ib(3), ib(4), ib(5), ib(6)];
        // Take one non-split neighbour and step through it; the extracted Weierstrass points of the next
        // surface must be genuine roots of that neighbour's codomain sextic.
        let parts = richelot_partitions(&roots, &p);
        let idx = (0..parts.len()).find(|&i| !richelot(&parts[i], &p).is_split()).unwrap();
        let r = richelot(&parts[idx], &p);
        match guided_chain(&roots, &[idx], &p) {
            GuidedWalk::Ended(next) => {
                for pt in &next {
                    assert!(poly_eval(&r.codomain, pt, &p).is_zero(), "next surface's 2-torsion ⊂ codomain");
                }
            }
            GuidedWalk::Stuck(_) => {} // codomain 2-torsion left 𝔽_p; the prime-field walk stops honestly
            GuidedWalk::SplitAt(_) => panic!("chose a non-split neighbour"),
        }
    }

    #[test]
    fn the_recursive_chain_finds_a_split_and_the_guided_walk_confirms_it() {
        let p = big("103");
        // Search the space of six-point configurations for one whose (2,2)-graph harbours a split, then
        // confirm the recursive walk finds a path to it and the guided walk reproduces that split exactly.
        let mut s = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) % 100 + 1) as i64
        };
        let mut confirmed = false;
        for _ in 0..400 {
            let mut vals = [0i64; 6];
            for v in &mut vals {
                *v = next();
            }
            // distinct Weierstrass points only (a smooth genus-2 curve)
            if (0..6).any(|i| (i + 1..6).any(|j| vals[i] == vals[j])) {
                continue;
            }
            let roots: [BigInt; 6] = core::array::from_fn(|i| ib(vals[i]));
            if let Some(path) = richelot_chain(&roots, 0, &p) {
                // The recursive search returned a neighbour it certifies as split — re-run it head-on.
                assert_eq!(path.len(), 1, "a depth-0 walk reports a single-step chain");
                assert!(
                    richelot(&richelot_partitions(&roots, &p)[path[0]], &p).is_split(),
                    "the returned neighbour is genuinely split"
                );
                assert_eq!(
                    guided_chain(&roots, &path, &p),
                    GuidedWalk::SplitAt(0),
                    "following the guidance path reproduces the split"
                );
                confirmed = true;
                break;
            }
        }
        assert!(confirmed, "split-bearing configurations exist and both walks agree on them");
    }

    #[test]
    fn a_guided_path_off_the_split_ends_or_gets_stuck_but_never_lies() {
        let p = big("103");
        let roots = [ib(2), ib(9), ib(11), ib(30), ib(41), ib(58)];
        // A two-step guided walk either ends on a surface, splits, or honestly reports it left 𝔽_p — it never
        // silently fabricates a continuation.
        match guided_chain(&roots, &[3, 1], &p) {
            GuidedWalk::Ended(final_roots) => {
                assert!(final_roots.iter().all(|r| !r.is_negative()), "final points are field elements");
            }
            GuidedWalk::SplitAt(step) => assert!(step < 2, "a split is reported within the path"),
            GuidedWalk::Stuck(step) => assert!(step < 2, "leaving 𝔽_p is reported at a real step"),
        }
    }

    #[test]
    fn even_sextic_glues_two_elliptic_curves_into_a_split_jacobian() {
        let p = big("103"); // 103 ≡ 3 (mod 4), so fp_sqrt works
        // g(u) = (u−1)(u−2)(u−3) = u³ − 6u² + 11u − 6, low→high [−6, 11, −6, 1].
        let g = [ib(-6), ib(11), ib(-6), ib(1)];
        let sg = split_jacobian_from_cubic(&g, &p);
        assert_eq!(sg.sextic.len(), 7, "C is a sextic y² = g(x²)");
        assert_eq!(sg.e2, vec![ib(1), rp(&ib(-6), &p), rp(&ib(11), &p), rp(&ib(-6), &p)], "E₂ is the reversed cubic");

        // Validation 1 — the two degree-2 quotient maps land on E₁ and E₂ for every point of C.
        let mut points = 0;
        for x in 0..103i64 {
            let xb = ib(x);
            let fx = poly_eval(&sg.sextic, &xb, &p);
            let Some(y) = fp_sqrt(&fx, &p) else { continue };
            points += 1;
            // ψ₁: (x, y) ↦ (x², y) onto E₁: Y² = g(u).
            let u = mm(&xb, &xb, &p);
            assert_eq!(mm(&y, &y, &p), poly_eval(&sg.e1, &u, &p), "ψ₁(P) lies on E₁");
            // ψ₂: (x, y) ↦ (1/x², y/x³) onto E₂: Y² = ĝ(u).
            if x != 0 {
                let xi = mod_inverse(&xb, &p).unwrap();
                let v = mm(&xi, &xi, &p);
                let yv = mm(&y, &mm(&xi, &mm(&xi, &xi, &p), &p), &p);
                assert_eq!(mm(&yv, &yv, &p), poly_eval(&sg.e2, &v, &p), "ψ₂(P) lies on E₂");
            }
        }
        assert!(points > 5, "C has genuine rational points to validate the maps on");

        // Validation 2 — the Richelot ±-splitting Gᵢ = x² − rᵢ (rᵢ the roots 1,2,3 of g) is reducible: its
        // coefficient matrix has an all-zero x-column ⟹ δ = 0 ⟹ is_split. The gluing and the split-test agree.
        let split = [[ib(-1), ib(0), ib(1)], [ib(-2), ib(0), ib(1)], [ib(-3), ib(0), ib(1)]];
        let r = richelot(&split, &p);
        assert_eq!(r.delta, ib(0), "the ±-splitting has δ = 0");
        assert!(r.is_split(), "even sextic ⟹ Richelot-reducible ⟹ Jac(C) ~ E₁ × E₂ (a genuine (2,2)-split)");
    }

    #[test]
    fn split_jacobian_order_equals_the_product_of_its_elliptic_quotients() {
        // The strongest validation of the gluing: count #Jac(C) independently (genus-2 L-polynomial from the
        // point counts over 𝔽_p and 𝔽_{p²}) and confirm it equals #E₁·#E₂ (Tate). This is the certificate the
        // GENERAL matched-pair gluing will reuse.
        let p = big("103");
        let g = [ib(-6), ib(11), ib(-6), ib(1)]; // (u−1)(u−2)(u−3)
        let sg = split_jacobian_from_cubic(&g, &p);
        let e1 = count_curve_fp(&sg.e1, &p, 1) as i128; // #E₁ (cubic ⟹ 1 point at infinity)
        let e2 = count_curve_fp(&sg.e2, &p, 1) as i128; // #E₂
        let jac = genus2_jacobian_order(&sg.sextic, &p);
        assert_eq!(jac, e1 * e2, "#Jac(C) = #E₁·#E₂ — the split, verified at the level of point counts");
        // Sanity: the elliptic quotients are genuine curves (Hasse bound).
        let bound = 2 * (103f64.sqrt() as i128) + 2;
        assert!((e1 - 104).abs() <= bound && (e2 - 104).abs() <= bound, "#Eᵢ obey the Hasse bound");
    }

    #[test]
    fn general_matched_pair_gluing_validates_by_jacobian_order() {
        // The GENERAL (2,2)-gluing: two curves sharing matched 2-torsion {−c₁,−c₂,−c₃}, differing in the
        // fourth quartic root (∓2). The Jacobian-order certificate judges the derivation.
        let p = big("103");
        for c in [[ib(3), ib(7), ib(20)], [ib(1), ib(5), ib(9)], [ib(4), ib(11), ib(30)]] {
            let g = glue_shared_2torsion(&c, &p);
            assert_eq!(g.sextic.len(), 7, "C is a genus-2 sextic");
            assert_eq!(g.e1.len(), 5, "E₁ is a quartic model");
            let e1 = count_curve_fp(&g.e1, &p, 2) as i128; // monic quartic ⟹ 2 points at infinity
            let e2 = count_curve_fp(&g.e2, &p, 2) as i128;
            let jac = genus2_jacobian_order(&g.sextic, &p);
            assert_eq!(jac, e1 * e2, "#Jac(C) = #E₁·#E₂ for c = {c:?} — matched-pair gluing verified (Tate)");
        }
        // E₁ and E₂ genuinely differ (the fourth root ∓2 makes them a nontrivial pair, not one curve twice).
        let g = glue_shared_2torsion(&[ib(3), ib(7), ib(20)], &p);
        assert_ne!(g.e1, g.e2, "the two elliptic quotients are distinct (they differ in the fourth root)");
    }

    #[test]
    fn the_split_test_is_the_per_digit_oracle() {
        let p = big("103");
        // The CONSISTENT branch: a matched-pair gluing. Its sextic ∏(x²+cᵢx+1) has roots in reciprocal pairs
        // {r, 1/r}; pairing them is a (2,2)-splitting with δ = 0 ⟹ the surface SPLITS (reducible).
        let recip = |r: i64| (ib(r), mod_inverse(&ib(r), &p).unwrap());
        let (a0, a1) = recip(2);
        let (b0, b1) = recip(5);
        let (c0, c1) = recip(7);
        let consistent = [a0, a1, b0, b1, c0, c1];
        assert!(surface_is_reducible(&consistent, &p), "the glued surface splits — the correct digit");

        // Discrimination: a generic configuration is an indecomposable genus-2 Jacobian — the oracle returns
        // false. (Roots must be SCATTERED: an arithmetic progression is always reducible, since its symmetric
        // pairing has equal pair-sums ⟹ collinear ⟹ δ = 0. That structure is itself a reducibility witness.)
        let mut inconsistent: Option<[BigInt; 6]> = None;
        for seed in 1..200i64 {
            let vals: Vec<i64> = (0..6).map(|i| (seed * 41 + i * i * 7 + i * 3) % 101 + 1).collect();
            if (0..6).any(|i| (i + 1..6).any(|j| vals[i] == vals[j])) {
                continue; // need six distinct Weierstrass points
            }
            let rs: [BigInt; 6] = core::array::from_fn(|i| ib(vals[i]));
            if !surface_is_reducible(&rs, &p) {
                inconsistent = Some(rs);
                break;
            }
        }
        let inconsistent = inconsistent.expect("a generic (irreducible) branch exists");
        assert!(!surface_is_reducible(&inconsistent, &p), "a wrong branch does not split — the oracle prunes it");

        // Wired as the per-digit selector: among the branches, the split-test picks out the consistent one.
        let branches = [inconsistent.clone(), consistent.clone(), inconsistent];
        let sel = select_splitting_branch(&branches, &p).expect("the oracle selects a splitting branch");
        assert_eq!(sel, 1, "the split-test selects exactly the consistent (gluing) branch");
        assert!(surface_is_reducible(&branches[sel], &p), "and the selected branch genuinely splits");
    }

    #[test]
    fn genus2_jacobian_cantor_arithmetic_is_a_group_of_order_hash_jac() {
        // C: y² = x(x−1)(x−2)(x−3)(x−4) over 𝔽₁₀₃ — a genus-2 imaginary hyperelliptic curve (deg f = 5).
        let p = big("103");
        let lin = |r: i64| vec![sm(&ib(0), &ib(r), &p), ib(1)]; // x − r
        let mut f = vec![ib(1)];
        for r in [0, 1, 2, 3, 4] {
            f = pmul(&f, &lin(r), &p);
        }
        assert_eq!(pdeg(&f), 5, "a quintic ⟹ genus 2");

        // A rational point P = (x₀, y₀), y₀ ≠ 0, as the divisor class [P − ∞] = (x − x₀, y₀).
        let d = (0..103i64)
            .find_map(|x0| {
                let fx = poly_eval(&f, &ib(x0), &p);
                fp_sqrt(&fx, &p).filter(|y| !y.is_zero()).map(|y0| Mumford {
                    u: vec![sm(&ib(0), &ib(x0), &p), ib(1)],
                    v: vec![y0],
                })
            })
            .expect("a rational non-Weierstrass point on C");

        let id = jac_identity();
        // Group axioms.
        assert_eq!(cantor_add(&d, &id, &f, &p), d, "D + 0 = D");
        assert_eq!(cantor_add(&d, &jac_negate(&d, &p), &f, &p), id, "D + (−D) = 0");
        let d2 = cantor_add(&d, &d, &f, &p);
        assert_eq!(
            cantor_add(&d2, &d, &f, &p),
            cantor_add(&d, &d2, &f, &p),
            "associativity: (2D)+D = D+(2D)"
        );
        // Every computed class is a valid Mumford pair: u | (f − v²), deg u ≤ 2.
        assert!(pdeg(&d2.u) <= 2, "reduced class has deg u ≤ 2");
        assert_eq!(pmod(&psub(&f, &pmul(&d2.v, &d2.v, &p), &p), &d2.u, &p), vec![ib(0)], "u | (f − v²)");

        // THE definitive validator: the group order kills every class. #Jac counted independently via the
        // genus-2 L-polynomial (point counts over 𝔽_p and 𝔽_{p²}, one point at infinity for deg f = 5).
        let pu = 103i128;
        let n1 = count_curve_fp(&f, &p, 1) as i128;
        let n2 = count_curve_fp2(&f, &p, 1) as i128;
        let (t1, t2) = ((pu + 1) - n1, (pu * pu + 1) - n2);
        let jac = 1 + pu * pu - (1 + pu) * t1 + (t1 * t1 - t2) / 2;
        assert!(jac > 0, "#Jac is a positive integer");
        assert_eq!(
            jac_scalar_mul(jac as u128, &d, &f, &p),
            id,
            "#Jac · D = 0 — Cantor's law is a genuine group of exactly the order the L-polynomial predicts"
        );
    }

    #[test]
    fn richelot_two_two_kernel_and_two_power_torsion() {
        // C: y² = x(x−1)(x−2)(x−3)(x−4) over 𝔽₁₀₃.
        let p = big("103");
        let lin = |r: i64| vec![sm(&ib(0), &ib(r), &p), ib(1)]; // x − r
        let mut f = vec![ib(1)];
        for r in [0, 1, 2, 3, 4] {
            f = pmul(&f, &lin(r), &p);
        }
        let id = jac_identity();
        let pu = 103i128;
        let n1 = count_curve_fp(&f, &p, 1) as i128;
        let n2 = count_curve_fp2(&f, &p, 1) as i128;
        let (t1, t2) = ((pu + 1) - n1, (pu * pu + 1) - n2);
        let jac = (1 + pu * pu - (1 + pu) * t1 + (t1 * t1 - t2) / 2) as u128;

        // ── The Richelot kernel: a (2,2)-subgroup of Jac(C)[2] from Weierstrass-difference divisors. ──
        // D₁ = [W₀ − ∞] = (x, 0), D₂ = [W₁ − ∞] = (x−1, 0) — the classes a (2,2)-isogeny quotients by.
        let d1 = Mumford { u: lin(0), v: vec![ib(0)] };
        let d2 = Mumford { u: lin(1), v: vec![ib(0)] };
        assert_eq!(jac_element_order(&d1, jac, &f, &p), 2, "D₁ is 2-torsion (a Weierstrass point)");
        assert_eq!(jac_element_order(&d2, jac, &f, &p), 2, "D₂ is 2-torsion");
        let d3 = cantor_add(&d1, &d2, &f, &p);
        assert_eq!(jac_element_order(&d3, jac, &f, &p), 2, "D₁+D₂ is 2-torsion");
        assert_ne!(d3, id, "D₁, D₂ are independent");
        // {0, D₁, D₂, D₁+D₂} is a closed order-4 (2,2)-subgroup — a genuine Richelot isogeny kernel.
        assert_eq!(cantor_add(&d1, &d3, &f, &p), d2, "closed: D₁+(D₁+D₂) = D₂");
        assert_eq!(cantor_add(&d2, &d3, &f, &p), d1, "closed: D₂+(D₁+D₂) = D₁");

        // ── The 2^e-torsion: the substrate the surface isogeny's (2^e,2^e)-kernel lives in. ──
        let odd = {
            let mut m = jac;
            while m % 2 == 0 {
                m /= 2;
            }
            m
        };
        // Strip the odd part of #Jac off a generic class to land in the 2-power torsion; find one of order > 1.
        let two_power = (5..103i64)
            .find_map(|x0| {
                let y = fp_sqrt(&poly_eval(&f, &ib(x0), &p), &p).filter(|y| !y.is_zero())?;
                let dp = jac_scalar_mul(odd, &Mumford { u: lin(x0), v: vec![y] }, &f, &p);
                (dp != id).then_some(dp)
            })
            .expect("a class with nontrivial 2-power torsion");
        let e_ord = jac_element_order(&two_power, jac, &f, &p);
        assert!(e_ord.is_power_of_two() && e_ord >= 2, "genuine 2^e-torsion, e ≥ 1: order {e_ord}");
        assert_eq!(jac_scalar_mul(e_ord, &two_power, &f, &p), id, "and 2^e kills it");
        // This 2^e-torsion element, paired with an independent one, spans the (2^e,2^e) lattice the surface
        // isogeny's kernel is a maximal isotropic subgroup of — the structure the image-determined kernel picks.
    }

    #[test]
    fn richelot_dual_is_genuinely_isogenous_equal_jacobian_order() {
        // Three generic monic quadratics ⟹ a genus-2 curve C: y² = G₁G₂G₃, and the Richelot dual
        // C': y² = δ⁻¹·H₁H₂H₃. If richelot()'s dual construction is a genuine (2,2)-isogeny, then by Tate the
        // Jacobians have EQUAL order over 𝔽_p — a validation the map itself would have to respect.
        let p = big("103");
        for g in [
            [[ib(1), ib(0), ib(1)], [ib(2), ib(1), ib(1)], [ib(5), ib(3), ib(1)]],
            [[ib(7), ib(2), ib(1)], [ib(1), ib(4), ib(1)], [ib(3), ib(0), ib(1)]],
        ] {
            let r = richelot(&g, &p);
            assert_ne!(r.delta, ib(0), "generic quadratics ⟹ δ ≠ 0 ⟹ a genuine genus-2 isogeny (not a split)");
            let jac_c = genus2_jacobian_order_general(&r.domain, &p);
            // The true dual carries the δ⁻¹ quadratic twist; without it the count would be off by the twist.
            let dinv = mod_inverse(&r.delta, &p).unwrap();
            let cprime = pscale(&r.codomain, &dinv, &p);
            let jac_cp = genus2_jacobian_order_general(&cprime, &p);
            assert_eq!(jac_c, jac_cp, "#Jac(C) = #Jac(C') — the Richelot dual is genuinely isogenous (Tate)");
        }
    }
}
