use super::*;

fn arc(from: usize, to: usize, capacity: Option<i64>, cost: i64) -> Arc {
    Arc { from, to, capacity, cost }
}

/// The least cost by successive shortest paths over the residual network (Bellman-Ford, so negative residual costs
/// are fine), after every negative arc is saturated; `None` when the supplies cannot be met. Every negative arc must
/// have a capacity.
fn oracle(nodes: usize, supply: &[i64], arcs: &[Arc]) -> Option<i128> {
    let mut excess: Vec<i64> = supply.to_vec();
    let mut flow: Vec<i64> = vec![0; arcs.len()];
    for (k, a) in arcs.iter().enumerate() {
        if a.cost < 0 {
            let c = a.capacity.expect("a negative arc has a capacity");
            flow[k] = c;
            excess[a.from] -= c;
            excess[a.to] += c;
        }
    }
    loop {
        let Some(s) = (0..nodes).find(|&u| excess[u] > 0) else { break };
        // Residual arcs: (arc, forward).
        let mut dist: Vec<Option<i128>> = vec![None; nodes];
        let mut via: Vec<Option<(usize, bool)>> = vec![None; nodes];
        dist[s] = Some(0);
        for _ in 0..nodes {
            let mut changed = false;
            for (k, a) in arcs.iter().enumerate() {
                if let Some(d) = dist[a.from] {
                    if a.capacity.map_or(true, |c| flow[k] < c) && dist[a.to].map_or(true, |t| d + (a.cost as i128) < t) {
                        dist[a.to] = Some(d + a.cost as i128);
                        via[a.to] = Some((k, true));
                        changed = true;
                    }
                }
                if let Some(d) = dist[a.to] {
                    if flow[k] > 0 && dist[a.from].map_or(true, |t| d - (a.cost as i128) < t) {
                        dist[a.from] = Some(d - a.cost as i128);
                        via[a.from] = Some((k, false));
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let t = (0..nodes).filter(|&u| excess[u] < 0 && dist[u].is_some()).min_by_key(|&u| dist[u])?;
        let mut amount = excess[s].min(-excess[t]);
        let mut path = Vec::new();
        let mut at = t;
        while at != s {
            let (k, fwd) = via[at].unwrap();
            amount = amount.min(if fwd { arcs[k].capacity.map_or(i64::MAX, |c| c - flow[k]) } else { flow[k] });
            path.push((k, fwd));
            at = if fwd { arcs[k].from } else { arcs[k].to };
        }
        for (k, fwd) in path {
            flow[k] += if fwd { amount } else { -amount };
        }
        excess[s] -= amount;
        excess[t] += amount;
    }
    if excess.iter().any(|&x| x != 0) {
        return None;
    }
    Some(arcs.iter().zip(&flow).map(|(a, &f)| a.cost as i128 * f as i128).sum())
}

#[test]
fn a_transport_problem_ships_by_its_cheapest_routes() {
    // Two plants of 4 and 6 units, two markets of 5 and 5, routes with costs.
    let arcs = [arc(0, 2, None, 4), arc(0, 3, None, 6), arc(1, 2, None, 5), arc(1, 3, Some(3), 3), arc(1, 2, Some(1), 1)];
    let supply = [4, 6, -5, -5];
    let s = min_cost_flow(4, &supply, &arcs).unwrap();
    assert!(check(4, &supply, &arcs, &s), "{s:?}");
    let Solution::Optimal { cost, .. } = s else { panic!("{s:?}") };
    assert_eq!(Some(cost), oracle(4, &supply, &arcs));
}

#[test]
fn a_negative_cycle_without_a_capacity_has_no_optimum() {
    let arcs = [arc(0, 1, None, 2), arc(1, 2, None, -3), arc(2, 0, None, 0), arc(0, 2, Some(5), 1)];
    let supply = [1, 0, -1];
    let s = min_cost_flow(3, &supply, &arcs).unwrap();
    let Solution::Unbounded { cycle } = &s else { panic!("{s:?}") };
    let mut sorted = cycle.clone();
    sorted.sort();
    assert_eq!(sorted, [0, 1, 2]);
    assert!(check(3, &supply, &arcs, &s));
}

#[test]
fn supply_that_cannot_leave_is_infeasible_and_the_cut_shows_why() {
    let arcs = [arc(0, 1, Some(2), 1), arc(0, 2, Some(1), 1), arc(1, 3, None, 1), arc(2, 3, None, 1)];
    let supply = [5, 0, 0, -5];
    let s = min_cost_flow(4, &supply, &arcs).unwrap();
    let Solution::Infeasible { cut } = &s else { panic!("{s:?}") };
    assert!(cut.contains(&0) && !cut.contains(&3), "{cut:?}");
    assert!(check(4, &supply, &arcs, &s));
}

#[test]
fn a_tampered_answer_is_refused() {
    let arcs = [arc(0, 1, None, 1), arc(1, 2, None, 1), arc(0, 2, None, 3)];
    let supply = [2, 0, -2];
    let s = min_cost_flow(3, &supply, &arcs).unwrap();
    let Solution::Optimal { flow, potential, cost } = s.clone() else { panic!() };
    assert_eq!(cost, 4);
    assert!(check(3, &supply, &arcs, &s));
    assert!(!check(3, &supply, &arcs, &Solution::Optimal { flow: vec![0, 0, 2], potential: potential.clone(), cost: 6 }), "a dearer flow");
    assert!(!check(3, &supply, &arcs, &Solution::Optimal { flow: flow.clone(), potential: potential.clone(), cost: cost + 1 }), "the cost is the flow's");
    assert!(!check(3, &supply, &arcs, &Solution::Optimal { flow: vec![1, 1, 0], potential: potential.clone(), cost: 2 }), "the supplies are met");
    let mut wrong = potential.clone();
    wrong[1] += 100;
    assert!(!check(3, &supply, &arcs, &Solution::Optimal { flow, potential: wrong, cost }), "the potential proves the optimum");
    assert!(!check(3, &supply, &arcs, &Solution::Infeasible { cut: vec![0] }), "an uncapacitated arc leaves the cut");
    assert!(!check(3, &supply, &arcs, &Solution::Unbounded { cycle: vec![0, 1] }), "not a cycle");
}

#[test]
fn malformed_problems_are_refused() {
    assert_eq!(min_cost_flow(2, &[1, 0], &[]), Err(FlowError::Unbalanced));
    assert_eq!(min_cost_flow(2, &[0, 0], &[arc(0, 5, None, 1)]), Err(FlowError::OutOfRange { arc: 0 }));
    assert_eq!(min_cost_flow(2, &[0, 0], &[arc(0, 1, Some(-1), 1)]), Err(FlowError::OutOfBounds { arc: 0 }));
    assert_eq!(min_cost_flow(2, &[0, 0], &[arc(0, 1, None, COST_LIMIT)]), Err(FlowError::OutOfBounds { arc: 0 }));
    assert_eq!(min_cost_flow(2, &[FLOW_LIMIT - 1, 1 - FLOW_LIMIT], &[]), Err(FlowError::Unbalanced), "the sizes sum past the limit");
    assert!(matches!(min_cost_flow(0, &[], &[]), Ok(Solution::Optimal { .. })));
}

#[test]
fn random_problems_are_solved_with_certificates_and_agree_with_shortest_paths() {
    let mut s: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    let (mut optimal, mut infeasible) = (0, 0);
    for round in 0..400 {
        let nodes = 2 + next(8) as usize;
        let count = next(4 * nodes as u64) as usize;
        let arcs: Vec<Arc> = (0..count)
            .map(|_| {
                let cost = next(21) as i64 - 6;
                let capacity = if cost < 0 || next(3) > 0 { Some(next(6) as i64) } else { None };
                arc(next(nodes as u64) as usize, next(nodes as u64) as usize, capacity, cost)
            })
            .collect();
        let mut supply: Vec<i64> = (0..nodes).map(|_| next(7) as i64 - 3).collect();
        let total: i64 = supply.iter().sum();
        supply[0] -= total;
        let solved = min_cost_flow(nodes, &supply, &arcs).unwrap();
        assert!(check(nodes, &supply, &arcs, &solved), "round {round}: {solved:?}");
        match (&solved, oracle(nodes, &supply, &arcs)) {
            (Solution::Optimal { cost, .. }, Some(best)) => {
                optimal += 1;
                assert_eq!(*cost, best, "round {round}");
            }
            (Solution::Infeasible { .. }, None) => infeasible += 1,
            (got, want) => panic!("round {round}: simplex {got:?}, shortest paths {want:?}"),
        }
    }
    assert!(optimal > 100 && infeasible > 20, "both answers are exercised: {optimal} optimal, {infeasible} infeasible");
}

#[test]
fn a_large_problem_is_solved_and_certified() {
    let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    let nodes = 5_000;
    let mut arcs: Vec<Arc> = (0..nodes).map(|u| arc(u, (u + 1) % nodes, None, 50)).collect();
    arcs.extend((0..5 * nodes).map(|_| arc(next(nodes as u64) as usize, next(nodes as u64) as usize, Some(next(20) as i64), next(100) as i64)));
    let mut supply: Vec<i64> = (0..nodes).map(|_| next(11) as i64 - 5).collect();
    let total: i64 = supply.iter().sum();
    supply[0] -= total;
    let solved = min_cost_flow(nodes, &supply, &arcs).unwrap();
    assert!(matches!(solved, Solution::Optimal { .. }), "a ring without a capacity carries anything");
    assert!(check(nodes, &supply, &arcs, &solved));
}
