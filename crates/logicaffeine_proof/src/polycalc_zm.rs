//! Nullstellensatz over the **rings `ℤ/m`** — composite moduli, zero divisors and all, at arbitrary
//! degree. The characteristic axis ([`crate::polycalc_gfp`]) covered every *field*; this module covers
//! what is left of "GF(N) for any N": the moduli where no field exists (`ℤ/6`) and the prime powers
//! where the ring is not the field (`ℤ/4`, with its nilpotent `2`).
//!
//! Three facts shape the module:
//!
//! - **Completeness needs no division.** The partition-of-unity construction — the engine of "no
//!   finite formula is structureless" — only ever adds, subtracts, and multiplies: the atom
//!   `(1 − x) + x = 1` is a ring identity, the signed point indicators cancel by additive inverses,
//!   and multilinear representations of cube functions are unique over any commutative ring (the
//!   Möbius transform is unipotent). So **every UNSAT formula has a degree-`≤ n` certificate over
//!   every `ℤ/m`, `m ≥ 2`** — hardness-as-structurelessness has *no witness at any finite `n` over
//!   any modulus*. The honest boundary is unchanged: the certificate lives in the `2ⁿ` basis, and the
//!   asymptotic cost-growth statements (the certified degree lower bounds) are exactly the results
//!   pointing the other way.
//! - **Refutation needs real ring linear algebra.** Gaussian elimination dies without inverses, so
//!   span membership over `ℤ/m` is decided by a Howell-style echelon: gcd pivoting (every pivot
//!   normalized, by a unit, to a divisor of `m`) plus **annihilator completion** (each pivot row
//!   spawns its `(m/d)`-multiple, which lives strictly below it) — validated against an exhaustive
//!   all-combinations oracle on small systems and against the field engine at prime `m`.
//! - **Composite moduli intersect, they do not add.** For squarefree `m`, CRT splits the coefficient
//!   ring, so a `ℤ/6` refutation is exactly a `GF(2)` refutation *and* a `GF(3)` refutation at the
//!   same degree — the composite ring is the *conjunction* of its prime parts, weaker than either.
//!   Dual witnesses generalize with the normalization `L(1) ≠ 0` (a zero divisor is fine): a prime
//!   witness lifts to a ring witness by scaling with `m/p`.

use crate::cdcl::Lit;
pub use crate::polycalc::Mono;
use std::collections::BTreeMap;

/// A multilinear polynomial over `ℤ/m`: monomial → nonzero coefficient in `1..m` (absent = 0).
pub type ZmPoly = BTreeMap<Mono, u64>;

#[inline]
fn zm_add(m: u64, a: u64, b: u64) -> u64 {
    (a % m + b % m) % m
}
#[inline]
fn zm_neg(m: u64, a: u64) -> u64 {
    (m - a % m) % m
}
#[inline]
fn zm_mul(m: u64, a: u64, b: u64) -> u64 {
    (a % m) * (b % m) % m
}

fn gcd(a: u64, b: u64) -> u64 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// Extended gcd over the integers: `(g, x, y)` with `x·a + y·b = g = gcd(a, b)`.
fn egcd(a: i128, b: i128) -> (i128, i128, i128) {
    if b == 0 {
        (a, 1, 0)
    } else {
        let (g, x, y) = egcd(b, a % b);
        (g, y, x - (a / b) * y)
    }
}

/// A **unit** `u` of `ℤ/m` with `u·a ≡ gcd(a, m) (mod m)` — the scaling that normalizes a pivot to a
/// divisor of `m` without leaving the ring's unit group (the standard Howell "stabilization"). For
/// nonzero `a mod m`: write `d = gcd(a, m)`; then `a/d` is invertible mod `m/d`, and some lift
/// `u ≡ (a/d)⁻¹ (mod m/d)` coprime to `m` exists among `u₀ + k·(m/d)`, `k < d`.
fn unit_scaling(a: u64, m: u64) -> u64 {
    let a = a % m;
    debug_assert!(a != 0, "the zero pivot has no scaling");
    let d = gcd(a, m);
    let (ap, mp) = (a / d, m / d);
    if mp == 1 {
        return 1; // a ≡ 0 case is excluded; mp = 1 cannot occur for nonzero a < m
    }
    let (_, x, _) = egcd(ap as i128, mp as i128);
    let u0 = ((x % mp as i128 + mp as i128) % mp as i128) as u64;
    for k in 0..d {
        let u = u0 + k * mp;
        if u != 0 && gcd(u, m) == 1 {
            return u;
        }
    }
    unreachable!("a unit lift of (a/d)⁻¹ mod m/d always exists below m")
}

fn add_term(m: u64, p: &mut ZmPoly, mono: Mono, c: u64) {
    let c = c % m;
    if c == 0 {
        return;
    }
    let e = p.entry(mono).or_insert(0);
    *e = zm_add(m, *e, c);
    if *e == 0 {
        p.remove(&mono);
    }
}

fn poly_mul(m: u64, a: &ZmPoly, b: &ZmPoly) -> ZmPoly {
    let mut r = ZmPoly::new();
    for (&ma, &ca) in a {
        for (&mb, &cb) in b {
            add_term(m, &mut r, ma | mb, zm_mul(m, ca, cb));
        }
    }
    r
}

fn poly_mul_mono(m: u64, p: &ZmPoly, mono: Mono) -> ZmPoly {
    let mut r = ZmPoly::new();
    for (&t, &c) in p {
        add_term(m, &mut r, t | mono, c);
    }
    r
}

/// The degree of a multilinear `ℤ/m` polynomial (0 for the zero polynomial).
pub fn zm_poly_degree(p: &ZmPoly) -> usize {
    p.keys().map(|mo| mo.count_ones() as usize).max().unwrap_or(0)
}

/// The clause polynomial over `ℤ/m` — the signed false-indicator, exactly as over the fields: a
/// positive literal contributes `1 − x` (over `ℤ/6`: `1 + 5x`), a negative one contributes `x`; the
/// product is `1` on falsifying corners and `0` elsewhere over every commutative ring.
pub fn clause_polynomial_zm(m: u64, clause: &[Lit]) -> ZmPoly {
    let mut p: ZmPoly = [(0u64, 1u64)].into_iter().collect();
    for l in clause {
        let bit = 1u64 << l.var();
        let indicator: ZmPoly = if l.is_positive() {
            [(0u64, 1u64), (bit, zm_neg(m, 1))].into_iter().collect()
        } else {
            [(bit, 1u64)].into_iter().collect()
        };
        p = poly_mul(m, &p, &indicator);
    }
    p
}

/// The signed point indicator `δ_a` over `ℤ/m` — only addition and negation, so it is ring-generic.
fn point_indicator_zm(m: u64, a: u64, num_vars: usize) -> ZmPoly {
    let mask = (1u64 << num_vars).wrapping_sub(1);
    let ones = a & mask;
    let zeros = !a & mask;
    let mut p = ZmPoly::new();
    let mut sub = zeros;
    loop {
        let sign = if sub.count_ones() % 2 == 0 { 1 } else { zm_neg(m, 1) };
        p.insert(ones | sub, sign);
        if sub == 0 {
            break;
        }
        sub = (sub - 1) & zeros;
    }
    p
}

/// The partition-of-unity atom `(1 − x_v) + x_v` — the constant `1` in **every commutative ring**,
/// zero divisors notwithstanding: the identity needs only additive inverses.
pub fn pou_atom_zm(m: u64, v: usize) -> ZmPoly {
    let mut atom: ZmPoly = [(0u64, 1u64), (1u64 << v, zm_neg(m, 1))].into_iter().collect();
    add_term(m, &mut atom, 1u64 << v, 1);
    atom
}

/// The partition of unity `Σ_a δ_a` over `ℤ/m` — the constant `1` at every `n` over every modulus.
pub fn partition_of_unity_zm(m: u64, n: usize) -> ZmPoly {
    let mut sum = ZmPoly::new();
    for a in 0..(1u64 << n) {
        for (mo, c) in point_indicator_zm(m, a, n) {
            add_term(m, &mut sum, mo, c);
        }
    }
    sum
}

/// A constructive Nullstellensatz certificate over `ℤ/m`: `Σ_C p_C · g_C = 1` in the multilinear ring.
#[derive(Clone, Debug)]
pub struct NsCertificateZm {
    modulus: u64,
    num_vars: usize,
    coeffs: Vec<ZmPoly>,
}

impl NsCertificateZm {
    pub fn modulus(&self) -> u64 {
        self.modulus
    }

    pub fn num_vars(&self) -> usize {
        self.num_vars
    }

    /// The maximum monomial degree among the coefficient polynomials.
    pub fn degree(&self) -> usize {
        self.coeffs.iter().map(zm_poly_degree).max().unwrap_or(0)
    }

    /// The certificate's SIZE: the total nonzero-monomial count across the coefficient
    /// polynomials — the honest measure of the `2ⁿ`-basis cost the existence pole pays.
    pub fn coeff_monomial_count(&self) -> usize {
        self.coeffs.iter().map(|g| g.len()).sum()
    }

    /// **Re-check against the original clauses** (zero trust), twice: `Σ_C p_C · g_C = 1` recomputed in
    /// the multilinear ring over `ℤ/m`, then the engine-independent corner evaluation (`≤ 20` vars).
    /// Fails closed on a clause-count mismatch.
    pub fn verify(&self, clauses: &[Vec<Lit>]) -> bool {
        if self.coeffs.len() != clauses.len() {
            return false;
        }
        let m = self.modulus;
        let mut sum = ZmPoly::new();
        for (c, g) in clauses.iter().zip(&self.coeffs) {
            if g.is_empty() {
                continue;
            }
            for (mo, co) in poly_mul(m, &clause_polynomial_zm(m, c), g) {
                add_term(m, &mut sum, mo, co);
            }
        }
        if !(sum.len() == 1 && sum.get(&0u64) == Some(&1)) {
            return false;
        }
        if self.num_vars <= 20 {
            let eval = |p: &ZmPoly, a: u64| -> u64 {
                p.iter()
                    .fold(0u64, |acc, (&mo, &c)| if mo & !a == 0 { zm_add(m, acc, c) } else { acc })
            };
            for a in 0u64..(1u64 << self.num_vars) {
                let total = clauses.iter().zip(&self.coeffs).fold(0u64, |acc, (c, g)| {
                    zm_add(m, acc, zm_mul(m, eval(&clause_polynomial_zm(m, c), a), eval(g, a)))
                });
                if total != 1 {
                    return false;
                }
            }
        }
        true
    }
}

/// **The uniform completeness construction over `ℤ/m`** — partition-of-unity charging, valid over any
/// commutative ring because it never divides. Returns a constructive certificate proving UNSAT or a
/// satisfying assignment proving SAT: a *total, certifying* decision over every modulus `m ≥ 2` — so
/// "structureless" (no certificate at any degree `≤ n`) has **no witness among finite formulas over
/// any `ℤ/m`**. The honest cost is unchanged: the certificate lives in the `2ⁿ` monomial basis
/// (existence, not efficiency), and `num_vars ≤ 20` bounds the explicit-corner construction.
pub fn build_ns_certificate_zm(
    m: u64,
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Result<NsCertificateZm, Vec<bool>> {
    assert!(m >= 2, "a modulus needs at least two residues");
    assert!(num_vars <= 20, "the explicit-corner construction is bounded to num_vars ≤ 20");
    let mut coeffs: Vec<ZmPoly> = vec![ZmPoly::new(); clauses.len()];
    for a in 0u64..(1u64 << num_vars) {
        let sel = clauses
            .iter()
            .position(|c| !c.iter().any(|l| ((a >> l.var()) & 1 == 1) == l.is_positive()));
        match sel {
            None => return Err((0..num_vars).map(|i| (a >> i) & 1 == 1).collect()),
            Some(ci) => {
                for (mo, c) in point_indicator_zm(m, a, num_vars) {
                    add_term(m, &mut coeffs[ci], mo, c);
                }
            }
        }
    }
    Ok(NsCertificateZm { modulus: m, num_vars, coeffs })
}

/// A **Howell-style echelon over `ℤ/m`** — the ring replacement for Gaussian elimination. Invariants:
/// every stored pivot row is unit-normalized so its leading entry is a **divisor of `m`**, and every
/// pivot's **annihilator multiple** `(m/d)·row` (which vanishes at the pivot column) is recursively
/// inserted, so successive leading-column reduction decides span membership exactly — including the
/// zero-divisor moves Gaussian elimination cannot make.
struct ZmEchelon {
    m: u64,
    pivots: std::collections::HashMap<usize, Vec<u64>>,
}

fn row_lead(row: &[u64]) -> Option<usize> {
    row.iter().rposition(|&c| c != 0)
}

impl ZmEchelon {
    fn new(m: u64) -> Self {
        ZmEchelon { m, pivots: std::collections::HashMap::new() }
    }

    fn insert(&mut self, mut row: Vec<u64>) {
        let m = self.m;
        loop {
            let Some(c) = row_lead(&row) else { return };
            let Some(pivot) = self.pivots.get(&c) else {
                // Fresh pivot: unit-normalize the leading entry to gcd(row[c], m), a divisor of m.
                let u = unit_scaling(row[c], m);
                for v in 0..=c {
                    row[v] = zm_mul(m, row[v], u);
                }
                let d = row[c];
                debug_assert!(m % d == 0, "a normalized pivot divides the modulus");
                let ann: Vec<u64> = row.iter().map(|&x| zm_mul(m, x, m / d)).collect();
                self.pivots.insert(c, row);
                if ann.iter().any(|&x| x != 0) {
                    self.insert(ann); // the annihilator lives strictly below column c
                }
                return;
            };
            let d = pivot[c];
            let a = row[c];
            if a % d == 0 {
                // The pivot's ideal absorbs the entry: reduce and continue below.
                let f = a / d;
                let pivot = pivot.clone();
                for v in 0..=c {
                    row[v] = zm_add(m, row[v], zm_neg(m, zm_mul(m, pivot[v], f)));
                }
            } else {
                // Zero-divisor combine: replace the pivot by the gcd row, reinsert both remainders.
                let (g0, x, y) = egcd(d as i128, a as i128);
                let g = g0 as u64; // g = gcd(d, a) divides d, hence divides m
                let (s, t) = (
                    ((x % m as i128 + m as i128) % m as i128) as u64,
                    ((y % m as i128 + m as i128) % m as i128) as u64,
                );
                let pivot = pivot.clone();
                let comb: Vec<u64> = (0..=c)
                    .map(|v| zm_add(m, zm_mul(m, s, pivot[v]), zm_mul(m, t, row[v])))
                    .collect();
                debug_assert_eq!(comb[c], g % m, "the combined pivot leads with the gcd");
                let rem_p: Vec<u64> = (0..=c)
                    .map(|v| zm_add(m, pivot[v], zm_neg(m, zm_mul(m, comb[v], d / g))))
                    .collect();
                let rem_r: Vec<u64> = (0..=c)
                    .map(|v| zm_add(m, row[v], zm_neg(m, zm_mul(m, comb[v], a / g))))
                    .collect();
                let ann: Vec<u64> = comb.iter().map(|&v| zm_mul(m, v, m / g)).collect();
                self.pivots.insert(c, comb);
                if rem_p.iter().any(|&v| v != 0) {
                    self.insert(rem_p);
                }
                if ann.iter().any(|&v| v != 0) {
                    self.insert(ann);
                }
                row = rem_r;
            }
        }
    }

    fn contains(&self, mut target: Vec<u64>) -> bool {
        let m = self.m;
        while let Some(c) = row_lead(&target) {
            let Some(pivot) = self.pivots.get(&c) else { return false };
            let d = pivot[c];
            if target[c] % d != 0 {
                return false; // the pivot's ideal (d) ∌ target[c]
            }
            let f = target[c] / d;
            for v in 0..=c {
                target[v] = zm_add(m, target[v], zm_neg(m, zm_mul(m, pivot[v], f)));
            }
        }
        true
    }
}

/// Does a **degree-`d` Nullstellensatz refutation over the ring `ℤ/m`** exist for a polynomial
/// generator system — is `1` in the `ℤ/m`-span of `{ mono·g : deg ≤ d }`? Decided by the Howell
/// echelon (validated against the exhaustive all-combinations oracle and against the field engine at
/// prime `m`). Degree-bounded enumeration — `num_vars ≤ 63`.
pub fn ns_refutes_polys_zm(m: u64, num_vars: usize, gens: &[ZmPoly], degree: usize) -> bool {
    let basis = crate::polycalc::monomials_up_to_degree(num_vars, degree);
    let index: std::collections::HashMap<Mono, usize> =
        basis.iter().enumerate().map(|(i, &mo)| (mo, i)).collect();
    let nb = basis.len();
    let mut ech = ZmEchelon::new(m);
    for g in gens {
        if g.is_empty() {
            continue;
        }
        for &mo in &basis {
            let prod = poly_mul_mono(m, g, mo);
            if !prod.is_empty() && zm_poly_degree(&prod) <= degree {
                let mut row = vec![0u64; nb];
                for (t, c) in prod {
                    row[index[&t]] = c;
                }
                ech.insert(row);
            }
        }
    }
    let mut target = vec![0u64; nb];
    target[index[&0u64]] = 1;
    ech.contains(target)
}

/// [`ns_refutes_polys_zm`] for a CNF (signed clause false-indicators as generators). An empty clause
/// is `1 = 0` outright.
pub fn ns_refutes_zm(m: u64, num_vars: usize, clauses: &[Vec<Lit>], degree: usize) -> bool {
    if clauses.iter().any(|c| c.is_empty()) {
        return true;
    }
    let gens: Vec<ZmPoly> = clauses.iter().map(|c| clause_polynomial_zm(m, c)).collect();
    ns_refutes_polys_zm(m, num_vars, &gens, degree)
}

/// Re-check a **ring pseudo-expectation** (zero trust): `L(1) ≢ 0 (mod m)` — over a ring the
/// normalization cannot demand a unit, and a zero-divisor value like `m/p` is honest — and
/// `⟨L, mono·g⟩ ≡ 0` for every admitted generator product. Soundness is one line and field-free: a
/// degree-`d` refutation `1 = Σ λ·prod` would force `L(1) = Σ λ·L(prod) = 0`. So `true` certifies
/// that no degree-`d` refutation over `ℤ/m` exists. Degree-bounded enumeration — `num_vars ≤ 63`.
pub fn check_ns_lower_bound_polys_zm(
    m: u64,
    num_vars: usize,
    gens: &[ZmPoly],
    degree: usize,
    witness: &[(Mono, u64)],
) -> bool {
    let mut l: BTreeMap<Mono, u64> = BTreeMap::new();
    for &(mo, v) in witness {
        add_term(m, &mut l, mo, v);
    }
    if l.get(&0u64).copied().unwrap_or(0) == 0 {
        return false; // L(1) must be nonzero (a unit is not required — zero divisors are honest)
    }
    let value = |mo: &Mono| l.get(mo).copied().unwrap_or(0);
    for g in gens {
        if g.is_empty() {
            continue;
        }
        for &mo in &crate::polycalc::monomials_up_to_degree(num_vars, degree) {
            let prod = poly_mul_mono(m, g, mo);
            if !prod.is_empty() && zm_poly_degree(&prod) <= degree {
                let pairing =
                    prod.iter().fold(0u64, |acc, (t, &c)| zm_add(m, acc, zm_mul(m, c, value(t))));
                if pairing != 0 {
                    return false;
                }
            }
        }
    }
    true
}

/// [`check_ns_lower_bound_polys_zm`] for a CNF. An empty clause admits no lower bound at any degree.
pub fn check_ns_lower_bound_zm(
    m: u64,
    num_vars: usize,
    clauses: &[Vec<Lit>],
    degree: usize,
    witness: &[(Mono, u64)],
) -> bool {
    if clauses.iter().any(|c| c.is_empty()) {
        return false;
    }
    let gens: Vec<ZmPoly> = clauses.iter().map(|c| clause_polynomial_zm(m, c)).collect();
    check_ns_lower_bound_polys_zm(m, num_vars, &gens, degree, witness)
}

/// **Lift a prime-field pseudo-expectation to the ring**: for `p | m`, the functional
/// `L(x) = (m/p) · L_p(x mod p)` satisfies every `ℤ/m` constraint — `(m/p)·c·v` depends on `c` only
/// mod `p`, so each pairing is `(m/p) · ⟨L_p, prod mod p⟩ = 0` — and carries the zero-divisor
/// normalization `L(1) = m/p ≠ 0`. One prime witness therefore certifies the ring lower bound: the
/// composite modulus inherits the hardness of **each** of its prime parts (the witness face of the
/// CRT conjunction).
pub fn lift_prime_witness_to_zm(m: u64, p: u64, witness_p: &[(Mono, u64)]) -> Vec<(Mono, u64)> {
    assert!(p >= 2 && m % p == 0, "the lift needs a prime divisor of the modulus");
    let scale = m / p;
    witness_p
        .iter()
        .filter_map(|&(mo, v)| {
            let lifted = zm_mul(m, scale, v % p);
            (lifted != 0).then_some((mo, lifted))
        })
        .collect()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/polycalc_zm/tests.rs"]
mod tests;
