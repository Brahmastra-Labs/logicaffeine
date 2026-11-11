use super::*;
use crate::aig::Init;
use crate::itp::{check_induction, check_invariant, replays};
use std::collections::{HashSet, VecDeque};

/// The bad-state literals: the bad-state properties, or the outputs of a circuit that has none.
fn bads(aig: &Aig) -> Vec<AigLit> {
    let ports = if aig.bad().is_empty() { aig.outputs() } else { aig.bad() };
    ports.iter().map(|p| p.lit).collect()
}

/// Explicit-state reachability, the oracle: from every initial state, every input with the constraints holding steps
/// to the next state; a bad state reached under the constraints makes the circuit unsafe.
fn reachable_bad(aig: &Aig) -> bool {
    let latches = aig.latches();
    let ni = aig.inputs().len();
    let bad = bads(aig);
    let mut initial: Vec<Vec<bool>> = vec![Vec::new()];
    for l in latches {
        let choices: &[bool] = match l.init {
            Init::Zero => &[false],
            Init::One => &[true],
            Init::Free => &[false, true],
        };
        initial = initial.iter().flat_map(|s| choices.iter().map(move |&c| [s.clone(), vec![c]].concat())).collect();
    }
    let mut seen: HashSet<Vec<bool>> = initial.iter().cloned().collect();
    let mut queue: VecDeque<Vec<bool>> = initial.into_iter().collect();
    while let Some(s) = queue.pop_front() {
        for bits in 0..1u64 << ni {
            let x: Vec<bool> = (0..ni).map(|i| bits >> i & 1 == 1).collect();
            let v = aig.eval(&x, &s);
            if !aig.constraints().iter().all(|c| Aig::value(&v, c.lit)) {
                continue;
            }
            if bad.iter().any(|&b| Aig::value(&v, b)) {
                return true;
            }
            let next: Vec<bool> = latches.iter().map(|l| Aig::value(&v, l.next)).collect();
            if seen.insert(next.clone()) {
                queue.push_back(next);
            }
        }
    }
    false
}

/// A deterministic random sequential circuit: `latches` latches of random initial values, `inputs` inputs, `gates`
/// random gates over them, random next states, one bad-state property, sometimes a constraint.
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
    let ls: Vec<usize> = (0..latches)
        .map(|_| {
            let init = match next() % 3 {
                0 => Init::Zero,
                1 => Init::One,
                _ => Init::Free,
            };
            g.add_latch(None, init)
        })
        .collect();
    let mut pool: Vec<AigLit> = xs;
    pool.extend(ls.iter().map(|&k| g.latch_lit(k)));
    for _ in 0..gates {
        let a = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let b = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let c = g.and(a, b);
        pool.push(c);
    }
    let pick = |next: &mut dyn FnMut() -> u64| pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
    for &k in &ls {
        let n = pick(&mut next);
        g.set_next(k, n);
    }
    let bad = pick(&mut next);
    g.add_bad(bad, None);
    if next() % 4 == 0 {
        let c = pick(&mut next);
        g.add_constraint(c, None);
    }
    g
}

/// That a verdict is the oracle's, its certificate checking on the whole circuit: whether it is safe.
fn assert_oracles(aig: &Aig, verdict: &Verdict, what: &str) -> bool {
    let bad = reachable_bad(aig);
    match verdict {
        Verdict::Safe(inv) => {
            assert!(!bad, "{what}: Safe for a circuit that reaches a bad state");
            assert!(check_invariant(aig, inv), "{what}: the invariant checks on the whole circuit");
            true
        }
        Verdict::Induction(k) => {
            assert!(!bad, "{what}: inductive for a circuit that reaches a bad state");
            assert!(check_induction(aig, *k), "{what}: the induction checks on the whole circuit");
            true
        }
        Verdict::Unsafe(trace) => {
            assert!(bad, "{what}: Unsafe for a safe circuit");
            assert!(replays(aig, trace), "{what}: the run replays on the whole circuit");
            false
        }
        Verdict::Unknown(why) => panic!("{what}: a circuit of seven latches is decided ({why})"),
    }
}

#[test]
fn every_verdict_is_the_explicit_state_oracles_on_random_circuits() {
    // Random circuits up to seven latches, abstracted at the default stability and at none (the empty abstraction
    // decided first, every latch earned by a refinement): every verdict the oracle's, each certificate checking on the
    // whole circuit, and refinements taken.
    let (mut safe, mut unsafe_, mut refined) = (0, 0, 0);
    for seed in 0..500u64 {
        let latches = 2 + (seed % 6) as usize;
        let aig = random_circuit(seed, latches, 1 + (seed % 3) as usize, 4 + (seed % 12) as usize);
        for stable in [Options::default().stable, 0] {
            let report = prove_safe(&aig, &Options { stable, direct: 0, ..Options::default() });
            if assert_oracles(&aig, &report.verdict, &format!("seed {seed}, stable {stable}")) {
                safe += 1;
            } else {
                unsafe_ += 1;
            }
            assert!(report.kept <= aig.latches().len(), "seed {seed}: no more latches kept than the circuit has");
            refined += report.refinements.min(1);
        }
    }
    assert!(safe >= 150 && unsafe_ >= 150 && refined >= 100, "{safe} safe, {unsafe_} unsafe, {refined} refined");
}

#[test]
fn a_proof_keeps_the_latch_it_needs_and_none_of_the_logic_beside_it() {
    // A latch `a` that starts false and stays false; the bad state is `a` and a conjunction of sixty toggling latches
    // the property's cone holds. Only `a` is needed: the abstraction keeps it alone and proves the circuit safe, its
    // invariant checking on the whole circuit.
    let mut g = Aig::new();
    let x = g.add_input(None);
    let a = g.add_latch(None, Init::Zero);
    let la = g.latch_lit(a);
    let stays = g.and(la, x);
    g.set_next(a, stays);
    let mut all = la;
    for _ in 0..60 {
        let k = g.add_latch(None, Init::Free);
        let l = g.latch_lit(k);
        g.set_next(k, !l);
        all = g.and(all, l);
    }
    g.add_bad(all, None);
    let report = prove_safe(&g, &Options { direct: 0, ..Options::default() });
    assert!(matches!(&report.verdict, Verdict::Safe(inv) if check_invariant(&g, inv)), "{:?}", report.verdict);
    assert_eq!(report.kept, 1, "the latch `a` alone");
}

#[test]
fn a_run_of_the_abstraction_the_circuit_cannot_take_refines_it_until_the_proof_holds() {
    // A shift register of zeros, ten stages, bad when the last is set: decided from the empty abstraction, each run of
    // an abstraction breaks the stage it left out, and the tied unrolling names it. The proof holds once every stage
    // is kept, after a refinement for each.
    let mut g = Aig::new();
    let stages: Vec<usize> = (0..10).map(|_| g.add_latch(None, Init::Zero)).collect();
    g.set_next(stages[0], AigLit::FALSE);
    for w in stages.windows(2) {
        let previous = g.latch_lit(w[0]);
        g.set_next(w[1], previous);
    }
    g.add_bad(g.latch_lit(stages[9]), None);
    let report = prove_safe(&g, &Options { stable: 0, direct: 0, ..Options::default() });
    assert!(matches!(&report.verdict, Verdict::Safe(inv) if check_invariant(&g, inv)), "{:?}", report.verdict);
    assert!(report.refinements >= 1, "refined: {}", report.refinements);
    assert!(report.kept <= 10);
}

#[test]
fn an_unsafe_circuit_is_answered_by_its_own_run_however_coarse_the_abstraction() {
    // The same shift register loading an input: bad at the eleventh clock. From the empty abstraction on, the run that
    // answers is the circuit's own, as long as the shortest, and it replays on the whole circuit.
    let mut g = Aig::new();
    let i = g.add_input(None);
    let stages: Vec<usize> = (0..10).map(|_| g.add_latch(None, Init::Zero)).collect();
    g.set_next(stages[0], i);
    for w in stages.windows(2) {
        let previous = g.latch_lit(w[0]);
        g.set_next(w[1], previous);
    }
    g.add_bad(g.latch_lit(stages[9]), None);
    for stable in [0, Options::default().stable] {
        let report = prove_safe(&g, &Options { stable, direct: 0, ..Options::default() });
        match &report.verdict {
            Verdict::Unsafe(trace) => {
                assert!(replays(&g, trace), "stable {stable}: the run replays");
                assert_eq!(trace.inputs.len(), 11, "stable {stable}: the shortest run");
            }
            other => panic!("stable {stable}: {other:?}"),
        }
    }
}

#[test]
fn a_cone_of_at_most_the_direct_latches_is_decided_whole() {
    // The circuit of sixty toggling latches beside `a`: at the default, its sixty-one latches are decided by PDR with
    // every latch kept and no refinement, the verdict the same.
    let mut g = Aig::new();
    let x = g.add_input(None);
    let a = g.add_latch(None, Init::Zero);
    let la = g.latch_lit(a);
    let stays = g.and(la, x);
    g.set_next(a, stays);
    let mut all = la;
    for _ in 0..60 {
        let k = g.add_latch(None, Init::Free);
        let l = g.latch_lit(k);
        g.set_next(k, !l);
        all = g.and(all, l);
    }
    g.add_bad(all, None);
    let report = prove_safe(&g, &Options::default());
    assert!(matches!(&report.verdict, Verdict::Safe(inv) if check_invariant(&g, inv)), "{:?}", report.verdict);
    assert_eq!((report.kept, report.refinements), (61, 0), "the whole cone, unrefined");
}

#[test]
fn every_verdict_is_the_oracles_with_the_cone_reduced_by_its_correspondence() {
    // The random circuits again, each cone first reduced by its signal correspondence: every verdict the oracle's, its
    // certificate checking on the whole circuit.
    let (mut safe, mut unsafe_) = (0, 0);
    for seed in 0..500u64 {
        let latches = 2 + (seed % 6) as usize;
        let aig = random_circuit(seed, latches, 1 + (seed % 3) as usize, 4 + (seed % 12) as usize);
        for direct in [0, Options::default().direct] {
            let report = prove_safe(&aig, &Options { direct, correspond: true, ..Options::default() });
            if assert_oracles(&aig, &report.verdict, &format!("seed {seed}, direct {direct}")) {
                safe += 1;
            } else {
                unsafe_ += 1;
            }
        }
    }
    assert!(safe >= 150 && unsafe_ >= 150, "{safe} safe, {unsafe_} unsafe");
}

#[test]
fn a_cone_its_correspondence_makes_inductive_is_answered_by_an_invariant_of_the_whole() {
    // A latch that starts false and holds, feeding a chain of eight that all start false; the bad state is the chain's
    // last. Its correspondence makes every latch the constant false, the reduced cone none and inductive at once,
    // while the circuit itself is inductive only nine clocks deep: the answer is an invariant, the equalities in it,
    // checking on the whole circuit, and no latch is searched.
    let mut g = Aig::new();
    let c = g.add_latch(None, Init::Zero);
    let lc = g.latch_lit(c);
    g.set_next(c, lc);
    let mut last = lc;
    for _ in 0..8 {
        let k = g.add_latch(None, Init::Zero);
        g.set_next(k, last);
        last = g.latch_lit(k);
    }
    g.add_bad(last, None);
    assert!(!check_induction(&g, 4), "the circuit is no shallow induction");
    let report = prove_safe(&g, &Options { correspond: true, ..Options::default() });
    assert!(matches!(&report.verdict, Verdict::Safe(inv) if check_invariant(&g, inv)), "{:?}", report.verdict);
    assert_eq!(report.kept, 0, "the reduced cone has no latch to search");
}

#[test]
fn a_correspondence_that_leaves_most_of_the_cone_is_set_aside() {
    // Twenty latches each loading an input of its own, the first one twice over, and a bad state when all twenty-one
    // hold: the correspondence merges the twin alone, a reduction that leaves twenty of the twenty-one, so the cone is
    // decided as it is, every latch searched, and the run to the bad state replays.
    let mut g = Aig::new();
    let mut all = AigLit::TRUE;
    let mut first = None;
    for k in 0..20 {
        let x = g.add_input(None);
        let l = g.add_latch(None, Init::Zero);
        g.set_next(l, x);
        all = g.and(all, g.latch_lit(l));
        if k == 0 {
            first = Some(x);
        }
    }
    let twin = g.add_latch(None, Init::Zero);
    g.set_next(twin, first.expect("the first input"));
    all = g.and(all, g.latch_lit(twin));
    g.add_bad(all, None);
    let report = prove_safe(&g, &Options { correspond: true, ..Options::default() });
    assert!(matches!(&report.verdict, Verdict::Unsafe(run) if replays(&g, run)), "{:?}", report.verdict);
    assert_eq!(report.kept, 21, "every latch searched");
}
