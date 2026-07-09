//! Conflict-free traffic-signal PHASE DESIGNER (certified SAT synthesis).
//!
//! Grouping intersection movements into green phases so no two *conflicting* movements ever share
//! a phase is exactly graph k-colouring — a SAT problem. We encode it as a boolean [`ProofExpr`]
//! and discharge it with the project's own certified solver (`logicaffeine_proof::sat`), never Z3:
//!
//! * a feasible plan is a SAT **model** (a witness that is trivially re-checkable), and
//! * the claim "fewer phases is impossible" is a `prove_unsat` **`Refuted`** result, which is
//!   RUP-certified — so the *minimality* of the plan is certified, not merely asserted.
//!
//! The English frontend is a focused grammar ("X conflicts with Y, Z and …") rather than the
//! general FOL pipeline, so the designer is robust independent of that surface.

use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;
use std::collections::HashMap;

/// An intersection: a set of movements and the pairs that may not be green together.
#[derive(Clone, Debug, PartialEq)]
pub struct Intersection {
    /// Movement display names, in first-seen order.
    pub movements: Vec<String>,
    /// Conflicting movement pairs, as `(min, max)` indices into [`movements`](Self::movements).
    pub conflicts: Vec<(usize, usize)>,
}

impl Intersection {
    /// Resolve a slice of movement indices to their display names.
    pub fn names(&self, idxs: &[usize]) -> Vec<String> {
        idxs.iter().filter_map(|&i| self.movements.get(i).cloned()).collect()
    }
}

/// A synthesized signal plan: each movement assigned to a green phase.
#[derive(Clone, Debug, PartialEq)]
pub struct PhasePlan {
    /// Number of green phases used (the chromatic number when `minimal_certified`).
    pub num_phases: usize,
    /// `assignment[m]` = the phase index movement `m` is served in.
    pub assignment: Vec<usize>,
    /// `true` when `num_phases - 1` was proven infeasible (RUP-certified) — i.e. this is provably
    /// the *fewest* phases possible. (Trivially `true` for a single phase.)
    pub minimal_certified: bool,
}

impl PhasePlan {
    /// Movement indices grouped by phase, in phase order.
    pub fn groups(&self) -> Vec<Vec<usize>> {
        let mut groups = vec![Vec::new(); self.num_phases];
        for (m, &p) in self.assignment.iter().enumerate() {
            if p < self.num_phases {
                groups[p].push(m);
            }
        }
        groups
    }
}

/// Re-check that a plan is a genuine conflict-free colouring (used by tests and as a guard).
pub fn is_valid_coloring(it: &Intersection, plan: &PhasePlan) -> bool {
    if plan.assignment.len() != it.movements.len() {
        return false;
    }
    if plan.assignment.iter().any(|&p| p >= plan.num_phases.max(1)) {
        return false;
    }
    it.conflicts.iter().all(|&(a, b)| {
        a == b
            || a >= it.movements.len()
            || b >= it.movements.len()
            || plan.assignment.get(a) != plan.assignment.get(b)
    })
}

/// Design the minimal-phase conflict-free plan for an intersection, or `None` if it has no
/// movements.
///
/// The chromatic number `χ` is pinned between two **checkable combinatorial certificates** before
/// the SAT solver is ever consulted:
///
/// * a maximal **clique** of mutually-conflicting movements proves `χ ≥ |clique|` (you can verify
///   it by eye — every pair really is in conflict), and
/// * a **greedy proper colouring** is a self-checking witness that `χ ≤ greedy`.
///
/// When those meet (`|clique| == greedy`) the greedy colouring is provably optimal and we return it
/// with **zero SAT calls** — the common case for the perfect-ish conflict graphs real intersections
/// produce. Otherwise we close the gap `[lb, ub)` with the certified solver, pinning the clique to
/// phases `0..lb-1` to break colour-permutation symmetry; the first feasible `k` is `χ`, its witness
/// is a SAT model, and minimality is certified either by the clique (`k == lb`) or by the RUP-
/// `Refuted` solve at `k-1`. Either route is certified end-to-end.
pub fn design_phase_plan(it: &Intersection) -> Option<PhasePlan> {
    let n = it.movements.len();
    if n == 0 {
        return None;
    }
    let adj = build_adjacency(it, n);

    // Sharpest cheap lever first — bipartiteness. A 2-colouring (BFS, O(V+E)) either *is* the
    // certified-minimal plan (the graph is 1- or 2-chromatic; any edge rules out a single phase),
    // or it fails and hands back an **odd closed walk**: a checkable certificate that χ ≥ 3, since
    // colours must alternate along any walk and a 2-colourable graph therefore has none.
    match two_color(&adj, n) {
        Ok(coloring) => {
            let num_phases = coloring.iter().copied().max().map(|c| c + 1).unwrap_or(1);
            return Some(PhasePlan { num_phases, assignment: coloring, minimal_certified: true });
        }
        Err(odd_walk) => {
            debug_assert!(
                is_odd_closed_walk(&adj, &odd_walk),
                "two_color must return a genuine odd closed walk: {odd_walk:?}"
            );
        }
    }

    let clique = greedy_clique(&adj, n);
    // χ ≥ 3 (the odd walk just proved non-bipartiteness) and χ ≥ |clique|.
    let lb = clique.len().max(3);
    let (greedy_colors, ub) = greedy_coloring(&adj, n);

    // Fast path: lower bound meets the greedy upper bound, so the greedy colouring is optimal and
    // fewer phases is impossible — certified by the clique (when it sets the bound) or the odd walk.
    if lb >= ub {
        return Some(PhasePlan { num_phases: ub, assignment: greedy_colors, minimal_certified: true });
    }

    // Close the gap. `k = lb-1 ≥ 2` is already infeasible by a certificate (the clique when
    // `lb = |clique|`, otherwise the odd walk, which forbids any 2-colouring), so entering the scan
    // at `lb` is sound and `lb` is certified-minimal the moment it proves feasible.
    let mut prev_infeasible_certified = true;
    for k in lb..ub {
        match solve_coloring(it, &adj, &clique, k) {
            ColoringResult::Feasible(assignment) => {
                return Some(PhasePlan {
                    num_phases: k,
                    assignment,
                    minimal_certified: prev_infeasible_certified,
                });
            }
            ColoringResult::Infeasible => prev_infeasible_certified = true,
            ColoringResult::Unknown => prev_infeasible_certified = false,
        }
    }
    // Every `k` in `[lb, ub)` was infeasible, so `χ = ub`, witnessed by the greedy colouring we
    // already hold — no need to re-solve at `ub`.
    Some(PhasePlan { num_phases: ub, assignment: greedy_colors, minimal_certified: prev_infeasible_certified })
}

/// Parse an English spec into an [`Intersection`], then design its minimal plan.
pub fn design_from_spec(spec: &str) -> Result<(Intersection, PhasePlan), String> {
    let it = parse_intersection(spec)?;
    let plan =
        design_phase_plan(&it).ok_or_else(|| "no movements to schedule".to_string())?;
    Ok((it, plan))
}

// ── SAT encoding ────────────────────────────────────────────────────────────

enum ColoringResult {
    Feasible(Vec<usize>),
    Infeasible,
    Unknown,
}

/// Counts every actual SAT query, so tests can prove the bounds short-circuit (zero solves for a
/// perfect graph, one for an odd cycle, etc.) — the whole point of the speedup.
#[cfg(test)]
thread_local! {
    static SAT_SOLVE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Read and reset the per-thread SAT-solve counter (test instrumentation).
#[cfg(test)]
fn take_solve_count() -> usize {
    SAT_SOLVE_COUNT.with(|c| {
        let v = c.get();
        c.set(0);
        v
    })
}

fn solve_coloring(it: &Intersection, adj: &[Vec<bool>], clique: &[usize], k: usize) -> ColoringResult {
    #[cfg(test)]
    SAT_SOLVE_COUNT.with(|c| c.set(c.get() + 1));
    let formula = coloring_formula(it, adj, clique, k);
    match prove_unsat(&formula) {
        UnsatOutcome::Sat(model) => {
            ColoringResult::Feasible(decode(it.movements.len(), k, &model))
        }
        UnsatOutcome::Refuted => ColoringResult::Infeasible,
        UnsatOutcome::Unsupported => ColoringResult::Unknown,
    }
}

/// `a_m_p` ≙ "movement `m` is served in phase `p`".
fn assign_atom(m: usize, p: usize) -> ProofExpr {
    ProofExpr::Atom(format!("a_{m}_{p}"))
}

fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}

fn not(a: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(a))
}

fn conj(mut parts: Vec<ProofExpr>) -> ProofExpr {
    match parts.len() {
        0 => {
            let t = ProofExpr::Atom("__true".to_string());
            ProofExpr::Or(Box::new(t.clone()), Box::new(ProofExpr::Not(Box::new(t))))
        }
        1 => parts.pop().unwrap(),
        _ => {
            let mut acc = parts.pop().unwrap();
            while let Some(p) = parts.pop() {
                acc = and(p, acc);
            }
            acc
        }
    }
}

/// Build the boolean k-colouring obligation: every movement gets exactly one phase, and no
/// conflicting pair shares a phase. The `clique` is pinned to phases `0..|clique|` as a
/// **symmetry-breaking** constraint — colours are interchangeable, so a clique of distinct colours
/// can WLOG take `0, 1, 2, …` in order; this is satisfiability-preserving (so a SAT model is a real
/// witness and a RUP refutation still certifies the original UNSAT) while collapsing the otherwise
/// factorial colour-permutation search.
fn coloring_formula(it: &Intersection, adj: &[Vec<bool>], clique: &[usize], k: usize) -> ProofExpr {
    let n = it.movements.len();
    let mut clauses: Vec<ProofExpr> = Vec::new();

    // Every movement is served in at least one phase.
    for m in 0..n {
        let mut disj = assign_atom(m, 0);
        for p in 1..k {
            disj = ProofExpr::Or(Box::new(disj), Box::new(assign_atom(m, p)));
        }
        clauses.push(disj);
    }
    // …and at most one phase (no movement runs in two phases at once).
    for m in 0..n {
        for p in 0..k {
            for q in (p + 1)..k {
                clauses.push(not(and(assign_atom(m, p), assign_atom(m, q))));
            }
        }
    }
    // Conflicting movements never share a phase. (Adjacency already dropped self-loops and
    // out-of-range pairs, so no nonsensical conflict can make the problem unsatisfiable.)
    for x in 0..n {
        for y in (x + 1)..n {
            if adj[x][y] {
                for p in 0..k {
                    clauses.push(not(and(assign_atom(x, p), assign_atom(y, p))));
                }
            }
        }
    }
    // Symmetry break: pin clique member `i` to phase `i` (sound because k ≥ |clique| here).
    for (i, &m) in clique.iter().enumerate() {
        if i < k {
            clauses.push(assign_atom(m, i));
        }
    }
    conj(clauses)
}

// ── combinatorial bounds (checkable certificates that often avoid the solver) ──

/// `n × n` symmetric adjacency from the conflict pairs, with self-loops and out-of-range indices
/// dropped — the single normalised view shared by the bounds and the SAT encoding.
fn build_adjacency(it: &Intersection, n: usize) -> Vec<Vec<bool>> {
    let mut adj = vec![vec![false; n]; n];
    for &(x, y) in &it.conflicts {
        if x != y && x < n && y < n {
            adj[x][y] = true;
            adj[y][x] = true;
        }
    }
    adj
}

/// Degree of each vertex, then a descending-degree order (ties by index) — the ordering both the
/// clique and the colouring heuristics use.
fn degree_order(adj: &[Vec<bool>], n: usize) -> Vec<usize> {
    let deg = |v: usize| adj[v].iter().filter(|&&b| b).count();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| deg(b).cmp(&deg(a)).then(a.cmp(&b)));
    order
}

/// A large clique of mutually-conflicting movements: try growing greedily from each vertex (in
/// degree order) and keep the biggest. Any clique is a sound lower bound on the chromatic number;
/// a larger one just makes it tighter, so heuristic maximality is enough.
fn greedy_clique(adj: &[Vec<bool>], n: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let order = degree_order(adj, n);
    let mut best = vec![order[0]];
    for &start in &order {
        let mut clique = vec![start];
        for &v in &order {
            if v != start && clique.iter().all(|&u| adj[v][u]) {
                clique.push(v);
            }
        }
        if clique.len() > best.len() {
            best = clique;
        }
    }
    best
}

/// A greedy proper colouring (Welsh–Powell: descending degree, smallest non-conflicting colour).
/// Returns the assignment and the number of colours used — a self-checking upper bound on `χ`.
fn greedy_coloring(adj: &[Vec<bool>], n: usize) -> (Vec<usize>, usize) {
    let mut color = vec![usize::MAX; n];
    let mut num_colors = 0usize;
    for v in degree_order(adj, n) {
        let mut used = vec![false; num_colors + 1];
        for (u, &is_adj) in adj[v].iter().enumerate() {
            if is_adj && color[u] != usize::MAX && color[u] < used.len() {
                used[color[u]] = true;
            }
        }
        let c = used.iter().position(|&b| !b).unwrap_or(num_colors);
        color[v] = c;
        num_colors = num_colors.max(c + 1);
    }
    for c in color.iter_mut() {
        if *c == usize::MAX {
            *c = 0;
        }
    }
    (color, num_colors.max(1))
}

/// BFS 2-colouring. `Ok(colouring)` when the graph is bipartite (χ ≤ 2, the colouring uses 1 colour
/// if edgeless else 2); `Err(odd_walk)` otherwise, where `odd_walk` is an odd closed walk witnessing
/// χ ≥ 3. The walk is `v … root … u` (closed by the conflicting edge `u–v`); since `v` and `u` were
/// found at the same colour their BFS depths share parity, so the walk has odd length.
fn two_color(adj: &[Vec<bool>], n: usize) -> Result<Vec<usize>, Vec<usize>> {
    let mut color = vec![usize::MAX; n];
    let mut parent = vec![usize::MAX; n];
    for start in 0..n {
        if color[start] != usize::MAX {
            continue;
        }
        color[start] = 0;
        let mut stack = vec![start];
        while let Some(v) = stack.pop() {
            for u in 0..n {
                if !adj[v][u] {
                    continue;
                }
                if color[u] == usize::MAX {
                    color[u] = 1 - color[v];
                    parent[u] = v;
                    stack.push(u);
                } else if color[u] == color[v] {
                    return Err(odd_walk(&parent, v, u));
                }
            }
        }
    }
    Ok(color)
}

/// Reconstruct the odd closed walk `v → … → root → … → u` from the BFS parent forest (the edge
/// `u–v` then closes it).
fn odd_walk(parent: &[usize], v: usize, u: usize) -> Vec<usize> {
    let to_root = |mut x: usize| {
        let mut path = vec![x];
        while parent[x] != usize::MAX {
            x = parent[x];
            path.push(x);
        }
        path
    };
    let mut walk = to_root(v); // v … root
    let mut up_u = to_root(u); // u … root
    up_u.reverse(); // root … u
    walk.extend(up_u.into_iter().skip(1)); // … u (drop the duplicated root)
    walk
}

/// Verify a vertex sequence is a genuine odd closed walk: every consecutive pair (including the
/// wrap-around) is an edge, and the number of edges is odd — the property that makes it a checkable
/// certificate of `χ ≥ 3`.
fn is_odd_closed_walk(adj: &[Vec<bool>], walk: &[usize]) -> bool {
    let m = walk.len();
    m >= 3 && m % 2 == 1 && (0..m).all(|i| adj[walk[i]][walk[(i + 1) % m]])
}

fn decode(n: usize, k: usize, model: &[(String, bool)]) -> Vec<usize> {
    let truth: HashMap<&str, bool> = model.iter().map(|(s, b)| (s.as_str(), *b)).collect();
    let mut assignment = vec![0usize; n];
    for m in 0..n {
        for p in 0..k {
            let key = format!("a_{m}_{p}");
            if *truth.get(key.as_str()).unwrap_or(&false) {
                assignment[m] = p;
                break;
            }
        }
    }
    assignment
}

// ── English frontend (focused grammar) ──────────────────────────────────────

/// Parse a focused English spec into an [`Intersection`].
///
/// Recognised:
/// * `Movements: a, b, c.` — an optional explicit movement set (so isolated movements appear).
/// * `<A> conflicts with <B>, <C> and <D>.` — `A` may not share a phase with any of `B`, `C`, `D`.
///
/// Movement names are everything else (articles `the`/`a`/`an` are dropped). Names are matched
/// case-insensitively; the first spelling seen is the display name.
pub fn parse_intersection(spec: &str) -> Result<Intersection, String> {
    let mut names: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut conflicts: Vec<(usize, usize)> = Vec::new();

    for raw in spec.split(['.', '\n']) {
        let sentence = raw.trim();
        if sentence.is_empty() {
            continue;
        }
        let lower = sentence.to_lowercase();

        if let Some(mpos) = lower.find("movements:") {
            let list = &lower[mpos + "movements:".len()..];
            for m in split_list(list) {
                intern_movement(&mut names, &mut index, &m);
            }
            continue;
        }

        if let Some(cpos) = lower.find("conflict") {
            if let Some(wrel) = lower[cpos..].find("with") {
                let subject = &lower[..cpos];
                let objects = &lower[cpos + wrel + "with".len()..];
                if let Some(si) = intern_movement(&mut names, &mut index, subject) {
                    for obj in split_list(objects) {
                        if let Some(oi) = intern_movement(&mut names, &mut index, &obj) {
                            if si != oi {
                                push_conflict(&mut conflicts, si, oi);
                            }
                        }
                    }
                }
            }
        }
    }

    if names.is_empty() {
        return Err("no movements found — describe conflicts like \
            \"northbound-left conflicts with southbound-through\""
            .to_string());
    }
    Ok(Intersection { movements: names, conflicts })
}

fn intern_movement(
    names: &mut Vec<String>,
    index: &mut HashMap<String, usize>,
    raw: &str,
) -> Option<usize> {
    let name = clean_name(raw);
    if name.is_empty() {
        return None;
    }
    let key = name.to_lowercase();
    if let Some(&i) = index.get(&key) {
        return Some(i);
    }
    let i = names.len();
    names.push(name);
    index.insert(key, i);
    Some(i)
}

fn clean_name(raw: &str) -> String {
    let mut s = raw
        .trim()
        .trim_matches(|c: char| c == '.' || c == ',' || c == ';' || c == ':')
        .trim()
        .to_string();
    for article in ["the ", "a ", "an "] {
        if s.to_lowercase().starts_with(article) {
            s = s[article.len()..].trim().to_string();
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_list(s: &str) -> Vec<String> {
    s.replace(" and ", ",")
        .replace('&', ",")
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn push_conflict(conflicts: &mut Vec<(usize, usize)>, a: usize, b: usize) {
    let pair = (a.min(b), a.max(b));
    if !conflicts.contains(&pair) {
        conflicts.push(pair);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(n: usize, conflicts: &[(usize, usize)]) -> Intersection {
        Intersection {
            movements: (0..n).map(|i| format!("m{i}")).collect(),
            conflicts: conflicts.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect(),
        }
    }

    // ── colouring / minimality (the certified core) ──

    #[test]
    fn conflict_free_needs_one_phase() {
        let plan = design_phase_plan(&graph(3, &[])).unwrap();
        assert_eq!(plan.num_phases, 1);
        assert!(plan.minimal_certified);
    }

    #[test]
    fn one_conflict_needs_two_phases() {
        let g = graph(2, &[(0, 1)]);
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(plan.num_phases, 2);
        assert!(is_valid_coloring(&g, &plan));
        assert!(plan.minimal_certified, "1 phase must be RUP-refuted");
    }

    #[test]
    fn triangle_needs_three_phases() {
        let g = graph(3, &[(0, 1), (1, 2), (0, 2)]);
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(plan.num_phases, 3);
        assert!(is_valid_coloring(&g, &plan));
        assert!(plan.minimal_certified);
    }

    #[test]
    fn even_cycle_is_two_colourable() {
        let g = graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]);
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(plan.num_phases, 2);
        assert!(is_valid_coloring(&g, &plan));
    }

    #[test]
    fn odd_cycle_needs_three_phases() {
        let g = graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]);
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(plan.num_phases, 3);
        assert!(is_valid_coloring(&g, &plan));
    }

    #[test]
    fn k4_needs_four_phases() {
        let g = graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]);
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(plan.num_phases, 4);
        assert!(is_valid_coloring(&g, &plan));
        assert!(plan.minimal_certified);
    }

    #[test]
    fn every_returned_plan_is_a_valid_coloring() {
        // A grab-bag of graphs: the synthesized plan must always be conflict-free.
        for g in [
            graph(1, &[]),
            graph(6, &[(0, 1), (2, 3), (4, 5)]),
            graph(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]), // star → 2 phases
            graph(4, &[(0, 1), (1, 2), (2, 3)]),         // path → 2 phases
        ] {
            let plan = design_phase_plan(&g).unwrap();
            assert!(is_valid_coloring(&g, &plan), "invalid plan for {g:?}: {plan:?}");
        }
    }

    #[test]
    fn star_is_two_colourable() {
        let g = graph(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]);
        assert_eq!(design_phase_plan(&g).unwrap().num_phases, 2);
    }

    #[test]
    fn groups_partition_the_movements() {
        let g = graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]);
        let plan = design_phase_plan(&g).unwrap();
        let total: usize = plan.groups().iter().map(|g| g.len()).sum();
        assert_eq!(total, 4);
        assert_eq!(plan.groups().len(), plan.num_phases);
    }

    // ── English frontend ──

    #[test]
    fn parses_a_simple_conflict() {
        let it = parse_intersection("Northbound-left conflicts with southbound-through.").unwrap();
        assert_eq!(it.movements.len(), 2, "{:?}", it.movements);
        assert_eq!(it.conflicts.len(), 1);
    }

    #[test]
    fn parses_a_list_with_and_and_articles() {
        let it = parse_intersection(
            "Northbound-left conflicts with southbound-through and the east-west crossing.",
        )
        .unwrap();
        assert_eq!(it.movements.len(), 3, "{:?}", it.movements);
        assert_eq!(it.conflicts.len(), 2);
    }

    #[test]
    fn movements_declaration_keeps_isolated_movements() {
        let it = parse_intersection(
            "Movements: ns-through, ew-through, pedestrian.\n\
             ns-through conflicts with ew-through.",
        )
        .unwrap();
        assert_eq!(it.movements.len(), 3, "pedestrian must remain: {:?}", it.movements);
        assert_eq!(it.conflicts.len(), 1);
    }

    #[test]
    fn dedupes_symmetric_conflicts() {
        let it = parse_intersection("alpha conflicts with beta.\nbeta conflicts with alpha.").unwrap();
        assert_eq!(it.conflicts.len(), 1);
    }

    #[test]
    fn empty_spec_is_an_error() {
        assert!(parse_intersection("   ").is_err());
    }

    #[test]
    fn end_to_end_english_to_certified_plan() {
        let (it, plan) = design_from_spec(
            "Movements: ns, ew, ped.\n\
             ns conflicts with ew.\n\
             ped conflicts with ns and ew.",
        )
        .unwrap();
        // ns–ew, ped–ns, ped–ew form a triangle → exactly 3 phases.
        assert_eq!(plan.num_phases, 3);
        assert!(is_valid_coloring(&it, &plan));
        assert!(plan.minimal_certified);
    }

    // ── adversarial / robustness ──

    #[test]
    fn self_conflict_is_ignored_not_unsatisfiable() {
        // A movement "conflicting with itself" is nonsense; it must be ignored, not make the
        // whole problem unsolvable. Here only (0,1) is a real conflict → 2 phases.
        let g = Intersection {
            movements: vec!["a".into(), "b".into()],
            conflicts: vec![(0, 0), (0, 1)],
        };
        let plan = design_phase_plan(&g).expect("self-conflict must not break solving");
        assert_eq!(plan.num_phases, 2);
        assert!(is_valid_coloring(&g, &plan));
    }

    #[test]
    fn out_of_range_conflict_is_ignored() {
        // A hand-built intersection with a dangling index must not poison the solve.
        let g = Intersection {
            movements: vec!["a".into(), "b".into()],
            conflicts: vec![(0, 9), (0, 1)],
        };
        let plan = design_phase_plan(&g).expect("dangling conflict must not break solving");
        assert_eq!(plan.num_phases, 2);
        assert!(is_valid_coloring(&g, &plan));
    }

    #[test]
    fn disconnected_components_share_phases() {
        // Two independent conflict pairs are both 2-colourable together → 2 phases, not 4.
        let g = graph(4, &[(0, 1), (2, 3)]);
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(plan.num_phases, 2);
        assert!(is_valid_coloring(&g, &plan));
        assert!(plan.minimal_certified);
    }

    #[test]
    fn realistic_eight_movement_intersection() {
        // NS/EW through + 4 protected lefts + 2 peds, with a plausible conflict matrix; the
        // designer must return a valid, certified-minimal plan whatever the chromatic number is.
        let g = graph(
            8,
            &[
                (0, 1), (0, 4), (0, 5), (0, 6), (0, 7),
                (1, 4), (1, 5), (1, 6), (1, 7),
                (2, 3), (2, 6), (2, 7),
                (3, 6), (3, 7),
                (6, 7),
            ],
        );
        let plan = design_phase_plan(&g).unwrap();
        assert!(is_valid_coloring(&g, &plan), "plan must be conflict-free: {plan:?}");
        assert!(plan.minimal_certified, "minimality must be RUP-certified");
        assert!(plan.num_phases >= 2 && plan.num_phases <= 8);
    }

    #[test]
    fn parses_singular_conflict_with_and_three_objects() {
        let it = parse_intersection("ped conflicts with ns, ew, and nsl.").unwrap();
        assert_eq!(it.movements.len(), 4, "{:?}", it.movements);
        assert_eq!(it.conflicts.len(), 3);
    }

    // ── certified-speedup machinery (bounds short-circuit + symmetry break) ──

    /// The Grötzsch graph (Mycielskian of C5): triangle-free so its clique number is 2, yet its
    /// chromatic number is 4. The clique lower bound is far below `χ`, so the fast path can't fire
    /// and the certified SAT scan must close the whole gap — and certify minimality by RUP.
    fn grotzsch() -> Intersection {
        // a0..a4 = 0..4 (outer C5), b0..b4 = 5..9, c = 10.
        let mut edges = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)];
        let b_neighbors = [(4, 1), (0, 2), (1, 3), (2, 4), (3, 0)];
        for (i, &(lo, hi)) in b_neighbors.iter().enumerate() {
            edges.push((5 + i, lo));
            edges.push((5 + i, hi));
            edges.push((10, 5 + i));
        }
        graph(11, &edges)
    }

    fn is_clique(adj: &[Vec<bool>], clique: &[usize]) -> bool {
        clique.iter().enumerate().all(|(i, &u)| {
            clique.iter().skip(i + 1).all(|&v| adj[u][v])
        })
    }

    #[test]
    fn greedy_clique_returns_a_genuine_clique() {
        for g in [
            graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]), // K4 → clique 4
            graph(3, &[(0, 1), (1, 2), (0, 2)]),                          // triangle → 3
            graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]),          // C5 → 2
            grotzsch(),                                                    // triangle-free → 2
        ] {
            let n = g.movements.len();
            let adj = build_adjacency(&g, n);
            let clique = greedy_clique(&adj, n);
            assert!(is_clique(&adj, &clique), "not a clique for {g:?}: {clique:?}");
        }
        // Exact sizes on the easy cases.
        let k4 = graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]);
        assert_eq!(greedy_clique(&build_adjacency(&k4, 4), 4).len(), 4);
    }

    #[test]
    fn greedy_coloring_is_always_proper() {
        for g in [
            grotzsch(),
            graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]),
            graph(6, &[(0, 1), (2, 3), (4, 5)]),
        ] {
            let n = g.movements.len();
            let adj = build_adjacency(&g, n);
            let (colors, ub) = greedy_coloring(&adj, n);
            for x in 0..n {
                for y in (x + 1)..n {
                    if adj[x][y] {
                        assert_ne!(colors[x], colors[y], "adjacent {x},{y} share a colour in {g:?}");
                    }
                }
            }
            assert!(colors.iter().all(|&c| c < ub), "colour out of range in {g:?}");
        }
    }

    #[test]
    fn perfect_graph_design_uses_zero_sat_solves() {
        // K4, triangle, even cycle and star are perfect: clique == greedy, so the bounds meet and
        // the solver is never invoked.
        for g in [
            graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]),
            graph(3, &[(0, 1), (1, 2), (0, 2)]),
            graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]),
            graph(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]),
            graph(3, &[]),
        ] {
            let _ = take_solve_count();
            let plan = design_phase_plan(&g).unwrap();
            assert_eq!(take_solve_count(), 0, "perfect graph must not call the solver: {g:?}");
            assert!(is_valid_coloring(&g, &plan));
            assert!(plan.minimal_certified);
        }
    }

    /// The Petersen graph: 3-chromatic, triangle-free (clique number 2). The clique bound can't
    /// reach χ, but the odd-cycle bound (3) meets the greedy upper bound (3), so it is now solved
    /// with zero SAT calls.
    fn petersen() -> Intersection {
        graph(
            10,
            &[
                (0, 1), (1, 2), (2, 3), (3, 4), (4, 0),
                (0, 5), (1, 6), (2, 7), (3, 8), (4, 9),
                (5, 7), (7, 9), (9, 6), (6, 8), (8, 5),
            ],
        )
    }

    #[test]
    fn bipartite_graphs_use_zero_sat_solves() {
        // Even cycle, star, path and disjoint edges are all bipartite → the 2-colouring IS the
        // certified plan, no solver involved.
        for g in [
            graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]),  // even cycle → 2
            graph(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]),  // star → 2
            graph(4, &[(0, 1), (1, 2), (2, 3)]),          // path → 2
            graph(4, &[(0, 1), (2, 3)]),                  // two disjoint edges → 2
            graph(3, &[]),                                // edgeless → 1
        ] {
            let _ = take_solve_count();
            let plan = design_phase_plan(&g).unwrap();
            assert_eq!(take_solve_count(), 0, "bipartite graph must not call the solver: {g:?}");
            assert!(is_valid_coloring(&g, &plan));
            assert!(plan.minimal_certified);
        }
    }

    #[test]
    fn odd_cycle_design_uses_zero_sat_solves() {
        // C5: the odd-cycle lower bound (3) meets the greedy upper bound (3) → no SAT solve at all
        // (the previous clique-only designer still needed one refutation here).
        let g = graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]);
        let _ = take_solve_count();
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(take_solve_count(), 0, "odd-cycle bound should make C5 solver-free");
        assert_eq!(plan.num_phases, 3);
        assert!(plan.minimal_certified);
    }

    #[test]
    fn petersen_uses_zero_sat_solves() {
        let g = petersen();
        let _ = take_solve_count();
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(take_solve_count(), 0, "Petersen: odd-cycle bound meets greedy ub");
        assert_eq!(plan.num_phases, 3);
        assert!(is_valid_coloring(&g, &plan));
        assert!(plan.minimal_certified);
    }

    #[test]
    fn grotzsch_design_uses_a_single_sat_solve() {
        // The lone case in the whole corpus that still needs the solver: χ=4 sits strictly above
        // both the odd-cycle bound (3) and the clique bound (2), so exactly one refutation (k=3)
        // closes the gap; the greedy witness covers k=4.
        let g = grotzsch();
        let _ = take_solve_count();
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(take_solve_count(), 1, "Grötzsch should need exactly one refutation solve");
        assert_eq!(plan.num_phases, 4);
        assert!(plan.minimal_certified);
    }

    #[test]
    fn two_color_detects_bipartiteness_and_certifies_odd_cycles() {
        // Bipartite graphs are 2-coloured properly.
        for g in [
            graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]),
            graph(4, &[(0, 1), (1, 2), (2, 3)]),
            graph(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]),
        ] {
            let n = g.movements.len();
            let adj = build_adjacency(&g, n);
            let coloring = two_color(&adj, n).expect("graph is bipartite");
            for x in 0..n {
                for y in (x + 1)..n {
                    if adj[x][y] {
                        assert_ne!(coloring[x], coloring[y], "improper 2-colouring of {g:?}");
                    }
                }
            }
        }
        // Non-bipartite graphs yield a genuine odd closed walk (a checkable χ ≥ 3 certificate).
        for g in [
            graph(3, &[(0, 1), (1, 2), (0, 2)]),
            graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]),
            petersen(),
            grotzsch(),
        ] {
            let n = g.movements.len();
            let adj = build_adjacency(&g, n);
            let walk = two_color(&adj, n).expect_err("graph is not bipartite");
            assert!(
                is_odd_closed_walk(&adj, &walk),
                "two_color must certify non-bipartiteness with an odd walk for {g:?}: {walk:?}"
            );
        }
    }

    #[test]
    fn grotzsch_chromatic_number_is_four_and_certified() {
        // The headline correctness guard for the gap scan: clique=2 ≪ χ=4. The designer must still
        // return exactly 4 phases, a valid colouring, and RUP-certified minimality.
        let g = grotzsch();
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(plan.num_phases, 4, "Grötzsch is 4-chromatic");
        assert!(is_valid_coloring(&g, &plan), "plan must be conflict-free: {plan:?}");
        assert!(plan.minimal_certified, "3 phases must be RUP-refuted");
    }

    #[test]
    fn build_adjacency_drops_self_and_out_of_range() {
        let g = Intersection {
            movements: vec!["a".into(), "b".into()],
            conflicts: vec![(0, 0), (0, 9), (0, 1)],
        };
        let adj = build_adjacency(&g, 2);
        assert!(adj[0][1] && adj[1][0], "real conflict kept");
        assert!(!adj[0][0], "self-loop dropped");
    }
}
