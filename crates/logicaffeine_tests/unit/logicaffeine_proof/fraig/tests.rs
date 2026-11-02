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
