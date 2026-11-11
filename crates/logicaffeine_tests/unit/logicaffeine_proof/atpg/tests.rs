use super::*;
use crate::aig::Init;

fn xorshift(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    }
}

/// A random circuit: `inputs` inputs, `latches` latches, `gates` And/Or/Xor gates over earlier nodes, and `outputs`
/// of the last gates as outputs.
fn random_circuit(seed: u64, inputs: usize, latches: usize, gates: usize, outputs: usize) -> Aig {
    let mut next = xorshift(seed);
    let mut g = Aig::new();
    let mut pool: Vec<AigLit> = (0..inputs).map(|_| g.add_input(None)).collect();
    let ls: Vec<usize> = (0..latches).map(|_| g.add_latch(None, Init::Zero)).collect();
    pool.extend(ls.iter().map(|&k| g.latch_lit(k)));
    for _ in 0..gates {
        let a = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let b = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let c = match next() % 3 {
            0 => g.and(a, b),
            1 => g.or(a, b),
            _ => g.xor(a, b),
        };
        pool.push(c);
    }
    for &k in &ls {
        let n = pool[(next() % pool.len() as u64) as usize];
        g.set_next(k, n);
    }
    for k in 0..outputs {
        g.add_output(pool[pool.len() - 1 - k], None);
    }
    g
}

/// Whether some test detects `fault`, by trying every one.
fn testable(aig: &Aig, fault: Fault) -> bool {
    let (ni, nl) = (aig.inputs().len(), aig.latches().len());
    (0..1u64 << (ni + nl)).any(|bits| {
        let test = Test {
            inputs: (0..ni).map(|i| bits >> i & 1 == 1).collect(),
            latches: (0..nl).map(|j| bits >> (ni + j) & 1 == 1).collect(),
        };
        detects(aig, &test, fault)
    })
}

#[test]
fn every_fault_is_detected_by_its_test_or_proven_untestable_as_exhaustive_search_says() {
    let (mut detected, mut untestable, mut by_solver) = (0, 0, 0);
    for seed in 0..60u64 {
        let (inputs, latches, gates, outputs) = (3 + seed % 4, seed % 3, 10 + seed % 30, 1 + seed % 3);
        let aig = random_circuit(seed, inputs as usize, latches as usize, gates as usize, outputs as usize);
        let opts = Options { random_words: (seed % 3) as usize, ..Options::default() };
        let report = generate(&aig, &opts);
        assert!(report.check(&aig), "seed {seed}: every verdict stands");
        for (fault, status) in &report.faults {
            match status {
                Status::Detected(_) => {
                    assert!(testable(&aig, *fault), "seed {seed}: {fault:?} detected yet untestable");
                    detected += 1;
                }
                Status::Untestable(_) => {
                    assert!(!testable(&aig, *fault), "seed {seed}: {fault:?} proven untestable yet a test detects it");
                    untestable += 1;
                }
                Status::Aborted => panic!("seed {seed}: {fault:?} aborted without a budget"),
            }
        }
        by_solver += report.solves;
    }
    assert!(detected > 1000 && untestable > 100 && by_solver > 200, "{detected} detected, {untestable} untestable, {by_solver}");
}

#[test]
fn random_patterns_take_most_faults_each_test_detects_one_and_none_repeats() {
    let aig = random_circuit(7, 12, 3, 200, 6);
    let report = generate(&aig, &Options::default());
    assert!(report.check(&aig));
    let (detected, _, aborted) = report.counts();
    assert_eq!(aborted, 0);
    assert!(report.by_random * 2 > detected, "random patterns take most: {} of {detected}", report.by_random);
    for (t, test) in report.tests.iter().enumerate() {
        assert!(report.faults.iter().any(|(_, s)| *s == Status::Detected(t)), "test {t} detects a fault it is kept for");
        assert!(report.tests[..t].iter().all(|u| u != test), "test {t} repeats one before it");
    }
}

#[test]
fn a_redundant_gate_is_untestable_with_a_proof_that_checks_and_a_tampered_one_does_not() {
    // y = a ∨ (a ∧ b): the conjunction stuck at zero changes nothing.
    let mut g = Aig::new();
    let (a, b) = (g.add_input(None), g.add_input(None));
    let ab = g.and(a, b);
    let y = g.or(a, ab);
    g.add_output(y, None);
    let report = generate(&g, &Options::default());
    assert!(report.check(&g));
    let fault = Fault { node: ab.node(), stuck: false };
    let Some((_, Status::Untestable(proof))) = report.faults.iter().find(|(f, _)| *f == fault) else {
        panic!("{fault:?}: {:?}", report.faults)
    };
    let (num_vars, clauses) = miter(&g, fault);
    assert!(lrat::check(num_vars, &clauses, proof));
    let mut tampered = proof.clone();
    tampered.steps.remove(0);
    assert!(!lrat::check(num_vars, &clauses, &tampered), "a proof missing a step does not check");
    let other = Fault { node: ab.node(), stuck: true };
    assert!(!lrat::check(num_vars, &miter(&g, other).1, proof), "nor does it refute another fault's formula");
}

#[test]
fn a_budget_that_runs_out_aborts_a_fault_never_proves_it_untestable() {
    let aig = random_circuit(11, 14, 0, 300, 4);
    let tight = generate(&aig, &Options { random_words: 0, budget: 1, ..Options::default() });
    let full = generate(&aig, &Options { random_words: 0, ..Options::default() });
    assert!(tight.check(&aig) && full.check(&aig));
    let (_, untestable, aborted) = tight.counts();
    assert!(aborted > 0, "one conflict is not enough for every fault");
    assert!(untestable <= full.counts().1, "a fault the tight budget calls untestable is untestable");
    for ((f, s), (_, t)) in tight.faults.iter().zip(&full.faults) {
        if matches!(s, Status::Untestable(_)) {
            assert!(matches!(t, Status::Untestable(_)), "{f:?}");
        }
    }
}

#[test]
fn a_generation_is_the_same_every_time() {
    let aig = random_circuit(3, 10, 2, 120, 3);
    let first = generate(&aig, &Options::default());
    let again = generate(&aig, &Options::default());
    assert_eq!(first.tests, again.tests);
    assert_eq!(first.faults, again.faults);
}
