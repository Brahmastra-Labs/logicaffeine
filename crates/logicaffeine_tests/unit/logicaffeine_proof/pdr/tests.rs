use super::*;
use crate::aig::Init;
use crate::itp::{self, ItpOptions, ItpVerdict};
use std::collections::{HashSet, VecDeque};

/// Explicit-state reachability, the oracle: from every initial state, every input with the constraints holding steps
/// to the next state; a bad state reached under the constraints makes the circuit unsafe.
fn reachable_bad(aig: &Aig) -> bool {
    let latches = aig.latches();
    let nl = latches.len();
    let ni = aig.inputs().len();
    let bad = bad_lits(aig);
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
            debug_assert_eq!(next.len(), nl);
            if seen.insert(next.clone()) {
                queue.push_back(next);
            }
        }
    }
    false
}

/// Every state the circuit can reach under its constraints (whatever is bad), by the same search.
fn reachable_states(aig: &Aig) -> HashSet<Vec<bool>> {
    let latches = aig.latches();
    let ni = aig.inputs().len();
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
            let next: Vec<bool> = latches.iter().map(|l| Aig::value(&v, l.next)).collect();
            if seen.insert(next.clone()) {
                queue.push_back(next);
            }
        }
    }
    seen
}

/// A `width`-bit counter that increments when its enable input is high and wraps at `modulus`, all latches zero at
/// first; bad when its value equals `bad_value`.
fn counter(width: usize, modulus: u64, bad_value: u64) -> Aig {
    let mut g = Aig::new();
    let en = g.add_input(Some("en".into()));
    let ls: Vec<usize> = (0..width).map(|_| g.add_latch(None, Init::Zero)).collect();
    let bits: Vec<AigLit> = ls.iter().map(|&k| g.latch_lit(k)).collect();
    let equals = |g: &mut Aig, value: u64| {
        let mut e = AigLit::TRUE;
        for (i, &b) in bits.iter().enumerate() {
            let want = value >> i & 1 == 1;
            e = g.and(e, b.with_phase(!want));
        }
        e
    };
    let at_top = equals(&mut g, modulus - 1);
    // Ripple increment.
    let mut carry = AigLit::TRUE;
    let mut incremented = Vec::new();
    for &b in &bits {
        incremented.push(g.xor(b, carry));
        carry = g.and(b, carry);
    }
    for (i, &k) in ls.iter().enumerate() {
        let stepped = g.mux(at_top, AigLit::FALSE, incremented[i]);
        let next = g.mux(en, stepped, bits[i]);
        g.set_next(k, next);
    }
    let bad = equals(&mut g, bad_value);
    g.add_bad(bad, Some("bad".into()));
    g
}

fn safe(aig: &Aig) -> Invariant {
    match prove_safe(aig, &Options::default()).verdict {
        Verdict::Safe(inv) => inv,
        other => panic!("expected Safe, got {other:?}"),
    }
}

/// A random circuit of `latches` latches (each starting at zero, one, or anywhere) and `inputs` inputs: And gates over
/// them, each latch's next value and the bad state a literal of the pool, and now and then a constraint.
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

#[test]
fn verdicts_match_explicit_state_reachability_and_interpolation_on_random_circuits() {
    // Circuits up to seven latches: a Safe verdict only for a safe circuit, its invariant checking and holding on every
    // reachable state; an Unsafe one only for an unsafe circuit, its run replaying; never Unknown; and interpolation's
    // verdict the same.
    let (mut safe, mut unsafe_, mut deep) = (0, 0, 0);
    for seed in 0..300u64 {
        let latches = 2 + (seed % 6) as usize;
        let aig = random_circuit(seed, latches, 1 + (seed % 3) as usize, 4 + (seed % 9) as usize);
        let oracle_bad = reachable_bad(&aig);
        let report = prove_safe(&aig, &Options::default());
        match report.verdict {
            Verdict::Safe(inv) => {
                assert!(!oracle_bad, "seed {seed}: Safe for a circuit that reaches a bad state");
                assert!(check_invariant(&aig, &inv), "seed {seed}: the invariant checks");
                for s in reachable_states(&aig) {
                    assert!(inv.holds(&s), "seed {seed}: reachable state {s:?} outside the invariant");
                }
                assert!(matches!(itp::prove_safe(&aig, &ItpOptions::default()), ItpVerdict::Safe(_)), "seed {seed}");
                safe += 1;
            }
            Verdict::Induction(k) => {
                assert!(!oracle_bad, "seed {seed}: inductive for a circuit that reaches a bad state");
                assert!(check_induction(&aig, k), "seed {seed}: the induction checks");
                assert!(matches!(itp::prove_safe(&aig, &ItpOptions::default()), ItpVerdict::Safe(_)), "seed {seed}");
                safe += 1;
            }
            Verdict::Unsafe(trace) => {
                assert!(oracle_bad, "seed {seed}: Unsafe for a safe circuit");
                assert!(replays(&aig, &trace), "seed {seed}: the run replays");
                assert!(matches!(itp::prove_safe(&aig, &ItpOptions::default()), ItpVerdict::Unsafe(_)), "seed {seed}");
                unsafe_ += 1;
            }
            Verdict::Unknown(why) => panic!("seed {seed}: a circuit of at most seven latches is decided ({why})"),
        }
        deep += usize::from(report.stats.frames > 2);
    }
    assert!(safe >= 40 && unsafe_ >= 40 && deep >= 5, "{safe} safe, {unsafe_} unsafe, {deep} past two frames");
}

#[test]
fn a_counter_that_wraps_early_is_proven_safe_with_an_invariant_holding_every_reachable_state() {
    // A mod-5 counter never shows 6 or 7, and a mod-10 one never 12.
    for (width, modulus, bad) in [(3, 5, 6), (3, 5, 7), (4, 10, 12)] {
        let aig = counter(width, modulus, bad);
        let inv = safe(&aig);
        for s in reachable_states(&aig) {
            assert!(inv.holds(&s), "mod {modulus}: {s:?}");
        }
    }
}

#[test]
fn a_counter_that_reaches_its_bad_value_yields_a_shortest_run() {
    // The bad value v is first reachable at the v-th clock: the run is v clocks of counting and the clock at v.
    for (width, modulus, bad) in [(3, 8, 5), (4, 16, 11), (5, 32, 19)] {
        let aig = counter(width, modulus, bad);
        let Verdict::Unsafe(trace) = prove_safe(&aig, &Options::default()).verdict else { panic!("{bad} is reachable") };
        assert!(replays(&aig, &trace));
        assert_eq!(trace.inputs.len() as u64, bad + 1, "a shortest run to {bad}");
    }
}

/// Two `width`-bit counters stepping together from zero on one enable, bad when they differ: safe, by an invariant
/// relating the two that no frame of one counter alone holds.
fn lockstep(width: usize) -> Aig {
    let mut g = Aig::new();
    let en = g.add_input(Some("en".into()));
    let a: Vec<usize> = (0..width).map(|_| g.add_latch(None, Init::Zero)).collect();
    let b: Vec<usize> = (0..width).map(|_| g.add_latch(None, Init::Zero)).collect();
    let mut differ = AigLit::FALSE;
    for side in [&a, &b] {
        let bits: Vec<AigLit> = side.iter().map(|&k| g.latch_lit(k)).collect();
        let mut carry = AigLit::TRUE;
        for (i, &k) in side.iter().enumerate() {
            let inc = g.xor(bits[i], carry);
            carry = g.and(bits[i], carry);
            let next = g.mux(en, inc, bits[i]);
            g.set_next(k, next);
        }
    }
    for i in 0..width {
        let (x, y) = (g.latch_lit(a[i]), g.latch_lit(b[i]));
        let d = g.xor(x, y);
        differ = g.or(differ, d);
    }
    g.add_bad(differ, None);
    g
}

#[test]
fn counters_in_lockstep_are_proven_equal_by_an_invariant_relating_them() {
    for width in [3, 6, 10] {
        let aig = lockstep(width);
        let report = prove_safe(&aig, &Options::default());
        let Verdict::Safe(inv) = report.verdict else { panic!("width {width}: {:?}", report.verdict) };
        assert!(check_invariant(&aig, &inv), "width {width}");
        assert!(report.stats.invariant >= width, "width {width}: a clause a pair of bits at least: {:?}", report.stats);
    }
}

#[test]
fn constraints_and_free_initial_values_are_honoured() {
    let build = |constrained: bool| {
        let mut g = Aig::new();
        let i = g.add_input(None);
        let k = g.add_latch(None, Init::Free);
        let x = g.latch_lit(k);
        g.set_next(k, x);
        let bad = g.and(x, i);
        g.add_bad(bad, None);
        if constrained {
            g.add_constraint(!i, None);
        }
        g
    };
    let free = build(false);
    let Verdict::Unsafe(trace) = prove_safe(&free, &Options::default()).verdict else { panic!("x ∧ i is reachable") };
    assert!(replays(&free, &trace));
    assert_eq!(trace.inputs.len(), 1, "reachable at the first clock");
    safe(&build(true));
}

#[test]
fn outputs_stand_in_for_missing_bad_state_properties() {
    let mut g = Aig::new();
    let k = g.add_latch(None, Init::Zero);
    let x = g.latch_lit(k);
    g.set_next(k, !x);
    g.add_output(x, None);
    assert!(matches!(prove_safe(&g, &Options::default()).verdict, Verdict::Unsafe(_)), "the toggle reaches 1");
    let mut quiet = Aig::new();
    let k = quiet.add_latch(None, Init::Zero);
    let x = quiet.latch_lit(k);
    quiet.set_next(k, !x);
    safe(&quiet);
}

#[test]
fn the_bounds_end_the_search_without_a_verdict() {
    let aig = counter(6, 64, 40);
    let report = prove_safe(&aig, &Options { max_frames: 5, max_obligations: 1_000_000, ..Options::default() });
    assert!(matches!(report.verdict, Verdict::Unknown(_)), "40 lies past five frames: {:?}", report.verdict);
    let report = prove_safe(&aig, &Options { max_frames: 1000, max_obligations: 3, ..Options::default() });
    assert!(matches!(report.verdict, Verdict::Unknown(_)), "{:?}", report.verdict);
}

#[test]
fn a_predecessor_is_lifted_to_the_latches_its_step_needs() {
    // Eight latches; the first's next value is the second's and an input; the rest step on their own. Stepping the
    // first into true needs the second true and nothing of the other six.
    let mut g = Aig::new();
    let x = g.add_input(None);
    let ls: Vec<usize> = (0..8).map(|_| g.add_latch(None, Init::Zero)).collect();
    let next0 = g.and(g.latch_lit(ls[1]), x);
    g.set_next(ls[0], next0);
    for &k in &ls[1..] {
        let l = g.latch_lit(k);
        g.set_next(k, !l);
    }
    g.add_bad(g.latch_lit(ls[0]), None);
    let mut search = Search::new(&g);
    let state = vec![Some(true); 8];
    let target = [search.step.primed(Lit::pos(0))];
    let lifted = search.lift(&state, &[true], &target);
    assert_eq!(lifted, vec![Lit::pos(1)], "the second latch true, and none of the rest");
}

#[test]
fn propagation_pushes_a_clause_the_step_keeps_and_finds_the_frame_it_empties() {
    // A latch that starts false and stays false: the clause ¬l blocked at frame 1 holds at frame 2 too, so frame 1 is
    // left with no clause of its own.
    let mut g = Aig::new();
    let k = g.add_latch(None, Init::Zero);
    let l = g.latch_lit(k);
    g.set_next(k, l);
    g.add_bad(l, None);
    let mut search = Search::new(&g);
    search.open();
    search.open();
    search.block(&[Lit::pos(0)], 1);
    assert_eq!(search.propagate(1), Some(1));
    assert_eq!(search.levels[2], vec![vec![Lit::neg(0)]], "the clause moved to frame 2");
    assert!(search.levels[1].is_empty());
}

#[test]
fn a_predecessor_whose_lifted_cube_holds_an_initial_state_is_a_run_from_that_state() {
    // Two latches from zero, the first loading an input and the second the first; bad when the second is false. A
    // state (0, 1) steps into the bad cube ¬b by the first latch alone, and ¬a holds the initial state: the run starts
    // there, not at (0, 1), and replays.
    let mut g = Aig::new();
    let x = g.add_input(None);
    let a = g.add_latch(None, Init::Zero);
    let b = g.add_latch(None, Init::Zero);
    g.set_next(a, x);
    g.set_next(b, g.latch_lit(a));
    g.add_bad(!g.latch_lit(b), None);
    let mut search = Search::new(&g);
    search.obligations.push(Obligation { cube: vec![Lit::neg(1)], inputs: vec![false], parent: None });
    match search.predecessor(0, vec![Some(false), Some(true)], vec![true]) {
        Err(trace) => {
            assert_eq!(trace.initial, vec![false, false], "the initial state the lifted cube holds");
            assert!(replays(&g, &trace), "{trace:?}");
        }
        Ok(at) => panic!("{:?} holds the initial state", search.obligations[at].cube),
    }
    search.obligations.clear();
    search.obligations.push(Obligation { cube: vec![Lit::pos(1)], inputs: vec![false], parent: None });
    let pred = search.predecessor(0, vec![Some(true), Some(false)], vec![true]);
    assert!(pred.is_ok(), "the first latch needs to be true, which no initial state is");
}

/// A verdict as text that two runs of one search agree on: its kind and its certificate.
fn verdict_text(v: &Verdict) -> String {
    match v {
        Verdict::Safe(inv) => format!("safe {} {:?}", inv.aig.to_ascii(), inv.root),
        Verdict::Induction(k) => format!("induction {k}"),
        Verdict::Unsafe(trace) => format!("unsafe {trace:?}"),
        Verdict::Unknown(why) => format!("unknown {why}"),
    }
}

#[test]
fn a_search_resumed_in_slices_is_the_search_run_whole() {
    // Counters of two to four bits, every modulus and bad value, searched whole and then one to four obligations at a
    // time: the same verdict, the same certificate, and the same frames, obligations and solves.
    let mut designs = Vec::new();
    for width in 2..=4usize {
        for modulus in 2..=1u64 << width {
            for bad in 0..1u64 << width {
                designs.push(counter(width, modulus, bad));
            }
        }
    }
    let mut sliced = 0;
    for (seed, aig) in designs.iter().enumerate() {
        let aig = aig.clone();
        let seed = seed as u64;
        let mut run_whole = Search::new(&aig);
        let whole_verdict = run_whole.resume(&Options::default(), usize::MAX).expect("a verdict");
        let whole = Report { verdict: whole_verdict, stats: run_whole.stats };
        let mut search = Search::new(&aig);
        let budget = 1 + (seed % 4) as usize;
        let mut slices = 0;
        let verdict = loop {
            slices += 1;
            if let Some(v) = search.resume(&Options::default(), budget) {
                break v;
            }
        };
        assert_eq!(verdict_text(&verdict), verdict_text(&whole.verdict), "seed {seed}");
        assert_eq!(search.stats, whole.stats, "seed {seed}");
        sliced += usize::from(slices > 2);
    }
    assert!(sliced >= 100, "{sliced} searches resumed more than twice");
}

/// `aig` with `extra` latches beside it that nothing it decides reads: each loads the exclusive-or of a new input and
/// the latch before it, each starting at zero, one or anywhere in turn.
fn with_noise(mut aig: Aig, extra: usize) -> Aig {
    let x = aig.add_input(None);
    let mut prev = x;
    for k in 0..extra {
        let init = [Init::Zero, Init::One, Init::Free][k % 3];
        let j = aig.add_latch(None, init);
        let l = aig.latch_lit(j);
        let next = aig.xor(prev, l);
        aig.set_next(j, next);
        prev = l;
    }
    aig
}

#[test]
fn latches_outside_the_cone_of_the_bad_states_and_constraints_are_left_out_of_the_search() {
    // A mod-5 counter and a counter that reaches its bad value, each beside twenty latches the property never reads:
    // the search runs over the counter's three latches, and its invariant and its run check against the whole
    // circuit. A constraint reading one of the twenty brings it and the latches before it into the search.
    let safe_one = with_noise(counter(3, 5, 6), 20);
    let report = prove_safe(&safe_one, &Options::default());
    assert_eq!(report.stats.latches, 3);
    let Verdict::Safe(inv) = &report.verdict else { panic!("{:?}", report.verdict) };
    assert_eq!(inv.aig.inputs().len(), 23, "an invariant over every latch of the circuit");
    assert!(check_invariant(&safe_one, inv));
    let unsafe_one = with_noise(counter(3, 8, 5), 20);
    let report = prove_safe(&unsafe_one, &Options::default());
    assert_eq!(report.stats.latches, 3);
    let Verdict::Unsafe(trace) = &report.verdict else { panic!("{:?}", report.verdict) };
    assert_eq!(trace.initial.len(), 23, "a run of the whole circuit");
    assert!(replays(&unsafe_one, trace));
    let mut constrained = with_noise(counter(3, 5, 6), 20);
    let fourth = constrained.latch_lit(3 + 4);
    constrained.add_constraint(!fourth, None);
    let report = prove_safe(&constrained, &Options::default());
    assert_eq!(report.stats.latches, 3 + 5, "the counter's, the constrained latch and the four before it");
    assert!(matches!(report.verdict, Verdict::Safe(_)));
}

#[test]
fn counterexamples_to_generalization_cut_the_obligations_a_counter_needs() {
    // Counters that wrap one past half their range never show a value near the top: with counterexamples to
    // generalization taken up, the search blocks lemmas for them and needs fewer obligations than dropping literals
    // alone does, and either way proves the circuit safe with an invariant that checks.
    for width in 5..=6usize {
        let aig = counter(width, (1u64 << width) / 2 + 1, (1u64 << width) - 2);
        let with = prove_safe(&aig, &Options::default());
        let alone = prove_safe(&aig, &Options { ctgs: 0, ..Options::default() });
        for r in [&with, &alone] {
            let Verdict::Safe(inv) = &r.verdict else { panic!("width {width}: {:?}", r.verdict) };
            assert!(check_invariant(&aig, inv), "width {width}");
        }
        assert!(with.stats.ctgs > 0, "width {width}: lemmas blocked for counterexamples");
        assert_eq!(alone.stats.ctgs, 0, "width {width}");
        assert!(
            with.stats.obligations < alone.stats.obligations,
            "width {width}: {} obligations against {}",
            with.stats.obligations,
            alone.stats.obligations
        );
    }
}

#[test]
fn with_or_without_counterexamples_to_generalization_every_verdict_is_the_oracles() {
    for seed in 0..200u64 {
        let latches = 2 + (seed % 6) as usize;
        let aig = random_circuit(seed, latches, 1 + (seed % 3) as usize, 4 + (seed % 9) as usize);
        let oracle_bad = reachable_bad(&aig);
        for ctgs in [0, 1, 3] {
            match prove_safe(&aig, &Options { ctgs, ..Options::default() }).verdict {
                Verdict::Safe(inv) => assert!(!oracle_bad && check_invariant(&aig, &inv), "seed {seed}, {ctgs} ctgs"),
                Verdict::Induction(k) => assert!(!oracle_bad && check_induction(&aig, k), "seed {seed}, {ctgs} ctgs"),
                Verdict::Unsafe(trace) => assert!(oracle_bad && replays(&aig, &trace), "seed {seed}, {ctgs} ctgs"),
                Verdict::Unknown(why) => panic!("seed {seed}, {ctgs} ctgs: {why}"),
            }
        }
    }
}

#[test]
fn a_bad_state_is_lifted_to_the_latches_its_bad_output_reads_not_those_a_constraint_reads() {
    // Bad when the first latch is true; a constraint reads only the other three. The constraints hold on every state a
    // run passes through, so a lifted cube assumes them rather than keeping the latches that make them hold: the bad
    // cube is the first latch alone.
    let mut g = Aig::new();
    let x = g.add_input(None);
    let ls: Vec<usize> = (0..4).map(|_| g.add_latch(None, Init::Zero)).collect();
    for &k in &ls {
        g.set_next(k, x);
    }
    let (l1, l2, l3) = (g.latch_lit(ls[1]), g.latch_lit(ls[2]), g.latch_lit(ls[3]));
    let either = g.or(l1, l2);
    let reads = g.or(either, l3);
    g.add_constraint(reads, None);
    g.add_bad(g.latch_lit(ls[0]), None);
    let mut search = Search::new(&g);
    let state = vec![Some(true), Some(true), Some(false), Some(true)];
    let bad = search.step.bad;
    let lifted = search.lift(&state, &[false], &[bad]);
    assert_eq!(lifted, vec![Lit::pos(0)], "the first latch, and none a constraint alone reads");
}

#[test]
fn a_frame_holds_only_the_gates_in_the_cones_its_queries_read() {
    // A latch that starts false and stays false while a second latch is true, bad when the first is true, the second
    // loading a chain of ten thousand gates: blocking the bad state asks only for the first latch's next value, which
    // reads the second latch and not what it loads. Proven safe with no frame's solver and not the lifter holding the
    // chain.
    let mut g = Aig::new();
    let xs: Vec<AigLit> = (0..8).map(|_| g.add_input(None)).collect();
    let a = g.add_latch(None, Init::Zero);
    let b = g.add_latch(None, Init::Zero);
    let (la, lb) = (g.latch_lit(a), g.latch_lit(b));
    let mut chain = xs[0];
    for i in 0..10_000 {
        chain = !g.and(chain, xs[i % 8]);
    }
    g.set_next(b, chain);
    let stays = g.and(la, lb);
    g.set_next(a, stays);
    g.add_bad(la, None);
    let mut search = Search::new(&g);
    let verdict = search.resume(&Options::default(), usize::MAX).expect("a verdict");
    assert!(matches!(verdict, Verdict::Safe(_)), "{verdict:?}");
    for (i, held) in search.held.iter().enumerate() {
        let n = held.iter().filter(|&&h| h).count();
        assert!(n <= 1, "frame {i} holds {n} of the step's {} gates", held.len());
    }
    let n = search.lifter_held.iter().filter(|&&h| h).count();
    assert!(n <= 1, "the lifter holds {n} gates");
}

#[test]
fn a_lifted_cube_steps_into_its_target_whatever_the_inputs_its_model_left_open() {
    // Random circuits, every full state and partial input assignment whose step lands every completion in one cube of
    // next values: the target. The cube lifted from that state under the inputs set steps every one of its states into
    // the target under every value of the open inputs, the constraints holding.
    let mut checked = 0;
    for seed in 0..160u64 {
        let latches = 2 + (seed % 4) as usize;
        let aig = random_circuit(seed, latches, 2 + (seed % 2) as usize, 4 + (seed % 9) as usize);
        let (nl, ni) = (aig.latches().len(), aig.inputs().len());
        let mut search = Search::new(&aig);
        let completions = |partial: &[Option<bool>]| -> Vec<Vec<bool>> {
            let open: Vec<usize> = (0..ni).filter(|&i| partial[i].is_none()).collect();
            (0..1u64 << open.len())
                .map(|bits| {
                    let mut x: Vec<bool> = partial.iter().map(|v| v.unwrap_or(false)).collect();
                    for (k, &i) in open.iter().enumerate() {
                        x[i] = bits >> k & 1 == 1;
                    }
                    x
                })
                .collect()
        };
        let step = |s: &[bool], x: &[bool]| -> Option<Vec<bool>> {
            let v = aig.eval(x, s);
            aig.constraints().iter().all(|c| Aig::value(&v, c.lit)).then(|| aig.latches().iter().map(|l| Aig::value(&v, l.next)).collect())
        };
        for sbits in 0..1u64 << nl {
            let s: Vec<bool> = (0..nl).map(|j| sbits >> j & 1 == 1).collect();
            let partial: Vec<Option<bool>> = (0..ni).map(|i| (seed + sbits + i as u64) % 3 != 0).map(|set| set.then_some(sbits & 1 == 1)).collect();
            let nexts: Vec<Vec<bool>> = completions(&partial).iter().filter_map(|x| step(&s, x)).collect();
            if nexts.is_empty() {
                continue;
            }
            let fixed: Vec<usize> = (0..nl).filter(|&j| nexts.iter().all(|n| n[j] == nexts[0][j])).collect();
            if fixed.is_empty() {
                continue;
            }
            let target: Vec<Lit> = fixed.iter().map(|&j| search.step.primed(Lit::new(j as Var, nexts[0][j]))).collect();
            let state: Vec<Option<bool>> = s.iter().map(|&b| Some(b)).collect();
            let cube = search.lift(&state, &partial, &target);
            for tbits in 0..1u64 << nl {
                let t: Vec<bool> = (0..nl).map(|j| tbits >> j & 1 == 1).collect();
                if !cube.iter().all(|l| t[l.var() as usize] == l.is_positive()) {
                    continue;
                }
                for x in completions(&partial) {
                    if let Some(n) = step(&t, &x) {
                        assert!(fixed.iter().all(|&j| n[j] == nexts[0][j]), "seed {seed}: {t:?} under {x:?} leaves the target");
                    }
                }
            }
            checked += 1;
        }
    }
    assert!(checked >= 500, "{checked} lifts checked");
}

#[test]
fn over_steps_encoded_by_cuts_of_every_width_every_verdict_is_the_oracles() {
    // Random circuits up to seven latches, the search over steps whose cover has cuts of two to six leaves: every
    // verdict the explicit-state oracle's, an invariant checking and holding on every reachable state, a run replaying.
    let (mut safe, mut unsafe_) = (0, 0);
    for seed in 0..200u64 {
        let latches = 2 + (seed % 6) as usize;
        let aig = random_circuit(seed, latches, 1 + (seed % 3) as usize, 6 + (seed % 14) as usize);
        let oracle_bad = reachable_bad(&aig);
        let (cut, kept) = cone(&aig);
        for k in 2..=crate::aig_cnf::MAX_LEAVES {
            let mut search = Search::with_cuts(&cut, k);
            let found = search.resume(&Options::default(), usize::MAX).expect("a search without a budget ends");
            match whole(&aig, &kept, found) {
                Verdict::Safe(inv) => {
                    assert!(!oracle_bad, "seed {seed}, width {k}: Safe for a circuit that reaches a bad state");
                    for s in reachable_states(&aig) {
                        assert!(inv.holds(&s), "seed {seed}, width {k}: reachable state {s:?} outside the invariant");
                    }
                    safe += 1;
                }
                Verdict::Unsafe(trace) => {
                    assert!(oracle_bad, "seed {seed}, width {k}: Unsafe for a safe circuit");
                    assert!(replays(&aig, &trace), "seed {seed}, width {k}: the run replays");
                    unsafe_ += 1;
                }
                Verdict::Induction(depth) => {
                    assert!(!oracle_bad && check_induction(&aig, depth), "seed {seed}, width {k}: the induction checks");
                    safe += 1;
                }
                Verdict::Unknown(why) => panic!("seed {seed}, width {k}: a circuit of seven latches is decided ({why})"),
            }
        }
    }
    assert!(safe >= 150 && unsafe_ >= 150, "{safe} safe, {unsafe_} unsafe");
}

/// Whether `k + 1` clocks from some state, the constraints holding at each, are bad at the last and at none before: the
/// step of `k`-induction fails. By enumeration.
fn segment_to_bad(aig: &Aig, k: usize) -> bool {
    let (nl, ni) = (aig.latches().len(), aig.inputs().len());
    let bad = bad_lits(aig);
    let inputs: Vec<Vec<bool>> = (0..1u64 << ni).map(|x| (0..ni).map(|i| x >> i & 1 == 1).collect()).collect();
    let mut at: Vec<Vec<bool>> = (0..1u64 << nl).map(|s| (0..nl).map(|j| s >> j & 1 == 1).collect()).collect();
    let step = |s: &[bool], x: &[bool]| {
        let v = aig.eval(x, s);
        let ok = aig.constraints().iter().all(|c| Aig::value(&v, c.lit));
        (ok, bad.iter().any(|&b| Aig::value(&v, b)), aig.latches().iter().map(|l| Aig::value(&v, l.next)).collect::<Vec<bool>>())
    };
    for _ in 0..k {
        let mut next: Vec<Vec<bool>> = at.iter().flat_map(|s| inputs.iter().filter_map(|x| {
            let (ok, b, n) = step(s, x);
            (ok && !b).then_some(n)
        }).collect::<Vec<_>>()).collect();
        next.sort();
        next.dedup();
        at = next;
    }
    at.iter().any(|s| inputs.iter().any(|x| {
        let (ok, b, _) = step(s, x);
        ok && b
    }))
}

#[test]
fn the_induction_step_unrolled_forward_agrees_with_enumeration() {
    // One solver asked depths one to four in turn over hundreds of random circuits, free initial values and
    // constraints among them: refuted exactly when no segment of that many clean clocks ends in a bad one.
    let (mut refuted, mut met) = (0, 0);
    for seed in 0..200u64 {
        let aig = random_circuit(seed, 2 + (seed % 5) as usize, 1 + (seed % 3) as usize, 4 + (seed % 9) as usize);
        let search = Search::new(&aig);
        let mut induction = Induction::new(&search.step);
        for k in 1..=4 {
            let want = !segment_to_bad(&aig, k);
            assert_eq!(induction.ask(&search.step, k, 0), Some(want), "seed {seed}, depth {k}");
            if want {
                refuted += 1;
            } else {
                met += 1;
            }
        }
    }
    assert!(refuted >= 150 && met >= 150, "{refuted} refuted, {met} met");
}

#[test]
fn with_induction_every_verdict_is_the_oracles_and_shallow_inductive_circuits_are_proven_by_it() {
    // Random circuits up to seven latches with the induction step asked at each frame the search opens: a safe circuit
    // answered by an invariant that checks or by a depth `check_induction` accepts, an unsafe one by a run that
    // replays, never Unknown; and circuits inductive before their invariant is found answered by induction.
    let opts = Options { induction: 8, ..Options::default() };
    let (mut invariants, mut inductions, mut runs) = (0, 0, 0);
    for seed in 0..800u64 {
        let aig = random_circuit(seed, 2 + (seed % 6) as usize, 1 + (seed % 3) as usize, 4 + (seed % 9) as usize);
        let oracle_bad = reachable_bad(&aig);
        match prove_safe(&aig, &opts).verdict {
            Verdict::Safe(inv) => {
                assert!(!oracle_bad && check_invariant(&aig, &inv), "seed {seed}");
                invariants += 1;
            }
            Verdict::Induction(k) => {
                assert!(!oracle_bad && check_induction(&aig, k), "seed {seed}, depth {k}");
                inductions += 1;
            }
            Verdict::Unsafe(trace) => {
                assert!(oracle_bad && replays(&aig, &trace), "seed {seed}");
                runs += 1;
            }
            Verdict::Unknown(why) => panic!("seed {seed}: {why}"),
        }
    }
    assert!(inductions >= 30 && invariants >= 20 && runs >= 40, "{invariants} invariants, {inductions} inductions, {runs} runs");
}
