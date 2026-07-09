//! **The small-`n` SAT-space census** — the measurement instrument.
//!
//! Brute-force over *every* minimal UNSAT formula (= every minimal subcube cover, the irreducible MUS
//! atoms of the UNSAT universe) for a given `n`, one representative per hyperoctahedral (`Bₙ`) orbit, and
//! attach to each orbit the full menu of invariants the solver stack can read off it: its symmetry group
//! (orbit size + stabilizer), its face vector, its minimum resolution width, where it sits on the
//! certified proof-complexity ladder ([`ProofRung`] / [`Shadow`]), and which engine the full structured
//! router ([`crate::solve::solve_structured`]) actually decides it with.
//!
//! The point is the **audit**: the certified diagnostic ladder ([`weakest_crushing_rung`],
//! [`diagnose`]) and the full router can *disagree* about a cover — e.g. the router crushes a mod-`p`
//! germ with the polynomial GF(`p`) engine ([`crate::solve::Route::ModP`]) while the certified ladder,
//! which has no mod-`p` rung, sees only a high-degree GF(2) Nullstellensatz. Every such disagreement is a
//! concrete, located gap between *what the solver can do* and *what the certified cascade can prove
//! cheaply.* The census enumerates them exhaustively at small `n`.

use std::collections::BTreeMap;

use crate::affine::{all_affine_bijections, Affine};
use crate::cdcl::Lit;
use crate::hypercube::{
    canonical_cover, cube_group_closure, diagnose, face_vector, hyperoctahedral_generators,
    min_resolution_width, minimal_cover_orbits, weakest_crushing_rung,
    weakest_crushing_rung_with_char, Cover, ProofRung, Shadow,
};
use crate::solve::{solve_structured, Answer, Route};

/// One `Bₙ`-orbit of minimal UNSAT formulas over `n` variables, with every invariant the solver can
/// compute about it. The raw measurements; the audit flags ([`OrbitRecord::router_beats_ladder`],
/// `OrbitRecord::finder_is_incomplete`) are derived from them.
#[derive(Clone, Debug)]
pub struct OrbitRecord {
    pub n: usize,
    /// A canonical representative cover of the orbit.
    pub rep: Cover,
    pub num_clauses: usize,
    /// Number of distinct covers in the `Bₙ` orbit.
    pub orbit_size: usize,
    /// `|Stab| = |Bₙ| / orbit_size` — how symmetric this formula is.
    pub stabilizer_order: usize,
    /// Blockers per face dimension — a geometric invariant of the cover.
    pub face_vector: BTreeMap<usize, usize>,
    /// Minimum width of any resolution refutation.
    pub min_res_width: usize,
    /// The weakest *certified* cut on the proof-complexity ladder (GF(2) Nullstellensatz at the top).
    pub rung: ProofRung,
    /// The single-label certified shadow the diagnoser reads off (`Counting`/`Parity`/`CuttingPlanes`).
    pub shadow: Option<Shadow>,
    /// Which engine the full structured router actually decides it with.
    pub route: Route,
    /// Self-driving rule-symmetry: how many essentially-distinct clauses the *discovered* automorphisms
    /// leave (the production symmetry breaker, `symmetry_detect::find_generators`, used in the cascade).
    pub discovered_rule_orbits: usize,
    /// The same count under the cover's **full** `Bₙ` stabilizer (every geometric automorphism, computed
    /// exhaustively from the group). This is the strongest symmetry break achievable. When
    /// `discovered_rule_orbits > full_rule_orbits`, the production breaker is leaving symmetry unbroken —
    /// a case the solver could collapse further but currently does not.
    pub full_rule_orbits: usize,
    /// Is the UNSAT explained by an inconsistent GF(2) linear system (the parity shadow)?
    pub affine_explained: bool,
    /// Did the structured router crush it with the polynomial mod-`p` (GF(`p`)) engine?
    pub modp_routed: bool,
}

impl OrbitRecord {
    /// **Gap signal.** The full router crushed this cover with a *polynomial specialist* the certified
    /// proof-complexity ladder has no rung for — the router sees structure the certified cascade can only
    /// reach by an expensive general proof. The clearest case: a mod-`p` germ routed to `ModP` while the
    /// ladder reports a (GF(2)) `Nullstellensatz` height. This is the unification gap, per orbit.
    pub fn router_beats_ladder(&self) -> bool {
        matches!(self.route, Route::ModP | Route::Collapse | Route::HybridXor)
            && !matches!(self.rung, ProofRung::Counting | ProofRung::Parity)
    }

    /// **Symmetry left on the table.** The production breaker discovered strictly fewer rule-merges than
    /// the cover's full geometric stabilizer admits — so the symmetry-breaking predicate it injects is
    /// weaker than achievable, and a scaled-up member of this family will cost the solver search it could
    /// have avoided. Every such orbit is a concrete "break more symmetry → solve more cases" target.
    pub fn symmetry_underbroken(&self) -> bool {
        self.discovered_rule_orbits > self.full_rule_orbits
    }
}

/// The orbit's ladder placement under the **characteristic-extended cascade**
/// ([`weakest_crushing_rung_with_char`]), recomputed from the representative cover. The audit-gap
/// closer: an orbit the router crushes via its mod-`p` specialist while the legacy ladder pays an
/// algebraic price ([`OrbitRecord::router_beats_ladder`]) lands on its `ModCount { p }` rung here;
/// every cover outside the mod-`p` population is placed exactly as the legacy ladder places it.
pub fn extended_rung(rec: &OrbitRecord, primes: &[u64]) -> ProofRung {
    weakest_crushing_rung_with_char(rec.n, &rec.rep.clauses(), rec.n, primes)
}

/// **The hardness spectrum of the `n`-variable SAT hypercube** — the durable "how much do we cover, and
/// where is the hard core" map, aggregated from [`census`]. Each minimal-UNSAT family is placed on the
/// certified proof-complexity ladder by its *weakest crushing rung* (the cheapest proof system that
/// refutes it), and split by whether it carries `Bₙ` symmetry to break. The `Nullstellensatz` rung is
/// further resolved by minimum degree — the algebraic-hardness dial — so the spectrum shows not just
/// *that* the residue is algebraic but *how deep* it sits.
#[derive(Clone, Debug)]
pub struct CoverageSummary {
    pub n: usize,
    pub orbits: usize,
    /// Family count per proof rung: `trivial` (unit propagation), `counting`, `parity`,
    /// `nullstellensatz-d{degree}`, `beyond-budget`. The rung is exactly which proof system covers it.
    pub by_rung: BTreeMap<String, usize>,
    /// The deepest minimum-Nullstellensatz degree at this `n` — the algebraic-hardness ceiling.
    pub max_ns_degree: usize,
    /// Families with a non-trivial `Bₙ` stabilizer — symmetry there is to break.
    pub structured: usize,
    /// Families with trivial stabilizer — the rigid residue, no symmetry shortcut.
    pub rigid: usize,
    /// Distribution of minimum resolution width across the families.
    pub by_resolution_width: BTreeMap<usize, usize>,
}

/// Aggregate the [`census`] into the [`CoverageSummary`] hardness spectrum.
pub fn coverage_summary(n: usize) -> CoverageSummary {
    let mut by_rung: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_resolution_width: BTreeMap<usize, usize> = BTreeMap::new();
    let (mut max_ns_degree, mut structured, mut rigid) = (0usize, 0usize, 0usize);
    let records = census(n);
    for r in &records {
        let label = match r.rung {
            ProofRung::Trivial => "trivial".to_string(),
            ProofRung::Counting => "counting".to_string(),
            ProofRung::Parity => "parity".to_string(),
            ProofRung::ModCount { p } => format!("modcount-p{p}"),
            ProofRung::Nullstellensatz { min_degree } => {
                max_ns_degree = max_ns_degree.max(min_degree);
                format!("nullstellensatz-d{min_degree}")
            }
            ProofRung::BeyondBudget => "beyond-budget".to_string(),
        };
        *by_rung.entry(label).or_insert(0) += 1;
        *by_resolution_width.entry(r.min_res_width).or_insert(0) += 1;
        if r.stabilizer_order > 1 {
            structured += 1;
        } else {
            rigid += 1;
        }
    }
    CoverageSummary { n, orbits: records.len(), by_rung, max_ns_degree, structured, rigid, by_resolution_width }
}

/// **The residue map** — the honest "here's what we crush, and here's the wall" partition of the census.
/// Every minimal-UNSAT family lands in exactly one of: *crushed* (a structural specialist route decided it —
/// parity, mod-`p`, counting, a symmetry route, SoS, …), or the *residue* (routed to `Incompressible`/`Cdcl`
/// — no lens caught it). Within those, `targetable` are the families still carrying unbroken symmetry (break
/// more → crush cheaper), and `rigid_core` ⊆ residue are the families with *no* symmetry to break at all —
/// the incompressible wall the counting bound predicts. This is a search *with structure*: not random probing,
/// but a ranked pass of every lens over every family, surfacing exactly which ones resist and why.
#[derive(Clone, Debug)]
pub struct ResidueMap {
    pub n: usize,
    pub total: usize,
    /// A structural specialist route decided it (route ∉ {Incompressible, Cdcl}).
    pub crushed: usize,
    /// No lens caught it — fell through to CDCL / the certified "no shortcut" verdict. The hard core.
    pub residue: usize,
    /// Families with symmetry left unbroken (`discovered_rule_orbits > full_rule_orbits`) — the concrete
    /// "break more symmetry to crush this" targets. `0` means the symmetry breaking is *complete*.
    pub targetable: usize,
    /// Residue families that are also rigid (trivial `Bₙ` stabilizer) — no symmetry exists to break, so the
    /// wall here is intrinsic, not a gap in our breaking.
    pub rigid_core: usize,
    /// The deepest minimum-Nullstellensatz degree among the residue — how algebraically deep the wall sits.
    pub core_max_ns_degree: usize,
}

/// The set of falsifying assignments of a clause, as a bitmask over the `2ⁿ` points of `𝔽₂ⁿ` (`n ≤ 6`). A
/// clause `C` is falsified by `x` iff every literal is false: `+v` ⟹ `x_v = 0`, `¬v` ⟹ `x_v = 1`.
fn falsify_set(clause: &[Lit], num_vars: usize) -> u64 {
    let mut set = 0u64;
    for x in 0..(1u64 << num_vars) {
        if clause.iter().all(|lit| {
            let bit = (x >> (lit.var() as u64)) & 1;
            if lit.is_positive() { bit == 0 } else { bit == 1 }
        }) {
            set |= 1 << x;
        }
    }
    set
}

/// Push a point-set through an affine map `φ`: `{φ(p) : p ∈ s}`.
fn map_point_set(s: u64, phi: &Affine, num_vars: usize) -> u64 {
    let mut out = 0u64;
    for x in 0..(1u64 << num_vars) {
        if (s >> x) & 1 == 1 {
            out |= 1 << phi.apply(x);
        }
    }
    out
}

/// Recover the clause whose falsifying-point set is exactly `s`, or `None` if `s` is not an axis-aligned
/// subcube (a shear can map a clause's cube to a non-cube — precisely the images that are not CNFs). A
/// coordinate is *fixed* iff every point of `s` agrees on it; `s` is a subcube iff it equals the full
/// cylinder over the fixed pattern (`|s| = 2^{free}` with every matching point present). The literal on a
/// fixed coordinate `i` with value `v` is the one falsified there: positive when `v = 0`, negative when
/// `v = 1` (the inverse of [`falsify_set`]).
fn pointset_to_clause(s: u64, num_vars: usize) -> Option<Vec<Lit>> {
    if s == 0 {
        return None;
    }
    let points: Vec<u64> = (0..(1u64 << num_vars)).filter(|&x| (s >> x) & 1 == 1).collect();
    let mut clause = Vec::new();
    let mut free = 0usize;
    for i in 0..num_vars as u64 {
        let first = (points[0] >> i) & 1;
        if points.iter().all(|&p| (p >> i) & 1 == first) {
            clause.push(Lit::new(i as u32, first == 0)); // fixed: positive iff the fixed value is 0
        } else {
            free += 1;
        }
    }
    // A genuine subcube: the fixed pattern spans exactly `2^{free}` points and `s` holds all of them.
    if points.len() != (1usize << free) {
        return None;
    }
    Some(clause)
}

/// The image of a CNF under an affine map `φ`, or `None` if any clause's cube maps to a non-cube (so the
/// image is not a CNF). Each clause's falsifying-point set is pushed through `φ` and read back as a clause.
fn agl_image_formula(clauses: &[Vec<Lit>], phi: &Affine, num_vars: usize) -> Option<Vec<Vec<Lit>>> {
    clauses
        .iter()
        .map(|c| pointset_to_clause(map_point_set(falsify_set(c, num_vars), phi, num_vars), num_vars))
        .collect()
}

/// The image of a clause under the transvection `σ : x_i ↦ x_i ⊕ x_j`, computed **symbolically** (`O(clause)`
/// — no `2ⁿ` point set), or `None` if `σ` maps the clause's blocker subcube to a non-subcube (so `σ` is not a
/// clause automorphism). Derivation: `σ` is an involution, so the blocker constraint `x_i = c_i` becomes
/// `y_i ⊕ y_j = c_i`, which stays axis-aligned iff `y_j` is fixed (`j ∈ support`), giving the new value
/// `c_i ⊕ c_j` at `i`. So the image *is* the clause, except: when `i ∈ support` and `j ∉ support` it is not a
/// subcube (`None`); and when both `i, j ∈ support` the literal at `i` flips iff `j`'s literal is negative.
fn transvection_image_clause(clause: &[Lit], i: u32, j: u32) -> Option<Vec<Lit>> {
    let lit_i = clause.iter().find(|l| l.var() == i);
    let lit_j = clause.iter().find(|l| l.var() == j);
    match (lit_i, lit_j) {
        (Some(_), None) => None,
        (Some(_), Some(lj)) if !lj.is_positive() => {
            Some(clause.iter().map(|l| if l.var() == i { l.negated() } else { *l }).collect())
        }
        _ => Some(clause.to_vec()),
    }
}

/// A canonical, order-/duplicate-independent key for a clause set.
fn clause_set_key(clauses: &[Vec<Lit>]) -> std::collections::BTreeSet<Vec<(u32, bool)>> {
    clauses
        .iter()
        .map(|c| {
            let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive())).collect();
            k.sort_unstable();
            k
        })
        .collect()
}

/// **A scalable, `∀n` affine-symmetry finder — the transvection generators, in polynomial time.** Exhaustive
/// `AGL(n,2)` enumeration is stuck at `n ≤ 4`; this finds every *transvection* `x_i ↦ x_i ⊕ x_j` that is an
/// affine automorphism of the clause set, in `O(n² · |clauses|)` time via [`transvection_image_clause`] — no
/// `2ⁿ` anywhere, so it runs at any `n`. Sound: every returned pair is a genuine automorphism (the produced
/// clause set is checked equal to the original). *Incomplete* by design: an affine symmetry realized only by a
/// *composite* of transvections (e.g. parity's shears, products of two transvections) is not a single
/// generator and is not returned — completeness for general formulas is a hard affine-equivalence problem, and
/// the closed form ([`crate::affine::affine_subspace_agl_order`]) is the route for structured families.
pub fn affine_transvection_generators(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<(u32, u32)> {
    let original = clause_set_key(clauses);
    let mut gens = Vec::new();
    for i in 0..num_vars as u32 {
        for j in 0..num_vars as u32 {
            if i == j {
                continue;
            }
            let image: Option<Vec<Vec<Lit>>> =
                clauses.iter().map(|c| transvection_image_clause(c, i, j)).collect();
            if let Some(img) = image {
                if clause_set_key(&img) == original {
                    gens.push((i, j));
                }
            }
        }
    }
    gens
}

/// The image of a clause under a **composite shear** `x_i ↦ x_i ⊕ x_j for every i ∈ targets` (with `j ∉
/// targets`) — a product of transvections that all share the source `j`, hence commute (disjoint
/// destinations), so their action is the sequential composition of [`transvection_image_clause`]. `None` if
/// any stage leaves the subcube world.
fn composite_shear_image_clause(clause: &[Lit], targets: &[u32], j: u32) -> Option<Vec<Lit>> {
    let mut c = clause.to_vec();
    for &i in targets {
        c = transvection_image_clause(&c, i, j)?;
    }
    Some(c)
}

/// All size-`k` subsets of `pool` (small `k`; `pool` small).
fn combinations(pool: &[u32], k: usize) -> Vec<Vec<u32>> {
    if k == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for (idx, &x) in pool.iter().enumerate() {
        for mut rest in combinations(&pool[idx + 1..], k - 1) {
            rest.insert(0, x);
            out.push(rest);
        }
    }
    out
}

/// **A bounded-composite affine-symmetry finder — climbing the "single-transvection wall" instead of
/// declaring it.** [`affine_transvection_generators`] only sees *depth-1* shears (a single `x_i ↦ x_i ⊕ x_j`),
/// so it is blind to symmetries realized only as a *product* of transvections — the parity/linear shears are
/// the canonical example ("add `x_j` to `x_i` **and** `x_k`" preserves `⊕ x = b` because it adds `x_j` to the
/// form twice `= 0`; neither half does). This extends the finder to shears of composite depth `≤
/// max_targets`: for every source `j` and every target set `S ⊆ vars∖{j}` with `1 ≤ |S| ≤ max_targets`, the
/// commuting product "add `x_j` to each `x_i ∈ S`". At `max_targets = 1` it *is* the transvection finder; at
/// `max_targets = 2` it catches the parity shears I earlier called out of reach. Polynomial for fixed depth
/// (`O(n^{max_targets+1}·|clauses|)`), sound (every returned generator is verified to permute the clause set),
/// and still incomplete only at *unbounded* depth — but the depth is now a tunable window, and the composite
/// depth a formula's symmetries require is itself a complexity measure. Returns each generator as `(S, j)`.
pub fn affine_composite_shear_generators(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    max_targets: usize,
) -> Vec<(Vec<u32>, u32)> {
    let original = clause_set_key(clauses);
    let mut gens = Vec::new();
    for j in 0..num_vars as u32 {
        let others: Vec<u32> = (0..num_vars as u32).filter(|&v| v != j).collect();
        for size in 1..=max_targets.min(others.len()) {
            for combo in combinations(&others, size) {
                let image: Option<Vec<Vec<Lit>>> =
                    clauses.iter().map(|c| composite_shear_image_clause(c, &combo, j)).collect();
                if let Some(img) = image {
                    if clause_set_key(&img) == original {
                        gens.push((combo, j));
                    }
                }
            }
        }
    }
    gens
}

/// The image of a clause under the **general rank-1 GF(2) involution** `M_{u,v} : x ↦ x ⊕ (v·x)·u` (masks `u,
/// v`; a bijection iff `u·v = 0`), or `None` if `M` sends the clause's blocker subcube to a non-subcube. This
/// is the affine shear generalized: the shear `x_i ↦ x_i⊕x_j for i∈S` is `M_{u=1_S, v=e_j}`, and the
/// **symplectic transvection** `T_w = I⊕w wᵀ` is `M_{u=w, v=w}`.
///
/// A clause's blocker is the affine subcube `{y : y_i = c_i, i∈T}` (`c_i = 0` for a positive literal at `i`,
/// `1` for a negative; `T = supp`). Since `M` is an involution, `M(blocker) = {y : (M y)_i = c_i, i∈T}`, i.e.
/// the linear system with rows `(e_i ⊕ u_i·v)` and right-hand sides `c_i` — because row `i` of `M = I⊕uvᵀ` is
/// `e_i ⊕ u_i v`. Row-reduce over `GF(2)`: the image is a clause iff the reduced system is *axis-aligned*
/// (every pivot row a single coordinate), and then each reduced row `y_k = b` is the literal on `k`. A rank-1
/// map can re-align a clause onto a *different* support (that is exactly why the naive support-preserving rule
/// is wrong); the row reduction handles it. Cost `O(|T|²)`, no `2ⁿ`. Cross-checked against the exhaustive
/// point-set image in `rank1_symbolic_image_matches_the_pointset_computation`.
fn rank1_image_clause(clause: &[Lit], u: u64, v: u64) -> Option<Vec<Lit>> {
    // RREF of the constraint rows (mask, rhs) over GF(2), pivoting on the highest set bit.
    let mut pivots: Vec<(u64, u8)> = Vec::new();
    for l in clause {
        let i = l.var();
        let mut row = (1u64 << i) ^ (if (u >> i) & 1 == 1 { v } else { 0 });
        let mut rhs: u8 = if l.is_positive() { 0 } else { 1 };
        for &(prow, prhs) in &pivots {
            let pbit = 1u64 << (63 - prow.leading_zeros());
            if row & pbit != 0 {
                row ^= prow;
                rhs ^= prhs;
            }
        }
        if row == 0 {
            if rhs != 0 {
                return None; // inconsistent (empty image — cannot happen for a bijection, guarded anyway)
            }
            continue;
        }
        let nbit = 1u64 << (63 - row.leading_zeros());
        for p in pivots.iter_mut() {
            if p.0 & nbit != 0 {
                p.0 ^= row;
                p.1 ^= rhs;
            }
        }
        pivots.push((row, rhs));
    }
    let mut lits = Vec::with_capacity(pivots.len());
    for (row, rhs) in pivots {
        if row.count_ones() != 1 {
            return None; // reduced constraint couples two coordinates — image is not axis-aligned
        }
        lits.push(Lit::new(row.trailing_zeros(), rhs == 0));
    }
    Some(lits)
}

/// Every even-weight mask over `num_vars` bits with weight in `2..=max_weight` (the symplectic-transvection
/// candidates: `w·w = weight(w) mod 2 = 0`).
fn even_weight_masks(num_vars: usize, max_weight: usize) -> Vec<u64> {
    let pool: Vec<u32> = (0..num_vars as u32).collect();
    let mut out = Vec::new();
    let mut k = 2;
    while k <= max_weight.min(num_vars) {
        for combo in combinations(&pool, k) {
            out.push(combo.iter().fold(0u64, |m, &b| m | (1u64 << b)));
        }
        k += 2;
    }
    out
}

/// **The symplectic-transvection symmetry finder — one rung above affine shears.** An affine shear is a rank-1
/// map whose right factor is a *unit* vector (`v = e_j`); dropping that restriction to a general `v` gives the
/// full rank-1 group, whose bijective involutions are the **symplectic transvections** `T_w = I ⊕ w wᵀ` (even
/// weight `w`). Where the affine dichotomy of §4 found permutation-symmetric families (PHP, mod-counting) to be
/// affine-shear-*rigid*, this finder can still see their higher symmetry: it enumerates every even-weight `w`
/// with `2 ≤ |w| ≤ max_weight` and returns those whose `T_w` permutes the clause set (verified via
/// [`rank1_image_clause`]). Cost `O(Σ_{k≤W even} C(n,k) · |clauses|)` — polynomial for fixed weight. Sound
/// (every returned `w` is a checked automorphism); complete only up to the chosen weight. The *weight* of the
/// smallest such symmetry is the graded invariant we track against NS degree.
pub fn symplectic_transvection_generators(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    max_weight: usize,
) -> Vec<u64> {
    let original = clause_set_key(clauses);
    let mut gens = Vec::new();
    for w in even_weight_masks(num_vars, max_weight) {
        let image: Option<Vec<Vec<Lit>>> =
            clauses.iter().map(|c| rank1_image_clause(c, w, w)).collect();
        if let Some(img) = image {
            if clause_set_key(&img) == original {
                gens.push(w);
            }
        }
    }
    gens
}

/// **The clause-level AGL symmetry detector** — the affine analog of the `Bₙ` stabilizer, and the one
/// symmetry lens that works on UNSAT formulas (the model-based detector is vacuous — no models). An affine
/// map `φ : x ↦ Ax ⊕ b` (`φ ∈ AGL(n,2)`) is a clause-set automorphism iff, acting on `𝔽₂ⁿ`, it *permutes the
/// falsifying-point sets of the clauses*. Since `Bₙ ⊆ AGL(n,2)`, `|Aut_AGL| ≥ |Bₙ-stabilizer|`; a `Bₙ`-rigid
/// core with `|Aut_AGL| > 1` has **hidden affine symmetry** (breakable by an affine SBP); `|Aut_AGL| = 1`
/// means it is affine-rigid — structure-minimal at the affine level too. Brute over `AGL(n,2)`, feasible
/// `n ≤ 4`.
pub fn clause_agl_symmetries(num_vars: usize, clauses: &[Vec<Lit>]) -> usize {
    let blockers: std::collections::HashSet<u64> =
        clauses.iter().map(|c| falsify_set(c, num_vars)).collect();
    all_affine_bijections(num_vars)
        .into_iter()
        .filter(|phi| blockers.iter().all(|&s| blockers.contains(&map_point_set(s, phi, num_vars))))
        .count()
}

/// The sorted falsifying-point-set masks of a cover's clauses — the cover as a set of subcubes in point
/// space, the representation the affine group acts on directly.
fn blocker_masks(clauses: &[Vec<Lit>], num_vars: usize) -> Vec<u64> {
    let mut m: Vec<u64> = clauses.iter().map(|c| falsify_set(c, num_vars)).collect();
    m.sort_unstable();
    m.dedup();
    m
}

/// The point-permutation table of an affine map: `table[x] = φ(x)` over the `2ⁿ` corners. Precomputed once
/// so the group action on point-set masks is fast bit-arithmetic rather than a fresh `apply` per point.
fn perm_table(phi: &Affine, num_vars: usize) -> Vec<u32> {
    (0..(1u64 << num_vars)).map(|x| phi.apply(x) as u32).collect()
}

/// Push a point-set mask through a precomputed permutation table: `{table[x] : x ∈ s}`.
fn map_mask(s: u64, table: &[u32]) -> u64 {
    let mut out = 0u64;
    let mut bits = s;
    while bits != 0 {
        let x = bits.trailing_zeros() as usize;
        out |= 1u64 << table[x];
        bits &= bits - 1;
    }
    out
}

/// The canonical form of a point-set collection under a group of affine maps (given as permutation tables):
/// the lexicographically minimal sorted image. Over the full `AGL(n,2)` this is the affine-orbit invariant —
/// two CNFs are AGL-equivalent iff their blocker collections share it (the minimal image need not itself be a
/// CNF; it is only a label) — and over the `Bₙ` subgroup it is the signed-permutation canonical form.
fn canonical_over_tables(masks: &[u64], tables: &[Vec<u32>]) -> Vec<u64> {
    let mut best: Option<Vec<u64>> = None;
    for t in tables {
        let mut img: Vec<u64> = masks.iter().map(|&s| map_mask(s, t)).collect();
        img.sort_unstable();
        match &best {
            Some(b) if *b <= img => {}
            _ => best = Some(img),
        }
    }
    best.unwrap_or_default()
}

/// Whether an affine map's linear part is a permutation matrix (exactly one set bit per row, columns a
/// permutation) — i.e. the map is a signed permutation, an element of `Bₙ`.
fn is_permutation_matrix(matrix: &[u64], num_vars: usize) -> bool {
    let mut cols = 0u64;
    for row in matrix.iter().take(num_vars) {
        if row.count_ones() != 1 || cols & row != 0 {
            return false;
        }
        cols |= row;
    }
    cols == (1u64 << num_vars) - 1
}

/// The `Bₙ` subgroup of `AGL(n,2)`: the affine maps whose linear part is a permutation matrix (signed
/// permutations = variable permutations × literal negations). `|Bₙ| = n!·2ⁿ`.
fn bn_affines(num_vars: usize) -> Vec<Affine> {
    all_affine_bijections(num_vars).into_iter().filter(|a| is_permutation_matrix(&a.matrix, num_vars)).collect()
}

/// The elementary transvections `xᵢ ↦ xᵢ ⊕ xⱼ` (`i ≠ j`): the generators of `GL(n,2)` beyond the
/// permutation matrices. Together with `Bₙ` they generate all of `AGL(n,2)`, so a single transvection is
/// the smallest affine move outside the signed-permutation lens.
fn transvections(num_vars: usize) -> Vec<Affine> {
    let mut out = Vec::new();
    for i in 0..num_vars {
        for j in 0..num_vars {
            if i == j {
                continue;
            }
            let mut matrix: Vec<u64> = (0..num_vars).map(|k| 1u64 << k).collect();
            matrix[i] |= 1u64 << j;
            out.push(Affine { n: num_vars, matrix, translation: 0 });
        }
    }
    out
}

/// **The AGL collapse of the census** — how far the affine group `AGL(n,2) ⊋ Bₙ` merges the `Bₙ`-orbit
/// count. Because the certified family is AGL-invariant (proved), the AGL census carries the *same* family
/// tower — just fewer classes: more symmetry, less to cover.
#[derive(Clone, Debug)]
pub struct AglCollapse {
    pub n: usize,
    /// The `Bₙ`-orbit count (the census's covering-class count).
    pub bn_orbits: usize,
    /// The `AGL(n,2)`-class count after the affine lens merges affine-equivalent orbits.
    pub agl_classes: usize,
}

impl AglCollapse {
    /// The collapse factor `bn_orbits / agl_classes` — how many `Bₙ` orbits the affine lens fuses per class.
    pub fn factor(&self) -> f64 {
        self.bn_orbits as f64 / self.agl_classes.max(1) as f64
    }
}

/// **The exact AGL collapse** (`n ≤ 3`). Canonicalize every `Bₙ`-orbit representative under the full affine
/// group in point-set space (`affine_canonical_form`); the number of distinct canonical forms is the exact
/// AGL-class count. Brute over `|AGL(n,2)|` per orbit, so bounded to `n ≤ 3`.
pub fn agl_collapse_exact(n: usize) -> AglCollapse {
    let tables: Vec<Vec<u32>> = all_affine_bijections(n).iter().map(|p| perm_table(p, n)).collect();
    let mut classes: std::collections::BTreeSet<Vec<u64>> = std::collections::BTreeSet::new();
    let mut bn_orbits = 0usize;
    for cover in minimal_cover_orbits(n) {
        bn_orbits += 1;
        classes.insert(canonical_over_tables(&blocker_masks(&cover.clauses(), n), &tables));
    }
    AglCollapse { n, bn_orbits, agl_classes: classes.len() }
}

/// **The AGL collapse via CNF-preserving affine moves** — scales to `n = 4`. Since `AGL = ⟨Bₙ,
/// transvections⟩`, uniting the `Bₙ` orbits connected by a single CNF-preserving transvection (its image
/// stays a valid clause set, then `Bₙ`-canonicalized) merges genuine affine equivalences. The component
/// count is an *upper bound* on the AGL-class count — every union is a real affine equivalence; only merges
/// that must route through a non-CNF affine intermediate are missed — hence a *rigorous lower bound* on the
/// collapse, exact when the walk is tight (checked against [`agl_collapse_exact`] at `n = 3`).
pub fn agl_collapse_via_transvections(n: usize) -> AglCollapse {
    let reps: Vec<Vec<u64>> =
        minimal_cover_orbits(n).into_iter().map(|c| blocker_masks(&c.clauses(), n)).collect();
    let bn_tables: Vec<Vec<u32>> = bn_affines(n).iter().map(|p| perm_table(p, n)).collect();
    let tau_tables: Vec<Vec<u32>> = transvections(n).iter().map(|p| perm_table(p, n)).collect();
    let mut key_to_idx: std::collections::HashMap<Vec<u64>, usize> = std::collections::HashMap::new();
    for (i, masks) in reps.iter().enumerate() {
        key_to_idx.insert(canonical_over_tables(masks, &bn_tables), i);
    }
    let mut parent: Vec<usize> = (0..reps.len()).collect();
    fn find(parent: &mut [usize], x: usize) -> usize {
        let mut root = x;
        while parent[root] != root {
            root = parent[root];
        }
        let mut cur = x;
        while parent[cur] != cur {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }
    for (i, masks) in reps.iter().enumerate() {
        for tau in &tau_tables {
            let mut img: Vec<u64> = Vec::with_capacity(masks.len());
            let mut ok = true;
            for &s in masks {
                let t = map_mask(s, tau);
                if pointset_to_clause(t, n).is_none() {
                    ok = false;
                    break;
                }
                img.push(t);
            }
            if !ok {
                continue;
            }
            img.sort_unstable();
            img.dedup();
            if let Some(&j) = key_to_idx.get(&canonical_over_tables(&img, &bn_tables)) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let agl_classes = (0..reps.len()).filter(|&i| find(&mut parent, i) == i).count();
    AglCollapse { n, bn_orbits: reps.len(), agl_classes }
}

/// An **auto-discovered affine equivalence** between two `Bₙ`-orbit representatives: a concrete
/// `φ ∈ AGL(n,2)` carrying one cover's blocker set onto the other's. The `Bₙ` census reports the two orbits
/// as distinct classes; this witness proves they are the SAME unsatisfiable problem up to an affine
/// coordinate change — the structure the signed-permutation lens could not see, made explicit and
/// re-checkable. Codifying the collapse this way turns a merge *count* into concrete affine symmetries.
#[derive(Clone, Debug)]
pub struct AglWitness {
    pub map: Affine,
}

impl AglWitness {
    /// Re-check (zero trust in the producer): pushing `from`'s blocker set through `map` yields exactly
    /// `to`'s blocker set.
    pub fn verify(&self, from: &[u64], to: &[u64], num_vars: usize) -> bool {
        let table = perm_table(&self.map, num_vars);
        let img: std::collections::BTreeSet<u64> = from.iter().map(|&s| map_mask(s, &table)).collect();
        img == to.iter().copied().collect()
    }
}

/// Search `AGL(n,2)` for an affine map carrying `from`'s blocker set onto `to`'s — the witness that two `Bₙ`
/// orbits are affine-equivalent, or `None` if they are genuinely affine-distinct. Exhaustive, so `n ≤ 4`.
pub fn find_agl_witness(from: &[u64], to: &[u64], num_vars: usize) -> Option<AglWitness> {
    if from.len() != to.len() {
        return None;
    }
    let target: std::collections::BTreeSet<u64> = to.iter().copied().collect();
    for phi in all_affine_bijections(num_vars) {
        let table = perm_table(&phi, num_vars);
        let img: std::collections::BTreeSet<u64> = from.iter().map(|&s| map_mask(s, &table)).collect();
        if img == target {
            return Some(AglWitness { map: phi });
        }
    }
    None
}

/// **Peek inside the generic full-degree cores.** They're `Bₙ`-rigid and full-degree — but are they one
/// undifferentiated blob, or do finer invariants split them into sub-families? Sub-classify the *distinct*
/// generic full-degree types by their coarser invariants `(shadow, min-res-width)`: returns
/// `(shadow, width) → number of distinct full-degree types`. If a few buckets hold most of the types, the
/// "generic" bucket has real sub-structure (more families); if they're spread thin, it's genuinely varied.
pub fn generic_subfamilies(n: usize) -> BTreeMap<(String, usize), usize> {
    let full = format!("nullstellensatz-d{n}");
    let mut seen: std::collections::BTreeSet<(String, usize, Vec<(usize, usize)>)> = std::collections::BTreeSet::new();
    let mut sub: BTreeMap<(String, usize), usize> = BTreeMap::new();
    for cover in minimal_cover_orbits(n) {
        let clauses = cover.clauses();
        if rung_label(&weakest_crushing_rung(n, &clauses, n)) != full {
            continue; // only the generic full-degree cores
        }
        let shadow = format!("{:?}", diagnose(n, &clauses).cut);
        let width = min_resolution_width(&cover).unwrap_or(usize::MAX);
        let fv: Vec<(usize, usize)> = face_vector(&cover).into_iter().collect();
        if seen.insert((shadow.clone(), width, fv)) {
            *sub.entry((shadow, width)).or_insert(0) += 1; // a NEW distinct generic type
        }
    }
    sub
}

/// Name the family a structural signature belongs to. The named combinatorial families are the *low-degree*
/// ones (Tseitin=parity, PHP=counting); a `degree-d` algebraic core with `d < n` is a *bounded-algebraic*
/// family; and a **full-degree** core (`d = n`) is the **generic** type — no low-degree shortcut, the opposite
/// of a nice family. So the matcher's verdict on a giant is: is it a low-degree named family, a bounded
/// algebraic one, or the generic full-degree core?
fn family_label(rung_label: &str, n: usize) -> String {
    match rung_label {
        "trivial" => "unit-propagation".into(),
        "counting" => "PHP / cardinality (counting)".into(),
        "parity" => "Tseitin / XOR (parity, poly via GF(2))".into(),
        s if s == format!("nullstellensatz-d{n}") => "GENERIC full-degree core (no low-degree shortcut)".into(),
        s => format!("bounded-algebraic ({s})"),
    }
}

/// **Name the giants.** The top-`k` morph-classes at `n` (by orbit count), each labeled with the family it
/// matches. Confirms which of the huge morph-classes are recognizable named families vs. the generic
/// full-degree core.
pub fn named_giants(n: usize, k: usize) -> Vec<(usize, String)> {
    let mut sigs: BTreeMap<(String, String, usize, Vec<(usize, usize)>), usize> = BTreeMap::new();
    for cover in minimal_cover_orbits(n) {
        let clauses = cover.clauses();
        let rl = rung_label(&weakest_crushing_rung(n, &clauses, n));
        let shadow = format!("{:?}", diagnose(n, &clauses).cut);
        let width = min_resolution_width(&cover).unwrap_or(usize::MAX);
        let fv: Vec<(usize, usize)> = face_vector(&cover).into_iter().collect();
        *sigs.entry((rl, shadow, width, fv)).or_insert(0) += 1;
    }
    let mut entries: Vec<_> = sigs.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.into_iter().take(k).map(|((rl, _, _, _), count)| (count, family_label(&rl, n))).collect()
}

/// **Label ALL structural types.** Every distinct signature (morph-class) at `n`, labeled with its family;
/// returns `(num_types, family → type-count)` — so we see how the *types* (not orbits) distribute across the
/// families. This is the full census of "what are the 403 types and where do they come from": each is a
/// low-degree named family, a bounded-algebraic one, or the generic full-degree core.
pub fn family_of_types(n: usize) -> (usize, BTreeMap<String, usize>) {
    let mut sig_label: BTreeMap<(String, String, usize, Vec<(usize, usize)>), String> = BTreeMap::new();
    for cover in minimal_cover_orbits(n) {
        let clauses = cover.clauses();
        let rl = rung_label(&weakest_crushing_rung(n, &clauses, n));
        let shadow = format!("{:?}", diagnose(n, &clauses).cut);
        let width = min_resolution_width(&cover).unwrap_or(usize::MAX);
        let fv: Vec<(usize, usize)> = face_vector(&cover).into_iter().collect();
        sig_label.entry((rl.clone(), shadow, width, fv)).or_insert_with(|| family_label(&rl, n));
    }
    let mut by_family: BTreeMap<String, usize> = BTreeMap::new();
    for label in sig_label.values() {
        *by_family.entry(label.clone()).or_insert(0) += 1;
    }
    (sig_label.len(), by_family)
}

/// The certified **family** an orbit belongs to — the proof-system that refutes it, named. The algebraic
/// family is degree-graded (`algebraic-d{k}`), so the family tower shows the degree-`n` wall directly.
fn family_name(rung: &ProofRung) -> String {
    match rung {
        ProofRung::Trivial => "unit-propagation".into(),
        ProofRung::Counting => "counting (pigeonhole/cardinality)".into(),
        ProofRung::Parity => "parity (XOR/Tseitin)".into(),
        ProofRung::ModCount { p } => format!("mod-{p} counting (one-hot GF({p}))"),
        ProofRung::Nullstellensatz { min_degree } => format!("algebraic-d{min_degree}"),
        ProofRung::BeyondBudget => "unclassified".into(),
    }
}

/// **The family growth across `n`** — for each `n ≤ max_n`: the number of distinct certified families, and
/// which families are *new* (absent at `n−1`). The finding: the cheap families (unit-propagation, parity,
/// counting) form a *fixed* menu that saturates early; from then on **each increase in `n` forces exactly one
/// new family — the degree-`n` algebraic family (`algebraic-d{n}`)** — because `max_ns_degree = n`, so a
/// degree-`n` certificate becomes *necessary* at `n` and was not at `n−1`. That is the forcing mechanism: the
/// degree wall. The family count is therefore `Θ(n)` (fixed cheap menu + one algebraic rung per `n`), climbing
/// forever — no finite family set covers all `n` (bounded-degree incompleteness).
pub fn family_growth(max_n: usize) -> Vec<(usize, usize, Vec<String>)> {
    let mut prev: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    (1..=max_n)
        .map(|n| {
            let fam: std::collections::BTreeSet<String> = family_census(n).into_keys().collect();
            let new: Vec<String> = fam.difference(&prev).cloned().collect();
            prev.clone_from(&fam);
            (n, fam.len(), new)
        })
        .collect()
}

/// **The family census across `n`** — classify every minimal-UNSAT orbit into its certified family (lean:
/// reaches `n=4`). This is the parametric handle on the infinite tower: the families are defined for *all*
/// `n`, so recognizing membership (poly for the cheap families; `n^d` for the degree-`d` algebraic ones)
/// covers the structured tower without enumerating it — the only way past the super-exponential orbit count.
pub fn family_census(n: usize) -> BTreeMap<String, usize> {
    let mut fam: BTreeMap<String, usize> = BTreeMap::new();
    for cover in minimal_cover_orbits(n) {
        *fam.entry(family_name(&weakest_crushing_rung(n, &cover.clauses(), n))).or_insert(0) += 1;
    }
    fam
}

/// **The family tower — proven complete and finite at every `n`, without enumeration.** The constructive
/// Nullstellensatz completeness of [`crate::polycalc::build_ns_certificate`] (every UNSAT formula over `n`
/// variables has a degree-`≤ n` GF(2) refutation, so nothing is *beyond budget*) pins the *weakest crushing
/// rung* of every minimal-UNSAT formula to a FIXED, parametric set: the three cheap families (unit
/// propagation, counting, parity) and the algebraic degrees `d = 2..=n`. No family lies outside this tower.
/// So the complete list of families at scale `n` is produced in `O(n)` here — not read off the
/// super-exponential orbit census. This is the "we do not iterate — we PROVED the families" statement, and
/// [`family_census`] (which does enumerate) is always a subset of it. The size is `Θ(n)`, climbing forever:
/// no finite family set covers all `n`, because the degree wall forces a fresh `algebraic-d{n}` at each `n`.
pub fn family_tower(n: usize) -> Vec<String> {
    let mut fams = vec![
        "unit-propagation".to_string(),
        "counting (pigeonhole/cardinality)".to_string(),
        "parity (XOR/Tseitin)".to_string(),
    ];
    for d in 2..=n {
        fams.push(format!("algebraic-d{d}"));
    }
    fams
}

/// A parametric UNSAT witness realizing the **unit-propagation** rung at any `n ≥ 1`: a variable and its
/// negation. Carving alone closes it, so its weakest crushing rung is [`ProofRung::Trivial`].
pub fn witness_unit_propagation(n: usize) -> (usize, Vec<Vec<Lit>>) {
    (n.max(1), vec![vec![Lit::pos(0)], vec![Lit::neg(0)]])
}

/// A parametric UNSAT witness realizing the **parity** rung at any `n ≥ 3`: the odd XOR triangle
/// `x₀⊕x₁ = 1, x₁⊕x₂ = 1, x₂⊕x₀ = 1` (their sum is `1 ≠ 0`), encoded in CNF. No unit propagates and no
/// counting cut sees it — only a GF(2) parity refutation closes it, so the weakest rung is
/// [`ProofRung::Parity`].
pub fn witness_parity(n: usize) -> (usize, Vec<Vec<Lit>>) {
    let mut clauses = Vec::new();
    for (a, b) in [(0u32, 1u32), (1, 2), (2, 0)] {
        clauses.push(vec![Lit::pos(a), Lit::pos(b)]); // x_a ⊕ x_b = 1
        clauses.push(vec![Lit::neg(a), Lit::neg(b)]);
    }
    (n.max(3), clauses)
}

/// A parametric UNSAT witness realizing the **counting** rung: the pigeonhole principle with `pigeons`
/// pigeons and `pigeons − 1` holes ([`crate::families::php`]). Resolution-exponential, invisible to GF(2)
/// parity, closed only by a counting / Hall cut — so the weakest rung is [`ProofRung::Counting`].
pub fn witness_counting(pigeons: usize) -> (usize, Vec<Vec<Lit>>) {
    let (cnf, _) = crate::families::php(pigeons);
    (cnf.num_vars, cnf.clauses)
}

/// **The realization frontier at `n`** — the lower-bound companion to [`family_tower`]. For each certified
/// family that actually OCCURS among the minimal-UNSAT covers over `n` variables, one witness cover's
/// clauses. Where [`family_tower`] is the proven *upper* envelope (`families ⊆ tower`), this shows *which*
/// rungs are realized at scale `n`, with a witness. A tower family absent here is simply not yet realized at
/// this `n` — e.g. counting needs enough variables to host a pigeonhole, so it is empty at small `n`.
pub fn realized_tower_families(n: usize) -> BTreeMap<String, Vec<Vec<Lit>>> {
    let mut out: BTreeMap<String, Vec<Vec<Lit>>> = BTreeMap::new();
    for cover in minimal_cover_orbits(n) {
        let clauses = cover.clauses();
        out.entry(family_name(&weakest_crushing_rung(n, &clauses, n))).or_insert(clauses);
    }
    out
}

fn rung_label(rung: &ProofRung) -> String {
    match rung {
        ProofRung::Trivial => "trivial".into(),
        ProofRung::Counting => "counting".into(),
        ProofRung::Parity => "parity".into(),
        ProofRung::ModCount { p } => format!("modcount-p{p}"),
        ProofRung::Nullstellensatz { min_degree } => format!("nullstellensatz-d{min_degree}"),
        ProofRung::BeyondBudget => "beyond-budget".into(),
    }
}

/// The certificate degree of a rung label: trivial/counting = 0, parity = 1, `modcount-p{p}` = 1 (a
/// linear cut, just over another characteristic), `nullstellensatz-d{k}` = k, `beyond-budget` = `None`
/// (no bounded certificate — the "structureless at this budget").
fn degree_of_label(label: &str) -> Option<usize> {
    match label {
        "trivial" | "counting" => Some(0),
        "parity" => Some(1),
        "beyond-budget" => None,
        l if l.starts_with("modcount-p") => Some(1),
        l => l.strip_prefix("nullstellensatz-d").and_then(|d| d.parse().ok()),
    }
}

/// **Structure accounting** — the answer to "how many minimal-UNSAT families have *no* structure, and how
/// does cheap structure run out?" For each `n`: the cumulative number of orbits covered by a certified proof
/// of degree `≤ d` (`covered_by_degree[d]`), and the `structureless` count = orbits with no bounded
/// certificate at all (rung = `beyond-budget`). The honest finding at `n ≤ 4`: **`structureless = 0`** — every
/// minimal-UNSAT cover is algebraically structured (degree `≤ n`). Nothing at finite `n` is truly random; but
/// the *degree* the structure requires grows with `n` (`= n`), so any *fixed*-degree ("cheap") lens covers a
/// shrinking fraction. That reconciles "no finite randomness" with the counting bound: structure always
/// exists, but cheap structure runs out.
#[derive(Clone, Debug)]
pub struct StructureAccounting {
    pub n: usize,
    pub orbits: usize,
    /// `covered_by_degree[d]` = orbits refutable by a certified proof of degree `≤ d`, `d = 0..=n`.
    pub covered_by_degree: Vec<usize>,
    /// Orbits with no bounded certificate (rung = `beyond-budget`) — the truly-structureless count.
    pub structureless: usize,
}

/// Compute [`StructureAccounting`] for `n` (lean: certified rung per orbit, reaches `n=4`).
pub fn structure_accounting(n: usize) -> StructureAccounting {
    let mut by_rung: BTreeMap<String, usize> = BTreeMap::new();
    let mut orbits = 0usize;
    for cover in minimal_cover_orbits(n) {
        orbits += 1;
        *by_rung.entry(rung_label(&weakest_crushing_rung(n, &cover.clauses(), n))).or_insert(0) += 1;
    }
    let mut covered_by_degree = vec![0usize; n + 1];
    let mut structureless = 0usize;
    for (label, &count) in &by_rung {
        match degree_of_label(label) {
            Some(d) => {
                for slot in covered_by_degree.iter_mut().skip(d.min(n)) {
                    *slot += count;
                }
            }
            None => structureless += count,
        }
    }
    StructureAccounting { n, orbits, covered_by_degree, structureless }
}

/// **The lens-menu split + morph clustering.** A *lean* pass (per-orbit certified invariants only — no full
/// router, no group closure — so it reaches `n=4`): which certified lens-class covers each orbit, and how
/// many *distinct structural signatures* the orbits collapse to. A signature is `(rung, shadow, min-res-width,
/// face-vector)` — the structural fingerprint. When `distinct_signatures ≪ orbits`, the vast orbit count is
/// mostly **morphs of a few structural types** (the same family, mutated), exactly as one expects: symmetry
/// already quotiented the exact symmetries, and the residual multiplicity is near-symmetric variation.
#[derive(Clone, Debug)]
pub struct MenuSplit {
    pub n: usize,
    pub orbits: usize,
    /// Orbit count per certified proof-rung — the fixed lens menu (trivial/counting/parity) vs the growing
    /// `nullstellensatz-d{degree}` dial.
    pub by_rung: BTreeMap<String, usize>,
    /// Number of distinct `(rung, shadow, min-res-width, face-vector)` signatures — the structural-type count.
    pub distinct_signatures: usize,
    /// Orbits in the single most common signature — the largest morph-class.
    pub largest_morph_class: usize,
}

/// Compute the [`MenuSplit`] for `n` (lean: reaches `n=4`).
pub fn menu_split(n: usize) -> MenuSplit {
    let mut by_rung: BTreeMap<String, usize> = BTreeMap::new();
    let mut sigs: BTreeMap<(String, String, usize, Vec<(usize, usize)>), usize> = BTreeMap::new();
    let mut orbits = 0usize;
    for cover in minimal_cover_orbits(n) {
        orbits += 1;
        let clauses = cover.clauses();
        let label = rung_label(&weakest_crushing_rung(n, &clauses, n));
        *by_rung.entry(label.clone()).or_insert(0) += 1;
        let shadow = format!("{:?}", diagnose(n, &clauses).cut);
        let width = min_resolution_width(&cover).unwrap_or(usize::MAX);
        let fv: Vec<(usize, usize)> = face_vector(&cover).into_iter().collect();
        *sigs.entry((label, shadow, width, fv)).or_insert(0) += 1;
    }
    MenuSplit {
        n,
        orbits,
        by_rung,
        distinct_signatures: sigs.len(),
        largest_morph_class: sigs.values().copied().max().unwrap_or(0),
    }
}

/// **The degree-growth wall, measured.** The deepest minimum-Nullstellensatz degree among the minimal-UNSAT
/// families at `n` whose *weakest* certified refutation is algebraic (families caught by a cheaper lens —
/// counting, parity — contribute nothing, correctly). This is a *lean* pass: only the certified
/// proof-complexity ladder ([`weakest_crushing_rung`]) per orbit, none of the full router's symmetry/algebraic
/// machinery — so it reaches `n=4` where the full [`census`] times out. As `n` grows this climbs (bounded-degree
/// Nullstellensatz is incomplete — Tseitin/expander families force degree `Ω(n)`), so any *fixed*-degree lens
/// is eventually outrun. That climb is the honest wall, not a fixed residue.
pub fn max_ns_degree_at(n: usize) -> usize {
    minimal_cover_orbits(n)
        .into_iter()
        .filter_map(|cover| match weakest_crushing_rung(n, &cover.clauses(), n) {
            ProofRung::Nullstellensatz { min_degree } => Some(min_degree),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// Compute the [`ResidueMap`]: rank every lens over every family and partition by which resists.
pub fn residue_map(n: usize) -> ResidueMap {
    let records = census(n);
    let mut m = ResidueMap {
        n,
        total: records.len(),
        crushed: 0,
        residue: 0,
        targetable: 0,
        rigid_core: 0,
        core_max_ns_degree: 0,
    };
    for r in &records {
        let fell_through = matches!(r.route, Route::Incompressible | Route::Cdcl);
        if fell_through {
            m.residue += 1;
            if r.stabilizer_order == 1 {
                m.rigid_core += 1;
            }
            if let ProofRung::Nullstellensatz { min_degree } = r.rung {
                m.core_max_ns_degree = m.core_max_ns_degree.max(min_degree);
            }
        } else {
            m.crushed += 1;
        }
        if r.symmetry_underbroken() {
            m.targetable += 1;
        }
    }
    m
}

/// The `Bₙ`-orbit census of all minimal UNSAT formulas over `n` variables.
pub fn census(n: usize) -> Vec<OrbitRecord> {
    let gens = hyperoctahedral_generators(n);
    let group = cube_group_closure(&gens, n);
    let group_order = group.len();
    minimal_cover_orbits(n)
        .into_iter()
        .map(|cover| {
            let clauses = cover.clauses();
            let (_, orbit_size) = canonical_cover(&cover, &gens);
            // The cover's full Bₙ stabilizer — every geometric automorphism — and the rule-orbit count
            // it induces: the strongest symmetry break this formula admits.
            let stabilizer: Vec<_> =
                group.iter().filter(|g| g.is_automorphism(&cover)).cloned().collect();
            let full_rule_orbits =
                cover.blocker_orbits(&stabilizer).map(|o| o.len()).unwrap_or(clauses.len());
            let rung = weakest_crushing_rung(n, &clauses, n);
            let shadow = diagnose(n, &clauses).cut;
            let solved = solve_structured(n, &clauses);
            let affine_explained =
                cover.to_expr().map(|e| crate::xorsat::refute_via_parity(&e)).unwrap_or(false);
            OrbitRecord {
                n,
                num_clauses: clauses.len(),
                orbit_size,
                stabilizer_order: group_order / orbit_size,
                face_vector: face_vector(&cover),
                min_res_width: min_resolution_width(&cover).unwrap_or(usize::MAX),
                rung,
                shadow,
                route: solved.via,
                discovered_rule_orbits: cover.discovered_rule_symmetry().rule_orbits,
                full_rule_orbits,
                affine_explained,
                modp_routed: matches!(solved.via, Route::ModP)
                    && matches!(solved.answer, Answer::Unsat),
                rep: cover,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residue_map_partitions_the_census_and_locates_the_wall() {
        for n in 2..=3 {
            let m = residue_map(n);
            // The partition is exhaustive: every minimal-UNSAT family is either crushed by a lens or residue.
            assert_eq!(m.crushed + m.residue, m.total, "crushed ⊎ residue = all orbits at n={n}");
            assert!(m.crushed > 0, "structural specialists crush families at n={n}");
            // The rigid core is a subset of the residue (residue that also has no symmetry to break).
            assert!(m.rigid_core <= m.residue, "rigid core ⊆ residue");
            // The measurement — the honest map. (Reported; the numbers are the finding, not an assertion.)
            eprintln!(
                "n={n}: total={} crushed={} residue={} targetable(symmetry-left)={} rigid_core={} core_ns_deg={}",
                m.total, m.crushed, m.residue, m.targetable, m.rigid_core, m.core_max_ns_degree
            );
        }
    }

    /// **The characteristic rung closes the router-beats-ladder audit gap, conservatively.** For every
    /// census orbit at `n ≤ 3`, the extended ladder ([`extended_rung`], primes 3/5/7) differs from the
    /// legacy placement ONLY by verified `ModCount` placements: whenever it reports `ModCount { p }`,
    /// re-running the one-hot recovery and the `GF(p)` Gaussian independently confirms a re-checkable
    /// refutation at that exact modulus; every other orbit gets the identical rung. And the gap
    /// direction: every orbit the router crushed with its mod-`p` specialist while the ladder had no
    /// narrow rung (`modp_routed && router_beats_ladder`) is caught by `ModCount`. The gap population is
    /// counted and reported honestly — at these small `n` it may be empty (one-hot mod-`p` germs are
    /// wide), which is exactly why the synthetic mod-3 Tseitin instance in the hypercube test pins the
    /// non-vacuous case; here the claim is census-wide conservativity plus gap coverage.
    #[test]
    fn the_characteristic_rung_closes_the_router_ladder_audit_gap() {
        let primes = [3u64, 5, 7];
        let (mut gap_closed, mut modcount_seen) = (0usize, 0usize);
        for n in 1..=3usize {
            for rec in census(n) {
                let ext = extended_rung(&rec, &primes);
                match ext {
                    ProofRung::ModCount { p } => {
                        modcount_seen += 1;
                        let clauses = rec.rep.clauses();
                        let r = crate::modp::recover_from_cnf(n, &clauses)
                            .expect("a ModCount placement implies a recognized one-hot encoding");
                        assert_eq!(r.modulus, p, "the rung names the recovered modulus");
                        match crate::modp::solve(&r.equations, r.num_vars, p) {
                            crate::modp::ModpOutcome::Unsat(combo) => assert!(
                                crate::modp::is_refutation(&r.equations, r.num_vars, p, &combo),
                                "n={n}: the ModCount placement re-checks independently"
                            ),
                            crate::modp::ModpOutcome::Sat(_) => {
                                panic!("n={n}: a ModCount placement must be refutable")
                            }
                        }
                    }
                    _ => assert_eq!(
                        ext, rec.rung,
                        "n={n}: off the ModCount population the ladders place identically"
                    ),
                }
                if rec.modp_routed && rec.router_beats_ladder() {
                    assert!(
                        matches!(ext, ProofRung::ModCount { .. }),
                        "n={n}: the audit-gap orbit lands on the characteristic rung"
                    );
                    gap_closed += 1;
                }
            }
        }
        eprintln!(
            "census n ≤ 3: {gap_closed} audit-gap orbits closed, {modcount_seen} ModCount placements"
        );
    }

    #[test]
    fn degree_growth_curve_and_covering_class_counts() {
        // Two curves vs n, both lean (orbit enumeration + certified rung, no full router):
        //   (1) covering-class count = number of Bₙ-orbits of minimal-UNSAT covers — the "covering ring"
        //       size at n: how many ESSENTIALLY-DISTINCT covering classes exist once symmetry collapses the
        //       raw 2ⁿ·n! covers to one representative per orbit (this is exactly "try a TON less").
        //   (2) max_ns_degree = the algebraic depth the residue forces — the degree-growth wall.
        for n in 1..=3 {
            let classes = crate::hypercube::minimal_cover_orbits(n).len();
            eprintln!("n={n}: covering_classes(orbits)={classes} max_ns_degree={}", max_ns_degree_at(n));
        }
        // The covering ring grows fast (1, 4, 43, … 42263 at n=4) — the count of classes to cover climbs.
        let (c2, c3) = (crate::hypercube::minimal_cover_orbits(2).len(), crate::hypercube::minimal_cover_orbits(3).len());
        assert!(c3 > c2, "the number of covering classes grows with n: {c2} → {c3}");
        // The degree-growth wall: n=3 forces strictly deeper algebra than n=2.
        assert!(max_ns_degree_at(3) > max_ns_degree_at(2), "the NS-degree wall climbs n=2 → n=3");
    }

    #[test]
    fn lens_menu_split_and_morph_clustering() {
        for n in 2..=3 {
            let m = menu_split(n);
            eprintln!(
                "n={n}: orbits={} distinct_signatures={} largest_morph_class={} by_rung={:?}",
                m.orbits, m.distinct_signatures, m.largest_morph_class, m.by_rung
            );
            assert_eq!(m.by_rung.values().sum::<usize>(), m.orbits, "the lens menu covers every orbit");
            assert!(m.distinct_signatures <= m.orbits, "signatures ≤ orbits");
        }
        // At n=3 the 43 orbits collapse to strictly fewer structural signatures — morphs of fewer types.
        assert!(menu_split(3).distinct_signatures < menu_split(3).orbits, "orbits are morphs of fewer types");
    }

    #[test]
    fn clause_agl_detector_probes_the_generic_cores_for_hidden_affine_symmetry() {
        let full = "nullstellensatz-d3".to_string();
        let (mut parity_checked, mut hidden, mut affine_rigid) = (false, 0usize, 0usize);
        for cover in minimal_cover_orbits(3) {
            let clauses = cover.clauses();
            let label = rung_label(&weakest_crushing_rung(3, &clauses, 3));
            let agl = clause_agl_symmetries(3, &clauses);
            assert!(agl >= 1, "the identity is always an AGL automorphism");
            if label == "parity" {
                // Validation: the parity family is highly symmetric ⟹ the detector finds real AGL symmetry.
                assert!(agl > 1, "the detector finds the parity family's affine symmetry");
                parity_checked = true;
            }
            if label == full {
                if agl > 1 {
                    hidden += 1; // Bₙ-rigid but affine-symmetric — a NEW breakable structure
                } else {
                    affine_rigid += 1; // structure-minimal at the affine level too
                }
            }
        }
        assert!(parity_checked, "validated the detector on the symmetric parity family");
        eprintln!("n=3 generic cores: hidden_affine_symmetry={hidden} affine_rigid={affine_rigid}");
    }

    #[test]
    #[ignore = "n=4 sampled clause-AGL: AGL(4,2)=322560 per core, minutes"]
    fn clause_agl_at_n4_sampled_does_hidden_affine_symmetry_persist() {
        let full = "nullstellensatz-d4".to_string();
        let (mut hidden, mut rigid, mut sampled) = (0usize, 0usize, 0usize);
        for cover in minimal_cover_orbits(4) {
            if sampled >= 40 {
                break;
            }
            let clauses = cover.clauses();
            if rung_label(&weakest_crushing_rung(4, &clauses, 4)) != full {
                continue;
            }
            sampled += 1;
            if clause_agl_symmetries(4, &clauses) > 1 {
                hidden += 1;
            } else {
                rigid += 1;
            }
        }
        eprintln!("n=4 generic cores (sampled {sampled}): hidden_affine={hidden} affine_rigid={rigid}");
    }

    #[test]
    fn peek_inside_the_generic_full_degree_cores() {
        let sub = generic_subfamilies(3);
        eprintln!("n=3 generic sub-families (shadow, width) → distinct types: {sub:?}");
        let generic_types: usize = sub.values().sum();
        // The generic full-degree cores are NOT one undifferentiated blob — finer invariants split them.
        assert!(generic_types > 0, "there are generic full-degree types at n=3");
        assert!(!sub.is_empty(), "they sub-classify by (shadow, width) — sub-structure exists");
        // "Zero structure" is impossible: structure_accounting proves structureless = 0 (degree ≤ n always
        // refutes). So even the generic cores HAVE structure (a full-degree certificate); "generic" means
        // no CHEAPER structure, not none.
        assert_eq!(structure_accounting(3).structureless, 0, "no core has literally zero structure");
    }

    #[test]
    fn label_every_structural_type_by_family() {
        // n=3: label all 27 types; report the family distribution + name the giants.
        let (num_types, by_family) = family_of_types(3);
        eprintln!("n=3: {num_types} types → {by_family:?}");
        eprintln!("n=3 giants: {:?}", named_giants(3, 5));
        assert_eq!(num_types, by_family.values().sum::<usize>(), "every type is labeled by exactly one family");
        assert!(num_types > 1, "multiple structural types at n=3");
        // The giants are labeled with a concrete family (no unlabeled type).
        assert!(named_giants(3, 3).iter().all(|(_, label)| !label.is_empty()), "every giant named");
    }

    #[test]
    #[ignore = "n=4: label all 403 types, ~minutes"]
    fn label_all_403_types_at_n4() {
        let (num_types, by_family) = family_of_types(4);
        eprintln!("n=4: {num_types} types → {by_family:?}");
        eprintln!("n=4 giants: {:?}", named_giants(4, 8));
    }

    #[test]
    fn family_growth_is_forced_by_the_degree_wall() {
        let growth = family_growth(3);
        for (n, count, new) in &growth {
            eprintln!("n={n}: family_count={count} new_families={new:?}");
        }
        // The family count climbs with n (a fixed cheap menu + a new algebraic degree per n).
        assert!(growth[2].1 > growth[0].1, "more families at n=3 than n=1");
        // THE forcing mechanism: at n=3 the new family opened is the degree-3 algebraic one — the degree wall
        // (max_ns_degree = n) necessitates a degree-n certificate that did not exist at n−1.
        assert!(growth[2].2.iter().any(|f| f == "algebraic-d3"), "n=3 forces the algebraic-d3 family");
    }

    #[test]
    fn family_census_classifies_every_orbit_across_n() {
        for n in 1..=3 {
            let fam = family_census(n);
            let total: usize = fam.values().sum();
            eprintln!("n={n}: {fam:?}");
            // Every orbit is classified into a certified family — none "unclassified".
            assert!(!fam.contains_key("unclassified"), "no unclassified orbit at n={n}");
            assert_eq!(total, crate::hypercube::minimal_cover_orbits(n).len(), "the family census covers all orbits");
        }
        // Cross-n trend: the algebraic families appear and deepen with n (the degree tower); n=3 carries an
        // algebraic-d3 family absent at n=2 — the parametric handle on the tower, one degree per n.
        assert!(family_census(3).keys().any(|k| k == "algebraic-d3"), "n=3 opens the algebraic-d3 family");
        assert!(!family_census(2).keys().any(|k| k == "algebraic-d3"), "which n=2 does not have");
    }

    #[test]
    fn structure_accounting_no_finite_randomness_but_cheap_structure_runs_out() {
        for n in 1..=3 {
            let a = structure_accounting(n);
            eprintln!(
                "n={n}: orbits={} structureless={} covered_by_degree={:?}",
                a.orbits, a.structureless, a.covered_by_degree
            );
            // THE answer to "how many are random": zero — every minimal-UNSAT cover has degree-≤n structure.
            assert_eq!(a.structureless, 0, "no truly-structureless family at n={n} (degree ≤ n always suffices)");
            // Full coverage is reached by degree n (the last slot covers everything non-structureless).
            assert_eq!(*a.covered_by_degree.last().unwrap(), a.orbits, "degree ≤ n covers ALL orbits");
            // Cheap structure runs out: coverage is monotone nondecreasing in the degree budget.
            assert!(a.covered_by_degree.windows(2).all(|w| w[0] <= w[1]), "coverage grows with the degree dial");
        }
        // Cheap (bounded) degree covers a SHRINKING fraction as n grows: at n=3, degree ≤ 2 covers < all.
        let a3 = structure_accounting(3);
        assert!(a3.covered_by_degree[2] < a3.orbits, "at n=3 a degree-2 lens already misses families (need d=3)");
    }

    #[test]
    fn parametric_witnesses_realize_the_cheap_rungs_for_all_n() {
        // Each cheap tower rung has an explicit PARAMETRIC witness whose WEAKEST crushing rung is certified to
        // be exactly that rung (not a cheaper one) — realization for all n above the rung's threshold.
        for n in 1..=6 {
            let (nv, cl) = witness_unit_propagation(n);
            assert!(matches!(weakest_crushing_rung(nv, &cl, nv), ProofRung::Trivial), "unit-prop realized @ n={n}");
        }
        for n in 3..=6 {
            let (nv, cl) = witness_parity(n);
            assert!(matches!(weakest_crushing_rung(nv, &cl, nv), ProofRung::Parity), "parity realized @ n={n}");
        }
        for pigeons in 3..=4 {
            let (nv, cl) = witness_counting(pigeons);
            assert!(
                matches!(weakest_crushing_rung(nv, &cl, nv), ProofRung::Counting),
                "counting realized (PHP {pigeons} pigeons, {nv} vars)"
            );
        }
    }

    #[test]
    fn the_realization_frontier_certifies_which_tower_rungs_occur() {
        // The lower-bound companion to the tower: which rungs are REALIZED among the n-variable covers, with a
        // witness. Every realized family is in the tower (containment) and every witness is genuinely UNSAT.
        for n in 2..=3 {
            let realized = realized_tower_families(n);
            let tower: std::collections::BTreeSet<String> = family_tower(n).into_iter().collect();
            eprintln!("n={n}: realized families = {:?}", realized.keys().collect::<Vec<_>>());
            for (fam, clauses) in &realized {
                assert!(tower.contains(fam), "n={n}: realized family {fam} lies in the tower");
                assert!(crate::polycalc::build_ns_certificate(n, clauses).is_ok(), "n={n}: witness for {fam} is UNSAT");
            }
        }
        // The algebraic ladder is populated with EXACT minimum degree: d=2 (@n=2) and d=3 (@n=3) each have a
        // witness refuted at degree d but NOT at d−1 — the degree wall's rungs, occupied and certified.
        for d in 2..=3usize {
            let witness = realized_tower_families(d)
                .remove(&format!("algebraic-d{d}"))
                .unwrap_or_else(|| panic!("algebraic-d{d} must be realized at n={d}"));
            assert!(crate::polycalc::nullstellensatz_refutes(d, &witness, d), "algebraic-d{d}: refuted at degree {d}");
            assert!(
                !crate::polycalc::nullstellensatz_refutes(d, &witness, d - 1),
                "algebraic-d{d}: NOT refuted at degree {} — minimum degree is exactly {d}",
                d - 1
            );
        }
    }

    #[test]
    #[ignore = "n=4 census scan for the algebraic-d4 witness, minutes"]
    fn algebraic_d4_rung_is_realized_with_exact_minimum_degree() {
        let witness = realized_tower_families(4)
            .remove("algebraic-d4")
            .expect("the algebraic-d4 rung must be realized at n=4");
        assert!(crate::polycalc::nullstellensatz_refutes(4, &witness, 4), "algebraic-d4: refuted at degree 4");
        assert!(
            !crate::polycalc::nullstellensatz_refutes(4, &witness, 3),
            "algebraic-d4: NOT refuted at degree 3 — minimum degree is exactly 4"
        );
    }

    #[test]
    fn agl_collapse_factor_at_small_n_and_the_transvection_walk_is_tight() {
        // The affine lens can only MERGE orbits (never split): AGL-classes ≤ Bₙ-orbits at every n.
        for n in 1..=3 {
            let ex = agl_collapse_exact(n);
            eprintln!(
                "n={n}: Bₙ-orbits={} → AGL-classes={} (collapse ×{:.2})",
                ex.bn_orbits, ex.agl_classes, ex.factor()
            );
            assert!(ex.agl_classes <= ex.bn_orbits, "n={n}: AGL can only merge classes, never split");
            assert!(ex.agl_classes >= 1, "n={n}: at least one class");
        }
        // MORE SYMMETRY ⟹ strictly fewer classes at n=3: the affine lens sees equivalences Bₙ cannot.
        let e3 = agl_collapse_exact(3);
        assert!(e3.agl_classes < e3.bn_orbits, "AGL strictly collapses the n=3 census (43 → fewer)");
        // The transvection walk (which SCALES to n=4) is an upper bound on classes — and TIGHT at n=3,
        // evidence the CNF-preserving affine moves capture the full affine equivalence (no non-CNF detour
        // is needed at this scale).
        let w3 = agl_collapse_via_transvections(3);
        assert!(w3.agl_classes >= e3.agl_classes, "the walk over-counts classes (a lower bound on collapse)");
        assert_eq!(w3.agl_classes, e3.agl_classes, "the transvection walk is TIGHT at n=3 (matches exact AGL)");
    }

    #[test]
    fn agl_merges_are_auto_discovered_and_codified_as_verified_affine_witnesses() {
        // Group the n=3 Bₙ orbits into AGL classes, then codify the 43→38 collapse: every SAME-class pair
        // must carry a VERIFIED affine witness (the map Bₙ could not see), and every CROSS-class pair must
        // have NONE (genuinely affine-distinct). This proves the AGL partition is exactly right AND turns
        // each merge into a concrete, re-checkable affine equivalence — the auto-discovered structure.
        let reps: Vec<Vec<u64>> =
            minimal_cover_orbits(3).into_iter().map(|c| blocker_masks(&c.clauses(), 3)).collect();
        let tables: Vec<Vec<u32>> = all_affine_bijections(3).iter().map(|p| perm_table(p, 3)).collect();
        let canon: Vec<Vec<u64>> = reps.iter().map(|m| canonical_over_tables(m, &tables)).collect();
        let mut merges = 0usize;
        for i in 0..reps.len() {
            for j in (i + 1)..reps.len() {
                let same_class = canon[i] == canon[j];
                match find_agl_witness(&reps[i], &reps[j], 3) {
                    Some(w) => {
                        assert!(same_class, "an affine witness exists only between affine-equivalent orbits");
                        assert!(w.verify(&reps[i], &reps[j], 3), "the auto-discovered affine witness must re-check");
                        // The witness genuinely uses affine structure: a shear is invisible to Bₙ.
                        merges += 1;
                    }
                    None => assert!(!same_class, "affine-distinct orbits must have NO affine witness"),
                }
            }
        }
        assert!(merges > 0, "the 43→38 collapse yields real affine equivalences to codify");
    }

    #[test]
    #[ignore = "n=4 AGL collapse: 42263 orbits × 12 transvections × Bₙ-canon(384), minutes"]
    fn agl_collapse_factor_at_n4() {
        let w = agl_collapse_via_transvections(4);
        eprintln!(
            "n=4: Bₙ-orbits={} → AGL-classes={} (collapse ×{:.2})",
            w.bn_orbits, w.agl_classes, w.factor()
        );
    }

    #[test]
    fn transvection_symbolic_image_matches_the_pointset_computation() {
        // The O(clause) symbolic transvection rule must agree with the exhaustive 2ⁿ point-set computation, on
        // hundreds of random clauses — the correctness anchor for the scalable finder.
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let mut rng = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let key = |o: &Option<Vec<Lit>>| {
            o.as_ref().map(|cl| {
                let mut k: Vec<(u32, bool)> = cl.iter().map(|l| (l.var(), l.is_positive())).collect();
                k.sort_unstable();
                k
            })
        };
        for _ in 0..500 {
            let n = 3 + (rng() % 4) as usize; // 3..=6
            let width = 1 + (rng() % n as u64) as usize;
            let mut seen = std::collections::HashSet::new();
            let mut c: Vec<Lit> = Vec::new();
            while c.len() < width {
                let v = (rng() % n as u64) as u32;
                if seen.insert(v) {
                    c.push(Lit::new(v, rng() & 1 == 0));
                }
            }
            let a = (rng() % n as u64) as u32;
            let mut b = (rng() % n as u64) as u32;
            while b == a {
                b = (rng() % n as u64) as u32;
            }
            let sym = transvection_image_clause(&c, a, b);
            let tau = Affine {
                n,
                matrix: {
                    let mut m: Vec<u64> = (0..n).map(|k| 1u64 << k).collect();
                    m[a as usize] |= 1u64 << b;
                    m
                },
                translation: 0,
            };
            let point_based = pointset_to_clause(map_point_set(falsify_set(&c, n), &tau, n), n);
            assert_eq!(key(&sym), key(&point_based), "symbolic transvection image ≠ point-based for {c:?}, σ({a}↦{a}⊕{b})");
        }
    }

    #[test]
    fn the_affine_transvection_finder_scales_to_large_n() {
        // Polynomial, no 2ⁿ: run at n = 60 — impossible for any AGL enumeration. A family where x0 and x1
        // always co-occur positively is invariant under σ: x0 ↦ x0 ⊕ x1 (both in support, x1 positive ⟹ no
        // flip); the finder recovers it in O(n²·|clauses|) time.
        let n = 60usize;
        let clauses: Vec<Vec<Lit>> =
            (2..n as u32).map(|v| vec![Lit::pos(0), Lit::pos(1), Lit::pos(v)]).collect();
        let gens = affine_transvection_generators(n, &clauses);
        assert!(gens.contains(&(0, 1)), "the finder recovers σ(0↦0⊕1) at n={n}");
        assert!(gens.contains(&(1, 0)), "and its partner σ(1↦1⊕0)");
        // Sanity: a shear mixing a variable that does not co-occur is correctly rejected.
        assert!(!gens.contains(&(0, 2)), "σ(0↦0⊕2) is not a symmetry (x2 is absent from most clauses)");
    }

    #[test]
    fn rank1_symbolic_image_matches_the_pointset_computation() {
        // The O(clause) rank-1 rule M_{u,v}: x ↦ x ⊕ (v·x)u must agree with the exhaustive 2ⁿ point-set image,
        // on hundreds of random bijective (u·v = 0) instances — the correctness anchor for the symplectic finder.
        let mut s = 0xD1B5_4A32_D192_ED03u64;
        let mut rng = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let key = |o: &Option<Vec<Lit>>| {
            o.as_ref().map(|cl| {
                let mut k: Vec<(u32, bool)> = cl.iter().map(|l| (l.var(), l.is_positive())).collect();
                k.sort_unstable();
                k
            })
        };
        let mut bijective_seen = 0;
        for _ in 0..800 {
            let n = 3 + (rng() % 4) as usize; // 3..=6
            let mask = (1u64 << n) - 1;
            let u = rng() & mask;
            let v = rng() & mask;
            if (u & v).count_ones() % 2 != 0 {
                continue; // require u·v = 0 (M bijective)
            }
            bijective_seen += 1;
            // a random nonempty clause
            let width = 1 + (rng() % n as u64) as usize;
            let mut seen = std::collections::HashSet::new();
            let mut c: Vec<Lit> = Vec::new();
            while c.len() < width {
                let var = (rng() % n as u64) as u32;
                if seen.insert(var) {
                    c.push(Lit::new(var, rng() & 1 == 0));
                }
            }
            let sym = rank1_image_clause(&c, u, v);
            // point-based oracle: matrix row a = e_a ⊕ (u_a ? v : 0), since (Mx)_a = x_a ⊕ u_a(v·x).
            let matrix: Vec<u64> =
                (0..n).map(|a| (1u64 << a) ^ (if (u >> a) & 1 == 1 { v } else { 0 })).collect();
            let tau = Affine { n, matrix, translation: 0 };
            let point_based = pointset_to_clause(map_point_set(falsify_set(&c, n), &tau, n), n);
            assert_eq!(key(&sym), key(&point_based), "rank-1 image ≠ point-based for {c:?}, u={u:#b}, v={v:#b}");
        }
        assert!(bijective_seen > 200, "exercised enough bijective (u·v=0) cases: {bijective_seen}");
    }

    #[test]
    fn the_parity_wall_is_a_depth_not_a_wall_composite_finder_catches_it() {
        // The full parity constraint x0⊕x1⊕x2⊕x3 = 0, as CNF: one width-4 clause forbidding each odd-parity
        // point. NO single transvection preserves it (each adds x_j to the parity form once, flipping it), but
        // the depth-2 shear "add x_j to two coordinates" preserves it (adds x_j twice = 0). So the "wall" I
        // named was a depth-1 horizon, not a barrier — the composite finder climbs straight over it.
        let n = 4usize;
        let clauses: Vec<Vec<Lit>> = (0u32..(1 << n))
            .filter(|p| p.count_ones() % 2 == 1)
            .map(|p| {
                (0..n as u32)
                    .map(|i| if (p >> i) & 1 == 1 { Lit::neg(i) } else { Lit::pos(i) })
                    .collect()
            })
            .collect();
        assert_eq!(clauses.len(), 8, "half of the 16 points (the odd-parity ones) are forbidden");

        // Depth 1: blind. No single transvection is a symmetry of parity.
        let depth1 = affine_transvection_generators(n, &clauses);
        assert!(depth1.is_empty(), "single transvections cannot preserve parity — the depth-1 horizon");

        // Depth 2: the parity shears appear. Each is genuinely verified to permute the clause set.
        let depth2 = affine_composite_shear_generators(n, &clauses, 2);
        assert!(!depth2.is_empty(), "depth-2 composite shears preserve parity — the wall was a horizon");
        assert!(
            depth2.iter().all(|(s, _)| s.len() == 2),
            "every parity generator is genuinely composite (needs two targets, never one)"
        );
        // Concretely: "add x2 to x0 and x1" is one such symmetry.
        assert!(
            depth2.iter().any(|(s, j)| *j == 2 && s.contains(&0) && s.contains(&1)),
            "the shear (add x2 to {{x0,x1}}) is found"
        );
        eprintln!("parity(n={n}): depth-1 finds {} shears, depth-2 finds {}", depth1.len(), depth2.len());
    }

    #[test]
    fn symplectic_transvection_weight_cannot_grade_ns_degree() {
        // Chasing a graded tracking of symmetry weight against NS degree — and finding, honestly, that the
        // naive axis cannot work. Going one rung above affine shears to the full rank-1 involutions (symplectic
        // transvections T_w = I⊕wwᵀ) does NOT expose growing-weight symmetries: PHP, whose NS degree grows, is
        // rigid even here. And a single parity block's minimal symplectic weight is a constant 2 — because the
        // classical groups (Sₙ, Sp, O) are all generated by weight-2 transvections, so the *minimal* weight is
        // structurally pinned at 2 (or absent) and can never grade. The graded invariant that DOES track NS
        // degree is not a symmetry-group weight but the certificate depth (polycalc's bridge theorem).
        use crate::polycalc::nullstellensatz_refutes;

        // PHP: NS degree grows (4 → 6) yet no symplectic transvection exists up to the variable count — its
        // hardness is protected by *permutation* symmetry, expressible as no single low-rank linear involution.
        for (m, deg) in [(3usize, 4usize), (4, 6)] {
            let (php, _) = crate::families::php(m);
            let nv = php.num_vars;
            let gens = symplectic_transvection_generators(nv, &php.clauses, nv.min(6));
            assert!(gens.is_empty(), "PHP({m}) is symplectic-rigid — no rank-1 linear involution symmetry");
            assert!(nullstellensatz_refutes(nv, &php.clauses, deg), "PHP({m}) NS degree reaches {deg}");
            assert!(!nullstellensatz_refutes(nv, &php.clauses, deg - 1), "…and is exactly {deg}, growing with m");
        }

        // Parity blocks: a symplectic symmetry exists but its minimal weight is a constant 2 for every width —
        // the weight-2-generation of the symmetry group, made visible.
        for w in [4usize, 6] {
            let vars: Vec<u32> = (0..w as u32).collect();
            let clauses: Vec<Vec<Lit>> = (0u32..(1 << w))
                .filter(|p| p.count_ones() % 2 == 1)
                .map(|p| (0..w).map(|i| Lit::new(vars[i], (p >> i) & 1 == 0)).collect())
                .collect();
            let min_weight = symplectic_transvection_generators(w, &clauses, w)
                .iter()
                .map(|g| g.count_ones() as usize)
                .min();
            assert_eq!(min_weight, Some(2), "parity_block({w}): minimal symplectic weight is a constant 2");
        }
    }

    #[test]
    fn family_is_agl_invariant_so_the_affine_lens_preserves_structure() {
        // THEOREM: an affine coordinate change x ↦ Ax⊕b is a degree-PRESERVING automorphism of the multilinear
        // GF(2) ring, so it carries a degree-d Nullstellensatz certificate to a degree-d one — the minimum NS
        // degree, hence the certified FAMILY, is AGL-invariant. Validated exhaustively at n=3: for every census
        // orbit and every affine map that keeps the cover a CNF, the image has the SAME weakest rung and family.
        // This is why AGL (⊋ Bₙ — "more symmetry") is a structure-PRESERVING census lens: quotienting by it
        // loses no family information, it only MERGES orbits — exactly the coarser lens that collapses toward n=5.
        let mut checked = 0usize;
        let mut beyond_bn = 0usize; // images produced by a genuine shear (linear part not a permutation matrix)
        for cover in minimal_cover_orbits(3) {
            let f = cover.clauses();
            let fam_f = family_name(&weakest_crushing_rung(3, &f, 3));
            for phi in all_affine_bijections(3) {
                if let Some(g) = agl_image_formula(&f, &phi, 3) {
                    let fam_g = family_name(&weakest_crushing_rung(3, &g, 3));
                    assert_eq!(fam_f, fam_g, "the family must be invariant under an affine coordinate change");
                    checked += 1;
                    // A row with ≥2 set bits is a shear — a map no signed permutation (Bₙ) can express.
                    if phi.matrix.iter().any(|r| r.count_ones() >= 2) {
                        beyond_bn += 1;
                    }
                }
            }
        }
        assert!(checked > 43, "invariance exercised across every orbit and its affine images");
        // The lens genuinely reaches past Bₙ: some structure-preserving images come from true shears.
        assert!(beyond_bn > 0, "AGL reaches valid CNF images via shears the permutation lens cannot");
    }

    #[test]
    fn the_family_tower_is_provably_complete_and_finite_without_enumeration() {
        // The tower is produced in O(n) parametrically — no orbit enumeration at all.
        for n in 1..=6 {
            let tower = family_tower(n);
            // Finite, size Θ(n): the 3 cheap families + the algebraic degrees 2..=n.
            assert_eq!(tower.len(), 3 + n.saturating_sub(1), "n={n}: tower = 3 cheap families + algebraic d=2..=n");
        }
        // SOUNDNESS of the parametric tower against the ACTUAL census (n=2,3, cheap): every orbit's weakest
        // family lies in the tower, none is beyond-budget (the completeness the constructive certificate
        // guarantees), and that membership is WITNESSED per orbit by a re-checking certificate.
        for n in 2..=3 {
            let tower: std::collections::BTreeSet<String> = family_tower(n).into_iter().collect();
            for cover in minimal_cover_orbits(n) {
                let clauses = cover.clauses();
                let rung = weakest_crushing_rung(n, &clauses, n);
                assert!(!matches!(rung, ProofRung::BeyondBudget), "n={n}: completeness ⟹ no beyond-budget family");
                assert!(tower.contains(&family_name(&rung)), "n={n}: every orbit's family lies in the parametric tower");
                let cert = crate::polycalc::build_ns_certificate(n, &clauses).expect("UNSAT ⟹ a certificate");
                assert!(cert.verify(&clauses), "the constructive certificate witnesses the orbit's tower membership");
            }
        }
        // The enumerated family set is always a SUBSET of the parametric tower — the tower is the complete
        // envelope, computed without iterating the orbits.
        for n in 1..=3 {
            let tower: std::collections::BTreeSet<String> = family_tower(n).into_iter().collect();
            assert!(family_census(n).keys().all(|k| tower.contains(k)), "n={n}: enumerated families ⊆ the tower");
        }
    }

    #[test]
    fn the_coverage_verdict_no_random_family_theta_n_families_but_no_fixed_cost_lens() {
        // The honest answer to "is there a random family / how many families / can we cover them all", proven:
        //
        // (Q2) Is there ONE genuinely-random family with no checkable structure? NO — `structureless = 0` at
        // every n: every UNSAT formula has a degree-≤n GF(2) certificate. Randomness needs unbounded
        // complexity; a finite cube caps it at n. Nothing is structureless.
        for n in 1..=3 {
            assert_eq!(structure_accounting(n).structureless, 0, "no genuinely-random family at n={n}");
        }
        // (Q3a) Are there ~2ⁿ families we can't cover? NO at the FAMILY level: the family count is Θ(n)
        // (3 cheap families + one algebraic degree per n), so the families are coverable PARAMETRICALLY — the
        // super-exponential blow-up is in INSTANCES/orbits (1,4,43,42263,…), which collapse to Θ(n) families.
        for n in 1..=6 {
            assert_eq!(family_tower(n).len(), 3 + n.saturating_sub(1), "n={n}: Θ(n) families, not 2ⁿ");
        }
        // (Q3b) BUT no FIXED-cost (bounded-degree) lens is complete: the minimum certificate degree climbs
        // without bound (max_ns_degree = n), so for each degree budget d there is a family — at n = d+1 — whose
        // weakest certificate needs degree d+1 > d, escaping every degree-d lens. THIS is the real wall: not an
        // unbounded number of families, and not randomness, but unbounded certificate COST.
        for d in 1..=2usize {
            let n = d + 1;
            let witness = realized_tower_families(n)
                .remove(&format!("algebraic-d{n}"))
                .unwrap_or_else(|| panic!("a full-degree family exists at n={n}"));
            assert!(
                !crate::polycalc::nullstellensatz_refutes(n, &witness, d),
                "a degree-{d} lens MISSES a family at n={n} — no fixed-cost lens is complete"
            );
        }
        // (Q1) How fast to CHECK once the degree is known? The certificate basis is Σ_{k≤d} C(n,k) = O(nᵈ) —
        // polynomial for FIXED d, only 2ⁿ at the full degree d = n. So structure, once its degree is known, is
        // cheap to check; the 2ⁿ is the worst-case for the generic full-degree core (where the hardness lives).
        assert!(crate::polycalc::nullstellensatz_basis_size(10, 2) < 10u128.pow(3), "fixed-degree check is polynomial");
        assert_eq!(crate::polycalc::nullstellensatz_basis_size(10, 10), 1u128 << 10, "full degree is 2ⁿ");
    }

    #[test]
    fn every_census_cover_carries_a_constructive_certificate_that_generalizes_past_the_wall() {
        // `structure_accounting` only *counts* `structureless = 0` at `n ≤ 4`. This EXHIBITS the witness:
        // every minimal-UNSAT orbit at n=3 has a constructive degree-≤n Nullstellensatz certificate that
        // re-checks against its own clauses — structureless = 0 proven *per orbit*, not merely tallied.
        for cover in minimal_cover_orbits(3) {
            let clauses = cover.clauses();
            let cert = crate::polycalc::build_ns_certificate(3, &clauses)
                .expect("a minimal-UNSAT cover is unsatisfiable, so the construction yields a certificate");
            assert!(cert.verify(&clauses), "the certificate re-checks against the cover's own clauses");
            assert!(cert.degree() <= 3, "the certificate degree is ≤ n");
        }
        // The SAME uniform construction reaches n=5 — where `minimal_cover_orbits` is infeasible (~10⁷
        // orbits) — on a constructed n=5 UNSAT cover (all 2⁵ corners forbidden by full-width clauses). So
        // `structureless = 0` is settled at n=5 by *proof*, not enumeration: we do not iterate, we construct.
        let n = 5;
        let full: Vec<Vec<Lit>> = (0u64..(1u64 << n))
            .map(|a| (0..n as u32).map(|v| Lit::new(v, (a >> v) & 1 == 0)).collect())
            .collect();
        let cert = crate::polycalc::build_ns_certificate(n, &full).expect("the all-corners-forbidden cover is UNSAT");
        assert!(cert.verify(&full), "the n=5 certificate re-checks — structureless=0 proven past the census wall");
        assert!(cert.degree() <= n, "the n=5 certificate degree is ≤ n");
    }

    #[test]
    #[ignore = "n=4 lean menu split: 42263 orbits × (rung+shadow+width+face-vector), minutes"]
    fn lens_menu_split_at_n4() {
        let m = menu_split(4);
        eprintln!(
            "n=4: orbits={} distinct_signatures={} largest_morph_class={} by_rung={:?}",
            m.orbits, m.distinct_signatures, m.largest_morph_class, m.by_rung
        );
    }

    #[test]
    #[ignore = "n=4 lean degree pass: 42263-orbit enumeration + certified rungs, minutes"]
    fn degree_growth_at_n4() {
        eprintln!(
            "n=4: covering_classes={} max_ns_degree={}",
            crate::hypercube::minimal_cover_orbits(4).len(),
            max_ns_degree_at(4)
        );
    }

    #[test]
    #[ignore = "census-scale: 42263 orbits, minutes"]
    fn residue_map_at_census_scale_n4() {
        let m = residue_map(4);
        assert_eq!(m.crushed + m.residue, m.total, "the partition is exhaustive at census scale");
        eprintln!(
            "n=4: total={} crushed={} residue={} targetable(symmetry-left)={} rigid_core={} core_ns_deg={}",
            m.total, m.crushed, m.residue, m.targetable, m.rigid_core, m.core_max_ns_degree
        );
    }
}
