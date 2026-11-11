use super::*;
use crate::aig::{AigLit, Init};
use crate::itp::{check_induction, check_invariant, replays};
use std::collections::{HashSet, VecDeque};

/// Explicit-state reachability, the oracle: from every initial state, every input with the constraints holding steps
/// to the next state; a bad state reached under the constraints makes the circuit unsafe.
fn reachable_bad(aig: &Aig) -> bool {
    let latches = aig.latches();
    let nl = latches.len();
    let ni = aig.inputs().len();
    let bad: Vec<AigLit> = if aig.bad().is_empty() { aig.outputs() } else { aig.bad() }.iter().map(|p| p.lit).collect();
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

/// The clocks of a shortest run to a bad state under the constraints, the bad clock counted, by breadth-first search
/// over explicit states: `None` for a safe circuit.
fn shortest_run(aig: &Aig) -> Option<usize> {
    let latches = aig.latches();
    let ni = aig.inputs().len();
    let bad: Vec<AigLit> = if aig.bad().is_empty() { aig.outputs() } else { aig.bad() }.iter().map(|p| p.lit).collect();
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

/// Counters of two to four bits, every modulus and every bad value.
fn counters() -> Vec<Aig> {
    let mut designs = Vec::new();
    for width in 2..=4usize {
        for modulus in 2..=1u64 << width {
            for bad in 0..1u64 << width {
                designs.push(counter(width, modulus, bad));
            }
        }
    }
    designs
}

/// A verdict as text: its kind and its certificate.
fn verdict_text(v: &Verdict) -> String {
    match v {
        Verdict::Safe(inv) => format!("safe {} {:?}", inv.aig.to_ascii(), inv.root),
        Verdict::Induction(k) => format!("induction {k}"),
        Verdict::Unsafe(trace) => format!("unsafe {trace:?}"),
        Verdict::Unknown(why) => format!("unknown {why}"),
    }
}

#[test]
fn the_race_decides_as_explicit_state_search_does_and_answers_an_unsafe_circuit_with_a_shortest_run() {
    // Random circuits up to seven latches and every small counter: a safe circuit proven by PDR with an invariant that
    // checks; an unsafe one answered with BMC's run, as short as breadth-first search finds and replaying.
    let (mut safe, mut unsafe_) = (0, 0);
    let random = (0..300u64).map(|seed| {
        random_circuit(seed, 2 + (seed % 6) as usize, 1 + (seed % 3) as usize, 4 + (seed % 9) as usize)
    });
    for (seed, aig) in random.chain(counters()).enumerate() {
        let out = prove_safe(&aig, &Options::default());
        match &out.verdict {
            Verdict::Safe(inv) => {
                assert!(!reachable_bad(&aig), "design {seed}: Safe for a circuit that reaches a bad state");
                assert!(check_invariant(&aig, inv), "design {seed}");
                assert_eq!(out.engine, Engine::Pdr, "design {seed}");
                safe += 1;
            }
            Verdict::Induction(k) => {
                assert!(!reachable_bad(&aig), "design {seed}: inductive for a circuit that reaches a bad state");
                assert!(check_induction(&aig, *k), "design {seed}");
                assert_eq!(out.engine, Engine::Pdr, "design {seed}");
                safe += 1;
            }
            Verdict::Unsafe(trace) => {
                assert_eq!(Some(trace.inputs.len()), shortest_run(&aig), "design {seed}: a shortest run");
                assert!(replays(&aig, trace), "design {seed}");
                assert_eq!(out.engine, Engine::Bmc, "design {seed}");
                unsafe_ += 1;
            }
            Verdict::Unknown(why) => panic!("design {seed}: {why}"),
        }
    }
    assert!(safe >= 150 && unsafe_ >= 150, "{safe} safe, {unsafe_} unsafe");
}

#[test]
fn the_race_answers_the_same_whatever_its_slices() {
    // The certificate follows from the circuit alone: slices of one obligation and one clock, or of many, the same
    // verdict, the same invariant or run, the same engine.
    let sizes = [(1, 1), (3, 2), (64, 1), (1000, 7)];
    let random = (0..80u64).map(|seed| {
        random_circuit(seed, 2 + (seed % 6) as usize, 1 + (seed % 3) as usize, 4 + (seed % 9) as usize)
    });
    for (seed, aig) in random.chain(counters().into_iter().step_by(5)).enumerate() {
        let answers: Vec<(String, Engine)> = sizes
            .iter()
            .map(|&(pdr_slice, bmc_slice)| {
                let out = prove_safe(&aig, &Options { pdr_slice, bmc_slice, ..Options::default() });
                (verdict_text(&out.verdict), out.engine)
            })
            .collect();
        assert!(answers.windows(2).all(|w| w[0] == w[1]), "design {seed}: {answers:?}");
    }
}

#[test]
fn the_race_stops_without_a_verdict_once_both_engines_run_out() {
    // A mod-5 counter that never shows 6, PDR allowed one frame: PDR stops without a verdict, and BMC, bounded by the
    // same frame, finds no run.
    let aig = counter(3, 5, 6);
    let opts = Options { pdr: pdr::Options { max_frames: 1, ..pdr::Options::default() }, ..Options::default() };
    let out = prove_safe(&aig, &opts);
    assert_eq!(out.engine, Engine::None);
    assert!(matches!(out.verdict, Verdict::Unknown(_)), "{:?}", out.verdict);
}
