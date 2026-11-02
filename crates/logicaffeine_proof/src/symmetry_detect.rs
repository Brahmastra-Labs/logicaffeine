//! General symmetry detection for CNF — the engine behind certified symmetry breaking.
//!
//! A symmetry of a formula is a literal permutation `σ` (respecting negation) that maps the
//! clause set onto itself: `σ(F) = F`. Such a `σ` permutes models, so adding a lex-leader
//! predicate that keeps the canonical representative of each orbit is satisfiability-preserving
//! — and, with the witness `σ`, independently certifiable by the PR checker ([`crate::pr`]).
//!
//! The cornerstone is the **soundness gate** [`perm_is_automorphism`]: whatever search produces
//! a candidate generator, it is re-verified here by clause-multiset invariance before any use.
//! A detector that emits a non-symmetry is caught and dropped; a detector that misses a symmetry
//! only costs search speed. So the intricate part — finding generators — is never
//! soundness-critical; this gate is.

use crate::cdcl::Lit;
use crate::proof::Perm;

/// Canonical key of a single clause: its `(var, sign)` codes sorted and deduped. Two clauses are
/// equal as sets of literals iff their keys match.
pub(crate) fn clause_key(c: &[Lit]) -> Vec<u32> {
    let mut k: Vec<u32> = c
        .iter()
        .map(|l| l.var() * 2 + u32::from(!l.is_positive()))
        .collect();
    k.sort_unstable();
    k.dedup();
    k
}

/// Is `sigma` a genuine automorphism of `clauses` — does applying it map the clause set exactly
/// onto itself? The independent re-verification every generator must pass.
///
/// `σ` is an automorphism iff `σ(C) ∈ F` for every clause `C ∈ F` (a literal-permutation is a
/// bijection on clauses, so mapping `F` into itself forces it *onto* itself). A clause whose
/// variables are all **fixed** by `σ` maps to itself and is trivially present — so only clauses
/// touching `σ`'s *support* (its moved variables) need a membership check. For the small-support
/// generators symmetry breaking actually uses (transpositions, short cycles) this inspects a
/// handful of clauses instead of re-sorting the whole database, which is the difference between
/// an `O(n⁴)` and an `O(n³)` certified pigeonhole refutation.
pub fn perm_is_automorphism(clauses: &[Vec<Lit>], sigma: &Perm) -> bool {
    let nv = sigma.num_vars();
    let moved: Vec<bool> = (0..nv)
        .map(|v| sigma.apply(Lit::pos(v as u32)) != Lit::pos(v as u32))
        .collect();
    // Nothing moves ⇒ identity ⇒ automorphism of anything.
    if moved.iter().all(|&m| !m) {
        return true;
    }
    let set: std::collections::HashSet<Vec<u32>> = clauses.iter().map(|c| clause_key(c)).collect();
    for c in clauses {
        let touches_support = c
            .iter()
            .any(|l| (l.var() as usize) < nv && moved[l.var() as usize]);
        if touches_support && !set.contains(&clause_key(&sigma.apply_clause(c))) {
            return false;
        }
    }
    true
}

/// An **incrementally-maintained** clause database for fast repeated automorphism checks.
///
/// A certified symmetry-breaking refutation re-verifies an automorphism `σ` against the *current*
/// database once per step, but the database only grows by one clause each step. Rebuilding the
/// membership set from scratch every call (as the stateless [`perm_is_automorphism`] must) makes
/// the whole refutation `O(n⁴)`; maintaining the set — and a variable→clause occurrence map —
/// incrementally drops each check to `O(support of σ)`, and the refutation to `O(n³)`.
///
/// The verdict is identical to [`perm_is_automorphism`] (a differential fuzz proves it); this is a
/// pure acceleration structure, sound by construction: `σ` is an automorphism iff `σ(C) ∈ F` for
/// every clause `C` it moves, and a clause disjoint from `σ`'s support maps to itself.
pub struct AutomorphismIndex {
    nv: usize,
    clauses: Vec<Vec<Lit>>,
    keys: std::collections::HashSet<Vec<u32>>,
    /// Clause key → an index of a clause with that key, so an involution `σ` can mark the partner
    /// `σ(C)` of a just-verified clause `C` as already-checked (`σ(σ(C)) = C`, so the reverse
    /// direction is free) — halving the automorphism re-verification on transposition generators.
    index_by_key: std::collections::HashMap<Vec<u32>, usize>,
    by_var: Vec<Vec<usize>>,
    /// The **persistent** unit-propagation fixpoint of every inserted clause — the literals forced
    /// regardless of any per-call assumption. Maintained incrementally on `insert`, so propagation
    /// never re-derives the standing units (which on pigeonhole number `O(n²)` and would cost
    /// `O(n⁴)` to re-seed every step).
    base: Vec<Option<bool>>,
    /// Whether the standing clauses already unit-propagate to a conflict (the database is refuted
    /// by unit propagation alone, independent of any assumption).
    base_conflict: bool,
    /// Stamped per-call scratch layered over `base`: variable `v` is temporarily assigned iff
    /// `temp_gen[v] == gen`, with value `temp_val[v]`. Bumping `gen` resets the whole layer in
    /// `O(1)`, so a propagation call costs `O(touched)`, never `O(num_vars)`.
    temp_gen: Vec<u32>,
    temp_val: Vec<bool>,
    gen: u32,
    /// Stamped visited marks for `is_automorphism`, reset the same `O(1)` way.
    visited_gen: Vec<u32>,
    vgen: u32,
}

impl AutomorphismIndex {
    /// An empty index over `nv` variables.
    pub fn new(nv: usize) -> Self {
        AutomorphismIndex {
            nv,
            clauses: Vec::new(),
            keys: std::collections::HashSet::new(),
            index_by_key: std::collections::HashMap::new(),
            by_var: vec![Vec::new(); nv],
            base: vec![None; nv],
            base_conflict: false,
            temp_gen: vec![0; nv],
            temp_val: vec![false; nv],
            gen: 0,
            visited_gen: Vec::new(),
            vgen: 0,
        }
    }

    /// An index pre-loaded with `clauses`.
    pub fn with_clauses(nv: usize, clauses: &[Vec<Lit>]) -> Self {
        let mut ix = Self::new(nv);
        for c in clauses {
            ix.insert(c.clone());
        }
        ix
    }

    /// Add one clause, updating the membership set, the occurrence lists, and the persistent
    /// unit-propagation base (which may cascade) — all amortized over the life of the index.
    pub fn insert(&mut self, clause: Vec<Lit>) {
        let idx = self.clauses.len();
        let key = clause_key(&clause);
        self.keys.insert(key.clone());
        self.index_by_key.insert(key, idx);
        let mut touched: Vec<usize> = clause
            .iter()
            .map(|l| l.var() as usize)
            .filter(|&v| v < self.nv)
            .collect();
        touched.sort_unstable();
        touched.dedup();
        for v in touched {
            self.by_var[v].push(idx);
        }
        self.visited_gen.push(0);
        self.clauses.push(clause);
        // Fold the new clause into the persistent UP fixpoint: if it is (or becomes) unit under
        // `base` it forces a literal, which may cascade to other clauses sharing that variable.
        self.base_propagate(idx);
    }

    /// Incremental base unit-propagation seeded by a freshly-inserted clause index. Reads `base`,
    /// writes `base`, never allocates per propagated literal beyond the worklist.
    fn base_propagate(&mut self, start: usize) {
        if self.base_conflict {
            return;
        }
        let mut wl = vec![start];
        let mut wi = 0;
        while wi < wl.len() {
            let ci = wl[wi];
            wi += 1;
            let mut satisfied = false;
            let mut unit: Option<Lit> = None;
            let mut more_than_one = false;
            for &l in &self.clauses[ci] {
                let val = self.base[l.var() as usize].map(|b| b == l.is_positive());
                match val {
                    Some(true) => {
                        satisfied = true;
                        break;
                    }
                    Some(false) => {}
                    None => match unit {
                        None => unit = Some(l),
                        Some(u) if u == l => {}
                        Some(u) if u == l.negated() => {
                            satisfied = true;
                            break;
                        }
                        Some(_) => more_than_one = true,
                    },
                }
            }
            if satisfied {
                continue;
            }
            match unit {
                None => {
                    self.base_conflict = true;
                    return;
                }
                Some(u) if !more_than_one => {
                    self.base[u.var() as usize] = Some(u.is_positive());
                    for &cj in &self.by_var[u.var() as usize] {
                        wl.push(cj);
                    }
                }
                _ => {}
            }
        }
    }

    /// Is `sigma` an automorphism of the indexed database? Inspects only the clauses `sigma`
    /// actually moves — the union of the occurrence lists of its moved variables — using a stamped
    /// visited mark instead of a fresh `O(|db|)` allocation each call.
    pub fn is_automorphism(&mut self, sigma: &Perm) -> bool {
        self.vgen = self.vgen.wrapping_add(1);
        if self.vgen == 0 {
            for g in &mut self.visited_gen {
                *g = 0;
            }
            self.vgen = 1;
        }
        // If σ is an involution (σ² = id — every transposition, the bread and butter of symmetry
        // breaking, is one) the moved clauses pair up as {C, σ(C)}: verifying σ(C) ∈ F also settles
        // σ(σ(C)) = C ∈ F for free, so we mark the partner checked and skip it — halving the work.
        let involution = (0..self.nv as u32).all(|v| {
            let pv = Lit::pos(v);
            sigma.apply(sigma.apply(pv)) == pv
        });
        // A single reusable key buffer for the whole call — the σ-image of each visited clause is
        // built in place and probed against the membership set by slice, so the inner loop performs
        // ZERO heap allocations (the previous two-Vec-per-clause cost dominated at pigeonhole scale).
        let mut keybuf: Vec<u32> = Vec::new();
        for v in 0..self.nv {
            if sigma.apply(Lit::pos(v as u32)) == Lit::pos(v as u32) {
                continue;
            }
            for k in 0..self.by_var[v].len() {
                let ci = self.by_var[v][k];
                if self.visited_gen[ci] == self.vgen {
                    continue;
                }
                self.visited_gen[ci] = self.vgen;
                keybuf.clear();
                for &l in &self.clauses[ci] {
                    let m = sigma.apply(l);
                    keybuf.push(m.var() * 2 + u32::from(!m.is_positive()));
                }
                keybuf.sort_unstable();
                keybuf.dedup();
                if !self.keys.contains(keybuf.as_slice()) {
                    return false;
                }
                // The partner σ(C) is verified present; for an involution its own image is C, so
                // mark it checked to avoid re-probing the reverse direction.
                if involution {
                    if let Some(&partner) = self.index_by_key.get(keybuf.as_slice()) {
                        self.visited_gen[partner] = self.vgen;
                    }
                }
            }
        }
        true
    }

    /// Occurrence-driven unit propagation layered over the persistent `base`: assume every literal
    /// of `assume`, propagate, and report whether a conflict results. Visits only clauses sharing a
    /// variable with a newly-assigned literal, and starts from the already-derived standing units —
    /// so a call costs `O(touched)`, the difference between an `O(n⁴)` and an `O(n³)` refutation.
    ///
    /// Robust to duplicate literals (`x ∨ x` is the unit `x`) and tautologies (`x ∨ ¬x`), matching
    /// the trusted `crate::rup::propagate` verdict exactly (a differential fuzz proves it).
    pub fn propagate_to_conflict(&mut self, _num_vars: usize, assume: &[Lit]) -> bool {
        if self.base_conflict {
            return true;
        }
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            for g in &mut self.temp_gen {
                *g = 0;
            }
            self.gen = 1;
        }
        let mut queue: Vec<u32> = Vec::new();
        // Seed the assumptions over the standing base. `None` means assign; an opposite value (in
        // base or the live temp layer) is an immediate conflict.
        for &l in assume {
            let v = l.var() as usize;
            let cur = if self.temp_gen[v] == self.gen {
                Some(self.temp_val[v])
            } else {
                self.base[v]
            };
            match cur {
                Some(b) if b == l.is_positive() => {}
                Some(_) => return true,
                None => {
                    self.temp_gen[v] = self.gen;
                    self.temp_val[v] = l.is_positive();
                    queue.push(v as u32);
                }
            }
        }
        let mut qi = 0;
        while qi < queue.len() {
            let v = queue[qi] as usize;
            qi += 1;
            for k in 0..self.by_var[v].len() {
                let ci = self.by_var[v][k];
                let mut satisfied = false;
                let mut unit: Option<Lit> = None;
                let mut more_than_one = false;
                for &l in &self.clauses[ci] {
                    let vi = l.var() as usize;
                    let raw = if self.temp_gen[vi] == self.gen {
                        Some(self.temp_val[vi])
                    } else {
                        self.base[vi]
                    };
                    match raw.map(|b| b == l.is_positive()) {
                        Some(true) => {
                            satisfied = true;
                            break;
                        }
                        Some(false) => {}
                        None => match unit {
                            None => unit = Some(l),
                            Some(u) if u == l => {}
                            Some(u) if u == l.negated() => {
                                satisfied = true;
                                break;
                            }
                            Some(_) => more_than_one = true,
                        },
                    }
                }
                if satisfied {
                    continue;
                }
                match unit {
                    None => return true, // every literal false ⇒ conflict
                    Some(u) if !more_than_one => {
                        let uv = u.var() as usize;
                        self.temp_gen[uv] = self.gen;
                        self.temp_val[uv] = u.is_positive();
                        queue.push(uv as u32);
                    }
                    _ => {}
                }
            }
        }
        false
    }
}

// ===========================================================================================
// General symmetry detection (saucy/bliss-style individualization-refinement)
// ===========================================================================================
//
// We reduce the CNF to a vertex-colored graph whose color-preserving automorphisms are exactly
// the symmetries of the formula, then search for a generating set of that automorphism group.
//
// Vertices: two per variable — the positive and negative literal — joined by a phase-consistency
// edge (so an automorphism maps a variable's two literals together, possibly swapping phase); and
// one per clause, joined to each of its literal vertices. Literal vertices share one color; clause
// vertices are colored by length, disjoint from literals — so no automorphism maps a clause to a
// literal or to a clause of a different length. The literal-vertex index is exactly the packed
// `Lit` code (`2*var + sign`), so the literal half of a graph automorphism reads off as a [`Perm`].
//
// Soundness rests entirely on [`perm_is_automorphism`]: every candidate the search proposes is
// re-verified before it is returned, so a bug in the (intricate) search can only cost
// completeness, never soundness.

/// Find a generating set of the symmetry group of `clauses` — literal permutations `σ` with
/// `σ(F) = F`. Each returned generator is independently verified by [`perm_is_automorphism`].
pub fn find_generators(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Perm> {
    if num_vars == 0 {
        return Vec::new();
    }
    let (adj, init_colors) = build_graph(num_vars, clauses);
    let adj_sorted: Vec<Vec<usize>> = adj
        .iter()
        .map(|a| {
            let mut s = a.clone();
            s.sort_unstable();
            s
        })
        .collect();
    let base = refine(&adj, &init_colors);
    let mut ctx = Search {
        adj: &adj,
        adj_sorted: &adj_sorted,
        init_colors: &init_colors,
        num_vars,
        clauses,
        parent: (0..adj.len()).collect(),
        first_leaf: None,
        gens: Vec::new(),
        seen: std::collections::HashSet::new(),
        budget: 500_000,
    };
    ctx.search(base);
    ctx.gens
}

/// Search state threaded through the individualization-refinement recursion.
struct Search<'a> {
    adj: &'a [Vec<usize>],
    adj_sorted: &'a [Vec<usize>],
    init_colors: &'a [u32],
    num_vars: usize,
    clauses: &'a [Vec<Lit>],
    /// Union-find over graph vertices, for orbit pruning (orbits only grow as generators appear).
    parent: Vec<usize>,
    /// The first discrete leaf reached — the reference labeling all later leaves compare against.
    first_leaf: Option<Vec<u32>>,
    gens: Vec<Perm>,
    /// Dedup of emitted generators by their literal-image codes.
    seen: std::collections::HashSet<Vec<u32>>,
    /// A node budget guarding against blow-up on highly symmetric inputs (sound if exhausted).
    budget: usize,
}

impl Search<'_> {
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }

    fn search(&mut self, coloring: Vec<u32>) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        match target_cell(&coloring) {
            None => self.visit_leaf(coloring),
            Some(cell) => {
                // Explore one representative per orbit: always the first, then any not yet in the
                // orbit (under generators found so far) of an explored representative.
                let mut explored: Vec<usize> = Vec::new();
                for (i, &u) in cell.iter().enumerate() {
                    if i > 0 {
                        let ru = self.find(u);
                        if explored.iter().any(|&r| self.find(r) == ru) {
                            continue;
                        }
                    }
                    explored.push(u);
                    let child = individualize(self.adj, &coloring, u);
                    self.search(child);
                }
            }
        }
    }

    fn visit_leaf(&mut self, leaf: Vec<u32>) {
        let ref0 = match &self.first_leaf {
            None => {
                self.first_leaf = Some(leaf);
                return;
            }
            Some(r) => r.clone(),
        };
        let Some((vperm, lperm)) = self.candidate(&ref0, &leaf) else {
            return;
        };
        let key = perm_key(self.num_vars, &lperm);
        if !lperm.is_identity()
            && !self.seen.contains(&key)
            && perm_is_automorphism(self.clauses, &lperm)
        {
            self.seen.insert(key);
            for u in 0..vperm.len() {
                self.union(u, vperm[u]);
            }
            self.gens.push(lperm);
        }
    }

    /// From the reference labeling `ref0` and another discrete `leaf`, form the vertex permutation
    /// `g(u) = leaf⁻¹(ref0(u))` and, if it preserves colors and adjacency, read off its literal
    /// part as a [`Perm`]. Returns `None` if the leaves are not related by a graph automorphism.
    fn candidate(&self, ref0: &[u32], leaf: &[u32]) -> Option<(Vec<usize>, Perm)> {
        let v = ref0.len();
        let mut leaf_inv = vec![0usize; v];
        for (u, &r) in leaf.iter().enumerate() {
            leaf_inv[r as usize] = u;
        }
        let g: Vec<usize> = (0..v).map(|u| leaf_inv[ref0[u] as usize]).collect();
        for u in 0..v {
            if self.init_colors[u] != self.init_colors[g[u]] {
                return None;
            }
            let mut mapped: Vec<usize> = self.adj[u].iter().map(|&w| g[w]).collect();
            mapped.sort_unstable();
            if mapped != self.adj_sorted[g[u]] {
                return None;
            }
        }
        let nlit = 2 * self.num_vars;
        let mut images = Vec::with_capacity(self.num_vars);
        for var in 0..self.num_vars {
            let gv = g[2 * var];
            if gv >= nlit {
                return None;
            }
            images.push(Lit::new((gv / 2) as u32, gv % 2 == 0));
        }
        Some((g, Perm::from_images(images)))
    }
}

/// Map a literal-permutation automorphism to a permutation of the `2·num_vars` **literal points**
/// (`2v` = `+xᵥ`, `2v+1` = `¬xᵥ`), so the permutation-group backend ([`crate::permgroup`]) can analyze
/// it. `σ` respects negation, so the result is a genuine permutation of the literal points.
fn literal_perm_to_points(num_vars: usize, sigma: &Perm) -> crate::permgroup::Perm {
    let idx = |l: Lit| 2 * l.var() as usize + usize::from(!l.is_positive());
    let mut p = vec![0usize; 2 * num_vars];
    for v in 0..num_vars as u32 {
        for l in [Lit::pos(v), Lit::neg(v)] {
            p[idx(l)] = idx(sigma.apply(l));
        }
    }
    p
}

/// **The formula's automorphism group as a BSGS — Schreier–Sims as the symmetry backend.** The detected
/// generators ([`find_generators`]) are bridged to permutations of the `2·num_vars` literal points and a
/// base + strong generating set is built, so the symmetry layer can compute **`|Aut(F)|`**
/// ([`crate::permgroup::Bsgs::order`]) and decide **membership / cosets**
/// ([`crate::permgroup::Bsgs::contains`]) in polynomial time — group computations it previously could not
/// do (it worked only with the raw generators). The stabilizer chain is the symmetry break, generalized
/// from the abelian linear engines to an arbitrary (non-abelian) permutation group.
pub fn automorphism_group(num_vars: usize, clauses: &[Vec<Lit>]) -> crate::permgroup::Bsgs {
    let gens: Vec<crate::permgroup::Perm> = find_generators(num_vars, clauses)
        .iter()
        .filter(|g| !g.is_identity())
        .map(|g| literal_perm_to_points(num_vars, g))
        .collect();
    crate::permgroup::schreier_sims(2 * num_vars, &gens)
}

/// `|Aut(F)|` — the exact number of formula automorphisms, via the Schreier–Sims backend.
pub fn aut_order(num_vars: usize, clauses: &[Vec<Lit>]) -> u128 {
    automorphism_group(num_vars, clauses).order()
}

/// Build the literal+clause colored graph: adjacency lists and initial colors.
fn build_graph(num_vars: usize, clauses: &[Vec<Lit>]) -> (Vec<Vec<usize>>, Vec<u32>) {
    let nlit = 2 * num_vars;
    let vtotal = nlit + clauses.len();
    let mut adj = vec![Vec::new(); vtotal];
    let mut color = vec![0u32; vtotal];
    for var in 0..num_vars {
        let (a, b) = (2 * var, 2 * var + 1);
        adj[a].push(b);
        adj[b].push(a);
    }
    for (ci, clause) in clauses.iter().enumerate() {
        let cv = nlit + ci;
        color[cv] = 1 + clause.len() as u32;
        for &l in clause {
            let lv = (l.var() * 2 + u32::from(!l.is_positive())) as usize;
            adj[cv].push(lv);
            adj[lv].push(cv);
        }
    }
    for a in adj.iter_mut() {
        a.sort_unstable();
        a.dedup();
    }
    (adj, color)
}

/// Color refinement to an equitable partition (1-dimensional Weisfeiler–Leman), canonicalized to
/// dense ranks `0..k` deterministically (sorted by `(color, sorted neighbor colors)`).
fn refine(adj: &[Vec<usize>], colors: &[u32]) -> Vec<u32> {
    let v = adj.len();
    let mut cur = colors.to_vec();
    loop {
        let sigs: Vec<(u32, Vec<u32>)> = (0..v)
            .map(|u| {
                let mut nb: Vec<u32> = adj[u].iter().map(|&w| cur[w]).collect();
                nb.sort_unstable();
                (cur[u], nb)
            })
            .collect();
        let mut order: Vec<usize> = (0..v).collect();
        order.sort_by(|&a, &b| sigs[a].cmp(&sigs[b]));
        let mut new = vec![0u32; v];
        let mut rank = 0u32;
        for i in 0..v {
            if i > 0 && sigs[order[i]] != sigs[order[i - 1]] {
                rank += 1;
            }
            new[order[i]] = rank;
        }
        if new == cur {
            return new;
        }
        cur = new;
    }
}

/// Individualize vertex `u` (make it a singleton in its cell) and re-refine.
fn individualize(adj: &[Vec<usize>], colors: &[u32], u: usize) -> Vec<u32> {
    let mut c = colors.to_vec();
    c[u] = u32::MAX;
    refine(adj, &c)
}

/// The first non-singleton color class (the branching target), or `None` if the coloring is
/// discrete (every vertex its own color).
fn target_cell(colors: &[u32]) -> Option<Vec<usize>> {
    let maxc = *colors.iter().max().unwrap();
    for c in 0..=maxc {
        let members: Vec<usize> = (0..colors.len()).filter(|&u| colors[u] == c).collect();
        if members.len() > 1 {
            return Some(members);
        }
    }
    None
}

/// A `Perm`'s identity key: the packed code of each variable's positive-literal image.
fn perm_key(num_vars: usize, perm: &Perm) -> Vec<u32> {
    (0..num_vars as u32)
        .map(|v| {
            let l = perm.apply(Lit::pos(v));
            l.var() * 2 + u32::from(!l.is_positive())
        })
        .collect()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/symmetry_detect/tests.rs"]
mod tests;
