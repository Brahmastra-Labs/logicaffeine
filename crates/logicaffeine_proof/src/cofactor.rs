//! **The cofactor-DAG lens: symmetry above the instance.**
//!
//! §4 of `work/PAPER.md` measures symmetry *in the instance* — automorphisms of the cover (`Bₙ`, `AGL`,
//! shears). The residue (`census::residue_map`) is rigid to all of them; that rigidity is the wall.
//! This module lifts the lens one level up, onto the formula's **Shannon cofactor DAG**: the DAG
//! whose nodes are the distinct residual sub-formulas `F|ρ` under partial assignments `ρ`, with
//! edges the Shannon expansion `F = x·F|_{x=1} + x̄·F|_{x=0}`.
//!
//! A hard core has **exponentially many distinct cofactors** — that is exactly what "no small
//! variable-order decision DAG / exponential resolution width" means, and it is the finite, per-`n`
//! incompressibility pole ([`crate::ait::incompressible_string_exists`]) one level up. The
//! hypothesis this module makes executable: those exponentially-many cofactors may fall into
//! **polynomially many equivalence classes** under a congruence `~` that is *not* induced by any
//! automorphism of the instance. Quotient the DAG by `~` and the exponential object becomes a
//! polynomial one; when the certificate is `~`-invariant the quotient DAG **is** the refutation,
//! with class-sharing playing the role of extension variables (the Resolution → ER/SR jump).
//!
//! The collapse measure, and the certificate — kept deliberately separate:
//!
//!   - [`distinct_width`] / [`cofactor_set`] — the strict distinct-cofactor set under the fixed
//!     order `0..n` (the OBDD width): the honest exponential floor for the residue.
//!   - [`quotient_class_count`] — the number of `~`-classes among that *same fixed set*. Because it
//!     only quotients a fixed set, `quotient_class_count(Raw) = distinct_width` and coarser
//!     congruences give monotonically fewer classes: `iso ≤ rename ≤ raw = distinct` are
//!     **theorems** (not artifacts of a branching order), so they hold on every formula, the
//!     residue included. This is the clean measure of "do exponentially-many cofactors fall into
//!     polynomially-many classes?"
//!   - [`quotient_dag`] — the *certificate realization*: the cofactor DAG quotiented by `~`, every
//!     edge carrying the explicit [`Twist`] (literal renaming) verified locally by
//!     [`check_quotient_dag`] (zero trust). It is a sound, poly-time-checkable refutation; its node
//!     count is *a* certificate size, reported but never conflated with the collapse measure (the
//!     two constructions are size-incomparable — renaming can re-order the branch variable).
//!
//! The congruence ladder, finest → coarsest: [`Raw`] (strict equality) ⊂ [`Rename`] (first-appearance
//! renaming) ⊂ [`GroupInduced`] (an instance symmetry group's orbits — `Bₙ`, `AGL`) ⊂ [`CofactorIso`]
//! (full CNF-isomorphism: any variable relabeling **and** polarity flip, not tied to an instance
//! automorphism — the strongest *decidable* rung, the one that sees symmetry above the instance).
//! Whether a poly-index congruence exists for the residue past this rung — an SR-definable one — is
//! `3-SAT ∈ coNP`.

use crate::cdcl::Lit;
use crate::proof::Perm;
use std::collections::{BTreeSet, HashMap, HashSet};

/// A canonicalized CNF: clauses of `(var, polarity)` literals, each clause sorted+deduped, the
/// clause list sorted+deduped. The node currency of every cofactor DAG here.
pub type CanonClauses = Vec<Vec<(u32, bool)>>;

/// An explicit literal renaming `var → (var′, flip)`, stored on a quotient-DAG edge and verified
/// by the checker: it witnesses that a recomputed cofactor is `~`-equivalent to its stored child.
pub type Twist = Vec<(u32, u32, bool)>;

/// Canonicalize a CNF given as packed [`Lit`]s.
pub fn canon(clauses: &[Vec<Lit>]) -> CanonClauses {
    canon_raw(
        &clauses
            .iter()
            .map(|c| c.iter().map(|l| (l.var(), l.is_positive())).collect())
            .collect::<Vec<_>>(),
    )
}

/// Canonicalize a CNF given in raw `(var, polarity)` form.
pub fn canon_raw(clauses: &[Vec<(u32, bool)>]) -> CanonClauses {
    let mut out: CanonClauses = clauses
        .iter()
        .map(|c| {
            let mut lits = c.clone();
            lits.sort_unstable();
            lits.dedup();
            lits
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The Shannon cofactor `F|_{x=b}`: drop clauses satisfied by `x=b`, delete the `x` literal from the
/// rest, recanonicalize. An empty clause survives as `⊥` (the branch is UNSAT on the spot).
pub fn cofactor(clauses: &CanonClauses, x: u32, b: bool) -> CanonClauses {
    canon_raw(
        &clauses
            .iter()
            .filter(|c| !c.iter().any(|&(v, pos)| v == x && pos == b))
            .map(|c| c.iter().copied().filter(|&(v, _)| v != x).collect())
            .collect::<Vec<_>>(),
    )
}

/// A leaf: the clause set contains the empty clause `⊥`.
pub fn is_leaf(clauses: &CanonClauses) -> bool {
    clauses.iter().any(|c| c.is_empty())
}

// =============================================================================
// The strict distinct-cofactor DAG (fixed variable order, no renaming): OBDD width.
// =============================================================================

/// A node of the strict distinct-cofactor DAG.
#[derive(Clone, Debug)]
pub enum Node {
    /// The clause set contains `⊥` — UNSAT on the spot.
    Leaf(CanonClauses),
    /// Branch on `var`: children are the two Shannon cofactors (indices into the node vector).
    Internal { clauses: CanonClauses, var: u32, lo: usize, hi: usize },
}

/// Build the memoized strict distinct-cofactor DAG over the fixed order `0..n`. Identical cofactors
/// at the same depth SHARE a node. Returns `None` iff the formula is satisfiable (some fully
/// unfolded branch carries no `⊥`). The node count is the number of distinct cofactors — the OBDD
/// width — and is the honest exponential floor on the residue (no order keeps it small).
pub fn distinct_cofactor_dag(n: usize, clauses: &CanonClauses) -> Option<(usize, Vec<Node>)> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut memo: HashMap<(usize, CanonClauses), Option<usize>> = HashMap::new();
    fn go(
        depth: usize,
        n: usize,
        clauses: CanonClauses,
        nodes: &mut Vec<Node>,
        memo: &mut HashMap<(usize, CanonClauses), Option<usize>>,
    ) -> Option<usize> {
        if let Some(&hit) = memo.get(&(depth, clauses.clone())) {
            return hit;
        }
        let result = if clauses.iter().any(|c| c.is_empty()) {
            let id = nodes.len();
            nodes.push(Node::Leaf(clauses.clone()));
            Some(id)
        } else if depth == n {
            None
        } else {
            let x = depth as u32;
            let lo = go(depth + 1, n, cofactor(&clauses, x, false), nodes, memo);
            let hi = go(depth + 1, n, cofactor(&clauses, x, true), nodes, memo);
            match (lo, hi) {
                (Some(lo), Some(hi)) => {
                    let id = nodes.len();
                    nodes.push(Node::Internal { clauses: clauses.clone(), var: x, lo, hi });
                    Some(id)
                }
                _ => None,
            }
        };
        memo.insert((depth, clauses), result);
        result
    }
    let root = go(0, n, clauses.clone(), &mut nodes, &mut memo)?;
    Some((root, nodes))
}

/// **Zero-trust local checker** for the strict DAG: leaves carry `⊥`; every internal node's two
/// children are exactly its recomputed cofactors. A pass certifies the root UNSAT by structural
/// induction, in time linear in the DAG.
pub fn check_distinct_dag(root: usize, nodes: &[Node], expected: &CanonClauses) -> bool {
    match &nodes[root] {
        Node::Leaf(c) | Node::Internal { clauses: c, .. } if c != expected => return false,
        _ => {}
    }
    nodes.iter().all(|node| match node {
        Node::Leaf(c) => c.iter().any(|cl| cl.is_empty()),
        Node::Internal { clauses, var, lo, hi } => {
            let want_lo = cofactor(clauses, *var, false);
            let want_hi = cofactor(clauses, *var, true);
            let got = |id: usize| match &nodes[id] {
                Node::Leaf(c) => c,
                Node::Internal { clauses, .. } => clauses,
            };
            *got(*lo) == want_lo && *got(*hi) == want_hi
        }
    })
}

/// The per-level widths of the strict decision DAG: `w[i]` = number of distinct residual clause-sets
/// reachable after branching on variables `0..i`. The distinct-cofactor count is `Σ w[i]`.
pub fn level_widths(n: usize, root: &CanonClauses) -> Vec<usize> {
    let mut levels: Vec<HashSet<CanonClauses>> = vec![HashSet::new(); n + 1];
    let mut visited: HashSet<(usize, CanonClauses)> = HashSet::new();
    fn go(
        depth: usize,
        n: usize,
        clauses: CanonClauses,
        levels: &mut Vec<HashSet<CanonClauses>>,
        visited: &mut HashSet<(usize, CanonClauses)>,
    ) {
        if !visited.insert((depth, clauses.clone())) {
            return;
        }
        levels[depth].insert(clauses.clone());
        if clauses.iter().any(|c| c.is_empty()) || depth == n {
            return;
        }
        let x = depth as u32;
        go(depth + 1, n, cofactor(&clauses, x, false), levels, visited);
        go(depth + 1, n, cofactor(&clauses, x, true), levels, visited);
    }
    go(0, n, root.clone(), &mut levels, &mut visited);
    levels.iter().map(|s| s.len()).collect()
}

/// The strict distinct-cofactor set: every `(depth, cofactor)` pair reachable by prefix-restricting
/// the fixed order `0..n`. Its cardinality is [`distinct_width`]; quotienting it by a congruence can
/// only merge pairs, so every [`quotient_class_count`] is `≤ distinct_width` and coarser congruences
/// give monotonically fewer classes — both are theorems about quotients of this one fixed set, and
/// therefore hold on every formula.
pub fn cofactor_set(n: usize, clauses: &CanonClauses) -> BTreeSet<(usize, CanonClauses)> {
    fn go(depth: usize, n: usize, clauses: CanonClauses, set: &mut BTreeSet<(usize, CanonClauses)>) {
        if !set.insert((depth, clauses.clone())) {
            return;
        }
        if is_leaf(&clauses) || depth == n {
            return;
        }
        let x = depth as u32;
        go(depth + 1, n, cofactor(&clauses, x, false), set);
        go(depth + 1, n, cofactor(&clauses, x, true), set);
    }
    let mut set = BTreeSet::new();
    go(0, n, clauses.clone(), &mut set);
    set
}

/// The distinct-cofactor count — the OBDD width under the fixed order `0..n`. Equal to
/// `Σ (level widths)` and to `|cofactor_set|`.
pub fn distinct_width(n: usize, clauses: &CanonClauses) -> usize {
    cofactor_set(n, clauses).len()
}

/// **The collapse measure.** The number of `~`-classes among the strict distinct-cofactor set:
/// canonicalize every `(depth, cofactor)` under `~` and count distinct. Because it is a quotient of
/// the fixed set [`cofactor_set`], it satisfies — as *theorems*, for every formula —
/// `quotient_class_count(Raw) = distinct_width` and monotonicity under congruence coarsening
/// (`iso ≤ rename ≤ raw`). This is the clean instrument for "do exponentially-many cofactors fall
/// into polynomially-many classes?", with no dependence on any branching order.
pub fn quotient_class_count<C: Congruence + ?Sized>(
    n: usize,
    clauses: &CanonClauses,
    cong: &C,
) -> usize {
    cofactor_set(n, clauses)
        .into_iter()
        .map(|(d, c)| (d, cong.canonicalize(&c).0))
        .collect::<BTreeSet<_>>()
        .len()
}

// =============================================================================
// The quotient DAG: the cofactor DAG merged under a pluggable congruence.
// =============================================================================

/// An equivalence relation on cofactors, given by a canonical representative of each class plus the
/// [`Twist`] realizing the merge. To be a sound certificate substrate it must be a **congruence**:
/// its twists are literal renamings (bijection + polarity flips), which preserve (un)satisfiability,
/// so isomorphism-invariance of UNSAT makes the local check sound. Two cofactors are `~`-equivalent
/// iff [`canonicalize`](Congruence::canonicalize) returns the same representative.
pub trait Congruence {
    /// A short label for reporting (`"identity"`, `"Bn"`, `"AGL"`, `"cofactor-iso"`).
    fn name(&self) -> &str;
    /// The canonical representative of `clauses`'s class, and the twist mapping `clauses` onto it
    /// (i.e. `apply_twist(clauses, twist) == representative`).
    fn canonicalize(&self, clauses: &CanonClauses) -> (CanonClauses, Twist);
}

/// Deterministic name-normalization: rename variables by first appearance over the sorted clause
/// list, iterated to a fixpoint of (rename, sort). Returns the normalized set and the renaming.
pub fn normalize(clauses: &CanonClauses) -> (CanonClauses, Vec<(u32, u32)>) {
    let mut cur = clauses.clone();
    let mut total: HashMap<u32, u32> = HashMap::new();
    for c in clauses.iter().flatten() {
        total.entry(c.0).or_insert(c.0);
    }
    for _ in 0..3 {
        let mut next_name: u32 = 0;
        let mut ren: HashMap<u32, u32> = HashMap::new();
        for c in &cur {
            for &(v, _) in c {
                ren.entry(v).or_insert_with(|| {
                    let x = next_name;
                    next_name += 1;
                    x
                });
            }
        }
        let renamed: Vec<Vec<(u32, bool)>> =
            cur.iter().map(|c| c.iter().map(|&(v, p)| (ren[&v], p)).collect()).collect();
        let renamed = canon_raw(&renamed);
        for (_, tgt) in total.iter_mut() {
            if let Some(&t2) = ren.get(tgt) {
                *tgt = t2;
            }
        }
        if renamed == cur {
            break;
        }
        cur = renamed;
    }
    (cur.clone(), total.into_iter().collect())
}

/// Apply a [`Twist`] to a clause set; `None` if a live variable is unmapped.
pub fn apply_twist(clauses: &CanonClauses, twist: &Twist) -> Option<CanonClauses> {
    let map: HashMap<u32, (u32, bool)> = twist.iter().map(|&(a, b, f)| (a, (b, f))).collect();
    let mut out = Vec::new();
    for c in clauses {
        let mut nc = Vec::new();
        for &(v, pos) in c {
            let &(v2, f) = map.get(&v)?;
            nc.push((v2, pos ^ f));
        }
        out.push(nc);
    }
    Some(canon_raw(&out))
}

/// The group-canonical form of a clause set: the minimum, over every group element `g`, of the
/// name-normalized image — plus the twist realizing it. Two cofactors in the same `G`-orbit
/// canonicalize identically, whatever corner of the orbit they sit in.
pub fn group_canon(clauses: &CanonClauses, group: &[Perm]) -> (CanonClauses, Twist) {
    let mut best: Option<(CanonClauses, Twist)> = None;
    for g in group {
        let mapped: Vec<Vec<(u32, bool)>> = clauses
            .iter()
            .map(|c| {
                c.iter()
                    .map(|&(v, pos)| {
                        let img = g.apply(Lit::new(v, pos));
                        (img.var(), img.is_positive())
                    })
                    .collect()
            })
            .collect();
        let mapped = canon_raw(&mapped);
        let (normed, ren) = normalize(&mapped);
        let ren_map: HashMap<u32, u32> = ren.into_iter().collect();
        let twist: Twist = clauses
            .iter()
            .flatten()
            .map(|&(v, _)| {
                let img = g.apply(Lit::pos(v));
                (v, ren_map[&img.var()], !img.is_positive())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if best.as_ref().map_or(true, |(b, _)| normed < *b) {
            best = Some((normed, twist));
        }
    }
    best.unwrap_or_else(|| (clauses.clone(), Vec::new()))
}

/// The full CNF-isomorphism canonical form: the lex-least image over **all** relabelings of the live
/// variables and **all** polarity flips — not tied to any instance automorphism. Above `cap` live
/// variables it degrades gracefully to first-appearance normalization (the boundary is reported by
/// callers, never silently exceeded).
pub fn iso_canon(clauses: &CanonClauses, cap: usize) -> (CanonClauses, Twist) {
    let live: Vec<u32> = clauses
        .iter()
        .flatten()
        .map(|&(v, _)| v)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let k = live.len();
    if k == 0 {
        return (clauses.clone(), Vec::new());
    }
    if k > cap {
        let (normed, ren) = normalize(clauses);
        let twist: Twist = ren
            .into_iter()
            .map(|(a, b)| (a, b, false))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return (normed, twist);
    }
    let mut best: Option<(CanonClauses, Twist)> = None;
    for perm in permutations(k) {
        for flip_mask in 0u32..(1u32 << k) {
            let map: HashMap<u32, (u32, bool)> = (0..k)
                .map(|i| (live[i], (perm[i] as u32, (flip_mask >> i) & 1 == 1)))
                .collect();
            let mapped: Vec<Vec<(u32, bool)>> = clauses
                .iter()
                .map(|c| {
                    c.iter()
                        .map(|&(v, p)| {
                            let (v2, f) = map[&v];
                            (v2, p ^ f)
                        })
                        .collect()
                })
                .collect();
            let mapped = canon_raw(&mapped);
            if best.as_ref().map_or(true, |(b, _)| mapped < *b) {
                let twist: Twist = live
                    .iter()
                    .map(|&v| {
                        let (v2, f) = map[&v];
                        (v, v2, f)
                    })
                    .collect();
                best = Some((mapped, twist));
            }
        }
    }
    best.unwrap()
}

/// Unit-propagate a clause set to fixpoint: repeatedly assign a unit clause's literal (drop satisfied
/// clauses, delete the falsified literal), stopping at a derived `⊥` or when no unit remains. A sound
/// structural reduction (RUP-certified), monotone under relabeling.
pub fn unit_propagate(clauses: &CanonClauses) -> CanonClauses {
    let mut cur = clauses.clone();
    while let Some(&[(v, p)]) = cur.iter().find(|c| c.len() == 1).map(|c| c.as_slice()) {
        cur = canon_raw(
            &cur.iter()
                .filter(|c| !c.iter().any(|&(vv, pp)| vv == v && pp == p))
                .map(|c| c.iter().copied().filter(|&(vv, _)| vv != v).collect())
                .collect::<Vec<_>>(),
        );
        if cur.iter().any(|c| c.is_empty()) {
            break;
        }
    }
    cur
}

/// Eliminate pure literals: a variable occurring in only one polarity is satisfied, dropping every
/// clause it appears in. SAT-equivalence-preserving, monotone under relabeling.
fn pure_eliminate(clauses: &CanonClauses) -> CanonClauses {
    let mut pos: HashSet<u32> = HashSet::new();
    let mut neg: HashSet<u32> = HashSet::new();
    for &(v, p) in clauses.iter().flatten() {
        if p {
            pos.insert(v);
        } else {
            neg.insert(v);
        }
    }
    let pure: HashSet<u32> = pos.symmetric_difference(&neg).copied().collect();
    if pure.is_empty() {
        return clauses.clone();
    }
    canon_raw(
        &clauses
            .iter()
            .filter(|c| !c.iter().any(|&(v, _)| pure.contains(&v)))
            .cloned()
            .collect::<Vec<_>>(),
    )
}

/// Remove subsumed clauses: drop `C` whenever some strictly shorter `D ⊊ C` exists (canon has already
/// deduped equal clauses, so only strict supersets are removed). SAT-equivalence-preserving.
fn subsume(clauses: &CanonClauses) -> CanonClauses {
    canon_raw(
        &clauses
            .iter()
            .filter(|c| {
                let cset: BTreeSet<(u32, bool)> = c.iter().copied().collect();
                !clauses.iter().any(|d| d.len() < c.len() && d.iter().all(|l| cset.contains(l)))
            })
            .cloned()
            .collect::<Vec<_>>(),
    )
}

/// A sound poly reduction to fixpoint: unit propagation, then pure-literal elimination, then
/// subsumption, iterated. Each step preserves (un)satisfiability and commutes with relabeling, so
/// `reduce(π(F)) = π(reduce(F))` — which makes [`ReduceIso`] a legitimate coarsening of [`UnitPropIso`].
pub fn reduce(clauses: &CanonClauses) -> CanonClauses {
    let mut cur = clauses.clone();
    loop {
        let next = subsume(&pure_eliminate(&unit_propagate(&cur)));
        if is_leaf(&next) || next == cur {
            return next;
        }
        cur = next;
    }
}

/// Every permutation of `0..k` (k! of them). `k` is the live-variable count of a small cofactor.
fn permutations(k: usize) -> Vec<Vec<usize>> {
    let items: Vec<usize> = (0..k).collect();
    let mut out = Vec::new();
    fn rec(remaining: &[usize], acc: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if remaining.is_empty() {
            out.push(acc.clone());
            return;
        }
        for i in 0..remaining.len() {
            let mut rest = remaining.to_vec();
            let x = rest.remove(i);
            acc.push(x);
            rec(&rest, acc, out);
            acc.pop();
        }
    }
    rec(&items, &mut Vec::new(), &mut out);
    out
}

/// Strict equality — no canonicalization at all (the identity twist). The finest rung: it merges
/// nothing, so `quotient_class_count(Raw) == distinct_width` exactly.
pub struct Raw;

impl Congruence for Raw {
    fn name(&self) -> &str {
        "raw"
    }
    fn canonicalize(&self, clauses: &CanonClauses) -> (CanonClauses, Twist) {
        let twist: Twist = clauses
            .iter()
            .flatten()
            .map(|&(v, _)| (v, v, false))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        (clauses.clone(), twist)
    }
}

/// First-appearance renaming (trivial group). One rung coarser than [`Raw`]: it merges cofactors
/// that coincide after relabeling variables by order of first appearance.
pub struct Rename;

impl Congruence for Rename {
    fn name(&self) -> &str {
        "rename"
    }
    fn canonicalize(&self, clauses: &CanonClauses) -> (CanonClauses, Twist) {
        // First-appearance renaming with the trivial group — the pure-renaming twist (no flips).
        // Equal to `group_canon(clauses, &[Perm::identity(width)])`, without the Perm-sizing trap.
        let (normed, ren) = normalize(clauses);
        let ren_map: HashMap<u32, u32> = ren.into_iter().collect();
        let twist: Twist = clauses
            .iter()
            .flatten()
            .map(|&(v, _)| (v, ren_map[&v], false))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        (normed, twist)
    }
}

/// Merge cofactors lying in the same orbit under an instance-symmetry group (`Bₙ`, `AGL`, …).
pub struct GroupInduced {
    pub group: Vec<Perm>,
    pub label: String,
}

impl Congruence for GroupInduced {
    fn name(&self) -> &str {
        &self.label
    }
    fn canonicalize(&self, clauses: &CanonClauses) -> (CanonClauses, Twist) {
        group_canon(clauses, &self.group)
    }
}

/// Merge cofactors that are CNF-isomorphic (any relabeling + polarity flip) — the strongest
/// decidable rung, the symmetry that need not live in the instance. `cap` bounds the brute force.
pub struct CofactorIso {
    pub cap: usize,
}

impl Congruence for CofactorIso {
    fn name(&self) -> &str {
        "cofactor-iso"
    }
    fn canonicalize(&self, clauses: &CanonClauses) -> (CanonClauses, Twist) {
        iso_canon(clauses, self.cap)
    }
}

/// Unit-propagate to fixpoint, then canonicalize up to CNF-isomorphism — strictly coarser than
/// [`CofactorIso`] (cofactors that reduce to isomorphic residuals after propagation merge, and all
/// unit-refutable cofactors collapse into one). A **collapse-measure** congruence only: unit
/// propagation is a structural reduction, not a relabeling, so the returned twist is the iso-twist of
/// the *reduced* form — valid for [`quotient_class_count`] (which uses only the canonical key), not for
/// the twist-certificate [`quotient_dag`] (a propagation merge is certified by RUP instead).
pub struct UnitPropIso {
    pub cap: usize,
}

impl Congruence for UnitPropIso {
    fn name(&self) -> &str {
        "unitprop-iso"
    }
    fn canonicalize(&self, clauses: &CanonClauses) -> (CanonClauses, Twist) {
        iso_canon(&unit_propagate(clauses), self.cap)
    }
}

/// Apply the full sound reduction ([`reduce`]: unit-prop + pure-literal + subsumption to fixpoint),
/// then canonicalize up to CNF-isomorphism — one rung coarser than [`UnitPropIso`]. A collapse-measure
/// congruence (the reduction is structural, RUP/reduction-certified, not a relabeling), for
/// [`quotient_class_count`]. The ladder: `reduce-iso ≤ unitprop-iso ≤ iso ≤ rename ≤ raw = distinct`.
pub struct ReduceIso {
    pub cap: usize,
}

impl Congruence for ReduceIso {
    fn name(&self) -> &str {
        "reduce-iso"
    }
    fn canonicalize(&self, clauses: &CanonClauses) -> (CanonClauses, Twist) {
        iso_canon(&reduce(clauses), self.cap)
    }
}

/// The algebraic / **non-resolution** dispatcher routes — the ones that refute where resolution is
/// exponential (parity/GF(2), mod-`p`, exact-cover counting, algebraic collapse). Distinguished from
/// the resolution-simulatable specialists (2-SAT, Horn) so the non-resolution rung can be isolated.
fn is_non_resolution_route(r: &crate::solve::Route) -> bool {
    use crate::solve::Route::*;
    matches!(r, Parity | ModP | ModM | ExactCover | Collapse | HybridXor | Sos | Nullstellensatz | Pigeonhole)
}

/// A sentinel canonical form for "crushed by a non-resolution specialist" — a clause over the
/// impossible variable `u32::MAX`, distinct from every real (reduced) cofactor and from `⊥`.
fn non_res_crushed() -> CanonClauses {
    vec![vec![(u32::MAX, true)]]
}

/// The reduction rung **fused with the non-resolution crushers as terminals**: `reduce` to fixpoint,
/// then if a *non-resolution* specialist (parity/GF(2), mod-`p`, exact-cover, algebraic collapse)
/// refutes the reduced cofactor, collapse it to one "crushed" class; otherwise canonicalize up to iso.
/// Coarser than [`ReduceIso`] (all non-resolution-crushable reduced cofactors merge into one), so
/// `struct-reduce-iso ≤ reduce-iso`. This is the rung that can beat the Chvátal–Szemerédi resolution
/// cap — GF(2)/mod-`p` are polynomial exactly where resolution is exponential — measured via
/// [`quotient_class_count`] (specialist verdicts certified by their routes; reduce by RUP).
pub struct StructuredReduceIso {
    pub cap: usize,
}

impl Congruence for StructuredReduceIso {
    fn name(&self) -> &str {
        "struct-reduce-iso"
    }
    fn canonicalize(&self, clauses: &CanonClauses) -> (CanonClauses, Twist) {
        let r = reduce(clauses);
        match structured_leaf(&r) {
            Some(route) if is_non_resolution_route(&route) => (non_res_crushed(), Vec::new()),
            _ => iso_canon(&r, self.cap),
        }
    }
}

/// A node of the quotient DAG. Edges carry the [`Twist`] that maps the recomputed cofactor onto the
/// stored child representative.
#[derive(Clone, Debug)]
pub enum QNode {
    Leaf(CanonClauses),
    Internal { clauses: CanonClauses, var: u32, lo: usize, hi: usize, lo_twist: Twist, hi_twist: Twist },
}

/// The cofactor DAG quotiented by a [`Congruence`]: `root` node index, `nodes` (one per class), and
/// `visits` (the memoized recursion's work — output-sensitive: linear in the collapse found).
pub struct QuotientDag {
    pub root: usize,
    pub nodes: Vec<QNode>,
    pub visits: usize,
}

impl QuotientDag {
    /// The **quotient width** `W(F, ~)` — the number of `~`-classes, i.e. the certificate size.
    pub fn width(&self) -> usize {
        self.nodes.len()
    }
}

/// Build the cofactor DAG quotiented by `cong`: memoize on the canonical form, branch on the first
/// live variable of the (canonicalized) subproblem, and merge `~`-equivalent cofactors, recording
/// the realizing twist on each edge. Returns `None` iff the formula is satisfiable.
pub fn quotient_dag<C: Congruence + ?Sized>(
    n: usize,
    clauses: &CanonClauses,
    cong: &C,
) -> Option<QuotientDag> {
    let mut nodes: Vec<QNode> = Vec::new();
    let mut memo: HashMap<(usize, CanonClauses), Option<usize>> = HashMap::new();
    fn go<C: Congruence + ?Sized>(
        depth: usize,
        n: usize,
        clauses: CanonClauses,
        nodes: &mut Vec<QNode>,
        memo: &mut HashMap<(usize, CanonClauses), Option<usize>>,
        cong: &C,
    ) -> Option<usize> {
        if let Some(&hit) = memo.get(&(depth, clauses.clone())) {
            return hit;
        }
        let result = if clauses.iter().any(|c| c.is_empty()) {
            let id = nodes.len();
            nodes.push(QNode::Leaf(clauses.clone()));
            Some(id)
        } else if clauses.is_empty() || depth > n {
            None
        } else {
            let x = clauses.iter().flatten().map(|&(v, _)| v).min().unwrap();
            let mut children: Vec<(usize, Twist)> = Vec::new();
            let mut ok = true;
            for b in [false, true] {
                let cof = cofactor(&clauses, x, b);
                let (cn, twist) = cong.canonicalize(&cof);
                match go(depth + 1, n, cn, nodes, memo, cong) {
                    Some(id) => children.push((id, twist)),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                let id = nodes.len();
                let (lo, lo_twist) = children[0].clone();
                let (hi, hi_twist) = children[1].clone();
                nodes.push(QNode::Internal { clauses: clauses.clone(), var: x, lo, hi, lo_twist, hi_twist });
                Some(id)
            } else {
                None
            }
        };
        memo.insert((depth, clauses), result);
        result
    }
    let (root_canon, _) = cong.canonicalize(clauses);
    let root = go(0, n, root_canon, &mut nodes, &mut memo, cong)?;
    let visits = memo.len();
    Some(QuotientDag { root, nodes, visits })
}

/// The quotient width `W(F, ~)` — `None` iff satisfiable.
pub fn quotient_width<C: Congruence + ?Sized>(
    n: usize,
    clauses: &CanonClauses,
    cong: &C,
) -> Option<usize> {
    quotient_dag(n, clauses, cong).map(|d| d.width())
}

/// **Zero-trust checker with twist verification**: leaves carry `⊥`; each internal node's recomputed
/// cofactor, pushed through the stored twist, must equal the child EXACTLY. A pass certifies the
/// root UNSAT by structural induction (unsatisfiability is isomorphism-invariant and each twist is a
/// literal renaming), in time linear in the quotient DAG.
pub fn check_quotient_dag(nodes: &[QNode]) -> bool {
    nodes.iter().all(|node| match node {
        QNode::Leaf(c) => c.iter().any(|cl| cl.is_empty()),
        QNode::Internal { clauses, var, lo, hi, lo_twist, hi_twist } => {
            let child = |id: usize| match &nodes[id] {
                QNode::Leaf(c) => c,
                QNode::Internal { clauses, .. } => clauses,
            };
            let ok = |b: bool, id: usize, tw: &Twist| {
                apply_twist(&cofactor(clauses, *var, b), tw).map_or(false, |t| t == *child(id))
            };
            ok(false, *lo, lo_twist) && ok(true, *hi, hi_twist)
        }
    })
}

// =============================================================================
// The structured-leaf cofactor DAG: every specialist crusher becomes a leaf.
// =============================================================================

fn to_lits(clauses: &CanonClauses) -> Vec<Vec<Lit>> {
    clauses.iter().map(|c| c.iter().map(|&(v, p)| Lit::new(v, p)).collect()).collect()
}

/// A cofactor is a **structured leaf** if a decidable specialist refutes it — the full dispatcher
/// ([`crate::solve::solve_structured`]) returns UNSAT via a route that is *not* raw CDCL or the
/// certified-no-shortcut `Incompressible` verdict. So GF(2)/parity (Tseitin), mod-`p` (mod-`p`
/// Tseitin), counting (pigeonhole/exact-cover), 2-SAT, Horn — every nut the paper already cracks —
/// is a leaf here, certified internally by the route it fired. Returns that route, or `None` if no
/// specialist crushes the cofactor (it must be branched, or it is `⊥`/satisfiable).
pub fn structured_leaf(clauses: &CanonClauses) -> Option<crate::solve::Route> {
    if is_leaf(clauses) {
        return None; // ⊥ is a trivial leaf, handled separately
    }
    let nv = clauses.iter().flatten().map(|&(v, _)| v as usize + 1).max().unwrap_or(0);
    if nv == 0 {
        return None; // no clauses / no variables — satisfiable, not a refutation
    }
    // The O(clauses) specialist chain only — NO CDCL fallback. A per-node leaf check must be cheap, and
    // we already reject the `Cdcl` route, so calling `structured_prefix` (which returns `None` instead
    // of running the search) is behavior-identical and avoids a full CDCL solve at every cofactor.
    let solved = crate::solve::structured_prefix(nv, &to_lits(clauses))?;
    match solved.answer {
        crate::solve::Answer::Unsat
            if !matches!(solved.via, crate::solve::Route::Cdcl | crate::solve::Route::Incompressible) =>
        {
            Some(solved.via)
        }
        _ => None,
    }
}

/// A node of the structured-leaf cofactor DAG.
#[derive(Clone, Debug)]
pub enum SNode {
    /// The clause set contains `⊥` — UNSAT on the spot.
    Trivial(CanonClauses),
    /// A specialist route refutes this cofactor (`route` fired); certified internally by that route.
    Structured { clauses: CanonClauses, route: crate::solve::Route },
    /// Branch on `var`: children are the two Shannon cofactors.
    Internal { clauses: CanonClauses, var: u32, lo: usize, hi: usize },
}

impl SNode {
    fn clauses(&self) -> &CanonClauses {
        match self {
            SNode::Trivial(c) | SNode::Structured { clauses: c, .. } | SNode::Internal { clauses: c, .. } => c,
        }
    }
}

/// The cofactor DAG cut off at structured leaves: the whole dispatcher, organized as a decision DAG.
/// Its size is the number of cofactors that must be branched before a specialist (or `⊥`) fires —
/// `1` exactly when the root itself is crushed, growing only where no specialist ever fires (the
/// wall). Reported `structured`/`trivial` leaf counts show which crushers carried the refutation.
pub struct StructuredDag {
    pub root: usize,
    pub nodes: Vec<SNode>,
}

impl StructuredDag {
    /// Certificate size — the number of DAG nodes.
    pub fn size(&self) -> usize {
        self.nodes.len()
    }
    /// Leaves closed by a specialist route (rather than by branching to `⊥`).
    pub fn structured_leaves(&self) -> usize {
        self.nodes.iter().filter(|n| matches!(n, SNode::Structured { .. })).count()
    }
}

/// Build the structured-leaf cofactor DAG over the fixed order `0..n`: a node is a leaf when it is
/// `⊥` or when a specialist crushes it ([`structured_leaf`]); otherwise it branches. Identical
/// cofactors at the same depth share a node. `None` iff the formula is satisfiable.
pub fn structured_leaf_dag(n: usize, clauses: &CanonClauses) -> Option<StructuredDag> {
    let mut nodes: Vec<SNode> = Vec::new();
    let mut memo: HashMap<(usize, CanonClauses), Option<usize>> = HashMap::new();
    fn go(
        depth: usize,
        n: usize,
        clauses: CanonClauses,
        nodes: &mut Vec<SNode>,
        memo: &mut HashMap<(usize, CanonClauses), Option<usize>>,
    ) -> Option<usize> {
        if let Some(&hit) = memo.get(&(depth, clauses.clone())) {
            return hit;
        }
        let result = if is_leaf(&clauses) {
            let id = nodes.len();
            nodes.push(SNode::Trivial(clauses.clone()));
            Some(id)
        } else if let Some(route) = structured_leaf(&clauses) {
            let id = nodes.len();
            nodes.push(SNode::Structured { clauses: clauses.clone(), route });
            Some(id)
        } else if depth == n {
            None
        } else {
            let x = depth as u32;
            let lo = go(depth + 1, n, cofactor(&clauses, x, false), nodes, memo);
            let hi = go(depth + 1, n, cofactor(&clauses, x, true), nodes, memo);
            match (lo, hi) {
                (Some(lo), Some(hi)) => {
                    let id = nodes.len();
                    nodes.push(SNode::Internal { clauses: clauses.clone(), var: x, lo, hi });
                    Some(id)
                }
                _ => None,
            }
        };
        memo.insert((depth, clauses), result);
        result
    }
    let root = go(0, n, clauses.clone(), &mut nodes, &mut memo)?;
    Some(StructuredDag { root, nodes })
}

/// **The checker**: `⊥` leaves carry the empty clause; structured leaves re-fire their specialist
/// (the dispatcher's route re-checks internally — an idempotent re-verification); internal nodes'
/// children are exactly their recomputed Shannon cofactors. The internal skeleton is fully zero-trust;
/// the structured leaves are dispatcher-certified (extracting each route's own witness for end-to-end
/// zero trust is the next hardening).
pub fn check_structured_dag(nodes: &[SNode]) -> bool {
    nodes.iter().all(|node| match node {
        SNode::Trivial(c) => is_leaf(c),
        SNode::Structured { clauses, .. } => structured_leaf(clauses).is_some(),
        SNode::Internal { clauses, var, lo, hi } => {
            nodes[*lo].clauses() == &cofactor(clauses, *var, false)
                && nodes[*hi].clauses() == &cofactor(clauses, *var, true)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hypercube::{minimal_cover_orbits, php_perm_symmetries};

    fn php_canon(m: usize) -> (usize, CanonClauses) {
        let (php, _) = crate::families::php(m);
        (php.num_vars, canon(&php.clauses))
    }

    /// BFS closure of the pigeonhole symmetry generators (the small full group).
    fn php_group(m: usize) -> Vec<Perm> {
        let nv = m * (m - 1);
        let gens = php_perm_symmetries(m);
        let key = |p: &Perm| -> Vec<u32> { (0..nv).map(|v| p.apply(Lit::pos(v as u32)).var()).collect() };
        let id = Perm::identity(nv);
        let mut seen: BTreeSet<Vec<u32>> = [key(&id)].into_iter().collect();
        let mut group = vec![id.clone()];
        let mut frontier = vec![id];
        while let Some(p) = frontier.pop() {
            for g in &gens {
                let q = p.compose(g);
                if seen.insert(key(&q)) {
                    group.push(q.clone());
                    frontier.push(q);
                }
            }
        }
        group
    }

    fn xor_cycle(k: usize) -> CanonClauses {
        let mut raw: Vec<Vec<(u32, bool)>> = Vec::new();
        for i in 0..k {
            let j = (i + 1) % k;
            raw.push(vec![(i as u32, true), (j as u32, true)]);
            raw.push(vec![(i as u32, false), (j as u32, false)]);
        }
        canon_raw(&raw)
    }

    /// **The strict distinct-cofactor DAG is a total, locally-checkable certificate on the census,
    /// and its width is linear on the bounded-width XOR family** — pinning the promoted [`level_widths`]
    /// / [`distinct_cofactor_dag`] against the `tests/` prototypes verbatim.
    #[test]
    fn distinct_cofactor_dag_matches_the_prototype_and_the_checker_has_teeth() {
        let n = 3usize;
        for cover in minimal_cover_orbits(n) {
            let clauses = canon(&cover.clauses());
            let (root, nodes) = distinct_cofactor_dag(n, &clauses).expect("every UNSAT family unfolds");
            assert!(check_distinct_dag(root, &nodes, &clauses), "the strict DAG re-checks");
        }
        // SAT side refuses.
        let sat = canon(&[vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(2)]]);
        assert!(distinct_cofactor_dag(n, &sat).is_none(), "a satisfiable formula has no DAG");
        // Teeth: swap an internal node's children.
        let (root, mut nodes) = distinct_cofactor_dag(3, &xor_cycle(3)).unwrap();
        let internal = nodes
            .iter()
            .position(|nd| matches!(nd, Node::Internal { lo, hi, .. } if lo != hi))
            .unwrap();
        if let Node::Internal { lo, hi, .. } = &mut nodes[internal] {
            std::mem::swap(lo, hi);
        }
        assert!(!check_distinct_dag(root, &nodes, &xor_cycle(3)), "a corrupted DAG is rejected");
        // The width identity + constant-width XOR family (⟹ O(n) distinct-cofactor count).
        let widths: Vec<usize> = [5usize, 7, 9, 11, 13]
            .iter()
            .map(|&k| *level_widths(k, &xor_cycle(k)).iter().max().unwrap())
            .collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "XOR cycle max width constant: {widths:?}");
    }

    /// **The quotient DAG reproduces the `symmetric_dag_fusion` ratchets exactly** — the promotion is
    /// behavior-preserving. Rename (first-appearance renaming) gives the plain toll; the PHP group
    /// compounds the crush; every DAG re-checks; a corrupted twist is rejected.
    #[test]
    fn quotient_dag_reproduces_the_locked_pigeonhole_ratchets() {
        // (m, plain nodes under Rename, fused nodes under the PHP group) — locked 2026-07-03.
        for &(m, plain_expected, fused_expected) in &[(3usize, 25usize, 18usize), (4, 103, 60)] {
            let (nv, clauses) = php_canon(m);
            let plain = quotient_dag(nv, &clauses, &Rename).expect("PHP unfolds");
            let group = GroupInduced { group: php_group(m), label: "php-sym".into() };
            let fused = quotient_dag(nv, &clauses, &group).expect("PHP unfolds under the group");
            assert!(check_quotient_dag(&plain.nodes), "m={m}: plain re-checks");
            assert!(check_quotient_dag(&fused.nodes), "m={m}: fused re-checks, twists verified");
            assert_eq!(plain.width(), plain_expected, "m={m}: plain quotient width is locked");
            assert_eq!(fused.width(), fused_expected, "m={m}: fused quotient width is locked");
            assert!(fused.width() < plain.width(), "m={m}: the group compounds the crush");
            // Output-sensitivity: the finder's work is linear in the collapse it finds.
            assert!(fused.visits <= 2 * fused.width() + 2 * nv + 2, "m={m}: work linear in output");
        }
        // Teeth: corrupt one twist.
        let (nv, clauses) = php_canon(3);
        let group = GroupInduced { group: php_group(3), label: "php-sym".into() };
        let mut dag = quotient_dag(nv, &clauses, &group).unwrap();
        let victim = dag
            .nodes
            .iter()
            .position(|n| matches!(n, QNode::Internal { lo_twist, .. } if !lo_twist.is_empty()))
            .expect("a nontrivial twist exists");
        if let QNode::Internal { lo_twist, .. } = &mut dag.nodes[victim] {
            lo_twist[0].2 = !lo_twist[0].2;
        }
        assert!(!check_quotient_dag(&dag.nodes), "a corrupted twist is rejected");
    }

    /// **The cofactor-class ladder is monotone and floored by the distinct count — as theorems.**
    /// `quotient_class_count` quotients the one fixed [`cofactor_set`], so `Raw` recovers the
    /// distinct-cofactor floor exactly and every coarsening (`Rename`, then `CofactorIso`) can only
    /// merge: `iso ≤ rename ≤ raw = distinct`, holding on every formula. This is the clean measure of
    /// "do exponentially-many cofactors fall into polynomially-many classes?"; the certificate DAG is
    /// verified sound alongside (soundness is independent of the size relationship the old assertion
    /// wrongly conflated with this one).
    #[test]
    fn the_cofactor_class_ladder_is_monotone_and_bounded_by_the_distinct_floor() {
        for m in [3usize, 4] {
            let (nv, clauses) = php_canon(m);
            let distinct = distinct_width(nv, &clauses);
            let raw = quotient_class_count(nv, &clauses, &Raw);
            let rename = quotient_class_count(nv, &clauses, &Rename);
            let iso = quotient_class_count(nv, &clauses, &CofactorIso { cap: 6 });
            let unitprop = quotient_class_count(nv, &clauses, &UnitPropIso { cap: 6 });
            let reduceiso = quotient_class_count(nv, &clauses, &ReduceIso { cap: 6 });
            // Raw = strict equality: exactly the distinct-cofactor floor. And Σ level widths agrees.
            assert_eq!(raw, distinct, "m={m}: Raw class count == distinct_width");
            assert_eq!(level_widths(nv, &clauses).iter().sum::<usize>(), distinct, "m={m}: Σ widths");
            // MONOTONE — coarser congruence, never more classes: a theorem about quotients of one
            // fixed set, so it must hold on every formula (the residue included). The full ladder:
            // reduce-iso ≤ unitprop ≤ iso ≤ rename ≤ raw = distinct.
            assert!(rename <= raw, "m={m}: rename ≤ raw ({rename} ≤ {raw})");
            assert!(iso <= rename, "m={m}: iso ≤ rename ({iso} ≤ {rename})");
            assert!(unitprop <= iso, "m={m}: unitprop ≤ iso ({unitprop} ≤ {iso})");
            assert!(reduceiso <= unitprop, "m={m}: reduce-iso ≤ unitprop ({reduceiso} ≤ {unitprop})");
            assert!(iso <= distinct, "m={m}: every class count ≤ the distinct floor ({iso} ≤ {distinct})");
            // The certificate DAG re-checks with zero trust (soundness, independent of size).
            let dag = quotient_dag(nv, &clauses, &CofactorIso { cap: 6 }).expect("PHP unfolds under iso");
            assert!(check_quotient_dag(&dag.nodes), "m={m}: the iso certificate re-checks");
            eprintln!(
                "cofactor-classes[PHP({m})]: distinct {distinct} → rename {rename} → iso {iso} \
                 (certificate DAG {} nodes, re-checked)",
                dag.width()
            );
        }
    }
}
