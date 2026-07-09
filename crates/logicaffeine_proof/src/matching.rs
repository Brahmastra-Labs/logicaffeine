//! Certified bipartite-matching infeasibility — the polynomial reasoner for pigeonhole-shaped
//! problems that are *exponential* for resolution (and therefore for any CDCL SAT solver, ours
//! included).
//!
//! Many infeasibility claims are really "n items must each take a distinct slot, but they only
//! reach m < n slots" — graph colouring of a clique (n mutually-adjacent movements need n phases),
//! the pigeonhole principle, exam scheduling, register allocation. Encoded as boolean SAT these are
//! pigeonhole instances, which need exponentially long resolution refutations. But the underlying
//! question — does a system of "each slot holds at most one item" constraints admit an assignment
//! of every item? — is just **bipartite maximum matching**, decided in polynomial time.
//!
//! [`assign_or_hall`] returns either a feasible assignment (a checkable witness of feasibility) or a
//! **Hall witness**: a set `S` of items whose combined reachable slots `T` satisfy `|T| < |S|`, so
//! the items cannot be placed (a checkable witness of *in*feasibility, à la a clique or an odd
//! cycle). Both outcomes are independently re-verifiable — [`is_hall_witness`] and a feasibility
//! check — so this is a *certified* decision, never a trusted solver verdict.

/// The outcome of a bipartite "each slot holds at most one item" feasibility check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchOutcome {
    /// Every item placed: `assignment[i]` is the slot item `i` takes (all distinct).
    Feasible(Vec<usize>),
    /// No assignment exists, witnessed by a deficient set of items.
    Infeasible(HallWitness),
}

/// A Hall-theorem certificate of infeasibility: `items` (the set `S`) can collectively reach only
/// the slots in `slots` (a superset of `N(S)`), and `slots.len() < items.len()` — so by pigeonhole
/// the items cannot be placed one-per-slot. Independently checkable via [`is_hall_witness`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HallWitness {
    /// The deficient item set `S`.
    pub items: Vec<usize>,
    /// Slots covering `N(S)`, with `slots.len() < items.len()`.
    pub slots: Vec<usize>,
}

/// Decide whether every item can be assigned a distinct slot, where `adj[i]` lists the slots item
/// `i` may use and slots range over `0..num_slots`. Returns a perfect assignment or a certified
/// Hall witness. Finds a **maximum** matching with **Hopcroft–Karp** (`O(E·√V)` — many shortest
/// vertex-disjoint augmenting paths per phase, far faster than Kuhn's `O(V·E)` as instances grow),
/// then — on failure — the König alternating-reachability construction extracts the deficient set.
pub fn assign_or_hall(adj: &[Vec<usize>], num_slots: usize) -> MatchOutcome {
    let n = adj.len();
    // Counting fast-path: more items than slots is infeasible by pigeonhole alone — each slot holds
    // at most one item, so `n` items demand `n` distinct slots and only `num_slots < n` exist. No
    // matching needed; the whole item set against all slots is the Hall witness. O(n) instead of
    // O(E·√V), so pure pigeonhole (the common case) is decided in microseconds at any scale.
    if n > num_slots {
        return MatchOutcome::Infeasible(HallWitness {
            items: (0..n).collect(),
            slots: (0..num_slots).collect(),
        });
    }
    let mut slot_match: Vec<Option<usize>> = vec![None; num_slots]; // slot -> item
    let mut item_match: Vec<Option<usize>> = vec![None; n]; // item -> slot
    hopcroft_karp(adj, &mut slot_match, &mut item_match);
    if item_match.iter().all(|m| m.is_some()) {
        return MatchOutcome::Feasible(item_match.into_iter().map(|m| m.unwrap()).collect());
    }
    MatchOutcome::Infeasible(extract_hall(adj, num_slots, &slot_match, &item_match))
}

/// Hopcroft–Karp maximum bipartite matching. Each phase: a BFS layers the items by shortest
/// alternating-path distance toward a free slot, then DFS augments along a maximal set of
/// vertex-disjoint shortest paths — `O(√V)` phases of `O(E)`.
fn hopcroft_karp(
    adj: &[Vec<usize>],
    slot_match: &mut [Option<usize>],
    item_match: &mut [Option<usize>],
) {
    let n = adj.len();
    let num_slots = slot_match.len();
    const INF: usize = usize::MAX;
    loop {
        // BFS over items: dist[u] is u's layer; a phase exists iff a free slot becomes reachable.
        let mut dist = vec![INF; n];
        let mut queue = std::collections::VecDeque::new();
        for u in 0..n {
            if item_match[u].is_none() {
                dist[u] = 0;
                queue.push_back(u);
            }
        }
        let mut reachable_free = false;
        while let Some(u) = queue.pop_front() {
            for &v in &adj[u] {
                if v >= num_slots {
                    continue;
                }
                match slot_match[v] {
                    None => reachable_free = true,
                    Some(w) if dist[w] == INF => {
                        dist[w] = dist[u] + 1;
                        queue.push_back(w);
                    }
                    _ => {}
                }
            }
        }
        if !reachable_free {
            break; // no augmenting path remains ⇒ the matching is maximum
        }
        for u in 0..n {
            if item_match[u].is_none() {
                hk_dfs(u, adj, slot_match, item_match, &mut dist);
            }
        }
    }
}

/// One Hopcroft–Karp augmenting DFS, restricted to the BFS layering (`dist`). Dead ends are marked
/// `INF` so other DFS calls in the same phase skip them, keeping the augmenting paths disjoint.
fn hk_dfs(
    u: usize,
    adj: &[Vec<usize>],
    slot_match: &mut [Option<usize>],
    item_match: &mut [Option<usize>],
    dist: &mut [usize],
) -> bool {
    let num_slots = slot_match.len();
    for idx in 0..adj[u].len() {
        let v = adj[u][idx];
        if v >= num_slots {
            continue;
        }
        let proceed = match slot_match[v] {
            None => true,
            Some(w) => dist[w] == dist[u] + 1 && hk_dfs(w, adj, slot_match, item_match, dist),
        };
        if proceed {
            slot_match[v] = Some(u);
            item_match[u] = Some(v);
            return true;
        }
    }
    dist[u] = usize::MAX;
    false
}

/// König construction: alternating reachability from the unmatched items. Every reachable slot is
/// matched (else an augmenting path would exist, contradicting maximality), and its matched item is
/// reachable — so the reachable items outnumber the reachable slots by exactly the unmatched count,
/// and every slot adjacent to a reachable item is itself reachable. Hence `S = reachable items`,
/// `T = reachable slots` is a Hall witness with `N(S) ⊆ T` and `|T| < |S|`.
fn extract_hall(
    adj: &[Vec<usize>],
    num_slots: usize,
    slot_match: &[Option<usize>],
    item_match: &[Option<usize>],
) -> HallWitness {
    let n = adj.len();
    let mut item_reach = vec![false; n];
    let mut slot_reach = vec![false; num_slots];
    let mut stack: Vec<usize> = Vec::new();
    for (u, m) in item_match.iter().enumerate() {
        if m.is_none() {
            item_reach[u] = true;
            stack.push(u);
        }
    }
    while let Some(u) = stack.pop() {
        for &v in &adj[u] {
            if v < num_slots && !slot_reach[v] {
                slot_reach[v] = true;
                if let Some(w) = slot_match[v] {
                    if !item_reach[w] {
                        item_reach[w] = true;
                        stack.push(w);
                    }
                }
            }
        }
    }
    HallWitness {
        items: (0..n).filter(|&u| item_reach[u]).collect(),
        slots: (0..num_slots).filter(|&v| slot_reach[v]).collect(),
    }
}

/// Independently re-check a Hall witness: every item in `S` reaches only slots in `T`, and
/// `|T| < |S|`. This is the certificate verifier — a trusted, solver-free check that the claimed
/// infeasibility is genuine.
pub fn is_hall_witness(adj: &[Vec<usize>], w: &HallWitness) -> bool {
    if w.items.len() <= w.slots.len() {
        return false;
    }
    let slot_set: std::collections::HashSet<usize> = w.slots.iter().copied().collect();
    w.items.iter().all(|&i| {
        i < adj.len() && adj[i].iter().all(|s| slot_set.contains(s))
    })
}

/// Re-check a feasible assignment: one distinct slot per item, each within that item's allowed set.
pub fn is_valid_assignment(adj: &[Vec<usize>], num_slots: usize, assignment: &[usize]) -> bool {
    if assignment.len() != adj.len() {
        return false;
    }
    let mut used = vec![false; num_slots];
    assignment.iter().enumerate().all(|(i, &s)| {
        let ok = s < num_slots && adj[i].contains(&s) && !used[s];
        if s < num_slots {
            used[s] = true;
        }
        ok
    })
}

// ── Capacitated b-matching (each slot holds up to a capacity) ────────────────

/// The outcome of a capacitated assignment: each slot `s` holds at most `capacities[s]` items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapMatchOutcome {
    /// Every item placed: `assignment[i]` is the slot item `i` takes (respecting capacities).
    Feasible(Vec<usize>),
    /// No assignment exists, witnessed by a capacity-deficient item set.
    Infeasible(CapHallWitness),
}

/// A capacitated Hall certificate: the items in `S` can only reach the slots in `slots`, whose
/// *total capacity* is strictly less than `|S|` — so they cannot all be placed. Re-checkable via
/// [`is_cap_hall_witness`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapHallWitness {
    /// The deficient item set `S`.
    pub items: Vec<usize>,
    /// Slots covering `N(S)`, with `Σ capacities[s] < |S|`.
    pub slots: Vec<usize>,
}

/// Decide whether every item can be assigned a slot when slot `s` holds at most `capacities[s]`
/// items (a b-matching / resource-allocation feasibility — e.g. traffic movements sharing
/// capacity-limited green windows). Reduces to plain matching by splitting each slot into that many
/// interchangeable copies, then maps the result (and any Hall witness) back to original slots.
pub fn assign_or_hall_capacitated(adj: &[Vec<usize>], capacities: &[usize]) -> CapMatchOutcome {
    let num_slots = capacities.len();
    let mut offset = vec![0usize; num_slots + 1];
    for s in 0..num_slots {
        offset[s + 1] = offset[s] + capacities[s];
    }
    let total = offset[num_slots];
    // Each item, adjacent to slot s, is adjacent to all of slot s's copies.
    let exp_adj: Vec<Vec<usize>> = adj
        .iter()
        .map(|slots| {
            slots
                .iter()
                .filter(|&&s| s < num_slots)
                .flat_map(|&s| offset[s]..offset[s + 1])
                .collect()
        })
        .collect();
    let copy_to_slot = |c: usize| offset.partition_point(|&o| o <= c) - 1;
    match assign_or_hall(&exp_adj, total) {
        MatchOutcome::Feasible(copy_assign) => {
            CapMatchOutcome::Feasible(copy_assign.into_iter().map(copy_to_slot).collect())
        }
        MatchOutcome::Infeasible(w) => {
            // Reachable copies are exactly all copies of the reachable original slots, so their
            // total capacity equals |w.slots| < |w.items| — the capacitated Hall deficiency.
            let mut slots: Vec<usize> = w.slots.into_iter().map(copy_to_slot).collect();
            slots.sort_unstable();
            slots.dedup();
            CapMatchOutcome::Infeasible(CapHallWitness { items: w.items, slots })
        }
    }
}

/// Re-check a capacitated Hall witness: every item in `S` reaches only slots in `T`, and the total
/// capacity of `T` is below `|S|`.
pub fn is_cap_hall_witness(adj: &[Vec<usize>], capacities: &[usize], w: &CapHallWitness) -> bool {
    let cap: usize = w.slots.iter().map(|&s| capacities.get(s).copied().unwrap_or(0)).sum();
    if w.items.len() <= cap {
        return false;
    }
    let slot_set: std::collections::HashSet<usize> = w.slots.iter().copied().collect();
    w.items
        .iter()
        .all(|&i| i < adj.len() && adj[i].iter().all(|s| slot_set.contains(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pigeonhole PHP(n): n items each able to use any of n-1 slots — infeasible, and the witness
    /// is the whole item set against all n-1 slots.
    fn php(n: usize) -> (Vec<Vec<usize>>, usize) {
        let holes = n - 1;
        ((0..n).map(|_| (0..holes).collect()).collect(), holes)
    }

    #[test]
    fn pigeonhole_is_infeasible_with_a_genuine_hall_witness() {
        for n in 2..=12 {
            let (adj, slots) = php(n);
            match assign_or_hall(&adj, slots) {
                MatchOutcome::Infeasible(w) => {
                    assert!(is_hall_witness(&adj, &w), "PHP({n}) witness invalid: {w:?}");
                    assert_eq!(w.items.len(), n, "all {n} pigeons are deficient");
                    assert_eq!(w.slots.len(), n - 1, "against {} holes", n - 1);
                }
                other => panic!("PHP({n}) must be infeasible, got {other:?}"),
            }
        }
    }

    #[test]
    fn equal_items_and_slots_is_feasible() {
        // n items, n slots, complete bipartite → a perfect matching exists.
        for n in 1..=10 {
            let adj: Vec<Vec<usize>> = (0..n).map(|_| (0..n).collect()).collect();
            match assign_or_hall(&adj, n) {
                MatchOutcome::Feasible(a) => {
                    assert!(is_valid_assignment(&adj, n, &a), "invalid assignment {a:?}");
                }
                other => panic!("n={n} square should be feasible, got {other:?}"),
            }
        }
    }

    #[test]
    fn restricted_subset_triggers_hall() {
        // 3 items all confined to slots {0,1} (out of 4 slots) → infeasible: 3 items, 2 reachable.
        let adj = vec![vec![0, 1], vec![0, 1], vec![0, 1], vec![2, 3]];
        match assign_or_hall(&adj, 4) {
            MatchOutcome::Infeasible(w) => {
                assert!(is_hall_witness(&adj, &w), "witness invalid: {w:?}");
                assert!(w.items.len() > w.slots.len());
            }
            other => panic!("expected Hall violation, got {other:?}"),
        }
    }

    #[test]
    fn a_solvable_restricted_matching_is_feasible() {
        // A perfect matching exists (0->0, 1->1, 2->2, 3->3) despite restrictions.
        let adj = vec![vec![0, 1], vec![1, 2], vec![2, 3], vec![3, 0]];
        match assign_or_hall(&adj, 4) {
            MatchOutcome::Feasible(a) => assert!(is_valid_assignment(&adj, 4, &a)),
            other => panic!("expected a feasible matching, got {other:?}"),
        }
    }

    #[test]
    fn empty_is_trivially_feasible() {
        assert_eq!(assign_or_hall(&[], 0), MatchOutcome::Feasible(vec![]));
    }

    #[test]
    fn a_bad_hall_witness_is_rejected() {
        // |S| not > |T| ⇒ not a witness; and an item reaching outside T ⇒ not a witness.
        let adj = vec![vec![0, 1], vec![0, 1]];
        assert!(!is_hall_witness(&adj, &HallWitness { items: vec![0], slots: vec![0, 1] }));
        assert!(!is_hall_witness(&adj, &HallWitness { items: vec![0, 1], slots: vec![0] }),
            "item 1 reaches slot 1 ∉ T, so {{0}} cannot cover N(S)");
    }

    #[test]
    fn capacity_makes_overloaded_slots_infeasible() {
        // 5 movements all want the one green window: capacity 5 fits, capacity 4 does not.
        let adj = vec![vec![0], vec![0], vec![0], vec![0], vec![0]];
        assert!(matches!(assign_or_hall_capacitated(&adj, &[5]), CapMatchOutcome::Feasible(_)));
        match assign_or_hall_capacitated(&adj, &[4]) {
            CapMatchOutcome::Infeasible(w) => {
                assert!(is_cap_hall_witness(&adj, &[4], &w), "cap witness invalid: {w:?}");
                assert_eq!(w.items.len(), 5);
                assert_eq!(w.slots, vec![0]);
            }
            o => panic!("capacity 4 must be infeasible: {o:?}"),
        }
    }

    #[test]
    fn capacitated_assignment_respects_capacities() {
        // 4 items over 2 slots of capacity 2 → feasible, each slot used ≤ 2.
        let adj: Vec<Vec<usize>> = (0..4).map(|_| vec![0, 1]).collect();
        match assign_or_hall_capacitated(&adj, &[2, 2]) {
            CapMatchOutcome::Feasible(a) => {
                assert_eq!(a.len(), 4);
                assert!(a.iter().filter(|&&s| s == 0).count() <= 2);
                assert!(a.iter().filter(|&&s| s == 1).count() <= 2);
                assert!(a.iter().all(|&s| s == 0 || s == 1));
            }
            o => panic!("should be feasible: {o:?}"),
        }
        // 5 items, total capacity 4 → infeasible with a certified capacity-deficiency.
        let adj5: Vec<Vec<usize>> = (0..5).map(|_| vec![0, 1]).collect();
        match assign_or_hall_capacitated(&adj5, &[2, 2]) {
            CapMatchOutcome::Infeasible(w) => assert!(is_cap_hall_witness(&adj5, &[2, 2], &w)),
            o => panic!("5 items / capacity 4 must be infeasible: {o:?}"),
        }
    }

    #[test]
    fn hopcroft_karp_finds_the_maximum_matching() {
        // Independent Kuhn reference for the maximum-matching size.
        fn kuhn_size(adj: &[Vec<usize>], num_slots: usize) -> usize {
            fn aug(u: usize, adj: &[Vec<usize>], sm: &mut [Option<usize>], seen: &mut [bool]) -> bool {
                for &v in &adj[u] {
                    if v >= seen.len() || seen[v] {
                        continue;
                    }
                    seen[v] = true;
                    if sm[v].is_none() || aug(sm[v].unwrap(), adj, sm, seen) {
                        sm[v] = Some(u);
                        return true;
                    }
                }
                false
            }
            let mut sm = vec![None; num_slots];
            let mut count = 0;
            for u in 0..adj.len() {
                let mut seen = vec![false; num_slots];
                if aug(u, adj, &mut sm, &mut seen) {
                    count += 1;
                }
            }
            count
        }
        // Deterministic xorshift corpus of random bipartite graphs.
        let mut s: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..300 {
            let n = (next() % 9) as usize + 1;
            let num_slots = (next() % 9) as usize + 1;
            let adj: Vec<Vec<usize>> = (0..n)
                .map(|_| (0..num_slots).filter(|_| next() % 2 == 0).collect())
                .collect();
            let kuhn = kuhn_size(&adj, num_slots);
            let outcome = assign_or_hall(&adj, num_slots);
            // Hopcroft–Karp must agree on feasibility (perfect matching ⟺ max matching = n)…
            let feasible = matches!(outcome, MatchOutcome::Feasible(_));
            assert_eq!(
                feasible,
                kuhn == n,
                "HK/Kuhn disagree: adj={adj:?} slots={num_slots} hk_feasible={feasible} kuhn={kuhn} n={n}"
            );
            // …and every returned witness re-checks.
            match outcome {
                MatchOutcome::Feasible(a) => assert!(is_valid_assignment(&adj, num_slots, &a)),
                MatchOutcome::Infeasible(w) => assert!(is_hall_witness(&adj, &w)),
            }
        }
    }
}
