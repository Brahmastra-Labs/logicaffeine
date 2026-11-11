use super::*;
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

fn safe_invariant(aig: &Aig) -> Invariant {
    match prove_safe(aig, &ItpOptions::default()) {
        ItpVerdict::Safe(inv) => inv,
        other => panic!("expected Safe, got {other:?}"),
    }
}

/// A mod-5 counter never shows 6 or 7. The invariant is accepted by the independent check, holds on every reachable
/// state, and excludes both bad values.
#[test]
fn a_counter_that_wraps_early_is_proven_safe_with_a_checked_invariant() {
    for bad_value in [5u64, 6, 7] {
        let aig = counter(3, 5, bad_value);
        assert!(!reachable_bad(&aig), "the oracle agrees the counter is safe");
        let inv = safe_invariant(&aig);
        assert!(check_invariant(&aig, &inv), "the invariant passes its three obligations");
        for s in reachable_states(&aig) {
            assert!(inv.holds(&s), "bad={bad_value}: every reachable state lies in the invariant: {s:?}");
        }
        let value_bits: Vec<bool> = (0..3).map(|i| bad_value >> i & 1 == 1).collect();
        assert!(!inv.holds(&value_bits), "bad={bad_value}: the invariant excludes the bad value");
    }
}

/// A mod-8 counter reaches 5 after five enabled clocks: the verdict is a trace that replays, ending at the bad state.
#[test]
fn a_counter_that_reaches_its_bad_value_yields_a_replaying_trace() {
    let aig = counter(3, 8, 5);
    assert!(reachable_bad(&aig));
    match prove_safe(&aig, &ItpOptions::default()) {
        ItpVerdict::Unsafe(trace) => {
            assert!(replays(&aig, &trace), "the trace replays");
            assert!(trace.inputs.len() >= 6, "five increments take at least six clocks: {}", trace.inputs.len());
        }
        other => panic!("expected Unsafe, got {other:?}"),
    }
}

/// A deterministic random sequential circuit: `nl` latches with random initial values, `ni` inputs, random gates
/// over them, random next states, one bad-state property, sometimes a constraint.
fn random_circuit(seed: u64) -> Aig {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let nl = 2 + (next() % 3) as usize;
    let ni = 1 + (next() % 2) as usize;
    let mut g = Aig::new();
    let inputs: Vec<AigLit> = (0..ni).map(|_| g.add_input(None)).collect();
    let latches: Vec<usize> = (0..nl)
        .map(|_| {
            let init = match next() % 3 {
                0 => Init::Zero,
                1 => Init::One,
                _ => Init::Free,
            };
            g.add_latch(None, init)
        })
        .collect();
    let mut pool: Vec<AigLit> = inputs.clone();
    pool.extend(latches.iter().map(|&k| g.latch_lit(k)));
    for _ in 0..(3 + next() % 5) {
        let a = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let b = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let c = g.and(a, b);
        pool.push(c);
    }
    let pick = |next: &mut dyn FnMut() -> u64| pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
    for &k in &latches {
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

/// Against explicit-state reachability on hundreds of random circuits: a Safe verdict only for a safe circuit, with
/// an invariant that passes its check and holds on every reachable state; an Unsafe verdict only for an unsafe one,
/// with a trace that replays; and no Unknown at these sizes.
#[test]
fn verdicts_match_explicit_state_reachability_on_random_circuits() {
    let (mut safe, mut unsafe_) = (0, 0);
    for seed in 0..300u64 {
        let aig = random_circuit(seed);
        let oracle_bad = reachable_bad(&aig);
        match prove_safe(&aig, &ItpOptions::default()) {
            ItpVerdict::Safe(inv) => {
                assert!(!oracle_bad, "seed {seed}: Safe for a circuit that reaches a bad state");
                assert!(check_invariant(&aig, &inv), "seed {seed}: the invariant checks");
                for s in reachable_states(&aig) {
                    assert!(inv.holds(&s), "seed {seed}: reachable state {s:?} outside the invariant");
                }
                safe += 1;
            }
            ItpVerdict::Unsafe(trace) => {
                assert!(oracle_bad, "seed {seed}: Unsafe for a safe circuit");
                assert!(replays(&aig, &trace), "seed {seed}: the trace replays");
                unsafe_ += 1;
            }
            ItpVerdict::Unknown => panic!("seed {seed}: a circuit of at most four latches is decided"),
        }
    }
    assert!(safe >= 40 && unsafe_ >= 40, "both verdicts exercised: {safe} safe, {unsafe_} unsafe");
}

/// The invariant check fails closed: the empty set misses the initial state, the full set holds the bad value, and
/// a set that is not closed under a step (the counter's values 0..=2) is rejected though it holds the initial state
/// and no bad state.
#[test]
fn the_invariant_check_rejects_sets_that_are_not_safe_inductive_invariants() {
    let aig = counter(3, 5, 6);
    let set = |values: &[u64]| {
        let mut g = Aig::new();
        let bits: Vec<AigLit> = (0..3).map(|_| g.add_input(None)).collect();
        let mut root = AigLit::FALSE;
        for &v in values {
            let mut e = AigLit::TRUE;
            for (i, &b) in bits.iter().enumerate() {
                e = g.and(e, b.with_phase(v >> i & 1 == 0));
            }
            root = g.or(root, e);
        }
        Invariant { aig: g, root }
    };
    assert!(check_invariant(&aig, &set(&[0, 1, 2, 3, 4])), "the exact reachable set is an invariant");
    assert!(!check_invariant(&aig, &set(&[])), "the empty set misses the initial state");
    assert!(!check_invariant(&aig, &set(&[0, 1, 2, 3, 4, 5, 6, 7])), "the full set holds the bad value");
    assert!(!check_invariant(&aig, &set(&[0, 1, 2])), "{{0, 1, 2}} is not closed under a step");
    let mut wrong_arity = set(&[0, 1, 2, 3, 4]);
    wrong_arity.aig.add_input(None);
    assert!(!check_invariant(&aig, &wrong_arity), "an invariant over the wrong number of latches is refused");
}

/// A trace is checked, not believed: a flipped input, a wrong initial value, a dropped clock and a trace ending
/// before the bad state each fail to replay.
#[test]
fn a_trace_that_does_not_reach_a_bad_state_does_not_replay() {
    let aig = counter(3, 8, 3);
    let ItpVerdict::Unsafe(trace) = prove_safe(&aig, &ItpOptions::default()) else { panic!("the counter reaches 3") };
    assert!(replays(&aig, &trace));
    let mut flipped = trace.clone();
    for clock in flipped.inputs.iter_mut() {
        clock[0] = false;
    }
    assert!(!replays(&aig, &flipped), "with the enable held low the counter stays at 0");
    let mut wrong_init = trace.clone();
    wrong_init.initial[0] = true;
    assert!(!replays(&aig, &wrong_init), "a latch initialized to zero may not start at one");
    let mut short = trace.clone();
    short.inputs.pop();
    assert!(!replays(&aig, &short), "a trace ending a clock early is not at the bad state");
    let empty = Trace { initial: trace.initial.clone(), inputs: Vec::new() };
    assert!(!replays(&aig, &empty), "an empty trace reaches nothing");
}

/// Constraints and free initial values: a free latch `x` and an input `i`, bad when both hold. Unconstrained, the
/// bad state is reachable at once; with the constraint `¬i` it never is.
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
    let ItpVerdict::Unsafe(trace) = prove_safe(&free, &ItpOptions::default()) else { panic!("x ∧ i is reachable") };
    assert!(replays(&free, &trace));
    assert_eq!(trace.inputs.len(), 1, "reachable at the first clock");
    let constrained = build(true);
    safe_invariant(&constrained);
}

/// A circuit with no bad-state properties has its outputs read as them, and a circuit with neither is safe.
#[test]
fn outputs_stand_in_for_missing_bad_state_properties() {
    let mut g = Aig::new();
    let k = g.add_latch(None, Init::Zero);
    let x = g.latch_lit(k);
    g.set_next(k, !x);
    g.add_output(x, None);
    assert!(matches!(prove_safe(&g, &ItpOptions::default()), ItpVerdict::Unsafe(_)), "the toggle reaches 1");
    let mut quiet = Aig::new();
    let k = quiet.add_latch(None, Init::Zero);
    let x = quiet.latch_lit(k);
    quiet.set_next(k, !x);
    safe_invariant(&quiet);
}

/// An AIGER file in, a verdict out: a two-latch shift register fed a constant zero, bad when its second stage is one.
#[test]
fn a_parsed_aiger_circuit_is_decided() {
    // Latch 1 takes 0, latch 2 takes latch 1; bad = latch 2.
    let text = "aag 2 0 2 0 0 1\n2 0\n4 2\n4\n";
    let aig = Aig::parse(text.as_bytes()).expect("the AIGER text parses");
    assert!(!reachable_bad(&aig));
    let inv = safe_invariant(&aig);
    assert!(inv.holds(&[false, false]));
    assert!(!inv.holds(&[false, true]), "the second stage at one is excluded");
}

/// A trace is held to the latches' initial values: a run that is at the bad state from its first clock, but only
/// because it starts where a zero-initialised counter cannot, does not replay. A latch that starts at one may not
/// start at zero; a free latch may start at either value.
#[test]
fn a_trace_from_a_state_the_latches_cannot_start_in_does_not_replay() {
    let aig = counter(3, 8, 3);
    let at_three = Trace { initial: vec![true, true, false], inputs: vec![vec![false]] };
    assert!(!replays(&aig, &at_three), "the counter's latches start at zero, not at the bad value 3");
    let at_zero = Trace { initial: vec![false, false, false], inputs: vec![vec![false]] };
    assert!(!replays(&aig, &at_zero), "from zero with the enable low, the first clock shows 0");

    let latch = |init: Init, bad_when_set: bool| {
        let mut g = Aig::new();
        let k = g.add_latch(None, init);
        let x = g.latch_lit(k);
        g.set_next(k, x);
        g.add_bad(if bad_when_set { x } else { !x }, None);
        g
    };
    let one = latch(Init::One, false);
    assert!(!replays(&one, &Trace { initial: vec![false], inputs: vec![vec![]] }), "a latch that starts at one");
    let free = latch(Init::Free, true);
    assert!(replays(&free, &Trace { initial: vec![true], inputs: vec![vec![]] }), "a free latch may start at one");
    assert!(!replays(&free, &Trace { initial: vec![false], inputs: vec![vec![]] }), "at zero it is not bad");
}

/// The clocks of a shortest run to a bad state under the constraints, the bad clock counted, by breadth-first search
/// over explicit states: `None` for a safe circuit.
fn shortest_run(aig: &Aig) -> Option<usize> {
    let latches = aig.latches();
    let ni = aig.inputs().len();
    let bad = bad_lits(aig);
    let mut level: Vec<Vec<bool>> = vec![Vec::new()];
    for l in latches {
        let choices: &[bool] = match l.init {
            Init::Zero => &[false],
            Init::One => &[true],
            Init::Free => &[false, true],
        };
        level = level.iter().flat_map(|s| choices.iter().map(move |&c| [s.clone(), vec![c]].concat())).collect();
    }
    let mut seen: HashSet<Vec<bool>> = level.iter().cloned().collect();
    for clocks in 1.. {
        if level.is_empty() {
            return None;
        }
        let mut next_level = Vec::new();
        for s in &level {
            for bits in 0..1u64 << ni {
                let x: Vec<bool> = (0..ni).map(|i| bits >> i & 1 == 1).collect();
                let v = aig.eval(&x, s);
                if !aig.constraints().iter().all(|c| Aig::value(&v, c.lit)) {
                    continue;
                }
                if bad.iter().any(|&b| Aig::value(&v, b)) {
                    return Some(clocks);
                }
                let next: Vec<bool> = latches.iter().map(|l| Aig::value(&v, l.next)).collect();
                if seen.insert(next.clone()) {
                    next_level.push(next);
                }
            }
        }
        level = next_level;
    }
    unreachable!()
}

/// Bounded model checking against breadth-first search on hundreds of random circuits: a run exactly when one of at
/// most the bound exists, as short as the shortest, replaying.
#[test]
fn bounded_model_checking_finds_a_shortest_run_on_random_circuits() {
    let (mut runs, mut none) = (0, 0);
    for seed in 0..400u64 {
        let aig = random_circuit(seed);
        let shortest = shortest_run(&aig);
        match bmc(&aig, 20) {
            Some(trace) => {
                assert_eq!(Some(trace.inputs.len()), shortest, "seed {seed}: a shortest run");
                assert!(replays(&aig, &trace), "seed {seed}: the run replays");
                runs += 1;
            }
            None => {
                assert!(shortest.is_none_or(|c| c > 20), "seed {seed}: a run of {shortest:?} clocks missed");
                none += 1;
            }
        }
        let found = bmc(&aig, 20).is_some();
        assert_eq!(found, reachable_bad(&aig), "seed {seed}: four latches reach within twenty clocks");
    }
    assert!(runs >= 250 && none >= 60, "{runs} runs, {none} safe");
}

/// A counter enabled each clock shows 5 at its sixth clock: no run of five clocks, one of six.
#[test]
fn bounded_model_checking_goes_no_further_than_its_clocks() {
    let aig = counter(3, 8, 5);
    assert_eq!(shortest_run(&aig), Some(6));
    assert!(bmc(&aig, 5).is_none(), "no run of five clocks");
    let trace = bmc(&aig, 6).expect("a run of six clocks");
    assert_eq!(trace.inputs.len(), 6);
    assert!(replays(&aig, &trace));
    assert!(bmc(&aig, 0).is_none(), "no clocks, no run");
    assert!(bmc(&counter(3, 5, 6), 30).is_none(), "a mod-5 counter never shows 6");
}

/// A free latch `x` and an input `i`, bad when both hold: a run of one clock, unless the constraint `¬i` forbids every
/// one; a circuit with no bad state and no output has none.
#[test]
fn bounded_model_checking_honours_constraints_and_free_initial_values() {
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
    let trace = bmc(&free, 4).expect("x ∧ i at the first clock");
    assert_eq!(trace.inputs.len(), 1);
    assert!(replays(&free, &trace));
    assert!(bmc(&build(true), 10).is_none());
    let mut quiet = Aig::new();
    let k = quiet.add_latch(None, Init::Zero);
    let x = quiet.latch_lit(k);
    quiet.set_next(k, !x);
    assert!(bmc(&quiet, 10).is_none());
}

#[test]
fn bounded_model_checking_resumed_clock_by_clock_is_the_search_run_whole() {
    // Random circuits asked a clock at a time and twenty clocks at once: the same run, at the same clock.
    let mut runs = 0;
    for seed in 0..200u64 {
        let aig = random_circuit(seed);
        let whole = bmc(&aig, 20);
        let mut sliced = Bmc::new(&aig);
        let mut found = None;
        for _ in 0..20 {
            match sliced.resume(1) {
                Some(Ok(trace)) => {
                    found = Some(trace);
                    break;
                }
                Some(Err(())) => break,
                None => {}
            }
        }
        assert_eq!(found, whole, "seed {seed}");
        if let Some(trace) = &found {
            assert_eq!(sliced.clocks, trace.inputs.len(), "seed {seed}: the clock that answered");
            runs += 1;
        }
    }
    assert!(runs >= 100, "{runs} runs");
}

/// The three obligations of [`check_invariant`] decided by enumeration: every initial state some input satisfying the
/// constraints at lies in the set; every state of the set steps, under every input satisfying the constraints, into the
/// set; and no state of the set is bad under such an input.
fn invariant_by_enumeration(aig: &Aig, inv: &Invariant) -> bool {
    let (nl, ni) = (aig.latches().len(), aig.inputs().len());
    let bad = bad_lits(aig);
    for bits in 0..1u64 << nl {
        let s: Vec<bool> = (0..nl).map(|j| bits >> j & 1 == 1).collect();
        let initial = aig.latches().iter().zip(&s).all(|(l, &v)| match l.init {
            Init::Zero => !v,
            Init::One => v,
            Init::Free => true,
        });
        let inside = inv.holds(&s);
        for x in 0..1u64 << ni {
            let input: Vec<bool> = (0..ni).map(|i| x >> i & 1 == 1).collect();
            let v = aig.eval(&input, &s);
            if !aig.constraints().iter().all(|c| Aig::value(&v, c.lit)) {
                continue;
            }
            if initial && !inside {
                return false;
            }
            if inside {
                let next: Vec<bool> = aig.latches().iter().map(|l| Aig::value(&v, l.next)).collect();
                if !inv.holds(&next) || bad.iter().any(|&b| Aig::value(&v, b)) {
                    return false;
                }
            }
        }
    }
    true
}

/// A random set over `nl` latches: a conjunction of random clauses, or now and then a disjunction of two such
/// conjunctions, a root with no conjuncts to split.
fn random_set(seed: u64, nl: usize) -> Invariant {
    let mut s = seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut g = Aig::new();
    let bits: Vec<AigLit> = (0..nl).map(|_| g.add_input(None)).collect();
    let clauses = |g: &mut Aig, next: &mut dyn FnMut() -> u64| {
        let mut root = AigLit::TRUE;
        for _ in 0..next() % 6 {
            let mut c = AigLit::FALSE;
            for _ in 0..1 + next() % 3 {
                c = g.or(c, bits[(next() % nl as u64) as usize].with_phase(next() & 1 == 1));
            }
            root = g.and(root, c);
        }
        root
    };
    let root = if next() % 5 == 0 {
        let a = clauses(&mut g, &mut next);
        let b = clauses(&mut g, &mut next);
        g.or(a, b)
    } else {
        clauses(&mut g, &mut next)
    };
    Invariant { aig: g, root }
}

#[test]
fn the_invariant_check_agrees_with_enumeration_on_random_sets_over_random_circuits() {
    // Thousands of candidate sets: conjunctions of clauses, the shape a PDR invariant has and the check splits, and
    // disjunctions it cannot split. Accepted exactly when the three obligations hold state by state.
    let (mut accepted, mut rejected) = (0, 0);
    for seed in 0..600u64 {
        let aig = random_circuit(seed);
        for k in 0..8u64 {
            let inv = random_set(seed * 8 + k, aig.latches().len());
            let want = invariant_by_enumeration(&aig, &inv);
            assert_eq!(check_invariant(&aig, &inv), want, "seed {seed}, set {k}");
            if want {
                accepted += 1;
            } else {
                rejected += 1;
            }
        }
    }
    assert!(accepted >= 200 && rejected >= 1000, "{accepted} accepted, {rejected} rejected");
}

#[test]
fn the_conjuncts_of_a_root_are_what_its_uncomplemented_ands_join() {
    let mut g = Aig::new();
    let x: Vec<AigLit> = (0..4).map(|_| g.add_input(None)).collect();
    let c0 = g.or(x[0], x[1]);
    let c1 = g.or(!x[2], x[3]);
    let inner = g.and(c1, x[3]);
    let root = g.and(c0, inner);
    let mut parts = conjuncts(&g, root);
    parts.sort_by_key(|l| l.code());
    let mut want = vec![c0, c1, x[3]];
    want.sort_by_key(|l| l.code());
    assert_eq!(parts, want, "the ands joined through, the ors and the input kept whole");
    assert_eq!(conjuncts(&g, !root), vec![!root], "a complemented and is one conjunct");
    assert!(conjuncts(&g, AigLit::TRUE).is_empty(), "true has none");
    let twice = g.and(root, c0);
    assert_eq!(conjuncts(&g, twice).len(), 3, "a conjunct reached twice is one");
}

/// A random CNF over `vars` variables, then a goal variable defined as the conjunction of `parts` random literals
/// (its Tseitin clauses appended): the clauses, the goal and the parts.
fn goal_over_cnf(seed: u64, vars: u32, clauses: usize, parts: usize) -> (Vec<Vec<Lit>>, Lit, Vec<Lit>) {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut cnf: Vec<Vec<Lit>> = (0..clauses)
        .map(|k| (0..1 + k % 3).map(|_| Lit::new((next() % vars as u64) as Var, next() & 1 == 0)).collect())
        .collect();
    let goal = Lit::pos(vars as Var);
    let parts: Vec<Lit> = (0..parts).map(|_| Lit::new((next() % vars as u64) as Var, next() & 1 == 0)).collect();
    for &p in &parts {
        cnf.push(vec![goal.negated(), p]);
    }
    let mut back = vec![goal];
    back.extend(parts.iter().map(|p| p.negated()));
    cnf.push(back);
    (cnf, goal, parts)
}

/// Whether every model of `cnf` over `vars` variables sets `goal`, by enumeration.
fn implied(cnf: &[Vec<Lit>], vars: usize, goal: Lit) -> bool {
    (0..1u64 << vars).all(|bits| {
        let value = |l: &Lit| (bits >> l.var() & 1 == 1) == l.is_positive();
        !cnf.iter().all(|c| c.iter().any(value)) || value(&goal)
    })
}

#[test]
fn a_goal_is_derived_exactly_when_the_clauses_imply_it_whether_asked_whole_or_part_by_part() {
    // The whole negation refuted within the budget, asked part by part from the start (no budget), and part by part
    // after a single conflict: the same answer as enumeration every time, each answer a checked derivation.
    let (mut derived, mut not) = (0, 0);
    for seed in 0..400u64 {
        let vars = 6 + (seed % 6) as u32;
        let (cnf, goal, parts) = goal_over_cnf(seed, vars, 3 * vars as usize / 2, 1 + (seed % 4) as usize);
        let want = implied(&cnf, vars as usize + 1, goal);
        for budget in [u64::MAX, 0, 1] {
            assert_eq!(implies(vars as usize + 1, &cnf, goal, &parts, budget), want, "seed {seed}, budget {budget}");
        }
        if want {
            derived += 1;
        } else {
            not += 1;
        }
    }
    assert!(derived >= 60 && not >= 60, "{derived} derived, {not} not");
}

#[test]
fn parts_that_do_not_make_up_the_goal_cannot_carry_it() {
    // goal ⇔ a ∧ c, the clauses forcing a and b but not c: each of the parts a and b is derived, the goal is not.
    let (a, b, c, goal) = (Lit::pos(0), Lit::pos(1), Lit::pos(2), Lit::pos(3));
    let cnf = vec![vec![a], vec![b], vec![goal.negated(), a], vec![goal.negated(), c], vec![goal, a.negated(), c.negated()]];
    assert!(!implies(4, &cnf, goal, &[a, b], 0), "a and b hold, the goal needs c");
    assert!(implies(4, &cnf, a, &[a, b], 0), "a itself is derived");
}

#[test]
fn an_invariant_check_encodes_only_the_logic_its_obligations_read() {
    // A mod-5 counter beside a latch fed by a chain of a million gates over eight inputs of its own, read by nothing
    // the property needs: the counter's reachable set, over all four latches and silent on the fourth, checks in the
    // time the counter takes, not the chain. Moved into the invariant's support, the chain is read again and a set
    // that pins the fourth latch to false is refused, as it should be: the chain can load true.
    let mut aig = counter(3, 5, 6);
    let side = aig.add_latch(None, Init::Zero);
    let xs: Vec<AigLit> = (0..8).map(|_| aig.add_input(None)).collect();
    let mut chain = xs[0];
    for i in 0..1_000_000 {
        chain = !aig.and(chain, xs[i % 8]);
    }
    aig.set_next(side, chain);
    let set = |pin: bool| {
        let mut g = Aig::new();
        let bits: Vec<AigLit> = (0..4).map(|_| g.add_input(None)).collect();
        let mut root = AigLit::FALSE;
        for v in 0..5u64 {
            let mut e = AigLit::TRUE;
            for (i, &b) in bits[..3].iter().enumerate() {
                e = g.and(e, b.with_phase(v >> i & 1 == 0));
            }
            root = g.or(root, e);
        }
        if pin {
            root = g.and(root, !bits[3]);
        }
        Invariant { aig: g, root }
    };
    let started = std::time::Instant::now();
    assert!(check_invariant(&aig, &set(false)), "the counter's reachable set, the fourth latch free");
    let took = started.elapsed();
    assert!(took < std::time::Duration::from_secs(2), "a check of the counter took {took:?}");
    assert!(!check_invariant(&aig, &set(true)), "the chain loads true into the fourth latch");
}

/// The two refutations of [`check_induction`] decided by enumeration over a circuit of a few latches and inputs: no run
/// from an initial state is bad at one of its first `k` clocks, the constraints holding up to that clock; and no `k + 1`
/// clocks from any state, the constraints holding at each, are bad at the last and at none before.
fn induction_by_enumeration(aig: &Aig, k: usize) -> bool {
    let (nl, ni) = (aig.latches().len(), aig.inputs().len());
    let bad = bad_lits(aig);
    let step = |s: &[bool], x: &[bool]| -> (bool, bool, Vec<bool>) {
        let v = aig.eval(x, s);
        let ok = aig.constraints().iter().all(|c| Aig::value(&v, c.lit));
        let b = bad.iter().any(|&l| Aig::value(&v, l));
        (ok, b, aig.latches().iter().map(|l| Aig::value(&v, l.next)).collect())
    };
    let inputs: Vec<Vec<bool>> = (0..1u64 << ni).map(|x| (0..ni).map(|i| x >> i & 1 == 1).collect()).collect();
    let states: Vec<Vec<bool>> = (0..1u64 << nl).map(|s| (0..nl).map(|j| s >> j & 1 == 1).collect()).collect();
    let initial = |s: &[bool]| {
        aig.latches().iter().zip(s).all(|(l, &v)| match l.init {
            Init::Zero => !v,
            Init::One => v,
            Init::Free => true,
        })
    };
    // Runs from the initial states, clock by clock: a bad state under the constraints before clock `k` refutes the base.
    let mut frontier: Vec<Vec<bool>> = states.iter().filter(|s| initial(s)).cloned().collect();
    for _ in 0..k {
        let mut next = Vec::new();
        for s in &frontier {
            for x in &inputs {
                let (ok, b, n) = step(s, x);
                if !ok {
                    continue;
                }
                if b {
                    return false;
                }
                next.push(n);
            }
        }
        next.sort();
        next.dedup();
        frontier = next;
    }
    // Segments of k + 1 clocks from every state, the constraints holding throughout, good at the first k.
    let mut segments: Vec<Vec<bool>> = states.clone();
    for _ in 0..k {
        let mut next = Vec::new();
        for s in &segments {
            for x in &inputs {
                let (ok, b, n) = step(s, x);
                if ok && !b {
                    next.push(n);
                }
            }
        }
        next.sort();
        next.dedup();
        segments = next;
    }
    !segments.iter().any(|s| inputs.iter().any(|x| {
        let (ok, b, _) = step(s, x);
        ok && b
    }))
}

#[test]
fn the_induction_check_agrees_with_enumeration_on_random_circuits() {
    // Depths one to four over hundreds of random circuits, free initial values and constraints among them: accepted
    // exactly when no run is bad within the depth and the bad states are that deep inductive.
    let (mut accepted, mut rejected) = (0, 0);
    for seed in 0..400u64 {
        let aig = random_circuit(seed);
        for k in 1..=4usize {
            let want = induction_by_enumeration(&aig, k);
            assert_eq!(check_induction(&aig, k), want, "seed {seed}, depth {k}");
            if want {
                accepted += 1;
            } else {
                rejected += 1;
            }
        }
    }
    assert!(accepted >= 150 && rejected >= 150, "{accepted} accepted, {rejected} rejected");
    assert!(!check_induction(&random_circuit(0), 0), "no depth below one");
}

#[test]
fn a_property_inductive_only_at_depth_three_is_refused_shallower() {
    // A three-stage shift register from zero whose first stage loads false, bad when the last stage is set: from any
    // state the register is clear after three clocks, so a bad clock follows two good ones only from a set first or
    // middle stage, and three good ones never. Depths one and two are refused, three and four accepted.
    let mut g = Aig::new();
    let s: Vec<usize> = (0..3).map(|_| g.add_latch(None, Init::Zero)).collect();
    let l: Vec<AigLit> = s.iter().map(|&k| g.latch_lit(k)).collect();
    g.set_next(s[0], AigLit::FALSE);
    g.set_next(s[1], l[0]);
    g.set_next(s[2], l[1]);
    g.add_bad(l[2], None);
    for k in 1..=4 {
        assert_eq!(check_induction(&g, k), induction_by_enumeration(&g, k), "depth {k}");
    }
    assert!(!check_induction(&g, 1) && !check_induction(&g, 2) && check_induction(&g, 3) && check_induction(&g, 4));
}

/// A deterministic random sequential circuit wider than [`random_circuit`]'s: two to five latches of random initial
/// values, one to three inputs, fifteen to forty gates whose fan-ins often reach back over the pool so cuts of several
/// leaves form, one to three bad-state properties and up to two constraints.
fn random_deeper_circuit(seed: u64) -> Aig {
    let mut s = seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let nl = 2 + (next() % 4) as usize;
    let ni = 1 + (next() % 3) as usize;
    let mut g = Aig::new();
    let inputs: Vec<AigLit> = (0..ni).map(|_| g.add_input(None)).collect();
    let latches: Vec<usize> = (0..nl)
        .map(|_| {
            let init = match next() % 3 {
                0 => Init::Zero,
                1 => Init::One,
                _ => Init::Free,
            };
            g.add_latch(None, init)
        })
        .collect();
    let mut pool: Vec<AigLit> = inputs.clone();
    pool.extend(latches.iter().map(|&k| g.latch_lit(k)));
    for _ in 0..(15 + next() % 26) {
        let near = pool.len().saturating_sub(6);
        let pick = |next: &mut dyn FnMut() -> u64| {
            let from = if next() % 4 == 0 { 0 } else { near };
            pool[from + (next() % (pool.len() - from) as u64) as usize].with_phase(next() & 1 == 1)
        };
        let (a, b) = (pick(&mut next), pick(&mut next));
        let c = g.and(a, b);
        if c.node() != 0 {
            pool.push(c);
        }
    }
    let pick = |next: &mut dyn FnMut() -> u64| pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
    for &k in &latches {
        let n = pick(&mut next);
        g.set_next(k, n);
    }
    for _ in 0..(1 + next() % 3) {
        let bad = pick(&mut next);
        g.add_bad(bad, None);
    }
    for _ in 0..(next() % 3) {
        let c = pick(&mut next);
        g.add_constraint(c, None);
    }
    g
}

/// The unrolling over covers of every cut width against breadth-first search on hundreds of wider random circuits: a
/// run exactly when one of at most the bound exists, as short as the shortest, replaying, at every width alike.
#[test]
fn bounded_model_checking_finds_a_shortest_run_over_covers_of_every_cut_width() {
    let (mut runs, mut none) = (0, 0);
    for seed in 0..300u64 {
        let aig = random_deeper_circuit(seed);
        let shortest = shortest_run(&aig);
        for k in 2..=aig_cnf::MAX_LEAVES {
            match Bmc::with_cuts(&aig, k).resume(16) {
                Some(Ok(trace)) => {
                    assert_eq!(Some(trace.inputs.len()), shortest, "seed {seed}, width {k}: a shortest run");
                    assert!(replays(&aig, &trace), "seed {seed}, width {k}: the run replays");
                    runs += 1;
                }
                _ => {
                    let missed = shortest.is_some_and(|c| c <= 16);
                    assert!(!missed, "seed {seed}, width {k}: a run of {shortest:?} clocks missed");
                    none += 1;
                }
            }
        }
    }
    assert!(runs >= 600 && none >= 300, "{runs} runs, {none} without");
}

/// A latch and a gate the bad state never reads, however many clocks back, are never made: a one-latch toggle beside a
/// counter that nothing asks about.
#[test]
fn the_unrolling_makes_only_what_its_questions_read() {
    let mut g = Aig::new();
    let i = g.add_input(None);
    let t = g.add_latch(None, Init::Zero);
    let x = g.latch_lit(t);
    g.set_next(t, !x);
    let mut unread = Vec::new();
    let mut carry = i;
    for _ in 0..8 {
        let k = g.add_latch(None, Init::Zero);
        let b = g.latch_lit(k);
        let sum = g.xor(b, carry);
        carry = g.and(b, carry);
        g.set_next(k, sum);
        unread.push(b.node());
        unread.push(sum.node());
    }
    let bad = g.and(x, i);
    g.add_bad(bad, None);
    let mut bmc = Bmc::new(&g);
    let trace = match bmc.resume(10) {
        Some(Ok(trace)) => trace,
        other => panic!("x is set at the second clock: {other:?}"),
    };
    assert_eq!(trace.inputs.len(), 2);
    for c in 0..bmc.clocks {
        for &n in &unread {
            let made = bmc.slot[n as usize] != u32::MAX && bmc.at(c, n) != Signal::UNMADE;
            assert!(!made, "node {n} at clock {c} is outside every question's cone");
        }
    }
}

/// With neither inputs nor latches free to start anywhere every signal is a constant: a counter stepping each clock
/// shows its bad value at its sixth clock with no solver variable made.
#[test]
fn a_circuit_with_no_input_and_no_free_latch_unrolls_to_constants() {
    let mut g = Aig::new();
    let ls: Vec<usize> = (0..3).map(|_| g.add_latch(None, Init::Zero)).collect();
    let bits: Vec<AigLit> = ls.iter().map(|&k| g.latch_lit(k)).collect();
    let mut carry = AigLit::TRUE;
    for (&k, &b) in ls.iter().zip(&bits) {
        let sum = g.xor(b, carry);
        carry = g.and(b, carry);
        g.set_next(k, sum);
    }
    let five = g.and(bits[0], !bits[1]);
    let five = g.and(five, bits[2]);
    g.add_bad(five, None);
    let mut bmc = Bmc::new(&g);
    let trace = match bmc.resume(10) {
        Some(Ok(trace)) => trace,
        other => panic!("five shows at the sixth clock: {other:?}"),
    };
    assert_eq!(trace.inputs.len(), 6);
    assert!(replays(&g, &trace));
    assert_eq!(bmc.solver.add_vars(0), 0, "no variable");
}

/// A latch the bad state never reads keeps its initial value in the run: a latch that starts at one and never
/// changes sits beside the one the bad state reads, and the run replays only from a state where it is one.
#[test]
fn a_latch_the_unrolling_never_made_starts_the_run_at_its_initial_value() {
    let mut g = Aig::new();
    let i = g.add_input(None);
    let one = g.add_latch(None, Init::One);
    let kept = g.latch_lit(one);
    g.set_next(one, kept);
    let free = g.add_latch(None, Init::Free);
    let y = g.latch_lit(free);
    g.set_next(free, y);
    let z = g.add_latch(None, Init::Zero);
    let x = g.latch_lit(z);
    g.set_next(z, i);
    g.add_bad(x, None);
    let trace = bmc(&g, 5).expect("x follows i a clock later");
    assert_eq!(trace.inputs.len(), 2);
    assert_eq!(trace.initial, vec![true, false, false], "the one-latch at one, the free one at false, x at zero");
    assert!(replays(&g, &trace));
}

/// A latch that holds its value is one signal at every clock, and a gate over such latches one variable: a free latch
/// `x` that holds and a zero latch `y` that holds, the gate `x ∧ ¬y` read at every clock by the bad state with an
/// input.
#[test]
fn a_latch_that_holds_is_one_signal_at_every_clock() {
    let mut g = Aig::new();
    let i = g.add_input(None);
    let (fx, fy) = (g.add_latch(None, Init::Free), g.add_latch(None, Init::Free));
    let (x, y) = (g.latch_lit(fx), g.latch_lit(fy));
    g.set_next(fx, x);
    g.set_next(fy, y);
    let held = g.and(x, !y);
    let counter = g.add_latch(None, Init::Zero);
    let c = g.latch_lit(counter);
    g.set_next(counter, i);
    let bad = g.and(held, c);
    let bad = g.and(bad, !i);
    g.add_bad(bad, None);
    g.add_constraint(!y, None);
    let mut bmc = Bmc::new(&g);
    let trace = match bmc.resume(10) {
        Some(Ok(trace)) => trace,
        other => panic!("x set and i high then low: {other:?}"),
    };
    assert_eq!(trace.inputs.len(), 2);
    assert!(replays(&g, &trace));
    for t in 1..bmc.clocks {
        assert_eq!(bmc.at(t, x.node()), bmc.at(0, x.node()), "x at clock {t} is x at clock 0");
        assert_eq!(bmc.at(t, y.node()), bmc.at(0, y.node()), "y at clock {t} is y at clock 0");
    }
}

/// A latch `a` that starts false and stays false, and a latch `b` in the bad state's cone that toggles: the bad state is
/// `a` and `b` or an input. Every clock of the tied unrolling is refuted on `a`'s ties alone, and asked under no tie it
/// has a model, a run of the abstraction that frees every latch, which the circuit cannot take.
#[test]
fn a_tied_unrolling_names_the_latches_its_refutation_rests_on() {
    let mut g = Aig::new();
    let i = g.add_input(None);
    let (a, b) = (g.add_latch(None, Init::Zero), g.add_latch(None, Init::Zero));
    let (la, lb) = (g.latch_lit(a), g.latch_lit(b));
    let stays = g.and(la, i);
    g.set_next(a, stays);
    g.set_next(b, !lb);
    let either = g.or(lb, i);
    let bad = g.and(la, either);
    g.add_bad(bad, None);
    let mut bmc = Bmc::tied(&g, CUT_LEAVES);
    for t in 0..6 {
        match bmc.query(0, &[false, false]) {
            Query::Model(_, replays) => assert!(!replays, "clock {t}: the free latches' run is not the circuit's"),
            _ => panic!("clock {t}: with every latch free a bad state is reachable"),
        }
        match bmc.ask(0) {
            Answer::Refuted(used) => assert_eq!(used, vec![a], "clock {t}: the refutation rests on `a` alone"),
            _ => panic!("clock {t}: `a` stays false"),
        }
        assert_eq!(bmc.clocks, t + 1);
    }
}

/// The first clock of `trace` at which a bad state holds with the constraints held at it and every clock before, by
/// simulation: `None` when there is none.
fn first_bad_clock(aig: &Aig, trace: &Trace) -> Option<usize> {
    let bad = bad_lits(aig);
    let mut state = trace.initial.clone();
    for (t, x) in trace.inputs.iter().enumerate() {
        let v = aig.eval(x, &state);
        if !aig.constraints().iter().all(|c| Aig::value(&v, c.lit)) {
            return None;
        }
        if bad.iter().any(|&b| Aig::value(&v, b)) {
            return Some(t);
        }
        state = aig.latches().iter().map(|l| Aig::value(&v, l.next)).collect();
    }
    None
}

#[test]
fn windowed_bounded_model_checking_finds_a_run_exactly_when_one_is_within_the_bound() {
    // The wider random circuits against breadth-first search, asked two to sixteen clocks a query: a run exactly when
    // one of at most the bound exists; it replays, it is bad first at its last clock, and it ends in the first window
    // holding a run, the one the shortest run ends in.
    let (mut runs, mut none, mut longer) = (0, 0, 0);
    for seed in 0..300u64 {
        let aig = random_deeper_circuit(seed);
        let shortest = shortest_run(&aig);
        for w in [2, 3, 7, 16] {
            match bmc_windowed(&aig, 16, w) {
                Some(trace) => {
                    let s = shortest.unwrap_or_else(|| panic!("seed {seed}, window {w}: a run of a safe circuit"));
                    assert!(replays(&aig, &trace), "seed {seed}, window {w}: the run replays");
                    let len = trace.inputs.len();
                    assert_eq!(first_bad_clock(&aig, &trace), Some(len - 1), "seed {seed}, window {w}: bad first at its end");
                    let start = (s - 1) / w * w;
                    assert!(start < len && len <= (start + w).min(16), "seed {seed}, window {w}: {len} clocks, shortest {s}");
                    runs += 1;
                    longer += usize::from(len > s);
                }
                None => {
                    assert!(shortest.is_none_or(|c| c > 16), "seed {seed}, window {w}: a run of {shortest:?} clocks missed");
                    none += 1;
                }
            }
        }
    }
    assert!(runs >= 600 && none >= 300, "{runs} runs, {none} without, {longer} longer than the shortest");
}

#[test]
fn a_window_holds_each_clocks_constraints_only_up_to_its_bad_clock() {
    // `a` is true from the second clock, `b` from the third; bad is `a`, the constraint `¬b`. The only runs are bad at
    // the second clock, and none reaches a third: a window of eight clocks finds the run of two however many of its
    // clocks no run reaches.
    let mut g = Aig::new();
    let a = g.add_latch(None, Init::Zero);
    let b = g.add_latch(None, Init::Zero);
    g.set_next(a, AigLit::TRUE);
    let la = g.latch_lit(a);
    g.set_next(b, la);
    g.add_bad(la, None);
    let lb = g.latch_lit(b);
    g.add_constraint(!lb, None);
    assert_eq!(shortest_run(&g), Some(2));
    let trace = bmc_windowed(&g, 8, 8).expect("the run bad at the second clock");
    assert_eq!(trace.inputs.len(), 2);
    assert!(replays(&g, &trace));
}

#[test]
fn windows_left_to_their_budget_and_clocks_asked_one_at_a_time_after_them_keep_the_search_whole() {
    // Each random circuit asked windows within one conflict a query, the clocks a window leaves to its budget then asked
    // one at a time, and so on to the bound: a run exactly when one of at most the bound exists, replaying.
    let (mut runs, mut budgets) = (0, 0);
    for seed in 0..300u64 {
        let aig = random_deeper_circuit(seed);
        let shortest = shortest_run(&aig);
        let mut bmc = Bmc::new(&aig);
        let mut found = None;
        while bmc.clocks < 16 && found.is_none() {
            match bmc.window(5.min(16 - bmc.clocks), 1) {
                Answer::Run(trace) => found = Some(trace),
                Answer::Ended => break,
                Answer::Refuted(_) => {}
                Answer::Budget => {
                    budgets += 1;
                    match bmc.resume(1) {
                        Some(Ok(trace)) => found = Some(trace),
                        Some(Err(())) => break,
                        None => {}
                    }
                }
            }
        }
        match found {
            Some(trace) => {
                assert!(shortest.is_some_and(|s| s <= trace.inputs.len()), "seed {seed}: a run no shorter than the shortest");
                assert!(replays(&aig, &trace), "seed {seed}: the run replays");
                runs += 1;
            }
            None => assert!(shortest.is_none_or(|c| c > 16), "seed {seed}: a run of {shortest:?} clocks missed"),
        }
    }
    assert!(runs >= 150 && budgets >= 30, "{runs} runs, {budgets} windows left to the budget");
}

#[test]
fn interleaved_bounded_model_checking_finds_a_run_exactly_when_one_is_within_the_bound() {
    // The wider random circuits again, the clock-by-clock and the windowed search taken in turns: a run exactly when one
    // of at most the bound exists, replaying, bad first at its last clock, within the bound; and the same answer asked
    // twice.
    let mut runs = 0;
    for seed in 0..300u64 {
        let aig = random_deeper_circuit(seed);
        let shortest = shortest_run(&aig);
        for bound in [9, 16] {
            let w = bound;
            let found = bmc_interleaved(&aig, bound);
            assert_eq!(found, bmc_interleaved(&aig, bound), "seed {seed}, bound {bound}: the same answer");
            match found {
                Some(trace) => {
                    assert!(shortest.is_some_and(|s| s <= trace.inputs.len()), "seed {seed}, window {w}: a run of a safe circuit");
                    assert!(replays(&aig, &trace) && trace.inputs.len() <= bound, "seed {seed}, window {w}: the run replays");
                    assert_eq!(first_bad_clock(&aig, &trace), Some(trace.inputs.len() - 1), "seed {seed}, window {w}");
                    runs += 1;
                }
                None => assert!(shortest.is_none_or(|c| c > bound), "seed {seed}, window {w}: a run of {shortest:?} clocks missed"),
            }
        }
    }
    assert!(runs >= 300, "{runs} runs");
}

/// A three-bit counter `k` from 0, a latch `a` set for good once the input `x` is, the constraint that `x` is true
/// only when `k` is 4, and the bad state `a` with `k` at least 2: the one run sets `x` at clock 4 and is bad at clock 5.
fn a_counter_whose_input_waits_for_four() -> Aig {
    let mut g = Aig::new();
    let x = g.add_input(None);
    let k: Vec<usize> = (0..3).map(|_| g.add_latch(None, Init::Zero)).collect();
    let a = g.add_latch(None, Init::Zero);
    let (k0, k1, k2, la) = (g.latch_lit(k[0]), g.latch_lit(k[1]), g.latch_lit(k[2]), g.latch_lit(a));
    let carry = g.and(k1, k0);
    let n1 = g.xor(k1, k0);
    let n2 = g.xor(k2, carry);
    g.set_next(k[0], !k0);
    g.set_next(k[1], n1);
    g.set_next(k[2], n2);
    let set = g.or(la, x);
    g.set_next(a, set);
    let low = g.and(!k1, !k0);
    let four = g.and(k2, low);
    let allowed = g.or(!x, four);
    g.add_constraint(allowed, None);
    let past_one = g.or(k1, k2);
    let bad = g.and(la, past_one);
    g.add_bad(bad, None);
    g
}

#[test]
fn a_refuted_window_holds_its_clocks_constraints_for_every_window_after_it() {
    // Asked two or three clocks a window, the first window holds no run; the next one is bad at its first clock only
    // under an `x` set at clock 0 or 1, which the first window's constraints rule out. Kept as the window is refuted,
    // they leave the run of six clocks, bad first at clock 5.
    let g = a_counter_whose_input_waits_for_four();
    assert_eq!(shortest_run(&g), Some(6), "the one run sets x at clock 4");
    for w in [2, 3] {
        let run = bmc_windowed(&g, 16, w).unwrap_or_else(|| panic!("window {w}: the run of six clocks"));
        assert!(replays(&g, &run), "window {w}: the run replays");
        assert_eq!(run.inputs.len(), 6, "window {w}: the run of six clocks");
        assert_eq!(first_bad_clock(&g, &run), Some(5), "window {w}: bad first at clock 5");
    }
}

/// A five-bit counter from 0, bad when it reaches 16: its one run is bad first at its seventeenth clock.
fn a_counter_bad_at_sixteen() -> Aig {
    let mut g = Aig::new();
    let k: Vec<usize> = (0..5).map(|_| g.add_latch(None, Init::Zero)).collect();
    let bits: Vec<AigLit> = k.iter().map(|&j| g.latch_lit(j)).collect();
    let mut carry = AigLit::TRUE;
    for (&j, &b) in k.iter().zip(&bits) {
        let next = g.xor(b, carry);
        g.set_next(j, next);
        carry = g.and(carry, b);
    }
    let mut sixteen = bits[4];
    for &b in &bits[..4] {
        sixteen = g.and(sixteen, !b);
    }
    g.add_bad(sixteen, None);
    g
}

#[test]
fn a_window_never_asks_past_the_bound() {
    // The run is seventeen clocks long. Within sixteen, no window of any width finds it, the last window cut at the
    // bound; within seventeen, every width finds that run; and so do the clock-by-clock and the interleaved searches.
    let g = a_counter_bad_at_sixteen();
    assert_eq!(shortest_run(&g), Some(17), "bad first at the seventeenth clock");
    for w in [2, 3, 5, 7, 16, 64] {
        assert_eq!(bmc_windowed(&g, 16, w), None, "window {w}: no run within sixteen clocks");
        let run = bmc_windowed(&g, 17, w).unwrap_or_else(|| panic!("window {w}: the run of seventeen clocks"));
        assert!(replays(&g, &run) && run.inputs.len() == 17, "window {w}: the run of seventeen clocks");
    }
    assert_eq!(bmc(&g, 16), None);
    assert_eq!(bmc(&g, 17).map(|run| run.inputs.len()), Some(17));
    assert_eq!(bmc_interleaved(&g, 16), None);
    assert_eq!(bmc_interleaved(&g, 17).map(|run| run.inputs.len()), Some(17));
}
