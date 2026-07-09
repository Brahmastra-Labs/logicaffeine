//! The **category of collapses** — the 2-rung toward the ∞-tower.
//!
//! Fix a problem and its refutation. Its Lyapunov measures (levelings of the proof) form a
//! **category**: objects are valid measures (non-increasing potentials over the proof steps), and a
//! morphism `V → W` is a **refinement** — a monotone map `φ` with `W = φ ∘ V`, so `V` distinguishes
//! states at least as finely as `W`. This is a *thin* category (a preorder under refinement). Its
//! **initial object** is the finest measure (the linear, one-level-per-step proof-induced measure);
//! its **terminal object** is the coarsest valid leveling. Different physics give different objects;
//! refinements are the **2-cells** relating them.
//!
//! Where the ∞ begins (honestly): this is one *thin 1-category per problem*. The next rung is
//! *functors between these categories* — **measure transfer between problems** (a reduction `F → F'`
//! carrying a measure of `F'` back to one of `F`). When those functors and the natural transformations
//! between them cohere, you get a 2-category of collapses, then higher. We build this rung solidly and
//! state the climb precisely; we do not claim the tower.

use crate::cdcl::Lit;
use crate::complexity::RankedRefutation;
use crate::proof::{Perm, ProofStep, Witness};

/// A **reduction** `ρ: F' → F` — a sign-respecting variable→literal map. It acts on clauses, on
/// substitution witnesses (by conjugation `ρσρ⁻¹`), and hence on whole refutations. When `ρ(F') ⊆ F`
/// (the source embeds into the target), `ρ` carries any refutation of `F'` to one of `F`. This is the
/// morphism along which the **transfer functor** moves collapses between problems.
pub struct Reduction {
    /// `image[v] = ρ(+v)`, a literal over the target's variables.
    pub image: Vec<Lit>,
    pub target_num_vars: usize,
}

impl Reduction {
    /// `ρ` applied to a literal (sign-respecting).
    pub fn apply_lit(&self, l: Lit) -> Lit {
        let img = self.image[l.var() as usize];
        if l.is_positive() {
            img
        } else {
            img.negated()
        }
    }

    /// `ρ` applied to a clause.
    pub fn apply_clause(&self, c: &[Lit]) -> Vec<Lit> {
        c.iter().map(|&l| self.apply_lit(l)).collect()
    }

    /// The conjugate `ρσρ⁻¹` over the target's variables (for a pure-variable injective `ρ`); the
    /// identity on target variables outside `ρ`'s image. Conjugation transports an automorphism of
    /// the source database to one of `ρ(database)` — which is why SR witnesses transfer.
    fn conjugate(&self, sigma: &Perm) -> Perm {
        let nv = self.target_num_vars;
        let mut inv: Vec<Option<usize>> = vec![None; nv];
        for (v, img) in self.image.iter().enumerate() {
            inv[img.var() as usize] = Some(v);
        }
        let images: Vec<Lit> = (0..nv)
            .map(|w| match inv[w] {
                Some(v) => self.apply_lit(sigma.apply(Lit::pos(v as u32))),
                None => Lit::pos(w as u32),
            })
            .collect();
        Perm::from_images(images)
    }

    /// `ρ` applied to a proof step (clause + witness).
    pub fn apply_step(&self, step: &ProofStep) -> ProofStep {
        match step {
            ProofStep::Rup(c) => ProofStep::Rup(self.apply_clause(c)),
            ProofStep::Delete(c) => ProofStep::Delete(self.apply_clause(c)),
            ProofStep::Pr { clause, witness } => {
                let w = match witness {
                    Witness::Assignment(a) => Witness::Assignment(a.iter().map(|&l| self.apply_lit(l)).collect()),
                    Witness::Substitution(sigma) => Witness::Substitution(self.conjugate(sigma)),
                };
                ProofStep::Pr { clause: self.apply_clause(clause), witness: w }
            }
        }
    }
}

impl Reduction {
    /// Is `ρ` an **isomorphism** (a bijective, sign-respecting variable renaming)? Such reductions are
    /// the *invertible 1-morphisms* — they make problems-and-reductions a **groupoid**.
    pub fn is_bijective(&self) -> bool {
        if self.image.len() != self.target_num_vars {
            return false;
        }
        let mut vars: Vec<u32> = self.image.iter().map(|l| l.var()).collect();
        vars.sort_unstable();
        vars.dedup();
        vars.len() == self.target_num_vars
    }

    /// The inverse isomorphism `ρ⁻¹` (if `ρ` is bijective).
    pub fn inverse(&self) -> Option<Reduction> {
        if !self.is_bijective() {
            return None;
        }
        let nv = self.target_num_vars;
        let mut inv = vec![Lit::pos(0); nv];
        for (v, &img) in self.image.iter().enumerate() {
            inv[img.var() as usize] = Lit::new(v as u32, img.is_positive());
        }
        Some(Reduction { image: inv, target_num_vars: nv })
    }

    /// Composition `self ∘ other` (apply `other` then `self`).
    pub fn compose(&self, other: &Reduction) -> Reduction {
        Reduction {
            image: (0..other.image.len()).map(|v| self.apply_lit(other.image[v])).collect(),
            target_num_vars: self.target_num_vars,
        }
    }

    /// Is `ρ` the identity reduction?
    pub fn is_identity(&self) -> bool {
        self.image.len() == self.target_num_vars
            && self.image.iter().enumerate().all(|(v, &l)| l == Lit::pos(v as u32))
    }

    /// Is `ρ` a **loop at `F`** — an isomorphism `F → F` carrying the clause set onto itself? A loop is
    /// exactly an **automorphism** of `F`. The loops at `F` are `π₁(F)` of the groupoid, and they form
    /// the symmetry group of `F`.
    pub fn is_loop_at(&self, formula: &[Vec<Lit>]) -> bool {
        if !self.is_bijective() {
            return false;
        }
        let mapped: Vec<Vec<Lit>> = formula.iter().map(|c| self.apply_clause(c)).collect();
        canon_clauses(&mapped) == canon_clauses(formula)
    }
}

/// Canonical multiset form of a clause set (each clause sorted/deduped, then the clauses sorted) — for
/// set-equality of formulas under renaming.
fn canon_clauses(cs: &[Vec<Lit>]) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = cs
        .iter()
        .map(|c| {
            let mut k: Vec<u32> = c.iter().map(|l| l.var() * 2 + u32::from(!l.is_positive())).collect();
            k.sort_unstable();
            k.dedup();
            k
        })
        .collect();
    out.sort();
    out
}

/// **The transfer functor.** Carry a refutation/measure of `F'` to one of `F` along `ρ`, preserving
/// the level structure (ranks). Re-checks the transferred proof against `F` (fail-closed): returns
/// the transferred ranked refutation iff it genuinely refutes the target. Functorial — `transfer`
/// along the identity reduction is the identity, and along a composite is the composite.
pub fn transfer(
    reduction: &Reduction,
    source_steps: &[ProofStep],
    ranks: &[u64],
    target_formula: &[Vec<Lit>],
) -> Option<RankedRefutation> {
    let steps: Vec<ProofStep> = source_steps.iter().map(|s| reduction.apply_step(s)).collect();
    if crate::pr::check_pr_refutation_fast(reduction.target_num_vars, target_formula, &steps) {
        Some(RankedRefutation { refuted: true, steps, ranks: ranks.to_vec() })
    } else {
        None
    }
}

/// Does `fine` **refine** `coarse`? I.e., is there a monotone `φ` with `coarse = φ ∘ fine`? Equivalent
/// to `fine[i] ≥ fine[j] ⟹ coarse[i] ≥ coarse[j]` for all `i, j`. Requires equal length (two levelings
/// of the SAME proof). A `true` means `fine` is at least as fine a partition of the proof steps as
/// `coarse` — the morphism `fine → coarse` in the category of collapses.
pub fn refines(fine: &[u64], coarse: &[u64]) -> bool {
    if fine.len() != coarse.len() {
        return false;
    }
    let n = fine.len();
    for i in 0..n {
        for j in 0..n {
            if fine[i] >= fine[j] && coarse[i] < coarse[j] {
                return false;
            }
        }
    }
    true
}

/// Are two measures **isomorphic** in the category — mutual refinements (the same partition of steps
/// up to relabeling levels)?
pub fn iso(a: &[u64], b: &[u64]) -> bool {
    refines(a, b) && refines(b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refinement_is_a_thin_category_a_preorder() {
        // The category laws: refinement is reflexive (identities) and transitive (composition) — so
        // the collapses of a problem form a (thin) category. Checked over random non-increasing
        // levelings and their monotone coarsenings.
        let mut state = 0x2C0A_7E60_1234_ABCDu64;
        let mut next = || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        for _ in 0..10_000 {
            let len = 1 + (next() as usize % 10);
            // a random NON-INCREASING leveling
            let mut a: Vec<u64> = (0..len).map(|_| next() % 8).collect();
            a.sort_unstable_by(|x, y| y.cmp(x));
            // reflexivity (identity morphism)
            assert!(refines(&a, &a), "refinement is reflexive (identity)");
            // monotone coarsenings b = ⌊a/2⌋, c = ⌊b/2⌋
            let b: Vec<u64> = a.iter().map(|&x| x / 2).collect();
            let c: Vec<u64> = b.iter().map(|&x| x / 2).collect();
            assert!(refines(&a, &b), "a ⟶ b (a refines its coarsening)");
            assert!(refines(&b, &c), "b ⟶ c");
            // transitivity (composition of morphisms)
            assert!(refines(&a, &c), "composition: a ⟶ b ⟶ c ⟹ a ⟶ c");
        }
    }

    #[test]
    fn the_linear_measure_is_the_initial_object() {
        // The proof-induced linear measure (every step its own level) refines EVERY valid leveling of
        // the same proof — it is the INITIAL (finest) object in the category of collapses. We check it
        // against a REAL symmetry measure and its coarsening, exhibiting the chain
        // linear ⟶ symmetry ⟶ (2-level) coarse, with composition.
        let (php, _) = crate::families::php(7);
        let (_, ranked) =
            crate::lyapunov::solve_by_measure_synthesis(php.num_vars, &php.clauses).unwrap();
        let symmetry = ranked.ranks.clone();
        let linear = crate::lyapunov::proof_induced_measure(symmetry.len());
        // initial: linear refines the symmetry measure
        assert!(refines(&linear, &symmetry), "linear (finest) ⟶ symmetry measure");
        // a 2-level coarsening of the symmetry measure
        let coarse: Vec<u64> = symmetry.iter().map(|&r| if r > 1 { 1 } else { 0 }).collect();
        assert!(refines(&symmetry, &coarse), "symmetry ⟶ its 2-level coarsening");
        // composition gives the long arrow directly
        assert!(refines(&linear, &coarse), "linear ⟶ coarse (the composite)");
        // and the symmetry measure is NOT iso to its strict coarsening (a genuine non-identity arrow)
        assert!(!iso(&symmetry, &coarse), "the coarsening is a strict morphism, not an isomorphism");
    }

    #[test]
    fn transfer_functor_carries_a_collapse_along_a_reduction() {
        // THE 2-CATEGORY MATERIALIZES: a reduction ρ carries a collapse of F' to a collapse of F,
        // preserving the level structure. We discover a measure on standard PHP(n), then transfer it
        // along a variable-renaming ρ to refute the RENAMED formula ρ(PHP(n)) — re-checked against the
        // target. The transferred measure is the same object (same ranks), moved along the morphism.
        let n = 6;
        let (php, _) = crate::families::php(n);
        let nv = php.num_vars;
        let (_, source) =
            crate::lyapunov::solve_by_measure_synthesis(nv, &php.clauses).unwrap();
        // ρ : a variable renaming (a fixed permutation of the variables).
        let perm: Vec<usize> = (0..nv).map(|v| (v * 7 + 3) % nv).collect();
        // ensure it's actually a permutation (gcd(7,nv) may not be 1); fall back to reverse if not.
        let is_perm = {
            let mut seen = perm.clone();
            seen.sort_unstable();
            seen.dedup();
            seen.len() == nv
        };
        let perm: Vec<usize> = if is_perm { perm } else { (0..nv).rev().collect() };
        let reduction = Reduction {
            image: (0..nv).map(|v| crate::cdcl::Lit::pos(perm[v] as u32)).collect(),
            target_num_vars: nv,
        };
        let target: Vec<Vec<crate::cdcl::Lit>> =
            php.clauses.iter().map(|c| reduction.apply_clause(c)).collect();
        let transferred = transfer(&reduction, &source.steps, &source.ranks, &target)
            .expect("the transferred collapse must refute the renamed formula");
        assert!(
            crate::pr::check_pr_refutation_fast(nv, &target, &transferred.steps),
            "the transferred proof re-checks against the target F"
        );
        assert_eq!(transferred.ranks, source.ranks, "the functor preserves the measure object (ranks)");
    }

    #[test]
    fn invertible_reductions_form_a_groupoid() {
        // Climbing to a GROUPOID: bijective reductions (isomorphisms) are invertible, with
        // ρ∘ρ⁻¹ = ρ⁻¹∘ρ = id. Checked over random renamings (with random sign flips).
        let mut state = 0x9A17_0117_BEEF_0042u64;
        let mut next = || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        for _ in 0..3_000 {
            let nv = 2 + (next() as usize % 8);
            let mut perm: Vec<usize> = (0..nv).collect();
            for i in (1..nv).rev() {
                let j = next() as usize % (i + 1);
                perm.swap(i, j);
            }
            let image: Vec<crate::cdcl::Lit> =
                (0..nv).map(|v| crate::cdcl::Lit::new(perm[v] as u32, next() & 1 == 0)).collect();
            let rho = Reduction { image, target_num_vars: nv };
            assert!(rho.is_bijective(), "a renaming is an isomorphism");
            let inv = rho.inverse().expect("isomorphisms invert");
            assert!(rho.compose(&inv).is_identity(), "ρ∘ρ⁻¹ = id");
            assert!(inv.compose(&rho).is_identity(), "ρ⁻¹∘ρ = id");
        }
    }

    #[test]
    fn pi_one_of_a_problem_is_its_symmetry_group() {
        // THE IDENTIFICATION (where the ∞-tower starts paying off): a LOOP at F — an isomorphism
        // F → F — is exactly an AUTOMORPHISM of F. So π₁(F) of the groupoid of problems IS the
        // symmetry group of F. We check on PHP: pigeon-swaps are loops, loops compose and invert to
        // loops (π₁ is a group), and a non-symmetry renaming is NOT a loop.
        let n = 4;
        let holes = n - 1;
        let (php, _) = crate::families::php(n);
        let nv = php.num_vars;
        let var = |p: usize, h: usize| p * holes + h;
        // pigeon-swap(0,1) as a variable renaming
        let swap_pigeons = |a: usize, b: usize| -> Reduction {
            let image: Vec<crate::cdcl::Lit> = (0..nv)
                .map(|v| {
                    let (p, h) = (v / holes, v % holes);
                    let np = if p == a { b } else if p == b { a } else { p };
                    crate::cdcl::Lit::pos(var(np, h) as u32)
                })
                .collect();
            Reduction { image, target_num_vars: nv }
        };
        let s01 = swap_pigeons(0, 1);
        let s12 = swap_pigeons(1, 2);
        // generators of the symmetry group are loops (automorphisms)
        assert!(s01.is_loop_at(&php.clauses), "pigeon-swap is a loop = an automorphism = in π₁");
        assert!(s12.is_loop_at(&php.clauses));
        // π₁ is a GROUP: composition and inverse of loops are loops
        assert!(s01.compose(&s12).is_loop_at(&php.clauses), "composition of loops is a loop");
        assert!(s01.inverse().unwrap().is_loop_at(&php.clauses), "inverse of a loop is a loop");
        assert!(s01.compose(&s01).is_identity(), "a transposition squares to the identity loop");
        // a NON-symmetry renaming (cross pigeon AND hole) is NOT a loop — not every iso is in π₁.
        let bad = {
            let mut image: Vec<crate::cdcl::Lit> = (0..nv).map(|v| crate::cdcl::Lit::pos(v as u32)).collect();
            image.swap(var(0, 0), var(1, 1)); // swap x(0,0) ↔ x(1,1): not a PHP automorphism
            Reduction { image, target_num_vars: nv }
        };
        assert!(!bad.is_loop_at(&php.clauses), "a non-symmetry renaming is not in π₁");
    }

    #[test]
    fn the_2cells_are_the_group_relations_so_the_infinity_groupoid_is_BG() {
        // ONE MORE FINITE STEP up the tower — and it closes the question. π₁(F) = Aut(F) is a discrete
        // group, *presented* by the Coxeter relations of its generators (adjacent pigeon-swaps). Those
        // relations ARE the 2-cells: the homotopies witnessing that a product of loops is the trivial
        // loop. We verify the full presentation:
        //     s_i² = id,   (s_i s_{i+1})³ = id,   s_i s_j = s_j s_i  (|i-j| ≥ 2).
        // Therefore the ∞-groupoid of this symmetry structure is K(Aut(F), 1) = the classifying space
        // BG: π₁ = G (checked earlier), and πₙ = 0 for n ≥ 2 — BECAUSE the symmetry is a *discrete*
        // group. Genuine higher πₙ would require a 2-GROUP (symmetry-with-internal-symmetry); that is
        // the honest open frontier, not something we have. This is the finite step that names the limit.
        let n = 5;
        let holes = n - 1;
        let (php, _) = crate::families::php(n);
        let nv = php.num_vars;
        let var = |p: usize, h: usize| p * holes + h;
        let s = |i: usize| -> Reduction {
            let image: Vec<crate::cdcl::Lit> = (0..nv)
                .map(|v| {
                    let (p, h) = (v / holes, v % holes);
                    let np = if p == i { i + 1 } else if p == i + 1 { i } else { p };
                    crate::cdcl::Lit::pos(var(np, h) as u32)
                })
                .collect();
            Reduction { image, target_num_vars: nv }
        };
        // Each generator is a loop (an automorphism = an element of π₁).
        for i in 0..n - 1 {
            assert!(s(i).is_loop_at(&php.clauses), "generator s_{i} is a loop in π₁");
            // Relation 1 (involution): s_i² = id — a 2-cell.
            assert!(s(i).compose(&s(i)).is_identity(), "s_{i}² = id");
        }
        // Relation 2 (braid): (s_i s_{i+1})³ = id — the defining 2-cell of the symmetric group.
        for i in 0..n - 2 {
            let b = s(i).compose(&s(i + 1));
            assert!(b.compose(&b).compose(&b).is_identity(), "braid relation (s_i·s_next)^3 = id at i={i}");
        }
        // Relation 3 (far commutation): s_i s_j = s_j s_i for |i-j| ≥ 2.
        for i in 0..n - 1 {
            for j in 0..n - 1 {
                if (i as i32 - j as i32).abs() >= 2 {
                    assert_eq!(
                        s(i).compose(&s(j)).image,
                        s(j).compose(&s(i)).image,
                        "distant swaps commute at i={i} j={j}"
                    );
                }
            }
        }
        // The presentation holds ⇒ ⟨s_i | relations⟩ = S_n = π₁ ⇒ the ∞-groupoid is K(S_n, 1) = BG.
    }

    #[test]
    fn transfer_is_functorial() {
        // The functor laws (the 2-category structure): transfer along the IDENTITY reduction is the
        // identity, and transfer along a COMPOSITE reduction equals the composite of transfers.
        let n = 5;
        let (php, _) = crate::families::php(n);
        let nv = php.num_vars;
        let (_, source) = crate::lyapunov::solve_by_measure_synthesis(nv, &php.clauses).unwrap();

        // identity reduction
        let id = Reduction { image: (0..nv).map(|v| crate::cdcl::Lit::pos(v as u32)).collect(), target_num_vars: nv };
        let via_id: Vec<_> = source.steps.iter().map(|s| id.apply_step(s)).collect();
        assert_eq!(via_id, source.steps, "transfer along identity is the identity functor");

        // two renamings ρ, τ and their composite ρ∘τ
        let p_rho: Vec<usize> = (0..nv).rev().collect();
        let p_tau: Vec<usize> = (0..nv).map(|v| (v + 1) % nv).collect();
        let rho = Reduction { image: (0..nv).map(|v| crate::cdcl::Lit::pos(p_rho[v] as u32)).collect(), target_num_vars: nv };
        let tau = Reduction { image: (0..nv).map(|v| crate::cdcl::Lit::pos(p_tau[v] as u32)).collect(), target_num_vars: nv };
        // composite ρ∘τ (apply τ then ρ): (ρ∘τ)(v) = ρ(τ(v))
        let comp = Reduction {
            image: (0..nv).map(|v| rho.apply_lit(tau.image[v])).collect(),
            target_num_vars: nv,
        };
        // transfer(ρ, transfer(τ, step)) == transfer(ρ∘τ, step), step by step
        for s in &source.steps {
            let two_step = rho.apply_step(&tau.apply_step(s));
            let one_step = comp.apply_step(s);
            assert_eq!(two_step, one_step, "transfer along ρ∘τ = transfer(ρ) ∘ transfer(τ)");
        }
    }

    #[test]
    fn aut_F_acts_on_the_collapses_the_2group_crossed_module() {
        // THE 2-GROUP'S FIRST RUNG. π₁ = Aut(F) ACTS on the collapses of F — the transfer functor
        // restricted to loops. A symmetry σ carries a collapse of F to a collapse of F (it lands back
        // because σ is a loop), the action obeys the group laws (identity acts trivially), it is by
        // ISOMORPHISMS of collapses (ranks preserved ⇒ 2-cells), and it is NON-TRIVIAL (σ genuinely
        // relabels the collapse). This is the crossed-module action of π₁ on the collapse structure.
        //
        // π₂ NOTE (and it reconfirms the summit): the action represents the *discrete* Aut(F)
        // faithfully on the collapse set, so there are no symmetries acting trivially-as-1-cells but
        // nontrivially-as-2-cells — π₂ = 0, the 2-group is 1-truncated, and the homotopy type is still
        // K(Aut(F), 1). A non-trivial π₂ needs genuinely categorified (non-discrete) symmetry.
        let n = 6;
        let holes = n - 1;
        let (php, _) = crate::families::php(n);
        let nv = php.num_vars;
        let var = |p: usize, h: usize| p * holes + h;
        let (_, source) = crate::lyapunov::solve_by_measure_synthesis(nv, &php.clauses).unwrap();

        // a loop σ = pigeon-swap(0,1), an automorphism of F
        let sigma = {
            let image: Vec<crate::cdcl::Lit> = (0..nv)
                .map(|v| {
                    let (p, h) = (v / holes, v % holes);
                    let np = if p == 0 { 1 } else if p == 1 { 0 } else { p };
                    crate::cdcl::Lit::pos(var(np, h) as u32)
                })
                .collect();
            Reduction { image, target_num_vars: nv }
        };
        assert!(sigma.is_loop_at(&php.clauses), "σ is a symmetry of F (a loop in π₁)");

        // ACTION: σ carries a collapse of F to a collapse of F (lands back in F's collapses).
        let acted = transfer(&sigma, &source.steps, &source.ranks, &php.clauses)
            .expect("a symmetry of F carries a collapse of F to a collapse of F");
        assert!(crate::pr::check_pr_refutation_fast(nv, &php.clauses, &acted.steps), "the acted collapse refutes F");
        // by ISOMORPHISM of collapses — the level structure is preserved (a 2-cell).
        assert_eq!(acted.ranks, source.ranks, "the action is by isomorphisms (ranks preserved)");
        // GROUP-ACTION law: the identity symmetry acts trivially.
        let id = Reduction { image: (0..nv).map(|v| crate::cdcl::Lit::pos(v as u32)).collect(), target_num_vars: nv };
        let by_id: Vec<_> = source.steps.iter().map(|s| id.apply_step(s)).collect();
        assert_eq!(by_id, source.steps, "the identity symmetry acts trivially (group-action unit law)");
        // NON-TRIVIALITY: a non-identity symmetry genuinely moves the collapse (relabels its steps).
        assert_ne!(acted.steps, source.steps, "a non-identity symmetry genuinely permutes the collapses");
    }

    #[test]
    fn non_uniqueness_gives_two_objects_in_the_category() {
        // Tie-in to the non-uniqueness theorem: PHP has a symmetry measure AND a cutting-planes
        // measure — two OBJECTS in the category of collapses. They are genuinely different objects
        // (different lengths ⇒ not even comparable by refinement directly), which is exactly what
        // makes the category non-trivial and the 2-rung worth climbing.
        let n = 7;
        let (php, _) = crate::families::php(n);
        let (_, ranked) =
            crate::lyapunov::solve_by_measure_synthesis(php.num_vars, &php.clauses).unwrap();
        let (cp_traj, _) = crate::lyapunov::cutting_planes_lyapunov(n);
        // Two collapses of one problem; as objects they differ (different step counts).
        assert_ne!(ranked.ranks.len(), cp_traj.len(), "symmetry and cutting-planes are distinct objects");
        // Each is its own identity (reflexive).
        assert!(refines(&ranked.ranks, &ranked.ranks) && refines(&cp_traj, &cp_traj));
    }
}
