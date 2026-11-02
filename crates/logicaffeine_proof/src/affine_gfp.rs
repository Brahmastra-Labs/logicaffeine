//! **The affine group `AGL(n,p)` over `GF(p)` — the mod-`p` generalization of the affine break.**
//!
//! [`crate::affine`] handles the `GF(2)` cube: the shears `xᵢ↦xᵢ⊕xⱼ` no permutation can see, and the
//! linear obstruction (parity) that decides them. Over a prime `p` the same picture lifts: the cube
//! becomes `GF(p)ⁿ`, signed permutations become the **monomial** group (one nonzero scalar per row/column —
//! the `Bₙ` analog over `GF(p)`), and `AGL(n,p) = { x ↦ A x + b : A ∈ GL(n,p), b ∈ GF(p)ⁿ }` is strictly
//! larger — its `GF(p)` shears are what mod-`p` counting (the `Count_p` / mod-`p` Tseitin families) is
//! invariant under and the monomial breakers are blind to. This module is the `GF(p)` affine layer:
//! the group itself, an exhaustive ground-truth detector of a `GF(p)`-system's affine symmetries (so
//! `AGL(n,p) ⊋ monomial` is *measured*), and the certified `GF(p)` affine refutation via the mod-`p`
//! Gaussian engine and the `emit_modp_drat` bridge. It reuses [`crate::modp`]'s `GF(p)` arithmetic
//! throughout (`gl_order_p`, `is_invertible_modp`, `recover_from_cnf`, `solve`).

use std::collections::HashSet;

use crate::cdcl::Lit;
use crate::modp::{self, ModpEquation};

/// An affine map of `GF(p)ⁿ`: `x ↦ A x + b`. `matrix[i][j]` is `A`'s entry, `translation` is `b`; both
/// reduced mod `p`. A bijection iff `A ∈ GL(n,p)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AffineP {
    pub p: u64,
    pub n: usize,
    pub matrix: Vec<Vec<u64>>,
    pub translation: Vec<u64>,
}

impl AffineP {
    /// The identity `x ↦ x`.
    pub fn identity(n: usize, p: u64) -> Self {
        let matrix = (0..n)
            .map(|i| (0..n).map(|j| u64::from(i == j)).collect())
            .collect();
        AffineP {
            p,
            n,
            matrix,
            translation: vec![0; n],
        }
    }

    /// Apply: output `i` is `(Σⱼ A[i][j]·x[j] + b[i]) mod p`.
    pub fn apply(&self, x: &[u64]) -> Vec<u64> {
        (0..self.n)
            .map(|i| {
                let s = (0..self.n).fold(0u64, |a, j| (a + self.matrix[i][j] * x[j]) % self.p);
                (s + self.translation[i]) % self.p
            })
            .collect()
    }

    /// Composition `self ∘ other` over `GF(p)`: linear parts multiply, `b ↦ A_self·b_other + b_self`.
    pub fn compose(&self, other: &AffineP) -> AffineP {
        let (p, n) = (self.p, self.n);
        let mut matrix = vec![vec![0u64; n]; n];
        for (i, row) in matrix.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..n).fold(0u64, |a, k| {
                    (a + self.matrix[i][k] * other.matrix[k][j]) % p
                });
            }
        }
        let translation = (0..n)
            .map(|i| {
                let av = (0..n).fold(0u64, |a, k| {
                    (a + self.matrix[i][k] * other.translation[k]) % p
                });
                (av + self.translation[i]) % p
            })
            .collect();
        AffineP {
            p,
            n,
            matrix,
            translation,
        }
    }

    /// Whether the linear part is invertible over `GF(p)` (so the map is a bijection of the cube).
    pub fn is_bijection(&self) -> bool {
        modp::is_invertible_modp(self.n, self.p, &self.matrix)
    }

    /// Whether the linear part is **monomial** — one nonzero entry per row and per column (a scaled
    /// permutation). These are exactly the `Bₙ`-analog over `GF(p)`; everything else is a genuine shear.
    pub fn is_monomial(&self) -> bool {
        let n = self.n;
        let rows = self
            .matrix
            .iter()
            .all(|r| r.iter().filter(|&&x| x != 0).count() == 1);
        let cols = (0..n).all(|j| (0..n).filter(|&i| self.matrix[i][j] != 0).count() == 1);
        rows && cols
    }
}

/// `|AGL(n,p)| = pⁿ · |GL(n,p)|`.
pub fn agl_p_order(n: u32, p: u64) -> u128 {
    (p as u128).pow(n) * modp::gl_order_p(n, p)
}

/// Every affine bijection of `GF(p)ⁿ` (each invertible matrix × each translation). Exhaustive — for
/// ground-truth symmetry computation only — bounded so `p^{n²} ≤ 200_000`.
pub fn all_affine_p_bijections(n: usize, p: u64) -> Vec<AffineP> {
    assert!(
        (p as u128).pow((n * n) as u32) <= 200_000,
        "exhaustive AGL(n,p) enumeration is bounded (p^{{n²}} ≤ 200k)"
    );
    let decode = |mut code: u64, len: usize| -> Vec<u64> {
        (0..len)
            .map(|_| {
                let d = code % p;
                code /= p;
                d
            })
            .collect()
    };
    let mut out = Vec::new();
    let matrices = (p).pow((n * n) as u32);
    let translations = (p).pow(n as u32);
    for code in 0..matrices {
        let flat = decode(code, n * n);
        let matrix: Vec<Vec<u64>> = (0..n).map(|i| flat[i * n..(i + 1) * n].to_vec()).collect();
        if !modp::is_invertible_modp(n, p, &matrix) {
            continue;
        }
        for tcode in 0..translations {
            out.push(AffineP {
                p,
                n,
                matrix: matrix.clone(),
                translation: decode(tcode, n),
            });
        }
    }
    out
}

/// The `GF(p)`-valued solutions of a mod-`p` linear system (`Σ coeffs·x ≡ rhs (mod p)` for each
/// equation), brute force over `pⁿ` — small `n` only.
pub fn models_p(n: usize, p: u64, equations: &[ModpEquation]) -> Vec<Vec<u64>> {
    assert!(
        (p as u128).pow(n as u32) <= 60_000,
        "model enumeration is brute force — small n"
    );
    let total = p.pow(n as u32);
    (0..total)
        .filter_map(|code| {
            let mut c = code;
            let x: Vec<u64> = (0..n)
                .map(|_| {
                    let d = c % p;
                    c /= p;
                    d
                })
                .collect();
            equations
                .iter()
                .all(|eq| {
                    eq.coeffs
                        .iter()
                        .fold(0u64, |a, &(v, co)| (a + co * x[v]) % p)
                        % p
                        == eq.rhs % p
                })
                .then_some(x)
        })
        .collect()
}

/// The **affine symmetry group of a `GF(p)` model set**, computed exhaustively: every `φ ∈ AGL(n,p)` that
/// maps the model set onto itself. The `AGL(n,p)` analogue of [`crate::affine::affine_symmetries`] — the
/// instrument that *measures* `AGL(n,p) ⊋ monomial`.
pub fn affine_p_symmetries(n: usize, p: u64, models: &[Vec<u64>]) -> Vec<AffineP> {
    let set: HashSet<Vec<u64>> = models.iter().cloned().collect();
    all_affine_p_bijections(n, p)
        .into_iter()
        .filter(|phi| set.iter().all(|m| set.contains(&phi.apply(m))))
        .collect()
}

/// The clausal DRAT **certificate** for a `GF(p)` affine refutation, or `None` if the formula's mod-`p`
/// core is not inconsistent (or the resolution expansion overruns its budget). Recovers the one-hot
/// `GF(p)` system, finds the `Σ multiplierᵢ·equationᵢ` dependency whose left side cancels while the right
/// does not ([`crate::modp::solve`]), and compiles it to RUP resolvent lemmas over the Boolean one-hot
/// encoding through the [`crate::xor_drat`] bridge — `drat-trim`-checkable against the original CNF, the
/// mod-`p` generalization of [`crate::affine::affine_refutation_drat`].
pub fn affine_p_refutation_drat(
    num_bool_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<Vec<Vec<Lit>>> {
    crate::xor_drat::emit_modp_drat(num_bool_vars, clauses)
}

/// The outcome of the **`GF(p)` SAT-side break**.
pub enum AffinePForced {
    /// The mod-`p` core is inconsistent — UNSAT, with the mod-`p` DRAT certificate.
    Refuted(Option<Vec<Vec<Lit>>>),
    /// Forced Boolean one-hot units — implied by the mod-`p` structure, none already present.
    Forced(Vec<Vec<Lit>>),
    /// No recoverable mod-`p` structure, or nothing determined.
    Unchanged,
}

/// `GF(p)` multiplication, subtraction, and (Fermat) inverse — `modp`'s are private, and these are small.
fn gfp_mul(a: u64, b: u64, p: u64) -> u64 {
    (a % p) * (b % p) % p
}
fn gfp_sub(a: u64, b: u64, p: u64) -> u64 {
    (a % p + p - b % p) % p
}
fn gfp_inv(a: u64, p: u64) -> u64 {
    let (mut result, mut base, mut exp) = (1u64, a % p, p - 2);
    while exp > 0 {
        if exp & 1 == 1 {
            result = gfp_mul(result, base, p);
        }
        base = gfp_mul(base, base, p);
        exp >>= 1;
    }
    result
}

/// **The `GF(p)` SAT-side break.** Recover the one-hot mod-`p` system ([`crate::modp::recover_from_cnf`])
/// and solve its solution space ([`crate::modp::solve_space`]), whose kernel is the affine translation
/// symmetry. Two structures fall out, the mod-`p` analogues of [`crate::affine::affine_reduce`]'s forced
/// units and equivalence classes:
///
/// * **Forced** — a `GF(p)` variable the kernel never moves is pinned to a single value `v`; lift that
///   through the one-hot encoding (the group's bit for `v` true, the rest false) to Boolean units.
/// * **Linked** — variables whose kernel columns are *scalar-proportional* satisfy `x_g = c·x_rep + d` in
///   every solution. That lifts to **value-permuted bit-equivalences** `b(g,v) ↔ b(rep, (v−d)·c⁻¹)` — the
///   genuinely `GF(p)` shear-links no monomial break can see, the value permutation a clause equivalence
///   cannot express on its own.
///
/// Every emitted clause is `GF(p)`-entailed by the formula (sound to conjoin) and new. An inconsistent
/// core instead [`AffinePForced::Refuted`]s (certified).
pub fn affine_p_forced(num_bool_vars: usize, clauses: &[Vec<Lit>]) -> AffinePForced {
    let Some(rec) = modp::recover_from_cnf(num_bool_vars, clauses) else {
        return AffinePForced::Unchanged;
    };
    if !modp::is_prime(rec.modulus) {
        return AffinePForced::Unchanged; // a composite modulus is the affine_m_forced path — solve_space needs a field
    }
    let Some(ss) = modp::solve_space(&rec.equations, rec.num_vars, rec.modulus) else {
        return AffinePForced::Refuted(crate::xor_drat::emit_modp_drat(num_bool_vars, clauses));
    };
    let p = rec.modulus;
    let key = |c: &[Lit]| -> Vec<(u32, bool)> {
        let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive())).collect();
        k.sort_unstable();
        k
    };
    let existing: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| key(c)).collect();
    let mut out: Vec<Vec<Lit>> = Vec::new();
    let mut push_new = |c: Vec<Lit>| {
        if !existing.contains(&key(&c)) {
            out.push(c);
        }
    };
    // The kernel "column" of a variable — which kernel directions move it.
    let col = |g: usize| -> Vec<u64> { ss.kernel_basis.iter().map(|kv| kv[g]).collect() };

    // Forced variables (zero column) → one-hot units; everything else is grouped by its column up to a
    // GF(p) scalar (projective equivalence) — that is exactly the linked relation.
    let mut classes: std::collections::HashMap<Vec<u64>, Vec<usize>> =
        std::collections::HashMap::new();
    for g in 0..rec.num_vars {
        let c = col(g);
        if c.iter().all(|&x| x == 0) {
            let val = ss.particular[g];
            for (v, &bvar) in rec.groups[g].iter().enumerate() {
                push_new(vec![if v as u64 == val {
                    Lit::pos(bvar)
                } else {
                    Lit::neg(bvar)
                }]);
            }
        } else {
            let scale = gfp_inv(*c.iter().find(|&&x| x != 0).unwrap(), p); // normalize: first nonzero ↦ 1
            classes
                .entry(c.iter().map(|&x| gfp_mul(x, scale, p)).collect())
                .or_default()
                .push(g);
        }
    }
    let mut class_list: Vec<Vec<usize>> = classes
        .into_values()
        .map(|mut v| {
            v.sort_unstable();
            v
        })
        .collect();
    class_list.sort_unstable_by_key(|c| c[0]);
    for members in &class_list {
        if members.len() < 2 {
            continue; // a lone variable carries no link
        }
        let rep = members[0];
        let s_rep = *col(rep).iter().find(|&&x| x != 0).unwrap();
        for &g in &members[1..] {
            let s_g = *col(g).iter().find(|&&x| x != 0).unwrap();
            let c_g = gfp_mul(s_g, gfp_inv(s_rep, p), p); // x_g = c_g·x_rep + d_g
            let d_g = gfp_sub(ss.particular[g], gfp_mul(c_g, ss.particular[rep], p), p);
            let inv_c = gfp_inv(c_g, p);
            for v in 0..p {
                let sv = gfp_mul(gfp_sub(v, d_g, p), inv_c, p); // b(g,v) ↔ b(rep, (v−d)·c⁻¹)
                let (bg, br) = (rec.groups[g][v as usize], rec.groups[rep][sv as usize]);
                push_new(vec![Lit::neg(bg), Lit::pos(br)]);
                push_new(vec![Lit::pos(bg), Lit::neg(br)]);
            }
        }
    }

    if out.is_empty() {
        AffinePForced::Unchanged
    } else {
        AffinePForced::Forced(out)
    }
}

/// `|AGL(n, ℤ/m)|` for **squarefree** `m`: by CRT (`ℤ/m ≅ ∏ GF(pᵢ)`) the affine group factors as
/// `∏ AGL(n, pᵢ)`, so the order is the product of the prime orders. `None` if `m` is not squarefree.
pub fn agl_m_order(n: u32, m: u64) -> Option<u128> {
    Some(
        crate::modm::squarefree_primes(m)?
            .iter()
            .map(|&p| agl_p_order(n, p))
            .product(),
    )
}

/// Combine residues with pairwise-coprime moduli into `(value, ∏ moduli)` by incremental CRT.
fn crt_combine(residues: &[(u64, u64)]) -> (u64, u64) {
    let (mut acc_r, mut acc_m): (i128, i128) = (0, 1);
    for &(r, modu) in residues {
        let modu = modu as i128;
        let diff = (r as i128 - acc_r).rem_euclid(modu);
        let t = (diff * mod_inverse(acc_m.rem_euclid(modu) as u64, modu as u64) as i128)
            .rem_euclid(modu);
        acc_r += acc_m * t;
        acc_m *= modu;
        acc_r = acc_r.rem_euclid(acc_m);
    }
    (acc_r as u64, acc_m as u64)
}

/// The inverse of `a` mod `m` (extended Euclid) — `a` must be coprime to `m`.
fn mod_inverse(a: u64, m: u64) -> u64 {
    let (mut old_r, mut r): (i128, i128) = (a as i128, m as i128);
    let (mut old_s, mut s): (i128, i128) = (1, 0);
    while r != 0 {
        let q = old_r / r;
        (old_r, r) = (r, old_r - q * r);
        (old_s, s) = (s, old_s - q * s);
    }
    old_s.rem_euclid(m as i128) as u64
}

/// **The composite `ℤ/m` SAT-side break** (squarefree `m`), the full mod-`m` analogue of
/// [`affine_p_forced`] via CRT (`ℤ/m ≅ ∏ GF(pᵢ)`). Solve the system over each prime field, then for each
/// variable combine the per-prime structure:
///
/// * **Forced / partially forced** — where the kernel pins `x_g` mod a set `S` of primes, `x_g` is
///   congruent to one residue mod `∏S`; forbid every one-hot value off that residue (a single allowed
///   value when `S` is *all* primes — the fully-forced units).
/// * **Linked** — variables free mod *every* prime whose kernel columns are scalar-proportional mod each
///   prime are linked over `ℤ/m` by `x_g = c·x_rep + d` with `c = CRT(cᵢ)`, `d = CRT(dᵢ)`; lift to
///   value-permuted bit-equivalences `b(g,v) ↔ b(rep, (v−d)·c⁻¹ mod m)`.
///
/// Every emitted clause is `ℤ/m`-entailed and new; an inconsistent prime field [`AffinePForced::Refuted`]s.
/// A prime modulus defers to [`affine_p_forced`]; a non-squarefree `m` (prime-power Smith case) is untouched.
pub fn affine_m_forced(num_bool_vars: usize, clauses: &[Vec<Lit>]) -> AffinePForced {
    let Some(rec) = modp::recover_from_cnf(num_bool_vars, clauses) else {
        return AffinePForced::Unchanged;
    };
    let m = rec.modulus;
    if modp::is_prime(m) {
        return AffinePForced::Unchanged; // the prime path is affine_p_forced's
    }
    let Some(primes) = crate::modm::squarefree_primes(m) else {
        return prime_power_forced(num_bool_vars, &rec, clauses); // non-squarefree (prime-power) branch
    };
    let mut spaces: Vec<crate::modp::SolutionSpaceP> = Vec::with_capacity(primes.len());
    for &p in &primes {
        let Some(ss) = modp::solve_space(&rec.equations, rec.num_vars, p) else {
            return AffinePForced::Refuted(crate::xor_drat::emit_modp_drat(num_bool_vars, clauses));
        };
        spaces.push(ss);
    }

    let key = |c: &[Lit]| -> Vec<(u32, bool)> {
        let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive())).collect();
        k.sort_unstable();
        k
    };
    let existing: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| key(c)).collect();
    let mut out: Vec<Vec<Lit>> = Vec::new();
    let push_new = |c: Vec<Lit>, out: &mut Vec<Vec<Lit>>| {
        if !existing.contains(&key(&c)) {
            out.push(c);
        }
    };
    let col = |i: usize, g: usize| -> Vec<u64> {
        spaces[i].kernel_basis.iter().map(|kk| kk[g]).collect()
    };
    let forced_res = |i: usize, g: usize| -> Option<u64> {
        col(i, g)
            .iter()
            .all(|&x| x == 0)
            .then(|| spaces[i].particular[g])
    };

    // PART 1 — forced / partially forced: `x_g ≡ res (mod q)` over the primes that pin it.
    for g in 0..rec.num_vars {
        let constraints: Vec<(u64, u64)> = primes
            .iter()
            .enumerate()
            .filter_map(|(i, &p)| forced_res(i, g).map(|v| (v, p)))
            .collect();
        if constraints.is_empty() {
            continue;
        }
        let (res, q) = crt_combine(&constraints);
        let fully = constraints.len() == primes.len();
        for (v, &bvar) in rec.groups[g].iter().enumerate() {
            if (v as u64) % q != res {
                push_new(vec![Lit::neg(bvar)], &mut out); // x_g ≠ v (off the forced residue)
            } else if fully {
                push_new(vec![Lit::pos(bvar)], &mut out); // the single allowed value
            }
        }
    }

    // PART 2 — composite links: variables free mod EVERY prime, grouped by per-prime projective signature.
    let mut classes: std::collections::HashMap<Vec<Vec<u64>>, Vec<usize>> =
        std::collections::HashMap::new();
    for g in 0..rec.num_vars {
        if (0..primes.len()).any(|i| forced_res(i, g).is_some()) {
            continue;
        }
        let sig: Vec<Vec<u64>> = (0..primes.len())
            .map(|i| {
                let (p, c) = (primes[i], col(i, g));
                let s = gfp_inv(*c.iter().find(|&&x| x != 0).unwrap(), p);
                c.iter().map(|&x| gfp_mul(x, s, p)).collect()
            })
            .collect();
        classes.entry(sig).or_default().push(g);
    }
    let mut class_list: Vec<Vec<usize>> = classes
        .into_values()
        .map(|mut v| {
            v.sort_unstable();
            v
        })
        .collect();
    class_list.sort_unstable_by_key(|c| c[0]);
    for members in &class_list {
        if members.len() < 2 {
            continue;
        }
        let rep = members[0];
        for &g in &members[1..] {
            let (mut cs, mut ds): (Vec<(u64, u64)>, Vec<(u64, u64)>) = (Vec::new(), Vec::new());
            for (i, &p) in primes.iter().enumerate() {
                let s_rep = *col(i, rep).iter().find(|&&x| x != 0).unwrap();
                let s_g = *col(i, g).iter().find(|&&x| x != 0).unwrap();
                let c_i = gfp_mul(s_g, gfp_inv(s_rep, p), p);
                let d_i = gfp_sub(
                    spaces[i].particular[g],
                    gfp_mul(c_i, spaces[i].particular[rep], p),
                    p,
                );
                cs.push((c_i, p));
                ds.push((d_i, p));
            }
            let (c, _) = crt_combine(&cs);
            let (d, _) = crt_combine(&ds);
            let inv_c = mod_inverse(c, m);
            for v in 0..m {
                let sv = ((v + m - d) % m) * inv_c % m; // (v − d)·c⁻¹ mod m
                let (bg, br) = (rec.groups[g][v as usize], rec.groups[rep][sv as usize]);
                push_new(vec![Lit::neg(bg), Lit::pos(br)], &mut out);
                push_new(vec![Lit::pos(bg), Lit::neg(br)], &mut out);
            }
        }
    }

    if out.is_empty() {
        AffinePForced::Unchanged
    } else {
        AffinePForced::Forced(out)
    }
}

/// Greatest common divisor over `u64`.
fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// `a·b mod q` (helper for the ring-link arithmetic).
fn cg_mul(a: u64, b: u64, q: u64) -> u64 {
    a % q * (b % q) % q
}

/// The kernel "column" of variable `g` in a component's solution space.
fn kcol(ss: &crate::modm::SolutionSpaceM, g: usize) -> Vec<u64> {
    ss.kernel_basis.iter().map(|kk| kk[g]).collect()
}

/// The composite break structure shared by the inject ([`prime_power_forced`]) and eliminate
/// ([`affine_m_canonicalize`]) paths over a non-prime modulus: per prime-power component its Smith solution
/// space, and per variable its tightest congruence `(res, modu)` plus (when free everywhere) its ring link
/// `(rep, c, d)`.
struct BreakStructure {
    residue: Vec<(u64, u64)>,
    link: Vec<Option<(usize, u64, u64)>>,
}

enum BreakOutcome {
    Inconsistent,
    Structure(BreakStructure),
}

/// Compute the [`BreakStructure`] by Smith normal form per prime-power component + CRT. `None` on overflow.
fn composite_break_structure(
    eqs: &[ModpEquation],
    num_vars: usize,
    m: u64,
) -> Option<BreakOutcome> {
    use crate::modm::PrimePowerSpace;
    let factors = crate::modm::prime_power_factorize(m)?;
    let mut spaces: Vec<(u64, u64, crate::modm::SolutionSpaceM)> =
        Vec::with_capacity(factors.len());
    for (p, k) in factors {
        match crate::modm::solve_space_prime_power(eqs, num_vars, p, k) {
            None => return None,
            Some(PrimePowerSpace::Inconsistent) => return Some(BreakOutcome::Inconsistent),
            Some(PrimePowerSpace::Space(ss)) => spaces.push((p, p.pow(k), ss)),
        }
    }

    // Each variable's tightest congruence: the CRT of its per-component coset (modulus gcd(pᵏ, kernel)).
    let residue: Vec<(u64, u64)> = (0..num_vars)
        .map(|g| {
            let pairs: Vec<(u64, u64)> = spaces
                .iter()
                .filter_map(|(_, q, ss)| {
                    let d = kcol(ss, g).iter().fold(*q, |acc, &x| gcd_u64(acc, x));
                    (d > 1).then(|| (ss.particular[g] % d, d))
                })
                .collect();
            if pairs.is_empty() {
                (0, 1)
            } else {
                (
                    crt_combine(&pairs).0,
                    pairs.iter().map(|&(_, d)| d).product(),
                )
            }
        })
        .collect();

    // Ring links: free-everywhere variables grouped by per-component canonical column (first-unit-normalized).
    let mut classes: std::collections::HashMap<Vec<Vec<u64>>, Vec<usize>> =
        std::collections::HashMap::new();
    'g: for g in 0..num_vars {
        let mut sig: Vec<Vec<u64>> = Vec::with_capacity(spaces.len());
        for (p, q, ss) in &spaces {
            let c = kcol(ss, g);
            let Some(piv) = c.iter().position(|&x| x % p != 0) else {
                continue 'g;
            };
            let inv = mod_inverse(c[piv], *q);
            sig.push(c.iter().map(|&x| x * inv % q).collect());
        }
        classes.entry(sig).or_default().push(g);
    }
    let mut link = vec![None; num_vars];
    let mut class_list: Vec<Vec<usize>> = classes
        .into_values()
        .map(|mut v| {
            v.sort_unstable();
            v
        })
        .collect();
    class_list.sort_unstable_by_key(|c| c[0]);
    for members in &class_list {
        if members.len() < 2 {
            continue;
        }
        let rep = members[0];
        for &g in &members[1..] {
            let (mut cs, mut ds): (Vec<(u64, u64)>, Vec<(u64, u64)>) = (Vec::new(), Vec::new());
            for (p, q, ss) in &spaces {
                let (cg, cr) = (kcol(ss, g), kcol(ss, rep));
                let piv = cr.iter().position(|&x| x % p != 0).unwrap();
                let c_c = cg[piv] * mod_inverse(cr[piv], *q) % q;
                let d_c = (ss.particular[g] + q - cg_mul(c_c, ss.particular[rep], *q)) % q;
                cs.push((c_c, *q));
                ds.push((d_c, *q));
            }
            link[g] = Some((rep, crt_combine(&cs).0, crt_combine(&ds).0));
        }
    }

    Some(BreakOutcome::Structure(BreakStructure { residue, link }))
}

/// The **prime-power / mixed** (non-squarefree) branch of [`affine_m_forced`] — the full INJECT break over a
/// ring. Computes the [`composite_break_structure`] and emits its consequences: forbid every off-residue
/// one-hot value (forced ⇒ a single value, partial ⇒ a value-subset), and the ring links as value-permuted
/// bit-equivalences `b(g,v) ↔ b(rep, (v−d)·c⁻¹)`. An inconsistent component refutes.
fn prime_power_forced(
    num_bool_vars: usize,
    rec: &modp::ModpRecovery,
    clauses: &[Vec<Lit>],
) -> AffinePForced {
    let m = rec.modulus;
    let st = match composite_break_structure(&rec.equations, rec.num_vars, m) {
        None => return AffinePForced::Unchanged,
        Some(BreakOutcome::Inconsistent) => {
            return AffinePForced::Refuted(crate::xor_drat::emit_modp_drat(num_bool_vars, clauses));
        }
        Some(BreakOutcome::Structure(s)) => s,
    };
    let key = |c: &[Lit]| -> Vec<(u32, bool)> {
        let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive())).collect();
        k.sort_unstable();
        k
    };
    let existing: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| key(c)).collect();
    let mut out: Vec<Vec<Lit>> = Vec::new();
    let push_new = |c: Vec<Lit>, out: &mut Vec<Vec<Lit>>| {
        if !existing.contains(&key(&c)) {
            out.push(c);
        }
    };
    for (g, &(res, modu)) in st.residue.iter().enumerate() {
        if modu <= 1 {
            continue;
        }
        for (v, &bvar) in rec.groups[g].iter().enumerate() {
            if (v as u64) % modu != res {
                push_new(vec![Lit::neg(bvar)], &mut out);
            } else if modu == m {
                push_new(vec![Lit::pos(bvar)], &mut out);
            }
        }
    }
    for g in 0..rec.num_vars {
        if let Some((rep, c, d)) = st.link[g] {
            let inv_c = mod_inverse(c, m);
            for v in 0..m {
                let sv = (v + m - d) % m * inv_c % m;
                let (bg, br) = (rec.groups[g][v as usize], rec.groups[rep][sv as usize]);
                push_new(vec![Lit::neg(bg), Lit::pos(br)], &mut out);
                push_new(vec![Lit::pos(bg), Lit::neg(br)], &mut out);
            }
        }
    }
    if out.is_empty() {
        AffinePForced::Unchanged
    } else {
        AffinePForced::Forced(out)
    }
}

/// How an eliminated one-hot bit is recovered from the reduced model.
#[derive(Clone, Debug)]
enum BoolSub {
    /// Pinned to a constant (a forced group's bit).
    Const(bool),
    /// Survives as reduced variable `new_index` (a free/representative group's bit).
    Survive(u32),
    /// Equal to reduced variable `new_index` — a linked group's bit aliased to its representative's,
    /// value-permuted (`b(g,v) = b(rep, σ(v))`).
    Alias(u32),
}

/// The reduced one-hot formula over the surviving groups, plus the lift map.
#[derive(Clone, Debug)]
pub struct AffinePCanonical {
    pub num_vars: usize,
    pub clauses: Vec<Vec<Lit>>,
    sub: Vec<BoolSub>,
}

impl AffinePCanonical {
    /// Lift a model of the reduced formula back over the original one-hot bits.
    pub fn lift(&self, reduced_model: &[bool]) -> Vec<bool> {
        self.sub
            .iter()
            .map(|s| match *s {
                BoolSub::Const(c) => c,
                BoolSub::Survive(ni) | BoolSub::Alias(ni) => reduced_model[ni as usize],
            })
            .collect()
    }
}

/// The outcome of the `GF(p)` canonical elimination.
pub enum AffinePCanon {
    Refuted(Option<Vec<Vec<Lit>>>),
    Canonical(AffinePCanonical),
    Unchanged,
}

/// **The `GF(p)` canonical RREF break (elimination).** The one-hot analogue of
/// [`crate::affine::affine_canonicalize`]: recover the mod-`p` system and solve its space, then physically
/// *eliminate* the determined one-hot groups rather than merely injecting their consequences. A **forced**
/// group's bits become constants; a **linked** group's bits alias to its representative's, value-permuted
/// (`b(g,v) = b(rep, σ(v))` with `σ(v) = (v−d)·c⁻¹`). The result is an equisatisfiable formula over the
/// surviving (free/representative) groups, with [`AffinePCanonical::lift`] to recover the eliminated bits.
/// An inconsistent core [`AffinePCanon::Refuted`]s; a composite modulus is left to [`affine_m_forced`].
pub fn affine_p_canonicalize(num_bool_vars: usize, clauses: &[Vec<Lit>]) -> AffinePCanon {
    let Some(rec) = modp::recover_from_cnf(num_bool_vars, clauses) else {
        return AffinePCanon::Unchanged;
    };
    let p = rec.modulus;
    if !modp::is_prime(p) {
        return AffinePCanon::Unchanged;
    }
    let Some(ss) = modp::solve_space(&rec.equations, rec.num_vars, p) else {
        return AffinePCanon::Refuted(crate::xor_drat::emit_modp_drat(num_bool_vars, clauses));
    };

    // Classify each GF(p) variable: forced to a constant, or a member of a projective (linked) class.
    enum Role {
        Forced(u64),
        Survive,
        Linked { rep: usize, c: u64, d: u64 },
    }
    let col = |g: usize| -> Vec<u64> { ss.kernel_basis.iter().map(|k| k[g]).collect() };
    let mut role: Vec<Role> = Vec::with_capacity(rec.num_vars);
    let mut classes: std::collections::HashMap<Vec<u64>, Vec<usize>> =
        std::collections::HashMap::new();
    for g in 0..rec.num_vars {
        let c = col(g);
        if c.iter().all(|&x| x == 0) {
            role.push(Role::Forced(ss.particular[g]));
        } else {
            let s = gfp_inv(*c.iter().find(|&&x| x != 0).unwrap(), p);
            classes
                .entry(c.iter().map(|&x| gfp_mul(x, s, p)).collect())
                .or_default()
                .push(g);
            role.push(Role::Survive); // provisional; linked members are rewritten below
        }
    }
    for members in classes.values() {
        let mut ms = members.clone();
        ms.sort_unstable();
        let rep = ms[0];
        let s_rep = *col(rep).iter().find(|&&x| x != 0).unwrap();
        for &g in &ms[1..] {
            let s_g = *col(g).iter().find(|&&x| x != 0).unwrap();
            let c = gfp_mul(s_g, gfp_inv(s_rep, p), p);
            let d = gfp_sub(ss.particular[g], gfp_mul(c, ss.particular[rep], p), p);
            role[g] = Role::Linked { rep, c, d };
        }
    }

    // Assign reduced indices to the surviving bits (free/representative groups, then any non-group bits).
    let mut in_group = vec![false; num_bool_vars];
    for grp in &rec.groups {
        for &b in grp {
            in_group[b as usize] = true;
        }
    }
    let mut new_idx: Vec<Option<u32>> = vec![None; num_bool_vars];
    let mut next = 0u32;
    for g in 0..rec.num_vars {
        if matches!(role[g], Role::Survive) {
            for &b in &rec.groups[g] {
                new_idx[b as usize] = Some(next);
                next += 1;
            }
        }
    }
    for (b, slot) in new_idx.iter_mut().enumerate() {
        if !in_group[b] {
            *slot = Some(next);
            next += 1;
        }
    }
    let reduced_nv = next as usize;

    // Build the per-bit substitution.
    let mut sub = vec![BoolSub::Const(false); num_bool_vars];
    for (b, slot) in new_idx.iter().enumerate() {
        if let Some(ni) = slot {
            sub[b] = BoolSub::Survive(*ni); // surviving + non-group bits
        }
    }
    for g in 0..rec.num_vars {
        match role[g] {
            Role::Forced(val) => {
                for (v, &b) in rec.groups[g].iter().enumerate() {
                    sub[b as usize] = BoolSub::Const(v as u64 == val);
                }
            }
            Role::Survive => {} // already Survive
            Role::Linked { rep, c, d } => {
                let inv_c = gfp_inv(c, p);
                for (v, &b) in rec.groups[g].iter().enumerate() {
                    let sv = gfp_mul(gfp_sub(v as u64, d, p), inv_c, p); // σ(v) = (v − d)·c⁻¹
                    let rep_b = rec.groups[rep][sv as usize];
                    sub[b as usize] = BoolSub::Alias(new_idx[rep_b as usize].unwrap());
                }
            }
        }
    }
    if !sub
        .iter()
        .any(|s| matches!(s, BoolSub::Const(_) | BoolSub::Alias(_)))
    {
        return AffinePCanon::Unchanged; // nothing eliminated
    }

    // Apply the substitution to every clause, staying in CNF.
    let mut out: Vec<Vec<Lit>> = Vec::new();
    'clause: for c in clauses {
        let mut seen: std::collections::HashMap<u32, bool> = std::collections::HashMap::new();
        let mut lits: Vec<Lit> = Vec::new();
        for l in c {
            let (ni, pol) = match sub[l.var() as usize] {
                BoolSub::Const(cst) => {
                    if cst == l.is_positive() {
                        continue 'clause; // literal true ⇒ clause satisfied
                    }
                    continue; // literal false ⇒ drop it
                }
                BoolSub::Survive(ni) | BoolSub::Alias(ni) => (ni, l.is_positive()),
            };
            match seen.get(&ni) {
                Some(&prev) if prev != pol => continue 'clause, // tautology
                Some(_) => continue,                            // duplicate
                None => {
                    seen.insert(ni, pol);
                    lits.push(Lit::new(ni, pol));
                }
            }
        }
        out.push(lits);
    }
    AffinePCanon::Canonical(AffinePCanonical {
        num_vars: reduced_nv,
        clauses: out,
        sub,
    })
}

/// **The composite-ring canonical elimination.** The `ℤ/m` analogue of [`affine_p_canonicalize`] and the
/// eliminate twin of `prime_power_forced`: over a non-prime modulus, recover the mod-`m` system, take its
/// `composite_break_structure`, and physically *eliminate* the determined one-hot structure rather than
/// merely injecting it. A **forced** group (`x_g` pinned to a single value) becomes constants; a **partially
/// forced** group (`x_g` confined to a coset `v ≡ res (mod modu)`, `1 < modu < m`) keeps only its on-coset
/// bits, the rest pinned false; a **linked** group's bits alias its representative's, value-permuted
/// (`b(g,v) = b(rep, σ(v))`, `σ(v) = (v−d)·c⁻¹ mod m`). The result is equisatisfiable over the surviving
/// bits, with [`AffinePCanonical::lift`] to recover the rest. An inconsistent component
/// [`AffinePCanon::Refuted`]s; a prime modulus is left to [`affine_p_canonicalize`].
pub fn affine_m_canonicalize(num_bool_vars: usize, clauses: &[Vec<Lit>]) -> AffinePCanon {
    let Some(rec) = modp::recover_from_cnf(num_bool_vars, clauses) else {
        return AffinePCanon::Unchanged;
    };
    let m = rec.modulus;
    if modp::is_prime(m) {
        return AffinePCanon::Unchanged; // prime ⇒ the field path
    }
    let st = match composite_break_structure(&rec.equations, rec.num_vars, m) {
        None => return AffinePCanon::Unchanged,
        Some(BreakOutcome::Inconsistent) => {
            return AffinePCanon::Refuted(crate::xor_drat::emit_modp_drat(num_bool_vars, clauses));
        }
        Some(BreakOutcome::Structure(s)) => s,
    };

    enum Role {
        Forced(u64),
        Partial { res: u64, modu: u64 },
        Survive,
        Linked { rep: usize, c: u64, d: u64 },
    }
    let role: Vec<Role> = (0..rec.num_vars)
        .map(|g| {
            let (res, modu) = st.residue[g];
            if modu == m {
                Role::Forced(res)
            } else if modu > 1 {
                Role::Partial { res, modu }
            } else if let Some((rep, c, d)) = st.link[g] {
                Role::Linked { rep, c, d }
            } else {
                Role::Survive
            }
        })
        .collect();

    // Assign reduced indices to the surviving bits: free groups' bits, partial groups' on-coset bits, then
    // any non-group bits.
    let mut in_group = vec![false; num_bool_vars];
    for grp in &rec.groups {
        for &b in grp {
            in_group[b as usize] = true;
        }
    }
    let mut new_idx: Vec<Option<u32>> = vec![None; num_bool_vars];
    let mut next = 0u32;
    for g in 0..rec.num_vars {
        match role[g] {
            Role::Survive => {
                for &b in &rec.groups[g] {
                    new_idx[b as usize] = Some(next);
                    next += 1;
                }
            }
            Role::Partial { res, modu } => {
                for (v, &b) in rec.groups[g].iter().enumerate() {
                    if (v as u64) % modu == res {
                        new_idx[b as usize] = Some(next);
                        next += 1;
                    }
                }
            }
            _ => {}
        }
    }
    for (b, slot) in new_idx.iter_mut().enumerate() {
        if !in_group[b] {
            *slot = Some(next);
            next += 1;
        }
    }
    let reduced_nv = next as usize;

    // Build the per-bit substitution. Surviving (free, on-coset, non-group) bits are set above; forced bits
    // become constants, off-coset partial bits false, linked bits value-permuted aliases.
    let mut sub = vec![BoolSub::Const(false); num_bool_vars];
    for (b, slot) in new_idx.iter().enumerate() {
        if let Some(ni) = slot {
            sub[b] = BoolSub::Survive(*ni);
        }
    }
    for g in 0..rec.num_vars {
        match role[g] {
            Role::Forced(val) => {
                for (v, &b) in rec.groups[g].iter().enumerate() {
                    sub[b as usize] = BoolSub::Const(v as u64 == val);
                }
            }
            Role::Partial { .. } | Role::Survive => {} // already set by the surviving-bit pass
            Role::Linked { rep, c, d } => {
                let inv_c = mod_inverse(c, m);
                for (v, &b) in rec.groups[g].iter().enumerate() {
                    let sv = (v as u64 + m - d) % m * inv_c % m; // σ(v) = (v − d)·c⁻¹
                    let rep_b = rec.groups[rep][sv as usize];
                    sub[b as usize] = BoolSub::Alias(new_idx[rep_b as usize].unwrap());
                }
            }
        }
    }
    if !sub
        .iter()
        .any(|s| matches!(s, BoolSub::Const(_) | BoolSub::Alias(_)))
    {
        return AffinePCanon::Unchanged; // nothing eliminated
    }

    // Apply the substitution to every clause, staying in CNF.
    let mut out: Vec<Vec<Lit>> = Vec::new();
    'clause: for c in clauses {
        let mut seen: std::collections::HashMap<u32, bool> = std::collections::HashMap::new();
        let mut lits: Vec<Lit> = Vec::new();
        for l in c {
            let (ni, pol) = match sub[l.var() as usize] {
                BoolSub::Const(cst) => {
                    if cst == l.is_positive() {
                        continue 'clause; // literal true ⇒ clause satisfied
                    }
                    continue; // literal false ⇒ drop it
                }
                BoolSub::Survive(ni) | BoolSub::Alias(ni) => (ni, l.is_positive()),
            };
            match seen.get(&ni) {
                Some(&prev) if prev != pol => continue 'clause, // tautology
                Some(_) => continue,                            // duplicate
                None => {
                    seen.insert(ni, pol);
                    lits.push(Lit::new(ni, pol));
                }
            }
        }
        out.push(lits);
    }
    AffinePCanon::Canonical(AffinePCanonical {
        num_vars: reduced_nv,
        clauses: out,
        sub,
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/affine_gfp/tests.rs"]
mod tests;
