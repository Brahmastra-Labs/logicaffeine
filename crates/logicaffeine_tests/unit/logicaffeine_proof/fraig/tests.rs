use super::*;
use crate::aig::Aig;
use crate::sweep;

fn named(g: &mut Aig, name: &str) -> AigLit {
    g.add_input(Some(name.to_string()))
}

fn word(g: &mut Aig, name: &str, width: usize) -> Vec<AigLit> {
    (0..width).map(|i| named(g, &format!("{name}[{i}]"))).collect()
}

fn output_word(g: &mut Aig, name: &str, bits: &[AigLit]) {
    for (i, &b) in bits.iter().enumerate() {
        g.add_output(b, Some(format!("{name}[{i}]")));
    }
}

/// A ripple-carry adder.
fn ripple(g: &mut Aig, a: &[AigLit], b: &[AigLit]) -> Vec<AigLit> {
    let mut carry = AigLit::FALSE;
    let mut sum = Vec::new();
    for i in 0..a.len() {
        let p = g.xor(a[i], b[i]);
        sum.push(g.xor(p, carry));
        let gen = g.and(a[i], b[i]);
        let prop = g.and(p, carry);
        carry = g.or(gen, prop);
    }
    sum
}

/// A Kogge–Stone adder.
fn kogge_stone(g: &mut Aig, a: &[AigLit], b: &[AigLit]) -> Vec<AigLit> {
    let n = a.len();
    let p: Vec<AigLit> = (0..n).map(|i| g.xor(a[i], b[i])).collect();
    let mut gg: Vec<AigLit> = (0..n).map(|i| g.and(a[i], b[i])).collect();
    let mut pp = p.clone();
    let mut d = 1;
    while d < n {
        let (prev_g, prev_p) = (gg.clone(), pp.clone());
        for i in d..n {
            let t = g.and(prev_p[i], prev_g[i - d]);
            gg[i] = g.or(prev_g[i], t);
            pp[i] = g.and(prev_p[i], prev_p[i - d]);
        }
        d *= 2;
    }
    (0..n).map(|i| if i == 0 { p[0] } else { g.xor(p[i], gg[i - 1]) }).collect()
}

fn adders(width: usize) -> (Aig, Aig) {
    let mut l = Aig::new();
    let (a, b) = (word(&mut l, "a", width), word(&mut l, "b", width));
    let s = ripple(&mut l, &a, &b);
    output_word(&mut l, "s", &s);
    let mut r = Aig::new();
    let (b2, a2) = (word(&mut r, "b", width), word(&mut r, "a", width));
    let s = kogge_stone(&mut r, &a2, &b2);
    output_word(&mut r, "s", &s);
    (l, r)
}

/// An array multiplier of two `width`-bit words, its rows summed by ripple adders.
fn multiplier(g: &mut Aig, a: &[AigLit], b: &[AigLit]) -> Vec<AigLit> {
    let width = a.len();
    let mut acc: Vec<AigLit> = vec![AigLit::FALSE; width];
    for i in 0..width {
        let row: Vec<AigLit> = (0..width).map(|j| if j < i { AigLit::FALSE } else { g.and(a[j - i], b[i]) }).collect();
        acc = ripple(g, &acc, &row);
    }
    acc
}

#[test]
fn a_ripple_adder_equals_a_kogge_stone_adder_and_the_derivation_checks() {
    for width in [4, 8, 16, 32] {
        let (l, r) = adders(width);
        let report = equivalent(&l, &r, &Options::default()).unwrap();
        assert!(report.all_equal(), "{width}: {:?}", report.verdicts);
        assert_eq!(report.verdicts.len(), width);
        assert!(report.check(), "{width}: the derivation proves every output pair");
    }
}

#[test]
fn a_circuit_against_itself_needs_no_solver_and_no_proof_step() {
    // The miter hashes both sides' identical structure into one node each: every pair is one literal.
    let (l, _) = adders(16);
    let report = equivalent(&l, &l.clone(), &Options::default()).unwrap();
    assert!(report.all_equal());
    assert!(report.check());
    assert_eq!(report.stats.queries, 0, "{:?}", report.stats);
    assert!(report.proof.steps.is_empty(), "a pair that is one literal claims nothing");
}

/// `a ⊕ b` two ways: as (a ∧ ¬b) ∨ (¬a ∧ b) on the left, ¬((a ∧ b) ∨ (¬a ∧ ¬b)) on the right.
fn xor_two_ways(g: &mut Aig, a: AigLit, b: AigLit, left: bool) -> AigLit {
    if left {
        let (x, y) = (g.and(a, !b), g.and(!a, b));
        g.or(x, y)
    } else {
        let (x, y) = (g.and(a, b), g.and(!a, !b));
        !g.or(x, y)
    }
}

#[test]
fn a_node_whose_fan_ins_were_proven_equal_merges_by_structure() {
    // Each XOR spelled two ways needs the solver; the AND chain over them is the same on both sides once the XORs
    // are merged, so every link of it merges with no solver call.
    let width = 12;
    let side = |left: bool| {
        let mut g = Aig::new();
        let (a, b) = (word(&mut g, "a", width), word(&mut g, "b", width));
        let x: Vec<AigLit> = (0..width).map(|i| xor_two_ways(&mut g, a[i], b[i], left)).collect();
        let mut chain = x[0];
        let mut outs = Vec::new();
        for &xi in &x[1..] {
            chain = g.and(chain, xi);
            outs.push(chain);
        }
        output_word(&mut g, "c", &outs);
        g
    };
    let report = equivalent(&side(true), &side(false), &Options::default()).unwrap();
    assert!(report.all_equal(), "{:?}", report.verdicts);
    assert!(report.check());
    assert!(report.stats.merges >= width, "each XOR merged by the solver: {:?}", report.stats);
    assert!(report.stats.structural >= width - 1, "every link of the chain by structure: {:?}", report.stats);
}

#[test]
fn different_functions_are_told_apart_by_a_witness_that_replays() {
    let mut l = Aig::new();
    let (a, b) = (named(&mut l, "a"), named(&mut l, "b"));
    let y = l.and(a, b);
    l.add_output(y, Some("y".into()));
    let mut r = Aig::new();
    let (a2, b2) = (named(&mut r, "a"), named(&mut r, "b"));
    let y = r.or(a2, b2);
    r.add_output(y, Some("y".into()));
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    let Verdict::Differ { inputs, latches } = &report.verdicts[0].1 else { panic!("{:?}", report.verdicts) };
    let v = report.miter.aig.eval(inputs, latches);
    let (_, x, z) = &report.miter.pairs[0];
    assert_ne!(Aig::value(&v, *x), Aig::value(&v, *z), "the witness separates the pair");
    assert!(report.check(), "a difference's witness replays and no equality is claimed for it");
}

#[test]
fn a_wrong_carry_is_found_at_the_bit_it_breaks_and_the_rest_are_proven() {
    let width = 8;
    let (l, _) = adders(width);
    let mut r = Aig::new();
    let (a, b) = (word(&mut r, "a", width), word(&mut r, "b", width));
    let mut s = ripple(&mut r, &a, &b);
    s[5] = r.xor(a[5], b[5]);
    output_word(&mut r, "s", &s);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    for (name, v) in &report.verdicts {
        let broken = name == "s[5]";
        assert_eq!(matches!(v, Verdict::Differ { .. }), broken, "{name}: {v:?}");
        assert_eq!(*v == Verdict::Equal, !broken, "{name}");
    }
    assert!(report.check());
}

#[test]
fn a_constant_output_is_proven_constant() {
    let mut l = Aig::new();
    let a = named(&mut l, "a");
    let na = l.and(a, !a);
    l.add_output(na, Some("y".into()));
    let mut r = Aig::new();
    let (a2, b2) = (named(&mut r, "a"), named(&mut r, "b"));
    let ab = r.and(a2, b2);
    let anb = r.and(a2, !b2);
    let both = r.and(ab, anb);
    r.add_output(both, Some("y".into()));
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert!(report.all_equal(), "{:?}", report.verdicts);
    assert!(report.check());
}

#[test]
fn a_budget_that_runs_out_leaves_the_pair_unknown_never_equal() {
    let width = 8;
    let mut l = Aig::new();
    let (a, b) = (word(&mut l, "a", width), word(&mut l, "b", width));
    let p = multiplier(&mut l, &a, &b);
    output_word(&mut l, "p", &p);
    let mut r = Aig::new();
    let (a2, b2) = (word(&mut r, "a", width), word(&mut r, "b", width));
    let p = multiplier(&mut r, &b2, &a2);
    output_word(&mut r, "p", &p);
    let opts = Options { budget: 1, final_budget: 1, rounds: 1, ..Options::default() };
    let report = equivalent(&l, &r, &opts).unwrap();
    assert!(report.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Unknown(_))), "{:?}", report.verdicts);
    assert!(report.verdicts.iter().all(|(_, v)| !matches!(v, Verdict::Differ { .. })));
    assert!(report.check(), "what it did prove still checks");
    let full = equivalent(&l, &r, &Options::default()).unwrap();
    assert!(full.all_equal(), "a commuted multiplier is the multiplier");
    assert!(full.check());
}

#[test]
fn a_tampered_derivation_or_a_claim_on_a_difference_does_not_check() {
    let (l, r) = adders(8);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert!(report.check());
    let mut cut = report.clone();
    cut.proof.steps.truncate(cut.proof.steps.len() / 2);
    assert!(!cut.check(), "half a derivation");
    let mut short = report.clone();
    let last = short.proof.steps.last_mut().unwrap();
    last.hints.pop();
    assert!(!short.check(), "a step whose hints stop short of a conflict");
    let mut lie = report.clone();
    lie.verdicts[0].1 = Verdict::Differ { inputs: vec![false; lie.miter.aig.inputs().len()], latches: Vec::new() };
    assert!(!lie.check(), "a difference whose witness does not separate the pair");
}

/// A random circuit over `inputs` named inputs and `gates` gates, `outputs` outputs named y0..: seeded.
fn random_circuit(seed: u64, inputs: usize, gates: usize, outputs: usize) -> Aig {
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut g = Aig::new();
    let mut lits: Vec<AigLit> = (0..inputs).map(|i| named(&mut g, &format!("x{i}"))).collect();
    for _ in 0..gates {
        let a = lits[(next() % lits.len() as u64) as usize].with_phase(next() & 1 == 1);
        let b = lits[(next() % lits.len() as u64) as usize].with_phase(next() & 1 == 1);
        let c = match next() % 3 {
            0 => g.and(a, b),
            1 => g.or(a, b),
            _ => g.xor(a, b),
        };
        lits.push(c);
    }
    for k in 0..outputs {
        let o = lits[lits.len() - 1 - k];
        g.add_output(o, Some(format!("y{k}")));
    }
    g
}

#[test]
fn every_verdict_agrees_with_the_sweeping_engine_on_random_pairs() {
    // Pairs of random circuits over the same inputs, and each against a re-derived copy of itself through an XOR
    // with zero, so some pairs are equal by structure, some by SAT and some differ.
    for seed in 0..40u64 {
        let l = random_circuit(seed * 2 + 1, 6, 30, 4);
        let r = if seed % 2 == 0 { random_circuit(seed * 2 + 2, 6, 30, 4) } else { l.clone() };
        let ours = equivalent(&l, &r, &Options::default()).unwrap();
        let theirs = sweep::equivalent(&l, &r, &sweep::Options::default()).unwrap();
        for ((name, a), (_, b)) in ours.verdicts.iter().zip(&theirs.verdicts) {
            assert_eq!(*a == Verdict::Equal, *b == Verdict::Equal, "seed {seed}, {name}: {a:?} vs {b:?}");
        }
        assert!(ours.check(), "seed {seed}");
    }
}

/// A commuted 6-bit multiplier: equal, and work for the solver.
fn commuted_multipliers(width: usize) -> (Aig, Aig) {
    let mut l = Aig::new();
    let (a, b) = (word(&mut l, "a", width), word(&mut l, "b", width));
    let p = multiplier(&mut l, &a, &b);
    output_word(&mut l, "p", &p);
    let mut r = Aig::new();
    let (a2, b2) = (word(&mut r, "a", width), word(&mut r, "b", width));
    let p = multiplier(&mut r, &b2, &a2);
    output_word(&mut r, "p", &p);
    (l, r)
}

#[test]
fn solvers_replaced_every_few_variables_still_derive_every_equality_and_drop_what_no_merge_used() {
    let (l, r) = commuted_multipliers(6);
    for recycle in [4, 16, 64] {
        let report = equivalent(&l, &r, &Options { recycle, ..Options::default() }).unwrap();
        let s = &report.stats;
        assert!(report.all_equal(), "recycle {recycle}: {:?}", report.verdicts);
        assert!(report.check(), "recycle {recycle}: the derivation checks across every solver: {s:?}");
        assert!(s.solvers > 1, "recycle {recycle}: {s:?}");
        assert!(s.forgotten > 0, "recycle {recycle}: a replaced solver's unused learned clauses go with it: {s:?}");
        assert!(s.forgotten <= s.learned && s.proof <= s.log, "{s:?}");
    }
}

#[test]
fn every_step_of_the_derivation_is_a_claim_or_named_by_a_later_step() {
    let (adder_l, adder_r) = adders(8);
    let (mul_l, mul_r) = commuted_multipliers(5);
    for (l, r) in [(adder_l, adder_r), (mul_l, mul_r)] {
        let report = equivalent(&l, &r, &Options { recycle: 32, ..Options::default() }).unwrap();
        assert!(report.all_equal() && report.check());
        let key = |c: &[Lit]| {
            let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive())).collect();
            k.sort_unstable();
            k
        };
        let claims: HashSet<Vec<(u32, bool)>> = report
            .miter
            .pairs
            .iter()
            .filter(|(_, a, b)| a != b)
            .flat_map(|(_, a, b)| claim(*a, *b))
            .map(|c| key(&c))
            .collect();
        let named: HashSet<u64> = report.proof.steps.iter().flat_map(|s| s.hints.iter().copied()).collect();
        assert!(!report.proof.steps.is_empty());
        for s in &report.proof.steps {
            assert!(
                named.contains(&s.id) || claims.contains(&key(&s.clause)),
                "step {} ({:?}) is neither claimed nor used",
                s.id,
                s.clause
            );
            assert!(s.hints.iter().all(|&h| h < s.id), "step {} names a clause after it: {:?}", s.id, s.hints);
        }
    }
}

#[test]
fn every_verdict_agrees_with_the_sweeping_engine_on_solvers_replaced_every_few_variables() {
    for seed in 0..40u64 {
        let l = random_circuit(seed * 2 + 1, 6, 30, 4);
        let r = if seed % 2 == 0 { random_circuit(seed * 2 + 2, 6, 30, 4) } else { l.clone() };
        let ours = equivalent(&l, &r, &Options { recycle: 3, ..Options::default() }).unwrap();
        let theirs = sweep::equivalent(&l, &r, &sweep::Options::default()).unwrap();
        for ((name, a), (_, b)) in ours.verdicts.iter().zip(&theirs.verdicts) {
            assert_eq!(*a == Verdict::Equal, *b == Verdict::Equal, "seed {seed}, {name}: {a:?} vs {b:?}");
        }
        assert!(ours.check(), "seed {seed}: {:?}", ours.stats);
    }
}

#[test]
fn a_log_collected_every_few_clauses_lets_go_of_what_nothing_can_name_and_still_derives_every_equality() {
    // Seven bits: enough conflicts that the solver reduces its learned clauses, so some leave the database and
    // nothing can name them again.
    let (l, r) = commuted_multipliers(7);
    let whole = equivalent(&l, &r, &Options { recycle: usize::MAX, collect: usize::MAX, ..Options::default() }).unwrap();
    assert!(whole.all_equal() && whole.check());
    assert_eq!((whole.stats.solvers, whole.stats.forgotten), (1, 0), "one solver, never collected: {:?}", whole.stats);
    for collect in [1, 8, 64] {
        let report = equivalent(&l, &r, &Options { recycle: usize::MAX, collect, ..Options::default() }).unwrap();
        let s = &report.stats;
        assert!(report.all_equal(), "collect {collect}: {:?}", report.verdicts);
        assert!(report.check(), "collect {collect}: the derivation checks with the log collected as it went: {s:?}");
        assert_eq!(s.solvers, 1, "{s:?}");
        assert!(s.forgotten > 0 && s.log < whole.stats.log, "collect {collect}: the log let go of some: {s:?} against {:?}", whole.stats);
        assert_eq!(s.learned, whole.stats.learned, "collecting changes what is kept, never the search");
        assert_eq!(report.proof, whole.proof, "nor the derivation the claims reach");
    }
}

#[test]
fn the_hints_the_log_holds_are_counted_as_they_come_and_go() {
    let (l, r) = commuted_multipliers(6);
    let m = miter(&l, &r).unwrap();
    let pairs: Vec<(AigLit, AigLit)> = m.pairs.iter().map(|(_, a, b)| (*a, *b)).collect();
    for (recycle, collect) in [(64, 4), (usize::MAX, 16), (usize::MAX, usize::MAX)] {
        let opts = Options { recycle, collect, ..Options::default() };
        let mut e = Engine::new(&m.aig, &pairs, &opts);
        e.sweep(&opts, opts.budget);
        let mut claims = Vec::new();
        for &(a, b) in &pairs {
            assert_eq!(e.decide_pair(a, b, 0, &mut claims), Verdict::Equal);
        }
        e.sync();
        let held: usize = e.log.iter().map(|s| s.hints.len()).sum();
        assert_eq!(e.stats.hints, held, "recycle {recycle}, collect {collect}: the count is the log's");
        assert!(e.log.windows(2).all(|w| w[0].id < w[1].id), "the log stays in id order");
        for (k, step) in e.log.iter().enumerate() {
            assert_eq!(e.place(step.id), Some(k), "each step found where it is");
        }
        let held: HashSet<u64> = e.log.iter().map(|s| s.id).collect();
        let first = e.formula.len() as u64 + 1;
        assert!((first..e.next_id).filter(|id| !held.contains(id)).all(|id| e.place(id).is_none()), "nothing let go is found");
        let learned_kept = e.log.iter().filter(|s| s.learned).count();
        assert_eq!(learned_kept + e.stats.forgotten, e.stats.learned, "every learned clause kept or let go, once");
    }
}

#[test]
fn rounds_at_ten_times_the_budget_prove_what_one_round_at_a_tiny_budget_leaves_open() {
    let (l, r) = commuted_multipliers(7);
    let one = equivalent(&l, &r, &Options { budget: 1, final_budget: 1, rounds: 1, ..Options::default() }).unwrap();
    assert!(one.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Unknown(_))), "one round at a conflict leaves some open");
    assert!(one.check());
    assert_eq!(one.stats.rounds, 1);
    let more = equivalent(&l, &r, &Options { budget: 1, final_budget: 1, rounds: 4, ..Options::default() }).unwrap();
    assert!(more.all_equal(), "{:?}", more.verdicts);
    assert!(more.check(), "the last round's derivation proves every pair: {:?}", more.stats);
    assert!(more.stats.rounds > 1, "{:?}", more.stats);
}

#[test]
fn a_round_that_decides_every_pair_is_the_last() {
    let (l, r) = adders(16);
    let report = equivalent(&l, &r, &Options { rounds: 5, ..Options::default() }).unwrap();
    assert!(report.all_equal() && report.check());
    assert_eq!(report.stats.rounds, 1, "the first round decided everything: {:?}", report.stats);
}

#[test]
fn a_difference_found_in_the_first_round_ends_the_decision_there() {
    let width = 8;
    let (l, _) = adders(width);
    let mut r = Aig::new();
    let (a, b) = (word(&mut r, "a", width), word(&mut r, "b", width));
    let mut s = ripple(&mut r, &a, &b);
    s[5] = r.xor(a[5], b[5]);
    output_word(&mut r, "s", &s);
    let report = equivalent(&l, &r, &Options { rounds: 3, ..Options::default() }).unwrap();
    assert_eq!(report.stats.rounds, 1, "a difference is a verdict, not a pair left open");
    assert!(report.check());
}

/// Two adders' sums paired bit for bit in one circuit, a multiplier beside them that no pair reads, built after them.
fn adders_beside_a_multiplier(width: usize, beside: bool) -> (Aig, Vec<(AigLit, AigLit)>) {
    let mut g = Aig::new();
    let (a, b) = (word(&mut g, "a", width), word(&mut g, "b", width));
    let (s, t) = (ripple(&mut g, &a, &b), kogge_stone(&mut g, &a, &b));
    if beside {
        let (c, d) = (word(&mut g, "c", width), word(&mut g, "d", width));
        let p = multiplier(&mut g, &c, &d);
        output_word(&mut g, "p", &p);
    }
    (g, s.into_iter().zip(t).collect())
}

#[test]
fn a_node_no_pair_reaches_is_never_swept_and_costs_the_decision_nothing() {
    let width = 12;
    let (g, pairs) = adders_beside_a_multiplier(width, true);
    let (alone, alone_pairs) = adders_beside_a_multiplier(width, false);
    let reached: HashSet<u32> = g.cone(&pairs.iter().flat_map(|&(l, r)| [l, r]).collect::<Vec<_>>()).into_iter().collect();
    assert!(reached.len() * 2 < g.num_nodes(), "the multiplier is most of the circuit");
    let (verdicts, proof, stats) = decide(&g, &pairs, &Options::default());
    assert!(verdicts.iter().all(|v| *v == Verdict::Equal), "{verdicts:?}");
    let claims: Vec<Vec<Lit>> = pairs.iter().filter(|(l, r)| l != r).flat_map(|&(l, r)| claim(l, r)).collect();
    assert!(lrat::check_derivation(g.num_nodes(), &formula(&g), &proof, &claims), "the derivation proves every pair");
    assert_eq!((stats.swept, stats.cone, stats.nodes), (reached.len(), reached.len(), g.num_nodes()), "the sweep passes the nodes the pairs reach and no other");
    let (_, _, by_itself) = decide(&alone, &alone_pairs, &Options::default());
    let cost = |s: &Stats| (s.swept, s.structural, s.queries, s.merges, s.counterexamples, s.unknown, s.conflicts);
    assert_eq!(cost(&stats), cost(&by_itself), "the decision is the one of the pairs' circuit alone");
}

#[test]
fn the_simulation_holds_the_nodes_the_pairs_reach_and_no_other() {
    let (g, pairs) = adders_beside_a_multiplier(12, true);
    let mut e = Engine::new(&g, &pairs, &Options::default());
    let cone = e.stats.cone;
    assert!(cone * 2 < g.num_nodes());
    assert_eq!(e.sim.values.len(), cone * e.sim.words, "a value per word for each node the pairs reach");
    e.sweep(&Options::default(), 30);
    let inputs = vec![true; g.inputs().len()];
    e.sim.add(&g, &inputs, &[]);
    assert_eq!(e.sim.values.len(), cone * e.sim.words, "a counterexample simulates the same nodes again");
    let (lhs, rhs) = pairs[3];
    let both = e.sim.signature(lhs.node()).0 == e.sim.signature(rhs.node()).0;
    assert!(both, "two equal sums simulate alike");
}

fn xorshift(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    }
}

/// A random circuit over `inputs` inputs and `latches` latches: each gate an And, Or or Xor of two earlier nodes.
fn random_sequential(seed: u64, inputs: usize, latches: usize, gates: usize) -> Aig {
    let mut next = xorshift(seed);
    let mut g = Aig::new();
    let mut lits: Vec<AigLit> = (0..inputs).map(|i| named(&mut g, &format!("x{i}"))).collect();
    for k in 0..latches {
        let l = g.add_latch(Some(format!("q{k}")), crate::aig::Init::Zero);
        lits.push(g.latch_lit(l));
    }
    for _ in 0..gates {
        let a = lits[(next() % lits.len() as u64) as usize].with_phase(next() & 1 == 1);
        let b = lits[(next() % lits.len() as u64) as usize].with_phase(next() & 1 == 1);
        let c = match next() % 3 {
            0 => g.and(a, b),
            1 => g.or(a, b),
            _ => g.xor(a, b),
        };
        lits.push(c);
    }
    g
}

/// A random circuit of mostly conjunctions over `inputs` inputs and `latches` latches, so that many of its nodes are
/// rarely set and look alike under random patterns without being alike.
fn rarely_set(seed: u64, inputs: usize, latches: usize, gates: usize) -> Aig {
    let mut next = xorshift(seed);
    let mut g = Aig::new();
    let mut lits: Vec<AigLit> = (0..inputs).map(|i| named(&mut g, &format!("x{i}"))).collect();
    for k in 0..latches {
        let l = g.add_latch(Some(format!("q{k}")), crate::aig::Init::Zero);
        lits.push(g.latch_lit(l));
    }
    for _ in 0..gates {
        let a = lits[(next() % lits.len() as u64) as usize].with_phase(next() % 8 == 0);
        let b = lits[(next() % lits.len() as u64) as usize].with_phase(next() % 8 == 0);
        let c = if next() % 6 == 0 { g.xor(a, b) } else { g.and(a, b) };
        lits.push(c);
    }
    g
}

#[test]
fn a_node_is_found_the_first_node_joined_with_its_signature_through_every_counterexample() {
    // Conjunctions over many inputs are alike under the first random words without being alike, and counterexamples
    // drawn mostly true set them, so classes of several nodes split, across fills and words folded away; every class
    // is found against the definition: the first node joined whose signature over every pattern so far is the node's.
    let mut changed = 0;
    for seed in 0..8u64 {
        let g = rarely_set(seed, 16, 2, 160);
        let nodes: Vec<u32> = (0..g.num_nodes() as u32).collect();
        let mut sim = Sim::random(&g, 1 + seed as usize % 2, seed, nodes.clone());
        let mut next = xorshift(seed + 100);
        let mut joined: Vec<u32> = Vec::new();
        let mut patterns = 0;
        let classes = |sim: &Sim, joined: &[u32]| -> Vec<Option<u32>> {
            let signature: Vec<Vec<u64>> = nodes.iter().map(|&n| sim.signature(n).0).collect();
            nodes.iter().map(|&n| joined.iter().copied().find(|&m| signature[m as usize] == signature[n as usize])).collect()
        };
        let check = |sim: &Sim, expected: &[Option<u32>], when: &str| {
            for &n in &nodes {
                assert_eq!(sim.class_of(n), expected[n as usize], "seed {seed}, {when}: node {n}");
            }
        };
        for &n in &nodes {
            if next() % 4 != 0 {
                sim.join(n);
                joined.push(n);
                check(&sim, &classes(&sim, &joined), &format!("node {n} joined"));
            }
            for _ in 0..next() % 4 {
                let before = classes(&sim, &joined);
                let inputs: Vec<bool> = (0..g.inputs().len()).map(|_| next() % 8 != 0).collect();
                let latches: Vec<bool> = (0..g.latches().len()).map(|_| next() % 8 != 0).collect();
                sim.add(&g, &inputs, &latches);
                patterns += 1;
                let values = g.eval(&inputs, &latches);
                let (word, bit) = (sim.words - 1, sim.used - 1);
                for &m in &nodes {
                    let (signature, phase) = sim.signature(m);
                    let simulated = (signature[word] >> bit & 1 == 1) ^ phase;
                    assert_eq!(simulated, values[m as usize], "seed {seed}, pattern {patterns}: node {m} simulates as it evaluates");
                }
                let after = classes(&sim, &joined);
                changed += joined.iter().filter(|&&m| before[m as usize] != after[m as usize]).count();
                check(&sim, &after, &format!("pattern {patterns}"));
            }
        }
        assert!(patterns > 130, "seed {seed}: the counterexamples fill more than two words ({patterns})");
    }
    assert!(changed > 20, "classes of several joined nodes split ({changed} nodes left the class they were in)");
}

#[test]
fn a_class_a_node_joins_is_split_by_the_next_counterexample_that_tells_its_nodes_apart() {
    // Two conjunctions of twelve inputs, one with an input complemented: random patterns all but never set either, so
    // the second joins the first's class, and the pattern setting the first tells them apart, whether it lands in a
    // fresh word, at the end of the word the joining happened in, or after that word is full.
    let mut g = Aig::new();
    let x = word(&mut g, "x", 12);
    let p = x[1..].iter().fold(x[0], |acc, &v| g.and(acc, v));
    let mut flipped = x.clone();
    flipped[11] = !flipped[11];
    let q = flipped[1..].iter().fold(flipped[0], |acc, &v| g.and(acc, v));
    let nodes: Vec<u32> = (0..g.num_nodes() as u32).collect();
    for before in [0, 63, 64] {
        let mut sim = Sim::random(&g, 1, 5, nodes.clone());
        assert_eq!(sim.signature(p.node()).0, sim.signature(q.node()).0, "the two look alike");
        for _ in 0..before {
            sim.add(&g, &[false; 12], &[]);
        }
        sim.join(p.node());
        sim.join(q.node());
        assert_eq!(sim.class_of(q.node()), Some(p.node()), "{before} patterns before: the second joins the first");
        sim.add(&g, &[true; 12], &[]);
        assert_eq!(sim.class_of(p.node()), Some(p.node()), "{before} patterns before");
        assert_eq!(sim.class_of(q.node()), Some(q.node()), "{before} patterns before: the pattern tells them apart");
    }
}

#[test]
fn a_counterexample_simulates_one_word_of_each_node_however_many_words_there_are() {
    let (g, pairs) = adders_beside_a_multiplier(12, true);
    for words in [1, 16, 64] {
        let mut e = Engine::new(&g, &pairs, &Options { sim_words: words, ..Options::default() });
        let cone = e.stats.cone as u64;
        assert_eq!(e.sim.simulated, cone * words as u64, "words {words}: the random words simulate each node a word each");
        for k in 1..=130u64 {
            let inputs: Vec<bool> = (0..g.inputs().len() as u64).map(|i| (i * 7 + k * 3) % 5 < 2).collect();
            e.sim.add(&g, &inputs, &[]);
            assert_eq!(e.sim.simulated, cone * (words as u64 + k), "words {words}: counterexample {k} simulates one word");
        }
    }
}

/// Two conjunctions of `width` inputs, a chain against a balanced tree, and the chain against a tree with one input
/// complemented: nodes random patterns all but never set, alike under simulation until counterexamples tell them apart.
fn wide_ands(width: usize) -> (Aig, Aig) {
    let mut l = Aig::new();
    let x = word(&mut l, "x", width);
    let chain = x[1..].iter().fold(x[0], |acc, &v| l.and(acc, v));
    l.add_output(chain, Some("y".into()));
    l.add_output(chain, Some("z".into()));
    let mut r = Aig::new();
    let x = word(&mut r, "x", width);
    fn tree(g: &mut Aig, leaves: &[AigLit]) -> AigLit {
        match leaves {
            [one] => *one,
            _ => {
                let (a, b) = leaves.split_at(leaves.len() / 2);
                let (a, b) = (tree(g, a), tree(g, b));
                g.and(a, b)
            }
        }
    }
    let y = tree(&mut r, &x);
    let mut flipped = x.clone();
    flipped[width / 2] = !flipped[width / 2];
    let z = tree(&mut r, &flipped);
    r.add_output(y, Some("y".into()));
    r.add_output(z, Some("z".into()));
    (l, r)
}

#[test]
fn a_decision_simulates_a_word_of_each_node_for_each_random_word_and_each_counterexample() {
    let (l, r) = wide_ands(24);
    let opts = Options::default();
    let report = equivalent(&l, &r, &opts).unwrap();
    let s = &report.stats;
    assert_eq!(report.verdicts[0].1, Verdict::Equal, "{:?}", report.verdicts);
    assert!(matches!(report.verdicts[1].1, Verdict::Differ { .. }), "{:?}", report.verdicts);
    assert!(report.check(), "{s:?}");
    assert!(s.counterexamples > 0, "the conjunctions' nodes are alike under random patterns: {s:?}");
    assert_eq!(s.simulated, s.cone as u64 * (opts.sim_words + s.counterexamples) as u64, "{s:?}");
}

#[test]
fn a_model_costs_a_query_its_cone_not_the_solver() {
    // A solver holding two multipliers' cones answers a query on two inputs' conjunction against their disjunction,
    // which differ, by deciding within that cone of four nodes and assigning nothing past it, in each of its two solves.
    let width = 6;
    let mut g = Aig::new();
    let (a, b) = (word(&mut g, "a", width), word(&mut g, "b", width));
    let p = multiplier(&mut g, &a, &b);
    let q = multiplier(&mut g, &b, &a);
    let (x, y) = (named(&mut g, "x"), named(&mut g, "y"));
    let (and, or) = (g.and(x, y), g.or(x, y));
    let pairs: Vec<(AigLit, AigLit)> = p.iter().copied().zip(q.iter().copied()).chain([(and, or)]).collect();
    let mut e = Engine::new(&g, &pairs, &Options::default());
    for &(l, r) in &pairs[..width] {
        for side in [l, r] {
            assert!(matches!(e.prove(side, AigLit::FALSE, 0), Proved::Differ(..)), "a product bit can be set");
        }
    }
    let held = e.nodes.len();
    assert!(held > 100, "the solver holds the multipliers ({held} variables)");
    let (before, propagated) = (e.solver.decisions(), e.solver.propagations());
    assert!(matches!(e.prove(and, or, 0), Proved::Differ(..)));
    let (decided, assigned) = (e.solver.decisions() - before, e.solver.propagations() - propagated);
    assert!(decided <= 4, "{decided} decisions, the solver holding {held} variables");
    assert!(assigned <= 2 * 4, "{assigned} assignments, the solver holding {held} variables");
}

#[test]
fn a_model_is_checked_on_the_two_cones_alone_and_as_the_circuit_evaluates() {
    for seed in 0..20u64 {
        let g = random_sequential(seed, 6, 2, 120);
        let top = AigLit::new(g.num_nodes() as u32 - 1, false);
        let mut e = Engine::new(&g, &[(top, !top)], &Options::default());
        let mut next = xorshift(seed + 7);
        for _ in 0..30 {
            let mut pick = || AigLit::new((next() % g.num_nodes() as u64) as u32, next() & 1 == 1);
            let (a, b) = (pick(), pick());
            let inputs: Vec<bool> = (0..g.inputs().len()).map(|_| next() & 1 == 1).collect();
            let latches: Vec<bool> = (0..g.latches().len()).map(|_| next() & 1 == 1).collect();
            let values = g.eval(&inputs, &latches);
            let apart = Aig::value(&values, a) != Aig::value(&values, b);
            assert_eq!(e.separates(&inputs, &latches, a, b), apart, "seed {seed}: {a:?} against {b:?}");
            let cone: HashSet<u32> = g.cone(&[a, b]).into_iter().collect();
            for n in 0..g.num_nodes() as u32 {
                assert_eq!(e.met[n as usize] == e.walk, cone.contains(&n), "seed {seed}: node {n} looked at only in the cones");
            }
        }
    }
}

#[test]
fn a_counterexample_draws_the_inputs_its_query_leaves_open_and_still_separates_the_pair() {
    // Two inputs' conjunction against their disjunction beside thirty inputs no query reads: every counterexample sets
    // the two as the solver's model does and draws the thirty, both values among them, the same draws for the same seed.
    let mut g = Aig::new();
    let (x, y) = (named(&mut g, "x"), named(&mut g, "y"));
    let rest = word(&mut g, "r", 30);
    let (and, or) = (g.and(x, y), g.or(x, y));
    let wide = rest.iter().fold(AigLit::TRUE, |acc, &r| g.and(acc, r));
    g.add_output(wide, Some("w".into()));
    let draws = |seed: u64| -> Vec<Vec<bool>> {
        let mut e = Engine::new(&g, &[(and, or)], &Options { seed, ..Options::default() });
        (0..20)
            .map(|_| match e.prove(and, or, 0) {
                Proved::Differ(inputs, _) => {
                    let values = g.eval(&inputs, &[]);
                    assert_ne!(Aig::value(&values, and), Aig::value(&values, or), "the counterexample separates the pair");
                    inputs
                }
                other => panic!("the two differ: {:?}", matches!(other, Proved::Equal(_))),
            })
            .collect()
    };
    let first = draws(7);
    let open: Vec<bool> = first.iter().flat_map(|inputs| inputs[2..].to_vec()).collect();
    assert!(open.iter().any(|&v| v) && open.iter().any(|&v| !v), "the open inputs are drawn, not fixed");
    assert_eq!(draws(7), first, "the same seed draws the same counterexamples");
    assert_ne!(draws(8), first, "another seed draws others");
}
