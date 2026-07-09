//! Linear algebra over `GF(p)` — the mod-`p` generalization of the GF(2) parity cut ([`crate::xorsat`]).
//!
//! A system of congruences `Σ aᵢ·xᵢ ≡ c (mod p)` is decided in **polynomial time** by Gaussian
//! elimination with modular inverses, and it is **certified**: an inconsistent system yields a
//! re-checkable linear-dependency refutation — a combination of the original equations whose left side
//! cancels to `0` while the right side is some nonzero residue, i.e. `0 ≡ r ≢ 0 (mod p)`. A consistent
//! system yields a satisfying assignment over `GF(p)`.
//!
//! This matters because the parity cut only speaks GF(2). The mod-`p` *counting principles* (`Count_p`:
//! "partition a set whose size is not a multiple of `p` into `p`-blocks") are resolution-hard, and a
//! polynomial-calculus proof over the *wrong* characteristic cannot refute them either — but Gaussian
//! elimination over the *right* `GF(p)` decides them instantly. A genuinely new invariant, the parity
//! crush carried to every prime.

/// A congruence `Σ (a·x) ≡ rhs (mod p)`. Coefficients and `rhs` are reduced mod `p`. `p` must be prime
/// (so every nonzero element is invertible, via Fermat).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModpEquation {
    pub coeffs: Vec<(usize, u64)>,
    pub rhs: u64,
}

impl ModpEquation {
    pub fn new(coeffs: impl Into<Vec<(usize, u64)>>, rhs: u64) -> Self {
        ModpEquation { coeffs: coeffs.into(), rhs }
    }
}

/// The outcome of solving a mod-`p` linear system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModpOutcome {
    /// Satisfiable, with an assignment over `0..p` for each of `0..num_vars` (re-checkable via
    /// [`satisfies`]).
    Sat(Vec<u64>),
    /// Unsatisfiable, witnessed by a combination `Σ (multiplier · equationᵢ)` whose left side cancels
    /// while the right side is nonzero — re-checkable via [`is_refutation`].
    Unsat(Vec<(usize, u64)>),
}

#[inline]
fn add(a: u64, b: u64, p: u64) -> u64 {
    (a + b) % p
}
#[inline]
fn sub(a: u64, b: u64, p: u64) -> u64 {
    (a + p - b % p) % p
}
#[inline]
fn mul(a: u64, b: u64, p: u64) -> u64 {
    (a % p) * (b % p) % p
}
#[inline]
fn powm(mut a: u64, mut e: u64, p: u64) -> u64 {
    let mut r = 1u64 % p;
    a %= p;
    while e > 0 {
        if e & 1 == 1 {
            r = mul(r, a, p);
        }
        a = mul(a, a, p);
        e >>= 1;
    }
    r
}
/// Modular inverse of a nonzero `a` over a prime field (Fermat: `a^{p-2}`).
#[inline]
fn inv(a: u64, p: u64) -> u64 {
    powm(a, p - 2, p)
}

/// Decide a mod-`p` linear system by Gaussian elimination over `GF(p)`. Returns a satisfying
/// assignment, or an inconsistency-witnessing combination of the original equations. `p` must be prime.
pub fn solve(equations: &[ModpEquation], num_vars: usize, p: u64) -> ModpOutcome {
    let m = equations.len();
    // Each row carries its variable coefficients, its rhs, and its provenance (how it is built from the
    // original equations — a coefficient per original equation). All arithmetic is mod p.
    let mut coeff: Vec<Vec<u64>> = Vec::with_capacity(m);
    let mut rhs: Vec<u64> = Vec::with_capacity(m);
    let mut prov: Vec<Vec<u64>> = Vec::with_capacity(m);
    for (i, eq) in equations.iter().enumerate() {
        let mut c = vec![0u64; num_vars];
        for &(v, a) in &eq.coeffs {
            if v < num_vars {
                c[v] = add(c[v], a, p);
            }
        }
        coeff.push(c);
        rhs.push(eq.rhs % p);
        let mut pr = vec![0u64; m];
        pr[i] = 1 % p;
        prov.push(pr);
    }

    let mut pivot_col_of_row: Vec<usize> = Vec::new();
    let mut row = 0usize;
    for col in 0..num_vars {
        let Some(sel) = (row..coeff.len()).find(|&r| coeff[r][col] != 0) else {
            continue;
        };
        coeff.swap(row, sel);
        rhs.swap(row, sel);
        prov.swap(row, sel);
        // Normalize the pivot row so its pivot coefficient is 1.
        let factor = inv(coeff[row][col], p);
        for v in 0..num_vars {
            coeff[row][v] = mul(coeff[row][v], factor, p);
        }
        rhs[row] = mul(rhs[row], factor, p);
        for k in 0..m {
            prov[row][k] = mul(prov[row][k], factor, p);
        }
        // Eliminate this column from every other row.
        for r in 0..coeff.len() {
            if r != row && coeff[r][col] != 0 {
                let f = coeff[r][col];
                for v in 0..num_vars {
                    coeff[r][v] = sub(coeff[r][v], mul(f, coeff[row][v], p), p);
                }
                rhs[r] = sub(rhs[r], mul(f, rhs[row], p), p);
                for k in 0..m {
                    prov[r][k] = sub(prov[r][k], mul(f, prov[row][k], p), p);
                }
            }
        }
        pivot_col_of_row.push(col);
        row += 1;
        if row == coeff.len() {
            break;
        }
    }

    // An all-zero row with a nonzero rhs is `0 ≡ nonzero` — a refutation; its provenance is the combo.
    for r in 0..coeff.len() {
        if coeff[r].iter().all(|&x| x == 0) && rhs[r] != 0 {
            let combo: Vec<(usize, u64)> =
                prov[r].iter().enumerate().filter(|&(_, &m)| m != 0).map(|(i, &m)| (i, m)).collect();
            return ModpOutcome::Unsat(combo);
        }
    }

    // Consistent: free variables take 0, each pivot variable takes its (reduced) rhs.
    let mut assignment = vec![0u64; num_vars];
    for (r, &col) in pivot_col_of_row.iter().enumerate() {
        assignment[col] = rhs[r];
    }
    ModpOutcome::Sat(assignment)
}

/// The **complete solution space** of a `GF(p)` linear system `A x = b`, in symmetry-broken form: one
/// particular solution `x₀` plus a basis of the kernel (null space). Every solution is `x₀ +` a `GF(p)`
/// combination of the kernel basis, so all `p^{n−rank}` solutions are generated from this compressed
/// witness. The kernel is the **translation symmetry** of the solution coset — the `GF(p)` analogue of
/// [`crate::gf2::SolutionSpace`], and the substrate of the affine SAT-side break: a variable the kernel
/// never moves is forced to a single value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolutionSpaceP {
    pub num_vars: usize,
    pub p: u64,
    pub particular: Vec<u64>,
    pub kernel_basis: Vec<Vec<u64>>,
}

impl SolutionSpaceP {
    /// The number of solutions: `p^{dim kernel}`.
    pub fn count(&self) -> u128 {
        (self.p as u128).pow(self.kernel_basis.len() as u32)
    }

    /// Generate **every** solution: `x₀` plus each `GF(p)` combination of the kernel basis.
    pub fn enumerate(&self) -> Vec<Vec<u64>> {
        let k = self.kernel_basis.len();
        let total = (self.p as u128).pow(k as u32);
        (0..total as u64)
            .map(|mut code| {
                let mut x = self.particular.clone();
                for b in 0..k {
                    let coef = code % self.p;
                    code /= self.p;
                    if coef != 0 {
                        for v in 0..self.num_vars {
                            x[v] = add(x[v], mul(coef, self.kernel_basis[b][v], self.p), self.p);
                        }
                    }
                }
                x
            })
            .collect()
    }
}

/// Solve a `GF(p)` system for its **entire** solution space via Gaussian elimination to reduced row
/// echelon form, returning the symmetry-broken [`SolutionSpaceP`] (particular solution + kernel basis),
/// or `None` iff the system is inconsistent. Generalizes [`solve`], which returns just one witness, to the
/// full coset — the `GF(p)` analogue of [`crate::gf2::solve_gf2`]. `p` must be prime.
pub fn solve_space(equations: &[ModpEquation], num_vars: usize, p: u64) -> Option<SolutionSpaceP> {
    let mut coeff: Vec<Vec<u64>> = Vec::with_capacity(equations.len());
    let mut rhs: Vec<u64> = Vec::with_capacity(equations.len());
    for eq in equations {
        let mut c = vec![0u64; num_vars];
        for &(v, a) in &eq.coeffs {
            if v < num_vars {
                c[v] = add(c[v], a, p);
            }
        }
        coeff.push(c);
        rhs.push(eq.rhs % p);
    }

    let mut pivot_col_of_row: Vec<usize> = Vec::new();
    let mut row = 0usize;
    for col in 0..num_vars {
        let Some(sel) = (row..coeff.len()).find(|&r| coeff[r][col] != 0) else {
            continue;
        };
        coeff.swap(row, sel);
        rhs.swap(row, sel);
        let factor = inv(coeff[row][col], p);
        for v in 0..num_vars {
            coeff[row][v] = mul(coeff[row][v], factor, p);
        }
        rhs[row] = mul(rhs[row], factor, p);
        // Full reduction: clear this pivot column from every other row.
        for r in 0..coeff.len() {
            if r != row && coeff[r][col] != 0 {
                let f = coeff[r][col];
                for v in 0..num_vars {
                    coeff[r][v] = sub(coeff[r][v], mul(f, coeff[row][v], p), p);
                }
                rhs[r] = sub(rhs[r], mul(f, rhs[row], p), p);
            }
        }
        pivot_col_of_row.push(col);
        row += 1;
    }

    // Inconsistent: a fully-reduced row with no coefficients but a nonzero right-hand side (0 = c ≠ 0).
    for r in 0..coeff.len() {
        if coeff[r].iter().all(|&x| x == 0) && rhs[r] != 0 {
            return None;
        }
    }

    let mut is_pivot = vec![false; num_vars];
    for &c in &pivot_col_of_row {
        is_pivot[c] = true;
    }
    // Particular: free variables 0, each pivot variable = its row's right-hand side.
    let mut particular = vec![0u64; num_vars];
    for (r, &pc) in pivot_col_of_row.iter().enumerate() {
        particular[pc] = rhs[r];
    }
    // Kernel: one vector per free column f — set x_f = 1, each pivot var = −(its row's f-coefficient).
    let mut kernel_basis = Vec::new();
    for f in 0..num_vars {
        if is_pivot[f] {
            continue;
        }
        let mut kv = vec![0u64; num_vars];
        kv[f] = 1;
        for (r, &pc) in pivot_col_of_row.iter().enumerate() {
            kv[pc] = sub(0, coeff[r][f], p);
        }
        kernel_basis.push(kv);
    }
    Some(SolutionSpaceP { num_vars, p, particular, kernel_basis })
}

/// The canonical scalable mod-`p` obstruction: a cycle of differences `xᵢ − x_{i+1} ≡ 1 (mod p)` around
/// an `n`-cycle. Summing all `n` equations telescopes the left side to `0` and the right to `n`, so the
/// system is inconsistent **exactly when `n` is not a multiple of `p`** — the mod-`p` counting fact. For
/// even `n` with `p > 2 ∤ n` it is satisfiable over `GF(2)` yet refuted over `GF(p)`: a family the parity
/// cut cannot see. (`x − y` is written `x + (p−1)y`.)
pub fn cycle_system(n: usize, p: u64) -> Vec<ModpEquation> {
    (0..n)
        .map(|i| ModpEquation::new(vec![(i, 1), ((i + 1) % n, p - 1)], 1))
        .collect()
}

/// Re-check a satisfying assignment: every congruence holds mod `p`.
pub fn satisfies(equations: &[ModpEquation], assignment: &[u64], p: u64) -> bool {
    equations.iter().all(|eq| {
        let lhs = eq
            .coeffs
            .iter()
            .fold(0u64, |acc, &(v, a)| add(acc, mul(a, *assignment.get(v).unwrap_or(&0), p), p));
        lhs == eq.rhs % p
    })
}

/// Re-check a refutation: the chosen combination of equations has every variable coefficient `≡ 0` and
/// a nonzero right-hand side mod `p` — a solver-free certificate of inconsistency.
pub fn is_refutation(
    equations: &[ModpEquation],
    num_vars: usize,
    p: u64,
    combo: &[(usize, u64)],
) -> bool {
    if combo.is_empty() {
        return false;
    }
    let mut lhs = vec![0u64; num_vars];
    let mut rhs = 0u64;
    for &(idx, mult) in combo {
        let Some(eq) = equations.get(idx) else {
            return false;
        };
        for &(v, a) in &eq.coeffs {
            if v < num_vars {
                lhs[v] = add(lhs[v], mul(mult, a, p), p);
            }
        }
        rhs = add(rhs, mul(mult, eq.rhs, p), p);
    }
    lhs.iter().all(|&x| x == 0) && rhs != 0
}

/// A mod-`m` linear system recovered from an opaque Boolean CNF: the one-hot groups (each a `ℤ/m`
/// variable, with the boolean var ids listed in value order) plus the congruences fitted to the
/// forbidden-combination clauses. The recovered system is **equisatisfiable** to the source CNF, so the
/// modular verdict carries back — over the prime field [`solve`] when `modulus` is prime, over the
/// composite ring [`crate::modm::solve`] otherwise. See [`recover_from_cnf`].
#[derive(Clone, Debug)]
pub struct ModpRecovery {
    pub modulus: u64,
    /// One `ℤ/modulus` variable per one-hot group.
    pub num_vars: usize,
    pub equations: Vec<ModpEquation>,
    /// `groups[g][val]` = the boolean variable that means "variable `g` takes value `val`".
    pub groups: Vec<Vec<u32>>,
}

pub fn is_prime(p: u64) -> bool {
    if p < 2 {
        return false;
    }
    let mut d = 2u64;
    while d * d <= p {
        if p % d == 0 {
            return false;
        }
        d += 1;
    }
    true
}

/// A basis of the null space `{ a : (row · a) ≡ 0 for every row }` over `GF(p)`, by reduced row echelon
/// form: each non-pivot (free) column yields one basis vector. Used to fit a congruence's coefficient
/// vector as the (unique up to scalar) normal of the hyperplane spanned by the allowed value tuples.
fn nullspace(rows: &[Vec<u64>], k: usize, p: u64) -> Vec<Vec<u64>> {
    let mut m: Vec<Vec<u64>> = rows.iter().map(|r| r.iter().map(|&x| x % p).collect()).collect();
    let mut where_pivot: Vec<isize> = vec![-1; k];
    let mut row = 0usize;
    for col in 0..k {
        let Some(sel) = (row..m.len()).find(|&r| m[r][col] != 0) else {
            continue;
        };
        m.swap(row, sel);
        let finv = inv(m[row][col], p);
        for c in 0..k {
            m[row][c] = mul(m[row][c], finv, p);
        }
        for r in 0..m.len() {
            if r != row && m[r][col] != 0 {
                let f = m[r][col];
                for c in 0..k {
                    m[r][c] = sub(m[r][c], mul(f, m[row][c], p), p);
                }
            }
        }
        where_pivot[col] = row as isize;
        row += 1;
        if row == m.len() {
            break;
        }
    }
    let mut basis = Vec::new();
    for free in 0..k {
        if where_pivot[free] != -1 {
            continue;
        }
        let mut v = vec![0u64; k];
        v[free] = 1;
        for (col, &pr) in where_pivot.iter().enumerate() {
            if pr != -1 {
                v[col] = sub(0, m[pr as usize][free], p);
            }
        }
        basis.push(v);
    }
    basis
}

/// Fit the unique linear congruence `Σ aᵢ·tᵢ ≡ c (mod m)` whose solution set is EXACTLY `allowed` (the
/// complement of the forbidden tuples). Over a **prime field** this is the hyperplane normal — the
/// `allowed` set must be `m^{k-1}` points and `a` is the (up-to-scalar unique) null vector of their
/// differences. Over a **composite ring** there is no field inverse, so `a` is found by a bounded search
/// over coefficient vectors (`c` is forced once `a` and a base point are fixed). Either way the returned
/// `(a, c)` is re-verified to reproduce the split exactly, so soundness never depends on which branch
/// fired. Returns `None` when no single congruence reproduces the split (then the caller declines).
fn fit_congruence(
    k: usize,
    allowed: &[Vec<u64>],
    forbidden: &[Vec<u64>],
    m: u64,
) -> Option<(Vec<u64>, u64)> {
    let t0 = allowed.first()?;
    let mm = m as u128;
    let eval = |a: &[u64], t: &[u64]| -> u64 {
        (0..k).fold(0u128, |acc, i| (acc + a[i] as u128 * t[i] as u128) % mm) as u64
    };
    let candidate: Option<Vec<u64>> = if is_prime(m) {
        // Prime field: a single congruence has exactly m^{k-1} solutions, and its normal is the unique
        // null direction of the allowed-tuple differences.
        if (allowed.len() as u128) != mm.pow((k - 1) as u32) {
            return None;
        }
        let diffs: Vec<Vec<u64>> =
            allowed.iter().skip(1).map(|t| (0..k).map(|i| sub(t[i], t0[i], m)).collect()).collect();
        let basis = nullspace(&diffs, k, m);
        (basis.len() == 1 && basis[0].iter().any(|&x| x != 0)).then(|| basis[0].clone())
    } else {
        // Composite ring: bounded brute search over the coefficient vectors.
        let total = mm.checked_pow(k as u32)?;
        if total.checked_mul(total)? > (1u128 << 24) {
            return None; // refuse an oversized ring fit; let CDCL have it
        }
        let mut found = None;
        for code in 0..total {
            let mut a = vec![0u64; k];
            let mut x = code;
            for slot in a.iter_mut() {
                *slot = (x % mm) as u64;
                x /= mm;
            }
            if a.iter().all(|&v| v == 0) {
                continue;
            }
            let c = eval(&a, t0);
            if allowed.iter().all(|t| eval(&a, t) == c) && forbidden.iter().all(|f| eval(&a, f) != c) {
                found = Some(a);
                break;
            }
        }
        found
    };
    let a = candidate?;
    let c = eval(&a, t0);
    if allowed.iter().any(|t| eval(&a, t) != c) || forbidden.iter().any(|f| eval(&a, f) == c) {
        return None;
    }
    Some((a, c))
}

/// **Lift an opaque Boolean CNF onto `ℤ/m`.** Recognize the canonical one-hot encoding of a mod-`m`
/// linear system — each variable a group of `m` bits with an at-least-one clause and the full pairwise
/// at-most-one, plus all-negative "forbidden combination" clauses pinning the congruences — and recover
/// the system over `ℤ/m`. The modulus `m` is the group size; it may be prime (a field) or composite
/// (a ring), and [`fit_congruence`] handles both. Returns `None` (declining, never guessing) unless
/// every clause fits the pattern and every forbidden set is **exactly** the complement of a single
/// congruence: then the recovered system is equisatisfiable to the CNF, so the modular solver's verdict
/// transfers (UNSAT certificate carries; a SAT model is re-checked against the clauses by the caller).
/// The `m = 2` case is the parity cut; this is that cut generalized to every modulus — the obstruction
/// GF(2) is blind to, decided in polynomial time where resolution (CDCL, Z3, Kissat) needs `2^Ω(n)`.
pub fn recover_from_cnf(num_bool_vars: usize, clauses: &[Vec<crate::cdcl::Lit>]) -> Option<ModpRecovery> {
    use std::collections::{BTreeMap, HashMap, HashSet};
    if clauses.is_empty() {
        return None;
    }

    // Pass 1 — discover one-hot groups (all-positive clauses of size ≥ 2) and the at-most-one pairs.
    let mut group_candidates: Vec<Vec<u32>> = Vec::new();
    let mut neg_pairs: HashSet<(u32, u32)> = HashSet::new();
    let mut appears: HashSet<u32> = HashSet::new();
    for c in clauses {
        for l in c {
            appears.insert(l.var());
        }
        if c.len() >= 2 && c.iter().all(|l| l.is_positive()) {
            let mut g: Vec<u32> = c.iter().map(|l| l.var()).collect();
            g.sort_unstable();
            g.dedup();
            if g.len() != c.len() {
                return None;
            }
            group_candidates.push(g);
        } else if c.len() == 2 && c.iter().all(|l| !l.is_positive()) {
            let (a, b) = (c[0].var(), c[1].var());
            neg_pairs.insert((a.min(b), a.max(b)));
        }
    }
    if group_candidates.is_empty() {
        return None;
    }
    let m = group_candidates[0].len() as u64;
    if m < 2 {
        return None;
    }

    // Validate groups: uniform size, disjoint, full pairwise at-most-one present.
    let mut var_to_group: HashMap<u32, usize> = HashMap::new();
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for g in &group_candidates {
        if g.len() as u64 != m {
            return None;
        }
        for i in 0..g.len() {
            for j in (i + 1)..g.len() {
                if !neg_pairs.contains(&(g[i], g[j])) {
                    return None;
                }
            }
        }
        let gid = groups.len();
        for &v in g {
            if var_to_group.insert(v, gid).is_some() {
                return None; // a variable in two groups: not a clean one-hot partition
            }
        }
        groups.push(g.clone());
    }
    // Every variable that appears must belong to a group, or the encoding is not pure one-hot.
    if appears.iter().any(|v| !var_to_group.contains_key(v)) {
        return None;
    }
    let _ = num_bool_vars;
    let pos_in_group = |v: u32, gid: usize| groups[gid].iter().position(|&x| x == v).unwrap() as u64;

    // Pass 2 — classify every clause; collect forbidden tuples per scope (a sorted set of groups).
    let mut scopes: BTreeMap<Vec<usize>, Vec<Vec<u64>>> = BTreeMap::new();
    for c in clauses {
        if c.len() >= 2 && c.iter().all(|l| l.is_positive()) {
            continue; // at-least-one of a group: one-hot structure
        }
        if c.len() == 2 && c.iter().all(|l| !l.is_positive()) {
            let g0 = *var_to_group.get(&c[0].var())?;
            let g1 = *var_to_group.get(&c[1].var())?;
            if g0 == g1 {
                continue; // at-most-one within a group: one-hot structure
            }
        }
        if !c.iter().all(|l| !l.is_positive()) {
            return None; // anything mixing polarities is not part of the recognized encoding
        }
        let mut pairs: Vec<(usize, u64)> = Vec::new();
        let mut seen = HashSet::new();
        for l in c {
            let g = *var_to_group.get(&l.var())?;
            if !seen.insert(g) {
                return None; // two bits of the same group in one forbidden combo
            }
            pairs.push((g, pos_in_group(l.var(), g)));
        }
        pairs.sort_by_key(|&(g, _)| g);
        let scope: Vec<usize> = pairs.iter().map(|&(g, _)| g).collect();
        let tuple: Vec<u64> = pairs.iter().map(|&(_, v)| v).collect();
        scopes.entry(scope).or_default().push(tuple);
    }
    if scopes.is_empty() {
        return None;
    }

    // For each scope, fit the unique congruence whose violated set is EXACTLY the forbidden tuples.
    let mut equations: Vec<ModpEquation> = Vec::new();
    for (scope, forbidden) in &scopes {
        let k = scope.len();
        let total = (m as u128).checked_pow(k as u32)?;
        if total > (1u128 << 22) {
            return None; // refuse to enumerate an oversized scope; let CDCL have it
        }
        let forbidden_set: HashSet<Vec<u64>> = forbidden.iter().cloned().collect();
        let mut allowed: Vec<Vec<u64>> = Vec::new();
        for idx in 0..total {
            let mut t = vec![0u64; k];
            let mut x = idx;
            for slot in t.iter_mut() {
                *slot = (x % m as u128) as u64;
                x /= m as u128;
            }
            if !forbidden_set.contains(&t) {
                allowed.push(t);
            }
        }
        let (a, c) = fit_congruence(k, &allowed, forbidden, m)?;
        let coeffs: Vec<(usize, u64)> =
            scope.iter().enumerate().map(|(i, &g)| (g, a[i])).filter(|&(_, ai)| ai != 0).collect();
        if coeffs.is_empty() {
            return None;
        }
        equations.push(ModpEquation::new(coeffs, c));
    }

    Some(ModpRecovery { modulus: m, num_vars: groups.len(), equations, groups })
}

/// `|GL(n,p)|` over `GF(p)` via the orbit–stabilizer (ordered-basis) factorization
/// `Π_{i=0}^{n-1}(pⁿ − pⁱ)`. `GL(n,p)` acts **simply transitively on ordered bases** of `GF(p)ⁿ`, so each
/// factor `pⁿ − pⁱ` counts the vectors outside the i-dimensional span built so far — the same symmetry
/// break as over `GF(2)`, now across the field. The `p = 2` case is `gf2::gl_order`.
pub fn gl_order_p(n: u32, p: u64) -> u128 {
    let pn = (p as u128).pow(n);
    (0..n).map(|i| pn - (p as u128).pow(i)).product()
}

/// Is an `n×n` matrix over `GF(p)` invertible? Gaussian elimination with modular pivots: full rank `n`.
/// `p` must be prime.
pub fn is_invertible_modp(n: usize, p: u64, matrix: &[Vec<u64>]) -> bool {
    let mut a: Vec<Vec<u64>> = matrix.iter().map(|r| r.iter().map(|&x| x % p).collect()).collect();
    let mut rank = 0usize;
    for col in 0..n {
        if let Some(piv) = (rank..n).find(|&r| a[r][col] != 0) {
            a.swap(rank, piv);
            let pinv = inv(a[rank][col], p);
            for c in 0..n {
                a[rank][c] = mul(a[rank][c], pinv, p);
            }
            for r in 0..n {
                if r != rank && a[r][col] != 0 {
                    let f = a[r][col];
                    for c in 0..n {
                        a[r][c] = sub(a[r][c], mul(f, a[rank][c], p), p);
                    }
                }
            }
            rank += 1;
        }
    }
    rank == n
}

/// The invertibility *density* over `GF(p)`: `Π_{j=1}^n (1 − p⁻ʲ) = |GL(n,p)| / p^{n²}`. As the field
/// grows the density → 1 (fewer linear collisions); `p = 2` is the densest-collision regime, the smallest
/// constant `φ(½) ≈ 0.28879`.
pub fn invertibility_density_p(n: u32, p: u64) -> f64 {
    (1..=n).map(|j| 1.0 - (p as f64).powi(-(j as i32))).product()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A tiny seeded SplitMix64 — reproducible, no wall-clock.
    fn splitmix(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn count_invertible_modp_bruteforce(n: usize, p: u64) -> u128 {
        let cells = (n * n) as u32;
        let total = (p as u128).pow(cells);
        let mut count = 0u128;
        for idx in 0..total {
            let mut m = vec![vec![0u64; n]; n];
            let mut x = idx;
            for row in m.iter_mut() {
                for cell in row.iter_mut() {
                    *cell = (x % p as u128) as u64;
                    x /= p as u128;
                }
            }
            if is_invertible_modp(n, p, &m) {
                count += 1;
            }
        }
        count
    }

    /// **The symmetry break across the field: the invertible count is `|GL(n,p)|` over every `GF(p)`.**
    /// Brute force over all `p^{n²}` matrices equals the orbit–stabilizer product `Π(pⁿ − pⁱ)` — the
    /// invertible matrices are exactly the ordered bases, the simply-transitive orbit of `GL(n,p)`. Proven
    /// exhaustively over several prime fields, with pinned group orders.
    #[test]
    fn gl_order_p_is_the_invertible_count_over_gf_p() {
        for &(n, p) in &[(1usize, 2u64), (1, 3), (2, 2), (2, 3), (2, 5), (2, 7), (3, 2)] {
            assert_eq!(
                count_invertible_modp_bruteforce(n, p),
                gl_order_p(n as u32, p),
                "brute invertible count over GF({p}) must equal |GL({n},{p})| = Π(pⁿ−pⁱ)"
            );
        }
        assert_eq!(gl_order_p(2, 2), 6, "|GL(2,2)| = 6 ≅ S₃");
        assert_eq!(gl_order_p(2, 3), 48, "|GL(2,3)| = 48");
        assert_eq!(gl_order_p(2, 5), 480, "|GL(2,5)| = 480");
        assert_eq!(gl_order_p(3, 2), 168, "|GL(3,2)| = 168");
    }

    /// **The field size is a symmetry axis, and `GF(2)` is its densest-collision end.** The density
    /// `Π(1−p⁻ʲ) = |GL(n,p)|/p^{n²}` is exact, increases strictly with `p` (bigger field ⟹ fewer linear
    /// collisions ⟹ more likely invertible), and → 1 as `p → ∞`. The `p = 2` value is `φ(½)`, the smallest
    /// — and it agrees with the dedicated GF(2) module (cross-check).
    #[test]
    fn the_field_size_is_a_symmetry_axis() {
        // Exact |GL|/p^(n²) == density — small n only, before the u128 product (≈ p^(n²)) overflows.
        for &p in &[2u64, 3, 5, 7] {
            for n in 1..=4u32 {
                let exact = gl_order_p(n, p) as f64 / (p as f64).powi((n * n) as i32);
                assert!((invertibility_density_p(n, p) - exact).abs() < 1e-12, "density == |GL|/p^(n²) at n={n},p={p}");
            }
        }
        // strictly increasing in p (denser field ⟹ closer to always-invertible), at fixed n
        let n = 6u32;
        let dens: Vec<f64> = [2u64, 3, 5, 7, 11].iter().map(|&p| invertibility_density_p(n, p)).collect();
        for w in dens.windows(2) {
            assert!(w[1] > w[0], "density increases with the field size: {dens:?}");
        }
        // GF(2) is the densest-collision regime — the smallest constant, exactly φ(½)
        assert!((invertibility_density_p(40, 2) - 0.288_788_095_1).abs() < 1e-9, "GF(2) density → φ(½)");
        // cross-module agreement: the p=2 specialization is the gf2 module's own constant
        for n in 1..=10u32 {
            assert!((invertibility_density_p(n, 2) - crate::gf2::invertibility_density(n)).abs() < 1e-12, "modp p=2 == gf2 at n={n}");
            assert_eq!(gl_order_p(n, 2), crate::gf2::gl_order(n), "|GL(n,2)| agrees across modules at n={n}");
        }
    }

    fn brute_force_sat(equations: &[ModpEquation], num_vars: usize, p: u64) -> bool {
        let total = (p as u128).pow(num_vars as u32);
        for code in 0..total {
            let mut a = vec![0u64; num_vars];
            let mut c = code;
            for slot in a.iter_mut() {
                *slot = (c % p as u128) as u64;
                c /= p as u128;
            }
            if satisfies(equations, &a, p) {
                return true;
            }
        }
        false
    }

    /// The certified mod-`p` cut, verified to the point of absurdity against brute force: over `GF(3)`
    /// and `GF(5)`, on a fuzz of random systems, `solve`'s verdict always matches exhaustive search —
    /// every `Sat` witness satisfies, every `Unsat` refutation independently re-checks.
    #[test]
    fn modp_gaussian_matches_brute_force() {
        for &p in &[2u64, 3, 5, 7] {
            let mut state = 0x1234_5678u64 ^ p;
            for _ in 0..40 {
                let num_vars = 2 + (splitmix(&mut state) % 3) as usize; // 2..4 variables
                let num_eqs = 1 + (splitmix(&mut state) % 5) as usize; // 1..5 equations
                let equations: Vec<ModpEquation> = (0..num_eqs)
                    .map(|_| {
                        let coeffs: Vec<(usize, u64)> = (0..num_vars)
                            .map(|v| (v, splitmix(&mut state) % p))
                            .filter(|&(_, a)| a != 0)
                            .collect();
                        ModpEquation::new(coeffs, splitmix(&mut state) % p)
                    })
                    .collect();
                let brute = brute_force_sat(&equations, num_vars, p);
                match solve(&equations, num_vars, p) {
                    ModpOutcome::Sat(a) => {
                        assert!(brute, "p={p}: solver Sat but brute force UNSAT: {equations:?}");
                        assert!(satisfies(&equations, &a, p), "p={p}: the model must satisfy: {a:?}");
                    }
                    ModpOutcome::Unsat(combo) => {
                        assert!(!brute, "p={p}: solver Unsat but a model exists: {equations:?}");
                        assert!(
                            is_refutation(&equations, num_vars, p, &combo),
                            "p={p}: the refutation must re-check: {combo:?}"
                        );
                    }
                }
            }
        }
    }

    /// A concrete mod-3 inconsistency the GF(2) parity cut is **blind** to. The system `x+y+z ≡ 0` and
    /// `x+y+z ≡ 2 (mod 3)` is inconsistent over `GF(3)` (subtract: `0 ≡ 2`). But reduce the right-hand
    /// sides mod 2 and *both* become `x+y+z ≡ 0` — the very same GF(2) equation, perfectly consistent.
    /// So a parity (GF(2)) solver sees no conflict; only the mod-3 cut refutes it, with a re-checkable
    /// certificate. The new field genuinely reaches a class the old one cannot.
    #[test]
    fn mod3_inconsistency_is_invisible_to_gf2() {
        let p = 3;
        let eqs = vec![
            ModpEquation::new(vec![(0, 1), (1, 1), (2, 1)], 0),
            ModpEquation::new(vec![(0, 1), (1, 1), (2, 1)], 2),
        ];
        match solve(&eqs, 3, p) {
            ModpOutcome::Unsat(combo) => {
                assert!(is_refutation(&eqs, 3, p, &combo), "the mod-3 refutation re-checks: {combo:?}");
            }
            other => panic!("expected the mod-3 system to be refuted, got {other:?}"),
        }
        // Over GF(2) both right-hand sides collapse to 0 — the same equation twice, satisfiable — so the
        // parity cut sees no conflict where the mod-3 cut crushes.
        let gf2_rhs: Vec<u64> = eqs.iter().map(|e| e.rhs % 2).collect();
        assert_eq!(gf2_rhs, vec![0, 0], "the GF(2) reduction has no conflict — parity is blind here");
    }

    /// **The scalable mod-p crush — and the cross-field punch.** The cycle obstruction is refuted in
    /// polynomial time exactly when `n` is not a multiple of `p`, with the all-ones combination as the
    /// re-checkable witness — at every length. And a 4-cycle is satisfiable over `GF(2)` (the parity
    /// cut sees nothing) yet refuted over `GF(3)`: the new field reaches a class the old one cannot.
    #[test]
    fn the_mod_p_cycle_obstruction_crushed_at_scale() {
        for &p in &[3u64, 5, 7] {
            for n in 2..=40 {
                let eqs = cycle_system(n, p);
                match solve(&eqs, n, p) {
                    ModpOutcome::Unsat(combo) => {
                        assert_ne!(n as u64 % p, 0, "p={p} n={n}: refuted ⟹ n not a multiple of p");
                        assert!(is_refutation(&eqs, n, p, &combo), "p={p} n={n}: cycle refutation re-checks");
                    }
                    ModpOutcome::Sat(a) => {
                        assert_eq!(n as u64 % p, 0, "p={p} n={n}: satisfiable ⟹ n is a multiple of p");
                        assert!(satisfies(&eqs, &a, p), "p={p} n={n}: the model must satisfy");
                    }
                }
            }
        }
        // The cross-field punch: a 4-cycle is SAT over GF(2) but UNSAT over GF(3).
        assert!(matches!(solve(&cycle_system(4, 2), 4, 2), ModpOutcome::Sat(_)), "4-cycle SAT over GF(2)");
        assert!(matches!(solve(&cycle_system(4, 3), 4, 3), ModpOutcome::Unsat(_)), "4-cycle UNSAT over GF(3)");
    }

    /// **`modp` over GF(2) *is* `xorsat`** — proven by a differential fuzz, not asserted. On 50 random
    /// GF(2) systems the mod-2 Gaussian and the dedicated parity engine agree on every verdict, so the
    /// new field is a faithful generalization of the old one (which is itself brute-force-verified).
    #[test]
    fn modp_over_gf2_agrees_with_xorsat() {
        use crate::xorsat::{self, XorEquation, XorOutcome};
        let mut state = 0x00AB_CDEFu64;
        for _ in 0..50 {
            let num_vars = 2 + (splitmix(&mut state) % 4) as usize;
            let num_eqs = 1 + (splitmix(&mut state) % 5) as usize;
            let systems: Vec<(Vec<usize>, bool)> = (0..num_eqs)
                .map(|_| {
                    let vars: Vec<usize> =
                        (0..num_vars).filter(|_| splitmix(&mut state) % 2 == 0).collect();
                    (vars, splitmix(&mut state) % 2 == 1)
                })
                .collect();
            let xor_eqs: Vec<XorEquation> =
                systems.iter().map(|(v, r)| XorEquation::new(v.clone(), *r)).collect();
            let modp_eqs: Vec<ModpEquation> = systems
                .iter()
                .map(|(v, r)| {
                    ModpEquation::new(v.iter().map(|&x| (x, 1u64)).collect::<Vec<_>>(), *r as u64)
                })
                .collect();
            let xor_unsat = matches!(xorsat::solve(&xor_eqs, num_vars), XorOutcome::Unsat(_));
            let modp_unsat = matches!(solve(&modp_eqs, num_vars, 2), ModpOutcome::Unsat(_));
            assert_eq!(xor_unsat, modp_unsat, "modp(p=2) must match xorsat on {systems:?}");
        }
    }

    /// The mod-`p` cut decides a satisfiable system and returns a real model.
    #[test]
    fn modp_solves_a_consistent_system() {
        // Over GF(5): x + 2y ≡ 3, 3y ≡ 4  ⟹  y ≡ 3 (3⁻¹=2, 2·4=8≡3), x ≡ 3 − 2·3 = −3 ≡ 2.
        let eqs = vec![
            ModpEquation::new(vec![(0, 1), (1, 2)], 3),
            ModpEquation::new(vec![(1, 3)], 4),
        ];
        match solve(&eqs, 2, 5) {
            ModpOutcome::Sat(a) => {
                assert!(satisfies(&eqs, &a, 5), "model must satisfy: {a:?}");
                assert_eq!(a, vec![2, 3], "the unique solution over GF(5)");
            }
            other => panic!("expected Sat, got {other:?}"),
        }
    }

    /// **The GF(p) lift is faithful at every prime.** For `p ∈ {3,5,7}` and several graph sizes/seeds,
    /// `recover_from_cnf` reconstructs the system from the opaque one-hot CNF, and its verdict matches
    /// both the hand-built supplied system *and* the family's declared UNSAT/SAT — on the inconsistent
    /// (Tseitin) and the consistent forms alike. The recovery variables align with the edges, so the
    /// recovered system is the same dimension as the supplied one. This is the equisatisfiability the
    /// soundness of the dispatcher route rests on, proven across the field.
    #[test]
    fn recover_from_cnf_is_faithful_across_primes() {
        use crate::families::{mod_p_consistent_onehot, mod_p_tseitin_expander, ExpectedVerdict};
        for &p in &[3u64, 5, 7] {
            for &n in &[4usize, 6, 8] {
                for seed in 0..3u64 {
                    for (supplied, cnf, expect) in
                        [mod_p_tseitin_expander(n, p, seed), mod_p_consistent_onehot(n, p, seed)]
                    {
                        let rec = recover_from_cnf(cnf.num_vars, &cnf.clauses).unwrap_or_else(|| {
                            panic!("p={p} n={n} seed={seed}: must recover the GF(p) system")
                        });
                        assert_eq!(rec.modulus, p, "p={p} n={n} seed={seed}: recovered the right field");
                        let rec_unsat =
                            matches!(solve(&rec.equations, rec.num_vars, p), ModpOutcome::Unsat(_));
                        let sup_unsat =
                            matches!(solve(&supplied, rec.num_vars, p), ModpOutcome::Unsat(_));
                        assert_eq!(rec_unsat, sup_unsat, "p={p} n={n} seed={seed}: recovered vs supplied");
                        assert_eq!(
                            rec_unsat,
                            matches!(expect, ExpectedVerdict::Unsat),
                            "p={p} n={n} seed={seed}: verdict matches the family's expectation"
                        );
                    }
                }
            }
        }
    }

    /// **Soundness of declining.** The recoverer must guess nothing: it returns `None` on inputs that are
    /// not a clean one-hot mod-`p` encoding — random 3-SAT (no groups), and an all-positive clause whose
    /// pairwise at-most-one structure is incomplete (so "exactly one" is not actually enforced).
    #[test]
    fn recover_declines_on_inputs_that_are_not_a_one_hot_encoding() {
        use crate::cdcl::Lit;
        let rnd = crate::families::random_3sat(20, 80, 0xF00D);
        assert!(recover_from_cnf(rnd.num_vars, &rnd.clauses).is_none(), "random 3-SAT has no GF(p) structure");
        let incomplete = vec![
            vec![Lit::pos(0), Lit::pos(1), Lit::pos(2)],
            vec![Lit::neg(0), Lit::neg(1)], // only one of the three at-most-one pairs
        ];
        assert!(
            recover_from_cnf(3, &incomplete).is_none(),
            "an incomplete at-most-one must not be mistaken for a one-hot group"
        );
    }
}
