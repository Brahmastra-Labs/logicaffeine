use super::*;

fn graph(n: usize, conflicts: &[(usize, usize)]) -> Intersection {
    Intersection {
        movements: (0..n).map(|i| format!("m{i}")).collect(),
        conflicts: conflicts
            .iter()
            .map(|&(a, b)| (a.min(b), a.max(b)))
            .collect(),
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
        assert!(
            is_valid_coloring(&g, &plan),
            "invalid plan for {g:?}: {plan:?}"
        );
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
    assert_eq!(
        it.movements.len(),
        3,
        "pedestrian must remain: {:?}",
        it.movements
    );
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
            (0, 1),
            (0, 4),
            (0, 5),
            (0, 6),
            (0, 7),
            (1, 4),
            (1, 5),
            (1, 6),
            (1, 7),
            (2, 3),
            (2, 6),
            (2, 7),
            (3, 6),
            (3, 7),
            (6, 7),
        ],
    );
    let plan = design_phase_plan(&g).unwrap();
    assert!(
        is_valid_coloring(&g, &plan),
        "plan must be conflict-free: {plan:?}"
    );
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
    clique
        .iter()
        .enumerate()
        .all(|(i, &u)| clique.iter().skip(i + 1).all(|&v| adj[u][v]))
}

#[test]
fn greedy_clique_returns_a_genuine_clique() {
    for g in [
        graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]), // K4 → clique 4
        graph(3, &[(0, 1), (1, 2), (0, 2)]),                         // triangle → 3
        graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]),         // C5 → 2
        grotzsch(),                                                  // triangle-free → 2
    ] {
        let n = g.movements.len();
        let adj = build_adjacency(&g, n);
        let clique = greedy_clique(&adj, n);
        assert!(
            is_clique(&adj, &clique),
            "not a clique for {g:?}: {clique:?}"
        );
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
                    assert_ne!(
                        colors[x], colors[y],
                        "adjacent {x},{y} share a colour in {g:?}"
                    );
                }
            }
        }
        assert!(
            colors.iter().all(|&c| c < ub),
            "colour out of range in {g:?}"
        );
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
        assert_eq!(
            take_solve_count(),
            0,
            "perfect graph must not call the solver: {g:?}"
        );
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
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 4),
            (4, 0),
            (0, 5),
            (1, 6),
            (2, 7),
            (3, 8),
            (4, 9),
            (5, 7),
            (7, 9),
            (9, 6),
            (6, 8),
            (8, 5),
        ],
    )
}

#[test]
fn bipartite_graphs_use_zero_sat_solves() {
    // Even cycle, star, path and disjoint edges are all bipartite → the 2-colouring IS the
    // certified plan, no solver involved.
    for g in [
        graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]), // even cycle → 2
        graph(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]), // star → 2
        graph(4, &[(0, 1), (1, 2), (2, 3)]),         // path → 2
        graph(4, &[(0, 1), (2, 3)]),                 // two disjoint edges → 2
        graph(3, &[]),                               // edgeless → 1
    ] {
        let _ = take_solve_count();
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(
            take_solve_count(),
            0,
            "bipartite graph must not call the solver: {g:?}"
        );
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
    assert_eq!(
        take_solve_count(),
        0,
        "odd-cycle bound should make C5 solver-free"
    );
    assert_eq!(plan.num_phases, 3);
    assert!(plan.minimal_certified);
}

#[test]
fn petersen_uses_zero_sat_solves() {
    let g = petersen();
    let _ = take_solve_count();
    let plan = design_phase_plan(&g).unwrap();
    assert_eq!(
        take_solve_count(),
        0,
        "Petersen: odd-cycle bound meets greedy ub"
    );
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
    assert_eq!(
        take_solve_count(),
        1,
        "Grötzsch should need exactly one refutation solve"
    );
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
    assert!(
        is_valid_coloring(&g, &plan),
        "plan must be conflict-free: {plan:?}"
    );
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
