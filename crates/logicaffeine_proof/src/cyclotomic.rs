//! # Power-of-two cyclotomic ring `R = ℤ[X]/(Xⁿ + 1)`, `n = 2ᵏ`
//!
//! When `n` is a power of two, `Xⁿ + 1 = Φ_{2n}(X)` is the `2n`-th cyclotomic polynomial, so `R` is the ring
//! of integers `𝒪_K` of the cyclotomic field `K = ℚ(ζ_{2n})` (`ζ_{2n}` a primitive `2n`-th root of unity,
//! played by `X`). This is the algebraic substrate of **Module-LWE** — ML-KEM/Kyber live over `R_q` with
//! `n = 256`. Multiplication is **negacyclic** (`Xⁿ ≡ −1`).
//!
//! The field's **Galois group** is `Gal(K/ℚ) ≅ (ℤ/2n)^×` — the ring automorphisms `σ_t : X ↦ X^t` for odd
//! `t`, of which there are exactly `φ(2n) = n`. This rigid, fully-known symmetry group is precisely what
//! structure-exploiting cryptanalysis (the log-unit lattice, the Principal Ideal Problem) rides on: two
//! generators of an ideal differ by a unit, and the units are governed by these automorphisms. It is the
//! lattice analogue of the `Aut(E₀)`-orbit collapse in the isogeny keyspace — **symmetry = compression**.
//!
//! This module builds the ring and its Galois action honestly. It does **not** break Module-LWE: it is the
//! substrate on which the (short-generator) attacks that *do* work are expressed, and the lens that measures
//! where they stop.

use logicaffeine_base::BigInt;
use std::f64::consts::PI;

#[inline]
fn zero() -> BigInt {
    BigInt::from_i64(0)
}

/// An element of `R = ℤ[X]/(Xⁿ + 1)`: the coefficient vector `[a₀, …, a_{n−1}]` of `Σ aᵢ Xⁱ`, with `n` a
/// power of two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cyclo {
    pub n: usize,
    pub coeffs: Vec<BigInt>,
}

impl Cyclo {
    /// An element from a coefficient vector (padded/verified to length `n`). `n` must be a power of two.
    pub fn new(n: usize, mut coeffs: Vec<BigInt>) -> Cyclo {
        assert!(
            n.is_power_of_two(),
            "R = ℤ[X]/(Xⁿ+1) requires n a power of two"
        );
        assert!(coeffs.len() <= n, "an element of R has degree < n");
        coeffs.resize(n, zero());
        Cyclo { n, coeffs }
    }

    /// Convenience constructor from small integers.
    pub fn from_ints(n: usize, v: &[i64]) -> Cyclo {
        Cyclo::new(n, v.iter().map(|&x| BigInt::from_i64(x)).collect())
    }

    pub fn zero(n: usize) -> Cyclo {
        Cyclo::new(n, vec![])
    }

    pub fn one(n: usize) -> Cyclo {
        Cyclo::from_ints(n, &[1])
    }

    /// `coeff · Xⁱ` reduced into `R` (any `i ≥ 0`): `Xⁱ = (−1)^{⌊i/n⌋} · X^{i mod n}`.
    pub fn monomial(n: usize, i: usize, coeff: BigInt) -> Cyclo {
        let mut c = vec![zero(); n];
        let signed = if (i / n) % 2 == 0 {
            coeff
        } else {
            zero().sub(&coeff)
        };
        c[i % n] = signed;
        Cyclo { n, coeffs: c }
    }

    pub fn add(&self, o: &Cyclo) -> Cyclo {
        Cyclo {
            n: self.n,
            coeffs: (0..self.n)
                .map(|i| self.coeffs[i].add(&o.coeffs[i]))
                .collect(),
        }
    }

    pub fn sub(&self, o: &Cyclo) -> Cyclo {
        Cyclo {
            n: self.n,
            coeffs: (0..self.n)
                .map(|i| self.coeffs[i].sub(&o.coeffs[i]))
                .collect(),
        }
    }

    pub fn neg(&self) -> Cyclo {
        Cyclo {
            n: self.n,
            coeffs: self.coeffs.iter().map(|c| zero().sub(c)).collect(),
        }
    }

    /// Negacyclic product: multiply as polynomials, then reduce `Xⁿ ≡ −1`.
    pub fn mul(&self, o: &Cyclo) -> Cyclo {
        let n = self.n;
        let mut c = vec![zero(); n];
        for i in 0..n {
            if self.coeffs[i].is_zero() {
                continue;
            }
            for j in 0..n {
                let prod = self.coeffs[i].mul(&o.coeffs[j]);
                let k = i + j;
                if k < n {
                    c[k] = c[k].add(&prod); // Xᵏ, k < n
                } else {
                    c[k - n] = c[k - n].sub(&prod); // Xᵏ = −X^{k−n}
                }
            }
        }
        Cyclo { n, coeffs: c }
    }

    /// The Galois automorphism `σ_t : X ↦ X^t` (`t` odd, i.e. a unit of `ℤ/2n`). On monomials
    /// `σ_t(Xⁱ) = X^{it}`, reduced by `Xⁿ ≡ −1`. A ring automorphism of `R`.
    pub fn galois(&self, t: u64) -> Cyclo {
        let n = self.n;
        let twon = 2 * n as u64;
        let t = t % twon;
        assert!(
            t % 2 == 1,
            "Galois automorphisms σ_t require t odd (a unit mod 2n)"
        );
        let mut c = vec![zero(); n];
        for i in 0..n {
            let m = ((i as u64) * t) % twon;
            let (idx, neg) = if (m as usize) < n {
                (m as usize, false)
            } else {
                (m as usize - n, true)
            };
            c[idx] = if neg {
                c[idx].sub(&self.coeffs[i])
            } else {
                c[idx].add(&self.coeffs[i])
            };
        }
        Cyclo { n, coeffs: c }
    }

    /// Complex conjugation `σ_{−1} : X ↦ X^{−1} = X^{2n−1}` — the order-two element of the Galois group.
    pub fn conjugate(&self) -> Cyclo {
        self.galois((2 * self.n - 1) as u64)
    }

    /// The product `∏_{σ ∈ Gal} σ(a)` as a ring element — Galois-invariant, hence a rational integer times
    /// `1`. Used by [`norm`](Cyclo::norm); exposed so callers can confirm the higher coefficients vanish.
    fn galois_product(&self) -> Cyclo {
        galois_group(self.n)
            .into_iter()
            .fold(Cyclo::one(self.n), |acc, t| acc.mul(&self.galois(t)))
    }

    /// The field **norm** `N(a) = ∏_{σ ∈ Gal} σ(a) ∈ ℤ` — multiplicative, and `±1` exactly on the units.
    pub fn norm(&self) -> BigInt {
        self.galois_product().coeffs[0].clone()
    }

    /// The field **trace** `Tr(a) = Σ_{σ ∈ Gal} σ(a) ∈ ℤ` — additive; `Tr(1) = n`.
    pub fn trace(&self) -> BigInt {
        let sum = galois_group(self.n)
            .into_iter()
            .fold(Cyclo::zero(self.n), |acc, t| acc.add(&self.galois(t)));
        sum.coeffs[0].clone()
    }

    /// Whether `a` is a unit of `R` — equivalently `N(a) = ±1`.
    pub fn is_unit(&self) -> bool {
        let nrm = self.norm();
        nrm == BigInt::from_i64(1) || nrm == BigInt::from_i64(-1)
    }

    /// The exact ring inverse of a **unit**, with no field division: since `N(u) = u·∏_{t≠1}σ_t(u) = ±1`, we
    /// have `u⁻¹ = N(u)·∏_{t≠1}σ_t(u)`. `None` for a non-unit.
    pub fn unit_inverse(&self) -> Option<Cyclo> {
        let nrm = self.norm();
        let one = BigInt::from_i64(1);
        let neg = BigInt::from_i64(-1);
        let sign_neg = if nrm == one {
            false
        } else if nrm == neg {
            true
        } else {
            return None;
        };
        let prod = galois_group(self.n)
            .into_iter()
            .filter(|&t| t != 1)
            .fold(Cyclo::one(self.n), |acc, t| acc.mul(&self.galois(t)));
        Some(if sign_neg { prod.neg() } else { prod })
    }

    /// The sum of absolute values of the coefficients — a coarse length used to recognize a *short* generator.
    pub fn coeff_norm(&self) -> i64 {
        self.coeffs
            .iter()
            .map(|c| c.to_i64().unwrap_or(i64::MAX / 2).abs())
            .sum()
    }
}

// ---- The log-unit lattice and CDPR short-generator recovery -----------------------------------------
//
// The canonical embedding sends a ∈ K to (a(ζ^j))_{j odd} ∈ ℂⁿ (the n complex places), and the log embedding
// to (log|a(ζ^j)|)_j ∈ ℝⁿ. The units map to a lattice Λ = Log(units); the cyclotomic units b_j = (Xʲ−1)/(X−1)
// give an explicit basis. A short generator g of a principal ideal sits *close to the origin* in this picture,
// so a generator h = g·u differs from it by the lattice vector Log(u); rounding Log(h) into Λ strips the unit.
// This is the AutOrbitClosure move — quotient out the units, collapse the space — for structured lattices.

/// `a(ζ^j)` in ℂ, `ζ = exp(2πi/2n)` — the `j`-th canonical embedding.
fn embed_at(a: &Cyclo, j: usize) -> (f64, f64) {
    let twon = 2.0 * a.n as f64;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, c) in a.coeffs.iter().enumerate() {
        let ci = c.to_i64().expect("embedding assumes coefficients fit i64") as f64;
        let ang = 2.0 * PI * (j as f64) * (i as f64) / twon;
        re += ci * ang.cos();
        im += ci * ang.sin();
    }
    (re, im)
}

/// The log embedding `Log(a) = (log|a(ζ^j)|)_{j odd} ∈ ℝⁿ`.
fn log_embedding(a: &Cyclo) -> Vec<f64> {
    (1..2 * a.n)
        .step_by(2)
        .map(|j| {
            let (re, im) = embed_at(a, j);
            0.5 * (re * re + im * im).ln()
        })
        .collect()
}

fn dot(u: &[f64], v: &[f64]) -> f64 {
    u.iter().zip(v).map(|(a, b)| a * b).sum()
}

/// Solve `A x = b` (small dense system) by Gaussian elimination with partial pivoting. `None` if singular.
fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let piv = (col..n).max_by(|&r1, &r2| {
            a[r1][col]
                .abs()
                .partial_cmp(&a[r2][col].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if a[piv][col].abs() < 1e-9 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r][col] / a[col][col];
            for k in col..n {
                a[r][k] -= f * a[col][k];
            }
            b[r] -= f * b[col];
        }
    }
    Some((0..n).map(|i| b[i] / a[i][i]).collect())
}

/// The cyclotomic units `b_j = 1 + X + ⋯ + X^{j−1} = (Xʲ−1)/(X−1)` for `j` an odd residue in `[3, n−1]`. These
/// are `n/2 − 1` independent units — a basis of a finite-index subgroup of the unit group, and the basis of
/// the log-unit lattice used by CDPR.
pub fn cyclotomic_units(n: usize) -> Vec<Cyclo> {
    (3..n)
        .step_by(2)
        .map(|j| Cyclo::from_ints(n, &vec![1i64; j]))
        .collect()
}

/// **CDPR short-generator recovery.** Given a generator `h` of a principal ideal that has an *unusually short*
/// generator `g` (so `h = g·u` for a unit `u`), recover a short generator: project `Log(h)` onto the log-unit
/// lattice against the cyclotomic-unit basis to strip the unit, divide it out **exactly** (via the
/// Galois-conjugate inverse), and pick the exact generator of smallest coefficient-norm over a local search
/// around the rounded lattice point. Returns `g` up to a root of unity `±Xᵐ`. This is the genuine break on
/// schemes that expose a short principal-ideal generator (Soliloquy, Smart–Vercauteren); it does **not** touch
/// Module-LWE, where the secret is not such a generator.
pub fn recover_short_generator(h: &Cyclo) -> Option<Cyclo> {
    let n = h.n;
    let units = cyclotomic_units(n);
    let r = units.len();
    let blogs: Vec<Vec<f64>> = units.iter().map(log_embedding).collect();
    let target = log_embedding(h);

    // Least-squares coordinates of Log(h) in the unit-log basis: (BᵀB) c = Bᵀ Log(h).
    let mut btb = vec![vec![0.0; r]; r];
    let mut bt = vec![0.0; r];
    for i in 0..r {
        for k in 0..r {
            btb[i][k] = dot(&blogs[i], &blogs[k]);
        }
        bt[i] = dot(&blogs[i], &target);
    }
    let c = solve_linear(btb, bt)?;
    let base: Vec<i64> = c.iter().map(|&x| x.round() as i64).collect();

    // Local search over the rounded exponents: the true short generator is the exact divisor of least norm.
    let inverses: Vec<Cyclo> = units.iter().filter_map(|u| u.unit_inverse()).collect();
    if inverses.len() != r {
        return None;
    }
    let mut best: Option<(i64, Cyclo)> = None;
    for mask in 0..3usize.pow(r as u32) {
        let mut m = mask;
        let mut uprime = Cyclo::one(n);
        for i in 0..r {
            let e = base[i] + (m % 3) as i64 - 1;
            m /= 3;
            let base_elt = if e >= 0 { &units[i] } else { &inverses[i] };
            for _ in 0..e.unsigned_abs() {
                uprime = uprime.mul(base_elt);
            }
        }
        let Some(uinv) = uprime.unit_inverse() else {
            continue;
        };
        let g = h.mul(&uinv);
        let norm = g.coeff_norm();
        if best.as_ref().is_none_or(|(bn, _)| norm < *bn) {
            best = Some((norm, g));
        }
    }
    best.map(|(_, g)| g)
}

/// Gram–Schmidt orthogonalization of a set of `ℝ^m` vectors (returns the orthogonal `bᵢ*`).
fn gram_schmidt(vs: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let mut out: Vec<Vec<f64>> = Vec::new();
    for v in vs {
        let mut u = v.clone();
        for w in &out {
            let coef = dot(v, w) / dot(w, w);
            for j in 0..u.len() {
                u[j] -= coef * w[j];
            }
        }
        out.push(u);
    }
    out
}

/// An audit of the **log-unit lattice** geometry `Λ = Log(units)` that governs CDPR recovery — the decoding
/// scale of the symmetry-quotient.
#[derive(Clone, Debug)]
pub struct LogUnitAudit {
    pub n: usize,
    /// The unit rank `r₁ + r₂ − 1 = n/2 − 1` (Dirichlet).
    pub rank: usize,
    /// Gram–Schmidt lengths `‖bᵢ*‖` of the cyclotomic-unit log-basis.
    pub gs_lengths: Vec<f64>,
    /// Covering-radius upper bound `μ ≤ ½·(Σ‖bᵢ*‖²)^½` — the scale below which a short generator is
    /// decodable by rounding in `Λ`.
    pub covering_radius_bound: f64,
}

/// Measure the log-unit lattice geometry for `R = ℤ[X]/(Xⁿ+1)`.
pub fn audit_log_unit(n: usize) -> LogUnitAudit {
    let units = cyclotomic_units(n);
    let blogs: Vec<Vec<f64>> = units.iter().map(log_embedding).collect();
    let gs = gram_schmidt(&blogs);
    let gs_lengths: Vec<f64> = gs.iter().map(|v| dot(v, v).sqrt()).collect();
    let cov = 0.5 * gs_lengths.iter().map(|l| l * l).sum::<f64>().sqrt();
    LogUnitAudit {
        n,
        rank: units.len(),
        gs_lengths,
        covering_radius_bound: cov,
    }
}

/// The **CDPR recovery margin** of a candidate generator `g`: `‖proj_{span Λ} Log(g)‖ / μ(Λ)`. A generator
/// whose log projects to well within the covering radius (`margin < 1`) is decodable by stripping the unit;
/// `margin ≫ 1` is beyond the decoding region. This measures *where the short-generator wall is* — and,
/// honestly, it is **small even for short (ML-KEM-shaped) secrets**, showing the log-unit collapse is *not*
/// where Module-LWE's hardness lives: that sits upstream (no principal-ideal-generator to hand the attacker,
/// module rank ≥ 2, and the `2^Õ(√n)` Ideal-SVP approximation gap).
pub fn recovery_margin(g: &Cyclo) -> f64 {
    let n = g.n;
    let blogs: Vec<Vec<f64>> = cyclotomic_units(n).iter().map(log_embedding).collect();
    let r = blogs.len();
    let lg = log_embedding(g);
    let mut btb = vec![vec![0.0; r]; r];
    let mut bt = vec![0.0; r];
    for i in 0..r {
        for k in 0..r {
            btb[i][k] = dot(&blogs[i], &blogs[k]);
        }
        bt[i] = dot(&blogs[i], &lg);
    }
    let Some(c) = solve_linear(btb, bt) else {
        return f64::INFINITY;
    };
    let mut proj = vec![0.0; n];
    for i in 0..r {
        for (j, pj) in proj.iter_mut().enumerate() {
            *pj += c[i] * blogs[i][j];
        }
    }
    dot(&proj, &proj).sqrt() / audit_log_unit(n).covering_radius_bound
}

/// **The upstream wall, measured.** The approximation scale of the log-unit collapse at dimension `n`: the
/// covering radius `μ(Λ)` against the shortest basis log-length — a proxy for the factor by which a
/// log-unit-decoded generator can exceed the true shortest vector. To *threaten* Module-LWE this factor would
/// have to stay **polynomial**; instead it grows super-polynomially with `n` (asymptotically `2^{Õ(√n)}`,
/// Cramer–Ducas–Wesolowski 2017). This is the real wall — not the symmetry-collapse rung (which is cheap, as
/// [`recovery_margin`] shows) but the *quality* of what the collapse can decode as the field grows.
pub fn approximation_scale(n: usize) -> f64 {
    let a = audit_log_unit(n);
    let shortest = a.gs_lengths.iter().cloned().fold(f64::INFINITY, f64::min);
    a.covering_radius_bound / shortest
}

/// The `n = φ(2n)` Galois automorphisms of `R = ℤ[X]/(Xⁿ+1)`, as the odd residues mod `2n` that index them.
pub fn galois_group(n: usize) -> Vec<u64> {
    (1..(2 * n as u64)).step_by(2).collect()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/cyclotomic/tests.rs"]
mod tests;
