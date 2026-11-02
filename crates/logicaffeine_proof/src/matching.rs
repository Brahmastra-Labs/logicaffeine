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
    w.items
        .iter()
        .all(|&i| i < adj.len() && adj[i].iter().all(|s| slot_set.contains(s)))
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
            CapMatchOutcome::Infeasible(CapHallWitness {
                items: w.items,
                slots,
            })
        }
    }
}

/// Re-check a capacitated Hall witness: every item in `S` reaches only slots in `T`, and the total
/// capacity of `T` is below `|S|`.
pub fn is_cap_hall_witness(adj: &[Vec<usize>], capacities: &[usize], w: &CapHallWitness) -> bool {
    let cap: usize = w
        .slots
        .iter()
        .map(|&s| capacities.get(s).copied().unwrap_or(0))
        .sum();
    if w.items.len() <= cap {
        return false;
    }
    let slot_set: std::collections::HashSet<usize> = w.slots.iter().copied().collect();
    w.items
        .iter()
        .all(|&i| i < adj.len() && adj[i].iter().all(|s| slot_set.contains(s)))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/matching/tests.rs"]
mod tests;
