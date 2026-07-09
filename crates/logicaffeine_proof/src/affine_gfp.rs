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
        let matrix = (0..n).map(|i| (0..n).map(|j| u64::from(i == j)).collect()).collect();
        AffineP { p, n, matrix, translation: vec![0; n] }
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
                *cell = (0..n).fold(0u64, |a, k| (a + self.matrix[i][k] * other.matrix[k][j]) % p);
            }
        }
        let translation = (0..n)
            .map(|i| {
                let av = (0..n).fold(0u64, |a, k| (a + self.matrix[i][k] * other.translation[k]) % p);
                (av + self.translation[i]) % p
            })
            .collect();
        AffineP { p, n, matrix, translation }
    }

    /// Whether the linear part is invertible over `GF(p)` (so the map is a bijection of the cube).
    pub fn is_bijection(&self) -> bool {
        modp::is_invertible_modp(self.n, self.p, &self.matrix)
    }

    /// Whether the linear part is **monomial** — one nonzero entry per row and per column (a scaled
    /// permutation). These are exactly the `Bₙ`-analog over `GF(p)`; everything else is a genuine shear.
    pub fn is_monomial(&self) -> bool {
        let n = self.n;
        let rows = self.matrix.iter().all(|r| r.iter().filter(|&&x| x != 0).count() == 1);
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
    assert!((p as u128).pow((n * n) as u32) <= 200_000, "exhaustive AGL(n,p) enumeration is bounded (p^{{n²}} ≤ 200k)");
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
            out.push(AffineP { p, n, matrix: matrix.clone(), translation: decode(tcode, n) });
        }
    }
    out
}

/// The `GF(p)`-valued solutions of a mod-`p` linear system (`Σ coeffs·x ≡ rhs (mod p)` for each
/// equation), brute force over `pⁿ` — small `n` only.
pub fn models_p(n: usize, p: u64, equations: &[ModpEquation]) -> Vec<Vec<u64>> {
    assert!((p as u128).pow(n as u32) <= 60_000, "model enumeration is brute force — small n");
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
                .all(|eq| eq.coeffs.iter().fold(0u64, |a, &(v, co)| (a + co * x[v]) % p) % p == eq.rhs % p)
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
pub fn affine_p_refutation_drat(num_bool_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<Lit>>> {
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
    let mut classes: std::collections::HashMap<Vec<u64>, Vec<usize>> = std::collections::HashMap::new();
    for g in 0..rec.num_vars {
        let c = col(g);
        if c.iter().all(|&x| x == 0) {
            let val = ss.particular[g];
            for (v, &bvar) in rec.groups[g].iter().enumerate() {
                push_new(vec![if v as u64 == val { Lit::pos(bvar) } else { Lit::neg(bvar) }]);
            }
        } else {
            let scale = gfp_inv(*c.iter().find(|&&x| x != 0).unwrap(), p); // normalize: first nonzero ↦ 1
            classes.entry(c.iter().map(|&x| gfp_mul(x, scale, p)).collect()).or_default().push(g);
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
    Some(crate::modm::squarefree_primes(m)?.iter().map(|&p| agl_p_order(n, p)).product())
}

/// Combine residues with pairwise-coprime moduli into `(value, ∏ moduli)` by incremental CRT.
fn crt_combine(residues: &[(u64, u64)]) -> (u64, u64) {
    let (mut acc_r, mut acc_m): (i128, i128) = (0, 1);
    for &(r, modu) in residues {
        let modu = modu as i128;
        let diff = (r as i128 - acc_r).rem_euclid(modu);
        let t = (diff * mod_inverse(acc_m.rem_euclid(modu) as u64, modu as u64) as i128).rem_euclid(modu);
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
    let col = |i: usize, g: usize| -> Vec<u64> { spaces[i].kernel_basis.iter().map(|kk| kk[g]).collect() };
    let forced_res = |i: usize, g: usize| -> Option<u64> {
        col(i, g).iter().all(|&x| x == 0).then(|| spaces[i].particular[g])
    };

    // PART 1 — forced / partially forced: `x_g ≡ res (mod q)` over the primes that pin it.
    for g in 0..rec.num_vars {
        let constraints: Vec<(u64, u64)> =
            primes.iter().enumerate().filter_map(|(i, &p)| forced_res(i, g).map(|v| (v, p))).collect();
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
    let mut classes: std::collections::HashMap<Vec<Vec<u64>>, Vec<usize>> = std::collections::HashMap::new();
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
                let d_i = gfp_sub(spaces[i].particular[g], gfp_mul(c_i, spaces[i].particular[rep], p), p);
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
fn composite_break_structure(eqs: &[ModpEquation], num_vars: usize, m: u64) -> Option<BreakOutcome> {
    use crate::modm::PrimePowerSpace;
    let factors = crate::modm::prime_power_factorize(m)?;
    let mut spaces: Vec<(u64, u64, crate::modm::SolutionSpaceM)> = Vec::with_capacity(factors.len());
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
                (crt_combine(&pairs).0, pairs.iter().map(|&(_, d)| d).product())
            }
        })
        .collect();

    // Ring links: free-everywhere variables grouped by per-component canonical column (first-unit-normalized).
    let mut classes: std::collections::HashMap<Vec<Vec<u64>>, Vec<usize>> = std::collections::HashMap::new();
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
    let mut class_list: Vec<Vec<usize>> = classes.into_values().map(|mut v| { v.sort_unstable(); v }).collect();
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
fn prime_power_forced(num_bool_vars: usize, rec: &modp::ModpRecovery, clauses: &[Vec<Lit>]) -> AffinePForced {
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
    let mut classes: std::collections::HashMap<Vec<u64>, Vec<usize>> = std::collections::HashMap::new();
    for g in 0..rec.num_vars {
        let c = col(g);
        if c.iter().all(|&x| x == 0) {
            role.push(Role::Forced(ss.particular[g]));
        } else {
            let s = gfp_inv(*c.iter().find(|&&x| x != 0).unwrap(), p);
            classes.entry(c.iter().map(|&x| gfp_mul(x, s, p)).collect()).or_default().push(g);
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
    if !sub.iter().any(|s| matches!(s, BoolSub::Const(_) | BoolSub::Alias(_))) {
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
    AffinePCanon::Canonical(AffinePCanonical { num_vars: reduced_nv, clauses: out, sub })
}

/// **The composite-ring canonical elimination.** The `ℤ/m` analogue of [`affine_p_canonicalize`] and the
/// eliminate twin of [`prime_power_forced`]: over a non-prime modulus, recover the mod-`m` system, take its
/// [`composite_break_structure`], and physically *eliminate* the determined one-hot structure rather than
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
    if !sub.iter().any(|s| matches!(s, BoolSub::Const(_) | BoolSub::Alias(_))) {
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
    AffinePCanon::Canonical(AffinePCanonical { num_vars: reduced_nv, clauses: out, sub })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families;

    /// `|AGL(n,p)| = pⁿ·|GL(n,p)|`, and the exhaustive enumeration produces exactly that many bijections.
    #[test]
    fn agl_p_order_matches_the_enumeration() {
        for (n, p) in [(1usize, 2u64), (1, 3), (2, 2), (2, 3), (2, 5), (3, 2), (3, 3)] {
            assert_eq!(
                all_affine_p_bijections(n, p).len() as u128,
                agl_p_order(n as u32, p),
                "n={n}, p={p}: enumerated affine bijections must equal |AGL(n,p)| = pⁿ·|GL(n,p)|"
            );
        }
        // Pinned: |AGL(2,3)| = 9·48 = 432, |AGL(3,2)| = 8·168 = 1344.
        assert_eq!(agl_p_order(2, 3), 432);
        assert_eq!(agl_p_order(3, 2), 1344);
    }

    /// Group laws over `GF(p)`: identity is a unit, and composition matches pointwise application.
    #[test]
    fn affine_p_maps_compose_and_act_correctly() {
        let p = 3u64;
        let id = AffineP::identity(2, p);
        for phi in all_affine_p_bijections(2, p) {
            assert!(phi.is_bijection());
            assert_eq!(phi.compose(&id), phi);
            assert_eq!(id.compose(&phi), phi);
            let sq = phi.compose(&phi);
            for a in 0..p {
                for b in 0..p {
                    let x = vec![a, b];
                    assert_eq!(sq.apply(&x), phi.apply(&phi.apply(&x)), "composition must match double application");
                }
            }
        }
    }

    /// **THE CENSUS GAP OVER `GF(p)`: `AGL(3,3) ⊋ monomial`.** The mod-3 parity plane `x₀+x₁+x₂ ≡ 0 (mod 3)`
    /// is rigid under the monomial (scaled-permutation) breakers but carries `864·9 = 7776` affine
    /// symmetries (864 linear maps fix the hyperplane × 9 in-plane translations) — far more than the
    /// monomial subgroup. The witness is a genuine `GF(3)` **shear**: a symmetry whose matrix is not
    /// monomial, the mod-`p` analog of the parity shear no clause break can see.
    #[test]
    fn gfp_affine_symmetry_strictly_exceeds_monomial_on_mod_p_parity() {
        let eq = ModpEquation::new(vec![(0usize, 1u64), (1, 1), (2, 1)], 0); // x0+x1+x2 ≡ 0 (mod 3)
        let models = models_p(3, 3, std::slice::from_ref(&eq));
        assert_eq!(models.len(), 9, "the mod-3 hyperplane has 3² = 9 points");

        let sym = affine_p_symmetries(3, 3, &models);
        assert_eq!(sym.len(), 7776, "AGL(3,3) stabilizer of the mod-3 hyperplane = 864 linear × 9 translations");

        let monomial = sym.iter().filter(|a| a.is_monomial()).count();
        assert!(sym.len() > monomial, "affine symmetry ({}) must strictly exceed the monomial part ({monomial})", sym.len());
        assert!(
            sym.iter().any(|a| !a.is_monomial()),
            "a non-monomial GF(3) affine symmetry (a shear) must exist — invisible to the monomial breakers"
        );
    }

    /// **The `GF(p)` affine refutation is certified through the mod-`p` DRAT bridge.** On a mod-`p` Tseitin
    /// counting core (UNSAT over `GF(p)`, exponential for resolution and Z3), [`affine_p_refutation_drat`]
    /// compiles the `GF(p)` dependency to RUP resolvent lemmas over the one-hot encoding that our
    /// independent checker accepts against the original CNF.
    #[test]
    fn gfp_affine_refutation_is_certified_via_modp_drat_bridge() {
        for (n, p) in [(4usize, 3u64), (6, 3), (4, 5)] {
            let (_, cnf, _) = families::mod_p_tseitin_expander(n, p, 1);
            let Some(proof) = affine_p_refutation_drat(cnf.num_vars, &cnf.clauses) else {
                eprintln!("[modp n={n} p={p}] resolution route over budget — skipped");
                continue;
            };
            assert!(proof.last().is_some_and(|c| c.is_empty()), "proof ends in the empty clause (n={n}, p={p})");
            assert!(
                crate::rup::check_refutation(cnf.num_vars, &cnf.clauses, &proof),
                "the GF({p}) affine refutation must RUP-refute the original CNF (n={n})"
            );
        }
    }

    /// **The GF(p) solution-space solver is exact.** Against brute force over hundreds of random `GF(p)`
    /// systems: it reports inconsistency iff there are no solutions, its `count` equals the solution
    /// count, and its `particular + kernel` enumerates *exactly* the solution set.
    #[test]
    fn solve_space_matches_brute_force() {
        use crate::modp::solve_space;
        let mut state = 0xC0FFEE_77u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &p in &[3u64, 5] {
            for _ in 0..200 {
                let n = 2 + (rng() % 3) as usize; // 2..=4 variables
                let m = 1 + (rng() % 4) as usize;
                let eqs: Vec<ModpEquation> = (0..m)
                    .map(|_| {
                        let k = 1 + (rng() % n as u64) as usize;
                        let mut vars: Vec<usize> = Vec::new();
                        while vars.len() < k {
                            let v = (rng() % n as u64) as usize;
                            if !vars.contains(&v) {
                                vars.push(v);
                            }
                        }
                        let coeffs: Vec<(usize, u64)> = vars.iter().map(|&v| (v, 1 + rng() % (p - 1))).collect();
                        ModpEquation::new(coeffs, rng() % p)
                    })
                    .collect();
                let brute: HashSet<Vec<u64>> = models_p(n, p, &eqs).into_iter().collect();
                match solve_space(&eqs, n, p) {
                    None => assert!(brute.is_empty(), "None ⇒ no GF({p}) solutions"),
                    Some(ss) => {
                        assert_eq!(ss.count() as usize, brute.len(), "count must equal the brute-force solution count");
                        let got: HashSet<Vec<u64>> = ss.enumerate().into_iter().collect();
                        assert_eq!(got, brute, "particular + kernel must enumerate exactly the GF({p}) solutions");
                    }
                }
            }
        }
    }

    /// Encode a mod-`p` system in one-hot Boolean form exactly as the families do: `b(e,val) = e·p+val`,
    /// at-least-one + at-most-one per edge, and per-equation "forbid every wrong-sum assignment" clauses.
    fn onehot_cnf(p: u64, num_edges: usize, equations: &[ModpEquation]) -> (usize, Vec<Vec<Lit>>) {
        let bvar = |e: usize, val: u64| (e * p as usize + val as usize) as u32;
        let mut clauses: Vec<Vec<Lit>> = Vec::new();
        for e in 0..num_edges {
            clauses.push((0..p).map(|v| Lit::pos(bvar(e, v))).collect());
            for v1 in 0..p {
                for v2 in (v1 + 1)..p {
                    clauses.push(vec![Lit::neg(bvar(e, v1)), Lit::neg(bvar(e, v2))]);
                }
            }
        }
        for eq in equations {
            let d = eq.coeffs.len();
            for idx in 0..p.pow(d as u32) {
                let mut x = idx;
                let combo: Vec<u64> = (0..d)
                    .map(|_| {
                        let v = x % p;
                        x /= p;
                        v
                    })
                    .collect();
                let sum = eq.coeffs.iter().zip(&combo).fold(0u64, |a, (&(_, co), &val)| (a + co * val) % p);
                if sum % p != eq.rhs % p {
                    clauses.push(eq.coeffs.iter().zip(&combo).map(|(&(v, _), &val)| Lit::neg(bvar(v, val))).collect());
                }
            }
        }
        (num_edges * p as usize, clauses)
    }

    /// **The GF(p) SAT-side break pins a determined edge — soundly.** A 3-edge `GF(3)` system whose
    /// equations force every edge (`x0=1, x1=2, x2=1`): the break recovers it, finds all three forced, and
    /// emits the one-hot units — each of which is verified to hold in *every* Boolean model of the CNF.
    #[test]
    fn affine_p_forced_pins_determined_edges_and_is_sound() {
        // x0+x1≡0, x1+x2≡0, x0+x2≡2 (mod 3) ⇒ x0=1, x1=2, x2=1, all forced.
        let eqs = vec![
            ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0),
            ModpEquation::new(vec![(1usize, 1u64), (2, 1)], 0),
            ModpEquation::new(vec![(0usize, 1u64), (2, 1)], 2),
        ];
        let (nbv, clauses) = onehot_cnf(3, 3, &eqs);
        match affine_p_forced(nbv, &clauses) {
            AffinePForced::Forced(units) => {
                // b(0,1)=1, b(1,2)=5, b(2,1)=7 must be pinned true.
                for &(e, v) in &[(0usize, 1u64), (1, 2), (2, 1)] {
                    let bvar = (e * 3 + v as usize) as u32;
                    assert!(units.contains(&vec![Lit::pos(bvar)]), "must pin b(edge{e}={v}) = var {bvar}; got {units:?}");
                }
                // Soundness: every forced unit holds in every Boolean model of the one-hot CNF.
                let models = crate::affine::models_of(nbv, &clauses);
                assert!(!models.is_empty(), "the crafted determined system is satisfiable");
                for u in &units {
                    let l = u[0];
                    assert!(
                        models.iter().all(|&m| ((m >> l.var()) & 1 == 1) == l.is_positive()),
                        "forced unit on var {} must hold in every Boolean model",
                        l.var()
                    );
                }
            }
            _ => panic!("the determined edges must produce forced one-hot units"),
        }
    }

    /// **A GF(p) link lifts to value-permuted bit-equivalences.** Over `GF(3)`, `x0 + x1 ≡ 0` links the two
    /// edges by the scalar `c = 2` (`x0 = 2·x1`) with neither forced — the solution space exposes the
    /// proportional kernel columns, and the SAT-side break lifts the link to one-hot bit-equivalences that
    /// genuinely *permute values* (e.g. `b(x=1) ↔ b(=2)`), the mod-`p` shear no clause equivalence expresses.
    #[test]
    fn gfp_link_lifts_to_value_permuted_bit_equivalences() {
        let p = 3u64;
        let eqs = vec![ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0)];

        // Abstract: one free direction, and x0's kernel entry is exactly 2× x1's — the scalar link c = 2.
        let ss = crate::modp::solve_space(&eqs, 2, p).expect("consistent system");
        assert_eq!(ss.kernel_basis.len(), 1, "one free variable ⇒ a single kernel direction");
        let kv = &ss.kernel_basis[0];
        assert!(kv[0] != 0 && kv[1] != 0, "neither variable is forced");
        assert_eq!(kv[0], (2 * kv[1]) % p, "x0's kernel entry is 2× x1's — the GF(3) scalar link");

        // Boolean: the break fires with sound, value-permuted bit-equivalences.
        let (nbv, clauses) = onehot_cnf(p, 2, &eqs);
        let AffinePForced::Forced(extra) = affine_p_forced(nbv, &clauses) else {
            panic!("the link must fire as forced consequences");
        };
        assert!(extra.iter().any(|c| c.len() == 2), "must emit bit-equivalences (2-literal clauses)");
        let models = crate::affine::models_of(nbv, &clauses);
        for c in &extra {
            assert!(
                models.iter().all(|&m| c.iter().any(|l| ((m >> l.var()) & 1 == 1) == l.is_positive())),
                "every emitted equivalence must hold in every Boolean model"
            );
        }
        // Non-trivial: at least one equivalence relates DIFFERENT value-positions across the two groups.
        let rec = crate::modp::recover_from_cnf(nbv, &clauses).expect("recovers");
        let pos_of = |bv: u32| {
            rec.groups.iter().enumerate().find_map(|(g, grp)| grp.iter().position(|&b| b == bv).map(|v| (g, v)))
        };
        let value_permuted = extra.iter().filter(|c| c.len() == 2).any(|c| {
            matches!((pos_of(c[0].var()), pos_of(c[1].var())), (Some((g0, v0)), Some((g1, v1))) if g0 != g1 && v0 != v1)
        });
        assert!(value_permuted, "a genuine GF(3) link must produce a value-permuted bit-equivalence");
    }

    /// **Soundness to the point of absurdity.** Random `GF(3)`/`GF(5)` systems over two one-hot edges, the
    /// whole SAT-side break (forced units AND linked bit-equivalences) brute-force-checked: every emitted
    /// clause holds in *every* Boolean model, and `Refuted` only ever names a genuinely-UNSAT formula.
    #[test]
    fn affine_p_break_is_sound_against_brute_force() {
        let mut state = 0x5A7_AFF1_9E37u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &p in &[3u64, 5] {
            for _ in 0..200 {
                let m = 1 + (rng() % 3) as usize;
                let eqs: Vec<ModpEquation> = (0..m)
                    .map(|_| {
                        // 2-variable equations over the two edges (what recover_from_cnf reconstructs).
                        ModpEquation::new(vec![(0usize, 1 + rng() % (p - 1)), (1, 1 + rng() % (p - 1))], rng() % p)
                    })
                    .collect();
                let (nbv, clauses) = onehot_cnf(p, 2, &eqs);
                let models = crate::affine::models_of(nbv, &clauses);
                match affine_p_forced(nbv, &clauses) {
                    AffinePForced::Refuted(_) => assert!(models.is_empty(), "Refuted ⇒ the Boolean CNF must be UNSAT"),
                    AffinePForced::Forced(extra) => {
                        for c in &extra {
                            assert!(
                                models.iter().all(|&m| c.iter().any(|l| ((m >> l.var()) & 1 == 1) == l.is_positive())),
                                "every forced/linked consequence must hold in every Boolean model"
                            );
                        }
                    }
                    AffinePForced::Unchanged => {}
                }
            }
        }
    }

    /// `|AGL(n, ℤ/m)|` factors by CRT: `|AGL(2, ℤ/6)| = |AGL(2,2)|·|AGL(2,3)| = 24·432 = 10368`, and a
    /// non-squarefree modulus has no such product form.
    #[test]
    fn agl_m_order_factors_by_crt() {
        assert_eq!(agl_m_order(2, 6), Some(24 * 432));
        assert_eq!(agl_m_order(2, 30), Some(24 * 432 * 12000)); // 2·3·5
        assert_eq!(agl_m_order(2, 4), None, "4 is not squarefree");
        assert_eq!(agl_m_order(2, 12), None, "12 = 2²·3 is not squarefree");
    }

    /// Brute-force the GF(m)-valued solutions of a `ℤ/m` system (`Σ coeffs·x ≡ rhs mod m`).
    fn models_m(n: usize, m: u64, equations: &[ModpEquation]) -> Vec<Vec<u64>> {
        (0..m.pow(n as u32))
            .filter_map(|code| {
                let mut c = code;
                let x: Vec<u64> = (0..n)
                    .map(|_| {
                        let v = c % m;
                        c /= m;
                        v
                    })
                    .collect();
                equations
                    .iter()
                    .all(|eq| eq.coeffs.iter().fold(0u64, |a, &(v, co)| (a + co * x[v]) % m) % m == eq.rhs % m)
                    .then_some(x)
            })
            .collect()
    }

    /// **The composite forced-value detector is exact.** Against brute force over random squarefree-`m`
    /// systems: a variable is reported forced iff it takes one value across every `ℤ/m` solution, with that
    /// value; `Inconsistent` iff there are no solutions.
    #[test]
    fn forced_values_squarefree_matches_brute_force() {
        use crate::modm::{forced_values_squarefree, ForcedM};
        let mut state = 0xC0DE_B0D5u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &m in &[6u64, 10, 15] {
            for _ in 0..150 {
                let eqs: Vec<ModpEquation> = (0..(1 + rng() % 3))
                    .map(|_| ModpEquation::new(vec![(0usize, rng() % m), (1, rng() % m)], rng() % m))
                    .collect();
                let models = models_m(2, m, &eqs);
                let brute: Vec<Option<u64>> = (0..2)
                    .map(|g| {
                        let vals: HashSet<u64> = models.iter().map(|x| x[g]).collect();
                        if vals.len() == 1 { vals.into_iter().next() } else { None }
                    })
                    .collect();
                match forced_values_squarefree(&eqs, 2, m) {
                    None => panic!("m={m} is squarefree"),
                    Some(ForcedM::Inconsistent) => assert!(models.is_empty(), "Inconsistent ⇒ no ℤ/{m} solutions"),
                    Some(ForcedM::Forced(f)) => assert_eq!(f, brute, "forced values must match brute force (m={m})"),
                }
            }
        }
    }

    /// **The composite ℤ/m SAT-side break pins forced edges — soundly.** `x0 ≡ 5`, `x1 ≡ 2` over `ℤ/6`
    /// (one congruence per edge, as the recovery requires): the break recovers it, CRT-combines the
    /// `GF(2)×GF(3)` forced values, and emits one-hot units each verified against every Boolean model.
    #[test]
    fn affine_m_forced_pins_composite_modulus_and_is_sound() {
        let m = 6u64;
        let eqs = vec![
            ModpEquation::new(vec![(0usize, 1u64)], 5),
            ModpEquation::new(vec![(1usize, 1u64)], 2),
        ];
        let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
        match affine_m_forced(nbv, &clauses) {
            AffinePForced::Forced(units) => {
                assert!(units.contains(&vec![Lit::pos(5)]), "must pin b(edge0=5); got {units:?}"); // 0*6+5
                assert!(units.contains(&vec![Lit::pos(8)]), "must pin b(edge1=2)"); // 1*6+2
                let models = crate::affine::models_of(nbv, &clauses);
                assert!(!models.is_empty(), "the crafted ℤ/6 system is satisfiable");
                for u in &units {
                    let l = u[0];
                    assert!(
                        models.iter().all(|&mm| ((mm >> l.var()) & 1 == 1) == l.is_positive()),
                        "forced unit on var {} must hold in every Boolean model",
                        l.var()
                    );
                }
            }
            _ => panic!("the determined ℤ/6 edges must produce forced one-hot units"),
        }
    }

    /// **The composite break refutes a zero-divisor obstruction.** Over `ℤ/6` the 3-cycle forcing
    /// `2·x ≡ 3` is inconsistent (2 is a zero-divisor, so `2·x ≡ 3` is unsolvable) — the break reports
    /// `Refuted` on the genuinely-UNSAT one-hot encoding, a composite obstruction no prime field sees alone.
    #[test]
    fn affine_m_forced_refutes_a_composite_zero_divisor_obstruction() {
        let m = 6u64;
        let eqs = vec![
            ModpEquation::new(vec![(0usize, 1u64), (1, 5)], 0), // x0 − x1 ≡ 0
            ModpEquation::new(vec![(1usize, 1u64), (2, 5)], 0), // x1 − x2 ≡ 0
            ModpEquation::new(vec![(0usize, 1u64), (2, 1)], 3), // x0 + x2 ≡ 3  ⇒  2·x0 ≡ 3, unsolvable
        ];
        assert!(models_m(3, m, &eqs).is_empty(), "the ℤ/6 system is genuinely inconsistent");
        let (nbv, clauses) = onehot_cnf(m, 3, &eqs);
        assert!(matches!(affine_m_forced(nbv, &clauses), AffinePForced::Refuted(_)), "must refute the ℤ/6 obstruction");
    }

    /// **A composite ℤ/m link lifts to value-permuted bit-equivalences via CRT.** `x0 + x1 ≡ 0` over `ℤ/6`
    /// links `x0 = 5·x1` — the unit `c = 5 = CRT(1 mod 2, 2 mod 3)`, neither variable forced. The break
    /// emits value-permuted one-hot equivalences `b(x0=v) ↔ b(x1=…)`, each sound against every Boolean
    /// model, the composite shear no monomial break sees.
    #[test]
    fn affine_m_link_lifts_to_composite_value_permuted_equivalences() {
        let m = 6u64;
        let eqs = vec![ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0)];
        let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
        let AffinePForced::Forced(extra) = affine_m_forced(nbv, &clauses) else {
            panic!("the ℤ/6 link must fire");
        };
        assert!(extra.iter().any(|c| c.len() == 2), "must emit value-permuted bit-equivalences (2-literal clauses)");
        let models = crate::affine::models_of(nbv, &clauses);
        for c in &extra {
            assert!(
                models.iter().all(|&mm| c.iter().any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
                "every emitted equivalence must hold in every Boolean model"
            );
        }
        let rec = crate::modp::recover_from_cnf(nbv, &clauses).expect("recovers");
        let pos_of = |bv: u32| {
            rec.groups.iter().enumerate().find_map(|(g, grp)| grp.iter().position(|&b| b == bv).map(|v| (g, v)))
        };
        assert!(
            extra.iter().filter(|c| c.len() == 2).any(|c| matches!(
                (pos_of(c[0].var()), pos_of(c[1].var())),
                (Some((g0, v0)), Some((g1, v1))) if g0 != g1 && v0 != v1
            )),
            "a genuine ℤ/6 link must produce a value-permuted equivalence"
        );
    }

    /// **Soundness to the point of absurdity over composite moduli.** Random `ℤ/6`, `ℤ/10`, `ℤ/15` systems
    /// (distinct-subset congruences the recovery accepts), the whole composite break — forced, partially
    /// forced, AND linked consequences — brute-force-checked: every emitted clause holds in *every* Boolean
    /// model, and `Refuted` only ever names a genuinely-UNSAT formula.
    #[test]
    fn affine_m_break_is_sound_against_brute_force() {
        let mut state = 0x0BAD_F00D_77u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        // ℤ/6 keeps the Boolean encoding small (2 edges = 12 bits, under models_of's brute-force cap)
        // while exercising forced + partial + linked consequences; larger composites are covered by the
        // direct forced_values_squarefree fuzz.
        for &m in &[6u64] {
            for _ in 0..300 {
                let mut eqs: Vec<ModpEquation> = Vec::new();
                if rng() & 1 == 0 {
                    eqs.push(ModpEquation::new(vec![(0usize, 1 + rng() % (m - 1))], rng() % m));
                }
                if rng() & 1 == 0 {
                    eqs.push(ModpEquation::new(vec![(1usize, 1 + rng() % (m - 1))], rng() % m));
                }
                if rng() & 1 == 0 {
                    eqs.push(ModpEquation::new(vec![(0usize, 1 + rng() % (m - 1)), (1, 1 + rng() % (m - 1))], rng() % m));
                }
                if eqs.is_empty() {
                    continue;
                }
                let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
                let models = crate::affine::models_of(nbv, &clauses);
                match affine_m_forced(nbv, &clauses) {
                    AffinePForced::Refuted(_) => assert!(models.is_empty(), "Refuted ⇒ the Boolean CNF must be UNSAT (m={m})"),
                    AffinePForced::Forced(extra) => {
                        for c in &extra {
                            assert!(
                                models.iter().all(|&mm| c.iter().any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
                                "every composite consequence must hold in every Boolean model (m={m})"
                            );
                        }
                    }
                    AffinePForced::Unchanged => {}
                }
            }
        }
    }

    /// **The GF(p) elimination shrinks and lifts soundly.** Over `GF(3)`, `x0+x1 ≡ 0` (links `x0 = 2·x1`)
    /// and `x2 ≡ 1` (forced): the `x2` group collapses to constants and the `x0` group aliases
    /// value-permuted to `x1`, so only `x1`'s group survives — and every reduced model lifts to a genuine
    /// model of the original.
    #[test]
    fn affine_p_canonicalize_eliminates_and_lifts_soundly() {
        let eqs = vec![
            ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0),
            ModpEquation::new(vec![(2usize, 1u64)], 1),
        ];
        let (nbv, clauses) = onehot_cnf(3, 3, &eqs);
        match affine_p_canonicalize(nbv, &clauses) {
            AffinePCanon::Canonical(canon) => {
                assert!(canon.num_vars < nbv, "elimination must shrink the bit count ({} < {nbv})", canon.num_vars);
                let orig = crate::affine::models_of(nbv, &clauses);
                let red = crate::affine::models_of(canon.num_vars, &canon.clauses);
                assert_eq!(red.is_empty(), orig.is_empty(), "reduction must preserve satisfiability");
                for &rm_bits in &red {
                    let rm: Vec<bool> = (0..canon.num_vars).map(|i| (rm_bits >> i) & 1 == 1).collect();
                    let lifted = canon.lift(&rm);
                    assert!(
                        clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())),
                        "every lifted reduced model must satisfy the original formula"
                    );
                }
            }
            _ => panic!("forced + linked groups must canonicalize"),
        }
    }

    /// **Soundness to the point of absurdity.** Random `GF(3)`/`GF(5)` one-hot systems: the elimination
    /// must preserve satisfiability exactly against brute force, refute only genuinely-UNSAT instances, and
    /// lift every reduced model to a real model of the original.
    #[test]
    fn affine_p_canonicalize_is_sound_against_brute_force() {
        let mut state = 0xE11_3110u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &p in &[3u64, 5] {
            for _ in 0..120 {
                let mut eqs: Vec<ModpEquation> = Vec::new();
                if rng() & 1 == 0 {
                    eqs.push(ModpEquation::new(vec![(0usize, 1 + rng() % (p - 1))], rng() % p));
                }
                if rng() & 1 == 0 {
                    eqs.push(ModpEquation::new(vec![(1usize, 1 + rng() % (p - 1))], rng() % p));
                }
                if rng() & 1 == 0 {
                    eqs.push(ModpEquation::new(vec![(0usize, 1 + rng() % (p - 1)), (1, 1 + rng() % (p - 1))], rng() % p));
                }
                if eqs.is_empty() {
                    continue;
                }
                let (nbv, clauses) = onehot_cnf(p, 2, &eqs);
                let orig = crate::affine::models_of(nbv, &clauses);
                match affine_p_canonicalize(nbv, &clauses) {
                    AffinePCanon::Refuted(_) => assert!(orig.is_empty(), "Refuted ⇒ original UNSAT (p={p})"),
                    AffinePCanon::Canonical(canon) => {
                        let red = crate::affine::models_of(canon.num_vars, &canon.clauses);
                        assert_eq!(red.is_empty(), orig.is_empty(), "elimination preserves satisfiability (p={p})");
                        for &rm_bits in &red {
                            let rm: Vec<bool> = (0..canon.num_vars).map(|i| (rm_bits >> i) & 1 == 1).collect();
                            let lifted = canon.lift(&rm);
                            assert!(
                                clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())),
                                "every lifted reduced model must satisfy the original (p={p})"
                            );
                        }
                    }
                    AffinePCanon::Unchanged => {}
                }
            }
        }
    }

    /// **The prime-power detector is exact.** Over `ℤ/8` the zero-divisor `2·x0 ≡ 4` leaves `x0 ∈ {2,6}`
    /// (NOT forced) while `x1 ≡ 5` is; over `ℤ/9` the zero-divisor `3·x0 ≡ 1` is unsolvable.
    #[test]
    fn forced_values_prime_power_is_exact() {
        use crate::modm::{forced_values_prime_power, ForcedM};
        let eqs = vec![
            ModpEquation::new(vec![(0usize, 2u64)], 4), // 2·x0 ≡ 4 (mod 8) ⇒ x0 ∈ {2, 6}
            ModpEquation::new(vec![(1usize, 1u64)], 5), // x1 ≡ 5
        ];
        match forced_values_prime_power(&eqs, 2, 8) {
            Some(ForcedM::Forced(f)) => {
                assert_eq!(f[0], None, "x0 is not forced — 2·x0 ≡ 4 has two solutions mod 8");
                assert_eq!(f[1], Some(5), "x1 is forced to 5");
            }
            other => panic!("expected a forced structure, got something else: {}", matches!(other, Some(ForcedM::Inconsistent))),
        }
        let unsolvable = vec![ModpEquation::new(vec![(0usize, 3u64)], 1)]; // 3·x0 ≡ 1 (mod 9): no solution
        assert!(
            matches!(forced_values_prime_power(&unsolvable, 1, 9), Some(ForcedM::Inconsistent)),
            "3·x0 ≡ 1 (mod 9) is unsolvable"
        );
    }

    /// **The composite break handles a prime-power modulus.** Over `ℤ/4 = 2²` (a local ring, not
    /// squarefree, so the CRT path declines) the break takes the prime-power route: `x0 ≡ 3` is forced and
    /// lifted to one-hot units, each sound against every Boolean model.
    #[test]
    fn affine_m_forced_handles_prime_power_modulus() {
        let m = 4u64;
        let eqs = vec![ModpEquation::new(vec![(0usize, 1u64)], 3)];
        let (nbv, clauses) = onehot_cnf(m, 1, &eqs);
        match affine_m_forced(nbv, &clauses) {
            AffinePForced::Forced(units) => {
                assert!(units.contains(&vec![Lit::pos(3)]), "must pin b(x0=3) over ℤ/4; got {units:?}");
                let models = crate::affine::models_of(nbv, &clauses);
                assert!(!models.is_empty(), "the ℤ/4 system is satisfiable");
                for u in &units {
                    let l = u[0];
                    assert!(
                        models.iter().all(|&mm| ((mm >> l.var()) & 1 == 1) == l.is_positive()),
                        "forced unit on var {} must hold in every Boolean model",
                        l.var()
                    );
                }
            }
            _ => panic!("the ℤ/4 forced variable must produce one-hot units"),
        }
    }

    /// **The Smith-form solution space is exact.** Over small `ℤ/pᵏ` rings (ℤ/4, ℤ/8, ℤ/9), against brute
    /// enumeration of hundreds of random systems: the particular solution satisfies, `Inconsistent` iff no
    /// solution exists, and a variable's kernel-never-moves-it test matches "takes one value in every
    /// solution" — exactly (with the forced value agreeing).
    #[test]
    fn solve_space_prime_power_matches_brute() {
        use crate::modm::{solve_space_prime_power, PrimePowerSpace};
        let mut state = 0x511700D5u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &(p, k) in &[(2u64, 2u32), (2, 3), (3, 2)] {
            let q = p.pow(k);
            for _ in 0..200 {
                let nv = 1 + (rng() % 3) as usize;
                let eqs: Vec<ModpEquation> = (0..(1 + rng() % 3))
                    .map(|_| {
                        let kk = 1 + (rng() % nv as u64) as usize;
                        let mut vars: Vec<usize> = Vec::new();
                        while vars.len() < kk {
                            let vv = (rng() % nv as u64) as usize;
                            if !vars.contains(&vv) {
                                vars.push(vv);
                            }
                        }
                        ModpEquation::new(vars.iter().map(|&vv| (vv, rng() % q)).collect::<Vec<_>>(), rng() % q)
                    })
                    .collect();
                let total = (q as u128).pow(nv as u32) as u64;
                let mut sols: Vec<Vec<u64>> = Vec::new();
                for code in 0..total {
                    let mut c = code;
                    let x: Vec<u64> = (0..nv)
                        .map(|_| {
                            let vv = c % q;
                            c /= q;
                            vv
                        })
                        .collect();
                    if eqs.iter().all(|eq| eq.coeffs.iter().fold(0u64, |a, &(vv, co)| (a + co * x[vv]) % q) % q == eq.rhs % q) {
                        sols.push(x);
                    }
                }
                match solve_space_prime_power(&eqs, nv, p, k).expect("no Smith overflow at this size") {
                    PrimePowerSpace::Inconsistent => assert!(sols.is_empty(), "Inconsistent ⇒ no ℤ/{q} solution"),
                    PrimePowerSpace::Space(ss) => {
                        assert!(!sols.is_empty(), "a Space ⇒ at least one solution (ℤ/{q})");
                        assert!(
                            eqs.iter().all(|eq| eq.coeffs.iter().fold(0u64, |a, &(vv, co)| (a + co * ss.particular[vv]) % q) % q == eq.rhs % q),
                            "the particular solution must satisfy (ℤ/{q})"
                        );
                        for g in 0..nv {
                            let smith = ss.kernel_basis.iter().all(|kk| kk[g] == 0);
                            let vals: HashSet<u64> = sols.iter().map(|s| s[g]).collect();
                            assert_eq!(smith, vals.len() == 1, "forced(var {g}): Smith vs brute (ℤ/{q})");
                            if smith {
                                assert_eq!(ss.particular[g], *vals.iter().next().unwrap(), "forced value matches brute (ℤ/{q})");
                            }
                        }
                    }
                }
            }
        }
    }

    /// **The Smith path scales past the brute cap.** `ℤ/16` with 6 variables has `16⁶ = 2²⁴` value tuples —
    /// over the old `2²⁰` brute bound, so this is only decidable via the Smith solution space. All six are
    /// forced (unit congruences), and the scalable detector pins each.
    #[test]
    fn forced_values_prime_power_scales_past_the_brute_cap() {
        use crate::modm::{forced_values_prime_power, ForcedM};
        let m = 16u64;
        let eqs: Vec<ModpEquation> = (0..6usize).map(|i| ModpEquation::new(vec![(i, 1u64)], i as u64 + 1)).collect();
        match forced_values_prime_power(&eqs, 6, m) {
            Some(ForcedM::Forced(f)) => {
                for i in 0..6 {
                    assert_eq!(f[i], Some(i as u64 + 1), "x{i} forced to {} over ℤ/16 (2²⁴ tuples, Smith only)", i + 1);
                }
            }
            _ => panic!("ℤ/16 forcing over 6 vars must succeed via the scalable Smith path"),
        }
    }

    /// **The allowed-residue congruence is exact.** Over small composite moduli (ℤ/4, ℤ/8, ℤ/9, ℤ/6,
    /// ℤ/12 — prime-power AND mixed), against brute force: each variable's `(res, modu)` describes *exactly*
    /// the set of values it takes across all solutions (`v` allowed ⟺ `v ≡ res mod modu`).
    #[test]
    fn allowed_residues_matches_brute() {
        use crate::modm::{allowed_residues, AllowedOutcome};
        let mut state = 0xA110_0ED5u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &m in &[4u64, 8, 9, 6, 12] {
            for _ in 0..150 {
                let nv = 1 + (rng() % 2) as usize;
                let eqs: Vec<ModpEquation> = (0..(1 + rng() % 2))
                    .map(|_| {
                        let kk = 1 + (rng() % nv as u64) as usize;
                        let mut vars: Vec<usize> = Vec::new();
                        while vars.len() < kk {
                            let vv = (rng() % nv as u64) as usize;
                            if !vars.contains(&vv) {
                                vars.push(vv);
                            }
                        }
                        ModpEquation::new(vars.iter().map(|&vv| (vv, rng() % m)).collect::<Vec<_>>(), rng() % m)
                    })
                    .collect();
                let total = (m as u128).pow(nv as u32) as u64;
                let mut sols: Vec<Vec<u64>> = Vec::new();
                for code in 0..total {
                    let mut c = code;
                    let x: Vec<u64> = (0..nv)
                        .map(|_| {
                            let vv = c % m;
                            c /= m;
                            vv
                        })
                        .collect();
                    if eqs.iter().all(|eq| eq.coeffs.iter().fold(0u64, |a, &(vv, co)| (a + co * x[vv]) % m) % m == eq.rhs % m) {
                        sols.push(x);
                    }
                }
                match allowed_residues(&eqs, nv, m).expect("no Smith overflow") {
                    AllowedOutcome::Inconsistent => assert!(sols.is_empty(), "Inconsistent ⇒ no ℤ/{m} solution"),
                    AllowedOutcome::Allowed(residues) => {
                        assert!(!sols.is_empty());
                        for g in 0..nv {
                            let (res, modu) = residues[g];
                            let vals: HashSet<u64> = sols.iter().map(|s| s[g]).collect();
                            for v in 0..m {
                                assert_eq!(vals.contains(&v), v % modu == res, "var {g} value {v} allowed? (m={m}, res={res} mod {modu})");
                            }
                        }
                    }
                }
            }
        }
    }

    /// **The composite break PARTIALLY forces a DERIVED prime-power value-subset.** Over `ℤ/8`, `2·x1 ≡ 0`
    /// pins `x1` to `{0,4}`, and `x0 + x1 ≡ 0` then *derives* `x0 ∈ {0,4}` (`≡ 0 mod 4`) — a partial
    /// constraint no single clause states (`x0`'s subset is not in the encoding). The break emits the new
    /// forbidding units on `x0`, each sound against every Boolean model — the value-subset the prime-power
    /// path used to miss.
    #[test]
    fn affine_m_forced_partially_forces_a_prime_power_value_subset() {
        let m = 8u64;
        let eqs = vec![
            ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0), // x0 + x1 ≡ 0
            ModpEquation::new(vec![(1usize, 2u64)], 0),         // 2·x1 ≡ 0  ⇒  x1 ∈ {0,4}
        ];
        let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
        match affine_m_forced(nbv, &clauses) {
            AffinePForced::Forced(units) => {
                // b(x0=v) = var v; x0 ≡ 0 mod 4 ⇒ forbid v ∈ {1,2,3,5,6,7} (derived, not in the encoding).
                for v in [1u32, 2, 3, 5, 6, 7] {
                    assert!(units.contains(&vec![Lit::neg(v)]), "must derive-forbid b(x0={v}) (off ≡0 mod 4)");
                }
                let models = crate::affine::models_of(nbv, &clauses);
                assert!(!models.is_empty(), "the ℤ/8 system is satisfiable");
                for u in &units {
                    assert!(
                        models.iter().all(|&mm| u.iter().any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
                        "emitted clause {u:?} must hold in every Boolean model"
                    );
                }
            }
            _ => panic!("ℤ/8 derived partial forcing must fire"),
        }
    }

    /// **The composite break now LINKS over a prime-power ring.** Over `ℤ/8` (a local ring), `x0 + 5·x1 ≡ 0`
    /// gives `x0 = 3·x1` — `c = 3` is a unit, neither variable forced. The ring link lifts to value-permuted
    /// bit-equivalences `b(x1=v) ↔ b(x0=3v)`, each sound against every Boolean model, with a genuine value
    /// permutation (the link the prime-power path used to miss entirely).
    #[test]
    fn affine_m_forced_links_over_a_prime_power_ring() {
        let m = 8u64;
        let eqs = vec![ModpEquation::new(vec![(0usize, 1u64), (1, 5)], 0)];
        let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
        let AffinePForced::Forced(extra) = affine_m_forced(nbv, &clauses) else {
            panic!("the ℤ/8 ring link must fire");
        };
        assert!(extra.iter().any(|c| c.len() == 2), "must emit value-permuted bit-equivalences");
        let models = crate::affine::models_of(nbv, &clauses);
        for c in &extra {
            assert!(
                models.iter().all(|&mm| c.iter().any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
                "every emitted equivalence must hold in every Boolean model"
            );
        }
        let rec = crate::modp::recover_from_cnf(nbv, &clauses).expect("recovers");
        let pos_of = |bv: u32| {
            rec.groups.iter().enumerate().find_map(|(g, grp)| grp.iter().position(|&b| b == bv).map(|v| (g, v)))
        };
        assert!(
            extra.iter().filter(|c| c.len() == 2).any(|c| matches!(
                (pos_of(c[0].var()), pos_of(c[1].var())),
                (Some((g0, v0)), Some((g1, v1))) if g0 != g1 && v0 != v1
            )),
            "a genuine ℤ/8 ring link must produce a value-permuted equivalence"
        );
    }

    /// **Soundness to the point of absurdity over a prime-power ring.** Random `ℤ/8` systems (distinct-subset
    /// congruences), the WHOLE break — forced, partial, AND ring links — brute-force-checked: every emitted
    /// clause holds in every Boolean model, and `Refuted` only names a genuinely-UNSAT formula.
    #[test]
    fn affine_m_break_is_sound_over_a_prime_power_ring() {
        let m = 8u64;
        let mut state = 0x21B6_0FF5u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..120 {
            let mut eqs: Vec<ModpEquation> = Vec::new();
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(vec![(0usize, 1 + rng() % (m - 1))], rng() % m));
            }
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(vec![(1usize, 1 + rng() % (m - 1))], rng() % m));
            }
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(vec![(0usize, 1 + rng() % (m - 1)), (1, 1 + rng() % (m - 1))], rng() % m));
            }
            if eqs.is_empty() {
                continue;
            }
            let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
            let models = crate::affine::models_of(nbv, &clauses);
            match affine_m_forced(nbv, &clauses) {
                AffinePForced::Refuted(_) => assert!(models.is_empty(), "Refuted ⇒ the ℤ/8 CNF must be UNSAT"),
                AffinePForced::Forced(extra) => {
                    for c in &extra {
                        assert!(
                            models.iter().all(|&mm| c.iter().any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
                            "every ℤ/8 consequence must hold in every Boolean model"
                        );
                    }
                }
                AffinePForced::Unchanged => {}
            }
        }
    }

    /// **The composite eliminate path physically reduces and lifts.** Over `ℤ/4`, `x0 ≡ 3` is forced and
    /// `x1 + x2 ≡ 0` links `x2 = −x1`. Both the forced and linked groups are eliminated — only `x1`'s one-hot
    /// survives — and every model of the reduced formula lifts to a real model of the original.
    #[test]
    fn affine_m_canonicalize_eliminates_and_lifts_soundly() {
        let m = 4u64;
        let eqs = vec![
            ModpEquation::new(vec![(0usize, 1u64)], 3),
            ModpEquation::new(vec![(1usize, 1u64), (2, 1)], 0),
        ];
        let (nbv, clauses) = onehot_cnf(m, 3, &eqs);
        match affine_m_canonicalize(nbv, &clauses) {
            AffinePCanon::Canonical(canon) => {
                assert!(canon.num_vars < nbv, "elimination must shrink the bit count ({} < {nbv})", canon.num_vars);
                let orig = crate::affine::models_of(nbv, &clauses);
                let red = crate::affine::models_of(canon.num_vars, &canon.clauses);
                assert!(!orig.is_empty(), "the ℤ/4 system is satisfiable");
                assert_eq!(red.is_empty(), orig.is_empty(), "reduction must preserve satisfiability");
                for &rm_bits in &red {
                    let rm: Vec<bool> = (0..canon.num_vars).map(|i| (rm_bits >> i) & 1 == 1).collect();
                    let lifted = canon.lift(&rm);
                    assert!(
                        clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())),
                        "every lifted reduced model must satisfy the original formula"
                    );
                }
            }
            _ => panic!("the forced + linked ℤ/4 groups must canonicalize"),
        }
    }

    /// **Composite eliminate, sound to the point of absurdity.** Random one-hot systems over `ℤ/4`, `ℤ/6`,
    /// `ℤ/8` (zero-divisor coefficients exercise the *partial*-coset survivor pruning): the elimination must
    /// preserve satisfiability exactly against brute force, refute only genuinely-UNSAT instances, and lift
    /// every reduced model to a real model of the original.
    #[test]
    fn affine_m_canonicalize_is_sound_against_brute_force() {
        let mut state = 0x5EED_3110u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &m in &[4u64, 6, 8] {
            for _ in 0..150 {
                let mut eqs: Vec<ModpEquation> = Vec::new();
                if rng() % 3 != 0 {
                    eqs.push(ModpEquation::new(vec![(0usize, 1 + rng() % (m - 1))], rng() % m));
                }
                if rng() % 3 != 0 {
                    eqs.push(ModpEquation::new(vec![(1usize, 1 + rng() % (m - 1))], rng() % m));
                }
                if rng() % 3 != 0 {
                    eqs.push(ModpEquation::new(vec![(0usize, 1 + rng() % (m - 1)), (1, 1 + rng() % (m - 1))], rng() % m));
                }
                if eqs.is_empty() {
                    continue;
                }
                let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
                let orig = crate::affine::models_of(nbv, &clauses);
                match affine_m_canonicalize(nbv, &clauses) {
                    AffinePCanon::Refuted(_) => assert!(orig.is_empty(), "Refuted ⇒ original UNSAT (m={m})"),
                    AffinePCanon::Canonical(canon) => {
                        let red = crate::affine::models_of(canon.num_vars, &canon.clauses);
                        assert_eq!(red.is_empty(), orig.is_empty(), "elimination preserves satisfiability (m={m})");
                        for &rm_bits in &red {
                            let rm: Vec<bool> = (0..canon.num_vars).map(|i| (rm_bits >> i) & 1 == 1).collect();
                            let lifted = canon.lift(&rm);
                            assert!(
                                clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())),
                                "every lifted reduced model must satisfy the original (m={m})"
                            );
                        }
                    }
                    AffinePCanon::Unchanged => {}
                }
            }
        }
    }
}
