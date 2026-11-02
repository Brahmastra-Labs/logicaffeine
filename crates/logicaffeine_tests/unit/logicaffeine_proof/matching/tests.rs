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
    assert!(!is_hall_witness(
        &adj,
        &HallWitness {
            items: vec![0],
            slots: vec![0, 1]
        }
    ));
    assert!(
        !is_hall_witness(
            &adj,
            &HallWitness {
                items: vec![0, 1],
                slots: vec![0]
            }
        ),
        "item 1 reaches slot 1 ∉ T, so {{0}} cannot cover N(S)"
    );
}

#[test]
fn capacity_makes_overloaded_slots_infeasible() {
    // 5 movements all want the one green window: capacity 5 fits, capacity 4 does not.
    let adj = vec![vec![0], vec![0], vec![0], vec![0], vec![0]];
    assert!(matches!(
        assign_or_hall_capacitated(&adj, &[5]),
        CapMatchOutcome::Feasible(_)
    ));
    match assign_or_hall_capacitated(&adj, &[4]) {
        CapMatchOutcome::Infeasible(w) => {
            assert!(
                is_cap_hall_witness(&adj, &[4], &w),
                "cap witness invalid: {w:?}"
            );
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
