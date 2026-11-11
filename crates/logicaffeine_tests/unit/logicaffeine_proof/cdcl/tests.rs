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

fn assert_unassigned_hold_no_reason(s: &Solver, context: &str) {
    for v in 0..s.num_vars {
        if s.val_of(Lit::pos(v as Var)) == Val::Unset {
            assert!(matches!(s.reason[v], Reason::Decision), "{context}: unassigned variable {v} still names a reason clause");
        }
    }
}

#[test]
fn an_unassigned_variable_holds_no_reason_after_any_backjump_restart_or_budget_stop() {
    for seed in 0..10u64 {
        let cnf = crate::families::random_3sat(120, 511, 0xDEAD_0000 ^ seed);
        let mut s = Solver::new(cnf.num_vars);
        s.set_reduce_limit(30);
        for c in &cnf.clauses {
            s.add_clause(c.clone());
        }
        for round in 0..40 {
            match s.solve_budgeted(25) {
                BudgetedResult::Budget => assert_unassigned_hold_no_reason(&s, &format!("seed {seed} round {round}")),
                _ => break,
            }
        }
    }
}

#[test]
fn heavy_reduction_and_inprocessing_never_follow_a_stale_reason_and_keep_every_verdict() {
    // A variable's reason clause can be deleted by one compaction while the variable is unassigned; a later
    // compaction of a smaller database must not read that index.
    let mut cases: Vec<(usize, Vec<Vec<Lit>>, bool)> = Vec::new();
    for holes in [5u32, 6, 7] {
        let (n, clauses) = php(holes);
        cases.push((n, clauses, false));
    }
    for seed in 0..16u64 {
        let cnf = crate::families::random_3sat(16, 70, 0xBADC_0DE0 ^ seed);
        let expected = sat_brute(cnf.num_vars, &cnf.clauses);
        cases.push((cnf.num_vars, cnf.clauses, expected));
    }
    for seed in 0..6u64 {
        let cnf = crate::families::random_3sat(140, 600, 0x51A1E ^ seed);
        let mut reference = Solver::new(cnf.num_vars);
        for c in &cnf.clauses {
            reference.add_clause(c.clone());
        }
        let expected = matches!(reference.solve(), SolveResult::Sat(_));
        cases.push((cnf.num_vars, cnf.clauses, expected));
    }
    for (i, (n, clauses, expected)) in cases.iter().enumerate() {
        for limit in [8usize, 16, 64] {
            let mut s = Solver::new(*n);
            s.set_reduce_limit(limit);
            s.set_inprocess_interval(40);
            for c in clauses {
                s.add_clause(c.clone());
            }
            match s.solve() {
                SolveResult::Sat(model) => {
                    assert!(*expected, "case {i} limit {limit}: SAT on an unsatisfiable formula");
                    assert!(check_model(clauses, &model), "case {i} limit {limit}: the model fails a clause");
                }
                SolveResult::Unsat => assert!(!*expected, "case {i} limit {limit}: UNSAT on a satisfiable formula"),
            }
            assert_unassigned_hold_no_reason(&s, &format!("case {i} limit {limit} after the solve"));
        }
    }
}

#[test]
fn target_phases_walks_and_every_restart_mode_keep_every_verdict_and_model() {
    let mut next = rng(0x7A26_E7);
    for round in 0..240u64 {
        let n = 3 + (next() % 12) as usize;
        let m = (next() % (5 * n as u64 + 1)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let width = 1 + (next() % 3) as usize;
                (0..width).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect()
            })
            .collect();
        let expected = sat_brute(n, &clauses);
        for mode in [RestartMode::Adaptive, RestartMode::Glucose, RestartMode::Luby] {
            for (target, walk) in [(true, true), (true, false), (false, true), (false, false)] {
                let mut s = Solver::new(n);
                s.set_restart_mode(mode);
                s.set_target_phases(target);
                s.set_walk(walk);
                s.set_reduce_policy(if round % 2 == 0 { ReducePolicy::Halving } else { ReducePolicy::Tiered });
                s.set_reduce_limit(3);
                for c in &clauses {
                    s.add_clause(c.clone());
                }
                match s.solve() {
                    SolveResult::Sat(model) => {
                        assert!(expected, "round {round} {mode:?} target={target} walk={walk}: SAT on an unsatisfiable formula");
                        assert!(check_model(&clauses, &model), "round {round} {mode:?}: the model fails a clause");
                    }
                    SolveResult::Unsat => assert!(!expected, "round {round} {mode:?} target={target} walk={walk}: UNSAT on a satisfiable formula"),
                }
            }
        }
    }
}

#[test]
fn stable_mode_rephases_on_schedule_and_walks_on_its_walking_steps() {
    // Luby mode is stable throughout, so the rephase schedule runs from the first limit on: the steps cycle best,
    // walk, inverted, best, walk, original, and only the walking ones walk. PHP(8→7) takes thousands of conflicts
    // under every heuristic, well past the first limit.
    let (n, clauses) = php(7);
    let solver = |mode: RestartMode, walk: bool| {
        let mut s = Solver::new(n);
        s.set_restart_mode(mode);
        s.set_walk(walk);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
        s
    };
    let s = solver(RestartMode::Luby, true);
    assert!(s.conflicts() > REPHASE_INIT, "the instance runs past the first rephase limit ({} conflicts)", s.conflicts());
    assert!(s.rephases() >= 1, "stable mode rephases past its limit");
    let walking_steps = (0..s.rephases()).filter(|k| k % 6 == 1 || k % 6 == 4).count() as u64;
    assert_eq!(s.walks(), walking_steps, "every walking step of the schedule walks, and no other");

    let quiet = solver(RestartMode::Luby, false);
    assert!(quiet.rephases() >= 1);
    assert_eq!(quiet.walks(), 0, "the walk switch silences the walking steps");

    let focused = solver(RestartMode::Glucose, true);
    assert_eq!((focused.rephases(), focused.walks()), (0, 0), "focused search never rephases");
}

#[test]
fn reductions_follow_the_square_root_schedule_and_keep_every_reason() {
    let (n, clauses) = php(7);
    for interval in [10usize, 100, 1000] {
        let mut s = Solver::new(n);
        s.set_reduce_policy(ReducePolicy::Tiered);
        s.set_reduce_limit(interval);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
        let k = s.reductions();
        assert!(k >= 1, "interval {interval}: {} conflicts and no reduction", s.conflicts());
        // The first reduction is due at `interval` conflicts and the j-th waits `interval·√(j−1)` after the one
        // before, so k of them take at least the sum; and none that fell due was skipped.
        let earliest: u64 = interval as u64 + (1..k).map(|j| (interval as f64 * (j as f64).sqrt()) as u64).sum::<u64>();
        assert!(s.conflicts() >= earliest, "interval {interval}: {k} reductions in {} conflicts runs ahead of the schedule", s.conflicts());
        assert!(s.conflicts() <= s.reduce_next, "interval {interval}: a reduction fell due and did not run");
        assert_unassigned_hold_no_reason(&s, &format!("interval {interval}"));
    }
}

#[test]
fn a_walk_that_finds_a_model_hands_it_to_the_search() {
    // Seed the phases from a model of a satisfiable random formula: a decision that follows the phases of a model
    // propagates only that model's values, so the search reaches it without one conflict.
    let cnf = crate::families::random_3sat(150, 560, 0xF00D);
    let mut reference = Solver::new(cnf.num_vars);
    for c in &cnf.clauses {
        reference.add_clause(c.clone());
    }
    let SolveResult::Sat(model) = reference.solve() else { panic!("ratio 3.73 at n=150 is satisfiable for this seed") };
    let mut s = Solver::new(cnf.num_vars);
    s.set_initial_phase(&model);
    for c in &cnf.clauses {
        s.add_clause(c.clone());
    }
    assert_eq!(s.solve(), SolveResult::Sat(model));
    assert_eq!(s.conflicts(), 0);
}

#[test]
fn the_halving_policy_reduces_when_the_live_learned_clauses_reach_the_limit() {
    let (n, clauses) = php(7);
    for limit in [20usize, 200] {
        let mut s = Solver::new(n);
        s.set_inprocess(false);
        s.set_reduce_limit(limit);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
        let k = s.reductions() as usize;
        assert!(k >= 1, "limit {limit}: {} conflicts and no reduction", s.conflicts());
        assert_eq!(s.reduce_limit, limit + k * HALVING_GROWTH, "limit {limit}: each reduction raises the limit once");
        assert!(s.learned_live < s.reduce_limit, "limit {limit}: a reduction fell due and did not run");
        assert_unassigned_hold_no_reason(&s, &format!("limit {limit}"));
    }
}

#[test]
fn both_reduction_policies_keep_every_verdict_under_incremental_assumptions() {
    let (n, clauses) = php(6);
    for policy in [ReducePolicy::Halving, ReducePolicy::Tiered] {
        let mut s = Solver::new(n);
        s.set_reduce_policy(policy);
        s.set_reduce_limit(5);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        for pigeon in 0..7u32 {
            let assume = [Lit::pos(pigeon * 6)];
            assert_eq!(s.solve_under_assumptions(&assume), SolveResult::Unsat, "{policy:?}: PHP stays unsatisfiable under assumptions");
        }
        assert!(s.reductions() >= 1, "{policy:?}: the incremental path reduces");
    }
}

#[test]
fn a_formula_satisfied_by_all_false_or_all_true_is_answered_lucky_without_a_conflict() {
    let mut next = rng(0x1C4E);
    for round in 0..200u64 {
        let n = 2 + (next() % 30) as usize;
        let polarity = round % 2 == 0;
        // Every clause holds a literal of the chosen polarity, so the all-`polarity` assignment is a model.
        let clauses: Vec<Vec<Lit>> = (0..(next() % 60) as usize)
            .map(|_| {
                let width = 1 + (next() % 4) as usize;
                let mut c: Vec<Lit> = (0..width).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect();
                c.push(Lit::new((next() % n as u64) as Var, polarity));
                c
            })
            .collect();
        let mut s = Solver::new(n);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        match s.solve() {
            SolveResult::Sat(model) => assert!(check_model(&clauses, &model), "round {round}"),
            SolveResult::Unsat => panic!("round {round}: the all-{polarity} assignment is a model"),
        }
        assert_eq!(s.conflicts(), 0, "round {round}: a lucky formula needs no search");
        assert_eq!(s.lucky_hits(), 1, "round {round}");
        assert!(s.trail_lim.is_empty(), "round {round}: the lucky solve leaves the trail at level 0");
    }
}

#[test]
fn lucky_phases_never_change_a_verdict_and_a_failed_attempt_leaves_level_zero_clean() {
    let mut next = rng(0x10C4);
    for round in 0..400u64 {
        let n = 1 + (next() % 12) as usize;
        let m = (next() % (5 * n as u64 + 1)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..m)
            .map(|_| (0..1 + (next() % 3) as usize).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect())
            .collect();
        let expected = sat_brute(n, &clauses);
        for lucky in [true, false] {
            let mut s = Solver::new(n);
            s.set_lucky(lucky);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            let found = s.lucky();
            assert!(s.trail_lim.is_empty(), "round {round}: an attempt returns to level 0");
            match &found {
                Lucky::Sat(model) => assert!(lucky && check_model(&clauses, model), "round {round}: a lucky model satisfies every clause"),
                Lucky::Unsat => assert!(lucky && !expected, "round {round}: a lucky refutation of a satisfiable formula"),
                Lucky::Unknown => {}
            }
            match s.solve() {
                SolveResult::Sat(model) => {
                    assert!(expected, "round {round} lucky={lucky}");
                    assert!(check_model(&clauses, &model), "round {round} lucky={lucky}");
                }
                SolveResult::Unsat => assert!(!expected, "round {round} lucky={lucky}"),
            }
            if !lucky {
                assert_eq!(s.lucky_hits(), 0);
            }
        }
    }
}

#[test]
fn a_decision_set_turns_lucky_phases_off() {
    // With decisions restricted, the excluded variables are a theory's to assign: no lucky model is claimed.
    let mut s = Solver::new(4);
    s.add_clause(vec![Lit::neg(0), Lit::neg(1)]);
    s.add_clause(vec![Lit::neg(2), Lit::pos(3)]);
    s.set_decision_vars(&[0, 1]);
    assert_eq!(s.lucky(), Lucky::Unknown);
    assert_eq!(s.lucky_hits(), 0);
}

#[test]
fn a_failed_first_decision_is_learned_as_a_unit_and_its_conflict_refutes() {
    // x0 forces both x1 and ¬x1, so the first lucky decision on x0 fails: ¬x0 is learned. With (x0 ∨ x2) and
    // (x0 ∨ ¬x2) as well, ¬x0 then conflicts at level 0.
    let l = |v: Var, p: bool| Lit::new(v, p);
    let mut s = Solver::new(3);
    for c in [vec![l(0, false), l(1, true)], vec![l(0, false), l(1, false)], vec![l(0, true), l(2, true)], vec![l(0, true), l(2, false)]] {
        s.add_clause(c);
    }
    s.set_initial_phase(&[true, false, false]);
    assert_eq!(s.lucky(), Lucky::Unsat);
    assert!(s.learned().iter().any(|c| c.lits == vec![l(0, false)]), "¬x0 is learned");
    assert_eq!(s.solve(), SolveResult::Unsat);
}

#[test]
fn lrat_tracing_turns_lucky_phases_off() {
    let mut s = Solver::new(2);
    s.add_clause(vec![Lit::neg(0), Lit::neg(1)]);
    s.set_lrat(true);
    assert_eq!(s.lucky(), Lucky::Unknown);
}

fn assert_queue_is_consistent(s: &Solver, context: &str) {
    // The queue links every variable exactly once, front to back by increasing stamp, and no unassigned decision
    // candidate lies past the search start.
    let mut seen = vec![false; s.num_vars];
    let mut v = s.vmtf_first;
    let mut prev = NONE;
    let mut count = 0;
    let mut past_search = false;
    while v != NONE {
        let vi = v as usize;
        assert!(!seen[vi], "{context}: variable {v} linked twice");
        seen[vi] = true;
        assert_eq!(s.vmtf_prev[vi], prev, "{context}: back link of {v}");
        if prev != NONE {
            assert!(s.vmtf_stamp[prev as usize] < s.vmtf_stamp[vi], "{context}: stamps rise front to back");
        }
        if past_search {
            assert!(s.val_of(Lit::pos(v)) != Val::Unset, "{context}: unassigned {v} lies past the search start");
        }
        if v == s.vmtf_search {
            past_search = true;
        }
        prev = v;
        v = s.vmtf_next[vi];
        count += 1;
    }
    assert_eq!(s.vmtf_last, prev, "{context}: the back of the queue");
    assert_eq!(count, s.num_vars, "{context}: every variable is queued");
}

#[test]
fn the_move_to_front_queue_keeps_every_verdict_and_its_links() {
    let mut next = rng(0x7F7F);
    for round in 0..300u64 {
        let n = 1 + (next() % 14) as usize;
        let m = (next() % (5 * n as u64 + 1)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..m)
            .map(|_| (0..1 + (next() % 3) as usize).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect())
            .collect();
        let expected = sat_brute(n, &clauses);
        for mode in [RestartMode::Glucose, RestartMode::Adaptive] {
            let mut s = Solver::new(n);
            s.set_restart_mode(mode);
            s.set_focused_queue(true);
            s.set_lucky(false);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            match s.solve() {
                SolveResult::Sat(model) => {
                    assert!(expected, "round {round} {mode:?}");
                    assert!(check_model(&clauses, &model), "round {round} {mode:?}");
                }
                SolveResult::Unsat => assert!(!expected, "round {round} {mode:?}"),
            }
            assert_queue_is_consistent(&s, &format!("round {round} {mode:?}"));
        }
    }
    // Long searches, a budgeted stop between them, and variables added after a solve.
    let (n, clauses) = php(7);
    let mut s = Solver::new(n);
    s.set_restart_mode(RestartMode::Glucose);
    s.set_focused_queue(true);
    for c in &clauses {
        s.add_clause(c.clone());
    }
    assert_eq!(s.solve_budgeted(300), BudgetedResult::Budget);
    assert_queue_is_consistent(&s, "PHP(8) after a budgeted stop");
    let first = s.add_vars(3);
    s.add_clause(vec![Lit::pos(first as Var), Lit::pos(first as Var + 1)]);
    assert_queue_is_consistent(&s, "after adding variables");
    assert_eq!(s.solve(), SolveResult::Unsat);
    assert_queue_is_consistent(&s, "PHP(8) refuted");
}

fn assert_trail_is_consistent(s: &Solver, context: &str) {
    // Every assigned variable sits on the trail where `trail_pos` says, at a level the trail has, and each
    // implied one's reason holds it true with every other literal false at a level no higher than its own.
    for (pos, &l) in s.trail.iter().enumerate() {
        let v = l.var() as usize;
        assert_eq!(s.trail_pos[v] as usize, pos, "{context}: trail position of {v}");
        assert_eq!(s.val_of(l), Val::True, "{context}: trail literal {v} is true");
        assert!(s.level[v] as usize <= s.trail_lim.len(), "{context}: level of {v} within the trail");
        if let Reason::Clause(ci) = s.reason[v] {
            let c = s.arena.clause(ci);
            assert!(c.contains(&l), "{context}: the reason of {v} holds it");
            for &q in c {
                if q != l {
                    assert_eq!(s.val_of(q), Val::False, "{context}: the reason of {v} is unit");
                    assert!(s.level[q.var() as usize] <= s.level[v], "{context}: a reason literal of {v} above its level");
                    assert!(s.trail_pos[q.var() as usize] < s.trail_pos[v], "{context}: a reason literal of {v} after it");
                }
            }
        }
    }
}

#[test]
fn chronological_backtracking_keeps_every_verdict_model_and_trail_invariant() {
    // A jump limit of 0 backtracks chronologically on every backjump past one level: the most out-of-order trails.
    let mut next = rng(0xC4120);
    let mut chrono_seen = 0u64;
    for round in 0..600u64 {
        let n = 3 + (next() % 14) as usize;
        let m = (next() % (5 * n as u64 + 1)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..m)
            .map(|_| (0..1 + (next() % 3) as usize).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect())
            .collect();
        let expected = sat_brute(n, &clauses);
        for limit in [0u32, 1, 3] {
            let mut s = Solver::new(n);
            s.set_chrono(Some(limit));
            s.set_lucky(false);
            s.set_reduce_limit(4);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            match s.solve() {
                SolveResult::Sat(model) => {
                    assert!(expected, "round {round} limit {limit}: SAT on an unsatisfiable formula");
                    assert!(check_model(&clauses, &model), "round {round} limit {limit}: the model fails a clause");
                    assert_trail_is_consistent(&s, &format!("round {round} limit {limit}"));
                }
                SolveResult::Unsat => assert!(!expected, "round {round} limit {limit}: UNSAT on a satisfiable formula"),
            }
            chrono_seen += s.chrono_backtracks();
        }
    }
    for holes in [5u32, 6] {
        let (n, clauses) = php(holes);
        let mut s = Solver::new(n);
        s.set_chrono(Some(0));
        for c in &clauses {
            s.add_clause(c.clone());
        }
        assert_eq!(s.solve(), SolveResult::Unsat, "PHP({}) under chronological backtracking", holes + 1);
        chrono_seen += s.chrono_backtracks();
    }
    for seed in 0..8u64 {
        let cnf = crate::families::random_3sat(90, 384, 0xC4 ^ seed);
        let mut reference = Solver::new(cnf.num_vars);
        for c in &cnf.clauses {
            reference.add_clause(c.clone());
        }
        let expected = matches!(reference.solve(), SolveResult::Sat(_));
        let mut s = Solver::new(cnf.num_vars);
        s.set_chrono(Some(0));
        s.set_lucky(false);
        for c in &cnf.clauses {
            s.add_clause(c.clone());
        }
        match s.solve() {
            SolveResult::Sat(model) => {
                assert!(expected, "seed {seed}");
                assert!(check_model(&cnf.clauses, &model), "seed {seed}");
                assert_trail_is_consistent(&s, &format!("seed {seed}"));
            }
            SolveResult::Unsat => assert!(!expected, "seed {seed}"),
        }
        chrono_seen += s.chrono_backtracks();
    }
    assert!(chrono_seen > 100, "the sample backtracks chronologically ({chrono_seen} times)");
}

#[test]
fn chronological_backtracking_is_off_by_default_and_under_assumptions() {
    let (n, clauses) = php(6);
    let mut plain = Solver::new(n);
    for c in &clauses {
        plain.add_clause(c.clone());
    }
    assert_eq!(plain.solve(), SolveResult::Unsat);
    assert_eq!(plain.chrono_backtracks(), 0);
    let mut assumed = Solver::new(n);
    assumed.set_chrono(Some(0));
    for c in &clauses {
        assumed.add_clause(c.clone());
    }
    assert_eq!(assumed.solve_under_assumptions(&[Lit::pos(0)]), SolveResult::Unsat);
    assert_eq!(assumed.chrono_backtracks(), 0, "an assumption solve keeps its trail in level order");
}

/// Lucky phases backtrack without saving phases, as Kissat's do: whenever they find no model, the saved phases are
/// the ones the solver held before them, so the search starts from its own phases and not from an attempt's.
#[test]
fn a_lucky_pass_that_finds_no_model_leaves_the_saved_phases_as_they_were() {
    let mut next = rng(0x9A5E);
    let mut unlucky = 0usize;
    for round in 0..600u64 {
        let n = 3 + (next() % 14) as usize;
        let m = (next() % (5 * n as u64)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..m)
            .map(|_| (0..2 + (next() % 2) as usize).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect())
            .collect();
        let phases: Vec<bool> = (0..n).map(|_| next() & 1 == 0).collect();
        let mut s = Solver::new(n);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        s.set_initial_phase(&phases);
        let before = s.saved_phase.clone();
        if !matches!(s.lucky(), Lucky::Sat(_)) {
            assert_eq!(s.saved_phase, before, "round {round}: the attempts left their phases behind");
            unlucky += 1;
        }
    }
    assert!(unlucky >= 100, "{unlucky} formulas without a lucky model");
}

/// A lucky pass that finds no model and refutes no first decision leaves no trace: the search after it meets the
/// formula as a solver without lucky phases would — the same watches, the same phases — and takes the same conflicts.
#[test]
fn a_fruitless_lucky_pass_leaves_the_search_as_it_would_have_been() {
    let mut next = rng(0x1E55);
    let mut compared = 0usize;
    for round in 0..400u64 {
        let n = 10 + (next() % 30) as usize;
        let m = (4.3 * n as f64) as usize;
        let clauses: Vec<Vec<Lit>> = (0..m).map(|_| (0..3).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect()).collect();
        let fresh = |lucky: bool| {
            let mut s = Solver::new(n);
            s.set_lucky(lucky);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            s
        };
        let mut probe = fresh(true);
        let learned_before = probe.learned().len();
        if !matches!(probe.lucky(), Lucky::Unknown) || probe.learned().len() != learned_before {
            continue;
        }
        let (mut with, mut without) = (fresh(true), fresh(false));
        let (a, b) = (with.solve(), without.solve());
        assert_eq!(matches!(a, SolveResult::Sat(_)), matches!(b, SolveResult::Sat(_)), "round {round}");
        assert_eq!(with.conflicts(), without.conflicts(), "round {round}: the lucky pass changed the search");
        compared += 1;
    }
    assert!(compared >= 100, "{compared} fruitless lucky passes compared");
}

/// What a [`KeptSpy`] saw: its calls, the literals the solver said changed, and the literals it was given in all.
#[derive(Default)]
struct KeptTally {
    calls: usize,
    changed: usize,
    given: usize,
}

/// A theory checking the solver's word on what it kept: each call's `kept` literals are the ones it was given last
/// time, unchanged.
struct KeptSpy {
    inner: Box<dyn Theory>,
    last: Vec<Lit>,
    tally: std::rc::Rc<std::cell::RefCell<KeptTally>>,
}

impl Theory for KeptSpy {
    fn propagate(&mut self, _: &[Lit]) -> Vec<Vec<Lit>> {
        unreachable!("the solver says what it kept")
    }

    fn propagate_kept(&mut self, trail: &[Lit], kept: usize) -> Vec<Vec<Lit>> {
        assert!(kept <= self.last.len() && kept <= trail.len(), "kept {kept} of {} given, {} now", self.last.len(), trail.len());
        assert_eq!(trail[..kept], self.last[..kept], "the kept literals are the ones given last time");
        let mut tally = self.tally.borrow_mut();
        tally.calls += 1;
        tally.changed += self.last.len() - kept;
        tally.given += self.last.len();
        self.last = trail.to_vec();
        self.inner.propagate(trail)
    }
}

#[test]
fn solve_with_tells_each_theory_the_trail_it_kept() {
    use crate::xorsat::XorEquation;
    let mut rng: u64 = 0x94D0_49BB_1331_11EB;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let tally = std::rc::Rc::new(std::cell::RefCell::new(KeptTally::default()));
    for _ in 0..60 {
        let num_vars = 40;
        let mut clauses: Vec<Vec<Lit>> = Vec::new();
        let mut halves: [Vec<XorEquation>; 2] = [Vec::new(), Vec::new()];
        for i in 0..30 {
            let mut vars: Vec<usize> = (0..3).map(|_| (next() % num_vars as u64) as usize).collect();
            vars.sort_unstable();
            vars.dedup();
            let rhs = next() % 2 == 0;
            for mask in 0u32..1 << vars.len() {
                if (mask.count_ones() % 2 == 1) != rhs {
                    clauses.push((0..vars.len()).map(|j| Lit::new(vars[j] as u32, mask >> j & 1 == 0)).collect());
                }
            }
            halves[i % 2].push(XorEquation::new(vars, rhs));
        }
        for _ in 0..60 {
            clauses.push((0..3).map(|_| Lit::new((next() % num_vars as u64) as u32, next() % 2 == 0)).collect());
        }
        let mut solver = Solver::new(num_vars);
        let mut plain = Solver::new(num_vars);
        for c in &clauses {
            solver.add_clause(c.clone());
            plain.add_clause(c.clone());
        }
        let spy = |eqs: &[XorEquation]| -> Box<dyn Theory> {
            Box::new(KeptSpy { inner: Box::new(crate::xor_engine::XorEngine::new(num_vars, eqs)), last: Vec::new(), tally: tally.clone() })
        };
        let mut theories: Vec<Box<dyn Theory>> = vec![spy(&halves[0]), spy(&halves[1])];
        let verdict = solver.solve_with(&mut theories);
        assert_eq!(matches!(verdict, SolveResult::Sat(_)), matches!(plain.solve(), SolveResult::Sat(_)));
    }
    let tally = tally.borrow();
    assert!(tally.calls > 1000, "{} calls", tally.calls);
    // Told nothing was kept, a theory would see every literal it was given change: `changed == given`.
    assert!(tally.changed * 2 < tally.given, "the solver keeps most of the trail between calls: {} of {} changed", tally.changed, tally.given);
}

fn satisfied_by(clauses: &[Vec<Lit>], model: &[bool]) -> bool {
    clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
}

#[test]
fn restricted_decisions_still_answer_with_a_model() {
    // Decisions fall on x0 alone, which no clause names; nothing propagates x1 and x2, and their default values, both
    // false, would leave x1 ∨ x2 unsatisfied.
    let clauses = vec![vec![Lit::pos(1), Lit::pos(2)], vec![Lit::neg(1), Lit::neg(2)]];
    for focused in [false, true] {
        for walk in [false, true] {
            let make = || {
                let mut solver = Solver::new(3);
                solver.set_focused_queue(focused);
                solver.set_walk(walk);
                for c in &clauses {
                    solver.add_clause(c.clone());
                }
                solver.set_decision_vars(&[0]);
                solver
            };
            let SolveResult::Sat(m) = make().solve() else { panic!("satisfiable") };
            assert!(satisfied_by(&clauses, &m), "solve: {m:?} (focused {focused}, walk {walk})");
            let SolveResult::Sat(m) = make().solve_with(&mut []) else { panic!("satisfiable") };
            assert!(satisfied_by(&clauses, &m), "solve_with: {m:?} (focused {focused}, walk {walk})");
            let SolveResult::Sat(m) = make().solve_under_assumptions(&[]) else { panic!("satisfiable") };
            assert!(satisfied_by(&clauses, &m), "solve_under_assumptions: {m:?} (focused {focused}, walk {walk})");
        }
    }
}

#[test]
fn restricted_decisions_never_change_the_verdict() {
    let mut rng: u64 = 0xC2B2_AE3D_27D4_EB4F;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let (mut sat, mut unsat) = (0, 0);
    for _ in 0..400 {
        let num_vars = 8 + (next() % 13) as usize;
        let count = num_vars * 4 + (next() % (num_vars as u64)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..count)
            .map(|_| (0..3).map(|_| Lit::new((next() % num_vars as u64) as u32, next() % 2 == 0)).collect())
            .collect();
        let mask: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
        let mut free = Solver::new(num_vars);
        let mut restricted = Solver::new(num_vars);
        for c in &clauses {
            free.add_clause(c.clone());
            restricted.add_clause(c.clone());
        }
        restricted.set_decision_vars(&mask);
        match (free.solve(), restricted.solve()) {
            (SolveResult::Sat(_), SolveResult::Sat(m)) => {
                assert!(satisfied_by(&clauses, &m), "a restricted model satisfies every clause: mask {mask:?}");
                sat += 1;
            }
            (SolveResult::Unsat, SolveResult::Unsat) => unsat += 1,
            (f, r) => panic!("free {f:?}, restricted {r:?}: mask {mask:?} {clauses:?}"),
        }
    }
    assert!(sat > 100 && unsat > 100, "{sat} SAT, {unsat} UNSAT");
}

/// Random 3-XOR gadgets over `num_vars` variables beside random 3-clauses, all satisfied by a planted assignment, with
/// the equations: satisfiable, and its search led through the equations.
fn xor_heavy(next: &mut impl FnMut() -> u64, num_vars: usize, equations: usize, residual: usize) -> (Vec<crate::xorsat::XorEquation>, Vec<Vec<Lit>>) {
    let planted: Vec<bool> = (0..num_vars).map(|_| next() % 2 == 0).collect();
    let mut eqs = Vec::new();
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for _ in 0..equations {
        let mut vars: Vec<usize> = (0..3).map(|_| (next() % num_vars as u64) as usize).collect();
        vars.sort_unstable();
        vars.dedup();
        let rhs = vars.iter().fold(false, |a, &v| a ^ planted[v]);
        for mask in 0u32..1 << vars.len() {
            if (mask.count_ones() % 2 == 1) != rhs {
                clauses.push((0..vars.len()).map(|j| Lit::new(vars[j] as u32, mask >> j & 1 == 0)).collect());
            }
        }
        eqs.push(crate::xorsat::XorEquation::new(vars, rhs));
    }
    while clauses.len() < residual + eqs.iter().map(|e| 1 << (e.vars.len() - 1)).sum::<usize>() {
        let clause: Vec<Lit> = (0..3).map(|_| Lit::new((next() % num_vars as u64) as u32, next() % 2 == 0)).collect();
        if clause.iter().any(|l| planted[l.var() as usize] == l.is_positive()) {
            clauses.push(clause);
        }
    }
    (eqs, clauses)
}

#[test]
fn a_theorys_propagations_leave_the_clause_database_alone() {
    let mut rng: u64 = 0x3C6E_F372_FE94_F82B;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let (mut explained, mut conflicts) = (0usize, 0u64);
    for _ in 0..40 {
        let (eqs, clauses) = xor_heavy(&mut next, 60, 45, 90);
        let mut solver = Solver::new(60);
        for c in &clauses {
            solver.add_clause(c.clone());
        }
        let mut theories: Vec<Box<dyn Theory>> = vec![Box::new(crate::xor_engine::WatchedXor::new(60, &eqs))];
        let verdict = solver.solve_with(&mut theories);
        if let SolveResult::Sat(m) = &verdict {
            assert!(satisfied_by(&clauses, m));
            explained += solver.explanations.entries.len();
        }
        // Each conflict adds at most the theory's false clause and the learned clause; a propagation adds nothing.
        assert!(solver.learned().len() as u64 <= 2 * solver.conflicts(), "{} logged over {} conflicts", solver.learned().len(), solver.conflicts());
        conflicts += solver.conflicts();
    }
    assert!(explained > 100 && conflicts > 100, "{explained} explanations live at the models, {conflicts} conflicts");
}

#[test]
fn every_live_explanation_explains_the_literal_at_its_trail_position() {
    let mut rng: u64 = 0xA54F_F53A_5F1D_36F1;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let mut checked = 0;
    for case in 0..60 {
        let (eqs, clauses) = xor_heavy(&mut next, 50, 40, 60 + case);
        let mut solver = Solver::new(50);
        for c in &clauses {
            solver.add_clause(c.clone());
        }
        let mut theories: Vec<Box<dyn Theory>> = vec![Box::new(crate::xor_engine::WatchedXor::new(50, &eqs))];
        if !matches!(solver.solve_with(&mut theories), SolveResult::Sat(_)) {
            continue;
        }
        let entries = solver.explanations.entries.clone();
        assert!(entries.windows(2).all(|p| p[0].at < p[1].at), "the explanations are in trail order");
        assert!(solver.explanations.asked.is_empty() && solver.explanations.asked_by.is_empty(), "no analysis holds a clause it asked for");
        for (k, e) in entries.iter().enumerate() {
            let lit = solver.trail[e.at as usize];
            assert!(matches!(solver.reason[lit.var() as usize], Reason::Theory(j) if j as usize == k), "that literal's reason is the explanation");
            let clause: Vec<Lit> = if e.theory == NONE {
                solver.explanations.get(k as u32).to_vec()
            } else {
                assert!(!solver.explanations.ready(k as u32), "a token is unasked between analyses");
                let mut clause = Vec::new();
                theories[e.theory as usize].explain(e.token, &mut clause);
                clause
            };
            assert_eq!(clause[0], lit, "an explanation starts with the literal it explains");
            assert!(clause[1..].iter().all(|&q| solver.lit_false(q) && solver.trail_pos[q.var() as usize] < e.at), "its other literals are false, and earlier");
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked} explanations checked");
}

#[test]
fn reason_side_bumping_bumps_the_reasons_of_the_learned_literals_and_nothing_else() {
    // x0 and x1 decided, x2 implied by x2 ∨ ¬x0 ∨ ¬x1, x3 decided: a learned ¬x2 ∨ ¬x3 has x2's reason side
    // {x0, x1}; x3, a decision, has none, and x4 and x5 take no part.
    let mut solver = Solver::new(6);
    solver.add_clause(vec![Lit::pos(2), Lit::neg(0), Lit::neg(1)]);
    solver.add_clause(vec![Lit::pos(4), Lit::pos(5)]);
    for v in [0, 1, 3] {
        solver.trail_lim.push(solver.trail.len());
        solver.enqueue(Lit::pos(v), Reason::Decision);
        assert!(solver.propagate().is_none());
    }
    assert!(matches!(solver.reason[2], Reason::Clause(_)), "x2 is implied");
    let before = solver.activity.clone();
    solver.bump_reason_side(&[Lit::neg(2), Lit::neg(3)]);
    let bumped: Vec<usize> = (0..6).filter(|&v| solver.activity[v] > before[v]).collect();
    assert_eq!(bumped, vec![0, 1], "the reason side of x2, and only it");
    assert!(solver.side_mark.iter().all(|&m| !m), "the marks are cleared");
}

#[test]
fn reason_side_bumping_holds_back_a_reason_side_past_its_limit() {
    // x_big implied by a clause over 25 decided variables: past ten a learned literal, nothing is bumped.
    let n = 27;
    let mut solver = Solver::new(n);
    let mut clause = vec![Lit::pos(25)];
    clause.extend((0..25).map(Lit::neg));
    solver.add_clause(clause);
    for v in 0..25 {
        solver.trail_lim.push(solver.trail.len());
        solver.enqueue(Lit::pos(v), Reason::Decision);
        assert!(solver.propagate().is_none());
    }
    solver.trail_lim.push(solver.trail.len());
    solver.enqueue(Lit::pos(26), Reason::Decision);
    let before = solver.activity.clone();
    solver.bump_reason_side(&[Lit::neg(25), Lit::neg(26)]);
    assert_eq!(solver.activity, before, "25 reason-side literals for a two-literal clause is past the limit of 20");
}

#[test]
fn reason_side_bumping_never_changes_a_verdict() {
    let mut rng: u64 = 0x8BB8_4B93_962E_ACC9;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let (mut sat, mut unsat) = (0, 0);
    for _ in 0..400 {
        let num_vars = 10 + (next() % 30) as usize;
        let count = (num_vars as f64 * (3.8 + (next() % 10) as f64 / 10.0)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..count)
            .map(|_| (0..3).map(|_| Lit::new((next() % num_vars as u64) as u32, next() % 2 == 0)).collect())
            .collect();
        let mut plain = Solver::new(num_vars);
        let mut bumping = Solver::new(num_vars);
        plain.set_bump_reasons(false);
        bumping.set_bump_reasons(true);
        for c in &clauses {
            plain.add_clause(c.clone());
            bumping.add_clause(c.clone());
        }
        match (plain.solve(), bumping.solve()) {
            (SolveResult::Sat(_), SolveResult::Sat(m)) => {
                assert!(satisfied_by(&clauses, &m));
                sat += 1;
            }
            (SolveResult::Unsat, SolveResult::Unsat) => unsat += 1,
            (p, b) => panic!("plain {p:?}, bumping {b:?}: {clauses:?}"),
        }
    }
    assert!(sat > 80 && unsat > 80, "{sat} SAT, {unsat} UNSAT");
}

#[test]
fn conflict_analysis_bumps_reason_sides_exactly_when_asked() {
    let cnf = crate::families::random_3sat(80, 341, 0x5EED_0B0B);
    let mut on = Solver::new(cnf.num_vars);
    let mut off = Solver::new(cnf.num_vars);
    on.set_bump_reasons(true);
    for c in &cnf.clauses {
        on.add_clause(c.clone());
        off.add_clause(c.clone());
    }
    let _ = (on.solve(), off.solve());
    assert!(on.conflicts() > 20, "the search learns ({} conflicts)", on.conflicts());
    assert!(on.reason_side_bumps() > 0, "asked, analysis bumps reason sides");
    assert_eq!(off.reason_side_bumps(), 0, "unasked, it bumps none");
}

#[test]
fn a_search_resumed_in_slices_is_the_search_run_whole() {
    let mut rng: u64 = 0x7F4A_7C15_9E37_79B9;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let (mut sat, mut unsat, mut sliced) = (0, 0, 0);
    for case in 0..120 {
        let num_vars = 30 + (next() % 60) as usize;
        let count = (num_vars as f64 * 4.26) as usize;
        let clauses: Vec<Vec<Lit>> = (0..count)
            .map(|_| (0..3).map(|_| Lit::new((next() % num_vars as u64) as u32, next() % 2 == 0)).collect())
            .collect();
        let build = || {
            let mut solver = Solver::new(num_vars);
            for c in &clauses {
                solver.add_clause(c.clone());
            }
            solver
        };
        let mut whole = build();
        let expected = whole.solve();
        let mut parts = build();
        let slice = 1 + case % 23;
        let got = match parts.begin(0) {
            Some(verdict) => verdict,
            None => loop {
                if let Some(verdict) = parts.resume(&mut [], slice as u64) {
                    break verdict;
                }
                sliced += 1;
            },
        };
        assert_eq!(got, expected, "case {case}: the sliced search reaches the whole one's verdict and model");
        assert_eq!(parts.conflicts(), whole.conflicts(), "case {case}: by the same conflicts");
        assert_eq!(parts.decisions(), whole.decisions(), "case {case}: and the same decisions");
        match expected {
            SolveResult::Sat(_) => sat += 1,
            SolveResult::Unsat => unsat += 1,
        }
    }
    assert!(sat > 20 && unsat > 20 && sliced > 500, "{sat} SAT, {unsat} UNSAT, {sliced} pauses");
}

/// A chain of conjunctions over `width` fresh inputs as Tseitin clauses: its inputs, then its gates, the last the
/// conjunction of every input.
fn and_chain(s: &mut Solver, width: usize) -> Vec<usize> {
    let first = s.add_vars(2 * width - 1);
    let mut vars: Vec<usize> = (first..first + width).collect();
    let mut acc = first;
    for k in 1..width {
        let g = first + width + k - 1;
        s.add_clause(vec![Lit::neg(g as u32), Lit::pos(acc as u32)]);
        s.add_clause(vec![Lit::neg(g as u32), Lit::pos((first + k) as u32)]);
        s.add_clause(vec![Lit::pos(g as u32), Lit::neg(acc as u32), Lit::neg((first + k) as u32)]);
        vars.push(g);
        acc = g;
    }
    vars
}

/// Whether `m` gives each gate of an [`and_chain`] the conjunction of its two fan-ins.
fn computes(chain: &[usize], width: usize, m: &[bool]) -> bool {
    (1..width).all(|k| {
        let acc = if k == 1 { chain[0] } else { chain[width + k - 2] };
        m[chain[width + k - 1]] == (m[acc] && m[chain[k]])
    })
}

/// A circuit as clauses: its inputs, then And gates over earlier variables (either polarity), each defined by its three
/// Tseitin clauses and given to the solver as a gate.
struct Circuit {
    inputs: usize,
    gates: Vec<(usize, Lit, Lit)>,
}

impl Circuit {
    fn random(next: &mut impl FnMut() -> u64, inputs: usize, gates: usize) -> Circuit {
        let mut c = Circuit { inputs, gates: Vec::new() };
        for g in inputs..inputs + gates {
            let pick = |next: &mut dyn FnMut() -> u64| Lit::new((next() % g as u64) as Var, next() & 1 == 0);
            let (a, b) = (pick(next), pick(next));
            c.gates.push((g, a, b));
        }
        c
    }

    fn vars(&self) -> usize {
        self.inputs + self.gates.len()
    }

    fn clauses(&self) -> Vec<Vec<Lit>> {
        let mut out = Vec::new();
        for &(g, a, b) in &self.gates {
            let g = g as Var;
            out.push(vec![Lit::neg(g), a]);
            out.push(vec![Lit::neg(g), b]);
            out.push(vec![Lit::pos(g), a.negated(), b.negated()]);
        }
        out
    }

    /// The solver over the circuit, deciding by justification and tracing LRAT, clause `k` of [`Circuit::clauses`]
    /// under id `k + 1`.
    fn justifying(&self) -> Solver {
        let mut s = Solver::new(self.vars());
        s.set_lrat(true);
        for (k, c) in self.clauses().into_iter().enumerate() {
            s.add_clause_with_id(c, k as u64 + 1);
        }
        for &(g, a, b) in &self.gates {
            s.set_gate(g, a, b);
        }
        s.set_justifying(true);
        s
    }

    /// Every variable's value with the inputs as `m` gives them.
    fn evaluate(&self, m: &[bool]) -> Vec<bool> {
        let mut v = m[..self.inputs].to_vec();
        for &(_, a, b) in &self.gates {
            let read = |l: Lit| v[l.var() as usize] == l.is_positive();
            let value = read(a) && read(b);
            v.push(value);
        }
        v
    }
}

#[test]
fn a_justifying_search_assigns_what_its_assumptions_need_and_leaves_the_rest_open() {
    // Two conjunction chains on their own variables. Asking one chain's output false is answered by justifying it down
    // one path to an input set false, a decision a gate at most, the other chain untouched however long it is; asking
    // it true sets every input of it.
    let mut s = Solver::new(0);
    let left = and_chain(&mut s, 8);
    let right = and_chain(&mut s, 40);
    for chain in [&left, &right] {
        let width = (chain.len() + 1) / 2;
        for k in 1..width {
            let acc = if k == 1 { chain[0] } else { chain[width + k - 2] };
            s.set_gate(chain[width + k - 1], Lit::pos(acc as u32), Lit::pos(chain[k] as u32));
        }
    }
    s.set_justifying(true);
    let out = *left.last().unwrap() as u32;
    let before = s.propagations();
    match s.solve_under_assumptions_budgeted(&[Lit::neg(out)], 0) {
        BudgetedResult::Sat(m) => {
            assert!(!m[out as usize] && computes(&left, 8, &m), "{m:?}");
            assert!(left[..8].iter().any(|&x| !m[x]), "an input of the chain is false: {m:?}");
            assert!(right.iter().all(|&v| !m[v]), "the other chain is never assigned: {m:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(s.propagations() - before <= left.len() as u64, "{} assignments", s.propagations() - before);
    assert!(s.decisions() <= 7, "a decision a gate of the chain at most: {}", s.decisions());
    match s.solve_under_assumptions_budgeted(&[Lit::pos(out)], 0) {
        BudgetedResult::Sat(m) => assert!(left.iter().all(|&v| m[v]) && right.iter().all(|&v| !m[v]), "{m:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_justifying_search_decides_as_plain_cdcl_its_models_evaluating_to_its_assumptions_and_its_refutations_replaying() {
    // Random circuits, one justifying solver each answering a run of queries on gate values with every clause it
    // learns kept: each verdict that of a plain solver, each model's inputs evaluating to the values assumed, and each
    // refutation's chain replaying from the circuit and the learned clauses.
    let mut next = rng(0x9E37_79B9_7F4A_7C15);
    let (mut sat, mut unsat, mut decided) = (0, 0, 0u64);
    for trial in 0..60 {
        let circuit = Circuit::random(&mut next, 4 + trial % 6, 20 + 3 * (trial % 10));
        let original = circuit.clauses();
        let mut s = circuit.justifying();
        s.set_reduce_limit(8);
        let mut steps: Vec<crate::lrat::LratStep> = Vec::new();
        let mut claims: Vec<Vec<Lit>> = Vec::new();
        let mut synced = 0;
        for _ in 0..25 {
            let width = circuit.vars() as u64;
            let asm: Vec<Lit> = (0..1 + next() % 3)
                .map(|_| Lit::new((circuit.inputs as u64 + next() % (width - circuit.inputs as u64)) as Var, next() & 1 == 0))
                .collect();
            let mut plain = Solver::new(circuit.vars());
            for c in &original {
                plain.add_clause(c.clone());
            }
            let truth = plain.solve_under_assumptions_budgeted(&asm, 0);
            let before = s.decisions();
            let verdict = s.solve_under_assumptions_budgeted(&asm, 0);
            decided += s.decisions() - before;
            for c in &s.learned()[synced..] {
                steps.push(crate::lrat::LratStep { id: c.id, clause: c.lits.clone(), hints: c.hints.clone() });
            }
            synced = s.learned().len();
            match (&truth, &verdict) {
                (BudgetedResult::Sat(_), BudgetedResult::Sat(m)) => {
                    let values = circuit.evaluate(m);
                    for a in &asm {
                        assert_eq!(values[a.var() as usize], a.is_positive(), "trial {trial}: the inputs of the model evaluate to {asm:?}");
                    }
                    sat += 1;
                }
                (BudgetedResult::Unsat, BudgetedResult::Unsat) => {
                    let negated: Vec<Lit> = asm.iter().map(|a| a.negated()).collect();
                    let id = s.next_clause_id();
                    steps.push(crate::lrat::LratStep { id, clause: negated.clone(), hints: s.unsat_hints().to_vec() });
                    s.add_clause_with_id(negated.clone(), id);
                    claims.push(negated);
                    unsat += 1;
                }
                (t, v) => panic!("trial {trial}, {asm:?}: plain {t:?}, justifying {v:?}"),
            }
        }
        let proof = crate::lrat::Lrat { steps };
        assert!(crate::lrat::check_derivation(circuit.vars(), &original, &proof, &claims), "trial {trial}");
    }
    assert!(sat > 300 && unsat > 100 && decided > 300, "{sat} SAT, {unsat} UNSAT, {decided} decisions");
}

#[test]
fn a_variable_a_justifying_search_leaves_open_has_no_value_and_one_it_assigns_has_the_models() {
    let mut s = Solver::new(0);
    let left = and_chain(&mut s, 6);
    let right = and_chain(&mut s, 6);
    for chain in [&left, &right] {
        for k in 1..6 {
            let acc = if k == 1 { chain[0] } else { chain[6 + k - 2] };
            s.set_gate(chain[6 + k - 1], Lit::pos(acc as u32), Lit::pos(chain[k] as u32));
        }
    }
    s.set_justifying(true);
    let out = *left.last().unwrap() as u32;
    let BudgetedResult::Sat(m) = s.solve_under_assumptions_budgeted(&[Lit::pos(out)], 0) else { panic!("satisfiable") };
    assert!(left.iter().all(|&v| s.value(v) == Some(m[v])), "the chain asked about is assigned as the model reads it");
    assert!(right.iter().all(|&v| s.value(v).is_none()), "the other chain is left open");
    assert_eq!(s.value(s.num_vars), None, "a variable the solver does not have has no value");
    s.set_justifying(false);
    let BudgetedResult::Sat(m) = s.solve_under_assumptions_budgeted(&[Lit::pos(out)], 0) else { panic!("satisfiable") };
    assert!((0..s.num_vars).all(|v| s.value(v) == Some(m[v])), "a complete search assigns every variable");
}

#[test]
fn an_unsat_core_is_assumptions_in_their_order_that_the_formula_refutes_alone() {
    // Random formulas under the threshold under random assumptions, one solver answering every query: each core a
    // subset of the assumptions in their order, refuted on its own by a fresh solver, often fewer than all of them;
    // a satisfiable answer leaves no core, and an unsatisfiable formula an empty one.
    let mut next = rng(0x2545_F491_4F6C_DD1D);
    let (mut refuted, mut smaller) = (0, 0);
    for trial in 0..120 {
        let n = 14 + trial % 12;
        let clauses: Vec<Vec<Lit>> = (0..(n as f64 * 3.6) as usize)
            .map(|_| (0..3).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect())
            .collect();
        let mut s = Solver::new(n);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        if s.solve_under_assumptions_budgeted(&[], 0) == BudgetedResult::Unsat {
            continue;
        }
        for _ in 0..6 {
            let mut vars: Vec<Var> = (0..n as Var).collect();
            let take = 2 + (next() % 6) as usize;
            let asm: Vec<Lit> = (0..take)
                .map(|_| {
                    let v = vars.swap_remove((next() % vars.len() as u64) as usize);
                    Lit::new(v, next() & 1 == 0)
                })
                .collect();
            match s.solve_under_assumptions_budgeted(&asm, 0) {
                BudgetedResult::Unsat => {
                    let core = s.unsat_core().to_vec();
                    let place = |l: &Lit| asm.iter().position(|a| a == l).expect("an assumption");
                    let at: Vec<usize> = core.iter().map(place).collect();
                    assert!(at.windows(2).all(|w| w[0] < w[1]), "trial {trial}: the core keeps the assumptions' order");
                    let mut alone = Solver::new(n);
                    for c in &clauses {
                        alone.add_clause(c.clone());
                    }
                    let alone_says = alone.solve_under_assumptions_budgeted(&core, 0);
                    assert_eq!(alone_says, BudgetedResult::Unsat, "trial {trial}");
                    assert!(!core.is_empty(), "trial {trial}: a satisfiable formula is refuted under some assumption");
                    refuted += 1;
                    smaller += usize::from(core.len() < asm.len());
                }
                BudgetedResult::Sat(_) => assert!(s.unsat_core().is_empty(), "trial {trial}"),
                BudgetedResult::Budget => unreachable!(),
            }
        }
    }
    assert!(refuted > 60 && smaller > 20, "{refuted} refuted, {smaller} by fewer than all the assumptions");
    let mut contradiction = Solver::new(1);
    contradiction.add_clause(vec![Lit::pos(0)]);
    contradiction.add_clause(vec![Lit::neg(0)]);
    assert_eq!(contradiction.solve_under_assumptions_budgeted(&[Lit::pos(0)], 0), BudgetedResult::Unsat);
    assert!(contradiction.unsat_core().is_empty(), "the formula itself refuted needs no assumption");
}

#[test]
fn a_justifying_search_makes_every_side_clause_true_and_any_value_of_what_it_leaves_open_is_a_model() {
    // Random circuits with side clauses over inputs and gates, one justifying solver each answering a run of queries
    // on gate values: each verdict that of a plain solver holding the side clauses as clauses, and in each model every
    // side clause holds a literal set true, and every value of the inputs left open evaluates the circuit to the values
    // set, the assumptions and every side clause.
    let mut next = rng(0xD1B5_4A32_D192_ED03);
    let (mut sat, mut unsat, mut open) = (0, 0, 0);
    for trial in 0..80 {
        let circuit = Circuit::random(&mut next, 4 + trial % 5, 16 + 2 * (trial % 9));
        let n = circuit.vars();
        let sides: Vec<Vec<Lit>> = (0..1 + trial % 4)
            .map(|_| (0..1 + next() % 3).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect())
            .collect();
        let mut s = circuit.justifying();
        let mut plain = Solver::new(n);
        for c in circuit.clauses() {
            plain.add_clause(c);
        }
        for c in &sides {
            s.add_side_clause(c.clone());
            plain.add_clause(c.clone());
        }
        for _ in 0..8 {
            let gates = circuit.gates.len() as u64;
            let gate = |next: &mut dyn FnMut() -> u64| (circuit.inputs as u64 + next() % gates) as Var;
            let asm: Vec<Lit> = (0..1 + next() % 3).map(|_| Lit::new(gate(&mut next), next() & 1 == 0)).collect();
            let expected = plain.solve_under_assumptions_budgeted(&asm, 0);
            match s.solve_under_assumptions_budgeted(&asm, 0) {
                BudgetedResult::Sat(_) => {
                    assert!(matches!(expected, BudgetedResult::Sat(_)), "trial {trial}: {asm:?}");
                    let value: Vec<Option<bool>> = (0..n).map(|v| s.value(v)).collect();
                    for c in &sides {
                        let set_true = c.iter().any(|&l| value[l.var() as usize] == Some(l.is_positive()));
                        assert!(set_true, "trial {trial}: side clause {c:?} under {asm:?}");
                    }
                    let left: Vec<usize> = (0..circuit.inputs).filter(|&i| value[i].is_none()).collect();
                    open += left.len();
                    for bits in 0..1u64 << left.len() {
                        let mut inputs: Vec<bool> = (0..circuit.inputs).map(|i| value[i].unwrap_or(false)).collect();
                        for (k, &i) in left.iter().enumerate() {
                            inputs[i] = bits >> k & 1 == 1;
                        }
                        let full = circuit.evaluate(&inputs);
                        let read = |l: Lit| full[l.var() as usize] == l.is_positive();
                        for (v, set) in value.iter().enumerate() {
                            assert!(set.is_none_or(|b| full[v] == b), "trial {trial}: variable {v} with {bits:b} open");
                        }
                        assert!(asm.iter().all(|&l| read(l)), "trial {trial}: {asm:?} with {bits:b} open");
                        assert!(sides.iter().all(|c| c.iter().any(|&l| read(l))), "trial {trial}: with {bits:b} open");
                    }
                    sat += 1;
                }
                BudgetedResult::Unsat => {
                    assert_eq!(expected, BudgetedResult::Unsat, "trial {trial}: {asm:?}");
                    unsat += 1;
                }
                BudgetedResult::Budget => unreachable!(),
            }
        }
    }
    assert!(sat > 150 && unsat > 60 && open > 100, "{sat} satisfiable, {unsat} refuted, {open} inputs left open");
}

#[test]
fn a_side_clause_is_made_true_by_a_decision_and_leaves_the_logic_nothing_needs_open() {
    // Two conjunction chains on their own variables, and a side clause over two inputs of the long one: asking the
    // short chain's output false justifies it down to one of its inputs and decides one literal of the side clause.
    // Of the long chain's inputs only that one is set, and of its gates only those that input false makes false, the
    // gates after it; every gate before it is open.
    let mut s = Solver::new(0);
    let left = and_chain(&mut s, 8);
    let right = and_chain(&mut s, 40);
    for chain in [&left, &right] {
        let width = (chain.len() + 1) / 2;
        for k in 1..width {
            let acc = if k == 1 { chain[0] } else { chain[width + k - 2] };
            s.set_gate(chain[width + k - 1], Lit::pos(acc as u32), Lit::pos(chain[k] as u32));
        }
    }
    s.set_justifying(true);
    s.add_side_clause(vec![Lit::neg(right[3] as u32), Lit::neg(right[17] as u32)]);
    let out = *left.last().unwrap() as u32;
    let BudgetedResult::Sat(_) = s.solve_under_assumptions_budgeted(&[Lit::neg(out)], 0) else { panic!("satisfiable") };
    assert_eq!(s.value(out as usize), Some(false));
    let decided: Vec<usize> = [3, 17].into_iter().filter(|&k| s.value(right[k]) == Some(false)).collect();
    assert_eq!(decided.len(), 1, "one literal of the side clause set");
    let at = decided[0];
    let inputs_set: Vec<usize> = (0..40).filter(|&k| s.value(right[k]).is_some()).collect();
    assert_eq!(inputs_set, vec![at], "of the long chain's inputs only the one decided");
    for k in 1..40 {
        let gate = s.value(right[40 + k - 1]);
        assert_eq!(gate, if k >= at { Some(false) } else { None }, "the long chain's gate {k}, input {at} decided");
    }
    assert!(s.decisions() <= 8, "the short chain's decisions and one more: {}", s.decisions());
    s.add_clause(vec![Lit::pos(right[3] as u32)]);
    s.add_clause(vec![Lit::pos(right[17] as u32)]);
    assert_eq!(s.solve_under_assumptions_budgeted(&[Lit::neg(out)], 0), BudgetedResult::Unsat, "the side clause binds");
}

/// A circuit of cover gates: its inputs, then gates each a random function of two to four earlier variables, defined by
/// the clauses of the sums of products of the function and of its complement ([`crate::aig_cnf::isop`]) and given to
/// the solver as a cover gate.
struct CoverCircuit {
    inputs: usize,
    gates: Vec<(usize, Vec<Var>, u64)>,
}

impl CoverCircuit {
    fn random(next: &mut impl FnMut() -> u64, inputs: usize, gates: usize) -> CoverCircuit {
        let mut c = CoverCircuit { inputs, gates: Vec::new() };
        for g in inputs..inputs + gates {
            let n = (2 + next() % 3).min(g as u64) as usize;
            let mut leaves: Vec<Var> = Vec::new();
            while leaves.len() < n {
                let v = (next() % g as u64) as Var;
                if !leaves.contains(&v) {
                    leaves.push(v);
                }
            }
            let width = 1u32 << n;
            let mut truth = next() & ((1u64 << width) - 1);
            for rep in 1..64 / width {
                truth |= (truth & ((1u64 << width) - 1)) << (rep * width);
            }
            c.gates.push((g, leaves, truth));
        }
        c
    }

    fn vars(&self) -> usize {
        self.inputs + self.gates.len()
    }

    /// Gate `k`'s products for true and for false.
    fn products(&self, k: usize) -> (Vec<Vec<Lit>>, Vec<Vec<Lit>>) {
        let (_, leaves, truth) = &self.gates[k];
        let product = |c: &crate::aig_cnf::Cube| -> Vec<Lit> {
            let mut lits = Vec::new();
            for (i, &v) in leaves.iter().enumerate() {
                if c.pos >> i & 1 == 1 {
                    lits.push(Lit::pos(v));
                } else if c.neg >> i & 1 == 1 {
                    lits.push(Lit::neg(v));
                }
            }
            lits
        };
        let n = leaves.len();
        (
            crate::aig_cnf::isop(*truth, n).iter().map(product).collect(),
            crate::aig_cnf::isop(!*truth, n).iter().map(product).collect(),
        )
    }

    fn clauses(&self) -> Vec<Vec<Lit>> {
        let mut out = Vec::new();
        for (k, &(g, ..)) in self.gates.iter().enumerate() {
            let (on, off) = self.products(k);
            for (cubes, value) in [(on, Lit::pos(g as Var)), (off, Lit::neg(g as Var))] {
                for c in cubes {
                    out.push(std::iter::once(value).chain(c.iter().map(|l| l.negated())).collect());
                }
            }
        }
        out
    }

    /// The solver over the circuit, deciding by justification and tracing LRAT, clause `k` of
    /// [`CoverCircuit::clauses`] under id `k + 1`.
    fn justifying(&self) -> Solver {
        let mut s = Solver::new(self.vars());
        s.set_lrat(true);
        for (k, c) in self.clauses().into_iter().enumerate() {
            s.add_clause_with_id(c, k as u64 + 1);
        }
        for (k, &(g, ..)) in self.gates.iter().enumerate() {
            let (on, off) = self.products(k);
            s.set_cover_gate(g, &on, &off);
        }
        s.set_justifying(true);
        s
    }

    /// Every variable's value with the inputs as `inputs` gives them.
    fn evaluate(&self, inputs: &[bool]) -> Vec<bool> {
        let mut v = inputs[..self.inputs].to_vec();
        for (_, leaves, truth) in &self.gates {
            let at = leaves.iter().enumerate().fold(0usize, |x, (i, &l)| x | (v[l as usize] as usize) << i);
            let value = truth >> at & 1 == 1;
            v.push(value);
        }
        v
    }
}

#[test]
fn a_justifying_search_over_cover_gates_decides_as_plain_cdcl_its_models_evaluating_to_its_assumptions() {
    // Random circuits of cover gates, one justifying solver each answering a run of queries on gate values with every
    // clause it learns kept: each verdict that of a plain solver; each model's inputs, the ones it leaves open read
    // false and then at random, evaluating to the values assumed; and each refutation's chain replaying from the
    // circuit and the learned clauses.
    let mut next = rng(0xC0DE_CAFE_F00D_0001);
    let (mut sat, mut unsat, mut open) = (0, 0, 0u64);
    for trial in 0..60 {
        let circuit = CoverCircuit::random(&mut next, 4 + trial % 6, 15 + 2 * (trial % 10));
        let original = circuit.clauses();
        let mut s = circuit.justifying();
        s.set_reduce_limit(8);
        let mut steps: Vec<crate::lrat::LratStep> = Vec::new();
        let mut claims: Vec<Vec<Lit>> = Vec::new();
        let mut synced = 0;
        for _ in 0..25 {
            let width = circuit.vars() as u64;
            let asm: Vec<Lit> = (0..1 + next() % 3)
                .map(|_| Lit::new((circuit.inputs as u64 + next() % (width - circuit.inputs as u64)) as Var, next() & 1 == 0))
                .collect();
            let mut plain = Solver::new(circuit.vars());
            for c in &original {
                plain.add_clause(c.clone());
            }
            let truth = plain.solve_under_assumptions_budgeted(&asm, 0);
            let verdict = s.solve_under_assumptions_budgeted(&asm, 0);
            for c in &s.learned()[synced..] {
                steps.push(crate::lrat::LratStep { id: c.id, clause: c.lits.clone(), hints: c.hints.clone() });
            }
            synced = s.learned().len();
            match (&truth, &verdict) {
                (BudgetedResult::Sat(_), BudgetedResult::Sat(m)) => {
                    let mut inputs: Vec<bool> = m[..circuit.inputs].to_vec();
                    for round in 0..4 {
                        let values = circuit.evaluate(&inputs);
                        for a in &asm {
                            let got = values[a.var() as usize] == a.is_positive();
                            assert!(got, "trial {trial}, completion {round}: the inputs evaluate to {asm:?}");
                        }
                        for (i, x) in inputs.iter_mut().enumerate() {
                            if s.value(i).is_none() {
                                *x = next() & 1 == 1;
                                open += (round == 0) as u64;
                            }
                        }
                    }
                    sat += 1;
                }
                (BudgetedResult::Unsat, BudgetedResult::Unsat) => {
                    let negated: Vec<Lit> = asm.iter().map(|a| a.negated()).collect();
                    let id = s.next_clause_id();
                    steps.push(crate::lrat::LratStep { id, clause: negated.clone(), hints: s.unsat_hints().to_vec() });
                    s.add_clause_with_id(negated.clone(), id);
                    claims.push(negated);
                    unsat += 1;
                }
                (t, v) => panic!("trial {trial}, {asm:?}: plain {t:?}, justifying {v:?}"),
            }
        }
        let proof = crate::lrat::Lrat { steps };
        assert!(crate::lrat::check_derivation(circuit.vars(), &original, &proof, &claims), "trial {trial}");
    }
    assert!(sat > 300 && unsat > 100 && open > 300, "{sat} SAT, {unsat} UNSAT, {open} inputs left open");
}

#[test]
fn a_cover_gate_is_justified_by_one_product_of_its_value_and_its_other_leaves_stay_open() {
    // g is (a ∧ b) ∨ (¬a ∧ c) over three inputs, beside an unrelated chain. Asking g true decides one product: a and b
    // true, or a false and c true, the third input and the chain never assigned.
    let mut s = Solver::new(3);
    let chain = and_chain(&mut s, 30);
    for k in 1..30 {
        let acc = if k == 1 { chain[0] } else { chain[30 + k - 2] };
        s.set_gate(chain[30 + k - 1], Lit::pos(acc as u32), Lit::pos(chain[k] as u32));
    }
    let g = s.add_vars(1);
    let (a, b, c) = (Lit::pos(0), Lit::pos(1), Lit::pos(2));
    let on = vec![vec![a, b], vec![a.negated(), c]];
    let off = vec![vec![a, b.negated()], vec![a.negated(), c.negated()]];
    for (cubes, value) in [(&on, Lit::pos(g as Var)), (&off, Lit::neg(g as Var))] {
        for cube in cubes.iter() {
            s.add_clause(std::iter::once(value).chain(cube.iter().map(|l| l.negated())).collect());
        }
    }
    s.set_cover_gate(g, &on, &off);
    s.set_justifying(true);
    for want in [true, false] {
        match s.solve_under_assumptions_budgeted(&[Lit::new(g as Var, want)], 0) {
            BudgetedResult::Sat(_) => {
                let set: Vec<Option<bool>> = (0..3).map(|v| s.value(v)).collect();
                let held = if want { &on } else { &off };
                let product = held.iter().find(|p| p.iter().all(|l| s.value(l.var() as usize) == Some(l.is_positive())));
                assert!(product.is_some(), "g = {want}: a product of its value holds: {set:?}");
                assert_eq!(set.iter().filter(|v| v.is_some()).count(), 2, "g = {want}: two leaves set: {set:?}");
                assert!(chain.iter().all(|&v| s.value(v).is_none()), "g = {want}: the chain is never assigned");
            }
            other => panic!("g = {want}: {other:?}"),
        }
    }
}
