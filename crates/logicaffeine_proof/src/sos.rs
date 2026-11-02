//! **Sum-of-Squares / Positivstellensatz refutation over ℚ — kept EXACT** (no SDP, no floating point)
//! by the LP slice of the cone. True SoS is a semidefinite program over the reals; that would forfeit
//! our certified guarantee. Instead we take the *diagonal* (fixed-square-basis) Positivstellensatz, which
//! is an exact rational linear program: a CNF is lifted to degree-2 pseudo-Boolean inequalities and the
//! lift is refuted by exact Fourier–Motzkin / Farkas elimination.
//!
//! The lift, over variables `xᵢ` and the products `zᵢⱼ = xᵢ·xⱼ` (i < j):
//! - **box**: `0 ≤ xᵢ ≤ 1`;
//! - **McCormick envelope** of each product: `zᵢⱼ ≥ 0`, `zᵢⱼ ≤ xᵢ`, `zᵢⱼ ≤ xⱼ`, `zᵢⱼ ≥ xᵢ+xⱼ−1`
//!   (the exact integral hull of `zᵢⱼ = xᵢ ∧ xⱼ`);
//! - **clauses**: `Σ val(lit) ≥ 1`, `val(x)=x`, `val(¬x)=1−x`;
//! - **Sherali–Adams products**: each clause times `xⱼ` and times `1−xⱼ` (the degree-2 lift);
//! - **squares**: `(xᵢ−xⱼ)² ≥ 0` and `(1−xᵢ−xⱼ)² ≥ 0` — the genuine sum-of-squares terms, the
//!   ordered-field content that Nullstellensatz/Polynomial-Calculus (equality-only over GF(2)) cannot
//!   express.
//!
//! **Soundness**: every constraint holds at every Boolean assignment, so a Boolean model of the CNF is a
//! feasible point of the lift; therefore an *infeasible* lift (a Farkas refutation) certifies the CNF
//! UNSAT. The elimination is exact `i128` and **fail-closed under load**: it declines (reports no
//! refutation) the moment a coefficient would exceed `MAG_CAP` or the row count would exceed
//! `ROW_CAP`, so it never returns an overflow-corrupted verdict. Incomplete at degree 2 (that is the
//! whole point of the degree dial), and the square basis is symmetry-reducible — quotient it by the
//! formula's automorphisms to shrink the LP, the same "symmetry break to uncover more" that collapses
//! the field cuts and the monomial basis.

use crate::cdcl::Lit;
use std::collections::BTreeMap;

/// Degree-2 lifts have ≈ n²/2 product variables; Fourier–Motzkin is doubly-exponential in the variable
/// count, so only small cores are lifted — larger instances decline to the other routes. (A rational
/// simplex, or the symmetry-reduced LP, is the scaling path.)
const MAX_VARS: usize = 6;
/// Decline once the working set grows past this — bounds the elimination's time/space. Genuine
/// degree-2 refutations of small cores are found in far fewer rows; past this we are in blow-up territory
/// and decline (soundly) rather than grind.
const ROW_CAP: usize = 20_000;
/// Decline once any coefficient would exceed this — keeps every `i128` operation exact (no overflow).
const MAG_CAP: i128 = 1 << 50;

/// A degree-≤2 multilinear polynomial over the Booleans: constant + Σ aᵢ·xᵢ + Σ bᵢⱼ·xᵢxⱼ (i < j).
#[derive(Clone, Default)]
struct Poly {
    c: i64,
    lin: BTreeMap<usize, i64>,
    quad: BTreeMap<(usize, usize), i64>,
}

impl Poly {
    fn constant(c: i64) -> Self {
        Poly {
            c,
            ..Default::default()
        }
    }
    fn x(i: usize) -> Self {
        Poly {
            lin: BTreeMap::from([(i, 1)]),
            ..Default::default()
        }
    }
    fn add(&self, o: &Self) -> Self {
        let mut r = self.clone();
        r.c += o.c;
        for (&k, &v) in &o.lin {
            *r.lin.entry(k).or_insert(0) += v;
        }
        for (&k, &v) in &o.quad {
            *r.quad.entry(k).or_insert(0) += v;
        }
        r.prune()
    }
    fn neg(&self) -> Self {
        Poly {
            c: -self.c,
            lin: self.lin.iter().map(|(&k, &v)| (k, -v)).collect(),
            quad: self.quad.iter().map(|(&k, &v)| (k, -v)).collect(),
        }
    }
    fn sub(&self, o: &Self) -> Self {
        self.add(&o.neg())
    }
    fn prune(mut self) -> Self {
        self.lin.retain(|_, v| *v != 0);
        self.quad.retain(|_, v| *v != 0);
        self
    }
    /// Multiply two **linear** polynomials (no quadratic part), folding `xᵢ² = xᵢ` (multilinear).
    fn mul_linear(a: &Self, b: &Self) -> Self {
        debug_assert!(
            a.quad.is_empty() && b.quad.is_empty(),
            "mul_linear needs degree-1 inputs"
        );
        let mut r = Poly::constant(a.c * b.c);
        for (&i, &ai) in &a.lin {
            *r.lin.entry(i).or_insert(0) += ai * b.c;
        }
        for (&j, &bj) in &b.lin {
            *r.lin.entry(j).or_insert(0) += a.c * bj;
        }
        for (&i, &ai) in &a.lin {
            for (&j, &bj) in &b.lin {
                if i == j {
                    *r.lin.entry(i).or_insert(0) += ai * bj; // xᵢ·xᵢ = xᵢ
                } else {
                    *r.quad.entry((i.min(j), i.max(j))).or_insert(0) += ai * bj;
                }
            }
        }
        r.prune()
    }
}

/// The value polynomial of a literal: `x` for a positive literal, `1 − x` for a negative one.
fn lit_value(l: &Lit) -> Poly {
    let x = Poly::x(l.var() as usize);
    if l.is_positive() {
        x
    } else {
        Poly::constant(1).sub(&x)
    }
}

/// A linear inequality `Σ coeffs·var + constant ≤ 0` over the lifted variables (`xᵢ` and `zᵢⱼ`), carrying
/// the non-negative combination `prov` of original constraints that produced it.
#[derive(Clone)]
struct Row {
    coeffs: BTreeMap<usize, i128>,
    constant: i128,
    prov: BTreeMap<usize, i128>,
}

impl Row {
    /// `combined = self·ka + other·kb` (ka, kb ≥ 0), or `None` if any magnitude would exceed [`MAG_CAP`].
    fn combine(&self, ka: i128, other: &Row, kb: i128) -> Option<Row> {
        let mut coeffs = BTreeMap::new();
        let mut acc =
            |dst: &mut BTreeMap<usize, i128>, src: &BTreeMap<usize, i128>, k: i128| -> Option<()> {
                for (&v, &c) in src {
                    let e = dst.entry(v).or_insert(0);
                    *e = e.checked_add(c.checked_mul(k)?)?;
                    if e.abs() > MAG_CAP {
                        return None;
                    }
                }
                Some(())
            };
        acc(&mut coeffs, &self.coeffs, ka)?;
        acc(&mut coeffs, &other.coeffs, kb)?;
        coeffs.retain(|_, c| *c != 0);
        let constant = self
            .constant
            .checked_mul(ka)?
            .checked_add(other.constant.checked_mul(kb)?)?;
        if constant.abs() > MAG_CAP {
            return None;
        }
        let mut prov = BTreeMap::new();
        acc(&mut prov, &self.prov, ka)?;
        acc(&mut prov, &other.prov, kb)?;
        prov.retain(|_, c| *c != 0);
        Some(Row {
            coeffs,
            constant,
            prov,
        })
    }
}

/// Convert a `Poly` (meaning `poly ≤ 0`) into a lift `Row`, tagged with original-constraint index `idx`.
/// `zᵢⱼ` gets a stable index above the `xᵢ` block.
fn poly_to_row(p: &Poly, num_vars: usize, idx: usize) -> Row {
    let mut coeffs = BTreeMap::new();
    for (&i, &v) in &p.lin {
        coeffs.insert(i, v as i128);
    }
    for (&(i, j), &v) in &p.quad {
        coeffs.insert(num_vars + i * num_vars + j, v as i128); // z index, unique for i<j<num_vars
    }
    Row {
        coeffs,
        constant: p.c as i128,
        prov: BTreeMap::from([(idx, 1)]),
    }
}

/// Push `p ≥ 0` into the lift as the constraint `−p ≤ 0`.
fn ge0(out: &mut Vec<Poly>, p: Poly) {
    out.push(p.neg());
}

/// The polynomials of the degree-2 lift, each meaning `poly ≤ 0`.
fn lift_polys(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Poly> {
    let mut out: Vec<Poly> = Vec::new();
    let one = Poly::constant(1);
    for i in 0..num_vars {
        ge0(&mut out, Poly::x(i));
        ge0(&mut out, one.sub(&Poly::x(i)));
    }
    for i in 0..num_vars {
        for j in (i + 1)..num_vars {
            let z = Poly {
                quad: BTreeMap::from([((i, j), 1)]),
                ..Default::default()
            };
            let (xi, xj) = (Poly::x(i), Poly::x(j));
            ge0(&mut out, z.clone());
            ge0(&mut out, xi.sub(&z));
            ge0(&mut out, xj.sub(&z));
            ge0(&mut out, z.sub(&xi).sub(&xj).add(&one));
            ge0(&mut out, xi.add(&xj).sub(&z).sub(&z)); // (xᵢ−xⱼ)²
            ge0(&mut out, one.sub(&xi).sub(&xj).add(&z).add(&z)); // (1−xᵢ−xⱼ)²
        }
    }
    for c in clauses {
        if c.is_empty() {
            out.push(Poly::constant(1)); // 1 ≤ 0
            continue;
        }
        let mut cl = Poly::constant(-1);
        for l in c {
            cl = cl.add(&lit_value(l));
        }
        ge0(&mut out, cl.clone());
        for j in 0..num_vars {
            ge0(&mut out, Poly::mul_linear(&cl, &Poly::x(j)));
            ge0(&mut out, Poly::mul_linear(&cl, &one.sub(&Poly::x(j))));
        }
    }
    out
}

/// Exact Fourier–Motzkin: is `{ rowᵢ ≤ 0 }` infeasible over ℚ? Returns the Farkas multipliers (a
/// non-negative combination of the original rows summing to a positive constant `≤ 0`) on infeasibility,
/// or `None` if feasible **or** if the elimination exceeded [`ROW_CAP`]/[`MAG_CAP`] (fail-closed: a
/// decline is reported as "no refutation found", never a false one).
fn farkas_refute(rows: Vec<Row>, var_count: usize) -> Option<BTreeMap<usize, i128>> {
    let contradiction = |rows: &[Row]| -> Option<BTreeMap<usize, i128>> {
        rows.iter()
            .find(|r| r.coeffs.is_empty() && r.constant > 0)
            .map(|r| r.prov.clone())
    };
    let mut rows = rows;
    if let Some(p) = contradiction(&rows) {
        return Some(p);
    }
    for v in 0..var_count {
        let (mut pos, mut neg, mut next): (Vec<&Row>, Vec<&Row>, Vec<Row>) =
            (Vec::new(), Vec::new(), Vec::new());
        for r in &rows {
            match r.coeffs.get(&v).copied().unwrap_or(0) {
                c if c > 0 => pos.push(r),
                c if c < 0 => neg.push(r),
                _ => next.push(r.clone()),
            }
        }
        for p in &pos {
            for n in &neg {
                let pc = p.coeffs[&v];
                let nc = -n.coeffs[&v];
                let combined = p.combine(nc, n, pc)?; // None ⟹ magnitude overflow ⟹ decline
                next.push(combined);
                if next.len() > ROW_CAP {
                    return None; // decline rather than blow up
                }
            }
        }
        rows = next;
        if let Some(p) = contradiction(&rows) {
            return Some(p);
        }
    }
    contradiction(&rows)
}

/// Does a degree-2 Positivstellensatz (diagonal SoS) refutation of the CNF exist over ℚ? Sound: a `true`
/// result is a certified UNSAT. Declines (false) when no degree-2 refutation exists or the instance
/// exceeds `MAX_VARS` / the elimination caps.
pub fn sos_refutes(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    sos_certificate(num_vars, clauses).is_some()
}

/// The Farkas certificate of a degree-2 SoS refutation: the non-negative multipliers over the lift
/// constraints whose combination is a positive constant `≤ 0`. `None` if no degree-2 refutation is found.
/// Re-checkable: combining the lift polynomials with these multipliers yields a bare positive constant.
pub fn sos_certificate(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<BTreeMap<usize, i128>> {
    if num_vars > MAX_VARS {
        return None;
    }
    let polys = lift_polys(num_vars, clauses);
    let rows: Vec<Row> = polys
        .iter()
        .enumerate()
        .map(|(i, p)| poly_to_row(p, num_vars, i))
        .collect();
    let var_count = num_vars + num_vars * num_vars; // xᵢ block + zᵢⱼ block (sparse upper indices)
    farkas_refute(rows, var_count)
}

/// Independently re-check an SoS Farkas certificate from [`sos_certificate`]: recompute the degree-2
/// lift from the clauses, combine its polynomials with the certificate's multipliers, and confirm the
/// result cancels every variable and leaves a strictly positive constant — the self-contained
/// contradiction `0 < d ≤ 0`. It shares no arithmetic with the Fourier–Motzkin elimination that
/// produced the certificate, so it is a genuine independent witness of the refutation (the SoS analog
/// of [`crate::xorsat::is_refutation`]). Fails closed on a negative multiplier, an out-of-range row,
/// an empty certificate, or any residual variable.
pub fn check_sos_certificate(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    cert: &BTreeMap<usize, i128>,
) -> bool {
    if cert.is_empty() {
        return false;
    }
    let polys = lift_polys(num_vars, clauses);
    let (mut lin, mut quad) = (
        BTreeMap::<usize, i128>::new(),
        BTreeMap::<(usize, usize), i128>::new(),
    );
    let mut constant: i128 = 0;
    for (&i, &mult) in cert {
        if mult < 0 || i >= polys.len() {
            return false; // Farkas multipliers are non-negative and index real lift rows.
        }
        let p = &polys[i];
        constant += mult * p.c as i128;
        for (&v, &c) in &p.lin {
            *lin.entry(v).or_insert(0) += mult * c as i128;
        }
        for (&k, &c) in &p.quad {
            *quad.entry(k).or_insert(0) += mult * c as i128;
        }
    }
    lin.values().all(|&c| c == 0) && quad.values().all(|&c| c == 0) && constant > 0
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/sos/tests.rs"]
mod tests;
