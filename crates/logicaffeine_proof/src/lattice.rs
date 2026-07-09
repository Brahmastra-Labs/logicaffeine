//! LLL lattice reduction — the geometric lens of the compression campaign.
//!
//! Every rung so far reads structure through a finite field: linear/algebraic/correlation attacks are
//! linear algebra or statistics over GF(2), GF(p), GF(2ⁿ). Lattice reduction reads it through the
//! GEOMETRY OF NUMBERS instead — an orthogonal lens. A lattice is the integer span of a basis; its short
//! vectors are its most compressed representatives, and a SHORT vector *is* a hidden small relation among
//! the basis rows. So "find the shortest vector" is exactly "find the maximal compression," and the
//! symmetry being broken is the lattice's own geometry rather than a field's algebra.
//!
//! This is the lens that reaches the cryptographic weaknesses the field-based rungs cannot express: an
//! RSA key that is *structured but not factorable by Fermat* — small hidden roots (Coppersmith),
//! partial key exposure, a private exponent below Wiener's bound (Boneh–Durfee `d < N^0.292`) — leaks a
//! short lattice vector even when no factoring shortcut exists. It is a different game than factoring:
//! the compression game, played on the reals. [`lll_reduce`] is that lens; Coppersmith's small-roots
//! method is built on top of it.
//!
//! The reduction is EXACT — Gram–Schmidt over [`Rational`], no floating point — so its output is a
//! certified reduced basis, not a numerical approximation.

use logicaffeine_base::numeric::{BigInt, Rational};

fn zero() -> Rational {
    Rational::zero()
}

fn rat(n: i64, d: i64) -> Rational {
    Rational::new(BigInt::from_i64(n), BigInt::from_i64(d)).expect("nonzero denominator")
}

/// The inner product of two rational vectors.
fn dot(a: &[Rational], b: &[Rational]) -> Rational {
    a.iter().zip(b).fold(zero(), |s, (x, y)| s.add(&x.mul(y)))
}

/// Gram–Schmidt orthogonalization: the orthogonal vectors `b*ᵢ` and the coefficients
/// `μᵢⱼ = ⟨bᵢ, b*ⱼ⟩ / ⟨b*ⱼ, b*ⱼ⟩`, all exact over the rationals.
fn gram_schmidt(b: &[Vec<Rational>]) -> (Vec<Vec<Rational>>, Vec<Vec<Rational>>) {
    let n = b.len();
    let mut bstar: Vec<Vec<Rational>> = Vec::with_capacity(n);
    let mut mu = vec![vec![zero(); n]; n];
    for i in 0..n {
        let mut v = b[i].clone();
        for j in 0..i {
            let m = dot(&b[i], &bstar[j]).div(&dot(&bstar[j], &bstar[j])).expect("independent basis");
            for d in 0..v.len() {
                v[d] = v[d].sub(&m.mul(&bstar[j][d]));
            }
            mu[i][j] = m;
        }
        bstar.push(v);
    }
    (bstar, mu)
}

/// The LLL algorithm proper, on rational vectors: returns a reduced basis of the same lattice.
fn lll_core(mut b: Vec<Vec<Rational>>) -> Vec<Vec<Rational>> {
    let n = b.len();
    if n < 2 {
        return b;
    }
    let delta = rat(3, 4);
    let half = rat(1, 2);
    let (mut bstar, mut mu) = gram_schmidt(&b);
    let mut norm: Vec<Rational> = (0..n).map(|i| dot(&bstar[i], &bstar[i])).collect();

    let mut k = 1;
    while k < n {
        // Size-reduce bₖ against b_{k-1}, …, b₀.
        for j in (0..k).rev() {
            if mu[k][j].abs() > half {
                let q = mu[k][j].round(); // nearest integer
                let qr = Rational::from_bigint(q);
                for d in 0..b[k].len() {
                    b[k][d] = b[k][d].sub(&qr.mul(&b[j][d]));
                }
                mu[k][j] = mu[k][j].sub(&qr);
                for l in 0..j {
                    mu[k][l] = mu[k][l].sub(&qr.mul(&mu[j][l]));
                }
            }
        }
        // Lovász condition: ‖b*ₖ‖² ≥ (δ − μ²ₖ,ₖ₋₁)·‖b*ₖ₋₁‖².
        let rhs = delta.sub(&mu[k][k - 1].mul(&mu[k][k - 1])).mul(&norm[k - 1]);
        if norm[k] >= rhs {
            k += 1;
        } else {
            b.swap(k, k - 1);
            let (bs, m) = gram_schmidt(&b);
            bstar = bs;
            mu = m;
            norm = (0..n).map(|i| dot(&bstar[i], &bstar[i])).collect();
            k = if k >= 2 { k - 1 } else { 1 };
        }
    }
    b
}

/// LLL-reduce an integer lattice basis (the rows of `basis`), returning a reduced basis of the same
/// lattice with short, nearly-orthogonal vectors. Exact arithmetic throughout (Gram–Schmidt over the
/// rationals, size reduction with `|μ| ≤ ½`, the Lovász swap at `δ = ¾`), so the result is a certified
/// LLL-reduced basis. The first row is a short vector — for a lattice built to encode a cryptographic
/// weakness, that short vector is the leaked secret.
pub fn lll_reduce(basis: &[Vec<i64>]) -> Vec<Vec<BigInt>> {
    let b = basis.iter().map(|row| row.iter().map(|&x| Rational::from_i64(x)).collect()).collect();
    lll_core(b).iter().map(|row| row.iter().map(|r| r.round()).collect()).collect()
}

/// The bit length of `|x|`.
fn bit_len(x: &BigInt) -> usize {
    let (_, bytes) = x.to_le_bytes();
    for (i, &byte) in bytes.iter().enumerate().rev() {
        if byte != 0 {
            return i * 8 + (8 - byte.leading_zeros() as usize);
        }
    }
    0
}

/// Floating-point Gram–Schmidt of an EXACT integer basis, uniformly down-scaled so the doubles stay
/// finite (the scale cancels in `μ` and in the Lovász ratio, both scale-invariant). Returns `(μ, ‖b*‖²)`.
fn gram_schmidt_f64(b: &[Vec<BigInt>]) -> (Vec<Vec<f64>>, Vec<f64>) {
    let (n, dim) = (b.len(), b[0].len());
    let maxbits = b.iter().flatten().map(bit_len).max().unwrap_or(0);
    let shift = maxbits.saturating_sub(900);
    let mut scale = BigInt::from_i64(1);
    for _ in 0..shift {
        scale = scale.mul(&BigInt::from_i64(2));
    }
    let cf: Vec<Vec<f64>> = b
        .iter()
        .map(|row| row.iter().map(|x| Rational::new(x.clone(), scale.clone()).map(|r| r.to_f64()).unwrap_or(0.0)).collect())
        .collect();
    let mut bstar = vec![vec![0f64; dim]; n];
    let mut mu = vec![vec![0f64; n]; n];
    for i in 0..n {
        bstar[i].clone_from(&cf[i]);
        for j in 0..i {
            let dp: f64 = cf[i].iter().zip(&bstar[j]).map(|(a, c)| a * c).sum();
            let nj: f64 = bstar[j].iter().map(|c| c * c).sum();
            let m = if nj != 0.0 { dp / nj } else { 0.0 };
            mu[i][j] = m;
            for d in 0..dim {
                bstar[i][d] -= m * bstar[j][d];
            }
        }
    }
    let norm = bstar.iter().map(|v| v.iter().map(|c| c * c).sum()).collect();
    (mu, norm)
}

fn vdot(u: &[BigInt], v: &[BigInt]) -> BigInt {
    u.iter().zip(v).fold(BigInt::zero(), |a, (x, y)| a.add(&x.mul(y)))
}

// ---- BigFloat: fixed-precision binary floating point for the L² reduction (fpLLL) --------------------
//
// The f64 Gram–Schmidt loses precision on Boneh–Durfee's huge dynamic range; the exact integer LLL is
// correct but its sub-determinants grow to ~2^(dim·bits) and crawl. BigFloat splits the difference: a
// `BigInt` mantissa kept to a FIXED number of significant bits (`FP_PREC`) times `2^exp`. It behaves like
// floating point (fast, bounded size — no blow-up) but with enough bits to survive the dynamic range.

const FP_PREC: u64 = 2048;

fn bit_length(x: &BigInt) -> u64 {
    let (_, bytes) = x.to_le_bytes();
    for (i, &b) in bytes.iter().enumerate().rev() {
        if b != 0 {
            return i as u64 * 8 + (8 - b.leading_zeros() as u64);
        }
    }
    0
}

fn two_pow(k: u64) -> BigInt {
    BigInt::from_i64(2).pow(k as u32)
}

fn shl(x: &BigInt, k: u64) -> BigInt {
    if k == 0 {
        x.clone()
    } else {
        x.mul(&two_pow(k))
    }
}

fn shr(x: &BigInt, k: u64) -> BigInt {
    if k == 0 {
        x.clone()
    } else {
        x.div_rem(&two_pow(k)).expect("nonzero").0
    }
}

#[derive(Clone)]
struct BigFloat {
    m: BigInt,
    e: i64,
}

impl BigFloat {
    fn normalized(m: BigInt, e: i64) -> Self {
        if m.is_zero() {
            return Self { m, e: 0 };
        }
        let bl = bit_length(&m);
        if bl > FP_PREC {
            let s = bl - FP_PREC;
            Self { m: shr(&m, s), e: e + s as i64 }
        } else {
            Self { m, e }
        }
    }
    fn zero() -> Self {
        Self { m: BigInt::zero(), e: 0 }
    }
    fn from_bigint(n: &BigInt) -> Self {
        Self::normalized(n.clone(), 0)
    }
    fn from_i64(n: i64) -> Self {
        Self::from_bigint(&BigInt::from_i64(n))
    }
    fn add(&self, o: &Self) -> Self {
        if self.m.is_zero() {
            return o.clone();
        }
        if o.m.is_zero() {
            return self.clone();
        }
        // Align to the MINIMUM exponent (shift mantissas UP, losing nothing); if one operand is more than
        // FP_PREC bits smaller in magnitude, it is below precision and the larger one is returned.
        let mag_s = self.e + bit_length(&self.m) as i64;
        let mag_o = o.e + bit_length(&o.m) as i64;
        if mag_s > mag_o + FP_PREC as i64 + 8 {
            return self.clone();
        }
        if mag_o > mag_s + FP_PREC as i64 + 8 {
            return o.clone();
        }
        let e = self.e.min(o.e);
        let ma = shl(&self.m, (self.e - e) as u64);
        let mb = shl(&o.m, (o.e - e) as u64);
        Self::normalized(ma.add(&mb), e)
    }
    fn neg(&self) -> Self {
        Self { m: self.m.negated(), e: self.e }
    }
    fn sub(&self, o: &Self) -> Self {
        self.add(&o.neg())
    }
    fn mul(&self, o: &Self) -> Self {
        Self::normalized(self.m.mul(&o.m), self.e + o.e)
    }
    fn div(&self, o: &Self) -> Self {
        let num = shl(&self.m, FP_PREC);
        Self::normalized(num.div_rem(&o.m).expect("nonzero div").0, self.e - o.e - FP_PREC as i64)
    }
    fn abs(&self) -> Self {
        Self { m: self.m.abs(), e: self.e }
    }
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        let d = self.sub(o).m;
        if d.is_zero() {
            std::cmp::Ordering::Equal
        } else if d.is_negative() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    }
    /// Nearest integer (ties away from zero).
    fn round(&self) -> BigInt {
        if self.e >= 0 {
            shl(&self.m, self.e as u64)
        } else {
            let k = (-self.e) as u64;
            let half = two_pow(k - 1);
            let adj = if self.m.is_negative() { self.m.sub(&half) } else { self.m.add(&half) };
            adj.div_rem(&two_pow(k)).expect("nonzero").0
        }
    }
}

fn dot_bf(u: &[BigFloat], v: &[BigFloat]) -> BigFloat {
    u.iter().zip(v).fold(BigFloat::zero(), |a, (x, y)| a.add(&x.mul(y)))
}

fn gram_schmidt_bf(b: &[Vec<BigInt>]) -> (Vec<Vec<BigFloat>>, Vec<BigFloat>) {
    let (n, dim) = (b.len(), b[0].len());
    let cf: Vec<Vec<BigFloat>> = b.iter().map(|r| r.iter().map(BigFloat::from_bigint).collect()).collect();
    let mut bstar = vec![vec![BigFloat::zero(); dim]; n];
    let mut mu = vec![vec![BigFloat::zero(); n]; n];
    for i in 0..n {
        bstar[i].clone_from(&cf[i]);
        for j in 0..i {
            let m = dot_bf(&cf[i], &bstar[j]).div(&dot_bf(&bstar[j], &bstar[j]));
            for d in 0..dim {
                bstar[i][d] = bstar[i][d].sub(&m.mul(&bstar[j][d]));
            }
            mu[i][j] = m;
        }
    }
    let norm = bstar.iter().map(|v| dot_bf(v, v)).collect();
    (mu, norm)
}

/// Incrementally update the Gram–Schmidt data `(mu, norm)` when swapping basis rows `k` and `k−1` — the
/// standard LLL swap formulas, so a swap is `O(n)` rather than a full `O(n²·dim)` recomputation.
fn swap_gso(k: usize, n: usize, b: &mut [Vec<BigInt>], mu: &mut [Vec<BigFloat>], norm: &mut [BigFloat]) {
    b.swap(k, k - 1);
    let nu = mu[k][k - 1].clone();
    let db = norm[k].add(&nu.mul(&nu).mul(&norm[k - 1])); // new ‖b*_{k-1}‖²
    let new_mu = nu.mul(&norm[k - 1]).div(&db);
    let new_norm_k = norm[k - 1].mul(&norm[k]).div(&db);
    {
        let (lower, upper) = mu.split_at_mut(k);
        let (row_km1, row_k) = (&mut lower[k - 1], &mut upper[0]);
        for j in 0..(k - 1) {
            std::mem::swap(&mut row_km1[j], &mut row_k[j]);
        }
    }
    mu[k][k - 1] = new_mu.clone();
    norm[k] = new_norm_k;
    norm[k - 1] = db;
    for i in (k + 1)..n {
        let t = mu[i][k].clone();
        mu[i][k] = mu[i][k - 1].sub(&nu.mul(&t));
        mu[i][k - 1] = t.add(&new_mu.mul(&mu[i][k]));
    }
}

/// The L² lattice reduction (fpLLL): LLL with a BigFloat Gram–Schmidt — fast like floating point, but
/// with `FP_PREC` bits of precision so it survives the high dynamic range of Coppersmith/Boneh–Durfee
/// lattices where f64 fails, and without the determinant blow-up of the exact integer LLL. The basis
/// stays exact integer; only the steering is in BigFloat, and swaps update the Gram–Schmidt incrementally
/// (`O(n³)` overall rather than the `O(n⁵)` of recompute-on-swap).
pub fn lll_reduce_bigint_fp(basis: &[Vec<BigInt>]) -> Vec<Vec<BigInt>> {
    let n = basis.len();
    if n < 2 {
        return basis.to_vec();
    }
    let dim = basis[0].len();
    let mut b = basis.to_vec();
    let delta = BigFloat::from_i64(3).div(&BigFloat::from_i64(4));
    let half = BigFloat::from_i64(1).div(&BigFloat::from_i64(2));
    let (mut mu, mut norm) = gram_schmidt_bf(&b);
    let max_bits = b.iter().flatten().map(bit_length).max().unwrap_or(1) as usize;
    let cap = (max_bits + 16) * n * n * 4;
    let mut k = 1;
    let mut guard = 0usize;
    while k < n && guard < cap {
        guard += 1;
        for j in (0..k).rev() {
            if mu[k][j].abs().cmp(&half) == std::cmp::Ordering::Greater {
                let q = mu[k][j].round();
                if !q.is_zero() {
                    let qf = BigFloat::from_bigint(&q);
                    for d in 0..dim {
                        b[k][d] = b[k][d].sub(&q.mul(&b[j][d]));
                    }
                    mu[k][j] = mu[k][j].sub(&qf);
                    for l in 0..j {
                        mu[k][l] = mu[k][l].sub(&qf.mul(&mu[j][l]));
                    }
                }
            }
        }
        let rhs = delta.sub(&mu[k][k - 1].mul(&mu[k][k - 1])).mul(&norm[k - 1]);
        if norm[k].cmp(&rhs) != std::cmp::Ordering::Less {
            k += 1;
        } else {
            swap_gso(k, n, &mut b, &mut mu, &mut norm);
            k = if k >= 2 { k - 1 } else { 1 };
        }
    }
    b
}

/// Nearest integer to `a/b` for `b > 0`.
fn round_div(a: &BigInt, b: &BigInt) -> BigInt {
    let (q, r) = a.div_rem(b).expect("nonzero");
    if BigInt::from_i64(2).mul(&r.abs()) > *b {
        if a.is_negative() {
            q.sub(&BigInt::from_i64(1))
        } else {
            q.add(&BigInt::from_i64(1))
        }
    } else {
        q
    }
}

fn redi(k: usize, l: usize, b: &mut [Vec<BigInt>], d: &[BigInt], lam: &mut [Vec<BigInt>]) {
    if BigInt::from_i64(2).mul(&lam[k][l].abs()) <= d[l] {
        return;
    }
    let q = round_div(&lam[k][l], &d[l]);
    let bl = b[l].clone();
    for c in 0..bl.len() {
        b[k][c] = b[k][c].sub(&q.mul(&bl[c]));
    }
    lam[k][l] = lam[k][l].sub(&q.mul(&d[l]));
    let laml = lam[l].clone();
    for i in 1..l {
        lam[k][i] = lam[k][i].sub(&q.mul(&laml[i]));
    }
}

fn swapi(k: usize, n: usize, b: &mut [Vec<BigInt>], d: &mut [BigInt], lam: &mut [Vec<BigInt>]) {
    b.swap(k, k - 1);
    for j in 1..=k.saturating_sub(2) {
        let t = lam[k][j].clone();
        lam[k][j] = lam[k - 1][j].clone();
        lam[k - 1][j] = t;
    }
    let lambda = lam[k][k - 1].clone();
    let bval = d[k - 2].mul(&d[k]).add(&lambda.mul(&lambda)).div_rem(&d[k - 1]).expect("exact").0;
    for i in (k + 1)..=n {
        let t = lam[i][k].clone();
        let new_ik = d[k].mul(&lam[i][k - 1]).sub(&lambda.mul(&t)).div_rem(&d[k - 1]).expect("exact").0;
        let new_ik1 = bval.mul(&t).add(&lambda.mul(&new_ik)).div_rem(&d[k]).expect("exact").0;
        lam[i][k] = new_ik;
        lam[i][k - 1] = new_ik1;
    }
    d[k - 1] = bval;
}

/// Exact integer LLL (Cohen, *A Course in Computational Algebraic Number Theory*, Algorithm 2.6.3): the
/// Gram–Schmidt quantities are tracked as EXACT integers (the sub-determinants `dᵢ` and the scaled
/// coefficients `λᵢⱼ = dⱼ·μᵢⱼ`), with Bareiss-style exact division — no floating point (so no precision
/// loss on high-dynamic-range lattices) and no rationals (so no coefficient blow-up). This is the
/// trustworthy reduction the Boneh–Durfee independence diagnosis needs.
pub fn lll_reduce_bigint_exact(basis: &[Vec<BigInt>]) -> Vec<Vec<BigInt>> {
    let n = basis.len();
    if n < 2 {
        return basis.to_vec();
    }
    let mut b: Vec<Vec<BigInt>> = vec![Vec::new()]; // 1-indexed; b[0] is a dummy
    b.extend(basis.iter().cloned());
    let mut d = vec![BigInt::zero(); n + 2];
    d[0] = BigInt::from_i64(1);
    let mut lam = vec![vec![BigInt::zero(); n + 2]; n + 2];

    d[1] = vdot(&b[1], &b[1]);
    let mut k = 2;
    let mut kmax = 1;
    while k <= n {
        if k > kmax {
            kmax = k;
            for j in 1..=k {
                let mut u = vdot(&b[k], &b[j]);
                for i in 1..j {
                    u = d[i].mul(&u).sub(&lam[k][i].mul(&lam[j][i])).div_rem(&d[i - 1]).expect("exact").0;
                }
                if j < k {
                    lam[k][j] = u;
                } else {
                    d[k] = u;
                }
            }
        }
        redi(k, k - 1, &mut b, &d, &mut lam);
        let lhs = BigInt::from_i64(4).mul(&d[k]).mul(&d[k - 2]);
        let rhs = BigInt::from_i64(3)
            .mul(&d[k - 1].mul(&d[k - 1]))
            .sub(&BigInt::from_i64(4).mul(&lam[k][k - 1].mul(&lam[k][k - 1])));
        if lhs < rhs {
            swapi(k, n, &mut b, &mut d, &mut lam);
            k = if k - 1 >= 2 { k - 1 } else { 2 };
        } else {
            for l in (1..=k.saturating_sub(2)).rev() {
                redi(k, l, &mut b, &d, &mut lam);
            }
            k += 1;
        }
    }
    b.into_iter().skip(1).collect()
}

/// LLL-reduce a `BigInt` lattice basis — the entry point for lattices with huge entries, such as the
/// `N^m`-scaled rows of Coppersmith's method. The symmetry that makes this fast: only the BASIS need be
/// exact; the Gram–Schmidt that steers the size-reductions and Lovász swaps runs in floating point, so
/// there is no rational coefficient blow-up. The reduced basis is still exact integer, and correctness is
/// re-checked downstream (a Coppersmith root either divides `N` or it does not).
pub fn lll_reduce_bigint(basis: &[Vec<BigInt>]) -> Vec<Vec<BigInt>> {
    let n = basis.len();
    if n < 2 {
        return basis.to_vec();
    }
    let dim = basis[0].len();
    let mut b = basis.to_vec();
    let (mut mu, mut norm) = gram_schmidt_f64(&b);
    let mut k = 1;
    let mut guard = 0usize;
    let cap = 2000 * n * n; // precision backstop against float-induced non-termination
    while k < n && guard < cap {
        guard += 1;
        for j in (0..k).rev() {
            if mu[k][j].abs() > 0.5 {
                let q = mu[k][j].round();
                if q != 0.0 {
                    let qb = BigInt::parse_decimal(&format!("{q:.0}")).unwrap_or_else(BigInt::zero);
                    for d in 0..dim {
                        b[k][d] = b[k][d].sub(&qb.mul(&b[j][d]));
                    }
                    mu[k][j] -= q;
                    for l in 0..j {
                        mu[k][l] -= q * mu[j][l];
                    }
                }
            }
        }
        if norm[k] >= (0.75 - mu[k][k - 1] * mu[k][k - 1]) * norm[k - 1] {
            k += 1;
        } else {
            b.swap(k, k - 1);
            let (m, nn) = gram_schmidt_f64(&b);
            mu = m;
            norm = nn;
            k = if k >= 2 { k - 1 } else { 1 };
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_i64(v: &[Vec<BigInt>]) -> Vec<Vec<i64>> {
        v.iter().map(|row| row.iter().map(|x| x.to_i64().expect("small entry")).collect()).collect()
    }

    fn norm_sq(row: &[i64]) -> i64 {
        row.iter().map(|&x| x * x).sum()
    }

    #[test]
    fn lll_reduces_a_skewed_basis_to_the_standard_one() {
        // The lattice ℤ² given by a badly skewed basis: [10,1] is long, but 10·[1,0] shears it to [0,1].
        let reduced = as_i64(&lll_reduce(&[vec![1, 0], vec![10, 1]]));
        assert_eq!(reduced, vec![vec![1, 0], vec![0, 1]], "LLL recovers the standard basis");
    }

    #[test]
    fn lll_finds_the_shortest_vector_of_a_flat_lattice() {
        // det = 15·17 − 23·11 = 2, so this lattice is very flat and its shortest vector is tiny ([1,1],
        // norm² = 2); both input rows have norm² in the hundreds. LLL surfaces it.
        let reduced = as_i64(&lll_reduce(&[vec![15, 23], vec![11, 17]]));
        let shortest = reduced.iter().map(|r| norm_sq(r)).min().unwrap();
        assert_eq!(shortest, 2, "LLL finds the shortest vector, norm² = 2");
        assert!(reduced.iter().any(|r| r == &[1, 1] || r == &[-1, -1]), "and it is ±[1,1]");
    }

    #[test]
    fn exact_integer_lll_matches_the_known_reductions() {
        let big = |rows: &[&[i64]]| -> Vec<Vec<BigInt>> {
            rows.iter().map(|r| r.iter().map(|&x| BigInt::from_i64(x)).collect()).collect()
        };
        let out = as_i64(&lll_reduce_bigint_exact(&big(&[&[1, 0], &[10, 1]])));
        assert_eq!(out, vec![vec![1, 0], vec![0, 1]], "exact LLL recovers the standard basis");

        let out = as_i64(&lll_reduce_bigint_exact(&big(&[&[15, 23], &[11, 17]])));
        let shortest = out.iter().map(|r| norm_sq(r)).min().unwrap();
        assert_eq!(shortest, 2, "exact LLL finds the shortest vector, norm² = 2");
    }

    #[test]
    fn l2_fplll_matches_the_known_reductions() {
        let big = |rows: &[&[i64]]| -> Vec<Vec<BigInt>> {
            rows.iter().map(|r| r.iter().map(|&x| BigInt::from_i64(x)).collect()).collect()
        };
        let out = as_i64(&lll_reduce_bigint_fp(&big(&[&[1, 0], &[10, 1]])));
        assert_eq!(out, vec![vec![1, 0], vec![0, 1]], "L² recovers the standard basis");

        let out = as_i64(&lll_reduce_bigint_fp(&big(&[&[15, 23], &[11, 17]])));
        let shortest = out.iter().map(|r| norm_sq(r)).min().unwrap();
        assert_eq!(shortest, 2, "L² finds the shortest vector, norm² = 2");
    }

    #[test]
    fn lll_output_is_size_reduced() {
        // A 3-D lattice: after reduction every off-diagonal Gram–Schmidt coefficient satisfies |μ| ≤ ½.
        let reduced = lll_reduce(&[vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 10]]);
        let b: Vec<Vec<Rational>> =
            reduced.iter().map(|row| row.iter().map(|x| Rational::from_bigint(x.clone())).collect()).collect();
        let (_, mu) = gram_schmidt(&b);
        for i in 0..b.len() {
            for j in 0..i {
                assert!(mu[i][j].abs() <= rat(1, 2), "size-reduced: |μ[{i}][{j}]| ≤ ½");
            }
        }
    }
}
