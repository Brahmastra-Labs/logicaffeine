//! **The affine group `AGL(n,2)` — the symmetry a permutation break cannot see.**
//!
//! The clause-level symmetry breakers (`symmetry_detect`, `sym_certify`) quotient by the hyperoctahedral
//! group `Bₙ` of signed variable permutations. `Bₙ` is exactly the subgroup of `AGL(n,2)` (affine
//! bijections `x ↦ A x ⊕ b` of the cube `GF(2)ⁿ`) whose linear part `A` is a permutation matrix. The
//! census proved the clause breakers are *complete* for `Bₙ` — yet `AGL(n,2) ⊋ Bₙ`, and the gap is the
//! **shears** `xᵢ ↦ xᵢ ⊕ xⱼ`. A shear maps an axis-aligned subcube (a clause) to a *non*-subcube, so no
//! amount of clause permutation can reach it. That gap is precisely why parity / Tseitin formulas — affine
//! invariant, `Bₙ`-rigid — defeat clause symmetry breaking and need a linear-algebra engine instead.
//!
//! This module is that engine, at the symmetry level: the `AGL(n,2)` group itself, an exhaustive
//! ground-truth detector of a formula's affine symmetries (so `AGL ⊋ Bₙ` is *measured*, not asserted), and
//! the affine **break** — recover the formula's `GF(2)`-linear substructure, Gauss-eliminate it, and either
//! refute an inconsistent linear core or inject the forced units / equivalences (the linear consequences no
//! single clause states) that collapse the residual search.

use std::collections::HashSet;

use crate::cdcl::Lit;
use crate::gf2;

/// An affine map of the Boolean cube `GF(2)ⁿ`: `x ↦ A x ⊕ b`. `matrix[i]` is row `i` of `A` as a
/// coefficient bitmask over the low `n` bits; `translation` is `b`. It is a bijection iff `A ∈ GL(n,2)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Affine {
    pub n: usize,
    pub matrix: Vec<u64>,
    pub translation: u64,
}

impl Affine {
    /// The identity `x ↦ x` (`A = I`, `b = 0`).
    pub fn identity(n: usize) -> Self {
        Affine {
            n,
            matrix: (0..n).map(|i| 1u64 << i).collect(),
            translation: 0,
        }
    }

    /// Apply the map to a point `x` (low `n` bits): output bit `i` is `parity(row_i · x) ⊕ b_i`.
    pub fn apply(&self, x: u64) -> u64 {
        let mut y = 0u64;
        for i in 0..self.n {
            let dot = (self.matrix[i] & x).count_ones() & 1;
            let bit = dot ^ ((self.translation >> i) & 1) as u32;
            y |= (bit as u64) << i;
        }
        y
    }

    /// Composition `self ∘ other`: `(self ∘ other)(x) = self(other(x))`. The linear parts multiply over
    /// `GF(2)` and the translations combine as `A_self · b_other ⊕ b_self`.
    pub fn compose(&self, other: &Affine) -> Affine {
        debug_assert_eq!(self.n, other.n);
        // Row i of A_self·A_other = XOR of A_other's rows selected by the set bits of A_self's row i.
        let mut matrix = vec![0u64; self.n];
        for i in 0..self.n {
            let mut row = 0u64;
            let mut sel = self.matrix[i];
            while sel != 0 {
                let k = sel.trailing_zeros() as usize;
                row ^= other.matrix[k];
                sel &= sel - 1;
            }
            matrix[i] = row;
        }
        // A_self applied to b_other, then ⊕ b_self.
        let mut translation = self.translation;
        for i in 0..self.n {
            let dot = (self.matrix[i] & other.translation).count_ones() & 1;
            translation ^= (dot as u64) << i;
        }
        Affine {
            n: self.n,
            matrix,
            translation,
        }
    }

    /// Whether the linear part is invertible (so the map is a bijection of the cube).
    pub fn is_bijection(&self) -> bool {
        gf2::is_invertible_gf2(self.n as u32, &self.matrix)
    }
}

/// `|AGL(n,2)| = 2ⁿ · |GL(n,2)|` — the `2ⁿ` translations times the invertible linear parts.
pub fn agl_order(n: u32) -> u128 {
    (1u128 << n) * gf2::gl_order(n)
}

/// **The affine automorphism group order of a `k`-dimensional affine subspace of `GF(2)ⁿ`** — i.e. the number
/// of affine maps `x ↦ Ax ⊕ b` fixing the model set of an XOR-defined family whose solution space has
/// dimension `k`. In *closed form*, computable at every `n` without enumerating `AGL(n,2)`:
/// `|GL(k,2)| · |GL(n−k,2)| · 2^{k(n−k)} · 2^k` — the block-upper-triangular invertible linear parts (`GL(k)`
/// on the subspace, `GL(n−k)` on the quotient, `2^{k(n−k)}` shears between them) times the `2^k` in-subspace
/// translations. The scalable affine-symmetry detector for the whole class of linear families: `Bₙ` sees a
/// vanishing fraction of it (see the tests).
pub fn affine_subspace_agl_order(n: u32, k: u32) -> u128 {
    debug_assert!(k <= n);
    gf2::gl_order(k) * gf2::gl_order(n - k) * (1u128 << (k * (n - k))) * (1u128 << k)
}

/// Every affine bijection of `GF(2)ⁿ` (each invertible matrix × each translation). Exhaustive — for
/// ground-truth symmetry computation only — so it is capped at `n ≤ 4` (`|AGL(4,2)| = 322 560`).
pub fn all_affine_bijections(n: usize) -> Vec<Affine> {
    assert!(
        n <= 4,
        "exhaustive AGL enumeration is for ground-truth tests only (n ≤ 4)"
    );
    let mut out = Vec::new();
    let total_matrices = 1u64 << (n * n);
    for code in 0..total_matrices {
        let matrix: Vec<u64> = (0..n).map(|i| (code >> (i * n)) & ((1 << n) - 1)).collect();
        if !gf2::is_invertible_gf2(n as u32, &matrix) {
            continue;
        }
        for translation in 0..(1u64 << n) {
            out.push(Affine {
                n,
                matrix: matrix.clone(),
                translation,
            });
        }
    }
    out
}

/// The satisfying assignments of a CNF, each packed into the low `num_vars` bits of a `u64`. Brute force
/// over `2^{num_vars}` — small `n` only (the census / symmetry-measurement regime).
pub fn models_of(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<u64> {
    assert!(
        num_vars <= 24,
        "model enumeration is brute force — small n only"
    );
    (0u64..(1u64 << num_vars))
        .filter(|&x| {
            clauses.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 == 1) == l.is_positive())
            })
        })
        .collect()
}

/// The **affine symmetry group of a formula**, computed exhaustively: every `φ ∈ AGL(n,2)` that maps the
/// model set onto itself (`φ` a bijection, so `φ(models) = models` iff `φ` maps each model to a model).
/// This is the `AGL(n,2)` analogue of `symmetry_detect::find_generators` (which finds only the `Bₙ`
/// part), and it is exact for `n ≤ 4` — the instrument that *measures* `AGL ⊋ Bₙ`.
pub fn affine_symmetries(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Affine> {
    let models: HashSet<u64> = models_of(num_vars, clauses).into_iter().collect();
    all_affine_bijections(num_vars)
        .into_iter()
        .filter(|phi| models.iter().all(|&m| models.contains(&phi.apply(m))))
        .collect()
}

/// Pack XOR equations into `(coefficient-mask, rhs)` rows over the low bits, for [`gf2::solve_gf2`].
fn eqs_to_rows(eqs: &[crate::xorsat::XorEquation]) -> (Vec<u64>, Vec<bool>) {
    let mut rows = Vec::with_capacity(eqs.len());
    let mut rhs = Vec::with_capacity(eqs.len());
    for eq in eqs {
        let mut mask = 0u64;
        for &v in &eq.vars {
            mask |= 1u64 << v;
        }
        rows.push(mask);
        rhs.push(eq.rhs);
    }
    (rows, rhs)
}

/// Recover the `GF(2)`-linear substructure of a CNF: the XOR equations its clauses encode (units,
/// binary equivalences, and complete wrong-parity bundles), as `(coefficient-mask, rhs)` rows over the
/// low `num_vars` bits. `None` when there is no linear structure or `num_vars` exceeds the `u64` mask.
pub fn recover_linear_system(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<(Vec<u64>, Vec<bool>)> {
    if num_vars > 63 {
        return None;
    }
    let eqs = crate::lyapunov::extract_xor(num_vars, clauses);
    if eqs.is_empty() {
        return None;
    }
    Some(eqs_to_rows(&eqs))
}

/// The clausal DRAT **certificate** for an affine refutation, or `None` if the formula's linear core is
/// not inconsistent (or `num_vars` exceeds the `u64` mask, or the resolution expansion overruns its
/// budget). Recovers the XOR system, finds the GF(2) linear dependency that sums to `0 = 1`
/// ([`crate::xorsat::solve`]), and compiles it to RUP resolvent lemmas through the [`crate::xor_drat`]
/// bridge — the same path `Route::Parity` uses, so it is `drat-trim`-checkable against the original CNF.
pub fn affine_refutation_drat(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<Lit>>> {
    if num_vars > 63 {
        return None;
    }
    let eqs = crate::lyapunov::extract_xor(num_vars, clauses);
    match crate::xorsat::solve(&eqs, num_vars) {
        crate::xorsat::XorOutcome::Unsat(refutation) => {
            crate::xor_drat::emit_xor_drat(&eqs, &refutation)
        }
        crate::xorsat::XorOutcome::Sat(_) => None,
    }
}

/// The outcome of the affine break.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AffineOutcome {
    /// The recovered linear system is inconsistent (`0 = 1`) ⇒ the formula is UNSAT, by an obstruction no
    /// clause permutation can express. The payload is the **certificate**: the GF(2) linear-dependency
    /// refutation compiled to clausal DRAT resolvents through the [`crate::xor_drat`] bridge (a sequence
    /// of RUP lemmas that `drat-trim` / [`crate::rup::check_refutation`] verifies against the original
    /// CNF), or `None` when the resolution expansion exceeds its budget — the verdict still stands on the
    /// native algebraic certificate, exactly as the `Route::Parity` cut does.
    Refuted(Option<Vec<Vec<Lit>>>),
    /// Consistent, but Gauss elimination forced linear consequences — units and/or equivalences not stated
    /// by any single clause. The carried clauses are sound to conjoin (implied by the formula) and are
    /// genuinely new (none already present), so adding them strictly strengthens it for search.
    Forced(Vec<Vec<Lit>>),
    /// No linear structure, or nothing new derivable.
    Unchanged,
}

/// Sorted `(var, polarity)` key for clause-set membership.
fn canon(clause: &[Lit]) -> Vec<(u32, bool)> {
    let mut k: Vec<(u32, bool)> = clause.iter().map(|l| (l.var(), l.is_positive())).collect();
    k.sort_unstable();
    k.dedup();
    k
}

/// The **affine `GF(2)` break.** Recover the formula's linear substructure and Gauss-eliminate it
/// ([`gf2::solve_gf2`]): an inconsistent core [`AffineOutcome::Refuted`]s outright; otherwise the
/// elimination's [`gf2::SolutionSpace`] exposes which variables are *forced* (the kernel never moves them
/// ⇒ a unit) and which are *locked together* (the kernel moves them in lockstep ⇒ an equivalence), and we
/// return those consequences as new clauses ([`AffineOutcome::Forced`]) — the linear inferences the
/// permutation breaker structurally cannot make.
pub fn affine_reduce(num_vars: usize, clauses: &[Vec<Lit>]) -> AffineOutcome {
    if num_vars > 63 {
        return AffineOutcome::Unchanged;
    }
    let eqs = crate::lyapunov::extract_xor(num_vars, clauses);
    if eqs.is_empty() {
        return AffineOutcome::Unchanged;
    }
    // Inconsistent linear core ⇒ UNSAT, certified through the xor_drat bridge: the GF(2) linear-dependency
    // witness compiles to clausal DRAT resolvents that re-check against the original CNF.
    if let crate::xorsat::XorOutcome::Unsat(refutation) = crate::xorsat::solve(&eqs, num_vars) {
        return AffineOutcome::Refuted(crate::xor_drat::emit_xor_drat(&eqs, &refutation));
    }
    // Consistent: derive forced consequences from the RREF solution space.
    let (rows, rhs) = eqs_to_rows(&eqs);
    let Some(ss) = gf2::solve_gf2(num_vars, &rows, &rhs) else {
        return AffineOutcome::Unchanged; // unreachable: xorsat consistent ⇒ solve_gf2 consistent
    };

    let existing: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| canon(c)).collect();
    let mut forced: Vec<Vec<Lit>> = Vec::new();
    let push_new = |clause: Vec<Lit>, forced: &mut Vec<Vec<Lit>>| {
        if !existing.contains(&canon(&clause)) {
            forced.push(clause);
        }
    };

    // A variable the kernel never moves is pinned to its value in every solution — a forced unit.
    let mut is_forced = vec![false; num_vars];
    for v in 0..num_vars {
        if ss.kernel_basis.iter().all(|k| !k[v]) {
            is_forced[v] = true;
            push_new(vec![Lit::new(v as u32, ss.particular[v])], &mut forced);
        }
    }
    // A pair the kernel always moves together is locked into `x_u ⊕ x_v = c` in every solution — a forced
    // equivalence. (Skip pairs with a forced endpoint; the units already pin them.)
    for u in 0..num_vars {
        if is_forced[u] {
            continue;
        }
        for v in (u + 1)..num_vars {
            if is_forced[v] {
                continue;
            }
            if ss.kernel_basis.iter().all(|k| k[u] == k[v]) {
                let (lu, lv) = (u as u32, v as u32);
                if ss.particular[u] ^ ss.particular[v] {
                    // x_u ≠ x_v: (x_u ∨ x_v) ∧ (¬x_u ∨ ¬x_v)
                    push_new(vec![Lit::pos(lu), Lit::pos(lv)], &mut forced);
                    push_new(vec![Lit::neg(lu), Lit::neg(lv)], &mut forced);
                } else {
                    // x_u = x_v: (¬x_u ∨ x_v) ∧ (x_u ∨ ¬x_v)
                    push_new(vec![Lit::neg(lu), Lit::pos(lv)], &mut forced);
                    push_new(vec![Lit::pos(lu), Lit::neg(lv)], &mut forced);
                }
            }
        }
    }

    if forced.is_empty() {
        AffineOutcome::Unchanged
    } else {
        AffineOutcome::Forced(forced)
    }
}

/// How an eliminated variable is recovered from the reduced model — its place in the affine quotient.
#[derive(Clone, Debug, PartialEq, Eq)]
enum VarSub {
    /// Forced to a constant by the linear core (the kernel never moves it).
    Const(bool),
    /// Survives as reduced variable `new_index` (a free generator or a class representative).
    Survive(u32),
    /// Equal to reduced variable `new_index`, XORed with `flip` — a member of an equivalence class.
    Alias(u32, bool),
}

/// The result of the canonical affine reduction: an equisatisfiable formula over the **free generators**
/// (the linearly-determined variables eliminated), plus the map to lift a reduced model back to the full
/// space. This is the affine analogue of breaking a permutation orbit down to one representative — here
/// the RREF canonicalizes the affine structure and the determined coordinates fall away.
#[derive(Clone, Debug)]
pub struct AffineCanonical {
    pub num_vars: usize,
    pub clauses: Vec<Vec<Lit>>,
    sub: Vec<VarSub>,
}

impl AffineCanonical {
    /// Lift a model of the reduced formula back to a model over the original variables, reconstructing
    /// each eliminated coordinate from its canonical definition (constant, or alias of a survivor).
    pub fn lift(&self, reduced_model: &[bool]) -> Vec<bool> {
        self.sub
            .iter()
            .map(|s| match *s {
                VarSub::Const(c) => c,
                VarSub::Survive(ni) => reduced_model[ni as usize],
                VarSub::Alias(ni, flip) => reduced_model[ni as usize] ^ flip,
            })
            .collect()
    }
}

/// The outcome of the **canonical RREF break**.
pub enum AffineCanon {
    /// The linear core is inconsistent — UNSAT, with the xor_drat certificate (as [`AffineOutcome::Refuted`]).
    Refuted(Option<Vec<Vec<Lit>>>),
    /// Reduced to the free generators, with the lifting map.
    Canonical(AffineCanonical),
    /// No linear structure, or nothing determined to eliminate.
    Unchanged,
}

/// **The affine SBP — the canonical RREF break.** Recover the formula's GF(2)-linear substructure and
/// take its reduced row-echelon form (via [`gf2::solve_gf2`], whose kernel basis is the affine
/// translation symmetry). The RREF partitions variables into *free generators* and *determined*
/// coordinates: every variable the kernel never moves is **forced** to a constant, and every set the
/// kernel moves in lockstep is an **equivalence class** collapsing to one representative. Substituting
/// those determined coordinates out yields an equisatisfiable formula over the free generators alone — the
/// affine symmetry quotiented to a canonical representative, exactly as a permutation lex-leader collapses
/// an orbit. An inconsistent core instead [`AffineCanon::Refuted`]s (certified). Sound: the eliminated
/// relations are GF(2)-entailed by the formula, so [`AffineCanonical::lift`] turns any reduced model into
/// a full one.
pub fn affine_canonicalize(num_vars: usize, clauses: &[Vec<Lit>]) -> AffineCanon {
    if num_vars > 63 {
        return AffineCanon::Unchanged;
    }
    let eqs = crate::lyapunov::extract_xor(num_vars, clauses);
    if eqs.is_empty() {
        return AffineCanon::Unchanged;
    }
    if let crate::xorsat::XorOutcome::Unsat(refutation) = crate::xorsat::solve(&eqs, num_vars) {
        return AffineCanon::Refuted(crate::xor_drat::emit_xor_drat(&eqs, &refutation));
    }
    let (rows, rhs) = eqs_to_rows(&eqs);
    let Some(ss) = gf2::solve_gf2(num_vars, &rows, &rhs) else {
        return AffineCanon::Unchanged; // unreachable: xorsat consistent ⇒ solve_gf2 consistent
    };

    // A variable's "kernel column" — which kernel-basis vectors move it. Forced variables have an
    // all-zero column; variables sharing a column move in lockstep (an equivalence class).
    let kdim = ss.kernel_basis.len();
    let column = |v: usize| -> Vec<bool> { (0..kdim).map(|i| ss.kernel_basis[i][v]).collect() };
    let zero = vec![false; kdim];

    let mut sub = vec![VarSub::Const(false); num_vars];
    let mut groups: std::collections::HashMap<Vec<bool>, Vec<usize>> =
        std::collections::HashMap::new();
    for v in 0..num_vars {
        let col = column(v);
        if col == zero {
            sub[v] = VarSub::Const(ss.particular[v]); // forced
        } else {
            groups.entry(col).or_default().push(v);
        }
    }
    // Each class collapses to its lowest-index representative; assign reduced indices in representative order.
    let mut classes: Vec<Vec<usize>> = groups
        .into_values()
        .map(|mut g| {
            g.sort_unstable();
            g
        })
        .collect();
    classes.sort_unstable_by_key(|g| g[0]);
    for (new_index, members) in classes.iter().enumerate() {
        let rep = members[0];
        let rep_par = ss.particular[rep];
        for &v in members {
            sub[v] = if v == rep {
                VarSub::Survive(new_index as u32)
            } else {
                VarSub::Alias(new_index as u32, ss.particular[v] ^ rep_par) // x_v = x_rep ⊕ flip
            };
        }
    }
    let reduced_nv = classes.len();
    if !sub
        .iter()
        .any(|s| matches!(s, VarSub::Const(_) | VarSub::Alias(_, _)))
    {
        return AffineCanon::Unchanged; // nothing determined to eliminate
    }

    // Apply the substitution to every clause (linear and clausal alike), staying in CNF.
    let mut out: Vec<Vec<Lit>> = Vec::new();
    'clause: for c in clauses {
        let mut seen: std::collections::HashMap<u32, bool> = std::collections::HashMap::new();
        let mut lits: Vec<Lit> = Vec::new();
        for l in c {
            let (ni, pol) = match sub[l.var() as usize] {
                VarSub::Const(cst) => {
                    if cst == l.is_positive() {
                        continue 'clause; // literal true ⇒ clause satisfied, drop it
                    }
                    continue; // literal false ⇒ drop the literal
                }
                VarSub::Survive(ni) => (ni, l.is_positive()),
                VarSub::Alias(ni, flip) => (ni, l.is_positive() ^ flip),
            };
            match seen.get(&ni) {
                Some(&prev) if prev != pol => continue 'clause, // x ∨ ¬x ⇒ tautology, drop the clause
                Some(_) => continue,                            // duplicate literal
                None => {
                    seen.insert(ni, pol);
                    lits.push(Lit::new(ni, pol));
                }
            }
        }
        out.push(lits); // a now-empty clause is a sound UNSAT marker the solver will catch
    }

    AffineCanon::Canonical(AffineCanonical {
        num_vars: reduced_nv,
        clauses: out,
        sub,
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/affine/tests.rs"]
mod tests;
