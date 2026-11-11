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
    // σ permutes its support among itself, so a clause touching the support maps to a clause touching it: only those
    // clauses need a key.
    let touching: Vec<&Vec<Lit>> =
        clauses.iter().filter(|c| c.iter().any(|l| (l.var() as usize) < nv && moved[l.var() as usize])).collect();
    let set: std::collections::HashSet<Vec<u32>> = touching.iter().map(|c| clause_key(c)).collect();
    touching.iter().all(|c| set.contains(&clause_key(&sigma.apply_clause(c))))
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
// The search is nauty's (McKay & Piperno, *Practical graph isomorphism II*, 2014) over an ordered partition kept as
// contiguous cells of one array, refined to an equitable partition by neighbor counts against splitter cells in a
// canonical order (each piece of a split ordered by its count), so refinement costs `O((V + E) log V)` and commutes
// with every automorphism. The first path individualizes, level by level, the first vertex of the first non-singleton
// cell down to a discrete leaf. Then, deepest level first, each other vertex `w` of a level's cell that is not yet in
// the orbit of the first path's vertex there is individualized instead, and its subtree is searched for a leaf
// equivalent to the first; a branch whose partition shape (cell lengths in position order) differs from the first
// path's at the same depth holds no such leaf and is pruned at once. A generator found at level `k` or below fixes
// the first path's vertices above `k`, so its orbits may prune level `k`: one generator per orbit point and level is
// a strong generating set (the Schreier–Sims coset argument), so the generators span the whole group.
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
    let mut search = Search::new(&adj, &init_colors, num_vars, clauses);
    search.run();
    search.gens
}

/// An ordered partition of the graph's vertices: the cells are contiguous ranges of `elems`, each named by its start
/// position, whose `cell_len` entry holds its length.
#[derive(Clone)]
struct Partition {
    elems: Vec<u32>,
    pos: Vec<u32>,
    cell_of: Vec<u32>,
    cell_len: Vec<u32>,
}

impl Partition {
    /// The partition of the vertices by `colors`, the cells in increasing color.
    fn by_colors(colors: &[u32]) -> Partition {
        let v = colors.len();
        let mut elems: Vec<u32> = (0..v as u32).collect();
        elems.sort_by_key(|&u| (colors[u as usize], u));
        let mut part = Partition { elems, pos: vec![0; v], cell_of: vec![0; v], cell_len: vec![0; v] };
        let mut start = 0;
        for i in 0..v {
            let u = part.elems[i] as usize;
            part.pos[u] = i as u32;
            if i > 0 && colors[u] != colors[part.elems[i - 1] as usize] {
                part.cell_len[start] = (i - start) as u32;
                start = i;
            }
            part.cell_of[u] = start as u32;
        }
        if v > 0 {
            part.cell_len[start] = (v - start) as u32;
        }
        part
    }

    fn cell_starts(&self) -> Vec<u32> {
        let mut starts = Vec::new();
        let mut i = 0;
        while i < self.elems.len() {
            starts.push(i as u32);
            i += self.cell_len[i] as usize;
        }
        starts
    }

    fn is_discrete(&self) -> bool {
        self.cell_starts().len() == self.elems.len()
    }

    /// The cell lengths in position order: equal for two partitions on the way to equivalent leaves.
    fn shape(&self) -> Vec<u32> {
        self.cell_starts().iter().map(|&s| self.cell_len[s as usize]).collect()
    }

    /// The first cell with more than one vertex.
    fn target(&self) -> Option<Vec<u32>> {
        let mut i = 0;
        while i < self.elems.len() {
            let len = self.cell_len[i] as usize;
            if len > 1 {
                return Some(self.elems[i..i + len].to_vec());
            }
            i += len;
        }
        None
    }

    /// Refine to the coarsest equitable partition finer than this one, splitting from the cells in `queue`.
    fn refine(&mut self, adj: &[Vec<usize>], mut queue: std::collections::VecDeque<u32>, count: &mut [u32]) {
        let mut queued = vec![false; self.elems.len()];
        for &c in &queue {
            queued[c as usize] = true;
        }
        let mut touched_cells: Vec<u32> = Vec::new();
        let mut cell_touched = vec![false; self.elems.len()];
        let mut touched: Vec<u32> = Vec::new();
        while let Some(splitter) = queue.pop_front() {
            queued[splitter as usize] = false;
            let (s, len) = (splitter as usize, self.cell_len[splitter as usize] as usize);
            let members: Vec<u32> = self.elems[s..s + len].to_vec();
            for &x in &members {
                for &y in &adj[x as usize] {
                    if count[y] == 0 {
                        touched.push(y as u32);
                        let c = self.cell_of[y];
                        if !cell_touched[c as usize] {
                            cell_touched[c as usize] = true;
                            touched_cells.push(c);
                        }
                    }
                    count[y] += 1;
                }
            }
            touched_cells.sort_unstable();
            for &c in &touched_cells {
                let (cs, clen) = (c as usize, self.cell_len[c as usize] as usize);
                if clen == 1 {
                    continue;
                }
                let mut block: Vec<u32> = self.elems[cs..cs + clen].to_vec();
                block.sort_by_key(|&u| (count[u as usize], u));
                if count[block[0] as usize] == count[block[clen - 1] as usize] {
                    continue;
                }
                let mut pieces: Vec<(usize, usize)> = Vec::new();
                let mut piece_start = 0;
                for i in 1..=clen {
                    if i == clen || count[block[i] as usize] != count[block[i - 1] as usize] {
                        pieces.push((cs + piece_start, i - piece_start));
                        piece_start = i;
                    }
                }
                for (i, &u) in block.iter().enumerate() {
                    self.elems[cs + i] = u;
                    self.pos[u as usize] = (cs + i) as u32;
                }
                let largest = pieces.iter().enumerate().max_by_key(|(k, &(_, l))| (l, std::cmp::Reverse(*k))).map(|(k, _)| k).unwrap();
                let was_queued = queued[cs];
                for (k, &(ps, pl)) in pieces.iter().enumerate() {
                    self.cell_len[ps] = pl as u32;
                    for &u in &self.elems[ps..ps + pl] {
                        self.cell_of[u as usize] = ps as u32;
                    }
                    if (was_queued || k != largest) && !queued[ps] {
                        queued[ps] = true;
                        queue.push_back(ps as u32);
                    }
                }
            }
            for &y in &touched {
                count[y as usize] = 0;
            }
            for &c in &touched_cells {
                cell_touched[c as usize] = false;
            }
            touched.clear();
            touched_cells.clear();
        }
    }

    /// Individualize `u`: make it a singleton at the front of its cell, then refine.
    fn individualize(&self, adj: &[Vec<usize>], u: u32, count: &mut [u32]) -> Partition {
        let mut part = self.clone();
        let c = part.cell_of[u as usize] as usize;
        let len = part.cell_len[c] as usize;
        let at = part.pos[u as usize] as usize;
        part.elems.swap(c, at);
        let moved = part.elems[at];
        part.pos[moved as usize] = at as u32;
        part.pos[u as usize] = c as u32;
        part.cell_len[c] = 1;
        part.cell_len[c + 1] = (len - 1) as u32;
        for &w in &part.elems[c + 1..c + len] {
            part.cell_of[w as usize] = (c + 1) as u32;
        }
        let queue: std::collections::VecDeque<u32> = [c as u32].into_iter().collect();
        part.refine(adj, queue, count);
        part
    }
}

/// Search state: the graph, the first path, the generators and their orbits.
struct Search<'a> {
    adj: &'a [Vec<usize>],
    adj_sorted: Vec<Vec<usize>>,
    init_colors: &'a [u32],
    num_vars: usize,
    clauses: &'a [Vec<Lit>],
    /// Union-find over graph vertices: the orbits of the generators found so far.
    parent: Vec<usize>,
    /// The first path: the partition at each depth, the vertex individualized there, and each partition's shape.
    path: Vec<Partition>,
    chosen: Vec<u32>,
    shapes: Vec<Vec<u32>>,
    first_leaf: Vec<u32>,
    gens: Vec<Perm>,
    seen: std::collections::HashSet<Vec<u32>>,
    count: Vec<u32>,
    /// Partitions refined in subtree searches: a budget guarding against blow-up (sound if exhausted).
    budget: usize,
}

impl<'a> Search<'a> {
    fn new(adj: &'a [Vec<usize>], init_colors: &'a [u32], num_vars: usize, clauses: &'a [Vec<Lit>]) -> Search<'a> {
        let adj_sorted = adj
            .iter()
            .map(|a| {
                let mut s = a.clone();
                s.sort_unstable();
                s
            })
            .collect();
        Search {
            adj,
            adj_sorted,
            init_colors,
            num_vars,
            clauses,
            parent: (0..adj.len()).collect(),
            path: Vec::new(),
            chosen: Vec::new(),
            shapes: Vec::new(),
            first_leaf: Vec::new(),
            gens: Vec::new(),
            seen: std::collections::HashSet::new(),
            count: vec![0; adj.len()],
            budget: 500_000,
        }
    }

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

    /// Lay the first path, then search each level's alternatives, deepest level first.
    fn run(&mut self) {
        let mut part = Partition::by_colors(self.init_colors);
        let all: std::collections::VecDeque<u32> = part.cell_starts().into_iter().collect();
        part.refine(self.adj, all, &mut self.count);
        loop {
            self.shapes.push(part.shape());
            let Some(cell) = part.target() else { break };
            let u = cell[0];
            let next = part.individualize(self.adj, u, &mut self.count);
            self.path.push(part);
            self.chosen.push(u);
            part = next;
        }
        self.first_leaf = part.elems.clone();
        for level in (0..self.chosen.len()).rev() {
            let cell = self.path[level].target().expect("a first-path partition has a target cell");
            let anchor = self.chosen[level] as usize;
            for &w in &cell {
                if self.budget == 0 {
                    return;
                }
                let (rw, ra) = (self.find(w as usize), self.find(anchor));
                if rw == ra {
                    continue;
                }
                let child = self.path[level].individualize(self.adj, w, &mut self.count);
                if let Some(leaf) = self.equivalent_leaf(child, level + 1) {
                    self.accept(&leaf);
                }
            }
        }
    }

    /// A leaf below `part` (at `depth`) whose labeling relates to the first leaf by a graph automorphism.
    fn equivalent_leaf(&mut self, part: Partition, depth: usize) -> Option<Vec<u32>> {
        if self.budget == 0 {
            return None;
        }
        self.budget -= 1;
        if depth >= self.shapes.len() || part.shape() != self.shapes[depth] {
            return None;
        }
        if part.is_discrete() {
            return self.relates(&part.elems).then(|| part.elems.clone());
        }
        let cell = part.target()?;
        for &x in &cell {
            let child = part.individualize(self.adj, x, &mut self.count);
            if let Some(leaf) = self.equivalent_leaf(child, depth + 1) {
                return Some(leaf);
            }
        }
        None
    }

    /// The vertex map from the first leaf to `leaf`, position by position.
    fn vertex_map(&self, leaf: &[u32]) -> Vec<usize> {
        let mut g = vec![0usize; leaf.len()];
        for (i, &u) in self.first_leaf.iter().enumerate() {
            g[u as usize] = leaf[i] as usize;
        }
        g
    }

    /// Whether `leaf` relates to the first leaf by a color- and adjacency-preserving vertex map.
    fn relates(&self, leaf: &[u32]) -> bool {
        let g = self.vertex_map(leaf);
        (0..g.len()).all(|u| {
            if self.init_colors[u] != self.init_colors[g[u]] {
                return false;
            }
            let mut mapped: Vec<usize> = self.adj[u].iter().map(|&w| g[w]).collect();
            mapped.sort_unstable();
            mapped == self.adj_sorted[g[u]]
        })
    }

    /// Record the automorphism to `leaf`: its literal part, when it is one and passes the soundness gate, becomes a
    /// generator, and its vertex orbits join.
    fn accept(&mut self, leaf: &[u32]) {
        let g = self.vertex_map(leaf);
        for u in 0..g.len() {
            self.union(u, g[u]);
        }
        let nlit = 2 * self.num_vars;
        let mut images = Vec::with_capacity(self.num_vars);
        for var in 0..self.num_vars {
            let gv = g[2 * var];
            if gv >= nlit {
                return;
            }
            images.push(Lit::new((gv / 2) as u32, gv % 2 == 0));
        }
        let lperm = Perm::from_images(images);
        let key = perm_key(self.num_vars, &lperm);
        if !lperm.is_identity() && !self.seen.contains(&key) && perm_is_automorphism(self.clauses, &lperm) {
            self.seen.insert(key);
            self.gens.push(lperm);
        }
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
