//! The Boolean hypercube `{0,1}ⁿ` and its subcube cover — the geometric substrate beneath SAT,
//! and the executable form of the Lean formalization's `HypercubeSAT`.
//!
//! Every clause of a CNF is a **blocker**: the set of corners (vertices) of the hypercube that
//! *falsify* it. A clause of width `w` forbids exactly the `2^{n-w}` corners that set each of its
//! literals false — a subcube (a face) of codimension `w`. The whole formula is UNSAT precisely
//! when its blockers **cover** the hypercube: every one of the `2ⁿ` corners is forbidden by some
//! clause, so no satisfying assignment escapes. SAT ⟺ some corner has *energy zero* — covered by
//! no blocker. This is `Pnp.lean`'s `Blocker` / `vertexEnergy` / `CoverUNSAT`, made concrete.
//!
//! The point of the representation: the **problem** (solutions = uncovered corners) and the
//! **rules** (clauses = blockers) live in the *same* world `{0,1}ⁿ`. A clause is not a separate
//! syntactic object; it is a region of the very space the solutions inhabit. So one group action —
//! a [`CubeSym`], `Pnp.lean`'s `CubeSymmetry` of coordinate permutations and per-coordinate flips —
//! moves *both*: it permutes blockers among themselves and permutes corners among themselves, in
//! lockstep. That is what lets us symmetry-break the rules and the solutions with a single move, and
//! it is why the cover-totality question (UNSAT) collapses by the order of the symmetry group: a
//! cover is total iff it covers **one representative per orbit**, not all `2ⁿ` corners.
//!
//! Pigeonhole is the canonical instance because we can always build one ([`php_cover`]): `n` pigeons
//! into `n-1` holes, with the full row×column symmetry group `Sₙ × Sₙ₋₁` acting on the grid of
//! variables. Here we build the stage, answer the first question — *which corners are we blocking?* —
//! and then **measure** how far each stacked symmetry collapses the cover-check.

use crate::cdcl::Lit;
use crate::dimacs::DimacsCnf;
use crate::proof::Perm;
use std::collections::{BTreeSet, HashMap};

/// A corner (vertex) of the hypercube `{0,1}ⁿ`: bit `v` holds the value of variable `v`.
/// Enumeration routines assume `n ≤ 63`; the algebra itself is width-agnostic.
pub type Corner = u64;

/// A subcube of `{0,1}ⁿ`: the coordinates set in `care` are fixed to the matching bits of `value`;
/// the rest are free. As a **blocker** it is the footprint of one clause — the corners that falsify
/// it. (`Pnp.lean`'s `Blocker`, generalized from clean 3-bit faces to any width.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Subcube {
    pub n: usize,
    /// Bitmask of the fixed ("cared-about") coordinates — the blocker's support.
    pub care: u64,
    /// Required values on the fixed coordinates; bits outside `care` are held at 0.
    pub value: u64,
}

impl Subcube {
    /// The blocker of a clause: the subcube of corners that set *every* literal false. A positive
    /// literal `xᵥ` is false at `c[v]=0`; a negative literal `¬xᵥ` is false at `c[v]=1`. So `care`
    /// is the clause's variable set and `value`'s bit `v` is set exactly for the negative literals.
    pub fn blocker(clause: &[Lit], n: usize) -> Subcube {
        let mut care = 0u64;
        let mut value = 0u64;
        for &lit in clause {
            let v = lit.var() as u64;
            care |= 1u64 << v;
            if !lit.is_positive() {
                value |= 1u64 << v;
            }
        }
        Subcube { n, care, value }
    }

    /// Does this subcube contain `corner`? (`Pnp.lean`'s `Blocker.Covers` — is the clause falsified
    /// at this corner?)
    #[inline]
    pub fn covers(&self, corner: Corner) -> bool {
        (corner & self.care) == self.value
    }

    /// The number of free coordinates — the subcube's dimension as a face (`n - |support|`).
    pub fn dimension(&self) -> usize {
        self.n - self.care.count_ones() as usize
    }

    /// How many corners this blocker forbids: `2^dimension` (the footprint cardinality).
    pub fn footprint_card(&self) -> u64 {
        1u64 << self.dimension()
    }

    /// Recover the clause this blocker is the footprint of — the inverse of [`Subcube::blocker`].
    /// A fixed coordinate `v` appears *positively* when `value`'s bit is 0 (the clause is falsified
    /// at `v=0`) and *negatively* when it is 1. Round-tripping a clause through `blocker` and back is
    /// the proof that the geometric representation loses nothing.
    pub fn clause_literals(&self) -> Vec<(usize, bool)> {
        (0..self.n)
            .filter(|&v| self.care & (1u64 << v) != 0)
            .map(|v| (v, self.value & (1u64 << v) == 0))
            .collect()
    }

    /// The LP value of this blocker's clause at a fractional point of `[0,1]ⁿ`: `Σ` over literals of
    /// `x` (positive) or `1−x` (negative). The clause's relaxation is satisfied iff the value is `≥ 1`.
    /// At the ½-center every literal contributes ½, so the value is `width/2` — satisfied iff width ≥ 2.
    pub fn clause_lp_value(&self, point: &[f64]) -> f64 {
        self.clause_literals()
            .iter()
            .map(|&(v, positive)| if positive { point[v] } else { 1.0 - point[v] })
            .sum()
    }

    /// **Resolve two blockers — the geometry of clause resolution, the engine of "rules beget rules."**
    /// Two blockers resolve when their fixed coordinates agree everywhere except a single *pivot* where
    /// they take opposite values (one clause carries the pivot literal, the other its negation, with no
    /// other clashing literal). The resolvent is the merged blocker with the pivot freed — a *new rule*
    /// derived from the two neighbors, exactly the Quine–McCluskey adjacency of two implicants. Returns
    /// `(pivot, resolvent)`, or `None` when there is no single clean pivot (no opposite literal, or a
    /// second clash making the resolvent a tautology).
    pub fn resolve(&self, other: &Subcube) -> Option<(usize, Subcube)> {
        let shared = self.care & other.care;
        let disagree = shared & (self.value ^ other.value);
        if disagree.count_ones() != 1 {
            return None;
        }
        let pivot = disagree.trailing_zeros() as usize;
        let care = (self.care | other.care) & !(1u64 << pivot);
        let value = (self.value | other.value) & care;
        Some((
            pivot,
            Subcube {
                n: self.n,
                care,
                value,
            },
        ))
    }

    /// Enumerate the blocked corners — the full footprint. Only sensible for small dimension.
    pub fn footprint(&self) -> Vec<Corner> {
        let free: Vec<u64> = (0..self.n as u64)
            .filter(|i| self.care & (1u64 << i) == 0)
            .collect();
        let mut out = Vec::with_capacity(1usize << free.len());
        for mask in 0..(1u64 << free.len()) {
            let mut c = self.value;
            for (j, &i) in free.iter().enumerate() {
                if mask & (1u64 << j) != 0 {
                    c |= 1u64 << i;
                }
            }
            out.push(c);
        }
        out
    }
}

/// A cover of the hypercube by clause-blockers — the geometric form of a CNF. UNSAT ⟺ the blockers
/// leave no corner uncovered.
#[derive(Clone, Debug)]
pub struct Cover {
    pub n: usize,
    pub blockers: Vec<Subcube>,
}

impl Cover {
    /// The blocker cover of a CNF: one subcube per clause.
    pub fn of_cnf(cnf: &DimacsCnf) -> Cover {
        let n = cnf.num_vars;
        let blockers = cnf.clauses.iter().map(|c| Subcube::blocker(c, n)).collect();
        Cover { n, blockers }
    }

    /// The **energy** of a corner: how many blockers cover it (`Pnp.lean`'s `vertexEnergy`).
    /// Energy zero ⟺ the corner is a satisfying assignment.
    pub fn vertex_energy(&self, corner: Corner) -> usize {
        self.blockers.iter().filter(|b| b.covers(corner)).count()
    }

    /// Is `corner` forbidden by some clause? (Does it falsify the formula?)
    pub fn blocks(&self, corner: Corner) -> bool {
        self.blockers.iter().any(|b| b.covers(corner))
    }

    /// `Pnp.lean`'s vertex energy classes. **Tight**: covered by exactly one blocker — that blocker is
    /// *essential* there, deleting it exposes the corner. **Redundant**: two or more cover it, robust
    /// to dropping one. (Uncovered, energy 0, is a model — [`escaping_corner`](Self::escaping_corner).)
    pub fn is_tight(&self, corner: Corner) -> bool {
        self.vertex_energy(corner) == 1
    }

    /// `Pnp.lean`'s `VertexOverlappedBy`: two or more blockers cover this corner.
    pub fn is_redundant(&self, corner: Corner) -> bool {
        self.vertex_energy(corner) >= 2
    }

    /// The **essential blockers** — the irreducible core of the cover. A blocker is essential when it
    /// *privately* covers some corner (a tight vertex no other blocker reaches); deleting it would
    /// break totality. The essential set is the geometric analog of a minimal resolution refutation:
    /// the rules you cannot drop. (Enumerates footprints — for small covers.)
    pub fn essential_blockers(&self) -> Vec<usize> {
        (0..self.blockers.len())
            .filter(|&i| {
                self.blockers[i]
                    .footprint()
                    .iter()
                    .any(|&c| self.vertex_energy(c) == 1)
            })
            .collect()
    }

    /// The first corner no blocker reaches — a satisfying assignment of energy zero — or `None`
    /// when the cover is total. `None` ⟺ the formula is UNSAT. Brute over all `2ⁿ` corners.
    pub fn escaping_corner(&self) -> Option<Corner> {
        (0u64..(1u64 << self.n)).find(|&c| !self.blocks(c))
    }

    /// UNSAT ⟺ the blockers cover **every** corner of the hypercube (`Pnp.lean`'s `HasNoHole`).
    pub fn is_total(&self) -> bool {
        self.escaping_corner().is_none()
    }

    /// The number of satisfying assignments — uncovered corners of energy zero (`solutionCount`).
    pub fn solution_count(&self) -> u64 {
        (0u64..(1u64 << self.n))
            .filter(|&c| !self.blocks(c))
            .count() as u64
    }

    /// `Pnp.lean`'s `HasNoHole` (refutation side): the cover is total — no corner escapes.
    pub fn has_no_hole(&self) -> bool {
        self.is_total()
    }

    /// **The ½ key — is the LP relaxation feasible at the symmetric center?** The all-½ point satisfies
    /// a clause's relaxation (`Σ lits ≥ 1`) iff the clause has width ≥ 2 (each literal contributes ½).
    /// When *every* blocker has width ≥ 2, the center `½ⁿ` is a feasible fractional point — so the cover
    /// can be integer-UNSAT while its LP relaxation is satisfiable. That integrality gap, sitting exactly
    /// at the symmetry-fixed center, is what resolution (which lives at the corners) cannot see and what
    /// the counting/cutting-planes shadows close.
    pub fn relaxation_feasible_at_center(&self) -> bool {
        self.blockers.iter().all(|b| b.care.count_ones() >= 2)
    }

    /// Generalized counting crush: derive the `O(1)` Hall certificate (`items > slots`) from *any*
    /// matching-shaped cover — pigeonhole, clique-coloring, anything that symmetry-breaks to the same
    /// two rule-types — by recovering the bipartite structure. The pigeonhole crush, no longer
    /// hard-coded to pigeonhole.
    pub fn counting_refutation(&self) -> Option<crate::pigeonhole::CountingCert> {
        crate::pigeonhole::counting_certificate(&self.to_expr()?)
    }

    /// The **full Hall refutation** — the matching invariant in its complete (subset) form. Catches a
    /// bipartite cover whose totals balance but where some subset of items competes for too few slots,
    /// returning the violating subset. Strictly stronger than `counting_refutation`.
    pub fn hall_refutation(&self) -> Option<crate::matching::HallWitness> {
        crate::pigeonhole::hall_refutation(&self.to_expr()?)
    }

    /// `Pnp.lean`'s `HasUniqueHole`: exactly one corner is uncovered (search-critical SAT).
    pub fn has_unique_hole(&self) -> bool {
        self.solution_count() == 1
    }

    /// `Pnp.lean`'s `HasAtLeastHoles k`: at least `k` corners remain uncovered (search-easy SAT).
    pub fn has_at_least_holes(&self, k: u64) -> bool {
        self.solution_count() >= k
    }

    /// `Pnp.lean`'s `BlockerFamily.SeparatedBy`: no blocker crosses the coordinate cut `cut` — each
    /// blocker's support lies entirely inside it or entirely outside. The hypercube version of a
    /// decomposition separator.
    pub fn separated_by(&self, cut: u64) -> bool {
        self.blockers
            .iter()
            .all(|b| (b.care & cut) == b.care || (b.care & cut) == 0)
    }

    /// `Pnp.lean`'s `BlockerFamily.VariableInteraction`: some blocker mentions both `i` and `j` —
    /// the primal-graph edge of the cover.
    pub fn variable_interaction(&self, i: usize, j: usize) -> bool {
        i != j
            && self
                .blockers
                .iter()
                .any(|b| b.care & (1u64 << i) != 0 && b.care & (1u64 << j) != 0)
    }

    /// Recover the CNF this cover is the geometry of, as a `ProofExpr` over atoms `x{var}` — the
    /// door back into the certified prover. `None` when a blocker is the empty clause (an immediate
    /// contradiction with no propositional form) or the cover has no blockers.
    pub fn to_expr(&self) -> Option<crate::ProofExpr> {
        use crate::ProofExpr;
        let lit = |v: usize, positive: bool| {
            let a = ProofExpr::Atom(format!("x{v}"));
            if positive {
                a
            } else {
                ProofExpr::Not(Box::new(a))
            }
        };
        let mut clauses = Vec::with_capacity(self.blockers.len());
        for b in &self.blockers {
            let lits = b.clause_literals();
            if lits.is_empty() {
                return None;
            }
            let mut it = lits.into_iter();
            let (v0, p0) = it.next().unwrap();
            clauses.push(it.fold(lit(v0, p0), |acc, (v, p)| {
                ProofExpr::Or(Box::new(acc), Box::new(lit(v, p)))
            }));
        }
        let mut it = clauses.into_iter();
        let first = it.next()?;
        Some(it.fold(first, |acc, c| ProofExpr::And(Box::new(acc), Box::new(c))))
    }

    /// Decide cover-totality through the **certified prover**, not brute force: route the cover's CNF
    /// into [`crate::sat::prove_unsat`], which returns a RUP/PR-checked `Refuted` when the cover is
    /// total (fail-closed — never a false `Refuted`) or a witnessing model when a corner escapes.
    /// This is what makes the geometry *provable*: pigeonhole covers certify via the counting shadow
    /// in polynomial time, where resolution would blow up.
    pub fn prove_total_certified(&self) -> crate::sat::UnsatOutcome {
        match self.to_expr() {
            Some(e) => crate::sat::prove_unsat(&e),
            None => crate::sat::UnsatOutcome::Unsupported,
        }
    }

    /// **Reference one rule, get the rules it nets us.** All blockers that resolve with blocker `i`,
    /// each paired with its pivot and the resolvent it produces — the neighbors of rule `i` in the
    /// resolution graph, and the new rules they beget.
    pub fn neighbors(&self, i: usize) -> Vec<(usize, usize, Subcube)> {
        (0..self.blockers.len())
            .filter(|&j| j != i)
            .filter_map(|j| {
                self.blockers[i]
                    .resolve(&self.blockers[j])
                    .map(|(pivot, r)| (j, pivot, r))
            })
            .collect()
    }

    /// Recover the clauses this cover is the geometry of, as packed [`Lit`]s — the door into the
    /// automorphism detector and the certified prover's `Lit`-level core.
    pub fn clauses(&self) -> Vec<Vec<Lit>> {
        self.blockers
            .iter()
            .map(|b| {
                b.clause_literals()
                    .into_iter()
                    .map(|(v, p)| Lit::new(v as u32, p))
                    .collect()
            })
            .collect()
    }

    /// **Symmetry-break the rules, not the corners.** Partition the blocker indices into orbits under
    /// the automorphism group generated by `generators`. Each generator must map the blocker *set*
    /// into itself (it is verified by the image landing back among the blockers); if one ever maps a
    /// blocker off the set it is not a rule-automorphism and we return `None`, fail-closed.
    ///
    /// This is the cheap, powerful move the corner-orbit walk is not: there are only *polynomially*
    /// many blockers (one per clause), so quotienting the rule set costs `O(generators · blockers ·
    /// n)` — no `2ⁿ` anywhere. The number of orbits is the count of *essentially distinct rules*: a
    /// complexity signature of the family computed without ever touching the exponential cube.
    /// (Assumes distinct blockers, as ordinary CNF families have.)
    pub fn blocker_orbits(&self, generators: &[CubeSym]) -> Option<Vec<Vec<usize>>> {
        let mut index: HashMap<Subcube, usize> = HashMap::new();
        for (i, b) in self.blockers.iter().enumerate() {
            index.entry(*b).or_insert(i);
        }
        let m = self.blockers.len();
        let mut seen = vec![false; m];
        let mut orbits = Vec::new();
        for start in 0..m {
            if seen[start] {
                continue;
            }
            let mut orbit = Vec::new();
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(i) = stack.pop() {
                orbit.push(i);
                for g in generators {
                    let image = g.map_subcube(&self.blockers[i]);
                    let &j = index.get(&image)?; // off the blocker set ⟹ not a rule-automorphism
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            orbit.sort_unstable();
            orbits.push(orbit);
        }
        Some(orbits)
    }

    /// **Discover** this cover's own symmetries and read off its rule-orbit signature — the fully
    /// self-driving complexity classifier. The detector ([`crate::symmetry_detect::find_generators`])
    /// returns a generating set of automorphisms as [`Perm`]s, and we quotient the rules by them with
    /// [`clause_orbits`] (clause-level, so it scales past the geometric cube's 63-variable ceiling).
    /// A maximally symmetric family (pigeonhole) collapses to a handful of orbits at every scale; a
    /// random instance, with a trivial automorphism group, collapses to nothing — every rule its own.
    pub fn discovered_rule_symmetry(&self) -> RuleSymmetry {
        let clauses = self.clauses();
        let generators = crate::symmetry_detect::find_generators(self.n, &clauses);
        let rule_orbits = clause_orbits(&clauses, &generators).len();
        RuleSymmetry {
            n: self.n,
            blockers: clauses.len(),
            generators: generators.len(),
            rule_orbits,
        }
    }
}

/// The rule-symmetry signature of a cover: how the polynomially-many blockers collapse under the
/// family's automorphism group. `rule_orbits` is the count of essentially-distinct rules — a measure
/// of structural complexity read off *without* enumerating the `2ⁿ` corners. A small, scale-invariant
/// `rule_orbits` is the geometric fingerprint of a family that admits a short symmetry-broken proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleSymmetry {
    pub n: usize,
    pub blockers: usize,
    pub generators: usize,
    pub rule_orbits: usize,
}

/// **Symmetry-break the rules at the clause level** — the lift-and-shift-left form that scales to any
/// number of variables. Partition the clause indices (the blockers) into orbits under a generating
/// set of automorphisms, applying each [`Perm`] to clause literals directly via the canonical
/// `clause_key`. A blocker *is* a clause, so this is the same
/// rule-quotient as [`Cover::blocker_orbits`] — but with no `2ⁿ` corner geometry and no 63-variable
/// ceiling, so it runs at scales where the cube is astronomically large. The orbit count is the
/// number of essentially-distinct rules. (Generators that move a clause off the set are simply not
/// followed — only genuine rule-automorphisms close orbits.)
pub fn clause_orbits(clauses: &[Vec<Lit>], generators: &[Perm]) -> Vec<Vec<usize>> {
    let index: HashMap<Vec<u32>, usize> = clauses
        .iter()
        .enumerate()
        .map(|(i, c)| (crate::symmetry_detect::clause_key(c), i))
        .collect();
    let m = clauses.len();
    let mut seen = vec![false; m];
    let mut orbits = Vec::new();
    for start in 0..m {
        if seen[start] {
            continue;
        }
        let mut orbit = Vec::new();
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            orbit.push(i);
            for g in generators {
                let key = crate::symmetry_detect::clause_key(&g.apply_clause(&clauses[i]));
                if let Some(&j) = index.get(&key) {
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        orbit.sort_unstable();
        orbits.push(orbit);
    }
    orbits
}

/// The pigeonhole grid symmetry group `Sₙ × Sₙ₋₁` as scalable [`Perm`]s — adjacent pigeon (row) and
/// hole (column) transpositions over the `n*(n-1)` grid variables. No `u64` cap.
pub fn php_perm_symmetries(n: usize) -> Vec<Perm> {
    let holes = n.saturating_sub(1);
    let num_vars = n * holes;
    let var = |p: usize, h: usize| p * holes + h;
    let mut gens = Vec::new();
    for p in 0..n.saturating_sub(1) {
        let mut images: Vec<Lit> = (0..num_vars as u32).map(Lit::pos).collect();
        for h in 0..holes {
            images.swap(var(p, h), var(p + 1, h));
        }
        gens.push(Perm::from_images(images));
    }
    for h in 0..holes.saturating_sub(1) {
        let mut images: Vec<Lit> = (0..num_vars as u32).map(Lit::pos).collect();
        for p in 0..n {
            images.swap(var(p, h), var(p, h + 1));
        }
        gens.push(Perm::from_images(images));
    }
    gens
}

/// The rule-symmetry signature of pigeonhole at scale `n`, computed at the clause level with the full
/// grid group `Sₙ × Sₙ₋₁`. The blocker set grows superlinearly and the cube has `2^{n(n-1)}` corners,
/// yet the rules always collapse to exactly **two** orbits — the complexity limit symmetry exposes,
/// computable at any `n` because it never touches the cube.
pub fn pigeonhole_rule_symmetry(n: usize) -> RuleSymmetry {
    let (cnf, _) = crate::families::php(n);
    let generators = php_perm_symmetries(n);
    let rule_orbits = clause_orbits(&cnf.clauses, &generators).len();
    RuleSymmetry {
        n,
        blockers: cnf.clauses.len(),
        generators: generators.len(),
        rule_orbits,
    }
}

/// An **abstract, scale-invariant refutation**: the family's rules symmetry-broken to their orbit
/// *types*, plus the abstract invariant those types violate, plus the constant-size witness. For
/// pigeonhole this is two rule-types (every pigeon takes a hole; no two share one) and the counting
/// fact pigeons > holes — identical at every scale. This is the lift-and-shift-left: the proof's true
/// size is `O(1)` in the rule-types, reached by *symmetry breaking the rules to their types*, never by
/// enumerating resolvents (the concrete and even the symmetric resolution closure both explode).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbstractRefutation {
    pub rule_types: usize,
    pub invariant: &'static str,
    pub witness: crate::pigeonhole::CountingCert,
}

/// Symmetry-break pigeonhole to its abstract certificate. The rules collapse (via [`clause_orbits`]) to
/// exactly **two** types regardless of `n`; the abstract invariant on those types is Hall's condition,
/// witnessed by the `O(1)` counting certificate pigeons > holes. The whole certificate is constant in
/// size and identical in shape at every scale — the auto-collapse of pigeonhole, lifted to the type
/// level where it actually scales. `None` only in the degenerate hole-free case.
pub fn pigeonhole_abstract_refutation(n: usize) -> Option<AbstractRefutation> {
    let (cnf, _) = crate::families::php(n);
    let rule_types = clause_orbits(&cnf.clauses, &php_perm_symmetries(n)).len();
    let witness =
        crate::pigeonhole::certify_pigeonhole_unsat(n as u128, n.saturating_sub(1) as u128)?;
    Some(AbstractRefutation {
        rule_types,
        invariant: "Hall/matching: pigeons > holes",
        witness,
    })
}

/// Apply a **flip-renaming** `x_v → ¬x_v` for every `v` with `flips[v]` — a phase-flip symmetry of the
/// cube. It permutes models bijectively (negate the flipped coordinates), so it preserves satisfiability.
pub fn apply_renaming(clauses: &[Vec<Lit>], flips: &[bool]) -> Vec<Vec<Lit>> {
    clauses
        .iter()
        .map(|c| {
            c.iter()
                .map(|l| {
                    if flips[l.var() as usize] {
                        l.negated()
                    } else {
                        *l
                    }
                })
                .collect()
        })
        .collect()
}

/// **Recognize a new symmetry: renamable-Horn.** Is there a flip-renaming under which every clause has
/// at most one positive literal (Horn)? Horn-SAT is polynomial (unit propagation finds the least model),
/// so a renamable-Horn formula is in a poly class our field cuts cannot see. Crucially, *finding the
/// renaming is itself a 2-SAT*: a clause is Horn-after-flip iff no two of its literals are both positive,
/// and "literal `l` is positive after flip" is the single `f`-literal `(l.var, l.is_positive)`. Returns
/// the flip-set, or `None` if no renaming makes it Horn.
pub fn renaming_to_horn(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<bool>> {
    use crate::twosat::{self, Lit as TLit, TwoSatOutcome};
    let flit = |l: &Lit| {
        if l.is_positive() {
            TLit::pos(l.var() as usize)
        } else {
            TLit::neg(l.var() as usize)
        }
    };
    let mut two_sat: Vec<(TLit, TLit)> = Vec::new();
    for c in clauses {
        for i in 0..c.len() {
            for j in (i + 1)..c.len() {
                two_sat.push((flit(&c[i]), flit(&c[j]))); // ¬(both positive after flip)
            }
        }
    }
    match twosat::solve(&two_sat, num_vars) {
        TwoSatOutcome::Sat(flips) => Some(flips),
        TwoSatOutcome::Unsat(_) => None,
    }
}

/// The order of a formula's **automorphism group** — `1` means *rigid* (only the identity preserves it),
/// the maximally asymmetric extreme. Discovers the generators and closes them under composition.
pub fn automorphism_group_size(num_vars: usize, clauses: &[Vec<Lit>]) -> usize {
    let generators = crate::symmetry_detect::find_generators(num_vars, clauses);
    let key = |p: &Perm| -> Vec<(u32, bool)> {
        (0..num_vars)
            .map(|v| {
                let l = p.apply(Lit::pos(v as u32));
                (l.var(), l.is_positive())
            })
            .collect()
    };
    let id = Perm::identity(num_vars);
    let mut seen: BTreeSet<Vec<(u32, bool)>> = [key(&id)].into_iter().collect();
    let mut group = vec![id];
    let mut i = 0;
    while i < group.len() {
        let g = group[i].clone();
        i += 1;
        for s in &generators {
            let h = s.compose(&g);
            if seen.insert(key(&h)) {
                group.push(h);
            }
        }
        if group.len() > 5_000_000 {
            break;
        }
    }
    group.len()
}

/// **Information theory — the bits of symmetry.** `log₂|Aut|` is the symmetry-entropy: the number of
/// bits the automorphism group compresses out of the formula's description (knowing one orbit
/// representative plus the group recovers the rest). High for symmetric structure, exactly `0` for a
/// rigid one — and that zero is the maximal-information, incompressible extreme.
pub fn symmetry_entropy_bits(num_vars: usize, clauses: &[Vec<Lit>]) -> f64 {
    (automorphism_group_size(num_vars, clauses) as f64).log2()
}

/// **Find the randomness.** Strip every structural lever — a certified cut decides it (so there was no
/// irreducible randomness), carve (unit/pure/subsumption) and bounded variable elimination peel structure
/// away — and return what survives: the irreducible core. `None` means the instance was fully structured
/// and got decided. `Some(core)` is the kernel where carving can do no more; check it with [`diagnose`] —
/// if it also has no cut and ~zero symmetry-bits, *that* is the randomness, isolated.
pub fn find_random_core(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    max_steps: usize,
) -> Option<Vec<Vec<Lit>>> {
    let mut current = clauses.to_vec();
    for _ in 0..max_steps {
        let cut = clauses_to_expr(&current).is_some_and(|e| {
            crate::pigeonhole::decide_pigeonhole_unsat(&e)
                || crate::xorsat::refute_via_parity(&e)
                || crate::pseudo_boolean::refute_clausal(&e)
        });
        if cut {
            return None; // structured — decided by a cut, no randomness to isolate
        }
        match carve(num_vars, &current) {
            CarveOutcome::Sat | CarveOutcome::Unsat => return None,
            CarveOutcome::Core { clauses: core, .. } if core.len() < current.len() => {
                current = core;
            }
            CarveOutcome::Core { clauses: core, .. } => {
                let eliminated = bounded_variable_elimination(num_vars, &core);
                if eliminated.len() < core.len() {
                    current = eliminated;
                } else {
                    return Some(core); // nothing reduces it further — the irreducible core
                }
            }
        }
    }
    Some(current)
}

/// Where `auto_advance` ended: a decided verdict, or the structureless residue (the irreducible core no
/// structural lever could touch — where you'd branch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdvanceStatus {
    Decided(bool),
    StructurelessResidue { core: usize },
}

/// One step of the self-driving reduction: the lever applied, and the structure remaining after it.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvanceStep {
    pub lever: &'static str,
    pub clauses: usize,
    pub symmetry_bits: f64,
}

/// **Auto-diagnose, auto-break, auto-advance — to the fixpoint.** Repeatedly diagnose the instance and
/// apply the most decisive lever it offers: a certified cut decides it outright; otherwise carve (unit /
/// pure-literal / subsumption) and bounded variable elimination peel it down; the loop advances until a
/// verdict drops out or no structural lever reduces it further — the structureless residue, where the
/// only remaining move is to branch. The returned trace shows the structure draining away step by step.
pub fn auto_advance(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    max_steps: usize,
) -> (AdvanceStatus, Vec<AdvanceStep>) {
    let mut current = clauses.to_vec();
    let mut trace = Vec::new();
    for _ in 0..max_steps {
        let bits = symmetry_entropy_bits(num_vars, &current);
        // A certified cut decides it.
        let cut = clauses_to_expr(&current).is_some_and(|e| {
            crate::pigeonhole::decide_pigeonhole_unsat(&e)
                || crate::xorsat::refute_via_parity(&e)
                || crate::pseudo_boolean::refute_clausal(&e)
        });
        if cut {
            trace.push(AdvanceStep {
                lever: "certified cut → UNSAT",
                clauses: current.len(),
                symmetry_bits: bits,
            });
            return (AdvanceStatus::Decided(false), trace);
        }
        // Carve.
        match carve(num_vars, &current) {
            CarveOutcome::Sat => {
                trace.push(AdvanceStep {
                    lever: "carve → SAT",
                    clauses: 0,
                    symmetry_bits: bits,
                });
                return (AdvanceStatus::Decided(true), trace);
            }
            CarveOutcome::Unsat => {
                trace.push(AdvanceStep {
                    lever: "carve → UNSAT",
                    clauses: 0,
                    symmetry_bits: bits,
                });
                return (AdvanceStatus::Decided(false), trace);
            }
            CarveOutcome::Core { clauses: core, .. } if core.len() < current.len() => {
                trace.push(AdvanceStep {
                    lever: "carve (unit/pure/subsume)",
                    clauses: core.len(),
                    symmetry_bits: bits,
                });
                current = core;
                continue;
            }
            CarveOutcome::Core { clauses: core, .. } => {
                // No carve reduction — try projecting out a dimension.
                let eliminated = bounded_variable_elimination(num_vars, &core);
                if eliminated.len() < core.len() {
                    trace.push(AdvanceStep {
                        lever: "variable elimination (project a dimension)",
                        clauses: eliminated.len(),
                        symmetry_bits: bits,
                    });
                    current = eliminated;
                    continue;
                }
                // Nothing reduces it — the structureless residue.
                trace.push(AdvanceStep {
                    lever: "irreducible core — no structure left (branch)",
                    clauses: core.len(),
                    symmetry_bits: bits,
                });
                return (
                    AdvanceStatus::StructurelessResidue { core: core.len() },
                    trace,
                );
            }
        }
    }
    (
        AdvanceStatus::StructurelessResidue {
            core: current.len(),
        },
        trace,
    )
}

/// A complete auto-diagnosis of an instance: every structure-detector run at once, so you can read off
/// the full menu of applicable symmetry-breaks and cuts — *what you can still do to it.*
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnosis {
    pub clauses: usize,
    pub symmetry_bits: f64,
    pub rule_quotient: usize,
    pub cut: Option<Shadow>,
    pub antipodal: bool,
    pub renamable_horn: bool,
    pub components: usize,
    pub autark_section: bool,
    pub core_clauses: usize,
}

/// Run every lever's detector and report what applies. The automated "what can we still do" — one call,
/// the whole portfolio probed.
pub fn diagnose(num_vars: usize, clauses: &[Vec<Lit>]) -> Diagnosis {
    let generators = crate::symmetry_detect::find_generators(num_vars, clauses);
    let rule_quotient = clause_orbits(clauses, &generators).len();
    let symmetry_bits = symmetry_entropy_bits(num_vars, clauses);
    let cut = clauses_to_expr(clauses).and_then(|e| {
        if crate::pigeonhole::decide_pigeonhole_unsat(&e) {
            Some(Shadow::Counting)
        } else if crate::xorsat::refute_via_parity(&e) {
            Some(Shadow::Parity)
        } else if crate::pseudo_boolean::refute_clausal(&e) {
            Some(Shadow::CuttingPlanes)
        } else {
            None
        }
    });
    let (_, assigned) = pure_literal_reduce(num_vars, clauses);
    let core_clauses = match carve(num_vars, clauses) {
        CarveOutcome::Core { clauses: c, .. } => c.len(),
        _ => 0,
    };
    Diagnosis {
        clauses: clauses.len(),
        symmetry_bits,
        rule_quotient,
        cut,
        antipodal: is_antipodally_symmetric(clauses),
        renamable_horn: renaming_to_horn(num_vars, clauses).is_some(),
        components: components(num_vars, clauses).len(),
        autark_section: !assigned.is_empty(),
        core_clauses,
    }
}

/// From a [`Diagnosis`], the list of symmetry-breaks and cuts that apply — the menu of moves, in order
/// of decisiveness. Empty global structure ⟹ the honest fallback: backdoor + branch on the residue.
pub fn applicable_levers(d: &Diagnosis) -> Vec<&'static str> {
    let mut levers = Vec::new();
    if let Some(s) = d.cut {
        levers.push(match s {
            Shadow::Counting => "counting/Hall cut (one-punch)",
            Shadow::Parity => "GF(2) parity cut (one-punch)",
            Shadow::CuttingPlanes => "cutting-planes cut (one-punch)",
        });
    }
    if d.symmetry_bits > 0.0 {
        levers.push("symmetry breaking (lex-leader prune)");
    }
    if d.antipodal {
        levers.push("antipodal / center-inversion (recursive)");
    }
    if d.renamable_horn {
        levers.push("renamable-Horn (poly via 2-SAT renaming)");
    }
    if d.components > 1 {
        levers.push("component decomposition");
    }
    if d.autark_section || d.core_clauses < d.clauses {
        levers.push("autarky / carving (unit, pure-literal, subsumption)");
    }
    if levers.is_empty() {
        levers.push("no global structure — backdoor + branch the residue (the honest wall)");
    }
    levers
}

/// The structural profile of an instance — what every lever reveals about *where its difficulty lives*.
/// `quotient` is the number of rule orbit-types under its discovered symmetry (how far the cube
/// collapses); `cut` is the certified shadow that decides it, if any; `core_clauses` is what survives
/// carving + bounded variable elimination (the irreducible residue). Together they place the instance
/// on the spectrum from "all symmetry, O(1) quotient, instantly cut" to "no symmetry, full quotient,
/// nothing reduces — the genuinely interesting core."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralProfile {
    pub clauses: usize,
    pub quotient: usize,
    pub cut: Option<Shadow>,
    pub core_clauses: usize,
}

/// Profile an instance: collapse its rules to orbit-types, probe which cut decides it, and carve it to
/// its irreducible core. The reading that emerges — quotient size tracks cut-decidability tracks core
/// reducibility — is the single axis underneath every lever: *difficulty is quotient size.*
pub fn structural_profile(num_vars: usize, clauses: &[Vec<Lit>]) -> StructuralProfile {
    let generators = crate::symmetry_detect::find_generators(num_vars, clauses);
    let quotient = clause_orbits(clauses, &generators).len();
    let cut = clauses_to_expr(clauses).and_then(|e| {
        if crate::pigeonhole::decide_pigeonhole_unsat(&e) {
            Some(Shadow::Counting)
        } else if crate::xorsat::refute_via_parity(&e) {
            Some(Shadow::Parity)
        } else if crate::pseudo_boolean::refute_clausal(&e) {
            Some(Shadow::CuttingPlanes)
        } else {
            None
        }
    });
    let core_clauses = match carve(num_vars, clauses) {
        CarveOutcome::Sat | CarveOutcome::Unsat => 0,
        CarveOutcome::Core { clauses: c, .. } => bounded_variable_elimination(num_vars, &c).len(),
    };
    StructuralProfile {
        clauses: clauses.len(),
        quotient,
        cut,
        core_clauses,
    }
}

/// Which certified shadow refutes a cover — the abstract class of its hardness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shadow {
    /// Counting / matching (pigeonhole, Hall) — resolution-hard, polynomial here.
    Counting,
    /// Parity / Gaussian over GF(2) (Tseitin, XOR) — resolution-hard, polynomial here.
    Parity,
    /// Cutting planes / cardinality (Farkas hyperplane) — resolution-hard, polynomial here.
    CuttingPlanes,
}

/// Push a model through an automorphism: if `σ` preserves the clause set and `m` satisfies it, so does
/// `σ(m)`. Variable `v`'s value lands on `σ(+v)`'s variable, negated when `σ(+v)` is the negative literal
/// — chosen so `m ⊨ C ⟹ σ(m) ⊨ σ(C)`.
pub fn apply_perm_to_model(perm: &Perm, model: &[bool]) -> Vec<bool> {
    let mut out = model.to_vec();
    for v in 0..model.len() {
        let image = perm.apply(Lit::pos(v as u32));
        out[image.var() as usize] = if image.is_positive() {
            model[v]
        } else {
            !model[v]
        };
    }
    out
}

/// **Symmetry generates solutions.** From one model, the entire orbit under a generating set of
/// automorphisms — every member a model too, produced with no search. The generative dual of resolution
/// (which begets *rules* from symmetry); here symmetry begets *solutions*.
pub fn model_orbit(model: &[bool], generators: &[Perm]) -> Vec<Vec<bool>> {
    let mut seen = BTreeSet::new();
    seen.insert(model.to_vec());
    let mut stack = vec![model.to_vec()];
    while let Some(m) = stack.pop() {
        for g in generators {
            let image = apply_perm_to_model(g, &m);
            if seen.insert(image.clone()) {
                stack.push(image);
            }
        }
    }
    seen.into_iter().collect()
}

/// **Symmetry-break the witness.** The canonical (lexicographically least) model in a witness's orbit
/// under the automorphisms — the symmetry-broken representative. All witnesses in one orbit reduce to the
/// same canonical witness, so the *essential* content of the solution set is one canonical witness per
/// orbit; the symmetry regenerates the rest via [`model_orbit`].
pub fn canonical_model(model: &[bool], generators: &[Perm]) -> Vec<bool> {
    model_orbit(model, generators).into_iter().min().unwrap()
}

/// The full symmetry group: every distinct `Perm` reachable by composing the generators (closure under
/// composition). Small-group only — the orbit-stabilizer accounting below needs the *whole* group, not a
/// generating set. Includes the identity.
pub fn perm_group_closure(generators: &[Perm], num_vars: usize) -> Vec<Perm> {
    let mut seen = std::collections::HashSet::new();
    let id = Perm::identity(num_vars);
    let mut frontier = vec![id.clone()];
    seen.insert(id);
    while let Some(g) = frontier.pop() {
        for h in generators {
            let gh = h.compose(&g);
            if seen.insert(gh.clone()) {
                frontier.push(gh);
            }
        }
    }
    seen.into_iter().collect()
}

/// **The stabilizer of a witness** — the subgroup of symmetries that fix it (`σ·m = m`). These are the
/// transformations under which the witness sees *itself*; they are exactly the redundancy in its
/// perspective of the others.
pub fn stabilizer(model: &[bool], group: &[Perm]) -> Vec<Perm> {
    group
        .iter()
        .filter(|g| apply_perm_to_model(g, model) == model)
        .cloned()
        .collect()
}

/// **Symmetry-break across the witness's perspective of the other witnesses.** From witness `m`, every
/// other witness in its orbit is reached by *some* symmetry — but many symmetries land on the same one
/// (they differ by a stabilizer element of `m`). The symmetry-broken perspective quotients that
/// redundancy out: exactly **one** representative transformation per distinct witness `m` can see — a
/// transversal of the coset space `G / Stab(m)`. The returned `(witness, σ)` pairs satisfy `σ·m =
/// witness`, and their count is `|G| / |Stab(m)| = |orbit(m)|` (orbit–stabilizer). The first entry is
/// `(m, identity)`: the witness's view of itself.
pub fn witness_perspective(
    model: &[bool],
    generators: &[Perm],
    num_vars: usize,
) -> Vec<(Vec<bool>, Perm)> {
    let group = perm_group_closure(generators, num_vars);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut by_dest: Vec<(Vec<bool>, Perm)> = Vec::new();
    for g in &group {
        let dest = apply_perm_to_model(g, model);
        if seen.insert(dest.clone()) {
            by_dest.push((dest, g.clone()));
        }
    }
    by_dest.sort_by(|a, b| a.0.cmp(&b.0));
    let here = by_dest.iter().position(|(d, _)| d == model).unwrap();
    out.push((model.to_vec(), Perm::identity(num_vars)));
    for (i, pair) in by_dest.into_iter().enumerate() {
        if i != here {
            out.push(pair);
        }
    }
    out
}

/// **Burnside orbit count — the number of essentially-distinct witnesses.** By Burnside's lemma the
/// number of orbits of a group action equals the *average* number of fixed points:
/// `#orbits = (1/|G|) · Σ_{g∈G} |Fix(g)|`, where `Fix(g) = { m : g·m = m }`. Applied to the solution
/// set (closed under the automorphisms, since every `g` is an automorphism), this counts the witnesses
/// **up to symmetry** — the essential solutions — as a fixed-point average, never enumerating an orbit.
/// The sum is exactly divisible by `|G|` (the lemma guarantees it); `group` must be the *whole* group
/// (use [`perm_group_closure`]).
pub fn burnside_orbit_count(models: &[Vec<bool>], group: &[Perm]) -> usize {
    let total_fixed: usize = group
        .iter()
        .map(|g| {
            models
                .iter()
                .filter(|m| apply_perm_to_model(g, m.as_slice()) == **m)
                .count()
        })
        .sum();
    total_fixed / group.len()
}

/// **Where an UNSAT instance sits in the proof-complexity landscape**, as our certified cuts see it.
/// This is a *ladder of proof systems* (Cook–Reckhow): each rung crushes families the cheaper ones are
/// blind to. `Counting` and `Parity` are **incomparable narrow detectors** — pigeonhole needs counting
/// and is invisible to GF(2); Tseitin needs GF(2) and is invisible to counting — while
/// `Nullstellensatz{min_degree}` is the *universal algebraic height* over GF(2), complete at degree `n`.
/// The honest face of the wall: an instance whose narrow cuts are silent and whose minimum NS degree is
/// large sits at the top of this ladder, and the cost *at* that height is exponential. We can *locate* an
/// instance on the ladder; we cannot prove the top rung is unavoidable for a family — that lower bound is
/// exactly P vs NP, and it stays open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofRung {
    /// Closed by unit propagation / carving alone — no real refutation needed.
    Trivial,
    /// A counting / Hall (pigeonhole) cut crushes it. Resolution-exponential families like PHP live here;
    /// incomparable to `Parity`.
    Counting,
    /// A GF(2) parity (Gaussian-elimination) cut crushes it. Tseitin / XOR families live here;
    /// incomparable to `Counting`.
    Parity,
    /// A certified mod-`p` Gaussian cut crushes it — `Parity` carried to the odd prime `p`: the CNF is a
    /// recognized one-hot encoding of a `GF(p)` linear system whose refutation re-checks. One rung per
    /// characteristic, each incomparable to the others and to `Counting`/`Parity` (the prime
    /// incomparability of `polycalc_gfp`). Reported only by the extended cascade
    /// ([`weakest_crushing_rung_with_char`]); the legacy cascade predates the characteristic axis.
    ModCount { p: u64 },
    /// No narrow cut fires; refuted only by Nullstellensatz / Polynomial Calculus over GF(2) at this
    /// minimum degree — the universal algebraic height. The rigid residue lives here.
    Nullstellensatz { min_degree: usize },
    /// No cut and no NS refutation within the degree budget — the wall as our detectors perceive it.
    BeyondBudget,
}

/// Locate an instance on the [`ProofRung`] ladder: the *weakest* certified cut that crushes it, probed
/// cheapest-first (carve ≺ counting ≺ parity ≺ Nullstellensatz-by-degree). The narrow rungs (`Counting`,
/// `Parity`) are incomparable; probe order only decides the label when more than one happens to fire.
///
/// **This has no satisfiability oracle.** `BeyondBudget` means *no certified cut fired* — which covers
/// **both** a satisfiable instance **and** a hard-UNSAT one beyond the degree budget. Our detectors
/// cannot tell those two apart cheaply; that very indistinguishability is the wall, and resolving it for
/// a family (proving the budget *must* be exceeded) is P vs NP.
pub fn weakest_crushing_rung(num_vars: usize, clauses: &[Vec<Lit>], ns_budget: usize) -> ProofRung {
    weakest_crushing_rung_with_char(num_vars, clauses, ns_budget, &[])
}

/// [`weakest_crushing_rung`] with the **characteristic rungs enabled** — the same cascade, plus, between
/// the parity probe and the algebraic ascent, a certified mod-`p` cut per prime in `primes`: when the
/// CNF is a recognized one-hot encoding of a `GF(p)` linear system ([`crate::modp::recover_from_cnf`],
/// which declines rather than guesses) and the `GF(p)` Gaussian refutation re-checks
/// ([`crate::modp::is_refutation`]), the instance lands on [`ProofRung::ModCount`]. This is the ladder
/// rung the census's `router_beats_ladder` audit flagged as missing: the structured router's
/// `Route::ModP` specialist finally has a certified proof system the ladder can name. With
/// `primes = &[]` the cascade is exactly the legacy one.
pub fn weakest_crushing_rung_with_char(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    ns_budget: usize,
    primes: &[u64],
) -> ProofRung {
    if let CarveOutcome::Unsat = carve(num_vars, clauses) {
        return ProofRung::Trivial;
    }
    let Some(e) = clauses_to_expr(clauses) else {
        return ProofRung::BeyondBudget;
    };
    if crate::pigeonhole::counting_certificate(&e).is_some()
        || crate::pigeonhole::hall_refutation(&e).is_some()
    {
        return ProofRung::Counting;
    }
    if crate::xorsat::refute_via_parity(&e) {
        return ProofRung::Parity;
    }
    if !primes.is_empty() {
        if let Some(rec) = crate::modp::recover_from_cnf(num_vars, clauses) {
            if primes.contains(&rec.modulus) {
                if let crate::modp::ModpOutcome::Unsat(combo) =
                    crate::modp::solve(&rec.equations, rec.num_vars, rec.modulus)
                {
                    if crate::modp::is_refutation(&rec.equations, rec.num_vars, rec.modulus, &combo)
                    {
                        return ProofRung::ModCount { p: rec.modulus };
                    }
                }
            }
        }
    }
    let cap = ns_budget.min(num_vars);
    if let Some(d) =
        (1..=cap).find(|&d| crate::polycalc::nullstellensatz_refutes(num_vars, clauses, d))
    {
        return ProofRung::Nullstellensatz { min_degree: d };
    }
    ProofRung::BeyondBudget
}

/// **Symmetry-aware solution counting.** Partition a set of models into orbits under a generating set
/// of automorphisms; each orbit is the full `model_orbit` of any of its members. The solution count is
/// the sum of the orbit sizes, so a symmetric instance collapses to *one representative per orbit* — far
/// fewer than the solutions themselves. (`models` should be closed under the symmetry, e.g. all models.)
pub fn partition_into_orbits(models: &[Vec<bool>], generators: &[Perm]) -> Vec<Vec<Vec<bool>>> {
    let model_set: BTreeSet<Vec<bool>> = models.iter().cloned().collect();
    let mut assigned: BTreeSet<Vec<bool>> = BTreeSet::new();
    let mut orbits = Vec::new();
    for m in models {
        if assigned.contains(m) {
            continue;
        }
        let orbit: Vec<Vec<bool>> = model_orbit(m, generators)
            .into_iter()
            .filter(|x| model_set.contains(x))
            .collect();
        for x in &orbit {
            assigned.insert(x.clone());
        }
        orbits.push(orbit);
    }
    orbits
}

/// **Autocarve — recursive carving that lets the rules fall out.** Carve the formula to its core,
/// decompose into independent components, and for each: if a certified cut recognizes it, the rule
/// *falls out* and the component closes; otherwise branch one variable and **carve again** on each
/// branch. Every decision cascades fresh unit propagations and pure literals, exposing structure the
/// previous level hid — so a buried, masked, or nested invariant surfaces on its own at the depth it
/// becomes visible. UNSAT iff any component is, SAT iff all are; `None` past the budget.
/// What an autocarve run did: how many recursion nodes it visited, how many times a certified cut
/// *fired* (a "punch"), and how deep the carving recursed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarveStats {
    pub nodes: usize,
    pub punches: usize,
    pub max_depth: usize,
}

pub fn autocarve(num_vars: usize, clauses: &[Vec<Lit>], budget: usize) -> Option<bool> {
    autocarve_measured(num_vars, clauses, budget).0
}

/// Like [`autocarve`], but also returns the [`CarveStats`] — node count, punch count, recursion depth.
pub fn autocarve_measured(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    budget: usize,
) -> (Option<bool>, CarveStats) {
    let mut stats = CarveStats::default();
    let verdict = autocarve_rec(num_vars, clauses, budget, 0, &mut stats);
    (verdict, stats)
}

fn autocarve_rec(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    budget: usize,
    depth: usize,
    stats: &mut CarveStats,
) -> Option<bool> {
    stats.nodes += 1;
    stats.max_depth = stats.max_depth.max(depth);
    if stats.nodes > budget {
        return None;
    }
    let core = match carve(num_vars, clauses) {
        CarveOutcome::Sat => return Some(true),
        CarveOutcome::Unsat => return Some(false),
        CarveOutcome::Core { clauses, .. } => clauses,
    };
    for component in components(num_vars, &core) {
        // The rule falls out: a certified cut recognizes the carved component — a punch. The full-set
        // counting bound (`items > slots`) is the *symmetric* special case — O(1) after extraction, no
        // matching — so try it first; only fall back to the general Hall matching when it doesn't fire.
        let cut = clauses_to_expr(&component).is_some_and(|e| {
            crate::pigeonhole::counting_certificate(&e).is_some()
                || crate::pigeonhole::decide_pigeonhole_unsat(&e)
                || crate::xorsat::refute_via_parity(&e)
                || crate::pseudo_boolean::refute_clausal(&e)
        });
        if cut {
            stats.punches += 1;
            return Some(false); // a UNSAT component refutes the whole formula
        }
        // Otherwise branch and carve again — the structure surfaces one decision deeper.
        let pivot = component[0][0].var();
        let mut component_sat = false;
        for value in [false, true] {
            let mut branch = component.clone();
            branch.push(vec![Lit::new(pivot, value)]);
            match autocarve_rec(num_vars, &branch, budget, depth + 1, stats) {
                Some(true) => {
                    component_sat = true;
                    break;
                }
                Some(false) => {}
                None => return None,
            }
        }
        if !component_sat {
            return Some(false); // both branches UNSAT ⟹ this component, and the whole formula, is UNSAT
        }
    }
    Some(true)
}

/// **The unified crush.** Compose every lever into one decision procedure: carve the autark sections
/// (pure literals), split into independent components, and decide each by the cut-enabled
/// symmetry-aware search — a component refuted by a certified cut at the root closes in one node, the
/// rest fall to bounded branch-and-cut. The formula is UNSAT iff any component is, SAT iff all are.
/// Returns `None` only when a component blows past the budget — the genuinely hard residue, honestly
/// surfaced rather than hidden.
pub fn crush(num_vars: usize, clauses: &[Vec<Lit>], budget: usize) -> Option<bool> {
    let (core, _) = pure_literal_reduce(num_vars, clauses);
    if core.is_empty() {
        return Some(true); // every section carved away — satisfiable
    }
    for component in components(num_vars, &core) {
        match search_cost(num_vars, &component, true, budget) {
            SearchCost::Decided { sat: false, .. } => return Some(false), // a UNSAT component refutes all
            SearchCost::Decided { sat: true, .. } => {}                   // satisfiable, keep going
            SearchCost::Exceeded { .. } => return None,                   // the hard residue
        }
    }
    Some(true) // every component satisfiable
}

fn resolve_on_var(cp: &[Lit], cn: &[Lit], v: usize) -> Option<Vec<Lit>> {
    let mut lits: Vec<Lit> = Vec::new();
    for &l in cp.iter().chain(cn.iter()) {
        if l.var() as usize != v && !lits.contains(&l) {
            lits.push(l);
        }
    }
    if lits.iter().any(|l| lits.contains(&l.negated())) {
        return None; // tautological resolvent — discard
    }
    Some(lits)
}

/// **Carve out a dimension.** Eliminate variable `v` by resolution (Davis–Putnam): drop every clause
/// mentioning `v`, and add the non-tautological resolvents of each `v`-clause against each `¬v`-clause.
/// Geometrically this *projects the cube's `v`-axis away* — the formula over `n` dimensions becomes an
/// equisatisfiable one over `n-1`. Sound: a model of the projection lifts to a model of the original.
pub fn eliminate_variable(v: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Lit>> {
    let (pv, nv) = (Lit::new(v as u32, true), Lit::new(v as u32, false));
    let mut result: Vec<Vec<Lit>> = clauses
        .iter()
        .filter(|c| !c.contains(&pv) && !c.contains(&nv))
        .cloned()
        .collect();
    let pos: Vec<&Vec<Lit>> = clauses.iter().filter(|c| c.contains(&pv)).collect();
    let neg: Vec<&Vec<Lit>> = clauses.iter().filter(|c| c.contains(&nv)).collect();
    for cp in &pos {
        for cn in &neg {
            if let Some(resolvent) = resolve_on_var(cp, cn, v) {
                result.push(resolvent);
            }
        }
    }
    result
}

/// **Bounded variable elimination** — carve away every dimension whose projection doesn't grow the
/// formula (resolvents ≤ clauses removed). Iterated to a fixpoint, it peels the cube down dimension by
/// dimension wherever it's free to do so; the variables that *would* explode (pigeonhole's, by Haken)
/// are left for the cuts. Satisfiability-preserving.
pub fn bounded_variable_elimination(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Lit>> {
    let mut current = clauses.to_vec();
    loop {
        let mut eliminated = false;
        for v in 0..num_vars {
            let pos = current
                .iter()
                .filter(|c| c.contains(&Lit::new(v as u32, true)))
                .count();
            let neg = current
                .iter()
                .filter(|c| c.contains(&Lit::new(v as u32, false)))
                .count();
            if pos == 0 || neg == 0 {
                continue;
            }
            let candidate = eliminate_variable(v, &current);
            if candidate.len() <= current.len() {
                current = candidate;
                eliminated = true;
            }
        }
        if !eliminated {
            break;
        }
    }
    current
}

/// What carving the hypercube reduced a formula to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarveOutcome {
    /// Carved to nothing — satisfiable.
    Sat,
    /// Carved to an empty clause — unsatisfiable.
    Unsat,
    /// Carved to an irreducible core, plus the literals forced along the way.
    Core {
        clauses: Vec<Vec<Lit>>,
        forced: Vec<Lit>,
    },
}

fn find_pure_literal(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Lit> {
    let mut pos = vec![false; num_vars];
    let mut neg = vec![false; num_vars];
    for c in clauses {
        for l in c {
            if l.is_positive() {
                pos[l.var() as usize] = true;
            } else {
                neg[l.var() as usize] = true;
            }
        }
    }
    (0..num_vars).find_map(|v| match (pos[v], neg[v]) {
        (true, false) => Some(Lit::new(v as u32, true)),
        (false, true) => Some(Lit::new(v as u32, false)),
        _ => None,
    })
}

fn subsume_once(clauses: &mut Vec<Vec<Lit>>) -> bool {
    for i in 0..clauses.len() {
        for j in 0..clauses.len() {
            if i != j
                && clauses[i].len() < clauses[j].len()
                && clauses[i].iter().all(|l| clauses[j].contains(l))
            {
                clauses.remove(j);
                return true;
            }
        }
    }
    false
}

/// **Carve away the hypercube.** Peel the formula down by the three classic simplifications, iterated to
/// a fixpoint: *unit propagation* (a unit clause carves the cube in half by forcing a variable),
/// *pure-literal* assignment (carves an autark section), and *subsumption* (drops a blocker contained in
/// a stronger one). All three preserve satisfiability, so the result is either a verdict or the
/// irreducible core that genuine hardness leaves behind. Pigeonhole carves to itself.
pub fn carve(num_vars: usize, clauses: &[Vec<Lit>]) -> CarveOutcome {
    let mut current: Vec<Vec<Lit>> = clauses.to_vec();
    let mut forced: Vec<Lit> = Vec::new();
    loop {
        if current.iter().any(|c| c.is_empty()) {
            return CarveOutcome::Unsat;
        }
        if current.is_empty() {
            return CarveOutcome::Sat;
        }
        let mut changed = false;
        if let Some(unit) = current.iter().find(|c| c.len() == 1).map(|c| c[0]) {
            forced.push(unit);
            let neg = unit.negated();
            current.retain(|c| !c.contains(&unit));
            for c in &mut current {
                c.retain(|&l| l != neg);
            }
            changed = true;
        } else if let Some(pure) = find_pure_literal(num_vars, &current) {
            forced.push(pure);
            current.retain(|c| !c.contains(&pure));
            changed = true;
        } else if subsume_once(&mut current) {
            changed = true;
        }
        if !changed {
            return CarveOutcome::Core {
                clauses: current,
                forced,
            };
        }
    }
}

/// **Cut out the autark sections.** A *pure literal* (a variable appearing in only one polarity) is the
/// simplest autarky: assigning it satisfies every clause it touches, so that whole section of the cube is
/// removed without affecting satisfiability. Iterate to a fixpoint and what remains is the hard core —
/// the part with no free section to cut. Returns `(core clauses, assigned pure literals)`. Sound: pure-
/// literal elimination preserves satisfiability (Davis–Putnam).
pub fn pure_literal_reduce(num_vars: usize, clauses: &[Vec<Lit>]) -> (Vec<Vec<Lit>>, Vec<Lit>) {
    let mut current: Vec<Vec<Lit>> = clauses.to_vec();
    let mut assigned = Vec::new();
    loop {
        let mut pos = vec![false; num_vars];
        let mut neg = vec![false; num_vars];
        for c in &current {
            for l in c {
                if l.is_positive() {
                    pos[l.var() as usize] = true;
                } else {
                    neg[l.var() as usize] = true;
                }
            }
        }
        let pure = (0..num_vars).find_map(|v| match (pos[v], neg[v]) {
            (true, false) => Some(Lit::new(v as u32, true)),
            (false, true) => Some(Lit::new(v as u32, false)),
            _ => None,
        });
        let Some(l) = pure else { break };
        assigned.push(l);
        current.retain(|c| !c.iter().any(|&x| x == l));
    }
    (current, assigned)
}

/// Partition a formula into its **independent components** — maximal clause groups sharing no variable
/// (the connected components of the variable-interaction graph). The formula is the conjunction of its
/// components, so it is UNSAT iff *any* component is, and each can be attacked on its own. A structured
/// UNSAT component buried in a big mixed formula — invisible to the monolithic cut — is laid bare here.
pub fn components(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Vec<Lit>>> {
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut parent: Vec<usize> = (0..num_vars.max(1)).collect();
    for clause in clauses {
        let vars: Vec<usize> = clause.iter().map(|l| l.var() as usize).collect();
        for pair in vars.windows(2) {
            let (a, b) = (find(&mut parent, pair[0]), find(&mut parent, pair[1]));
            parent[a] = b;
        }
    }
    let mut groups: HashMap<usize, Vec<Vec<Lit>>> = HashMap::new();
    for clause in clauses {
        let root = clause
            .first()
            .map(|l| find(&mut parent, l.var() as usize))
            .unwrap_or(0);
        groups.entry(root).or_default().push(clause.clone());
    }
    groups.into_values().collect()
}

/// Decompose into independent components and crush: return `true` (UNSAT) the moment any component is
/// refuted by a certified cut, never having examined the rest. Isolating a structured UNSAT component
/// unlocks a cut the monolithic formula hides.
pub fn decompose_and_crush(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    components(num_vars, clauses).iter().any(|comp| {
        clauses_to_expr(comp).is_some_and(|e| {
            crate::pigeonhole::decide_pigeonhole_unsat(&e)
                || crate::xorsat::refute_via_parity(&e)
                || crate::pseudo_boolean::refute_clausal(&e)
        })
    })
}

/// Is the cover invariant under the **antipodal map** — global negation `x → ¬x`, the center-inversion
/// of the cube? True iff flipping every literal of every clause maps the clause set onto itself. This is
/// a symmetry axis *distinct* from coordinate permutation: it is the involution whose unique fixed point
/// is the ½-center ([`CubeSym::map_fractional`] with all flips and no permutation fixes `½ⁿ`). When it
/// holds, satisfying assignments come in antipodal pairs `{a, ¬a}`, so one variable's value is free WLOG.
pub fn is_antipodally_symmetric(clauses: &[Vec<Lit>]) -> bool {
    let key = |c: &[Lit]| -> Vec<u32> {
        let mut k: Vec<u32> = c
            .iter()
            .map(|l| l.var() * 2 + u32::from(!l.is_positive()))
            .collect();
        k.sort_unstable();
        k.dedup();
        k
    };
    let original: BTreeSet<Vec<u32>> = clauses.iter().map(|c| key(c)).collect();
    let flipped: BTreeSet<Vec<u32>> = clauses
        .iter()
        .map(|c| key(&c.iter().map(|l| l.negated()).collect::<Vec<_>>()))
        .collect();
    original == flipped
}

/// What a laddered branch-and-cut search did: how many subcubes (nodes) it visited, how deep it
/// laddered, and how many subtrees a certified cut closed outright.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LadderStats {
    pub nodes: usize,
    pub max_depth: usize,
    pub cut_closures: usize,
    pub pruned: usize,
}

/// **Ladder up the hypercube: crush what we can, brute-force the rest.** Branch-and-cut over `{0,1}ⁿ`,
/// one variable at a time — exactly DPLL with our certified cuts as the theory:
/// 1. **base** — a residual empty clause means this subcube is fully covered (UNSAT branch); no
///    residual clauses means a corner escapes here (SAT).
/// 2. **unit propagation** — a residual unit clause forces its variable (free dimension reduction).
/// 3. **cut** — try the certified shadows (counting / parity / cutting-planes) on the residual; if one
///    fires, the whole subcube is crushed without descending — the learned invariant doing its work.
/// 4. **branch** — otherwise split on a residual variable and ladder down.
///
/// Structured families (pigeonhole, Tseitin, clique) are crushed by a cut at or near the root in a
/// handful of nodes *at any `n`*, because the cut is scale-free; the genuinely unstructured residual is
/// brute-forced by the branching. Works on raw clauses, so it ladders past the cube's 63-variable
/// geometric ceiling.
pub fn decide_laddered(num_vars: usize, clauses: &[Vec<Lit>]) -> (bool, LadderStats) {
    let mut stats = LadderStats {
        nodes: 0,
        max_depth: 0,
        cut_closures: 0,
        pruned: 0,
    };
    let sat = ladder(clauses, vec![None; num_vars], 0, &mut stats);
    (sat, stats)
}

/// **Symmetry-break the search itself.** The same branch-and-cut ladder, but it branches variables in
/// index order and *prunes* a node whenever a root automorphism maps its decided prefix to a
/// lexicographically smaller decided assignment — classic lex-leader symmetry breaking during search.
/// Sound by construction: every orbit of assignments keeps exactly one lex-leader, and only strict
/// non-leaders are pruned, so the verdict never changes; symmetric subtrees are simply skipped. The
/// automorphisms are discovered once at the root.
pub fn decide_laddered_sym(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    use_cut: bool,
) -> (bool, LadderStats) {
    let generators = crate::symmetry_detect::find_generators(num_vars, clauses);
    let mut stats = LadderStats {
        nodes: 0,
        max_depth: 0,
        cut_closures: 0,
        pruned: 0,
    };
    let sat = ladder_sym(
        clauses,
        vec![None; num_vars],
        0,
        &generators,
        use_cut,
        &mut stats,
    );
    (sat, stats)
}

/// The baseline the symmetry-pruned search is measured against: the *same* branch engine with no cut
/// and no generators (so `violates_lex_leader` never fires). Isolates the effect of symmetry pruning.
pub fn decide_laddered_nocut(num_vars: usize, clauses: &[Vec<Lit>]) -> (bool, LadderStats) {
    let mut stats = LadderStats {
        nodes: 0,
        max_depth: 0,
        cut_closures: 0,
        pruned: 0,
    };
    let sat = ladder_sym(clauses, vec![None; num_vars], 0, &[], false, &mut stats);
    (sat, stats)
}

/// The measured cost of a branch search: either it decided within the node budget, or the search blew
/// past it (the exponential explosion, captured rather than hung).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchCost {
    Decided { sat: bool, nodes: usize },
    Exceeded { budget: usize },
}

/// Run the branch engine purely to **measure** its size, with `use_cut` selecting whether the certified
/// cuts fire, and a hard `budget` on visited nodes so a resolution-class explosion is recorded as
/// `Exceeded` instead of running forever. With the cut off this is raw DPLL (resolution-strength); with
/// it on, the certified cut closes whole subtrees. The apples-to-apples gap between the two is the
/// campaign's thesis, quantified.
pub fn search_cost(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    use_cut: bool,
    budget: usize,
) -> SearchCost {
    let mut nodes = 0usize;
    match cost_rec(clauses, vec![None; num_vars], use_cut, budget, &mut nodes) {
        Some(sat) => SearchCost::Decided { sat, nodes },
        None => SearchCost::Exceeded { budget },
    }
}

/// **Recursive antipodal symmetry breaking.** Branch search that, at *every* node, re-detects whether
/// the residual is antipodally symmetric ([`is_antipodally_symmetric`]); when it is, it fixes the pivot
/// to `false` WLOG and prunes the `true` branch — soundly, since the residual's models come in antipodal
/// pairs. A disjoint union of self-complementary blocks keeps regaining the symmetry as each block's
/// first variable is fixed, so the break fires *recursively*, collapsing one factor of 2 per block.
pub fn search_cost_antipodal(num_vars: usize, clauses: &[Vec<Lit>], budget: usize) -> SearchCost {
    let mut nodes = 0usize;
    match antipodal_rec(clauses, vec![None; num_vars], budget, &mut nodes) {
        Some(sat) => SearchCost::Decided { sat, nodes },
        None => SearchCost::Exceeded { budget },
    }
}

fn antipodal_rec(
    clauses: &[Vec<Lit>],
    assignment: Vec<Option<bool>>,
    budget: usize,
    nodes: &mut usize,
) -> Option<bool> {
    *nodes += 1;
    if *nodes > budget {
        return None;
    }
    let residual = restrict(clauses, &assignment);
    if residual.iter().any(|c| c.is_empty()) {
        return Some(false);
    }
    if residual.is_empty() {
        return Some(true);
    }
    let pivot = residual[0][0].var() as usize;
    // When the residual is antipodally symmetric, the `true` branch mirrors the `false` one — prune it.
    let values: &[bool] = if is_antipodally_symmetric(&residual) {
        &[false]
    } else {
        &[false, true]
    };
    for &value in values {
        let mut next = assignment.clone();
        next[pivot] = Some(value);
        match antipodal_rec(clauses, next, budget, nodes) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => return None,
        }
    }
    Some(false)
}

fn cost_rec(
    clauses: &[Vec<Lit>],
    assignment: Vec<Option<bool>>,
    use_cut: bool,
    budget: usize,
    nodes: &mut usize,
) -> Option<bool> {
    *nodes += 1;
    if *nodes > budget {
        return None;
    }
    let residual = restrict(clauses, &assignment);
    if residual.iter().any(|c| c.is_empty()) {
        return Some(false);
    }
    if residual.is_empty() {
        return Some(true);
    }
    if use_cut {
        if let Some(e) = clauses_to_expr(&residual) {
            if crate::pigeonhole::decide_pigeonhole_unsat(&e)
                || crate::xorsat::refute_via_parity(&e)
                || crate::pseudo_boolean::refute_clausal(&e)
            {
                return Some(false);
            }
        }
    }
    let Some(pivot) = assignment.iter().position(|a| a.is_none()) else {
        return Some(true);
    };
    for value in [false, true] {
        let mut next = assignment.clone();
        next[pivot] = Some(value);
        match cost_rec(clauses, next, use_cut, budget, nodes) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => return None,
        }
    }
    Some(false)
}

fn ladder_sym(
    clauses: &[Vec<Lit>],
    assignment: Vec<Option<bool>>,
    depth: usize,
    generators: &[Perm],
    use_cut: bool,
    stats: &mut LadderStats,
) -> bool {
    stats.nodes += 1;
    stats.max_depth = stats.max_depth.max(depth);
    let residual = restrict(clauses, &assignment);
    if residual.iter().any(|c| c.is_empty()) {
        return false;
    }
    if residual.is_empty() {
        return true;
    }
    if use_cut {
        if let Some(e) = clauses_to_expr(&residual) {
            if crate::pigeonhole::decide_pigeonhole_unsat(&e)
                || crate::xorsat::refute_via_parity(&e)
                || crate::pseudo_boolean::refute_clausal(&e)
            {
                stats.cut_closures += 1;
                return false;
            }
        }
    }
    // Branch the lowest-index undecided variable (index order is what makes lex-leader sound).
    let Some(pivot) = assignment.iter().position(|a| a.is_none()) else {
        return true;
    };
    for value in [false, true] {
        let mut next = assignment.clone();
        next[pivot] = Some(value);
        if violates_lex_leader(&next, generators) {
            stats.pruned += 1;
            continue; // a symmetric, lex-smaller assignment is explored on an earlier branch
        }
        if ladder_sym(clauses, next, depth + 1, generators, use_cut, stats) {
            return true;
        }
    }
    false
}

/// Does some generator map the decided prefix of `a` to a lexicographically *smaller* decided
/// assignment over the same decided set? If so, `a` is not the lex-leader of its orbit and may be
/// pruned. Sound: an automorphism decreasing `a` proves the leader is strictly below `a`. (Checking
/// generators, not the whole group, only prunes *less* — never unsoundly.)
fn violates_lex_leader(a: &[Option<bool>], generators: &[Perm]) -> bool {
    let n = a.len();
    for sigma in generators {
        let mut b = vec![None; n];
        for v in 0..n {
            if let Some(val) = a[v] {
                let image = sigma.apply(Lit::pos(v as u32));
                b[image.var() as usize] = Some(if image.is_positive() { val } else { !val });
            }
        }
        // Only compare when σ keeps the decided set fixed (a clean restricted comparison).
        if (0..n).any(|v| a[v].is_some() != b[v].is_some()) {
            continue;
        }
        // Lexicographic compare b against a; prune iff b < a (false < true at the first difference).
        for v in 0..n {
            if let (Some(av), Some(bv)) = (a[v], b[v]) {
                if av != bv {
                    if !bv {
                        return true;
                    }
                    break;
                }
            }
        }
    }
    false
}

fn ladder(
    clauses: &[Vec<Lit>],
    assignment: Vec<Option<bool>>,
    depth: usize,
    stats: &mut LadderStats,
) -> bool {
    stats.nodes += 1;
    stats.max_depth = stats.max_depth.max(depth);
    let residual = restrict(clauses, &assignment);
    if residual.iter().any(|c| c.is_empty()) {
        return false; // an empty clause: this subcube is fully covered — UNSAT branch
    }
    if residual.is_empty() {
        return true; // nothing left to cover: a corner escapes — SAT
    }
    // Unit propagation: a forced literal reduces the free dimension with no branching.
    if let Some(unit) = residual.iter().find(|c| c.len() == 1) {
        let l = unit[0];
        let mut next = assignment.clone();
        next[l.var() as usize] = Some(l.is_positive());
        return ladder(clauses, next, depth + 1, stats);
    }
    // Cut: a certified shadow crushes the whole subcube if it recognizes the residual.
    if let Some(e) = clauses_to_expr(&residual) {
        if crate::pigeonhole::decide_pigeonhole_unsat(&e)
            || crate::xorsat::refute_via_parity(&e)
            || crate::pseudo_boolean::refute_clausal(&e)
        {
            stats.cut_closures += 1;
            return false;
        }
    }
    // Branch: split on a residual variable and ladder down (residual clauses are width ≥ 2 here).
    let pivot = residual[0][0].var() as usize;
    for value in [false, true] {
        let mut next = assignment.clone();
        next[pivot] = Some(value);
        if ladder(clauses, next, depth + 1, stats) {
            return true;
        }
    }
    false
}

/// The verdict of auto-cutting a cover: which certified cut showed it total (no corner escapes), or
/// that a corner escapes (satisfiable), or that it is not a cover the prover decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverVerdict {
    /// Total cover — UNSAT, every corner blocked. `cut` names the structured hyperplane family that
    /// certified it in polynomial time (counting / parity / cutting-planes), or `None` when the general
    /// certified prover (symmetry-broken CDCL → RUP) closed it.
    Total { cut: Option<Shadow> },
    /// A corner escapes the cover — satisfiable.
    Escapes,
    /// Not a propositional cover the prover handles.
    Unknown,
}

impl Cover {
    /// **Auto-cut and crush.** Try each certified cut in turn — the counting hyperplane (Hall), the
    /// affine GF(2) cut (Gaussian), the cardinality cutting plane (Farkas) — and fall back to the
    /// general certified prover if no structured cut fits. One call, every family: it reports which
    /// hyperplane family closed the cover, or that a corner escapes. This is the whole campaign behind
    /// a single door — and it is exactly `sat::prove_unsat`'s cascade, surfaced with the cut it used.
    pub fn auto_certify(&self) -> CoverVerdict {
        let Some(e) = self.to_expr() else {
            return CoverVerdict::Unknown;
        };
        if crate::pigeonhole::decide_pigeonhole_unsat(&e) {
            return CoverVerdict::Total {
                cut: Some(Shadow::Counting),
            };
        }
        if crate::xorsat::refute_via_parity(&e) {
            return CoverVerdict::Total {
                cut: Some(Shadow::Parity),
            };
        }
        if crate::pseudo_boolean::refute_clausal(&e) {
            return CoverVerdict::Total {
                cut: Some(Shadow::CuttingPlanes),
            };
        }
        match crate::sat::prove_unsat(&e) {
            crate::sat::UnsatOutcome::Refuted => CoverVerdict::Total { cut: None },
            crate::sat::UnsatOutcome::Sat(_) => CoverVerdict::Escapes,
            crate::sat::UnsatOutcome::Unsupported => CoverVerdict::Unknown,
        }
    }
}

/// The abstract signature of a family: its rules symmetry-broken to their orbit *types*, and which
/// certified shadow (if any) refutes it. This is the auto-collapse spread across families — the same
/// machinery that found pigeonhole's two-type counting abstraction, applied blind to any cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilySignature {
    pub num_vars: usize,
    pub clauses: usize,
    pub rule_types: usize,
    pub shadow: Option<Shadow>,
}

/// Symmetry-break any cover to its abstract signature: discover its automorphisms, quotient the rules
/// to orbit-types ([`clause_orbits`]), and probe the certified shadows in turn — counting, then parity,
/// then cutting-planes. `shadow = None` means no shadow recognizes it (it falls through to the general
/// certified CDCL core). A maximally symmetric family collapses to a few rule-types decided by one
/// shadow; an unstructured one spreads across many types with no shadow — but still has a backdoor.
pub fn abstract_signature(num_vars: usize, clauses: &[Vec<Lit>]) -> FamilySignature {
    let generators = crate::symmetry_detect::find_generators(num_vars, clauses);
    let rule_types = clause_orbits(clauses, &generators).len();
    let shadow = clauses_to_expr(clauses).and_then(|e| {
        if crate::pigeonhole::decide_pigeonhole_unsat(&e) {
            Some(Shadow::Counting)
        } else if crate::xorsat::refute_via_parity(&e) {
            Some(Shadow::Parity)
        } else if crate::pseudo_boolean::refute_clausal(&e) {
            Some(Shadow::CuttingPlanes)
        } else {
            None
        }
    });
    FamilySignature {
        num_vars,
        clauses: clauses.len(),
        rule_types,
        shadow,
    }
}

// ---- There is no such thing as random: backdoors to easy classes -------------------------------
//
// A "statistically random" instance is, once generated, a fixed deterministic object with definite
// structure — it merely lacks an obvious *global* symmetry. Its structure is *local*: a small set of
// variables (a **backdoor**) whose every fixing collapses the residual into a polynomially-decidable
// class. Hardness without a clean global symmetry is not noise; it is unfound structure. These
// primitives find that structure and use it — `2^k` easy branches instead of a `2ⁿ` search — which is
// also exactly how symmetry breaking buys *speed*, not just classification.

/// The residual CNF after fixing variables: `assignment[v] = Some(b)` fixes `v`, `None` leaves it
/// free. Clauses satisfied by a fixed literal are dropped; falsified literals are removed. An empty
/// clause in the result means the restriction already falsifies the formula.
pub fn restrict(clauses: &[Vec<Lit>], assignment: &[Option<bool>]) -> Vec<Vec<Lit>> {
    let mut out = Vec::new();
    'clause: for c in clauses {
        let mut residual = Vec::new();
        for &l in c {
            match assignment.get(l.var() as usize).copied().flatten() {
                Some(value) => {
                    if value == l.is_positive() {
                        continue 'clause; // a fixed literal satisfies the clause — drop it
                    }
                    // otherwise the literal is falsified — omit it from the residual
                }
                None => residual.push(l),
            }
        }
        out.push(residual);
    }
    out
}

fn to_twosat_lit(l: Lit) -> crate::twosat::Lit {
    if l.is_positive() {
        crate::twosat::Lit::pos(l.var() as usize)
    } else {
        crate::twosat::Lit::neg(l.var() as usize)
    }
}

/// Decide a width-≤2 CNF in polynomial time via the 2-SAT SCC solver. `true` = satisfiable. An empty
/// clause forces UNSAT. Panics if handed a clause wider than 2 (not a 2-SAT instance).
pub fn decide_2sat(clauses: &[Vec<Lit>], num_vars: usize) -> bool {
    let mut pairs = Vec::with_capacity(clauses.len());
    for c in clauses {
        match c.as_slice() {
            [] => return false,
            [a] => pairs.push((to_twosat_lit(*a), to_twosat_lit(*a))),
            [a, b] => pairs.push((to_twosat_lit(*a), to_twosat_lit(*b))),
            _ => panic!("decide_2sat given a width-{} clause", c.len()),
        }
    }
    matches!(
        crate::twosat::solve(&pairs, num_vars),
        crate::twosat::TwoSatOutcome::Sat(_)
    )
}

/// Greedily find a **backdoor to 2-SAT**: a set of variables `U` such that every clause has at most
/// two literals outside `U`. Then under *any* assignment to `U`, each clause is either satisfied or
/// shrinks to width ≤ 2, so the residual is 2-SAT — poly-decidable. Built by repeatedly fixing the
/// variable that appears in the most still-too-wide clauses (a hitting set of the wide clauses).
pub fn greedy_2sat_backdoor(clauses: &[Vec<Lit>], num_vars: usize) -> Vec<usize> {
    let mut chosen = vec![false; num_vars];
    let mut backdoor = Vec::new();
    loop {
        let mut freq = vec![0usize; num_vars];
        let mut any_wide = false;
        for c in clauses {
            let free = c.iter().filter(|l| !chosen[l.var() as usize]).count();
            if free > 2 {
                any_wide = true;
                for l in c {
                    if !chosen[l.var() as usize] {
                        freq[l.var() as usize] += 1;
                    }
                }
            }
        }
        if !any_wide {
            break;
        }
        let best = (0..num_vars).max_by_key(|&v| freq[v]).unwrap();
        chosen[best] = true;
        backdoor.push(best);
    }
    backdoor.sort_unstable();
    backdoor
}

/// Verify `U` is a **strong** backdoor to 2-SAT by enumeration: every one of the `2^{|U|}` assignments
/// to `U` yields a residual of width ≤ 2. Bounded to `|U| ≤ 24` (the enumeration guard).
pub fn is_strong_backdoor_to_2sat(
    clauses: &[Vec<Lit>],
    num_vars: usize,
    backdoor: &[usize],
) -> bool {
    let k = backdoor.len();
    if k > 24 {
        return false;
    }
    for mask in 0u32..(1u32 << k) {
        let mut assignment = vec![None; num_vars];
        for (i, &v) in backdoor.iter().enumerate() {
            assignment[v] = Some(mask & (1 << i) != 0);
        }
        if restrict(clauses, &assignment).iter().any(|c| c.len() > 2) {
            return false;
        }
    }
    true
}

/// Decide satisfiability through a 2-SAT backdoor: the instance is satisfiable iff *some* fixing of
/// `U` leaves a satisfiable 2-SAT residual. This solves in `2^{|U|}` polynomial branches instead of a
/// `2ⁿ` search — the structure (the backdoor) turned an exponential into a small, easy fan-out. `U`
/// must be a strong backdoor to 2-SAT (every residual width ≤ 2).
pub fn decide_sat_via_2sat_backdoor(
    clauses: &[Vec<Lit>],
    num_vars: usize,
    backdoor: &[usize],
) -> bool {
    for mask in 0u32..(1u32 << backdoor.len()) {
        let mut assignment = vec![None; num_vars];
        for (i, &v) in backdoor.iter().enumerate() {
            assignment[v] = Some(mask & (1 << i) != 0);
        }
        let residual = restrict(clauses, &assignment);
        if decide_2sat(&residual, num_vars) {
            return true;
        }
    }
    false
}

/// The orbit representative (canonical form) of a blocker under a generating set: the lexicographically
/// minimal blocker reachable through the generators. Two blockers are symmetric iff they share a
/// canonical form, so canonicalizing is how a symmetry-aware engine dedups derived rules by orbit.
pub fn canonical_blocker(b: &Subcube, generators: &[CubeSym]) -> Subcube {
    let mut best = *b;
    let mut seen = BTreeSet::new();
    seen.insert(*b);
    let mut stack = vec![*b];
    while let Some(x) = stack.pop() {
        for g in generators {
            let y = g.map_subcube(&x);
            if seen.insert(y) {
                if y < best {
                    best = y;
                }
                stack.push(y);
            }
        }
    }
    best
}

/// The full orbit of a blocker under the group generated by `generators` (BFS over images). For a
/// structured family this is *polynomial*-sized even though `|G|` is astronomical, because the
/// blocker's stabilizer is huge (a pigeonhole exclusion has orbit `holes·C(pigeons,2)`, not `|G|`).
pub fn blocker_orbit(b: &Subcube, generators: &[CubeSym]) -> Vec<Subcube> {
    let mut seen = BTreeSet::new();
    seen.insert(*b);
    let mut stack = vec![*b];
    while let Some(x) = stack.pop() {
        for g in generators {
            let y = g.map_subcube(&x);
            if seen.insert(y) {
                stack.push(y);
            }
        }
    }
    seen.into_iter().collect()
}

/// Symmetric resolution closure on orbit *representatives*: resolve each representative against every
/// image in another representative's orbit. Since resolution commutes with symmetry — `σ(resolve(c,d))
/// = resolve(σc,σd)` — this captures every derivable rule up to symmetry without building the raw
/// exponential closure. Returns the saturated count of orbit-types and whether the empty clause (a
/// refutation) was derived, bounded by `max_rounds` and a `max_reps` size guard.
///
/// **Measured limit (honest):** this refutes PHP(3) at 12 orbit-types but does **not** scale — at
/// PHP(4) the symmetric closure already explodes past tens of thousands of types. Even quotiented by
/// symmetry, the *full* resolution closure is the wrong abstraction; the `max_reps` guard makes it fail
/// safe rather than run away. The scalable path is [`pigeonhole_abstract_refutation`] — lift to the
/// rule-type level and apply the counting invariant, not the closure.
pub fn symmetric_resolution_closure(
    cover: &Cover,
    generators: &[CubeSym],
    max_rounds: usize,
    max_reps: usize,
) -> (usize, bool) {
    let empty = Subcube {
        n: cover.n,
        care: 0,
        value: 0,
    };
    let mut reps: BTreeSet<Subcube> = cover
        .blockers
        .iter()
        .map(|b| canonical_blocker(b, generators))
        .collect();
    let mut refuted = reps.contains(&empty);
    for _ in 0..max_rounds {
        if refuted {
            break;
        }
        let current: Vec<Subcube> = reps.iter().copied().collect();
        let orbits: Vec<Vec<Subcube>> = current
            .iter()
            .map(|d| blocker_orbit(d, generators))
            .collect();
        let mut added = false;
        'outer: for c in &current {
            for orbit in &orbits {
                for image in orbit {
                    if let Some((_, r)) = c.resolve(image) {
                        let canon = canonical_blocker(&r, generators);
                        if reps.insert(canon) {
                            added = true;
                            if canon == empty {
                                refuted = true;
                                break 'outer;
                            }
                            if reps.len() > max_reps {
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
        if !added {
            break;
        }
    }
    (reps.len(), refuted)
}

/// **Symmetric resolution closure — rules beget rules, collapsed by symmetry.** From a cover's
/// blockers, repeatedly resolve all pairs (each resolvent a new rule) and record, per round, both the
/// raw count of distinct derived rules and the count of their *orbit representatives* under the
/// automorphism group. The widening gap between the two is the symmetry collapse of the resolution
/// proof: raw resolution explodes, but modulo symmetry only a handful of essentially-distinct rules
/// are ever derived — the geometric reason symmetric proof systems refute pigeonhole in polynomial
/// size where plain resolution cannot.
pub fn symmetric_resolution_growth(
    cover: &Cover,
    generators: &[CubeSym],
    rounds: usize,
) -> Vec<(usize, usize)> {
    let mut raw: BTreeSet<Subcube> = cover.blockers.iter().copied().collect();
    let mut out = Vec::new();
    for _ in 0..rounds {
        let current: Vec<Subcube> = raw.iter().copied().collect();
        for i in 0..current.len() {
            for j in (i + 1)..current.len() {
                if let Some((_, r)) = current[i].resolve(&current[j]) {
                    raw.insert(r);
                }
            }
        }
        let orbits: BTreeSet<Subcube> = raw
            .iter()
            .map(|b| canonical_blocker(b, generators))
            .collect();
        out.push((raw.len(), orbits.len()));
    }
    out
}

/// **Symmetry breaking for speed.** Count the orbits of the `2^{|U|}` backdoor branches (assignments
/// to `U`) under the generators that preserve `U` setwise. A symmetry-aware solver inspects one branch
/// per orbit instead of all `2^{|U|}` — fewer poly-time solves for the same verdict. Generators that
/// move `U` off itself do not act on the branches and are skipped; each kept generator induces a
/// permutation-with-flips on the backdoor positions.
pub fn backdoor_branch_orbit_count(backdoor: &[usize], generators: &[Perm]) -> u64 {
    let k = backdoor.len();
    let position: HashMap<usize, usize> =
        backdoor.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let mut induced: Vec<(Vec<usize>, u32)> = Vec::new();
    for g in generators {
        let mut perm = vec![0usize; k];
        let mut flip = 0u32;
        let mut preserves = true;
        for (i, &v) in backdoor.iter().enumerate() {
            let image = g.apply(Lit::pos(v as u32));
            match position.get(&(image.var() as usize)) {
                Some(&j) => {
                    perm[i] = j;
                    if !image.is_positive() {
                        flip |= 1 << j;
                    }
                }
                None => {
                    preserves = false;
                    break;
                }
            }
        }
        if preserves {
            induced.push((perm, flip));
        }
    }
    let total = 1u32 << k;
    let mut seen = vec![false; total as usize];
    let mut orbits = 0u64;
    for start in 0..total {
        if seen[start as usize] {
            continue;
        }
        orbits += 1;
        let mut stack = vec![start];
        seen[start as usize] = true;
        while let Some(m) = stack.pop() {
            for (perm, flip) in &induced {
                let mut image = 0u32;
                for i in 0..k {
                    if m & (1 << i) != 0 {
                        image |= 1 << perm[i];
                    }
                }
                image ^= flip;
                if !seen[image as usize] {
                    seen[image as usize] = true;
                    stack.push(image);
                }
            }
        }
    }
    orbits
}

/// Build the CNF `ProofExpr` over atoms `x{var}` from raw clauses — the door into the certified
/// prover, scalable (no cube). `None` on an empty clause or empty formula.
pub fn clauses_to_expr(clauses: &[Vec<Lit>]) -> Option<crate::ProofExpr> {
    use crate::ProofExpr;
    let lit = |l: &Lit| {
        let a = ProofExpr::Atom(format!("x{}", l.var()));
        if l.is_positive() {
            a
        } else {
            ProofExpr::Not(Box::new(a))
        }
    };
    // Combine `nodes` into a BALANCED binary tree (depth O(log n)) rather than a linear left spine, so a
    // flat CNF's connective tree is logarithmically deep. Every recursive walker over it (clause
    // collectors, clausifiers, evaluators) then recurses O(log n) deep and cannot overflow the stack on
    // a several-thousand-clause formula — the pathological structure simply is never built.
    fn balanced(
        mut nodes: Vec<ProofExpr>,
        combine: impl Fn(Box<ProofExpr>, Box<ProofExpr>) -> ProofExpr,
    ) -> ProofExpr {
        while nodes.len() > 1 {
            let mut next = Vec::with_capacity((nodes.len() + 1) / 2);
            let mut it = nodes.into_iter();
            while let Some(a) = it.next() {
                match it.next() {
                    Some(b) => next.push(combine(Box::new(a), Box::new(b))),
                    None => next.push(a),
                }
            }
            nodes = next;
        }
        nodes
            .into_iter()
            .next()
            .expect("balanced() requires a non-empty node list")
    }
    let mut built = Vec::with_capacity(clauses.len());
    for c in clauses {
        if c.is_empty() {
            return None;
        }
        let lits: Vec<ProofExpr> = c.iter().map(|l| lit(l)).collect();
        built.push(balanced(lits, |a, b| ProofExpr::Or(a, b)));
    }
    if built.is_empty() {
        return None;
    }
    Some(balanced(built, |a, b| crate::ProofExpr::And(a, b)))
}

/// A hypercube symmetry: permute coordinates, then optionally flip each one (`Pnp.lean`'s
/// `CubeSymmetry`). `perm[j]` is the coordinate that `j`'s value lands on; `flip[j]` negates it.
/// These are the automorphisms that move blockers among blockers and corners among corners with the
/// **same** action — the bridge that puts rules and solutions in one mathematical world.
#[derive(Clone, Debug)]
pub struct CubeSym {
    pub perm: Vec<usize>,
    pub flip: Vec<bool>,
}

impl CubeSym {
    /// The identity symmetry on `n` coordinates.
    pub fn identity(n: usize) -> CubeSym {
        CubeSym {
            perm: (0..n).collect(),
            flip: vec![false; n],
        }
    }

    /// Push a corner forward through the symmetry (`mapVertex`): coordinate `j`'s (possibly flipped)
    /// value moves to coordinate `perm[j]`.
    pub fn map_corner(&self, c: Corner) -> Corner {
        let mut out = 0u64;
        for j in 0..self.perm.len() {
            let mut bit = (c >> j) & 1;
            if self.flip[j] {
                bit ^= 1;
            }
            out |= bit << self.perm[j];
        }
        out
    }

    /// Push a blocker forward through the symmetry (`mapBlocker`): each fixed coordinate `j` moves
    /// to `perm[j]`, its required value flipped when `flip[j]`.
    pub fn map_subcube(&self, s: &Subcube) -> Subcube {
        let mut care = 0u64;
        let mut value = 0u64;
        for j in 0..self.perm.len() {
            if s.care & (1u64 << j) != 0 {
                let pj = self.perm[j];
                care |= 1u64 << pj;
                let mut bit = (s.value >> j) & 1;
                if self.flip[j] {
                    bit ^= 1;
                }
                value |= bit << pj;
            }
        }
        Subcube {
            n: s.n,
            care,
            value,
        }
    }

    /// Act on a *fractional* point of the cube `[0,1]ⁿ`: coordinate `j`'s value (flipped to `1−x` when
    /// `flip[j]`) lands on coordinate `perm[j]`. The linear extension of `map_corner` to the solid cube.
    pub fn map_fractional(&self, point: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.perm.len()];
        for j in 0..self.perm.len() {
            out[self.perm[j]] = if self.flip[j] {
                1.0 - point[j]
            } else {
                point[j]
            };
        }
        out
    }

    /// Compose two cube symmetries: `(self ∘ other)` applies `other` then `self`.
    pub fn compose(&self, other: &CubeSym) -> CubeSym {
        let n = self.perm.len();
        let mut perm = vec![0usize; n];
        let mut flip = vec![false; n];
        for j in 0..n {
            let mid = other.perm[j];
            perm[j] = self.perm[mid];
            flip[j] = other.flip[j] ^ self.flip[mid];
        }
        CubeSym { perm, flip }
    }

    /// Is this symmetry an **automorphism** of the cover — does it map the blocker *set* onto
    /// itself? Re-verified directly (the soundness check), exactly as `swap_is_automorphism` is for
    /// the clausal path: a finder that proposes a non-automorphism is caught here, never trusted.
    pub fn is_automorphism(&self, cover: &Cover) -> bool {
        let original: BTreeSet<Subcube> = cover.blockers.iter().copied().collect();
        let mapped: BTreeSet<Subcube> =
            cover.blockers.iter().map(|b| self.map_subcube(b)).collect();
        original == mapped
    }
}

/// The group generated by `generators` (closure under composition) — for small groups, so the
/// combinatorial orbit-counting lemma can sum over every element.
fn group_closure(generators: &[CubeSym], n: usize) -> Vec<CubeSym> {
    let id = CubeSym::identity(n);
    let key = |g: &CubeSym| (g.perm.clone(), g.flip.clone());
    let mut seen: BTreeSet<(Vec<usize>, Vec<bool>)> = [key(&id)].into_iter().collect();
    let mut group = vec![id];
    let mut i = 0;
    while i < group.len() {
        let g = group[i].clone();
        i += 1;
        for s in generators {
            let h = s.compose(&g);
            if seen.insert(key(&h)) {
                group.push(h);
            }
        }
        if group.len() > 200_000 {
            break;
        }
    }
    group
}

/// **Combinatorics — Burnside's lemma.** The number of corner orbits equals the *average* number of
/// corners fixed by a group element: `(1/|G|) Σ_g |Fix(g)|`. A different invariant for the same count the
/// orbit-BFS computes — counting by fixed points instead of by walking orbits. (`n ≤ ~16`.)
pub fn burnside_corner_orbits(n: usize, generators: &[CubeSym]) -> u64 {
    let group = group_closure(generators, n);
    let fixed_total: u128 = group
        .iter()
        .map(|g| {
            (0u64..(1u64 << n))
                .filter(|&c| g.map_corner(c) == c)
                .count() as u128
        })
        .sum();
    (fixed_total / group.len() as u128) as u64
}

/// **Analysis — the Walsh–Hadamard (Fourier) spectrum.** Expand the cover's vertex-energy function over
/// the cube's characters `χ_S(x) = (-1)^{⟨S,x⟩}`: `f̂(S) = 2⁻ⁿ Σ_x energy(x) χ_S(x)`. Harmonic analysis on
/// the hypercube — the coefficients are the analytic invariant. (`n ≤ ~16`.)
pub fn walsh_hadamard_energy(cover: &Cover) -> Vec<f64> {
    let size = 1usize << cover.n;
    let f: Vec<f64> = (0..size as u64)
        .map(|x| cover.vertex_energy(x) as f64)
        .collect();
    (0..size)
        .map(|s| {
            let acc: f64 = (0..size)
                .map(|x| {
                    if ((s & x) as u64).count_ones() % 2 == 0 {
                        f[x]
                    } else {
                        -f[x]
                    }
                })
                .sum();
            acc / size as f64
        })
        .collect()
}

/// **Geometry — the f-vector.** The number of blockers of each face dimension. A symmetry permutes
/// blockers among themselves but preserves each one's dimension, so the f-vector is a geometric invariant
/// of the cover (the discrete analog of a polytope's face counts).
pub fn face_vector(cover: &Cover) -> std::collections::BTreeMap<usize, usize> {
    let mut fv = std::collections::BTreeMap::new();
    for b in &cover.blockers {
        *fv.entry(b.dimension()).or_insert(0) += 1;
    }
    fv
}

/// Orbit partition of the `2ⁿ` corners under a set of generators, each a verified automorphism.
/// Returns one representative per orbit (the orbit-collapsed corner set the cover check needs).
/// Materializes a `2ⁿ` seen-bitmap, so this is itself bounded by the hypercube size — it *measures*
/// the collapse rather than escaping it; the counting/parity shadows are what decide totality
/// without the `2ⁿ` walk. Honest by construction.
pub fn orbit_representatives(n: usize, generators: &[CubeSym]) -> Vec<Corner> {
    let total = 1u64 << n;
    let mut seen = vec![false; total as usize];
    let mut reps = Vec::new();
    for start in 0u64..total {
        if seen[start as usize] {
            continue;
        }
        reps.push(start);
        let mut stack = vec![start];
        seen[start as usize] = true;
        while let Some(c) = stack.pop() {
            for g in generators {
                let d = g.map_corner(c);
                if !seen[d as usize] {
                    seen[d as usize] = true;
                    stack.push(d);
                }
            }
        }
    }
    reps
}

/// How many orbits the corners fall into under these generators.
pub fn orbit_count(n: usize, generators: &[CubeSym]) -> u64 {
    orbit_representatives(n, generators).len() as u64
}

/// The **symmetry-broken** cover-totality check: when every generator is a verified automorphism,
/// the cover is total iff it covers one representative per orbit. Same verdict as [`Cover::is_total`],
/// but it inspects `orbit_count` corners instead of `2ⁿ`. `None` (fail-closed) when some generator is
/// not actually an automorphism — never a guessed answer.
pub fn is_total_via_orbits(cover: &Cover, generators: &[CubeSym]) -> Option<bool> {
    if !generators.iter().all(|g| g.is_automorphism(cover)) {
        return None;
    }
    let reps = orbit_representatives(cover.n, generators);
    Some(reps.iter().all(|&c| cover.blocks(c)))
}

/// One step of the collapse curve: how many orbits remain after stacking the first `k` generators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollapseStep {
    pub generators_used: usize,
    pub orbits: u64,
}

/// Stack the generators one at a time and record how the orbit count shrinks — the executable answer
/// to "how much does each symmetry break cut the problem?" Begins at `2ⁿ` (no symmetry) and descends
/// as generators compose. The ratios are *measured*, not asserted.
pub fn collapse_curve(n: usize, generators: &[CubeSym]) -> Vec<CollapseStep> {
    let mut steps = Vec::with_capacity(generators.len() + 1);
    steps.push(CollapseStep {
        generators_used: 0,
        orbits: 1u64 << n,
    });
    for k in 1..=generators.len() {
        steps.push(CollapseStep {
            generators_used: k,
            orbits: orbit_count(n, &generators[..k]),
        });
    }
    steps
}

/// The pigeonhole cover: `n` pigeons into `n-1` holes. Variable `(p,h)` ("pigeon `p` in hole `h`")
/// lives at index `p*(n-1)+h`, matching [`crate::families::php`]. Always buildable, always UNSAT.
pub fn php_cover(n: usize) -> Cover {
    let (cnf, _) = crate::families::php(n);
    Cover::of_cnf(&cnf)
}

/// The generating symmetries of the pigeonhole cover: adjacent transpositions of pigeons (rows) and
/// of holes (columns). Together they generate the full grid symmetry group `Sₙ × Sₙ₋₁` — pure
/// coordinate permutations (no phase flips). Each is a [`CubeSym`] over the `n*(n-1)` grid variables.
pub fn php_symmetries(n: usize) -> Vec<CubeSym> {
    let holes = n.saturating_sub(1);
    let num_vars = n * holes;
    let var = |p: usize, h: usize| p * holes + h;
    let mut gens = Vec::new();
    // Adjacent pigeon (row) swaps.
    for p in 0..n.saturating_sub(1) {
        let mut perm: Vec<usize> = (0..num_vars).collect();
        for h in 0..holes {
            perm.swap(var(p, h), var(p + 1, h));
        }
        gens.push(CubeSym {
            perm,
            flip: vec![false; num_vars],
        });
    }
    // Adjacent hole (column) swaps.
    for h in 0..holes.saturating_sub(1) {
        let mut perm: Vec<usize> = (0..num_vars).collect();
        for p in 0..n {
            perm.swap(var(p, h), var(p, h + 1));
        }
        gens.push(CubeSym {
            perm,
            flip: vec![false; num_vars],
        });
    }
    gens
}

/// Generators of the full **hyperoctahedral group** `Bₙ = (ℤ/2)ⁿ ⋊ Sₙ` — the signed permutations,
/// the automorphism group of the `n`-cube and the *complete* clause-level symmetry: every [`CubeSym`]
/// is one of its elements. The `n−1` adjacent coordinate transpositions generate `Sₙ`; the single
/// coordinate-0 flip, conjugated by those, generates the `(ℤ/2)ⁿ` of phase flips; together they
/// generate all of `Bₙ`. The [`cube_group_closure`] of these has order exactly `2ⁿ·n!`
/// (`1, 2, 8, 48, 384, 3840` for `n = 0..=5`). The census quotients minimal covers by this group.
pub fn hyperoctahedral_generators(n: usize) -> Vec<CubeSym> {
    let mut gens = Vec::new();
    for i in 0..n.saturating_sub(1) {
        let mut perm: Vec<usize> = (0..n).collect();
        perm.swap(i, i + 1);
        gens.push(CubeSym {
            perm,
            flip: vec![false; n],
        });
    }
    if n > 0 {
        let mut flip = vec![false; n];
        flip[0] = true;
        gens.push(CubeSym {
            perm: (0..n).collect(),
            flip,
        });
    }
    gens
}

/// The full group generated by `generators`, materialized (closure under composition) — the public
/// door onto the otherwise-internal `group_closure`. For the small groups the census needs (`Bₙ`,
/// `n ≤ 5` ⇒ `|G| = 3840`) the orbit–stabilizer cross-checks sum over every element.
pub fn cube_group_closure(generators: &[CubeSym], n: usize) -> Vec<CubeSym> {
    group_closure(generators, n)
}

/// **The minimum width of a resolution refutation of an UNSAT cover** — the size of the widest clause
/// any width-bounded refutation must carry, the classic resolution complexity measure (Ben-Sasson–
/// Wigderson). A subcube *is* a clause (`Subcube::clause_literals`) and [`Subcube::resolve`] is the
/// geometry of the resolution step; the empty subcube (`care = 0`, the whole cube blocked) is the
/// derived contradiction. For width budget `w` we seed with the input blockers of support `≤ w` and
/// saturate under width-`≤ w` resolution; the least `w` that derives the empty subcube is the width.
/// `None` only if the cover is satisfiable (no refutation at any width); every UNSAT cover succeeds by
/// `w = n` since full-width resolution is complete. (For small covers — it enumerates resolvents.)
pub fn min_resolution_width(cover: &Cover) -> Option<usize> {
    let n = cover.n;
    let empty = Subcube {
        n,
        care: 0,
        value: 0,
    };
    for w in 0..=n {
        let mut set: BTreeSet<Subcube> = cover
            .blockers
            .iter()
            .copied()
            .filter(|b| b.care.count_ones() as usize <= w)
            .collect();
        if set.contains(&empty) {
            return Some(w);
        }
        loop {
            let snapshot: Vec<Subcube> = set.iter().copied().collect();
            let mut added = false;
            for i in 0..snapshot.len() {
                for j in (i + 1)..snapshot.len() {
                    if let Some((_, r)) = snapshot[i].resolve(&snapshot[j]) {
                        if r.care.count_ones() as usize <= w && set.insert(r) {
                            added = true;
                        }
                    }
                }
            }
            if set.contains(&empty) {
                return Some(w);
            }
            if !added {
                break;
            }
        }
    }
    None
}

/// The lexicographic key of a cover — its sorted blocker list — for orbit canonicalization. Two covers
/// are the same set of clauses iff their keys match.
fn cover_key(blockers: &[Subcube]) -> Vec<Subcube> {
    let mut k = blockers.to_vec();
    k.sort_unstable();
    k.dedup();
    k
}

/// The canonical key of a cover under a **materialized** group: the lexicographically least sorted
/// blocker list over all images `{ g·blockers : g ∈ group }`. A flat `min` over the precomputed group —
/// no per-call orbit BFS — so it is cheap enough to evaluate on every node of the orderly-generation
/// search. Constant on a `Bₙ` orbit and distinct across orbits, hence a sound orbit invariant.
fn canonical_key(blockers: &[Subcube], group: &[CubeSym]) -> Vec<Subcube> {
    group
        .iter()
        .map(|g| {
            cover_key(
                &blockers
                    .iter()
                    .map(|b| g.map_subcube(b))
                    .collect::<Vec<_>>(),
            )
        })
        .min()
        .unwrap_or_else(|| cover_key(blockers))
}

/// **The `Bₙ`-canonical form of a cover, and its orbit size.** Acts the group on the *whole* cover by
/// `g·C = { g.map_subcube(b) }` and returns the lexicographically least sorted-blocker key together with
/// the number of distinct covers in the orbit. The canonical key is constant on an orbit and distinct
/// across orbits, a sound orbit invariant; the orbit size feeds the orbit–stabilizer identity
/// `|Stab| · orbit_size = |Bₙ|`.
pub fn canonical_cover(cover: &Cover, generators: &[CubeSym]) -> (Vec<Subcube>, usize) {
    let group = group_closure(generators, cover.n);
    let images: BTreeSet<Vec<Subcube>> = group
        .iter()
        .map(|g| {
            cover_key(
                &cover
                    .blockers
                    .iter()
                    .map(|b| g.map_subcube(b))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let best = images
        .iter()
        .next()
        .cloned()
        .unwrap_or_else(|| cover_key(&cover.blockers));
    (best, images.len())
}

/// **Enumerate every minimal UNSAT cover of the `n`-cube, one canonical representative per `Bₙ` orbit.**
/// A minimal cover is a set of subcube blockers that covers every corner (UNSAT) and *none of which is
/// droppable* (every blocker privately owns some corner) — i.e. a minimal unsatisfiable CNF (an MUS),
/// the irreducible atom of the UNSAT universe. Branch on the lex-least uncovered corner: it lies in
/// exactly `2ⁿ` subcubes (one per support `care ⊆ [n]`, with `value = corner & care`); recurse on each.
/// **Monotone pruning** kills a branch the moment any chosen blocker becomes fully redundant (it can
/// never recover a private corner once one is stolen). A leaf is total ∧ fully essential; leaves are
/// folded to their [`canonical_cover`] key so each orbit is reported once. (Exhaustive; for small `n`.)
pub fn minimal_cover_orbits(n: usize) -> Vec<Cover> {
    let generators = hyperoctahedral_generators(n);
    let group = group_closure(&generators, n); // the materialized Bₙ, built once and reused per node
    let mut orbits: HashMap<Vec<Subcube>, Cover> = HashMap::new();
    // **Orderly generation.** Visit each `Bₙ`-equivalence-class of partial covers exactly once, keyed
    // by its canonical form: expanding any representative of a class yields the same child classes (the
    // lex-least-uncovered corner and its covering subcubes commute with the group action), so one
    // representative per class still reaches every orbit leaf. This collapses both the branch-order
    // duplication and the up-to-`|Bₙ|` symmetric copies that otherwise make the raw tree intractable.
    let mut visited: BTreeSet<Vec<Subcube>> = BTreeSet::new();
    let mut chosen: Vec<Subcube> = Vec::new();
    enumerate_minimal_covers(n, &mut chosen, &group, &mut visited, &mut orbits);
    let mut out: Vec<Cover> = orbits.into_values().collect();
    out.sort_by(|a, b| cover_key(&a.blockers).cmp(&cover_key(&b.blockers)));
    out
}

/// A blocker is *redundant* in a partial cover when every corner it blocks is also blocked by some
/// other chosen blocker — it owns no private corner. Used both for monotone pruning and the leaf
/// minimality check.
fn blocker_is_redundant(blockers: &[Subcube], i: usize) -> bool {
    blockers[i].footprint().iter().all(|&c| {
        blockers
            .iter()
            .enumerate()
            .any(|(j, b)| j != i && b.covers(c))
    })
}

fn enumerate_minimal_covers(
    n: usize,
    chosen: &mut Vec<Subcube>,
    group: &[CubeSym],
    visited: &mut BTreeSet<Vec<Subcube>>,
    orbits: &mut HashMap<Vec<Subcube>, Cover>,
) {
    // Monotone minimality: if any chosen blocker is already fully redundant, no extension is minimal.
    if (0..chosen.len()).any(|i| blocker_is_redundant(chosen, i)) {
        return;
    }
    // Orderly generation: skip this partial if a symmetric copy (same canonical class) was already
    // expanded — every orbit it could reach is reachable from that copy.
    let canon = canonical_key(chosen, group);
    if !visited.insert(canon.clone()) {
        return;
    }
    let cover = Cover {
        n,
        blockers: chosen.clone(),
    };
    match cover.escaping_corner() {
        None => {
            // Total. By the monotone check above, every blocker is essential — a genuine minimal cover.
            // Its canonical key is the partial's canonical key (already computed).
            orbits.entry(canon).or_insert(cover);
        }
        Some(c) => {
            // Branch over the subcubes that contain the lex-least uncovered corner `c`: a subcube
            // contains `c` iff `value = c & care`, and every `care ⊆ [n]` is an integer in `0..2ⁿ`. We
            // start at `care = 1` to exclude the degenerate `care = 0` whole-cube blocker — the empty
            // clause `⊥`, which carries no variable structure and is the *only* minimal cover it can
            // belong to (it makes every other blocker redundant). The census is over genuine CNF: every
            // clause mentions at least one variable.
            for care in 1u64..(1u64 << n) {
                let blocker = Subcube {
                    n,
                    care,
                    value: c & care,
                };
                if !chosen.contains(&blocker) {
                    chosen.push(blocker);
                    enumerate_minimal_covers(n, chosen, group, visited, orbits);
                    chosen.pop();
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/hypercube/tests.rs"]
mod tests;
