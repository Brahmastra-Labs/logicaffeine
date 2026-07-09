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
        Affine { n, matrix: (0..n).map(|i| 1u64 << i).collect(), translation: 0 }
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
        Affine { n: self.n, matrix, translation }
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
    assert!(n <= 4, "exhaustive AGL enumeration is for ground-truth tests only (n ≤ 4)");
    let mut out = Vec::new();
    let total_matrices = 1u64 << (n * n);
    for code in 0..total_matrices {
        let matrix: Vec<u64> = (0..n).map(|i| (code >> (i * n)) & ((1 << n) - 1)).collect();
        if !gf2::is_invertible_gf2(n as u32, &matrix) {
            continue;
        }
        for translation in 0..(1u64 << n) {
            out.push(Affine { n, matrix: matrix.clone(), translation });
        }
    }
    out
}

/// The satisfying assignments of a CNF, each packed into the low `num_vars` bits of a `u64`. Brute force
/// over `2^{num_vars}` — small `n` only (the census / symmetry-measurement regime).
pub fn models_of(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<u64> {
    assert!(num_vars <= 24, "model enumeration is brute force — small n only");
    (0u64..(1u64 << num_vars))
        .filter(|&x| {
            clauses.iter().all(|c| {
                c.iter().any(|l| ((x >> l.var()) & 1 == 1) == l.is_positive())
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
pub fn recover_linear_system(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<(Vec<u64>, Vec<bool>)> {
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
        crate::xorsat::XorOutcome::Unsat(refutation) => crate::xor_drat::emit_xor_drat(&eqs, &refutation),
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
    let mut groups: std::collections::HashMap<Vec<bool>, Vec<usize>> = std::collections::HashMap::new();
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
    if !sub.iter().any(|s| matches!(s, VarSub::Const(_) | VarSub::Alias(_, _))) {
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
                Some(_) => continue,                           // duplicate literal
                None => {
                    seen.insert(ni, pol);
                    lits.push(Lit::new(ni, pol));
                }
            }
        }
        out.push(lits); // a now-empty clause is a sound UNSAT marker the solver will catch
    }

    AffineCanon::Canonical(AffineCanonical { num_vars: reduced_nv, clauses: out, sub })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `|AGL(n,2)| = 2ⁿ·|GL(n,2)|`, and the exhaustive enumeration produces exactly that many bijections.
    #[test]
    fn agl_order_matches_the_enumeration() {
        for n in 1..=4u32 {
            assert_eq!(
                all_affine_bijections(n as usize).len() as u128,
                agl_order(n),
                "n={n}: enumerated affine bijections must equal |AGL(n,2)| = 2ⁿ·|GL(n,2)|"
            );
        }
        // Pinned small values: |AGL(1,2)|=2, |AGL(2,2)|=24, |AGL(3,2)|=1344.
        assert_eq!(agl_order(1), 2);
        assert_eq!(agl_order(2), 24);
        assert_eq!(agl_order(3), 1344);
    }

    /// The group laws hold: composition is associative-ish (closure + identity) and every bijection's
    /// linear part is invertible. Spot-checked against direct point evaluation.
    #[test]
    fn affine_maps_compose_and_act_correctly() {
        let id = Affine::identity(3);
        for phi in all_affine_bijections(3) {
            assert!(phi.is_bijection());
            // identity is a left/right unit
            assert_eq!(phi.compose(&id), phi);
            assert_eq!(id.compose(&phi), phi);
            // composition matches pointwise application: (φ∘φ)(x) = φ(φ(x))
            let sq = phi.compose(&phi);
            for x in 0..8u64 {
                assert_eq!(sq.apply(x), phi.apply(phi.apply(x)), "composition must match double application");
            }
        }
    }

    /// **THE CENSUS GAP, MEASURED: `AGL ⊋ Bₙ`.** The single even-parity constraint `x₀⊕x₁⊕x₂ = 0` is
    /// rigid under the clause breakers but drowning in affine symmetry. Exactly `24·4 = 96` affine maps
    /// fix its model set (24 linear maps preserve the parity hyperplane, 4 translations stay inside it) —
    /// strictly more than the *entire* hyperoctahedral group `|B₃| = 2³·3! = 48` that bounds any
    /// permutation-automorphism the clause breakers could ever find. The witness is a concrete **shear**
    /// `x₁↦x₀⊕x₁, x₂↦x₀⊕x₂` (every column odd-weight, so parity-preserving): an automorphism of the model
    /// set that is *not* a signed permutation — exactly the kind no clause-level break can reach.
    #[test]
    fn affine_symmetry_strictly_exceeds_permutation_symmetry_on_parity() {
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        // Wrong-parity (odd) assignments of (x0,x1,x2): 100,010,001,111 — one clause forbidding each.
        let clauses = vec![
            vec![q(0), p(1), p(2)], // not (1,0,0)
            vec![p(0), q(1), p(2)], // not (0,1,0)
            vec![p(0), p(1), q(2)], // not (0,0,1)
            vec![q(0), q(1), q(2)], // not (1,1,1)
        ];

        let affine = affine_symmetries(3, &clauses);
        // 24 linear maps preserve the hyperplane × 4 in-plane translations.
        assert_eq!(affine.len(), 96, "the even-parity plane has exactly 96 affine symmetries");
        // … which strictly exceeds all of B₃ (48), hence any permutation symmetry the clause breakers find.
        const B3: usize = 8 * 6;
        assert!(affine.len() > B3, "AGL symmetry ({}) must exceed |B₃| = {B3}", affine.len());

        // The witness shear (rows x0, x0⊕x1, x0⊕x2 — every column odd-weight ⇒ parity-preserving) IS an
        // affine symmetry, and it mixes variables, so no signed permutation can express it.
        let shear = Affine { n: 3, matrix: vec![0b001, 0b011, 0b101], translation: 0 };
        assert!(shear.is_bijection());
        assert!(affine.contains(&shear), "the shear x₁↦x₀⊕x₁, x₂↦x₀⊕x₂ must be an affine symmetry of the plane");
        assert!(
            shear.matrix.iter().any(|r| r.count_ones() >= 2),
            "the shear mixes variables — outside Bₙ, invisible to every clause-level break"
        );
    }

    /// **The AGL lens-narrowness, quantified for every `n` (a theorem, not a measurement).** The even-parity
    /// family over `n` variables has closed-form symmetry-group orders under the two lenses:
    ///   - `|Bₙ-symmetry| = n! · 2^{n−1}` (all variable permutations, plus even sign-flips), and
    ///   - `|AGL-symmetry| = |GL(n,2)| / (2ⁿ−1) · 2^{n−1}` (linear maps fixing the parity hyperplane, plus
    ///     in-plane translations).
    /// Their ratio `|GL(n,2)| / ((2ⁿ−1)·n!)` grows *super-exponentially* (`|GL(n,2)| = 2^{Θ(n²)}`), so the
    /// clause-symmetry lens `Bₙ` is provably blind to almost all the symmetry — which is exactly why parity /
    /// affine families defeat clause-based symmetry breaking and need linear algebra. We pin the closed forms
    /// against the exhaustive count at `n = 3, 4`, then exhibit the unbounded growth via the formula.
    #[test]
    fn affine_symmetry_dwarfs_permutation_symmetry_on_parity_at_every_scale() {
        fn factorial(n: u128) -> u128 {
            (1..=n).product()
        }
        let bn_stab = |n: u32| -> u128 { factorial(n as u128) * (1u128 << (n - 1)) };
        let agl_stab = |n: u32| -> u128 { gf2::gl_order(n) / ((1u128 << n) - 1) * (1u128 << (n - 1)) };

        // Pin BOTH closed forms against exhaustive counts (n ≤ 4). The even-parity constraint is encoded by
        // forbidding every ODD-parity assignment with a full-width clause; its automorphisms are the
        // hyperplane-preserving affine maps, and its `Bₙ` sub-automorphisms are those with permutation-matrix
        // linear part.
        for n in 3..=4u32 {
            let clauses: Vec<Vec<Lit>> = (0u64..(1u64 << n))
                .filter(|a| a.count_ones() % 2 == 1)
                .map(|a| (0..n).map(|v| Lit::new(v, (a >> v) & 1 == 0)).collect())
                .collect();
            let syms = affine_symmetries(n as usize, &clauses);
            // A signed permutation = an affine map whose linear part is a permutation matrix.
            let is_perm = |a: &Affine| -> bool {
                let mut cols = 0u64;
                for &row in a.matrix.iter().take(n as usize) {
                    if row.count_ones() != 1 || cols & row != 0 {
                        return false;
                    }
                    cols |= row;
                }
                cols == (1u64 << n) - 1
            };
            let bn_count = syms.iter().filter(|a| is_perm(a)).count() as u128;
            assert_eq!(syms.len() as u128, agl_stab(n), "n={n}: AGL parity-stabilizer = |GL(n,2)|/(2ⁿ−1)·2^{{n−1}}");
            assert_eq!(bn_count, bn_stab(n), "n={n}: Bₙ parity-stabilizer = n!·2^{{n−1}}");
            assert!(agl_stab(n) > bn_stab(n), "n={n}: AGL symmetry exceeds Bₙ symmetry");
        }

        // The ratio grows without bound — the standard lens misses a super-exponential factor of the symmetry.
        let ratios: Vec<u128> = (3..=8u32).map(|n| agl_stab(n) / bn_stab(n)).collect();
        eprintln!("AGL/Bₙ parity-symmetry ratio, n = 3..8: {ratios:?}");
        assert!(ratios.windows(2).all(|w| w[1] > w[0]), "the AGL/Bₙ ratio grows with n: {ratios:?}");
        assert!(*ratios.last().unwrap() > 1_000_000, "by n=8 the lens misses a >10⁶ factor");
    }

    /// **The scalable affine-symmetry detector for XOR families: a closed form, verified `∀n`.** Enumerating
    /// `AGL(n,2)` is stuck at `n ≤ 4`, but the affine automorphism group of any XOR-defined family (model set a
    /// `k`-dim affine subspace) has the *closed form* [`affine_subspace_agl_order`], evaluable at any `n`. We
    /// pin it against the exhaustive count at `n ≤ 4` for every `1 ≤ k ≤ n−1`, then push the consequence to
    /// `n = 10`: for any nontrivial affine family the affine symmetry is `2^{Θ(n²)}`, which **dwarfs the entire
    /// hyperoctahedral group** `|Bₙ| = 2ⁿ·n! = 2^{O(n log n)}`. So clause-based symmetry breaking captures a
    /// vanishing `2^{−Θ(n²)}` fraction of the symmetry of a linear family — the linear structure is invisible
    /// to it, ∀n, and the finder scales without enumerating the group.
    #[test]
    fn affine_family_symmetry_closed_form_scales_to_all_n() {
        // Pin the closed form against exhaustive counts (n ≤ 4), every dimension. The model set of the units
        // `{¬x_j : k ≤ j < n}` is the k-dim coordinate subspace.
        for n in 2..=4u32 {
            for k in 1..n {
                let clauses: Vec<Vec<Lit>> = (k..n).map(|j| vec![Lit::neg(j)]).collect();
                assert_eq!(
                    affine_symmetries(n as usize, &clauses).len() as u128,
                    affine_subspace_agl_order(n, k),
                    "n={n} k={k}: closed-form affine-subspace order must match the exhaustive count"
                );
            }
        }
        // Push to n = 12: the affine symmetry of a densest (k = n/2) linear family dwarfs all of Bₙ, and the
        // ratio grows super-exponentially (2^{Θ(n²)} / 2^{O(n log n)}).
        let bn = |n: u128| (1u128 << n) * (1..=n).product::<u128>();
        let ratios: Vec<u128> = (6..=12u32).map(|n| affine_subspace_agl_order(n, n / 2) / bn(n as u128)).collect();
        eprintln!("affine-family AGL / |Bₙ| ratio, n = 6..12: {ratios:?}");
        assert!(ratios.iter().all(|&r| r > 1), "the affine symmetry exceeds |Bₙ| at every n");
        assert!(ratios.windows(2).all(|w| w[1] > w[0]), "the gap grows with n: {ratios:?}");
        assert!(*ratios.last().unwrap() > 1_000_000_000, "by n=12 the affine symmetry is a >10⁹ factor beyond |Bₙ|");
    }

    /// The affine break **refutes** an inconsistent linear core — a transitive XOR contradiction
    /// (`x₀=x₁, x₁=x₂, x₀≠x₂`) that is parity-inconsistent but has no two clauses in direct conflict.
    #[test]
    fn affine_reduce_refutes_an_inconsistent_linear_core() {
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        let clauses = vec![
            vec![q(0), p(1)], vec![p(0), q(1)], // x0 = x1
            vec![q(1), p(2)], vec![p(1), q(2)], // x1 = x2
            vec![p(0), p(2)], vec![q(0), q(2)], // x0 ≠ x2  ⇒ 0 = 1
        ];
        // Refuted, and the payload is a DRAT certificate that RUP-refutes the original CNF.
        match affine_reduce(3, &clauses) {
            AffineOutcome::Refuted(Some(drat)) => assert!(
                crate::rup::check_refutation(3, &clauses, &drat),
                "the affine refutation's xor_drat certificate must RUP-refute the original CNF"
            ),
            other => panic!("expected a certified refutation, got {other:?}"),
        }
        // Ground truth: it really is UNSAT.
        assert!(models_of(3, &clauses).is_empty(), "the linear core is genuinely unsatisfiable");
    }

    /// **The affine refutation is certified through the `xor_drat` bridge.** On a genuine parity core
    /// (an odd-charge expander Tseitin), [`affine_refutation_drat`] compiles the GF(2) linear dependency to
    /// RUP resolvent lemmas that our independent checker accepts against the original CNF — the same
    /// `drat-trim`-checkable path `Route::Parity` uses — and the dispatcher now carries that proof on its
    /// verdict rather than reporting UNSAT bare.
    #[test]
    fn affine_refutation_is_certified_via_xor_drat_bridge() {
        use crate::solve::{solve_structured, Answer};
        let (_, cnf, _) = crate::families::tseitin_expander(6, 1);
        let nv = cnf.num_vars;
        let drat = affine_refutation_drat(nv, &cnf.clauses).expect("an inconsistent XOR core has a certificate");
        assert!(!drat.is_empty(), "the certificate must carry resolvent lemmas");
        assert!(
            crate::rup::check_refutation(nv, &cnf.clauses, &drat),
            "the xor_drat certificate must RUP-refute the original CNF"
        );
        // Wired: the dispatcher decides UNSAT (and where the affine rung fires, ships this very proof).
        assert!(matches!(solve_structured(nv, &cnf.clauses).answer, Answer::Unsat), "the parity core is UNSAT");
    }

    /// The affine break **derives a forced unit** no clause states: `x0⊕x1=1 ∧ x0⊕x1⊕x2=0 ⇒ x2=1`. Gauss
    /// elimination finds it; unit propagation on the clauses alone does not.
    #[test]
    fn affine_reduce_derives_a_nonsyntactic_forced_unit() {
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        // x0⊕x1=1: (x0∨x1)∧(¬x0∨¬x1).  x0⊕x1⊕x2=0: the four even-parity clauses over {0,1,2}.
        let clauses = vec![
            vec![p(0), p(1)], vec![q(0), q(1)],
            vec![q(0), p(1), p(2)], vec![p(0), q(1), p(2)], vec![p(0), p(1), q(2)], vec![q(0), q(1), q(2)],
        ];
        match affine_reduce(3, &clauses) {
            AffineOutcome::Forced(extra) => {
                assert!(extra.contains(&vec![Lit::pos(2)]), "must derive the forced unit x2 = 1; got {extra:?}");
            }
            other => panic!("expected forced consequences, got {other:?}"),
        }
        // The derived unit is sound: x2 is 1 in every model.
        for m in models_of(3, &clauses) {
            assert_eq!((m >> 2) & 1, 1, "x2 must be 1 in every model");
        }
    }

    /// Wired: the dispatcher decides affine-reducible formulas correctly — it refutes an inconsistent
    /// linear core and returns a satisfying model for a forced-consequence formula (the model re-checked
    /// against the original clauses).
    #[test]
    fn the_dispatcher_decides_affine_reducible_formulas() {
        use crate::solve::{solve_structured, Answer};
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        // Transitive XOR contradiction (x0=x1, x1=x2, x0≠x2) — UNSAT, no two clauses in direct conflict.
        let unsat = vec![
            vec![q(0), p(1)], vec![p(0), q(1)], vec![q(1), p(2)], vec![p(1), q(2)], vec![p(0), p(2)], vec![q(0), q(2)],
        ];
        assert!(matches!(solve_structured(3, &unsat).answer, Answer::Unsat), "dispatcher must refute the linear core");
        // Forced-consequence SAT case: x0⊕x1=1 ∧ x0⊕x1⊕x2=0 forces x2=1, still satisfiable.
        let sat = vec![
            vec![p(0), p(1)], vec![q(0), q(1)],
            vec![q(0), p(1), p(2)], vec![p(0), q(1), p(2)], vec![p(0), p(1), q(2)], vec![q(0), q(1), q(2)],
        ];
        match solve_structured(3, &sat).answer {
            Answer::Sat(m) => assert!(
                sat.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the returned model must satisfy the original formula"
            ),
            Answer::Unsat => panic!("the forced-consequence formula is satisfiable"),
        }
    }

    /// Fail-closed: a formula with no inconsistent or constraining linear structure is left `Unchanged`,
    /// and a satisfiable parity plane is never falsely `Refuted`.
    #[test]
    fn affine_reduce_is_sound_and_quiet_when_there_is_nothing_to_do() {
        // Pure 2-CNF with no recoverable XOR bundles.
        let p = |v: u32| Lit::pos(v);
        let plain = vec![vec![p(0), p(1), p(2)]];
        assert_eq!(affine_reduce(3, &plain), AffineOutcome::Unchanged);
        // A satisfiable parity constraint: consistent, so never Refuted (Forced or Unchanged only).
        let q = |v: u32| Lit::neg(v);
        let sat_parity = vec![
            vec![q(0), p(1), p(2)], vec![p(0), q(1), p(2)], vec![p(0), p(1), q(2)], vec![q(0), q(1), q(2)],
        ];
        assert!(!matches!(affine_reduce(3, &sat_parity), AffineOutcome::Refuted(_)), "a satisfiable plane must not be refuted");
    }

    /// **The canonical RREF break collapses an equivalence chain.** `x0=x1=x2=x3` plus a forced unit
    /// `x4=0` and a clause `x0∨x4`: the four-variable equivalence class folds to one representative and
    /// `x4` is eliminated, so the formula reduces from 5 variables to 1 — and the reduced model lifts back
    /// to a genuine model of the original.
    #[test]
    fn affine_canonicalize_collapses_an_equivalence_chain() {
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        let clauses = vec![
            vec![q(0), p(1)], vec![p(0), q(1)], // x0 = x1
            vec![q(1), p(2)], vec![p(1), q(2)], // x1 = x2
            vec![q(2), p(3)], vec![p(2), q(3)], // x2 = x3
            vec![p(0), p(4)],                   // x0 ∨ x4
            vec![q(4)],                         // x4 = 0
        ];
        match affine_canonicalize(5, &clauses) {
            AffineCanon::Canonical(canon) => {
                assert!(canon.num_vars < 5, "the chain + unit must shrink 5 vars (got {})", canon.num_vars);
                let orig = models_of(5, &clauses);
                let red = models_of(canon.num_vars, &canon.clauses);
                assert_eq!(red.is_empty(), orig.is_empty(), "reduction must preserve satisfiability");
                for &rm_bits in &red {
                    let rm: Vec<bool> = (0..canon.num_vars).map(|i| (rm_bits >> i) & 1 == 1).collect();
                    let lifted = canon.lift(&rm);
                    assert!(
                        clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())),
                        "the lifted reduced model must satisfy the original formula"
                    );
                }
            }
            AffineCanon::Refuted(_) => panic!("the chain is satisfiable, not refuted"),
            AffineCanon::Unchanged => panic!("the chain + unit must reduce"),
        }
    }

    /// **Soundness to the point of absurdity.** Hundreds of random formulas with injected linear structure
    /// (equivalences, units, and clausal noise): the canonical break must preserve satisfiability EXACTLY
    /// against brute force, refute only genuinely-UNSAT instances, and lift every reduced model to a real
    /// model of the original.
    #[test]
    fn affine_canonicalize_is_sound_against_brute_force() {
        let mut state = 0xA5F1_C0DE_1234_5678u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..400 {
            let n = 4 + (rng() % 8) as usize; // 4..=11 variables
            let mut clauses: Vec<Vec<Lit>> = Vec::new();
            for _ in 0..(rng() % 4) {
                let k = 2 + (rng() % 2) as usize;
                let c: Vec<Lit> = (0..k).map(|_| Lit::new((rng() % n as u64) as u32, rng() & 1 == 0)).collect();
                clauses.push(c);
            }
            for _ in 0..(1 + rng() % 3) {
                let a = (rng() % n as u64) as u32;
                let b = (rng() % n as u64) as u32;
                if rng() & 1 == 0 {
                    if a == b {
                        continue;
                    }
                    if rng() & 1 == 0 {
                        clauses.push(vec![Lit::neg(a), Lit::pos(b)]); // x_a = x_b
                        clauses.push(vec![Lit::pos(a), Lit::neg(b)]);
                    } else {
                        clauses.push(vec![Lit::pos(a), Lit::pos(b)]); // x_a ≠ x_b
                        clauses.push(vec![Lit::neg(a), Lit::neg(b)]);
                    }
                } else {
                    clauses.push(vec![Lit::new(a, rng() & 1 == 0)]); // a unit
                }
            }
            let orig = models_of(n, &clauses);
            match affine_canonicalize(n, &clauses) {
                AffineCanon::Refuted(_) => assert!(orig.is_empty(), "Refuted ⇒ the original must be UNSAT"),
                AffineCanon::Canonical(canon) => {
                    let red = models_of(canon.num_vars, &canon.clauses);
                    assert_eq!(red.is_empty(), orig.is_empty(), "canonicalization must preserve satisfiability");
                    for &rm_bits in &red {
                        let rm: Vec<bool> = (0..canon.num_vars).map(|i| (rm_bits >> i) & 1 == 1).collect();
                        let lifted = canon.lift(&rm);
                        assert!(
                            clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())),
                            "every lifted reduced model must satisfy the original"
                        );
                    }
                }
                AffineCanon::Unchanged => {}
            }
        }
    }
}
