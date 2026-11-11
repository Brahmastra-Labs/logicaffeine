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
    (0..800u64).map(|seed| (seed, random_circuit(seed, 2 + (seed % 6) as usize, 1 + (seed % 3) as usize, 4 + (seed % 12) as usize)))
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
    let (mut unsafe_, mut safe, mut smaller) = (0, 0, 0);
    for (seed, aig) in circuits() {
        let corr = find(&aig, &Options::default()).expect("a small circuit settles");
        let reduced = reduce(&aig, &corr);
        assert_eq!(reduced.inputs().len(), aig.inputs().len(), "seed {seed}: the inputs in place");
        assert_eq!(reduced.latches().len(), aig.latches().len(), "seed {seed}: the latches in place");
        let bad = reachable_bad(&aig);
        assert_eq!(reachable_bad(&reduced), bad, "seed {seed}: the same verdict");
        unsafe_ += usize::from(bad);
        safe += usize::from(!bad);
        smaller += usize::from(pdr::cone(&reduced).1.len() < pdr::cone(&aig).1.len());
    }
    assert!(unsafe_ >= 100 && safe >= 100 && smaller >= 100, "{unsafe_} unsafe, {safe} safe, {smaller} with fewer latches");
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
    // The random circuits with the sweeper handed fifteen nodes over every round: a circuit whose questions need more
    // is left alone, and one whose correspondence is found merges what holds.
    let tight = Options { nodes: 15, ..Options::default() };
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

/// Sixteen latches free to start anywhere, each holding its value, and `p`, the conjunction of sixteen parity equations
/// over them, `x_i ⊕ x_(i+1) ⊕ x_(i+3) = b_i` around the ring, whose right sides a planted state sets. The circulant
/// `1 + z + z³` is prime to `z¹⁶ - 1` over GF(2), so the system has full rank and `p` holds in that one state. Returns
/// the circuit with no bad state yet, the latches, `p` and the planted state.
fn a_parity_lock() -> (Aig, Vec<AigLit>, AigLit, Vec<bool>) {
    const N: usize = 16;
    let planted: Vec<bool> = (0..N).map(|i| 0x9E37u32 >> i & 1 == 1).collect();
    let mut g = Aig::new();
    let mut xs = Vec::with_capacity(N);
    for _ in 0..N {
        let k = g.add_latch(None, Init::Free);
        let l = g.latch_lit(k);
        g.set_next(k, l);
        xs.push(l);
    }
    let mut p = AigLit::TRUE;
    for i in 0..N {
        let (a, b, c) = (i, (i + 1) % N, (i + 3) % N);
        let ab = g.xor(xs[a], xs[b]);
        let e = g.xor(ab, xs[c]);
        let holds = if planted[a] ^ planted[b] ^ planted[c] { e } else { !e };
        p = g.and(p, holds);
    }
    (g, xs, p, planted)
}

/// The rank over GF(2) of the parity lock's equations, each a row of the latches it reads.
fn parity_lock_rank() -> usize {
    let mut rows: Vec<u32> = (0..16).map(|i| 1 << i | 1 << (i + 1) % 16 | 1 << (i + 3) % 16).collect();
    let mut rank = 0;
    for bit in 0..16 {
        let Some(pivot) = (rank..rows.len()).find(|&r| rows[r] >> bit & 1 == 1) else { continue };
        rows.swap(rank, pivot);
        for r in 0..rows.len() {
            if r != rank && rows[r] >> bit & 1 == 1 {
                rows[r] ^= rows[rank];
            }
        }
        rank += 1;
    }
    rank
}

/// Every start state of `aig`'s first sixteen latches with the rest at `rest`: the parity lock's states, each held.
fn lock_states(rest: &[bool]) -> impl Iterator<Item = Vec<bool>> + '_ {
    (0..1u32 << 16).map(move |bits| (0..16).map(|i| bits >> i & 1 == 1).chain(rest.iter().copied()).collect())
}

#[test]
fn pairs_a_starved_sweeper_leaves_open_are_taken_apart_however_rarely_they_differ() {
    // The parity lock's gates differ from the constant and from each other only in few of its 2^16 states, `p` in one:
    // the random runs leave them in the constant's class, and a sweeper held to one conflict a query cannot tell `p`
    // from false. Every pair left open is taken apart, so every merge holds in every state, each a state of a run.
    let (mut aig, _, p, planted) = a_parity_lock();
    aig.add_bad(p, None);
    assert_eq!(parity_lock_rank(), 16, "the equations single out one state");
    assert!(Aig::value(&aig.eval(&[], &planted), p), "p holds in the planted state");
    let classes = simulated(&aig, &state_signals(&aig), &Options::default());
    assert_eq!(classes.rep[p.node() as usize], 0, "the random runs leave p in the constant's class");
    let starved = Options { fraig: fraig::Options { budget: 1, final_budget: 1, rounds: 1, ..fraig::Options::default() }, ..Options::default() };
    let (open, _, _) = fraig::decide(&aig, &[(p, AigLit::FALSE)], &starved.fraig);
    assert!(matches!(open[0], Answer::Unknown(_)), "a sweeper held to one conflict leaves p ≡ false open");
    let (told, _, _) = fraig::decide(&aig, &[(p, AigLit::FALSE)], &Options::default().fraig);
    assert!(matches!(told[0], Answer::Differ { .. }), "the sweeper at its default budget tells them apart");
    for (name, opts) in [("default", Options::default()), ("starved", starved)] {
        let corr = find(&aig, &opts).unwrap_or_else(|| panic!("{name}: the search settles"));
        for s in lock_states(&[]) {
            let v = aig.eval(&[], &s);
            for (m, r) in corr.members() {
                assert_eq!(v[m as usize], Aig::value(&v, r), "{name}: node {m} and {r:?} in state {s:?}");
            }
        }
        let reduced = reduce(&aig, &corr);
        assert!(bads(&reduced).iter().any(|&b| Aig::value(&reduced.eval(&[], &planted), b)), "{name}: still bad where p holds");
    }
}

#[test]
fn signals_alike_from_the_second_clock_that_differ_in_one_start_state_are_told_apart_by_the_base_question() {
    // `a` starts false and `c` anywhere, both loading the input `x`; `f` is false at the first clock only; and
    // `v = c ∧ (p ∨ f)` over the parity lock's `p`. From the second clock on `v` is `c`, which is `a`: the step proves
    // them equal. At the first clock `v` is `c ∧ p`, true in one start state of 2^17, which no random run meets, so only
    // the base question finds it and takes `v` apart from `a`. The bad state `a ⊕ v` holds there: merged, the reduced
    // circuit would be safe.
    let (mut aig, _, p, planted) = a_parity_lock();
    let x = aig.add_input(None);
    let (ka, kc, kf) = (aig.add_latch(None, Init::Zero), aig.add_latch(None, Init::Free), aig.add_latch(None, Init::Zero));
    let (a, c, f) = (aig.latch_lit(ka), aig.latch_lit(kc), aig.latch_lit(kf));
    aig.set_next(ka, x);
    aig.set_next(kc, x);
    aig.set_next(kf, AigLit::TRUE);
    let open = aig.or(p, f);
    let v = aig.and(c, open);
    let bad = aig.xor(a, v);
    aig.add_bad(bad, None);
    let start: Vec<bool> = planted.iter().copied().chain([false, true, false]).collect();
    assert!(Aig::value(&aig.eval(&[false], &start), bad), "the planted start with c true is bad at once");
    let classes = simulated(&aig, &state_signals(&aig), &Options::default());
    assert_eq!(classes.rep[v.node() as usize], a.node(), "the random runs leave v in a's class");
    let corr = find(&aig, &Options::default()).expect("the search settles");
    assert!(corr.members().all(|(m, _)| m != v.node()), "v is merged with nothing");
    for s in reachable(&aig) {
        for inputs in [[false], [true]] {
            let values = aig.eval(&inputs, &s);
            for (m, r) in corr.members() {
                assert_eq!(values[m as usize], Aig::value(&values, r), "node {m} and {r:?} in state {s:?}");
            }
        }
    }
    let reduced = reduce(&aig, &corr);
    assert!(bads(&reduced).iter().any(|&b| Aig::value(&reduced.eval(&[false], &start), b)), "the reduced circuit is bad at the planted start");
}

#[test]
fn a_search_whose_questions_outgrow_its_node_budget_merges_nothing() {
    // The parity lock's base question carries the gates of its sixteen equations: ten nodes over every round cannot
    // hold it, so the search gives up and merges nothing; at the default budget it settles.
    let (mut aig, _, p, _) = a_parity_lock();
    aig.add_bad(p, None);
    assert!(find(&aig, &Options { nodes: 10, ..Options::default() }).is_none(), "ten nodes cannot hold the base question");
    assert!(find(&aig, &Options::default()).is_some(), "the default budget settles");
}
