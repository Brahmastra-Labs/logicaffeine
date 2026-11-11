use super::*;
use crate::cdcl::{SolveResult, Solver};
use crate::families::{php, random_3sat};

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn random_formula(next: &mut impl FnMut() -> u64, num_vars: usize, num_clauses: usize, max_width: usize) -> Vec<Vec<Lit>> {
    (0..num_clauses)
        .map(|_| {
            let width = 1 + (next() % max_width as u64) as usize;
            (0..width).map(|_| Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0)).collect()
        })
        .collect()
}

fn satisfies(clauses: &[Vec<Lit>], model: &[bool]) -> bool {
    clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
}

fn models(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<bool>> {
    (0..1u32 << num_vars)
        .map(|bits| (0..num_vars).map(|v| bits >> v & 1 == 1).collect::<Vec<bool>>())
        .filter(|m| satisfies(clauses, m))
        .collect()
}

fn cdcl_sat(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    let mut s = Solver::new(num_vars);
    for c in clauses {
        s.add_clause(c.clone());
    }
    matches!(s.solve(), SolveResult::Sat(_))
}

/// Every addition is RUP against the input and the additions before it; RUP only gains from more clauses, so the
/// deletions are not replayed.
fn steps_replay(num_vars: usize, input: &[Vec<Lit>], steps: &[ProofStep]) -> bool {
    let mut db = input.to_vec();
    for step in steps {
        match step {
            ProofStep::Rup(c) => {
                if !crate::rup::is_rup(num_vars, &db, c) {
                    return false;
                }
                db.push(c.clone());
            }
            ProofStep::Delete(_) => {}
            ProofStep::Pr { .. } => return false,
        }
    }
    true
}

#[test]
fn every_model_of_the_simplified_formula_extends_to_a_model_of_the_input_and_satisfiability_is_kept() {
    let mut next = rng(0xE11A);
    let mut eliminated = 0;
    for round in 0..900u64 {
        let n = 1 + (next() % 10) as usize;
        let m = (next() % (4 * n as u64 + 2)) as usize;
        let input = random_formula(&mut next, n, m, 4);
        let p = preprocess(n, &input, &Options::default());
        eliminated += p.eliminated;
        let input_sat = !models(n, &input).is_empty();
        if p.unsat {
            assert!(!input_sat, "round {round}: a satisfiable input called unsatisfiable");
            continue;
        }
        let simplified_models = models(n, &p.clauses);
        assert_eq!(!simplified_models.is_empty(), input_sat, "round {round}: satisfiability changed");
        for mut model in simplified_models {
            p.extend(&mut model);
            assert!(satisfies(&input, &model), "round {round}: an extended model fails the input");
        }
    }
    assert!(eliminated > 300, "the sample exercises elimination ({eliminated} variables)");
}

#[test]
fn the_proof_replays_against_the_input_and_an_unsatisfiable_one_ends_in_the_empty_clause() {
    let mut next = rng(0x9E00F);
    let mut refuted = 0;
    for round in 0..900u64 {
        let n = 1 + (next() % 9) as usize;
        let m = (next() % (5 * n as u64 + 2)) as usize;
        let input = random_formula(&mut next, n, m, 3);
        let p = preprocess(n, &input, &Options::default());
        assert!(steps_replay(n, &input, &p.steps), "round {round}: a step does not replay");
        if p.unsat {
            refuted += 1;
            assert_eq!(p.steps.iter().rev().find(|s| matches!(s, ProofStep::Rup(_))).map(|s| s.clause().len()), Some(0), "round {round}: no empty clause");
        }
        for step in &p.steps {
            if let ProofStep::Rup(c) = step {
                assert!(c.iter().all(|l| (l.var() as usize) < n), "round {round}: a step names a variable past the input's");
            }
        }
    }
    assert!(refuted > 50, "the sample holds unsatisfiable inputs preprocessing refutes ({refuted})");
}

#[test]
fn each_simplification_can_be_switched_off_and_every_combination_stays_sound() {
    let mut next = rng(0xC0B0);
    for round in 0..200u64 {
        let n = 2 + (next() % 8) as usize;
        let m = (next() % (4 * n as u64 + 2)) as usize;
        let input = random_formula(&mut next, n, m, 4);
        let input_sat = !models(n, &input).is_empty();
        for mask in 0..8u32 {
            let options = Options { eliminate: mask & 1 != 0, substitute: mask & 2 != 0, subsume: mask & 4 != 0, ..Options::default() };
            let p = preprocess(n, &input, &options);
            assert!(steps_replay(n, &input, &p.steps), "round {round} mask {mask}");
            if p.unsat {
                assert!(!input_sat, "round {round} mask {mask}");
                continue;
            }
            let simplified = models(n, &p.clauses);
            assert_eq!(!simplified.is_empty(), input_sat, "round {round} mask {mask}");
            for mut model in simplified {
                p.extend(&mut model);
                assert!(satisfies(&input, &model), "round {round} mask {mask}");
            }
            if !options.eliminate {
                assert_eq!(p.eliminated, 0);
            }
            if !options.substitute {
                assert_eq!(p.substituted, 0);
            }
            if !options.subsume {
                assert_eq!((p.subsumed, p.strengthened), (0, 0));
            }
        }
    }
}

#[test]
fn a_chain_of_equivalences_collapses_to_one_variable_and_extension_restores_every_member() {
    // x0 ≡ x1 ≡ ¬x2 ≡ x3 by binary clauses, plus clauses that use each of them.
    let l = |v: Var, p: bool| Lit::new(v, p);
    let mut input = Vec::new();
    for (a, b) in [(l(0, true), l(1, true)), (l(1, true), l(2, false)), (l(2, false), l(3, true))] {
        input.push(vec![a.negated(), b]);
        input.push(vec![a, b.negated()]);
    }
    input.push(vec![l(0, true), l(4, true), l(5, true)]);
    input.push(vec![l(2, true), l(4, false), l(5, true)]);
    input.push(vec![l(3, false), l(5, false), l(6, true)]);
    let options = Options { eliminate: false, subsume: false, ..Options::default() };
    let p = preprocess(7, &input, &options);
    assert!(!p.unsat);
    assert_eq!(p.substituted, 3, "three of the four equivalent variables are replaced by their representative");
    let mentioned: std::collections::BTreeSet<Var> = p.clauses.iter().flatten().map(|l| l.var()).collect();
    assert_eq!(mentioned.iter().filter(|&&v| v <= 3).count(), 1, "one representative remains: {mentioned:?}");
    for mut model in models(7, &p.clauses) {
        p.extend(&mut model);
        assert!(satisfies(&input, &model));
        assert!(model[0] == model[1] && model[1] == !model[2] && !model[2] == model[3]);
    }
}

#[test]
fn opposite_literals_in_one_class_refute_the_formula() {
    // x0 → x1 → ¬x0 and ¬x0 → x2 → x0: the binary clauses put x0 and ¬x0 in one class.
    let l = |v: Var, p: bool| Lit::new(v, p);
    let input = vec![
        vec![l(0, false), l(1, true)],
        vec![l(1, false), l(0, false)],
        vec![l(0, true), l(2, true)],
        vec![l(2, false), l(0, true)],
        vec![l(1, true), l(2, true), l(3, true)],
    ];
    assert!(models(4, &input).is_empty());
    let options = Options { eliminate: false, subsume: false, ..Options::default() };
    let p = preprocess(4, &input, &options);
    assert!(p.unsat, "substitution alone sees x0 ≡ ¬x0");
    assert!(steps_replay(4, &input, &p.steps));
    assert_eq!(p.steps.last().map(|s| s.clause().len()), Some(0));
}

#[test]
fn an_and_gate_is_eliminated_by_its_definition_alone() {
    // y = a ∧ b ∧ c (Tseitin): (¬y ∨ a) (¬y ∨ b) (¬y ∨ c) (y ∨ ¬a ∨ ¬b ∨ ¬c), with y also in (y ∨ d), (y ∨ ¬e ∨ ¬f)
    // and (¬y ∨ e ∨ f). The gate's clauses against the other three give 7 resolvents, one per clause of y, so y goes
    // within bound 0; all pairs would give 8, the extra (d ∨ e ∨ f) of two non-gate clauses following from the 7.
    let l = |v: usize, p: bool| Lit::new(v as Var, p);
    let (y, a, b, c, d, e, f) = (0usize, 1, 2, 3, 4, 5, 6);
    let input = vec![
        vec![l(y, false), l(a, true)],
        vec![l(y, false), l(b, true)],
        vec![l(y, false), l(c, true)],
        vec![l(y, true), l(a, false), l(b, false), l(c, false)],
        vec![l(y, true), l(d, true)],
        vec![l(y, true), l(e, false), l(f, false)],
        vec![l(y, false), l(e, true), l(f, true)],
    ];
    let views: Vec<&[Lit]> = input.iter().map(|c| c.as_slice()).collect();
    assert_eq!(definition(&views, y as Var), Some(vec![0, 1, 2, 3]), "the AND definition of y is found");

    let options = Options::default();
    let mut db = Db::new(7, &options);
    db.load(&input);
    db.try_eliminate(y as Var);
    assert_eq!(db.eliminated, 1, "with the gate, y's elimination fits bound 0");
    let live: Vec<Vec<Lit>> = (0..db.clauses.len()).filter(|&i| db.alive[i]).map(|i| db.clauses[i].clone()).collect();
    assert_eq!(live.len(), 7, "one resolvent per removed clause");
    assert!(!live.iter().any(|c| c.iter().any(|lit| lit.var() == y as Var)), "y is gone");
    let def = normalized(&[l(d, true), l(e, true), l(f, true)]).unwrap();
    assert!(!live.contains(&def), "the non-gate resolvent (d ∨ e ∨ f) is not added");
    let p = db.finish();
    assert!(steps_replay(7, &input, &p.steps));
    for mut model in models(7, &p.clauses) {
        p.extend(&mut model);
        assert!(satisfies(&input, &model));
        assert_eq!(model[y], model[a] && model[b] && model[c], "y is restored to its definition");
    }
}

#[test]
fn an_xor_gate_is_found_over_every_sign_pattern() {
    // y = a ⊕ b as the four clauses of odd negation parity over {y, a, b}, in either orientation.
    let l = |v: usize, p: bool| Lit::new(v as Var, p);
    for flip in [false, true] {
        let mut gate: Vec<Vec<Lit>> = Vec::new();
        for bits in 0..8u32 {
            let lits: Vec<Lit> = (0..3).map(|v| l(v, bits >> v & 1 == 0)).collect();
            let negations = (bits.count_ones() % 2 == 1) ^ flip;
            if negations {
                gate.push(lits);
            }
        }
        assert_eq!(gate.len(), 4);
        let mut input = gate.clone();
        input.push(vec![l(0, true), l(3, true)]);
        let views: Vec<&[Lit]> = input.iter().map(|c| c.as_slice()).collect();
        let found = definition(&views, 0).expect("the XOR definition is found");
        assert_eq!(found, vec![0, 1, 2, 3]);
        let p = preprocess(4, &input, &Options::default());
        for mut model in models(4, &p.clauses) {
            p.extend(&mut model);
            assert!(satisfies(&input, &model));
        }
    }
    let partial: Vec<Vec<Lit>> = vec![vec![l(0, true), l(1, true), l(2, true)], vec![l(0, false), l(1, false), l(2, true)]];
    let views: Vec<&[Lit]> = partial.iter().map(|c| c.as_slice()).collect();
    assert_eq!(definition(&views, 0), None, "two of four parity clauses define nothing");
}

#[test]
fn elimination_never_grows_the_formula_past_its_bound() {
    let mut next = rng(0xB0D);
    for round in 0..300u64 {
        let n = 4 + (next() % 20) as usize;
        let m = (next() % (5 * n as u64 + 2)) as usize;
        let input = random_formula(&mut next, n, m, 4);
        for bound in [0usize, 1, 4] {
            let options = Options { substitute: false, subsume: false, bound, ..Options::default() };
            let p = preprocess(n, &input, &options);
            if p.unsat {
                continue;
            }
            let distinct_input: std::collections::HashSet<Vec<Lit>> = input.iter().map(|c| {
                let mut c = c.clone();
                c.sort_by_key(|l| l.index());
                c.dedup();
                c
            }).filter(|c| !c.windows(2).any(|w| w[0].var() == w[1].var())).collect();
            assert!(
                p.clauses.len() <= distinct_input.len() + bound * p.eliminated,
                "round {round} bound {bound}: {} clauses from {} after {} eliminations",
                p.clauses.len(),
                distinct_input.len(),
                p.eliminated
            );
        }
    }
}

#[test]
fn subsumed_clauses_go_and_self_subsumed_ones_shrink() {
    let l = |v: Var, p: bool| Lit::new(v, p);
    let input = vec![
        vec![l(0, true), l(1, true)],
        vec![l(0, true), l(1, true), l(2, true)],
        vec![l(0, true), l(1, false), l(3, true)],
        vec![l(2, true), l(3, true), l(4, true)],
    ];
    let options = Options { eliminate: false, substitute: false, ..Options::default() };
    let p = preprocess(5, &input, &options);
    assert_eq!(p.subsumed, 1, "(x0 ∨ x1 ∨ x2) is subsumed by (x0 ∨ x1)");
    assert_eq!(p.strengthened, 1, "(x0 ∨ ¬x1 ∨ x3) resolves with (x0 ∨ x1) to (x0 ∨ x3)");
    assert!(p.clauses.contains(&vec![l(0, true), l(3, true)]) || p.clauses.contains(&vec![l(3, true), l(0, true)]));
    assert!(steps_replay(5, &input, &p.steps));
}

#[test]
fn units_fix_their_variables_and_extension_restores_them() {
    let l = |v: Var, p: bool| Lit::new(v, p);
    let input = vec![vec![l(0, true)], vec![l(0, false), l(1, false)], vec![l(1, true), l(2, true), l(3, true)], vec![l(2, false), l(3, true)]];
    let p = preprocess(4, &input, &Options::default());
    assert!(!p.unsat);
    assert!(p.fixed >= 2, "x0 and then ¬x1 are fixed");
    assert!(!p.clauses.iter().flatten().any(|lit| lit.var() <= 1));
    for mut model in models(4, &p.clauses) {
        p.extend(&mut model);
        assert!(model[0] && !model[1] && satisfies(&input, &model));
    }
}

#[test]
fn random_3sat_and_pigeonhole_preprocess_equisatisfiably_at_scale() {
    for seed in 0..8u64 {
        for (n, m) in [(150usize, 600usize), (200, 860)] {
            let cnf = random_3sat(n, m, 0xD00D ^ seed);
            let p = preprocess(cnf.num_vars, &cnf.clauses, &Options::default());
            let before = cdcl_sat(cnf.num_vars, &cnf.clauses);
            assert!(steps_replay(cnf.num_vars, &cnf.clauses, &p.steps));
            if p.unsat {
                assert!(!before);
                continue;
            }
            let mut s = Solver::new(cnf.num_vars);
            for c in &p.clauses {
                s.add_clause(c.clone());
            }
            match s.solve() {
                SolveResult::Sat(mut model) => {
                    assert!(before, "seed {seed} n {n}: the simplified formula is satisfiable, the input is not");
                    p.extend(&mut model);
                    assert!(satisfies(&cnf.clauses, &model), "seed {seed} n {n}");
                }
                SolveResult::Unsat => assert!(!before, "seed {seed} n {n}: the simplified formula lost every model"),
            }
        }
    }
    let (cnf, _) = php(6);
    let p = preprocess(cnf.num_vars, &cnf.clauses, &Options::default());
    assert!(steps_replay(cnf.num_vars, &cnf.clauses, &p.steps));
    assert!(p.unsat || !cdcl_sat(cnf.num_vars, &p.clauses));
}

#[test]
fn preprocessing_is_deterministic() {
    let cnf = random_3sat(180, 760, 42);
    let a = preprocess(cnf.num_vars, &cnf.clauses, &Options::default());
    let b = preprocess(cnf.num_vars, &cnf.clauses, &Options::default());
    assert_eq!(a.clauses, b.clauses);
    assert_eq!(a.steps, b.steps);
    assert_eq!((a.eliminated, a.substituted, a.subsumed, a.strengthened, a.fixed), (b.eliminated, b.substituted, b.subsumed, b.strengthened, b.fixed));
}

#[test]
fn solve_preprocessed_matches_plain_cdcl_with_checked_models_and_refutations() {
    let mut next = rng(0x5017E);
    for round in 0..300u64 {
        let n = 2 + (next() % 30) as usize;
        let m = (next() % (5 * n as u64 + 2)) as usize;
        let input = random_formula(&mut next, n, m, 4);
        let expected = cdcl_sat(n, &input);
        match solve_preprocessed(n, &input) {
            (SolveResult::Sat(model), _) => {
                assert!(expected, "round {round}: SAT on an unsatisfiable input");
                assert!(satisfies(&input, &model), "round {round}: the model fails the input");
            }
            (SolveResult::Unsat, proof) => {
                assert!(!expected, "round {round}: UNSAT on a satisfiable input");
                let learned: Vec<Vec<Lit>> = proof.iter().filter_map(|s| match s {
                    ProofStep::Rup(c) => Some(c.clone()),
                    _ => None,
                }).collect();
                assert!(crate::rup::check_refutation(n, &input, &learned), "round {round}: the refutation does not check");
            }
        }
    }
}

/// The effort counts every clause a subsumption pass looks at, so a run stops when its effort is spent however wide
/// the clauses are and however often their literals occur: 10,000 random width-40 clauses over 60 variables — each
/// literal in ~3,300 clauses, ~10⁸ candidate pairs costing ~10⁹ literal reads unaccounted — finish within a minute
/// unoptimized on a loaded box at an effort of a million, the formula still satisfiable and the model extending.
#[test]
fn preprocessing_stops_when_its_effort_is_spent() {
    let mut next = rng(0xEFF0);
    let clauses: Vec<Vec<Lit>> = (0..10_000)
        .map(|_| {
            let mut vars: Vec<u32> = Vec::with_capacity(40);
            while vars.len() < 40 {
                let v = (next() % 60) as u32;
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            vars.into_iter().map(|v| Lit::new(v, next() & 1 == 0)).collect()
        })
        .collect();
    let formula = clauses.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let options = Options { effort: 1_000_000, ..Options::default() };
        let _ = tx.send(preprocess(60, &formula, &options));
    });
    let pre = rx.recv_timeout(std::time::Duration::from_secs(60)).expect("preprocessing stops when its effort is spent");
    assert!(!pre.unsat, "random 40-SAT over 60 variables is satisfiable");
    let mut s = Solver::new(60);
    for c in &pre.clauses {
        s.add_clause(c.clone());
    }
    let SolveResult::Sat(mut model) = s.solve() else { panic!("the simplified formula is satisfiable") };
    pre.extend(&mut model);
    assert!(satisfies(&clauses, &model), "the model extends to one of the input");
}

/// A gate of a random circuit: its kind and its input signals (with a negation flag each).
#[derive(Clone, Copy)]
enum Kind {
    And,
    Xor,
    /// `inputs[0] ? inputs[1] : inputs[2]`.
    Ite,
}

/// Tseitin clauses of `out = kind(inputs)`: an AND as its binaries and its long clause, an XOR as its parity gadget,
/// an if-then-else as its four ternary clauses.
fn encode_gate(out: Lit, kind: Kind, inputs: &[Lit]) -> Vec<Vec<Lit>> {
    match kind {
        Kind::Ite => {
            let (c, t, e) = (inputs[0], inputs[1], inputs[2]);
            vec![
                vec![c.negated(), t.negated(), out],
                vec![c.negated(), t, out.negated()],
                vec![c, e.negated(), out],
                vec![c, e, out.negated()],
            ]
        }
        Kind::And => {
            let mut clauses: Vec<Vec<Lit>> = inputs.iter().map(|&a| vec![out.negated(), a]).collect();
            clauses.push(std::iter::once(out).chain(inputs.iter().map(|a| a.negated())).collect());
            clauses
        }
        Kind::Xor => {
            // out ⊕ a₁ ⊕ … ⊕ a_k = 0: forbid every assignment of odd parity over (out, a₁, …, a_k).
            let lits: Vec<Lit> = std::iter::once(out).chain(inputs.iter().copied()).collect();
            (0u32..1 << lits.len())
                .filter(|m| m.count_ones() % 2 == 1)
                .map(|m| lits.iter().enumerate().map(|(i, &l)| if m >> i & 1 == 1 { l.negated() } else { l }).collect())
                .collect()
        }
    }
}

/// A random circuit over `inputs` primary inputs (variables `0..inputs`): `gates` gates, each an AND or an XOR of two
/// or three earlier signals or an if-then-else of three, some negated. Returns each gate's (kind, input literals) by
/// signal index.
fn random_circuit(next: &mut impl FnMut() -> u64, inputs: usize, gates: usize) -> Vec<(Kind, Vec<(usize, bool)>)> {
    (0..gates)
        .map(|g| {
            let signals = inputs + g;
            let kind = [Kind::And, Kind::Xor, Kind::Ite][(next() % 3) as usize];
            let kind = if matches!(kind, Kind::Ite) && signals < 3 { Kind::And } else { kind };
            let arity = if matches!(kind, Kind::Ite) { 3 } else { 2 + (next() % 2) as usize };
            let mut picked: Vec<usize> = Vec::new();
            while picked.len() < arity.min(signals) {
                let s = (next() % signals as u64) as usize;
                if !picked.contains(&s) {
                    picked.push(s);
                }
            }
            (kind, picked.into_iter().map(|s| (s, next() % 3 == 0)).collect())
        })
        .collect()
}

/// The clauses of a circuit whose gate `g` drives variable `base + g`, its signal `s` being the primary input `s` or
/// the gate `s − inputs`.
fn encode_circuit(circuit: &[(Kind, Vec<(usize, bool)>)], inputs: usize, base: usize) -> Vec<Vec<Lit>> {
    let signal = |s: usize, negated: bool| {
        let v = if s < inputs { s } else { base + s - inputs };
        Lit::new(v as Var, !negated)
    };
    circuit
        .iter()
        .enumerate()
        .flat_map(|(g, (kind, ins))| {
            let ins: Vec<Lit> = ins.iter().map(|&(s, n)| signal(s, n)).collect();
            encode_gate(Lit::pos((base + g) as Var), *kind, &ins)
        })
        .collect()
}

/// A miter: two circuits over the same primary inputs, gate variables `inputs..inputs+gates` and
/// `inputs+gates..inputs+2·gates`, and clauses saying their last outputs differ.
fn miter(first: &[(Kind, Vec<(usize, bool)>)], second: &[(Kind, Vec<(usize, bool)>)], inputs: usize) -> (usize, Vec<Vec<Lit>>) {
    let gates = first.len();
    let mut clauses = encode_circuit(first, inputs, inputs);
    clauses.extend(encode_circuit(second, inputs, inputs + gates));
    let (a, b) = (Lit::pos((inputs + gates - 1) as Var), Lit::pos((inputs + 2 * gates - 1) as Var));
    clauses.push(vec![a, b]);
    clauses.push(vec![a.negated(), b.negated()]);
    (inputs + 2 * gates, clauses)
}

/// Two AND gates of the same inputs have equal outputs: congruence merges them, and a formula demanding they differ
/// is refuted by preprocessing alone, its steps replaying against the input.
#[test]
fn congruent_and_gates_merge_and_their_miter_is_refuted_by_preprocessing() {
    let (a, b, c, g, h) = (Lit::pos(0), Lit::pos(1), Lit::neg(2), Lit::pos(3), Lit::pos(4));
    let mut clauses = encode_gate(g, Kind::And, &[a, b, c]);
    clauses.extend(encode_gate(h, Kind::And, &[c, a, b]));
    clauses.push(vec![g, h]);
    clauses.push(vec![g.negated(), h.negated()]);
    let options = Options { eliminate: false, subsume: false, ..Options::default() };
    let pre = preprocess(5, &clauses, &options);
    assert!(pre.unsat, "g ≡ h by congruence, so g ≠ h is unsatisfiable");
    assert!(steps_replay(5, &clauses, &pre.steps), "every congruence step is RUP");
    assert!(pre.congruent >= 1, "the merge is counted");
}

/// Two XOR gates of the same inputs, each written with its own sign pattern, have outputs equal up to their parity:
/// congruence merges them, refuting a formula that says otherwise with replaying steps.
#[test]
fn congruent_xor_gates_merge_through_their_signs_and_their_miter_is_refuted() {
    for negate in [false, true] {
        let (a, b, c) = (Lit::pos(0), Lit::pos(1), Lit::pos(2));
        let (g, h) = (Lit::pos(3), Lit::pos(4));
        let mut clauses = encode_gate(g, Kind::Xor, &[a, b, c]);
        // h = ¬a ⊕ b ⊕ c = ¬g when negated, else h = a ⊕ b ⊕ c = g.
        let first = if negate { a.negated() } else { a };
        clauses.extend(encode_gate(h, Kind::Xor, &[c, first, b]));
        let equal = !negate;
        // Demand the opposite of what congruence proves.
        if equal {
            clauses.push(vec![g, h]);
            clauses.push(vec![g.negated(), h.negated()]);
        } else {
            clauses.push(vec![g, h.negated()]);
            clauses.push(vec![g.negated(), h]);
        }
        let options = Options { eliminate: false, subsume: false, ..Options::default() };
        let pre = preprocess(5, &clauses, &options);
        assert!(pre.unsat, "negate={negate}: congruence decides the miter");
        assert!(steps_replay(5, &clauses, &pre.steps), "negate={negate}: every step is RUP");
    }
}

/// Two if-then-else gates of equal inputs — one written with its condition negated and its branches swapped — have
/// equal outputs: congruence merges them and refutes a formula that says otherwise, every step replaying.
#[test]
fn congruent_if_then_else_gates_merge_through_a_negated_condition() {
    let (c, t, e, g, h) = (Lit::pos(0), Lit::pos(1), Lit::neg(2), Lit::pos(3), Lit::pos(4));
    let mut clauses = encode_gate(g, Kind::Ite, &[c, t, e]);
    clauses.extend(encode_gate(h, Kind::Ite, &[c.negated(), e, t]));
    clauses.push(vec![g, h]);
    clauses.push(vec![g.negated(), h.negated()]);
    let options = Options { eliminate: false, subsume: false, ..Options::default() };
    let pre = preprocess(5, &clauses, &options);
    assert!(pre.unsat, "g ≡ h by congruence: ¬c ? e : t is c ? t : e");
    assert!(steps_replay(5, &clauses, &pre.steps), "every step is RUP");
}

/// Congruence closes: merging two gates' outputs makes the gates reading them congruent in turn, level by level.
#[test]
fn congruence_closes_through_every_level_of_a_circuit() {
    let mut next = rng(0xC1C1);
    for round in 0..40u64 {
        let inputs = 3 + (next() % 4) as usize;
        let gates = 6 + (next() % 10) as usize;
        let circuit = random_circuit(&mut next, inputs, gates);
        let (num_vars, clauses) = miter(&circuit, &circuit, inputs);
        let options = Options { eliminate: false, subsume: false, ..Options::default() };
        let pre = preprocess(num_vars, &clauses, &options);
        assert!(pre.unsat, "round {round}: a circuit equals itself, so its miter is refuted by congruence");
        assert!(steps_replay(num_vars, &clauses, &pre.steps), "round {round}: the refutation replays");
    }
}

/// On miters of two different random circuits — equivalent or not — preprocessing with congruence keeps the
/// verdict, every simplified model extends to a model of the input, and every step replays.
#[test]
fn congruence_keeps_every_verdict_on_miters_of_different_circuits() {
    let mut next = rng(0x317E);
    let (mut sat, mut unsat) = (0usize, 0usize);
    for round in 0..300u64 {
        let inputs = 2 + (next() % 4) as usize;
        let gates = 3 + (next() % 6) as usize;
        let first = random_circuit(&mut next, inputs, gates);
        let mut second = first.clone();
        for _ in 0..next() % 3 {
            let g = (next() % gates as u64) as usize;
            second[g] = random_circuit(&mut next, inputs + g, 1).remove(0);
        }
        let (num_vars, clauses) = miter(&first, &second, inputs);
        let expected = cdcl_sat(num_vars, &clauses);
        let pre = preprocess(num_vars, &clauses, &Options::default());
        assert!(steps_replay(num_vars, &clauses, &pre.steps), "round {round}: the steps replay");
        if pre.unsat {
            assert!(!expected, "round {round}: a satisfiable miter refuted");
            unsat += 1;
            continue;
        }
        let mut s = Solver::new(num_vars);
        for c in &pre.clauses {
            s.add_clause(c.clone());
        }
        match s.solve() {
            SolveResult::Sat(mut model) => {
                assert!(expected, "round {round}: an unsatisfiable miter's simplification has a model");
                pre.extend(&mut model);
                assert!(satisfies(&clauses, &model), "round {round}: the model extends to the input");
                sat += 1;
            }
            SolveResult::Unsat => assert!(!expected, "round {round}: a satisfiable miter's simplification is refuted"),
        }
    }
    assert!(sat >= 30 && unsat >= 30, "both verdicts are exercised: {sat} satisfiable, {unsat} refuted");
}

/// Congruence closure costs the circuit's size: the miter of a 20,000-gate random circuit with itself — 40,000 gates
/// to hash, closed level by level — is refuted by preprocessing within a minute unoptimized on a loaded box.
#[test]
fn congruence_refutes_a_large_self_miter_in_time_linear_in_it() {
    let mut next = rng(0xB16);
    let inputs = 64;
    let circuit = random_circuit(&mut next, inputs, 20_000);
    let (num_vars, clauses) = miter(&circuit, &circuit, inputs);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(preprocess(num_vars, &clauses, &Options::default()).unsat);
    });
    assert!(rx.recv_timeout(std::time::Duration::from_secs(60)).expect("congruence costs the circuit's size"), "the self-miter is refuted");
}

/// A replay that honours deletions, as a DRAT checker does: each RUP addition checked against the clauses present
/// then, each PR addition by its witness, each deletion removing its clause.
fn steps_check(num_vars: usize, input: &[Vec<Lit>], steps: &[ProofStep]) -> bool {
    let key = |d: &[Lit]| {
        let mut k: Vec<usize> = d.iter().map(|l| l.index()).collect();
        k.sort_unstable();
        k.dedup();
        k
    };
    let mut db = input.to_vec();
    for step in steps {
        match step {
            ProofStep::Rup(c) => {
                if !crate::rup::is_rup(num_vars, &db, c) {
                    return false;
                }
                db.push(c.clone());
            }
            ProofStep::Pr { clause, witness } => {
                if !crate::pr::is_pr(num_vars, &db, clause, witness) {
                    return false;
                }
                db.push(clause.clone());
            }
            ProofStep::Delete(c) => {
                if let Some(at) = db.iter().position(|d| key(d) == key(c)) {
                    db.swap_remove(at);
                }
            }
        }
    }
    true
}

/// A clause is derived before the clause it replaces is deleted — a shortened clause before its parent, a rewritten
/// one before its original, a strengthened one before the clause it shortens — so the proof replays with its
/// deletions honoured, as a DRAT checker reads it, on every simplification and every combination of them.
#[test]
fn the_proof_replays_with_its_deletions_honoured() {
    let mut next = rng(0xDE1E);
    for round in 0..800u64 {
        let n = 2 + (next() % 9) as usize;
        let m = (next() % (5 * n as u64)) as usize;
        let clauses = random_formula(&mut next, n, m, 3);
        for mask in [7u8, 1, 2, 4] {
            let options = Options { eliminate: mask & 1 != 0, substitute: mask & 2 != 0, subsume: mask & 4 != 0, ..Options::default() };
            let pre = preprocess(n, &clauses, &options);
            assert!(steps_check(n, &clauses, &pre.steps), "round {round}, passes {mask:03b}: a step fails once deletions count");
        }
    }
    let mut next = rng(0xC1C2);
    for round in 0..60u64 {
        let inputs = 3 + (next() % 4) as usize;
        let gates = 4 + (next() % 8) as usize;
        let circuit = random_circuit(&mut next, inputs, gates);
        let (num_vars, clauses) = miter(&circuit, &circuit, inputs);
        let pre = preprocess(num_vars, &clauses, &Options::default());
        assert!(steps_check(num_vars, &clauses, &pre.steps), "round {round}: a congruence proof fails once deletions count");
    }
}

/// Whether an assignment of the input's variables extends, over the variables preprocessing added, to a model of the
/// simplified clauses — by trying every assignment of the added ones.
fn extends_to_simplified(model: &[bool], pre: &Preprocessed) -> bool {
    let input = model.len();
    let added = pre.num_vars - input;
    assert!(added <= 12, "{added} added variables is past an exhaustive check");
    (0u32..1 << added).any(|bits| {
        let full: Vec<bool> = model.iter().copied().chain((0..added).map(|k| bits >> k & 1 == 1)).collect();
        satisfies(&pre.clauses, &full)
    })
}

fn pairwise_at_most_one(vars: &[Var]) -> Vec<Vec<Lit>> {
    let mut clauses = Vec::new();
    for (i, &a) in vars.iter().enumerate() {
        for &b in &vars[i + 1..] {
            clauses.push(vec![Lit::neg(a), Lit::neg(b)]);
        }
    }
    clauses
}

fn factoring() -> Options {
    Options { factor: true, ..Options::default() }
}

/// Bounded variable addition compresses a pairwise at-most-one: its 45 binaries over ten variables become fewer
/// clauses over a few added variables, every step checking as RUP or PR, and an assignment of the ten satisfies the
/// input exactly when it extends to a model of the result.
#[test]
fn factoring_compresses_a_pairwise_at_most_one_and_keeps_its_models() {
    let vars: Vec<Var> = (0..10).collect();
    let mut clauses = pairwise_at_most_one(&vars);
    clauses.push(vars.iter().map(|&v| Lit::pos(v)).collect());
    let options = Options { eliminate: false, ..factoring() };
    let pre = preprocess(10, &clauses, &options);
    assert!(!pre.unsat);
    assert!(pre.factored >= 1, "the at-most-one is factored");
    assert!(pre.num_vars > 10, "factoring adds variables");
    assert!(pre.clauses.len() < clauses.len(), "{} clauses from {}", pre.clauses.len(), clauses.len());
    assert!(steps_check(pre.num_vars, &clauses, &pre.steps), "every step is RUP or PR against the clauses then present");
    for bits in 0u32..1 << 10 {
        let model: Vec<bool> = (0..10).map(|v| bits >> v & 1 == 1).collect();
        assert_eq!(extends_to_simplified(&model, &pre), satisfies(&clauses, &model), "assignment {bits:010b}");
    }
}

/// Factoring keeps every verdict on formulas with factorable products planted among random clauses: a model of the
/// result, extended, satisfies the input; every step checks; a default run adds no variable.
#[test]
fn factoring_keeps_every_verdict_and_every_model_extends() {
    let mut next = rng(0xFAC7);
    let (mut sat, mut unsat, mut factored) = (0usize, 0usize, 0usize);
    for round in 0..400u64 {
        let n = 8 + (next() % 8) as usize;
        let m = (next() % (2 * n as u64)) as usize;
        let mut clauses: Vec<Vec<Lit>> = (0..m).map(|_| (0..3).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect()).collect();
        // Products of three to five literals by three to six rests: each saves clauses when factored.
        for _ in 0..1 + next() % 2 {
            let left: Vec<Lit> = (0..3 + next() % 3).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect();
            let rests: Vec<Vec<Lit>> = (0..3 + next() % 4).map(|_| (0..1 + next() % 2).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect()).collect();
            for &l in &left {
                for r in &rests {
                    clauses.push(std::iter::once(l).chain(r.iter().copied()).collect());
                }
            }
        }
        let expected = cdcl_sat(n, &clauses);
        assert_eq!(preprocess(n, &clauses, &Options::default()).num_vars, n, "round {round}: a default run adds no variable");
        let pre = preprocess(n, &clauses, &factoring());
        factored += pre.factored;
        assert!(steps_check(pre.num_vars, &clauses, &pre.steps), "round {round}: the steps check");
        if pre.unsat {
            assert!(!expected, "round {round}: a satisfiable formula refuted");
            unsat += 1;
            continue;
        }
        let mut s = Solver::new(pre.num_vars);
        for c in &pre.clauses {
            s.add_clause(c.clone());
        }
        match s.solve() {
            SolveResult::Sat(mut model) => {
                assert!(expected, "round {round}: an unsatisfiable formula's simplification has a model");
                pre.extend(&mut model);
                assert!(satisfies(&clauses, &model[..n]), "round {round}: the model extends to the input");
                sat += 1;
            }
            SolveResult::Unsat => assert!(!expected, "round {round}: a satisfiable formula's simplification refuted"),
        }
    }
    assert!(sat >= 50 && unsat >= 30 && factored >= 100, "{sat} satisfiable, {unsat} refuted, {factored} factorings");
}

/// Factoring a pairwise at-most-one over 400 variables — 79,800 binaries — costs its occurrences, not their square,
/// and leaves a few thousand clauses, within a minute unoptimized on a loaded box.
#[test]
fn factoring_a_wide_at_most_one_costs_its_occurrences() {
    let vars: Vec<Var> = (0..400).collect();
    let clauses = pairwise_at_most_one(&vars);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(preprocess(400, &clauses, &Options { eliminate: false, ..factoring() }));
    });
    let pre = rx.recv_timeout(std::time::Duration::from_secs(60)).expect("factoring costs its occurrences");
    assert!(pre.clauses.len() < 8000, "{} clauses remain of 79,800", pre.clauses.len());
}

#[test]
fn elimination_past_a_positive_bound_keeps_every_model_extensible_and_the_proof_replaying() {
    // Random 3-SAT near the threshold, where an elimination adding a few clauses is common and bound 0 forbids it.
    let mut next = rng(0xB0D4);
    let (mut grew, mut sat, mut unsat) = (0, 0, 0);
    for round in 0..40u64 {
        let n = 60 + (next() % 140) as usize;
        let input: Vec<Vec<Lit>> = (0..(n as f64 * 4.2) as usize)
            .map(|_| (0..3).map(|_| Lit::new((next() % n as u64) as Var, next() % 2 == 0)).collect())
            .collect();
        let input_sat = cdcl_sat(n, &input);
        let zero = preprocess(n, &input, &Options::default());
        for bound in [4usize, 16] {
            let p = preprocess(n, &input, &Options { bound, ..Options::default() });
            grew += p.eliminated.saturating_sub(zero.eliminated);
            assert!(steps_replay(n, &input, &p.steps), "round {round} bound {bound}: a step does not replay");
            if p.unsat {
                assert!(!input_sat, "round {round} bound {bound}: a satisfiable input called unsatisfiable");
                continue;
            }
            let mut solver = crate::cdcl::Solver::new(p.num_vars);
            for c in &p.clauses {
                solver.add_clause(c.clone());
            }
            match solver.solve() {
                crate::cdcl::SolveResult::Sat(mut model) => {
                    assert!(input_sat, "round {round} bound {bound}: the simplified formula is satisfiable, the input not");
                    p.extend(&mut model);
                    assert!(satisfies(&input, &model), "round {round} bound {bound}: an extended model fails the input");
                    sat += 1;
                }
                crate::cdcl::SolveResult::Unsat => {
                    assert!(!input_sat, "round {round} bound {bound}: satisfiability changed");
                    unsat += 1;
                }
            }
        }
    }
    assert!(grew > 100 && sat > 10 && unsat > 10, "a positive bound eliminates past bound 0's ({grew} more; {sat} SAT, {unsat} UNSAT)");
}

/// The Tseitin clauses of `gates` random AND gates over `inputs` inputs, each gate over two earlier signals of random
/// polarity, numbered after the inputs.
fn and_circuit(next: &mut impl FnMut() -> u64, inputs: usize, gates: usize) -> Vec<Vec<Lit>> {
    let mut clauses = Vec::new();
    for g in inputs..inputs + gates {
        let a = Lit::new((next() % g as u64) as Var, next() & 1 == 0);
        let b = Lit::new((next() % g as u64) as Var, next() & 1 == 0);
        let o = Lit::pos(g as Var);
        clauses.push(vec![o.negated(), a]);
        clauses.push(vec![o.negated(), b]);
        clauses.push(vec![o, a.negated(), b.negated()]);
    }
    clauses
}

fn counting() -> Options {
    Options { defined_only: true, renew_costs: true, ..Options::default() }
}

#[test]
fn eliminating_only_defined_variables_keeps_every_model() {
    // Every model of the simplified formula over the variables preprocessing left unsettled extends to a model of the
    // input keeping their values, and distinct ones to distinct models; when they reach as many models as the input
    // has, they reach them all, and the two formulas have one count. Circuits of AND gates, some outputs and random
    // clauses over all their variables constrained, under both elimination schedules.
    let mut next = rng(0xDEF1);
    let mut eliminated = 0;
    for round in 0..300u64 {
        let inputs = 2 + (next() % 5) as usize;
        let gates = 1 + (next() % (12 - inputs) as u64) as usize;
        let n = inputs + gates;
        let mut clauses = and_circuit(&mut next, inputs, gates);
        let extra = (next() % 4) as usize;
        clauses.extend(random_formula(&mut next, n, extra, 3));
        let input = models(n, &clauses);
        for renew_costs in [false, true] {
            let p = preprocess(n, &clauses, &Options { renew_costs, ..counting() });
            if p.unsat {
                assert!(input.is_empty(), "round {round}: refuted a formula with {} models", input.len());
                continue;
            }
            assert!(
                p.clauses.iter().flatten().all(|l| !p.settled[l.var() as usize]),
                "round {round}: a simplified clause holds a settled variable"
            );
            let kept: Vec<usize> = (0..n).filter(|&v| !p.settled[v]).collect();
            let mut reached = std::collections::HashSet::new();
            for bits in 0..1u32 << kept.len() {
                let mut model = vec![false; n];
                for (k, &v) in kept.iter().enumerate() {
                    model[v] = bits >> k & 1 == 1;
                }
                if !satisfies(&p.clauses, &model) {
                    continue;
                }
                p.extend(&mut model);
                assert!(satisfies(&clauses, &model), "round {round}: an extended model fails the input");
                assert!(kept.iter().enumerate().all(|(k, &v)| model[v] == (bits >> k & 1 == 1)), "round {round}");
                reached.insert(model);
            }
            assert_eq!(reached.len(), input.len(), "round {round} (renewed costs {renew_costs}): the counts differ");
            eliminated += p.eliminated;
        }
    }
    assert!(eliminated >= 1000, "the rounds eliminate defined variables ({eliminated})");
}

#[test]
fn a_variable_the_others_do_not_decide_is_kept() {
    // (x ∨ y): x is free once y holds, so no elimination of x keeps the formula's three models; bounded elimination
    // takes it, pure, and leaves y's two.
    let clauses = vec![vec![Lit::pos(0), Lit::pos(1)]];
    let kept = preprocess(2, &clauses, &counting());
    assert_eq!((kept.eliminated, kept.settled.clone()), (0, vec![false, false]));
    assert_eq!(kept.clauses, clauses);
    let plain = preprocess(2, &clauses, &Options::default());
    assert_eq!(plain.eliminated, 1, "bounded elimination takes the pure variable");
}

#[test]
fn a_definition_no_gate_shows_is_found_by_the_local_check() {
    // What a circuit left of a gate once the gates around it went. Both values of v5 fit an assignment of the others
    // only where one of v1, v2, v3 holds (its negative clauses), which forces v0 and then v4 (its positive ones), and
    // v4 refutes it: the others decide v5, though its clauses form no AND and no XOR.
    let clauses: Vec<Vec<Lit>> = [
        vec![-1, 5, -6],
        vec![1, -2, 6],
        vec![1, -3, 6],
        vec![1, -4, 6],
        vec![2, 3, 4, -6],
        vec![-2, -5, 6],
        vec![-3, -5, 6],
        vec![-4, -5, 6],
    ]
    .iter()
    .map(|c| c.iter().map(|&d: &i32| Lit::new(d.unsigned_abs() - 1, d > 0)).collect())
    .collect();
    let views: Vec<&[Lit]> = clauses.iter().map(Vec::as_slice).collect();
    assert_eq!(definition(&views, 5), None, "no gate defines v5");
    let p = preprocess(6, &clauses, &counting());
    assert!(p.settled[5], "the local check finds v5 defined: {:?}", p.settled);
    assert_eq!(models(6, &clauses).len(), {
        let kept: Vec<usize> = (0..6).filter(|&v| !p.settled[v]).collect();
        (0..1u32 << kept.len())
            .filter(|bits| {
                let mut model = vec![false; 6];
                for (k, &v) in kept.iter().enumerate() {
                    model[v] = bits >> k & 1 == 1;
                }
                satisfies(&p.clauses, &model)
            })
            .count()
    });
}

#[test]
fn renewed_costs_take_a_circuit_from_its_outputs_to_its_inputs() {
    // A circuit with no constraint has a model for every assignment of its inputs: eliminating each output once its
    // last reader has gone adds no clause, and the gates all go, the inputs free.
    for seed in 0..4u64 {
        let mut next = rng(seed);
        let (inputs, gates) = (30usize, 400usize);
        let clauses = and_circuit(&mut next, inputs, gates);
        let p = preprocess(inputs + gates, &clauses, &counting());
        assert!(!p.unsat);
        assert!(p.clauses.is_empty(), "seed {seed}: {} clauses left", p.clauses.len());
        assert_eq!(p.settled.iter().filter(|&&s| s).count(), gates, "seed {seed}: every gate settled, no input");
    }
}
