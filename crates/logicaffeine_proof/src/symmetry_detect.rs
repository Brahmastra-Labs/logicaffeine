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

/// Canonical multiset form of a clause set: each clause sorted+deduped by `(var, sign)` code,
/// then the clauses sorted. Two clause sets are equal as multisets iff their canon forms match.
fn canon(clauses: &[Vec<Lit>]) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = clauses
        .iter()
        .map(|c| {
            let mut k: Vec<u32> =
                c.iter().map(|l| l.var() * 2 + u32::from(!l.is_positive())).collect();
            k.sort_unstable();
            k.dedup();
            k
        })
        .collect();
    out.sort();
    out
}

/// Is `sigma` a genuine automorphism of `clauses` — i.e. does applying it map the clause set
/// exactly onto itself? This is the independent re-verification every generator must pass.
pub fn perm_is_automorphism(clauses: &[Vec<Lit>], sigma: &Perm) -> bool {
    let mapped: Vec<Vec<Lit>> = clauses.iter().map(|c| sigma.apply_clause(c)).collect();
    canon(&mapped) == canon(clauses)
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
        let Some((vperm, lperm)) = self.candidate(&ref0, &leaf) else { return };
        let key = perm_key(self.num_vars, &lperm);
        if !lperm.is_identity() && !self.seen.contains(&key) && perm_is_automorphism(self.clauses, &lperm) {
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
mod tests {
    use super::*;
    use crate::cdcl::Lit;
    use crate::families;
    use crate::proof::Perm;

    /// Build the permutation that swaps two pigeon rows of PHP(n): for every hole, exchange the
    /// two pigeons' variables. This is a textbook automorphism of the pigeonhole formula.
    fn swap_pigeon_rows(n: usize, p0: usize, p1: usize) -> Perm {
        let holes = n - 1;
        let images: Vec<Lit> = (0..n * holes)
            .map(|v| {
                let (p, h) = (v / holes, v % holes);
                let np = if p == p0 {
                    p1
                } else if p == p1 {
                    p0
                } else {
                    p
                };
                Lit::pos((np * holes + h) as u32)
            })
            .collect();
        Perm::from_images(images)
    }

    #[test]
    fn identity_is_always_an_automorphism() {
        let (cnf, _) = families::php(4);
        assert!(perm_is_automorphism(&cnf.clauses, &Perm::identity(cnf.num_vars)));
    }

    #[test]
    fn swapping_pigeon_rows_is_an_automorphism_of_php() {
        let (cnf, _) = families::php(4);
        for (p0, p1) in [(0, 1), (1, 2), (0, 3), (2, 3)] {
            let sigma = swap_pigeon_rows(4, p0, p1);
            assert!(
                perm_is_automorphism(&cnf.clauses, &sigma),
                "swapping pigeons {p0},{p1} must preserve PHP(4)"
            );
        }
    }

    #[test]
    fn a_non_symmetry_is_rejected() {
        // Map only pigeon 0's row to pigeon 1's row but NOT vice-versa: this collapses two rows
        // onto one, so it is not a bijection-preserving automorphism of the clause set.
        let (cnf, _) = families::php(3);
        let holes = 2;
        let images: Vec<Lit> = (0..cnf.num_vars)
            .map(|v| {
                let (p, h) = (v / holes, v % holes);
                let np = if p == 0 { 1 } else { p };
                Lit::pos((np * holes + h) as u32)
            })
            .collect();
        assert!(!perm_is_automorphism(&cnf.clauses, &Perm::from_images(images)));
    }

    // --- the general finder ---

    use std::collections::HashSet;

    fn p(v: u32) -> Lit {
        Lit::pos(v)
    }
    fn neg(v: u32) -> Lit {
        Lit::neg(v)
    }

    /// All permutations of `0..n`.
    fn all_var_perms(n: usize) -> Vec<Vec<usize>> {
        fn rec(cur: &mut Vec<usize>, rem: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
            if rem.is_empty() {
                out.push(cur.clone());
                return;
            }
            for i in 0..rem.len() {
                let x = rem.remove(i);
                cur.push(x);
                rec(cur, rem, out);
                cur.pop();
                rem.insert(i, x);
            }
        }
        let mut out = Vec::new();
        rec(&mut Vec::new(), &mut (0..n).collect(), &mut out);
        out
    }

    /// The TRUE symmetry group of `clauses`: every negation-respecting literal permutation
    /// (variable bijection × per-variable phase) that is an automorphism, as a set of keys.
    fn brute_force_group(num_vars: usize, clauses: &[Vec<Lit>]) -> HashSet<Vec<u32>> {
        let mut set = HashSet::new();
        for vp in all_var_perms(num_vars) {
            for phase in 0..(1u32 << num_vars) {
                let images: Vec<Lit> =
                    (0..num_vars).map(|v| Lit::new(vp[v] as u32, (phase >> v) & 1 == 0)).collect();
                let perm = Perm::from_images(images);
                if perm_is_automorphism(clauses, &perm) {
                    set.insert(perm_key(num_vars, &perm));
                }
            }
        }
        set
    }

    /// The set of element keys generated by `gens` (closure under composition, with identity).
    fn generated_group(num_vars: usize, gens: &[Perm]) -> HashSet<Vec<u32>> {
        let id = Perm::identity(num_vars);
        let mut set = HashSet::new();
        set.insert(perm_key(num_vars, &id));
        let mut frontier = vec![id];
        while let Some(x) = frontier.pop() {
            for g in gens {
                let y = g.compose(&x);
                let k = perm_key(num_vars, &y);
                if set.insert(k) {
                    frontier.push(y);
                }
            }
        }
        set
    }

    #[test]
    fn finds_the_full_php3_symmetry_group() {
        let (cnf, _) = families::php(3);
        let gens = find_generators(cnf.num_vars, &cnf.clauses);
        let group = generated_group(cnf.num_vars, &gens);
        // PHP(3): permute 3 pigeons × 2 holes = S_3 × S_2, order 12.
        assert_eq!(group.len(), 12, "discovered generators must generate the full S_3 × S_2");
    }

    #[test]
    fn discovered_generators_match_brute_force_across_cases() {
        // (num_vars, clauses): PHP(2), PHP(3), a swap-symmetric pair, and an asymmetric formula.
        let php2 = families::php(2).0;
        let php3 = families::php(3).0;
        let cases: Vec<(usize, Vec<Vec<Lit>>)> = vec![
            (php2.num_vars, php2.clauses),
            (php3.num_vars, php3.clauses),
            (2, vec![vec![p(0), p(1)], vec![neg(0), neg(1)]]), // exactly-one(a,b): a↔b symmetric
            (3, vec![vec![p(0)], vec![p(0), p(1)]]),           // asymmetric: only identity
            (3, vec![vec![p(0), p(1), p(2)]]),                 // a single symmetric clause: S_3
        ];
        for (num_vars, clauses) in cases {
            let gens = find_generators(num_vars, &clauses);
            let found = generated_group(num_vars, &gens);
            let truth = brute_force_group(num_vars, &clauses);
            assert_eq!(found, truth, "finder must generate exactly Aut(F) for {clauses:?}");
        }
    }

    #[test]
    fn every_discovered_generator_is_a_verified_automorphism() {
        let (cnf, _) = families::php(4);
        let gens = find_generators(cnf.num_vars, &cnf.clauses);
        assert!(!gens.is_empty(), "PHP(4) is highly symmetric — generators must be found");
        for g in &gens {
            assert!(perm_is_automorphism(&cnf.clauses, g), "every returned generator is sound");
        }
    }

    #[test]
    fn an_asymmetric_formula_yields_no_nontrivial_generators() {
        let clauses = vec![vec![p(0)], vec![p(0), p(1)], vec![neg(1), p(2)]];
        let gens = find_generators(3, &clauses);
        assert!(gens.iter().all(|g| g.is_identity()), "no non-trivial symmetry to find");
    }
}
