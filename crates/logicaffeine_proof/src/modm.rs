//! Linear algebra over `ℤ/m` for **composite** `m`, by the Chinese Remainder Theorem — the
//! multiplicative symmetry break. A system `Σ aᵢ·xᵢ ≡ c (mod m)` over a **squarefree** modulus
//! `m = p₁·…·p_t` factors, through the ring isomorphism `ℤ/m ≅ GF(p₁) × … × GF(p_t)`, into one
//! independent system over each prime field — each decided by [`crate::modp::solve`] — and the
//! per-field solutions are recombined by CRT. The *factorization of the modulus* is the symmetry that
//! splits the problem; the prime fields are the irreducible pieces:
//!
//! - **consistent mod `m`  ⟺  consistent over every prime factor `pᵢ`**, and the recombined residues
//!   give an assignment over `ℤ/m` that re-checks ([`satisfies`]);
//! - **inconsistent mod `m`  ⟺  some prime factor's system is inconsistent**, and that prime together
//!   with its GF(p) refutation combination is the re-checkable witness ([`crate::modp::is_refutation`]).
//!
//! This is [`crate::modp`] (the prime-field cut, itself the prime generalization of the GF(2) parity
//! cut) carried up onto the composites by multiplication. Prime-power moduli `p^k` need the residue
//! *ring* `ℤ/p^k` (a different object — `p` is a zero divisor), a separate rung; this module is exact
//! and complete for squarefree `m`.

use crate::modp::{self, ModpEquation, ModpOutcome};

fn gcd_i128(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// Extended Euclid: `(g, x, y)` with `a·x + b·y = g = gcd(a,b)`.
fn egcd(a: i128, b: i128) -> (i128, i128, i128) {
    if b == 0 {
        (a, 1, 0)
    } else {
        let (g, x, y) = egcd(b, a % b);
        (g, y, x - (a / b) * y)
    }
}

/// Inverse of `a` mod `n` (n need not be prime), assuming `gcd(a, n) = 1`; result in `0..n`.
fn modinv(a: i128, n: i128) -> i128 {
    let (_, x, _) = egcd(a.rem_euclid(n), n);
    x.rem_euclid(n)
}

/// The distinct prime factors of `m`, in increasing order — or `None` if `m < 2` or `m` is not
/// squarefree (some `p² | m`). Squarefreeness is exactly the condition under which `ℤ/m` is a product
/// of fields, so the CRT-over-prime-fields decision procedure below is complete.
pub fn squarefree_primes(m: u64) -> Option<Vec<u64>> {
    if m < 2 {
        return None;
    }
    let mut primes = Vec::new();
    let mut x = m;
    let mut d = 2u64;
    while d * d <= x {
        if x % d == 0 {
            x /= d;
            if x % d == 0 {
                return None; // p² | m ⇒ ℤ/m is not a product of fields
            }
            primes.push(d);
        }
        d += 1;
    }
    if x > 1 {
        primes.push(x);
    }
    Some(primes)
}

/// The outcome of deciding a linear system over `ℤ/m`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModmOutcome {
    /// Satisfiable, with an assignment over `0..m` for each variable (re-checkable via [`satisfies`]).
    Sat(Vec<u64>),
    /// Unsatisfiable: inconsistent already over the quotient ring `ℤ/modulus` for a prime-power factor
    /// `modulus = pᵏ` of `m` (a prime field when `k = 1`), witnessed by a combination of the equations
    /// whose left side cancels mod `modulus` while the right side does not. Because `modulus | m`, any
    /// solution mod `m` would reduce to one mod `modulus`, so this certifies UNSAT mod `m`. Re-checkable
    /// via [`is_refutation`].
    Unsat {
        modulus: u64,
        combo: Vec<(usize, u64)>,
    },
}

/// Re-check a refutation over `ℤ/modulus`: the chosen combination of equations has every variable
/// coefficient `≡ 0` and a nonzero right-hand side mod `modulus` — a solver-free certificate that the
/// system is inconsistent over that quotient ring (and hence over any `ℤ/m` with `modulus | m`).
pub fn is_refutation(
    equations: &[ModpEquation],
    num_vars: usize,
    modulus: u64,
    combo: &[(usize, u64)],
) -> bool {
    if combo.is_empty() {
        return false;
    }
    let mm = modulus as u128;
    let mut lhs = vec![0u128; num_vars];
    let mut rhs = 0u128;
    for &(idx, mult) in combo {
        let Some(eq) = equations.get(idx) else {
            return false;
        };
        for &(v, a) in &eq.coeffs {
            if v < num_vars {
                lhs[v] = (lhs[v] + mult as u128 * a as u128) % mm;
            }
        }
        rhs = (rhs + mult as u128 * eq.rhs as u128) % mm;
    }
    lhs.iter().all(|&x| x == 0) && rhs != 0
}

/// Decide a linear system over `ℤ/m` for **squarefree** `m`, via CRT over the prime fields. Returns
/// `None` only when `m` is not squarefree (the prime-power ring is a separate rung). The first prime
/// factor whose system is inconsistent is reported as the witness — by CRT, the whole composite system
/// is then inconsistent.
pub fn solve_squarefree(
    equations: &[ModpEquation],
    num_vars: usize,
    m: u64,
) -> Option<ModmOutcome> {
    let primes = squarefree_primes(m)?;
    let mut per_prime: Vec<(u64, Vec<u64>)> = Vec::with_capacity(primes.len());
    for &p in &primes {
        match modp::solve(equations, num_vars, p) {
            ModpOutcome::Sat(a) => per_prime.push((p, a)),
            ModpOutcome::Unsat(combo) => return Some(ModmOutcome::Unsat { modulus: p, combo }),
        }
    }
    // Every prime field is consistent: recombine each variable's residues into a value mod m by CRT.
    let mut assignment = vec![0u64; num_vars];
    for (i, slot) in assignment.iter_mut().enumerate() {
        let residues: Vec<(u64, u64)> = per_prime.iter().map(|(p, a)| (a[i], *p)).collect();
        *slot = crt(&residues);
    }
    Some(ModmOutcome::Sat(assignment))
}

/// Combine residues `(rᵢ mod mᵢ)` with pairwise-coprime moduli (primes or prime powers) into the unique
/// value mod `∏ mᵢ`, by incremental CRT: `x ≡ acc (mod M)` and `x ≡ r (mod mᵢ)` ⟹
/// `x = acc + M·((r−acc)·M⁻¹ mod mᵢ)`.
fn crt(residues: &[(u64, u64)]) -> u64 {
    let mut acc_r = 0i128;
    let mut acc_m = 1i128;
    for &(r, modu) in residues {
        let modu = modu as i128;
        let diff = (r as i128 - acc_r).rem_euclid(modu);
        let t = (diff * modinv(acc_m.rem_euclid(modu), modu)).rem_euclid(modu);
        acc_r += acc_m * t;
        acc_m *= modu;
        acc_r = acc_r.rem_euclid(acc_m);
    }
    acc_r as u64
}

/// The forced-value structure of a `ℤ/m` system over a **squarefree** `m`.
pub enum ForcedM {
    /// Inconsistent over `ℤ/m` (UNSAT) — some prime field is inconsistent.
    Inconsistent,
    /// `forced[g] = Some(v)` iff `x_g ≡ v (mod m)` in *every* solution, else `None`.
    Forced(Vec<Option<u64>>),
}

/// Which variables a **squarefree** `ℤ/m` system forces to a single value, by CRT of the `GF(pᵢ)` solution
/// spaces (`ℤ/m ≅ ∏ GF(pᵢ)`): a variable is forced mod `m` iff it is forced mod *every* prime factor
/// ([`crate::modp::solve_space`] — its kernel never moves it), and the value is the CRT of those residues.
/// `None` if `m` is not squarefree (the prime-power case needs Smith normal form, not handled here). This
/// is the substrate of the composite affine SAT-side break.
pub fn forced_values_squarefree(
    equations: &[ModpEquation],
    num_vars: usize,
    m: u64,
) -> Option<ForcedM> {
    let primes = squarefree_primes(m)?;
    let mut per_prime: Vec<(u64, Vec<Option<u64>>)> = Vec::with_capacity(primes.len());
    for &p in &primes {
        let Some(ss) = crate::modp::solve_space(equations, num_vars, p) else {
            return Some(ForcedM::Inconsistent);
        };
        let forced_p: Vec<Option<u64>> = (0..num_vars)
            .map(|g| {
                ss.kernel_basis
                    .iter()
                    .all(|k| k[g] == 0)
                    .then(|| ss.particular[g])
            })
            .collect();
        per_prime.push((p, forced_p));
    }
    let forced: Vec<Option<u64>> = (0..num_vars)
        .map(|g| {
            let residues: Option<Vec<(u64, u64)>> = per_prime
                .iter()
                .map(|(p, f)| f[g].map(|v| (v, *p)))
                .collect();
            residues.map(|res| crt(&res))
        })
        .collect();
    Some(ForcedM::Forced(forced))
}

/// The complete solution space of a `ℤ/pᵏ` linear system, in symmetry-broken form: a particular solution
/// plus a generating set of the kernel **submodule** (each generator is a column of the Smith transform
/// `V` scaled by its pivot's freedom `q/gcd(dₜ,q)`). The prime-power analogue of
/// [`crate::modp::SolutionSpaceP`] over a local ring — and the substrate of the *scalable* forced/linked
/// break: a variable the kernel never moves is forced.
pub struct SolutionSpaceM {
    pub num_vars: usize,
    pub m: u64,
    pub particular: Vec<u64>,
    pub kernel_basis: Vec<Vec<u64>>,
}

/// The outcome of [`solve_space_prime_power`].
pub enum PrimePowerSpace {
    Inconsistent,
    Space(SolutionSpaceM),
}

/// Solve a `ℤ/pᵏ` system for its full solution **space** via Smith normal form `U·A·V = D` — the scalable
/// analogue of [`crate::modp::solve_space`] over a prime-power ring (where `p` is a zero-divisor, so field
/// Gaussian fails). Returns the particular solution plus the kernel generators (so a variable the kernel
/// never moves is forced), or `Inconsistent`. `None` only when the integer Smith reduction overflows
/// `GROWTH_CAP`. `k = 1` delegates to the prime-field solver.
pub fn solve_space_prime_power(
    equations: &[ModpEquation],
    num_vars: usize,
    p: u64,
    k: u32,
) -> Option<PrimePowerSpace> {
    if k == 1 {
        return Some(match crate::modp::solve_space(equations, num_vars, p) {
            None => PrimePowerSpace::Inconsistent,
            Some(ss) => PrimePowerSpace::Space(SolutionSpaceM {
                num_vars,
                m: p,
                particular: ss.particular,
                kernel_basis: ss.kernel_basis,
            }),
        });
    }
    let q = (p as i128).pow(k);
    let (m, n) = (equations.len(), num_vars);
    let mut a = vec![vec![0i128; n]; m];
    let mut b = vec![0i128; m];
    let mut u = vec![vec![0i128; m]; m];
    let mut v = vec![vec![0i128; n]; n];
    for (i, ui) in u.iter_mut().enumerate() {
        ui[i] = 1;
    }
    for (j, vj) in v.iter_mut().enumerate() {
        vj[j] = 1;
    }
    for (i, eq) in equations.iter().enumerate() {
        for &(var, coef) in &eq.coeffs {
            if var < n {
                a[i][var] = (a[i][var] + coef as i128).rem_euclid(q);
            }
        }
        b[i] = (eq.rhs as i128).rem_euclid(q);
    }
    let exceeds = |a: &[Vec<i128>], u: &[Vec<i128>], v: &[Vec<i128>]| {
        a.iter()
            .chain(u)
            .chain(v)
            .any(|r| r.iter().any(|&x| x.abs() > GROWTH_CAP))
    };
    let mut rank = 0usize;
    for t in 0..m.min(n) {
        loop {
            let mut best: Option<(usize, usize, i128)> = None;
            for (i, row) in a.iter().enumerate().skip(t) {
                for (j, &val) in row.iter().enumerate().skip(t) {
                    if val != 0 && best.is_none_or(|(_, _, bv)| val.abs() < bv) {
                        best = Some((i, j, val.abs()));
                    }
                }
            }
            let Some((pi, pj, _)) = best else { break };
            if pi != t {
                a.swap(pi, t);
                u.swap(pi, t);
            }
            if pj != t {
                for row in a.iter_mut() {
                    row.swap(pj, t);
                }
                for row in v.iter_mut() {
                    row.swap(pj, t);
                }
            }
            let piv = a[t][t];
            for i in 0..m {
                if i != t && a[i][t] != 0 {
                    let f = a[i][t].div_euclid(piv);
                    if f != 0 {
                        for j in 0..n {
                            a[i][j] -= f * a[t][j];
                        }
                        for j in 0..m {
                            u[i][j] -= f * u[t][j];
                        }
                    }
                }
            }
            for j in 0..n {
                if j != t && a[t][j] != 0 {
                    let f = a[t][j].div_euclid(piv);
                    if f != 0 {
                        for row in a.iter_mut() {
                            row[j] -= f * row[t];
                        }
                        for row in v.iter_mut() {
                            row[j] -= f * row[t];
                        }
                    }
                }
            }
            if exceeds(&a, &u, &v) {
                return None;
            }
            if (0..m).all(|i| i == t || a[i][t] == 0) && (0..n).all(|j| j == t || a[t][j] == 0) {
                break;
            }
        }
        if a[t][t] != 0 {
            rank = t + 1;
        } else {
            break;
        }
    }
    let ub: Vec<i128> = (0..m)
        .map(|i| {
            (0..m)
                .fold(0i128, |acc, r| acc + u[i][r] * b[r])
                .rem_euclid(q)
        })
        .collect();

    // Solve D·y ≡ ub for the particular y, recording each coordinate's freedom step hₜ = q/gcd(dₜ,q)
    // (= q ⇒ forced; < q ⇒ the kernel can move yₜ by hₜ). Free columns (t ≥ rank) move by 1.
    let mut y = vec![0i128; n];
    let mut freedom = vec![1i128; n];
    for t in 0..rank {
        let d = a[t][t].rem_euclid(q);
        let g = gcd_i128(d, q);
        if ub[t].rem_euclid(g) != 0 {
            return Some(PrimePowerSpace::Inconsistent);
        }
        let qg = q / g;
        y[t] = if qg == 1 {
            0
        } else {
            (ub[t] / g).rem_euclid(qg) * modinv(d / g, qg) % qg
        };
        freedom[t] = qg;
    }
    for &ubt in ub.iter().skip(rank) {
        if ubt != 0 {
            return Some(PrimePowerSpace::Inconsistent); // a zero row with nonzero rhs ⇒ UNSAT
        }
    }
    let particular: Vec<u64> = (0..n)
        .map(|i| {
            (0..n)
                .fold(0i128, |acc, j| acc + v[i][j] * y[j])
                .rem_euclid(q) as u64
        })
        .collect();
    // Kernel generators: δx = hₜ·(t-th column of V), for every coordinate with genuine freedom (hₜ < q).
    let mut kernel_basis: Vec<Vec<u64>> = Vec::new();
    for t in 0..n {
        if freedom[t] >= q {
            continue;
        }
        let gen: Vec<u64> = (0..n)
            .map(|i| (v[i][t] * freedom[t]).rem_euclid(q) as u64)
            .collect();
        if gen.iter().any(|&x| x != 0) {
            kernel_basis.push(gen);
        }
    }
    Some(PrimePowerSpace::Space(SolutionSpaceM {
        num_vars: n,
        m: q as u64,
        particular,
        kernel_basis,
    }))
}

/// Which variables a `ℤ/m` system forces, for **any** composite `m` — by CRT over its prime-power
/// components (`ℤ/m ≅ ∏ ℤ/pᵢ^{kᵢ}`), each solved scalably by Smith normal form
/// ([`solve_space_prime_power`]): a variable is forced mod `m` iff its kernel never moves it in *every*
/// component, and the value is the CRT of the residues. This is the Smith-form generalization that
/// replaces the old bounded brute force — no size cap; `None` only on Smith overflow. `Inconsistent` when
/// any component is.
pub fn forced_values_prime_power(
    equations: &[ModpEquation],
    num_vars: usize,
    m: u64,
) -> Option<ForcedM> {
    let factors = prime_power_factorize(m)?;
    let mut per_component: Vec<(u64, Vec<Option<u64>>)> = Vec::with_capacity(factors.len());
    for (p, k) in factors {
        match solve_space_prime_power(equations, num_vars, p, k)? {
            PrimePowerSpace::Inconsistent => return Some(ForcedM::Inconsistent),
            PrimePowerSpace::Space(ss) => {
                let forced_c: Vec<Option<u64>> = (0..num_vars)
                    .map(|g| {
                        ss.kernel_basis
                            .iter()
                            .all(|kk| kk[g] == 0)
                            .then(|| ss.particular[g])
                    })
                    .collect();
                per_component.push((p.pow(k), forced_c));
            }
        }
    }
    let forced: Vec<Option<u64>> = (0..num_vars)
        .map(|g| {
            let residues: Option<Vec<(u64, u64)>> = per_component
                .iter()
                .map(|(q, f)| f[g].map(|v| (v, *q)))
                .collect();
            residues.map(|res| crt(&res))
        })
        .collect();
    Some(ForcedM::Forced(forced))
}

/// The per-variable **allowed-value congruence** of a `ℤ/m` system, for any composite `m`. By CRT over the
/// prime-power components, each variable's solution-projection is a coset `x_g ≡ residue (mod modulus)` —
/// the *tightest* congruence the system forces on it. Generalizes forcing: `modulus = m` is a single value
/// (fully forced), `1 < modulus < m` is **partial** (confined to a value-subset), `modulus = 1` is free.
/// Within each prime-power component the coset modulus is `gcd(p^k, kernel column at g)` from
/// [`solve_space_prime_power`]; the components recombine by CRT. `Inconsistent` if any component is.
pub enum AllowedOutcome {
    Inconsistent,
    Allowed(Vec<(u64, u64)>),
}

/// See [`AllowedOutcome`]. `None` only on Smith overflow.
pub fn allowed_residues(
    equations: &[ModpEquation],
    num_vars: usize,
    m: u64,
) -> Option<AllowedOutcome> {
    let factors = prime_power_factorize(m)?;
    let mut per_component: Vec<Vec<(u64, u64)>> = Vec::with_capacity(factors.len());
    for (p, k) in factors {
        let q = p.pow(k);
        match solve_space_prime_power(equations, num_vars, p, k)? {
            PrimePowerSpace::Inconsistent => return Some(AllowedOutcome::Inconsistent),
            PrimePowerSpace::Space(ss) => {
                let pv: Vec<(u64, u64)> = (0..num_vars)
                    .map(|g| {
                        // The coset modulus in this component: gcd of the ring order with the kernel's
                        // moves of x_g (an empty kernel ⇒ gcd = q ⇒ fully pinned).
                        let d = ss
                            .kernel_basis
                            .iter()
                            .fold(q, |acc, kk| gcd_i128(acc as i128, kk[g] as i128) as u64);
                        (ss.particular[g] % d, d)
                    })
                    .collect();
                per_component.push(pv);
            }
        }
    }
    let residues: Vec<(u64, u64)> = (0..num_vars)
        .map(|g| {
            // CRT the constrained components (their moduli are pairwise-coprime prime powers); a component
            // that leaves x_g free (modulus 1) contributes nothing.
            let pairs: Vec<(u64, u64)> = per_component
                .iter()
                .map(|pv| pv[g])
                .filter(|&(_, d)| d > 1)
                .collect();
            if pairs.is_empty() {
                (0, 1)
            } else {
                (crt(&pairs), pairs.iter().map(|&(_, d)| d).product())
            }
        })
        .collect();
    Some(AllowedOutcome::Allowed(residues))
}

/// Re-check a satisfying assignment over `ℤ/m`: every congruence holds mod `m`.
pub fn satisfies(equations: &[ModpEquation], assignment: &[u64], m: u64) -> bool {
    let mm = m as u128;
    equations.iter().all(|eq| {
        let lhs = eq.coeffs.iter().fold(0u128, |acc, &(v, a)| {
            (acc + a as u128 * *assignment.get(v).unwrap_or(&0) as u128) % mm
        });
        lhs == (eq.rhs as u128 % mm)
    })
}

/// The scalable composite obstruction: the `n`-cycle of differences `xᵢ − x_{i+1} ≡ 1 (mod m)`,
/// inconsistent **exactly when `m ∤ n`** (summing telescopes the left to `0`, the right to `n`). The
/// coefficient `−1` is written `m − 1`. Reuses [`crate::modp::cycle_system`]'s shape across the field.
pub fn cycle_system(n: usize, m: u64) -> Vec<ModpEquation> {
    modp::cycle_system(n, m)
}

/// The prime-power factorization of `m` (`m = ∏ pᵢ^{kᵢ}`), or `None` if `m < 2`.
pub fn prime_power_factorize(m: u64) -> Option<Vec<(u64, u32)>> {
    if m < 2 {
        return None;
    }
    let mut out = Vec::new();
    let mut x = m;
    let mut d = 2u64;
    while d * d <= x {
        if x % d == 0 {
            let mut k = 0u32;
            while x % d == 0 {
                x /= d;
                k += 1;
            }
            out.push((d, k));
        }
        d += 1;
    }
    if x > 1 {
        out.push((x, 1));
    }
    Some(out)
}

/// Integer-magnitude ceiling for the Smith diagonalization; beyond it we decline rather than risk
/// `i128` overflow. Small prime-power components stay far below this.
const GROWTH_CAP: i128 = 1i128 << 60;

/// Decide a linear system over the residue **ring** `ℤ/pᵏ`. For `k = 1` this is the prime field, handed
/// to the proven [`crate::modp::solve`]. For `k ≥ 2` the ring has zero divisors, so we diagonalize over
/// the integers — `U·A·V = D` with `U, V` unimodular (Smith-style: gcd row/column reduction) — after
/// which each `dₜ·yₜ ≡ (U·b)ₜ (mod pᵏ)` is an independent 1-D congruence, solvable iff
/// `gcd(dₜ, pᵏ) ∣ (U·b)ₜ`. `U` yields both the satisfying assignment (`x = V·y`) and, on failure, the
/// re-checkable refutation (a row of `U`, scaled to annihilate the coefficients mod `pᵏ`). Returns
/// `None` only if the integer entries would exceed `GROWTH_CAP`.
pub fn solve_prime_power(
    equations: &[ModpEquation],
    num_vars: usize,
    p: u64,
    k: u32,
) -> Option<ModmOutcome> {
    if k == 1 {
        return Some(match modp::solve(equations, num_vars, p) {
            ModpOutcome::Sat(a) => ModmOutcome::Sat(a),
            ModpOutcome::Unsat(combo) => ModmOutcome::Unsat { modulus: p, combo },
        });
    }
    let q = (p as i128).pow(k);
    let m = equations.len();
    let n = num_vars;
    if m == 0 {
        return Some(ModmOutcome::Sat(vec![0u64; n]));
    }

    // Working matrix `a = U·A·V`, with U (m×m) and V (n×n) the accumulated unimodular transforms, and
    // `b` the original right-hand side (so `U·b` is the transformed rhs). All over ℤ.
    let mut a = vec![vec![0i128; n]; m];
    let mut b = vec![0i128; m];
    let mut u = vec![vec![0i128; m]; m];
    let mut v = vec![vec![0i128; n]; n];
    for (i, ui) in u.iter_mut().enumerate() {
        ui[i] = 1;
    }
    for (j, vj) in v.iter_mut().enumerate() {
        vj[j] = 1;
    }
    for (i, eq) in equations.iter().enumerate() {
        for &(var, coef) in &eq.coeffs {
            if var < n {
                a[i][var] = (a[i][var] + coef as i128).rem_euclid(q);
            }
        }
        b[i] = (eq.rhs as i128).rem_euclid(q);
    }

    let exceeds_cap = |a: &[Vec<i128>], u: &[Vec<i128>], v: &[Vec<i128>]| {
        a.iter()
            .chain(u)
            .chain(v)
            .any(|r| r.iter().any(|&x| x.abs() > GROWTH_CAP))
    };

    let mut rank = 0usize;
    for t in 0..m.min(n) {
        loop {
            // Pivot on the minimal-magnitude nonzero entry of the active submatrix — the gcd descent.
            let mut best: Option<(usize, usize, i128)> = None;
            for (i, row) in a.iter().enumerate().skip(t) {
                for (j, &val) in row.iter().enumerate().skip(t) {
                    if val != 0 && best.is_none_or(|(_, _, bv)| val.abs() < bv) {
                        best = Some((i, j, val.abs()));
                    }
                }
            }
            let Some((pi, pj, _)) = best else { break };
            if pi != t {
                a.swap(pi, t);
                u.swap(pi, t);
            }
            if pj != t {
                for row in a.iter_mut() {
                    row.swap(pj, t);
                }
                for row in v.iter_mut() {
                    row.swap(pj, t);
                }
            }
            let piv = a[t][t];
            for i in 0..m {
                if i != t && a[i][t] != 0 {
                    let f = a[i][t].div_euclid(piv);
                    if f != 0 {
                        for j in 0..n {
                            a[i][j] -= f * a[t][j];
                        }
                        for j in 0..m {
                            u[i][j] -= f * u[t][j];
                        }
                    }
                }
            }
            for j in 0..n {
                if j != t && a[t][j] != 0 {
                    let f = a[t][j].div_euclid(piv);
                    if f != 0 {
                        for row in a.iter_mut() {
                            row[j] -= f * row[t];
                        }
                        for row in v.iter_mut() {
                            row[j] -= f * row[t];
                        }
                    }
                }
            }
            if exceeds_cap(&a, &u, &v) {
                return None;
            }
            let col_clean = (0..m).all(|i| i == t || a[i][t] == 0);
            let row_clean = (0..n).all(|j| j == t || a[t][j] == 0);
            if col_clean && row_clean {
                break;
            }
        }
        if a[t][t] != 0 {
            rank = t + 1;
        } else {
            break;
        }
    }

    let ub: Vec<i128> = (0..m)
        .map(|i| {
            (0..m)
                .fold(0i128, |acc, r| acc + u[i][r] * b[r])
                .rem_euclid(q)
        })
        .collect();
    let combo_from_row = |row: &[i128], lambda: i128| -> Vec<(usize, u64)> {
        row.iter()
            .enumerate()
            .map(|(i, &c)| (i, (lambda * c).rem_euclid(q) as u64))
            .filter(|&(_, mlt)| mlt != 0)
            .collect()
    };

    let mut y = vec![0i128; n];
    for t in 0..rank {
        let d = a[t][t].rem_euclid(q);
        let rhs = ub[t];
        let g = gcd_i128(d, q);
        if rhs.rem_euclid(g) != 0 {
            // No solution to dₜ·yₜ ≡ rhs: scale this row by q/g to annihilate the coefficients mod q.
            return Some(ModmOutcome::Unsat {
                modulus: q as u64,
                combo: combo_from_row(&u[t], q / g),
            });
        }
        let qg = q / g;
        y[t] = if qg == 1 {
            0
        } else {
            (rhs / g).rem_euclid(qg) * modinv(d / g, qg) % qg
        };
    }
    for (t, &ubt) in ub.iter().enumerate().skip(rank) {
        // A zero row of D with a nonzero transformed rhs: uₜ·A = 0 over ℤ, so uₜ is the refutation.
        if ubt != 0 {
            return Some(ModmOutcome::Unsat {
                modulus: q as u64,
                combo: combo_from_row(&u[t], 1),
            });
        }
    }

    let x: Vec<u64> = (0..n)
        .map(|i| {
            (0..n)
                .fold(0i128, |acc, j| acc + v[i][j] * y[j])
                .rem_euclid(q) as u64
        })
        .collect();
    debug_assert!(
        satisfies(equations, &x, q as u64),
        "the ring model must satisfy mod p^k"
    );
    Some(ModmOutcome::Sat(x))
}

/// Decide a linear system over `ℤ/m` for **any** `m ≥ 2`, closing every composite modulus. By CRT,
/// `ℤ/m ≅ ∏ ℤ/pᵢ^{kᵢ}`: solve each prime-power component ([`solve_prime_power`]) and recombine the
/// residues. Consistent mod `m` ⟺ consistent over every prime-power factor; the first inconsistent
/// factor is the witness (and certifies UNSAT mod `m`, since that factor divides `m`). Returns `None`
/// only if `m < 2`, or a component is too large / overflows the integer Smith reduction.
pub fn solve(equations: &[ModpEquation], num_vars: usize, m: u64) -> Option<ModmOutcome> {
    let factors = prime_power_factorize(m)?;
    let mut per_component: Vec<(u64, Vec<u64>)> = Vec::with_capacity(factors.len());
    for (p, k) in factors {
        let q = p.pow(k);
        if q as u128 > 1_000_000_000 {
            return None; // decline an oversized prime-power component
        }
        match solve_prime_power(equations, num_vars, p, k)? {
            ModmOutcome::Sat(a) => per_component.push((q, a)),
            unsat @ ModmOutcome::Unsat { .. } => return Some(unsat),
        }
    }
    let mut assignment = vec![0u64; num_vars];
    for (i, slot) in assignment.iter_mut().enumerate() {
        let residues: Vec<(u64, u64)> = per_component.iter().map(|(q, a)| (a[i], *q)).collect();
        *slot = crt(&residues);
    }
    Some(ModmOutcome::Sat(assignment))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/modm/tests.rs"]
mod tests;
