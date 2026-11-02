use super::*;

fn e(from: usize, to: usize, delay: i64, registers: i64) -> Edge {
    Edge { from, to, delay, registers }
}

/// The largest delay/registers over every simple cycle, by enumeration: `None` for a graph without cycles, and
/// `Some((d, 0))` for a combinational loop.
fn brute(nodes: usize, edges: &[Edge]) -> Option<(i64, i64)> {
    let mut best: Option<(i64, i64)> = None;
    fn walk(start: usize, at: usize, d: i64, r: i64, on: &mut Vec<bool>, edges: &[Edge], best: &mut Option<(i64, i64)>) {
        for x in edges.iter().filter(|x| x.from == at) {
            if x.to == start {
                let (cd, cr) = (d + x.delay, r + x.registers);
                let better = match *best {
                    None => true,
                    Some((_, 0)) => false,
                    Some((bd, br)) => cr == 0 || (cd as i128) * (br as i128) > (bd as i128) * (cr as i128),
                };
                if better {
                    *best = Some((cd, cr));
                }
            } else if x.to > start && !on[x.to] {
                on[x.to] = true;
                walk(start, x.to, d + x.delay, r + x.registers, on, edges, best);
                on[x.to] = false;
            }
        }
    }
    for s in 0..nodes {
        let mut on = vec![false; nodes];
        on[s] = true;
        walk(s, s, 0, 0, &mut on, edges, &mut best);
    }
    best
}

#[test]
fn a_graph_without_cycles_has_no_bound() {
    let edges = [e(0, 1, 5, 1), e(1, 2, 3, 0), e(0, 2, 9, 0)];
    let b = max_cycle_ratio(3, &edges).unwrap();
    assert_eq!(b, Bound::Acyclic);
    assert!(check(3, &edges, &b));
}

#[test]
fn a_cycle_without_a_register_is_a_combinational_loop() {
    let edges = [e(0, 1, 1, 1), e(1, 2, 2, 0), e(2, 1, 3, 0)];
    let b = max_cycle_ratio(3, &edges).unwrap();
    let Bound::CombinationalLoop(cycle) = &b else { panic!("{b:?}") };
    let mut nodes: Vec<usize> = cycle.iter().map(|&k| edges[k].from).collect();
    nodes.sort();
    assert_eq!(nodes, [1, 2]);
    assert!(check(3, &edges, &b));
}

#[test]
fn the_slowest_loop_bounds_the_period() {
    // A two-register loop of 10 (ratio 5) and a one-register loop of 9 (ratio 9) sharing node 0.
    let edges = [e(0, 1, 4, 1), e(1, 0, 6, 1), e(0, 2, 2, 0), e(2, 0, 7, 1)];
    let b = max_cycle_ratio(3, &edges).unwrap();
    let Bound::Ratio { delay, registers, cycle, .. } = &b else { panic!("{b:?}") };
    assert_eq!((*delay, *registers), (9, 1));
    let mut used = cycle.clone();
    used.sort();
    assert_eq!(used, [2, 3]);
    assert!(check(3, &edges, &b));
}

#[test]
fn a_ring_divides_its_delay_by_its_registers() {
    let edges = [e(0, 1, 3, 1), e(1, 2, 4, 0), e(2, 0, 5, 1)];
    let b = max_cycle_ratio(3, &edges).unwrap();
    assert!(matches!(b, Bound::Ratio { delay: 12, registers: 2, .. }), "{b:?}");
    assert_eq!(b.period(), Some((6, 1)), "12/2 reduced");
    assert!(check(3, &edges, &b));
}

#[test]
fn random_graphs_agree_with_enumeration() {
    let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    for round in 0..300 {
        let nodes = 2 + next(6) as usize;
        let count = nodes + next(2 * nodes as u64) as usize;
        let edges: Vec<Edge> = (0..count)
            .map(|_| e(next(nodes as u64) as usize, next(nodes as u64) as usize, next(20) as i64, (next(3) == 0) as i64 + (next(5) == 0) as i64))
            .collect();
        let b = max_cycle_ratio(nodes, &edges).unwrap();
        assert!(check(nodes, &edges, &b), "round {round}: {b:?}");
        match (brute(nodes, &edges), &b) {
            (None, Bound::Acyclic) => {}
            (Some((_, 0)), Bound::CombinationalLoop(_)) => {}
            (Some((d, r)), Bound::Ratio { delay, registers, .. }) => {
                assert_eq!((*delay as i128) * (r as i128), (d as i128) * (*registers as i128), "round {round}: {edges:?}");
            }
            (want, got) => panic!("round {round}: enumeration {want:?}, solver {got:?}, edges {edges:?}"),
        }
    }
}

#[test]
fn a_tampered_bound_is_refused() {
    let edges = [e(0, 1, 4, 1), e(1, 0, 6, 1), e(0, 2, 2, 0), e(2, 0, 7, 1)];
    let b = max_cycle_ratio(3, &edges).unwrap();
    let Bound::Ratio { delay, registers, cycle, potential } = b.clone() else { panic!() };
    let smaller = Bound::Ratio { delay: 10, registers: 2, cycle: vec![0, 1], potential: potential.clone() };
    assert!(!check(3, &edges, &smaller), "a cycle that is not the slowest cannot claim the bound");
    let mut broken = potential.clone();
    broken[0] += 1_000;
    assert!(!check(3, &edges, &Bound::Ratio { delay, registers, cycle: cycle.clone(), potential: broken }));
    assert!(!check(3, &edges, &Bound::Ratio { delay, registers, cycle: vec![2], potential: potential.clone() }), "not a cycle");
    assert!(!check(3, &edges, &Bound::Ratio { delay: delay + 1, registers, cycle, potential }), "the sums are the cycle's");
    assert!(!check(3, &edges, &Bound::Acyclic));
    assert!(!check(3, &edges, &Bound::CombinationalLoop(vec![2, 3])), "that loop holds a register");
}

#[test]
fn negative_weights_and_out_of_range_edges_are_refused() {
    let edges = [e(0, 1, -1, 1), e(1, 0, 1, 1)];
    assert_eq!(max_cycle_ratio(2, &edges), Err(GraphError::Negative { edge: 0 }), "delays and registers are counts");
    assert!(!check(2, &edges, &Bound::Ratio { delay: 0, registers: 2, cycle: vec![0, 1], potential: vec![0, 0] }));
    let edges = [e(0, 5, 1, 1)];
    assert_eq!(max_cycle_ratio(2, &edges), Err(GraphError::OutOfRange { edge: 0 }));
    assert!(!check(2, &edges, &Bound::Acyclic));
}

#[test]
fn values_at_the_limit_and_node_counts_past_the_bound_are_refused() {
    let below = [e(0, 1, LIMIT - 1, 1), e(1, 0, 0, LIMIT - 1)];
    assert!(max_cycle_ratio(2, &below).is_ok());
    assert_eq!(max_cycle_ratio(2, &[e(0, 1, LIMIT, 1)]), Err(GraphError::TooLarge { edge: 0 }));
    assert_eq!(max_cycle_ratio(2, &[e(0, 1, 1, 1), e(1, 0, 1, LIMIT)]), Err(GraphError::TooLarge { edge: 1 }));
    assert_eq!(max_cycle_ratio(NODES + 1, &[]), Err(GraphError::TooManyNodes));
    assert!(!check(NODES + 1, &[], &Bound::Acyclic));
}

#[test]
fn the_largest_values_stay_exact() {
    let n = 1_000;
    let mut edges: Vec<Edge> = (0..n).map(|i| e(i, (i + 1) % n, LIMIT - 1, (i == 0) as i64)).collect();
    edges.push(e(0, n / 2, LIMIT - 1, LIMIT - 1));
    let b = max_cycle_ratio(n, &edges).unwrap();
    assert_eq!(b.period(), Some((n as i64 * (LIMIT - 1), 1)));
    assert!(check(n, &edges, &b));
}

#[test]
fn a_cycle_walked_twice_or_a_potential_at_the_edge_of_i128_is_refused_without_a_panic() {
    let edges = [e(0, 1, 3, 1), e(1, 2, 4, 0), e(2, 0, 5, 1)];
    let b = max_cycle_ratio(3, &edges).unwrap();
    let Bound::Ratio { potential, .. } = b else { panic!() };
    let twice = Bound::Ratio { delay: 24, registers: 4, cycle: vec![0, 1, 2, 0, 1, 2], potential: potential.clone() };
    assert!(!check(3, &edges, &twice), "a cycle is simple");
    let high = Bound::Ratio { delay: 12, registers: 2, cycle: vec![0, 1, 2], potential: vec![i128::MAX; 3] };
    assert!(!check(3, &edges, &high), "edge 1 has positive weight, so a potential at the top cannot satisfy it");
    let low = Bound::Ratio { delay: 12, registers: 2, cycle: vec![0, 1, 2], potential: vec![i128::MIN, 0, 0] };
    assert!(!check(3, &edges, &low));
    assert!(!check(3, &edges, &Bound::CombinationalLoop(vec![1, 1])), "not a cycle, and not simple");
}

#[test]
fn a_large_graph_is_solved_and_certified() {
    let mut s: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    let nodes = 20_000;
    let edges: Vec<Edge> = (0..3 * nodes)
        .map(|i| {
            let from = if i < nodes { i } else { next(nodes as u64) as usize };
            let to = if i < nodes { (i + 1) % nodes } else { next(nodes as u64) as usize };
            e(from, to, next(1_000) as i64, 1 + (next(4) == 0) as i64)
        })
        .collect();
    let b = max_cycle_ratio(nodes, &edges).unwrap();
    assert!(matches!(b, Bound::Ratio { .. }), "{:?}", b.period());
    assert!(check(nodes, &edges, &b));
}

#[test]
fn medium_graphs_of_several_components_agree_with_enumeration() {
    let mut s: u64 = 0xd1b5_4a32_d192_ed03;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    let mut ratios = 0;
    for round in 0..200 {
        let nodes = 8 + next(5) as usize;
        let count = nodes + next(nodes as u64) as usize;
        let edges: Vec<Edge> = (0..count)
            .map(|_| e(next(nodes as u64) as usize, next(nodes as u64) as usize, next(50) as i64, 1 + (next(3) == 0) as i64))
            .collect();
        let b = max_cycle_ratio(nodes, &edges).unwrap();
        assert!(check(nodes, &edges, &b), "round {round}: {b:?}");
        match (brute(nodes, &edges), &b) {
            (None, Bound::Acyclic) => {}
            (Some((d, r)), Bound::Ratio { delay, registers, .. }) => {
                ratios += 1;
                assert_eq!((*delay as i128) * (r as i128), (d as i128) * (*registers as i128), "round {round}: {edges:?}");
            }
            (want, got) => panic!("round {round}: enumeration {want:?}, solver {got:?}, edges {edges:?}"),
        }
    }
    assert!(ratios > 150, "the rounds exercise cycles: {ratios}");
}
