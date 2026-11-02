use super::*;

#[test]
#[ignore = "bench: random-3SAT solve throughput — before/after engine tuning"]
fn bench_random_3sat_throughput() {
    use std::time::Instant;
    let mut total_ms = 0.0;
    let mut conflicts = 0u64;
    let mut props = 0u64;
    for seed in 0..24u64 {
        let n = 150;
        let m = (n as f64 * 4.26) as usize;
        let cnf = crate::families::random_3sat(n, m, 0xBEEFu64 ^ seed);
        let mut s = Solver::new(cnf.num_vars);
        for c in &cnf.clauses {
            s.add_clause(c.clone());
        }
        let t = Instant::now();
        let _ = s.solve();
        total_ms += t.elapsed().as_secs_f64() * 1e3;
        conflicts += s.conflicts();
        props += s.propagations();
    }
    eprintln!(
        "[bench] random-3SAT ×24 @ n=150: {total_ms:.1}ms total ({:.3}ms/inst), {conflicts} conflicts, {props} propagations",
        total_ms / 24.0
    );
}

#[test]
fn counters_track_search_work() {
    // All 8 clauses over 3 vars (every assignment blocked) → UNSAT, forcing real search:
    // decisions, propagations, and conflicts must all register, and propagations (one per
    // trail literal processed) must dominate conflicts.
    let mut s = Solver::new(3);
    for mask in 0..8u32 {
        let c: Vec<Lit> = (0..3).map(|v| Lit::new(v, (mask >> v) & 1 == 0)).collect();
        s.add_clause(c);
    }
    assert_eq!(s.solve(), SolveResult::Unsat);
    assert!(s.conflicts() > 0, "expected conflicts, got 0");
    assert!(s.decisions() > 0, "expected decisions, got 0");
    assert!(
        s.propagations() >= s.conflicts(),
        "propagations should dominate conflicts"
    );
}

#[test]
fn order_heap_pops_in_descending_activity() {
    let mut s = Solver::new(5);
    for _ in 0..5 {
        s.bump(4); // activity[4] = 5
    }
    for _ in 0..3 {
        s.bump(2); // activity[2] = 3
    }
    s.bump(0); // activity[0] = 1; vars 1,3 stay at 0
    let mut order = Vec::new();
    while let Some(v) = s.heap_pop() {
        order.push(v);
    }
    assert_eq!(&order[..3], &[4, 2, 0], "highest activity first");
    let mut rest = order[3..].to_vec();
    rest.sort();
    assert_eq!(rest, vec![1, 3], "zero-activity vars come last");
}

fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    // Enumerate all 2^n assignments; true iff some assignment satisfies every clause.
    for mask in 0u64..(1u64 << num_vars) {
        let val = |v: Var| (mask >> v) & 1 == 1;
        let ok = clauses.iter().all(|c| {
            c.iter().any(|l| {
                let b = val(l.var());
                if l.is_positive() {
                    b
                } else {
                    !b
                }
            })
        });
        if ok {
            return true;
        }
    }
    false
}

fn check_model(clauses: &[Vec<Lit>], model: &[bool]) -> bool {
    clauses.iter().all(|c| {
        c.iter().any(|l| {
            let b = model[l.var() as usize];
            if l.is_positive() {
                b
            } else {
                !b
            }
        })
    })
}

#[test]
fn unit_and_empty() {
    // Empty clause ⇒ Unsat.
    let mut s = Solver::new(1);
    s.add_clause(vec![]);
    assert_eq!(s.solve(), SolveResult::Unsat);

    // x ∧ ¬x ⇒ Unsat.
    let mut s = Solver::new(1);
    s.add_clause(vec![Lit::pos(0)]);
    s.add_clause(vec![Lit::neg(0)]);
    assert_eq!(s.solve(), SolveResult::Unsat);
}

#[test]
fn tiny_sat() {
    // (x ∨ y) ∧ (¬x ∨ y) ∧ (¬y ∨ z): forces y, then z; x free.
    let mut s = Solver::new(3);
    s.add_clause(vec![Lit::pos(0), Lit::pos(1)]);
    s.add_clause(vec![Lit::neg(0), Lit::pos(1)]);
    s.add_clause(vec![Lit::neg(1), Lit::pos(2)]);
    match s.solve() {
        SolveResult::Sat(m) => {
            assert!(m[1] && m[2], "y and z forced true");
        }
        SolveResult::Unsat => panic!("should be SAT"),
    }
}

#[test]
fn pigeonhole_3_into_2_unsat() {
    // 3 pigeons, 2 holes: p_{i,h} = pigeon i in hole h. Each pigeon in some hole; no
    // two pigeons share a hole. Classic UNSAT (PHP) — exercises conflict learning.
    let var = |i: usize, h: usize| (i * 2 + h) as Var;
    let mut s = Solver::new(6);
    for i in 0..3 {
        s.add_clause(vec![Lit::pos(var(i, 0)), Lit::pos(var(i, 1))]);
    }
    for h in 0..2 {
        for i in 0..3 {
            for j in (i + 1)..3 {
                s.add_clause(vec![Lit::neg(var(i, h)), Lit::neg(var(j, h))]);
            }
        }
    }
    assert_eq!(s.solve(), SolveResult::Unsat);
}

#[test]
fn random_against_brute_force() {
    // Deterministic pseudo-random 3-CNFs over up to 6 vars; cross-check SAT/UNSAT and
    // validate every returned model. The only honest way to trust a SAT core.
    let mut state = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _trial in 0..400 {
        let num_vars = 3 + (next() % 4) as usize; // 3..6
        let num_clauses = 3 + (next() % 12) as usize;
        let mut clauses = Vec::new();
        for _ in 0..num_clauses {
            let mut c = Vec::new();
            for _ in 0..3 {
                let v = (next() % num_vars as u64) as Var;
                let positive = next() & 1 == 0;
                c.push(Lit::new(v, positive));
            }
            clauses.push(c);
        }
        let expected = sat_brute(num_vars, &clauses);
        let mut s = Solver::new(num_vars);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        match s.solve() {
            SolveResult::Sat(m) => {
                assert!(expected, "solver said SAT but brute force says UNSAT");
                assert!(
                    check_model(&clauses, &m),
                    "returned model does not satisfy the formula"
                );
            }
            SolveResult::Unsat => {
                assert!(!expected, "solver said UNSAT but brute force found a model");
            }
        }
    }
}

#[test]
fn reduction_preserves_verdicts_and_models() {
    // LBD clause deletion must be verdict-invariant. With reduction forced after every few
    // learned clauses — so the delete + reason-remap + watch-rebuild path runs constantly —
    // every verdict must still match brute force and every model must satisfy the formula.
    // The strongest guard against a reduceDB bug.
    let mut state = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _trial in 0..500 {
        let num_vars = 3 + (next() % 5) as usize; // 3..7
        let num_clauses = 10 + (next() % 25) as usize; // over-constrained → conflicts → reductions
        let mut clauses = Vec::new();
        for _ in 0..num_clauses {
            let mut c = Vec::new();
            for _ in 0..3 {
                c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
            }
            clauses.push(c);
        }
        let expected = sat_brute(num_vars, &clauses);
        let mut s = Solver::new(num_vars);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        s.set_reduce_limit(4); // hammer the reduction path
        match s.solve() {
            SolveResult::Sat(m) => {
                assert!(
                    expected,
                    "reduce-on solver said SAT but brute force says UNSAT"
                );
                assert!(check_model(&clauses, &m), "model invalid under reduction");
            }
            SolveResult::Unsat => {
                assert!(!expected, "reduce-on solver said UNSAT but a model exists");
            }
        }
    }
}

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    }
}

#[test]
fn restart_modes_are_all_verdict_invariant_default_adaptive() {
    // The adaptive (alternating Glucose/Luby phase) policy is the default, and switching restart
    // heuristics changes only search ORDER — every verdict and model must still agree with brute
    // force under ALL three policies.
    assert_eq!(
        Solver::new(1).restart_mode(),
        RestartMode::Adaptive,
        "adaptive restarts are the default"
    );
    let mut next = rng(0x51ed_2701_a1b2_c3d4);
    for _ in 0..400 {
        let num_vars = 3 + (next() % 5) as usize; // 3..7
        let num_clauses = 8 + (next() % 22) as usize;
        let mut clauses = Vec::new();
        for _ in 0..num_clauses {
            let mut c = Vec::new();
            for _ in 0..3 {
                c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
            }
            clauses.push(c);
        }
        let expected = sat_brute(num_vars, &clauses);
        for mode in [
            RestartMode::Adaptive,
            RestartMode::Glucose,
            RestartMode::Luby,
        ] {
            let mut s = Solver::new(num_vars);
            s.set_restart_mode(mode);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            match s.solve() {
                SolveResult::Sat(m) => {
                    assert!(expected, "{mode:?}: SAT but brute force UNSAT");
                    assert!(check_model(&clauses, &m), "{mode:?}: invalid model");
                }
                SolveResult::Unsat => assert!(!expected, "{mode:?}: UNSAT but SAT"),
            }
        }
    }
}

fn php_solver(n: usize, holes: usize, mode: RestartMode) -> Solver {
    let var = |p: usize, h: usize| (p * holes + h) as Var;
    let mut s = Solver::new(n * holes);
    s.set_restart_mode(mode);
    for p in 0..n {
        s.add_clause((0..holes).map(|h| Lit::pos(var(p, h))).collect());
    }
    for h in 0..holes {
        for i in 0..n {
            for j in (i + 1)..n {
                s.add_clause(vec![Lit::neg(var(i, h)), Lit::neg(var(j, h))]);
            }
        }
    }
    s
}

#[test]
fn glucose_restarts_fire_and_beat_luby_on_pigeonhole() {
    // PHP(8→7) does real exponential-resolution work, and on it the dynamic LBD policy must
    // (a) actually restart — proving the mechanism is wired, not dormant — and (b) need no
    // more conflicts than the Luby baseline (here it roughly halves them). The blocking
    // counter is read to exercise its accounting. Verdicts are checked in the differential
    // test above; this one is about the restart *heuristic* paying off.
    let mut g = php_solver(8, 7, RestartMode::Glucose);
    assert_eq!(g.solve(), SolveResult::Unsat);
    assert!(
        g.restarts() > 0,
        "Glucose must restart on PHP(8); got {}",
        g.restarts()
    );
    let _ = g.blocked_restarts();

    let mut l = php_solver(8, 7, RestartMode::Luby);
    assert_eq!(l.solve(), SolveResult::Unsat);
    assert!(
        g.conflicts() <= l.conflicts(),
        "Glucose ({} conflicts) should not exceed Luby ({} conflicts) on PHP(8)",
        g.conflicts(),
        l.conflicts(),
    );
}

#[test]
fn vivify_preserves_verdicts() {
    // Interleave a vivification round into solving: learn a few clauses with a tiny budget,
    // strengthen them, then solve to completion — the verdict and any model must still match
    // brute force on the ORIGINAL formula. Vivify replaces a learned clause with an implied
    // sub-clause, so it can never change satisfiability. (That it *fires* is proven on
    // pigeonhole below, where instances are big enough to learn strengthenable clauses.)
    let mut next = rng(0x7654_3210_fedc_ba98);
    for _ in 0..600 {
        let num_vars = 3 + (next() % 5) as usize; // 3..7
        let num_clauses = 10 + (next() % 22) as usize; // over-constrained → conflicts
        let mut clauses = Vec::new();
        for _ in 0..num_clauses {
            let width = 2 + (next() % 2) as usize;
            let mut c = Vec::new();
            for _ in 0..width {
                c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
            }
            clauses.push(c);
        }
        let expected = sat_brute(num_vars, &clauses);
        let mut s = Solver::new(num_vars);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        // Vivify only from a clean budget-exhausted state (the supported inprocessing point,
        // mirroring the scheduler). A terminal verdict from the budgeted call is just checked.
        match s.solve_budgeted(12) {
            BudgetedResult::Sat(m) => {
                assert!(expected, "budgeted SAT but brute force UNSAT");
                assert!(check_model(&clauses, &m), "budgeted model invalid");
            }
            BudgetedResult::Unsat => assert!(!expected, "budgeted UNSAT but a model exists"),
            BudgetedResult::Budget => {
                if !s.vivify() {
                    assert!(!expected, "vivify reported UNSAT but a model exists");
                    continue;
                }
                match s.solve() {
                    SolveResult::Sat(m) => {
                        assert!(expected, "post-vivify SAT but brute force UNSAT");
                        assert!(check_model(&clauses, &m), "post-vivify model invalid");
                    }
                    SolveResult::Unsat => {
                        assert!(!expected, "post-vivify UNSAT but a model exists")
                    }
                }
            }
        }
    }
}

#[test]
fn vivify_fires_and_preserves_verdict_on_pigeonhole() {
    // PHP is big enough to learn strengthenable clauses, so vivification must actually fire
    // (not be a dormant no-op) AND keep the UNSAT verdict. PHP(6→5) yields ~16 strengthenings.
    let mut s = php_solver(6, 5, RestartMode::Glucose);
    assert_eq!(
        s.solve_budgeted(30),
        BudgetedResult::Budget,
        "budget should exhaust mid-search"
    );
    assert!(s.vivify(), "vivify should not yet prove UNSAT");
    assert!(
        s.vivifications() > 0,
        "vivification must fire on PHP — the pass is dormant"
    );
    assert_eq!(
        s.solve(),
        SolveResult::Unsat,
        "UNSAT preserved through vivification"
    );
}

#[test]
fn vivify_on_a_budgeted_prefix_preserves_pigeonhole_unsat() {
    // PHP(5→4) needs real search, so a small conflict budget exhausts cleanly — the supported
    // inprocessing state. Vivifying the learned clauses and finishing the solve must still
    // prove UNSAT (and the budgeted prefix must not regress to a bogus SAT).
    let mut s = php_solver(5, 4, RestartMode::Glucose);
    match s.solve_budgeted(5) {
        BudgetedResult::Unsat => {} // already proven within budget — still correct
        BudgetedResult::Sat(_) => panic!("PHP(5) is UNSAT"),
        BudgetedResult::Budget => {
            assert!(s.vivify(), "vivify should not yet prove UNSAT");
            assert_eq!(
                s.solve(),
                SolveResult::Unsat,
                "UNSAT preserved through vivification"
            );
        }
    }
}

#[test]
fn probe_preserves_verdicts() {
    // Failed-literal probing derives only units that the formula entails, so running a probing
    // round before the solve must never change satisfiability or invalidate a model. Checked
    // against brute force on many random formulas.
    let mut next = rng(0x1c1c_2d2d_3e3e_4f4f);
    for _ in 0..600 {
        let num_vars = 3 + (next() % 5) as usize; // 3..7
        let num_clauses = 6 + (next() % 18) as usize;
        let mut clauses = Vec::new();
        for _ in 0..num_clauses {
            let width = 2 + (next() % 2) as usize;
            let mut c = Vec::new();
            for _ in 0..width {
                c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
            }
            clauses.push(c);
        }
        let expected = sat_brute(num_vars, &clauses);
        let mut s = Solver::new(num_vars);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        if !s.probe() {
            assert!(!expected, "probe reported UNSAT but a model exists");
            continue;
        }
        match s.solve() {
            SolveResult::Sat(m) => {
                assert!(expected, "post-probe SAT but brute force UNSAT");
                assert!(check_model(&clauses, &m), "post-probe model invalid");
            }
            SolveResult::Unsat => assert!(!expected, "post-probe UNSAT but a model exists"),
        }
    }
}

#[test]
fn probe_derives_a_failed_literal_unit() {
    // (¬v ∨ a) ∧ (¬v ∨ ¬a): assuming v=true forces a AND ¬a — a conflict — so probing must
    // derive the unit ¬v. The formula is SAT (v=false), and the post-probe model must reflect
    // the forced ¬v.
    let (v, a) = (0u32, 1u32);
    let clauses = vec![
        vec![Lit::neg(v), Lit::pos(a)],
        vec![Lit::neg(v), Lit::neg(a)],
    ];
    let mut s = Solver::new(2);
    for c in &clauses {
        s.add_clause(c.clone());
    }
    assert!(s.probe(), "probing should not prove this SAT formula UNSAT");
    assert!(
        s.probes() > 0,
        "probing must derive the failed-literal unit ¬v"
    );
    match s.solve() {
        SolveResult::Sat(m) => {
            assert!(!m[v as usize], "¬v was forced by probing");
            assert!(check_model(&clauses, &m));
        }
        SolveResult::Unsat => panic!("formula is satisfiable (v = false)"),
    }
}

#[test]
fn inprocessing_engages_inside_solve_and_improves_search() {
    // PHP(9→8) crosses the inprocessing interval many times (at the tuned-down test interval),
    // so the scheduler must (a) actually fire inside solve() — observable via the counters —
    // and (b) cut the conflict count versus the same solve with inprocessing disabled. The
    // verdict is preserved both ways. (On much larger PHP instances the win grows to ~2×.)
    let mut on = php_solver(9, 8, RestartMode::Glucose);
    on.set_inprocess_interval(400);
    assert_eq!(on.solve(), SolveResult::Unsat);
    assert!(
        on.vivifications() > 0,
        "inprocessing must engage on a long solve; vivifications={}",
        on.vivifications(),
    );

    let mut off = php_solver(9, 8, RestartMode::Glucose);
    off.set_inprocess(false);
    assert_eq!(off.solve(), SolveResult::Unsat);
    assert_eq!(
        off.vivifications() + off.probes(),
        0,
        "toggle must suppress inprocessing"
    );
    assert!(
        on.conflicts() < off.conflicts(),
        "inprocessing should cut conflicts: on={} off={}",
        on.conflicts(),
        off.conflicts(),
    );
}

#[test]
fn self_subsumes_classifies_subsumption_and_ssr() {
    // Direct test of the subsumption classifier on (var,sign) codes: 2v = +v, 2v+1 = ¬v.
    let pos = |v: u32| 2 * v;
    let neg = |v: u32| 2 * v + 1;
    let sorted = |mut x: Vec<u32>| {
        x.sort_unstable();
        x
    };
    let c = sorted(vec![pos(0), pos(1)]); // {a, b}
                                          // {a,b} ⊆ {a,b,c} → subsumes.
    assert!(matches!(
        self_subsumes(&c, &sorted(vec![pos(0), pos(1), pos(2)])),
        Sub::Subsumes
    ));
    // {a,b} vs {¬a,b,c}: a flips → strengthen, dropping ¬a from D.
    assert!(matches!(
        self_subsumes(&c, &sorted(vec![neg(0), pos(1), pos(2)])),
        Sub::Strengthen(p) if p == neg(0)
    ));
    // {a,d} vs {¬a,b,c}: d absent in either polarity → no relation.
    assert!(matches!(
        self_subsumes(
            &sorted(vec![pos(0), pos(3)]),
            &sorted(vec![neg(0), pos(1), pos(2)])
        ),
        Sub::No
    ));
    // {a,b} vs {¬a,¬b,c}: two flips → no relation (resolution would not subsume).
    assert!(matches!(
        self_subsumes(&c, &sorted(vec![neg(0), neg(1), pos(2)])),
        Sub::No
    ));
}

#[test]
fn subsume_preserves_verdicts() {
    // Subsumption deletes only entailed learned clauses and SSR strengthens to sound resolvents,
    // so a subsumption round before finishing the solve must never change the verdict or
    // invalidate a model. Checked against brute force. (Firing at scale is covered by the
    // arena measurement; here soundness is the point.)
    let mut next = rng(0xa5a5_5a5a_c3c3_3c3c);
    for _ in 0..600 {
        let num_vars = 3 + (next() % 5) as usize;
        let num_clauses = 10 + (next() % 22) as usize;
        let mut clauses = Vec::new();
        for _ in 0..num_clauses {
            let width = 2 + (next() % 2) as usize;
            let mut c = Vec::new();
            for _ in 0..width {
                c.push(Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0));
            }
            clauses.push(c);
        }
        let expected = sat_brute(num_vars, &clauses);
        let mut s = Solver::new(num_vars);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        match s.solve_budgeted(12) {
            BudgetedResult::Sat(m) => {
                assert!(expected);
                assert!(check_model(&clauses, &m));
            }
            BudgetedResult::Unsat => assert!(!expected),
            BudgetedResult::Budget => {
                if !s.subsume() {
                    assert!(!expected, "subsume reported UNSAT but a model exists");
                    continue;
                }
                match s.solve() {
                    SolveResult::Sat(m) => {
                        assert!(expected, "post-subsume SAT but brute force UNSAT");
                        assert!(check_model(&clauses, &m), "post-subsume model invalid");
                    }
                    SolveResult::Unsat => {
                        assert!(!expected, "post-subsume UNSAT but a model exists")
                    }
                }
            }
        }
    }
}

#[test]
fn solve_under_assumptions_matches_brute_force() {
    // Incremental SAT, validated against the oracle: for each random formula, fire MANY
    // assumption queries at the SAME solver (so it accumulates learned clauses), and
    // demand every verdict + model agree with brute force on `clauses ∧ assumptions`.
    // Reusing the solver is the whole point — it proves learned-clause reuse across
    // different assumption sets stays sound.
    let mut state = 0x243f_6a88_85a3_08d3u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _trial in 0..300 {
        let num_vars = 3 + (next() % 4) as usize; // 3..6
        let num_clauses = 3 + (next() % 10) as usize;
        let mut clauses = Vec::new();
        for _ in 0..num_clauses {
            let width = 2 + (next() % 2) as usize; // 2- or 3-literal
            let mut c = Vec::new();
            for _ in 0..width {
                let v = (next() % num_vars as u64) as Var;
                let positive = next() & 1 == 0;
                c.push(Lit::new(v, positive));
            }
            clauses.push(c);
        }
        let mut s = Solver::new(num_vars);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        // Several assumption queries on this one (clause-accumulating) solver.
        for _ in 0..8 {
            let a_count = (next() % 3) as usize; // 0..2 assumptions (may contradict)
            let mut asm = Vec::new();
            for _ in 0..a_count {
                let v = (next() % num_vars as u64) as Var;
                let positive = next() & 1 == 0;
                asm.push(Lit::new(v, positive));
            }
            let mut full = clauses.clone();
            for &a in &asm {
                full.push(vec![a]);
            }
            let expected = sat_brute(num_vars, &full);
            match s.solve_under_assumptions(&asm) {
                SolveResult::Sat(m) => {
                    assert!(expected, "under {asm:?}: solver SAT but brute force UNSAT");
                    assert!(
                        check_model(&full, &m),
                        "under {asm:?}: model violates clauses or assumptions"
                    );
                }
                SolveResult::Unsat => {
                    assert!(!expected, "under {asm:?}: solver UNSAT but brute force SAT");
                }
            }
        }
    }
}

/// Pigeonhole `holes + 1` into `holes`: hard for resolution, so a small budget runs out on it.
fn php(holes: u32) -> (usize, Vec<Vec<Lit>>) {
    let var = |i: u32, h: u32| i * holes + h;
    let mut cnf: Vec<Vec<Lit>> = (0..=holes).map(|i| (0..holes).map(|h| Lit::pos(var(i, h))).collect()).collect();
    for h in 0..holes {
        for i in 0..=holes {
            for j in i + 1..=holes {
                cnf.push(vec![Lit::neg(var(i, h)), Lit::neg(var(j, h))]);
            }
        }
    }
    (((holes + 1) * holes) as usize, cnf)
}

#[test]
fn an_assumption_solve_stops_at_its_budget_and_goes_on_after_it() {
    let (vars, cnf) = php(7);
    let mut s = Solver::new(vars);
    for c in &cnf {
        s.add_clause(c.clone());
    }
    assert_eq!(s.solve_under_assumptions_budgeted(&[], 50), BudgetedResult::Budget);
    assert!(s.conflicts() >= 50, "the conflicts are counted: {}", s.conflicts());
    let spent = s.conflicts();
    assert_eq!(s.solve_under_assumptions_budgeted(&[], 0), BudgetedResult::Unsat, "0 is no budget at all");
    assert!(s.conflicts() > spent);
    let mut easy = Solver::new(2);
    easy.add_clause(vec![Lit::pos(0), Lit::pos(1)]);
    match easy.solve_under_assumptions_budgeted(&[Lit::neg(0)], 1) {
        BudgetedResult::Sat(m) => assert!(!m[0] && m[1]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_assumption_unsat_leaves_the_negated_assumptions_rup_after_the_learned_clauses() {
    // One solver, many queries, permanent clauses added between them: at each UNSAT, the clause the assumptions'
    // negation makes is derivable from the formula so far and every clause learned so far, in order.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _trial in 0..60 {
        let num_vars = 6;
        let mut s = Solver::new(num_vars);
        s.set_reduce_limit(4);
        let mut formula: Vec<Vec<Lit>> = Vec::new();
        let mut log: Vec<Vec<Lit>> = Vec::new();
        let mut synced = 0;
        for _ in 0..12 {
            let c: Vec<Lit> = (0..3).map(|_| Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0)).collect();
            s.add_clause(c.clone());
            formula.push(c);
        }
        for _ in 0..10 {
            let asm: Vec<Lit> = (0..2).map(|_| Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0)).collect();
            let mut full = formula.clone();
            full.extend(asm.iter().map(|&a| vec![a]));
            let expected = sat_brute(num_vars, &full);
            let verdict = s.solve_under_assumptions_budgeted(&asm, 0);
            log.extend(s.learned()[synced..].iter().map(|c| c.lits.clone()));
            synced = s.learned().len();
            match verdict {
                BudgetedResult::Sat(m) => {
                    assert!(expected, "SAT where brute force is not");
                    assert!(check_model(&full, &m));
                }
                BudgetedResult::Unsat => {
                    assert!(!expected, "UNSAT where brute force has a model");
                    let negated: Vec<Lit> = asm.iter().map(|a| a.negated()).collect();
                    log.push(negated.clone());
                    let target = log.len() - 1;
                    let proof = crate::lrat::derivation(num_vars, &formula, &log, &[target]).expect("the negated assumptions are RUP");
                    assert!(crate::lrat::check_derivation(num_vars, &formula, &proof, &[negated]));
                    // The lemma joins the formula as a permanent clause, as the sweep adds its lemmas.
                    s.add_clause(log[target].clone());
                }
                BudgetedResult::Budget => panic!("no budget was set"),
            }
        }
    }
}

#[test]
fn a_clause_added_after_a_solve_is_watched_where_it_can_still_propagate() {
    // x0 is fixed false at level 0 by a unit; (x0 ∨ x1) added after a solve is then a unit on x1, and (x0 ∨ ¬x1 ∨ x2)
    // with x1 true a unit on x2: a model must honour both, however the literals were listed.
    let mut s = Solver::new(3);
    s.add_clause(vec![Lit::neg(0)]);
    assert!(matches!(s.solve_under_assumptions_budgeted(&[Lit::neg(2)], 0), BudgetedResult::Sat(_)));
    s.add_clause(vec![Lit::pos(0), Lit::pos(1)]);
    s.add_clause(vec![Lit::pos(0), Lit::neg(1), Lit::pos(2)]);
    match s.solve_under_assumptions_budgeted(&[], 0) {
        BudgetedResult::Sat(m) => assert!(!m[0] && m[1] && m[2], "{m:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.solve_under_assumptions_budgeted(&[Lit::neg(2)], 0), BudgetedResult::Unsat, "¬x2 now contradicts");
    // Every literal false at level 0: the formula is unsatisfiable from then on.
    s.add_clause(vec![Lit::pos(0), Lit::neg(1)]);
    assert_eq!(s.solve_under_assumptions_budgeted(&[], 0), BudgetedResult::Unsat);
}

#[test]
fn decision_variables_join_one_by_one_and_only_they_are_decided() {
    // x0 ∨ x1 over three variables, x2 never a decision: a model leaves it unassigned (false), and once x2 joins and a
    // clause needs it the solver decides it too.
    let mut s = Solver::new(3);
    s.set_decision_vars(&[]);
    s.add_clause(vec![Lit::pos(0), Lit::pos(1)]);
    s.add_decision_vars(&[0, 1]);
    match s.solve_under_assumptions_budgeted(&[], 0) {
        BudgetedResult::Sat(m) => assert!((m[0] || m[1]) && !m[2], "{m:?}"),
        other => panic!("{other:?}"),
    }
    s.add_clause(vec![Lit::pos(2), Lit::neg(0)]);
    s.add_clause(vec![Lit::pos(2), Lit::neg(1)]);
    s.add_decision_vars(&[2, 2, 7]);
    match s.solve_under_assumptions_budgeted(&[], 0) {
        BudgetedResult::Sat(m) => assert!((m[0] || m[1]) && m[2], "{m:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.solve_under_assumptions_budgeted(&[Lit::neg(2)], 0), BudgetedResult::Unsat);
}

#[test]
fn a_long_assumption_solve_restarts_and_still_refutes() {
    // Pigeonhole 7 under an assumption that changes nothing: hundreds of conflicts, so the Luby schedule restarts,
    // each restart placing the assumption again.
    let (vars, cnf) = php(7);
    let mut s = Solver::new(vars);
    for c in &cnf {
        s.add_clause(c.clone());
    }
    assert_eq!(s.solve_under_assumptions_budgeted(&[Lit::pos(0)], 0), BudgetedResult::Unsat);
    assert!(s.conflicts() > 100);
    assert!(s.restarts() > 0, "{} conflicts, no restart", s.conflicts());
}

#[test]
fn variables_added_to_a_solver_take_the_next_numbers_and_solve_like_any() {
    // A solver born empty grows as clauses need variables: under a decision set and without one.
    for restricted in [false, true] {
        let mut s = Solver::new(0);
        if restricted {
            s.set_decision_vars(&[]);
        }
        assert_eq!(s.add_vars(2), 0);
        s.add_clause(vec![Lit::pos(0), Lit::pos(1)]);
        if restricted {
            s.add_decision_vars(&[0, 1]);
        }
        assert!(matches!(s.solve_under_assumptions_budgeted(&[Lit::neg(0)], 0), BudgetedResult::Sat(m) if m[1]));
        assert_eq!(s.add_vars(3), 2);
        s.add_clause(vec![Lit::neg(1), Lit::pos(4)]);
        s.add_clause(vec![Lit::neg(4), Lit::pos(3)]);
        if restricted {
            s.add_decision_vars(&[2, 3, 4]);
        }
        match s.solve_under_assumptions_budgeted(&[Lit::neg(0)], 0) {
            BudgetedResult::Sat(m) => assert!(m.len() == 5 && m[1] && m[4] && m[3], "{m:?}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.solve_under_assumptions_budgeted(&[Lit::neg(0), Lit::neg(3)], 0), BudgetedResult::Unsat);
    }
    let (vars, cnf) = php(5);
    let mut s = Solver::new(0);
    s.add_vars(vars);
    for c in &cnf {
        s.add_clause(c.clone());
    }
    assert_eq!(s.solve(), SolveResult::Unsat, "a solver grown to its size solves as one built at it");
}

#[test]
fn with_proof_tracing_every_learned_clause_and_every_refuted_assumption_set_replays_as_lrat() {
    // One tracing solver per trial, random 3-clauses with ids 1..=n, many assumption queries, and each refuted
    // assumption set's negation added back as a permanent clause (as the sweep adds its lemmas): the solver's learned
    // clauses and those negations, each with the hints the solver gave, form one LRAT derivation that replays.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut refuted_total = 0;
    for _trial in 0..80 {
        let num_vars = 7;
        let mut s = Solver::new(num_vars);
        s.set_lrat(true);
        s.set_reduce_limit(4);
        let mut original: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..14 {
            let c: Vec<Lit> = (0..3).map(|_| Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0)).collect();
            original.push(c.clone());
            s.add_clause_with_id(c, original.len() as u64);
        }
        let mut steps: Vec<crate::lrat::LratStep> = Vec::new();
        let mut claims: Vec<Vec<Lit>> = Vec::new();
        let mut synced = 0;
        for _ in 0..12 {
            let asm: Vec<Lit> = (0..2).map(|_| Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0)).collect();
            let verdict = s.solve_under_assumptions_budgeted(&asm, 0);
            for c in &s.learned()[synced..] {
                steps.push(crate::lrat::LratStep { id: c.id, clause: c.lits.clone(), hints: c.hints.clone() });
            }
            synced = s.learned().len();
            if verdict == BudgetedResult::Unsat {
                let negated: Vec<Lit> = asm.iter().map(|a| a.negated()).collect();
                let id = s.next_clause_id();
                steps.push(crate::lrat::LratStep { id, clause: negated.clone(), hints: s.unsat_hints().to_vec() });
                s.add_clause_with_id(negated.clone(), id);
                claims.push(negated);
                refuted_total += 1;
            }
        }
        let proof = crate::lrat::Lrat { steps };
        assert!(crate::lrat::check_derivation(num_vars, &original, &proof, &claims), "{original:?} {proof:?}");
    }
    assert!(refuted_total > 50, "the trials refute assumption sets: {refuted_total}");
}

#[test]
fn proof_tracing_carries_hints_through_a_hard_refutation_with_restarts_and_reductions() {
    // Pigeonhole 6: thousands of conflicts, restarts, learned-clause deletion; the empty clause's chain is the last.
    let (vars, cnf) = php(6);
    let mut s = Solver::new(vars);
    s.set_lrat(true);
    s.set_reduce_limit(50);
    for (i, c) in cnf.iter().enumerate() {
        s.add_clause_with_id(c.clone(), i as u64 + 1);
    }
    assert_eq!(s.solve_under_assumptions_budgeted(&[], 0), BudgetedResult::Unsat);
    let mut steps: Vec<crate::lrat::LratStep> =
        s.learned().iter().map(|c| crate::lrat::LratStep { id: c.id, clause: c.lits.clone(), hints: c.hints.clone() }).collect();
    steps.push(crate::lrat::LratStep { id: s.next_clause_id(), clause: Vec::new(), hints: s.unsat_hints().to_vec() });
    let proof = crate::lrat::Lrat { steps };
    assert!(crate::lrat::check(vars, &cnf, &proof), "a refutation with every chain the solver gave");
}

#[test]
fn a_hint_names_only_a_clause_the_database_held_when_the_solve_began_or_one_learned_since() {
    // What a proof log may forget between solves is what the database no longer holds: every hint a later solve gives
    // names a clause live when it began, or one it learned before. Learned clauses taken after each solve are handed
    // over once.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let (mut named, mut reduced) = (0usize, 0usize);
    for _trial in 0..60 {
        let num_vars = 16;
        let mut s = Solver::new(num_vars);
        s.set_lrat(true);
        s.set_reduce_limit(4);
        for k in 0..68u64 {
            let c: Vec<Lit> = (0..3).map(|_| Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0)).collect();
            s.add_clause_with_id(c, k + 1);
        }
        let mut taken: HashSet<u64> = HashSet::new();
        for _ in 0..24 {
            let live: HashSet<u64> = s.live_ids().iter().copied().collect();
            let asm: Vec<Lit> = (0..2).map(|_| Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0)).collect();
            let verdict = s.solve_under_assumptions_budgeted(&asm, 0);
            let mut known = live.clone();
            for c in s.take_learned() {
                assert!(taken.insert(c.id), "learned clause {} handed over twice", c.id);
                for h in &c.hints {
                    assert!(known.contains(h), "{} names {h}, which was neither live nor learned since", c.id);
                    named += 1;
                }
                known.insert(c.id);
            }
            assert!(s.learned().is_empty(), "a take leaves nothing behind");
            reduced += live.iter().filter(|id| !s.live_ids().contains(id)).count();
            if verdict == BudgetedResult::Unsat {
                for h in s.unsat_hints() {
                    assert!(known.contains(h), "the refutation names {h}, which was neither live nor learned since");
                }
                let id = s.next_clause_id();
                s.add_clause_with_id(asm.iter().map(|a| a.negated()).collect(), id);
            }
        }
    }
    assert!(named > 1000 && reduced > 0, "the trials learn, name and reduce: {named} hints, {reduced} reduced");
}

#[test]
fn a_level_0_fact_is_named_by_its_unit_clause_not_by_everything_level_0_derived_it_from() {
    // x0, then x_i → x_{i+1} for 600 steps: level 0 derives x600 down a chain of 600 reasons. Pigeonhole 5 with every
    // clause guarded by ¬x600 needs x600 in every conflict, and each chain names it by one unit clause, derived once.
    let chain = 600;
    let (vars, holes) = php(5);
    let x = |i: usize| (vars + i) as Var;
    let mut cnf: Vec<Vec<Lit>> = vec![vec![Lit::pos(x(0))]];
    for i in 0..chain {
        cnf.push(vec![Lit::neg(x(i)), Lit::pos(x(i + 1))]);
    }
    for c in &holes {
        let mut c = c.clone();
        c.push(Lit::neg(x(chain)));
        cnf.push(c);
    }
    let num_vars = vars + chain + 1;
    let mut s = Solver::new(num_vars);
    s.set_lrat(true);
    for (i, c) in cnf.iter().enumerate() {
        s.add_clause_with_id(c.clone(), i as u64 + 1);
    }
    assert_eq!(s.solve_under_assumptions_budgeted(&[], 0), BudgetedResult::Unsat);
    let learned = s.learned();
    let units = learned.iter().filter(|c| c.lits.len() == 1 && c.lits[0].var() as usize >= vars).count();
    assert!(units > 0 && units <= chain, "each fact of the chain derived as a unit at most once: {units}");
    let longest = learned.iter().chain([&LearnedClause { lits: Vec::new(), id: 0, hints: s.unsat_hints().to_vec() }]).map(|c| c.hints.len()).max().unwrap();
    assert!(longest < chain / 4, "no chain retraces level 0: the longest names {longest} clauses");
    let mut steps: Vec<crate::lrat::LratStep> =
        learned.iter().map(|c| crate::lrat::LratStep { id: c.id, clause: c.lits.clone(), hints: c.hints.clone() }).collect();
    steps.push(crate::lrat::LratStep { id: s.next_clause_id(), clause: Vec::new(), hints: s.unsat_hints().to_vec() });
    assert!(crate::lrat::check(num_vars, &cnf, &crate::lrat::Lrat { steps }), "the refutation replays with every unit in it");
}
