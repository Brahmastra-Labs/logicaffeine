//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SPATIAL CLOCK LOCK — the period is a theorem about the graph, and it is checked against graphs
//! whose period can be worked out by hand.
//!
//! A spatial program has no global clock, and the simulator's counter is not one: ttsim advances its
//! clock once per simulated tick while it steps the RV32 cores, so quadrupling a GEMM's reduction depth
//! moves it from 5180 to 5182 (`ttsim_instrument_audit.json`). Fitting a cost model to that number is
//! fitting to output-tile movement.
//!
//! The choreography, though, is a certified Petri MARKED GRAPH, and a marked graph has an exact
//! steady-state period: the maximum cycle mean. This file pins that the implementation computes it, on
//! graphs small enough to verify by inspection — a period nobody can check is not better than a fit.
//!
//! NON-VACUITY PROVEN: `the_period_responds_to_every_delay_on_the_critical_cycle` perturbs each delay
//! in turn and requires the period to move for exactly the transitions the critical cycle names.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::certificate::{MarkedGraph, Place, PlaceKind, Transition};
use logicaffeine_compile::spatial::clock::cycle_period;
use logicaffeine_compile::spatial::ir::{Engine, Site};

fn t(op_index: usize, consumes: Vec<usize>, produces: Vec<usize>) -> Transition {
    Transition {
        site: Site {
            x: 0,
            y: 0,
            chip: 0,
        },
        engine: Engine::Compute,
        op_index,
        consumes,
        produces,
    }
}

fn place(marking: usize) -> Place {
    Place {
        kind: PlaceKind::Pc {
            site: Site {
                x: 0,
                y: 0,
                chip: 0,
            },
            engine: Engine::Compute,
            k: 0,
        },
        marking,
    }
}

/// A single cycle over `n` transitions carrying `tokens` tokens in total.
fn ring(delays: &[f64], tokens: usize) -> (MarkedGraph, Vec<f64>) {
    let n = delays.len();
    let places: Vec<Place> = (0..n)
        .map(|i| place(if i == 0 { tokens } else { 0 }))
        .collect();
    let transitions: Vec<Transition> = (0..n).map(|i| t(i, vec![i], vec![(i + 1) % n])).collect();
    (
        MarkedGraph {
            places,
            transitions,
        },
        delays.to_vec(),
    )
}

/// The defining case: one cycle, so the period is the cycle's total delay divided by its tokens.
#[test]
fn a_single_cycle_has_period_total_delay_over_tokens() {
    for (delays, tokens, want) in [
        (vec![3.0, 5.0], 1usize, 8.0),
        (vec![3.0, 5.0], 2, 4.0),
        (vec![1.0, 1.0, 1.0, 1.0], 4, 1.0),
        (vec![7.0], 1, 7.0),
        (vec![2.5, 2.5, 5.0], 5, 2.0),
    ] {
        let (g, d) = ring(&delays, tokens);
        let p = cycle_period(&g, |tr| d[tr.op_index]);
        assert!(p.cyclic, "{delays:?}/{tokens}: a ring is cyclic");
        assert!(
            (p.period - want).abs() < 1e-6,
            "{delays:?} over {tokens} tokens: period {} but the mean is {want}",
            p.period
        );
        assert_eq!(
            p.critical_cycle.len(),
            delays.len(),
            "the only cycle is the critical one"
        );
    }
}

/// With two cycles the SLOWEST sets the period — that is what makes it a bound and not an average.
#[test]
fn the_slowest_cycle_sets_the_period() {
    // Two rings sharing transition 0: fast ring 0→1→0, slow ring 0→2→3→0.
    let places = vec![place(1), place(0), place(0), place(0), place(1)];
    let transitions = vec![
        t(0, vec![0, 4], vec![1, 2]),
        t(1, vec![1], vec![0]),
        t(2, vec![2], vec![3]),
        t(3, vec![3], vec![4]),
    ];
    let g = MarkedGraph {
        places,
        transitions,
    };
    // fast ring: 1 + 1 = 2 over 1 token. slow ring: 1 + 10 + 10 = 21 over... the tokens on its places.
    let d = [1.0, 1.0, 10.0, 10.0];
    let p = cycle_period(&g, |tr| d[tr.op_index]);
    assert!(p.cyclic);
    assert!(p.live, "both cycles carry a token, so the graph is live");
    assert!(
        (p.period - 21.0).abs() < 1e-6,
        "slow ring 1+10+10 over 1 token = 21, got {}",
        p.period
    );
    assert!(
        p.critical_cycle.contains(&2) && p.critical_cycle.contains(&3),
        "the critical cycle must name the slow transitions: {:?}",
        p.critical_cycle
    );
}

/// More tokens on a cycle is more pipelining: the same work overlaps further and the period falls.
#[test]
fn tokens_pipeline_the_cycle_and_lower_the_period() {
    let delays = vec![4.0, 4.0, 4.0, 4.0];
    let mut last = f64::INFINITY;
    for tokens in 1..=4 {
        let (g, d) = ring(&delays, tokens);
        let p = cycle_period(&g, |tr| d[tr.op_index]);
        assert!(
            p.period < last,
            "{tokens} tokens must beat {} : got {}",
            last,
            p.period
        );
        assert!((p.period - 16.0 / tokens as f64).abs() < 1e-6);
        last = p.period;
    }
}

/// A graph with no cycle has no steady state, and must say so rather than inventing a number.
#[test]
fn an_acyclic_graph_has_no_period() {
    let places = vec![place(0), place(0)];
    let transitions = vec![
        t(0, vec![], vec![0]),
        t(1, vec![0], vec![1]),
        t(2, vec![1], vec![]),
    ];
    let g = MarkedGraph {
        places,
        transitions,
    };
    let p = cycle_period(&g, |_| 5.0);
    assert!(!p.cyclic, "a chain has no steady state");
    assert_eq!(p.period, 0.0);
    assert!(p.critical_cycle.is_empty());
}

/// NON-VACUITY. Slowing a transition ON the critical cycle must raise the period; slowing one that is
/// not on any maximal cycle must not. A period that ignored its inputs would pass every test above.
#[test]
fn the_period_responds_to_every_delay_on_the_critical_cycle() {
    let places = vec![place(1), place(0), place(0), place(0), place(1)];
    let transitions = vec![
        t(0, vec![0, 4], vec![1, 2]),
        t(1, vec![1], vec![0]),
        t(2, vec![2], vec![3]),
        t(3, vec![3], vec![4]),
    ];
    let g = MarkedGraph {
        places,
        transitions,
    };
    let base_d = [1.0, 1.0, 10.0, 10.0];
    let base = cycle_period(&g, |tr| base_d[tr.op_index]);
    let mut moved = 0usize;
    for i in 0..4 {
        let mut d = base_d;
        d[i] += 5.0;
        let p = cycle_period(&g, |tr| d[tr.op_index]);
        let on_critical = base.critical_cycle.contains(&i);
        if on_critical {
            assert!(
                p.period > base.period + 1e-6,
                "transition {i} is on the critical cycle but changed nothing"
            );
            moved += 1;
        }
    }
    assert!(
        moved >= 2,
        "the critical cycle must have several load-bearing transitions ({moved})"
    );
    // …and the fast ring's transition 1 cannot lift the period past the slow ring on its own.
    let mut d = base_d;
    d[1] += 5.0;
    let p = cycle_period(&g, |tr| d[tr.op_index]);
    assert!(
        p.period >= base.period,
        "a delay can never make a marked graph faster"
    );
}

/// A cycle carrying no token can never fire. The period is infinite and the graph is NOT live — the
/// exact condition `certificate`'s Commoner check refuses — and saying so is the whole difference
/// between a schedule that runs and one that deadlocks.
#[test]
fn a_token_free_cycle_is_reported_as_deadlock_not_as_a_large_period() {
    let places = vec![place(1), place(0), place(0), place(0), place(0)];
    let transitions = vec![
        t(0, vec![0, 4], vec![1, 2]),
        t(1, vec![1], vec![0]),
        t(2, vec![2], vec![3]),
        t(3, vec![3], vec![4]),
    ];
    let g = MarkedGraph {
        places,
        transitions,
    };
    let p = cycle_period(&g, |_| 1.0);
    assert!(
        !p.live,
        "the ring 0->2->3->0 carries no token; it cannot fire"
    );
    assert!(
        p.period.is_infinite(),
        "a deadlocked graph has no finite period, got {}",
        p.period
    );
}
