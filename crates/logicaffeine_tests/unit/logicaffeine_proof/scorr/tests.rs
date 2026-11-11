use super::*;
use crate::aig::Init;
use crate::itp::check_invariant;
use crate::pdr;
use std::collections::{HashSet, VecDeque};

/// The bad-state literals: the bad-state properties, or the outputs of a circuit that has none.
fn bads(aig: &Aig) -> Vec<AigLit> {
    let ports = if aig.bad().is_empty() { aig.outputs() } else { aig.bad() };
    ports.iter().map(|p| p.lit).collect()
}

/// Every input assignment of `aig`.
fn assignments(aig: &Aig) -> impl Iterator<Item = Vec<bool>> {
    let ni = aig.inputs().len();
    (0..1u64 << ni).map(move |bits| (0..ni).map(|i| bits >> i & 1 == 1).collect())
}

/// The states a run reaches at a clock whose constraints held at every clock before it, explicit-state: every initial
/// state, and each successor under inputs that keep the constraints.
fn reachable(aig: &Aig) -> Vec<Vec<bool>> {
    let mut initial: Vec<Vec<bool>> = vec![Vec::new()];
    for l in aig.latches() {
        let choices: &[bool] = match l.init {
            Init::Zero => &[false],
            Init::One => &[true],
            Init::Free => &[false, true],
        };
        initial = initial.iter().flat_map(|s| choices.iter().map(move |&c| [s.clone(), vec![c]].concat())).collect();
    }
    let mut seen: HashSet<Vec<bool>> = initial.iter().cloned().collect();
    let mut queue: VecDeque<Vec<bool>> = initial.into_iter().collect();
    let mut out = Vec::new();
    while let Some(s) = queue.pop_front() {
        for x in assignments(aig) {
            let v = aig.eval(&x, &s);
            if !aig.constraints().iter().all(|c| Aig::value(&v, c.lit)) {
                continue;
            }
            let next: Vec<bool> = aig.latches().iter().map(|l| Aig::value(&v, l.next)).collect();
            if seen.insert(next.clone()) {
                queue.push_back(next);
            }
        }
        out.push(s);
    }
    out
}

/// Whether a bad state is reached with the constraints holding at its clock and every clock before.
fn reachable_bad(aig: &Aig) -> bool {
    let bad = bads(aig);
    reachable(aig).iter().any(|s| {
        assignments(aig).any(|x| {
            let v = aig.eval(&x, s);
            aig.constraints().iter().all(|c| Aig::value(&v, c.lit)) && bad.iter().any(|&b| Aig::value(&v, b))
        })
    })
}

/// A deterministic random sequential circuit: `latches` latches of random initial values over `inputs` inputs and
/// `gates` random gates, one bad state, sometimes a constraint; with the shapes correspondence finds mixed in, latches
/// that input 0 resets to their initial values and latches loaded as an earlier one is.
fn random_circuit(seed: u64, latches: usize, inputs: usize, gates: usize) -> Aig {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut g = Aig::new();
    let xs: Vec<AigLit> = (0..inputs).map(|_| g.add_input(None)).collect();
    let inits: Vec<Init> = (0..latches)
        .map(|_| match next() % 3 {
            0 => Init::Zero,
            1 => Init::One,
            _ => Init::Free,
        })
        .collect();
    let ls: Vec<usize> = inits.iter().map(|&i| g.add_latch(None, i)).collect();
    let mut pool: Vec<AigLit> = xs.clone();
    pool.extend(ls.iter().map(|&k| g.latch_lit(k)));
    for _ in 0..gates {
        let a = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let b = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let c = g.and(a, b);
        pool.push(c);
    }
    let mut nexts: Vec<AigLit> = Vec::new();
    for (j, &k) in ls.iter().enumerate() {
        let any = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let n = match next() % 6 {
            0 | 1 if inits[j] != Init::Free => {
                let held = if inits[j] == Init::One { AigLit::TRUE } else { AigLit::FALSE };
                let stay = if next() & 1 == 1 { g.latch_lit(k) } else { any };
                g.mux(xs[0], held, stay)
            }
            2 if j > 0 => nexts[(next() % j as u64) as usize],
            _ => any,
        };
        g.set_next(k, n);
        nexts.push(n);
    }
    let bad = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
    g.add_bad(bad, None);
    if next() % 4 == 0 {
        let c = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        g.add_constraint(c, None);
    }
    g
}

/// Circuits of two to seven latches over one to three inputs.
fn circuits() -> impl Iterator<Item = (u64, Aig)> {
    (0..400u64).map(|seed| (seed, random_circuit(seed, 2 + (seed % 6) as usize, 1 + (seed % 3) as usize, 4 + (seed % 12) as usize)))
}

#[test]
fn every_merged_signal_equals_its_representative_in_every_reachable_state() {
    // Random circuits checked against explicit-state reachability: in every state reached at a clock whose
    // constraints held at every clock before, each merged signal takes its representative's value, in its phase.
    let (mut merged, mut merging) = (0, 0);
    for (seed, aig) in circuits() {
        let corr = find(&aig, &Options::default()).unwrap_or_else(|| panic!("seed {seed}: a small circuit settles"));
        for s in reachable(&aig) {
            let v = aig.eval(&vec![false; aig.inputs().len()], &s);
            for (m, r) in corr.members() {
                assert_eq!(v[m as usize], Aig::value(&v, r), "seed {seed}: node {m} and {r:?} in state {s:?}");
            }
        }
        merged += corr.merged();
        merging += usize::from(corr.merged() > 0);
    }
    assert!(merging >= 300 && merged >= 2000, "{merging} circuits merged {merged} signals");
}

#[test]
fn the_reduced_circuit_reaches_a_bad_state_exactly_when_the_circuit_does() {
    // The reduced circuit against the oracle, and fewer latches left in the cone of its bad states and constraints.
    let (mut unsafe_, mut smaller) = (0, 0);
    for (seed, aig) in circuits() {
        let corr = find(&aig, &Options::default()).expect("a small circuit settles");
        let reduced = reduce(&aig, &corr);
        assert_eq!(reduced.inputs().len(), aig.inputs().len(), "seed {seed}: the inputs in place");
        assert_eq!(reduced.latches().len(), aig.latches().len(), "seed {seed}: the latches in place");
        let bad = reachable_bad(&aig);
        assert_eq!(reachable_bad(&reduced), bad, "seed {seed}: the same verdict");
        unsafe_ += usize::from(bad);
        smaller += usize::from(pdr::cone(&reduced).1.len() < pdr::cone(&aig).1.len());
    }
    assert!(unsafe_ >= 100 && unsafe_ <= 300 && smaller >= 100, "{unsafe_} unsafe, {smaller} with fewer latches");
}

#[test]
fn an_invariant_of_the_reduced_circuit_and_the_equalities_are_one_of_the_circuit() {
    // Each safe circuit's reduced circuit proven by PDR: its invariant, with every equality of the correspondence,
    // checks on the circuit itself.
    let mut lifted = 0;
    for (seed, aig) in circuits() {
        let corr = find(&aig, &Options::default()).expect("a small circuit settles");
        let reduced = reduce(&aig, &corr);
        if let pdr::Verdict::Safe(inv) = pdr::prove_safe(&reduced, &pdr::Options::default()).verdict {
            assert!(check_invariant(&aig, &invariant(&aig, &corr, &inv)), "seed {seed}: the lifted invariant checks");
            lifted += usize::from(corr.merged() > 0);
        }
    }
    assert!(lifted >= 100, "{lifted} invariants lifted over a correspondence that merged");
}

#[test]
fn a_register_its_reset_holds_at_its_initial_value_is_that_constant() {
    // Input `r` resets: `low` starts false and stays false, `high` starts true and stays true, and `toggle`, reset to
    // false, flips otherwise. The first two are the constants; `toggle` and its complement merge with nothing.
    let mut g = Aig::new();
    let r = g.add_input(None);
    let low = g.add_latch(None, Init::Zero);
    let high = g.add_latch(None, Init::One);
    let toggle = g.add_latch(None, Init::Zero);
    let (l, h, t) = (g.latch_lit(low), g.latch_lit(high), g.latch_lit(toggle));
    let held_low = g.mux(r, AigLit::FALSE, l);
    let held_high = g.mux(r, AigLit::TRUE, h);
    let flipped = g.mux(r, AigLit::FALSE, !t);
    g.set_next(low, held_low);
    g.set_next(high, held_high);
    g.set_next(toggle, flipped);
    let either = g.and(t, h);
    g.add_bad(either, None);
    let corr = find(&g, &Options::default()).expect("settles");
    assert_eq!(corr.repr[l.node() as usize], AigLit::FALSE);
    assert_eq!(corr.repr[h.node() as usize], AigLit::TRUE);
    assert_eq!(corr.repr[t.node() as usize], t, "the toggle is no constant");
    assert_eq!(corr.repr[either.node() as usize], t, "the toggle and true is the toggle");
}

#[test]
fn registers_loaded_alike_are_one_and_a_complement_is_its_negation() {
    // `p` and `q` start false and load `x`; `n` starts true and loads not `x`: `q` is `p`, and `n` is not `p`.
    let mut g = Aig::new();
    let x = g.add_input(None);
    let (p, q, n) = (g.add_latch(None, Init::Zero), g.add_latch(None, Init::Zero), g.add_latch(None, Init::One));
    g.set_next(p, x);
    g.set_next(q, x);
    g.set_next(n, !x);
    let (lp, lq, ln) = (g.latch_lit(p), g.latch_lit(q), g.latch_lit(n));
    let both = g.and(lq, ln);
    g.add_bad(both, None);
    let corr = find(&g, &Options::default()).expect("settles");
    assert_eq!(corr.repr[lq.node() as usize], lp);
    assert_eq!(corr.repr[ln.node() as usize], !lp);
    assert_eq!(corr.repr[both.node() as usize], AigLit::FALSE, "`q` and not `p` never hold together");
    let reduced = reduce(&g, &corr);
    assert!(pdr::cone(&reduced).1.is_empty(), "the bad state is the constant false, reading no latch");
}

#[test]
fn a_latch_free_to_start_anywhere_merges_with_nothing_it_can_differ_from_at_the_start() {
    // `free` and `zero` both load `x`, equal from the second clock on; at the first, `free` may be true.
    let mut g = Aig::new();
    let x = g.add_input(None);
    let (free, zero) = (g.add_latch(None, Init::Free), g.add_latch(None, Init::Zero));
    g.set_next(free, x);
    g.set_next(zero, x);
    let (lf, lz) = (g.latch_lit(free), g.latch_lit(zero));
    let differ = g.xor(lf, lz);
    g.add_bad(differ, None);
    let corr = find(&g, &Options::default()).expect("settles");
    assert_eq!(corr.repr[lz.node() as usize], lz);
    assert_eq!(corr.repr[lf.node() as usize], lf);
}

#[test]
fn a_constraint_holds_back_the_clocks_after_it_and_never_the_first() {
    // Constraint `x`: `p` loads `x`, `q` loads true, so on every run whose constraint held before a clock the two agree
    // there, from false at the first clock. `free` starts anywhere and the constraint `free` is asked of the first
    // clock too, but no constraint is assumed of the first clock: `free` is not the constant true it is on every run
    // the constraint keeps.
    let mut g = Aig::new();
    let x = g.add_input(None);
    let (p, q, free) = (g.add_latch(None, Init::Zero), g.add_latch(None, Init::Zero), g.add_latch(None, Init::Free));
    g.set_next(p, x);
    g.set_next(q, AigLit::TRUE);
    let lf = g.latch_lit(free);
    g.set_next(free, lf);
    g.add_constraint(x, None);
    g.add_constraint(lf, None);
    let (lp, lq) = (g.latch_lit(p), g.latch_lit(q));
    let differ = g.xor(lp, lq);
    g.add_bad(differ, None);
    let corr = find(&g, &Options::default()).expect("settles");
    assert_eq!(corr.repr[lq.node() as usize], lp, "`q` is `p` wherever the constraint held before");
    assert_eq!(corr.repr[lf.node() as usize], lf, "no constraint assumed at the first clock");
    assert!(!reachable_bad(&reduce(&g, &corr)) && !reachable_bad(&g));
}

#[test]
fn a_circuit_past_the_node_bound_is_left_alone() {
    let mut g = Aig::new();
    let x = g.add_input(None);
    let l = g.add_latch(None, Init::Zero);
    g.set_next(l, x);
    let bad = g.latch_lit(l);
    g.add_bad(bad, None);
    assert!(find(&g, &Options { max_nodes: 2, ..Options::default() }).is_none());
    assert!(find(&g, &Options::default()).is_some());
}

#[test]
fn a_pair_the_sweeper_leaves_open_is_never_merged() {
    // A sweeper held to one conflict a query leaves pairs open; each is taken apart, never merged on the strength of the
    // simulation, so every merge still holds in every reachable state and the reduced circuit keeps the verdict.
    let starved = Options { fraig: fraig::Options { budget: 1, final_budget: 1, rounds: 1, ..fraig::Options::default() }, ..Options::default() };
    let mut merged = 0;
    for (seed, aig) in circuits() {
        let Some(corr) = find(&aig, &starved) else { continue };
        for s in reachable(&aig) {
            let v = aig.eval(&vec![false; aig.inputs().len()], &s);
            for (m, r) in corr.members() {
                assert_eq!(v[m as usize], Aig::value(&v, r), "seed {seed}: node {m} and {r:?} in state {s:?}");
            }
        }
        assert_eq!(reachable_bad(&reduce(&aig, &corr)), reachable_bad(&aig), "seed {seed}: the same verdict");
        merged += corr.merged();
    }
    assert!(merged >= 500, "{merged} signals merged under the starved sweeper");
}

#[test]
fn a_search_past_its_sweeping_budget_merges_nothing() {
    // The random circuits with the sweeper handed forty nodes over every round: a circuit whose questions need more is
    // left alone, and one whose correspondence is found merges what holds.
    let tight = Options { nodes: 40, ..Options::default() };
    let (mut left, mut found) = (0, 0);
    for (seed, aig) in circuits() {
        match find(&aig, &tight) {
            None => left += 1,
            Some(corr) => {
                for s in reachable(&aig) {
                    let v = aig.eval(&vec![false; aig.inputs().len()], &s);
                    assert!(corr.members().all(|(m, r)| v[m as usize] == Aig::value(&v, r)), "seed {seed}");
                }
                found += 1;
            }
        }
    }
    assert!(left >= 100 && found >= 50, "{left} left alone, {found} found");
}
