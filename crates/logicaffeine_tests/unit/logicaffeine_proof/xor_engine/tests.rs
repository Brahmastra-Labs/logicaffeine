use super::*;
use crate::xorsat::XorEquation;

/// Brute-force oracle: enumerate every assignment of the unassigned variables consistent with
/// `assign` that satisfies all equations. Returns `None` if there are none (UNSAT under `assign`),
/// else `Some(per-variable forced value)` where a variable is forced iff it takes the same value
/// across every consistent completion.
fn oracle(
    num_vars: usize,
    eqs: &[XorEquation],
    assign: &[Option<bool>],
) -> Option<Vec<Option<bool>>> {
    let free: Vec<usize> = (0..num_vars).filter(|&v| assign[v].is_none()).collect();
    let mut seen: Vec<Option<bool>> = vec![None; num_vars];
    let mut first = true;
    let mut any = false;
    assert!(free.len() <= 16, "oracle only for small systems");
    for mask in 0u32..(1u32 << free.len()) {
        let mut full = vec![false; num_vars];
        for v in 0..num_vars {
            full[v] = assign[v].unwrap_or(false);
        }
        for (i, &v) in free.iter().enumerate() {
            full[v] = (mask >> i) & 1 == 1;
        }
        let ok = eqs
            .iter()
            .all(|e| e.vars.iter().fold(false, |a, &v| a ^ full[v]) == e.rhs);
        if !ok {
            continue;
        }
        any = true;
        if first {
            for v in 0..num_vars {
                seen[v] = Some(full[v]);
            }
            first = false;
        } else {
            for v in 0..num_vars {
                if seen[v] != Some(full[v]) {
                    seen[v] = None; // varies ⇒ not forced
                }
            }
        }
    }
    if any {
        // Only the unassigned variables count as "forced" discoveries.
        for v in 0..num_vars {
            if assign[v].is_some() {
                seen[v] = None;
            }
        }
        Some(seen)
    } else {
        None
    }
}

fn lit_sat(clause: &[Lit], full: &[bool]) -> bool {
    clause
        .iter()
        .any(|l| full[l.var() as usize] == l.is_positive())
}

#[test]
fn forces_a_variable_by_combining_two_equations() {
    // x0⊕x1=1, x0⊕x2=1 ⇒ x1⊕x2=0 (cross-equation). Assign x1=true ⇒ x2 forced true; x0 false too.
    let eqs = vec![
        XorEquation::new(vec![0, 1], true),
        XorEquation::new(vec![0, 2], true),
    ];
    let eng = XorEngine::new(3, &eqs);
    let assign = vec![None, Some(true), None];
    let forced = eng.all_consequences(&assign).expect("system is consistent");
    let (x2, reason) = forced
        .iter()
        .find(|(l, _)| l.var() == 2)
        .expect("x2 must be forced");
    assert!(x2.is_positive(), "x2 must be forced true");
    // the reason must be unit on x2 (its only unassigned literal).
    let unassigned: Vec<_> = reason
        .iter()
        .filter(|l| assign[l.var() as usize].is_none())
        .collect();
    assert_eq!(unassigned.len(), 1);
    assert_eq!(unassigned[0].var(), 2);
}

#[test]
fn dpll_xor_decides_a_small_mixed_instance_via_the_theory() {
    use crate::cdcl::{SolveResult, Solver};
    // CNF = the 4 gadget clauses of x0⊕x1⊕x2=0, plus units x0 and x1 ⇒ SAT with x2=false.
    let gadget = vec![
        vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, false)],
        vec![Lit::new(0, true), Lit::new(1, false), Lit::new(2, true)],
        vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, true)],
        vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, false)],
        vec![Lit::new(0, true)],
        vec![Lit::new(1, true)],
    ];
    let mut solver = Solver::new(3);
    for c in &gadget {
        solver.add_clause(c.clone());
    }
    let engine = XorEngine::new(3, &[XorEquation::new(vec![0, 1, 2], false)]);
    let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(engine)];
    match solver.solve_with(&mut theories) {
        SolveResult::Sat(m) => {
            for c in &gadget {
                assert!(
                    c.iter().any(|l| m[l.var() as usize] == l.is_positive()),
                    "model fails {c:?}"
                );
            }
            assert!(m[0] && m[1] && !m[2], "expected x0,x1 true and x2 false");
        }
        SolveResult::Unsat => panic!("instance is SAT"),
    }
}

#[test]
fn live_dpll_xor_matches_plain_cdcl_on_random_xor_formulas() {
    use crate::cdcl::{SolveResult, Solver};
    // The decisive integration gate: running the live IncXor as a CDCL theory must give the SAME
    // verdict as plain CDCL on the identical formula — for hundreds of random instances built
    // from real XOR structure plus residual clauses. The theory may only *carry* globally-valid
    // no-goods (every XOR consequence holds in every model), so it can never change the answer,
    // only the path. On SAT the returned model must satisfy every clause.
    let mut rng: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    // Expand a GF(2) equation to its CNF gadget clauses (each globally valid).
    let gadget_of = |e: &XorEquation, _nv: usize| -> Vec<Vec<Lit>> {
        let vs = &e.vars;
        let k = vs.len();
        let mut out = Vec::new();
        for mask in 0u32..(1u32 << k) {
            if ((mask.count_ones() % 2) == 1) != e.rhs {
                out.push(
                    (0..k)
                        .map(|i| Lit::new(vs[i] as u32, (mask >> i) & 1 == 0))
                        .collect(),
                );
            }
        }
        out
    };
    for _ in 0..400 {
        let num_vars = 4 + (next() % 7) as usize; // 4..=10
        let n_eqs = 1 + (next() % 4) as usize;
        let mut eqs = Vec::new();
        let mut clauses: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..n_eqs {
            let mut vars: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
            vars.truncate(4); // keep gadgets small
            if vars.is_empty() {
                continue;
            }
            let e = XorEquation::new(vars, next() % 2 == 0);
            clauses.extend(gadget_of(&e, num_vars));
            eqs.push(e);
        }
        if eqs.is_empty() {
            continue;
        }
        // A few residual clauses (units/binaries) to bias toward the hard SAT/UNSAT boundary.
        for _ in 0..(2 + next() % 4) {
            let w = 1 + (next() % 2) as usize;
            let c: Vec<Lit> = (0..w)
                .map(|_| Lit::new((next() % num_vars as u64) as u32, next() % 2 == 0))
                .collect();
            clauses.push(c);
        }

        let mut plain = Solver::new(num_vars);
        for c in &clauses {
            plain.add_clause(c.clone());
        }
        let truth = plain.solve();

        let mut live = Solver::new(num_vars);
        for c in &clauses {
            live.add_clause(c.clone());
        }
        let engine = IncXor::new(num_vars, &eqs);
        let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(engine)];
        let got = live.solve_with(&mut theories);

        match (&truth, &got) {
            (SolveResult::Sat(_), SolveResult::Sat(m)) => {
                assert!(
                    clauses
                        .iter()
                        .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                    "live model fails a clause (eqs={eqs:?})"
                );
            }
            (SolveResult::Unsat, SolveResult::Unsat) => {}
            _ => panic!(
                "verdict mismatch: plain={truth:?} live={got:?} (eqs={eqs:?} clauses={clauses:?})"
            ),
        }
    }
}

#[test]
#[ignore = "diagnostic: reads par32 from disk and prints the GF(2) search dimensions"]
fn par32_search_dimensions() {
    for name in ["par16-1-c", "par32-1-c", "par32-1"] {
        let path = format!("../../benchmarks/arena/instances/sat/crafted_parity/{name}.cnf");
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("{name}: missing");
            continue;
        };
        let cnf = crate::dimacs::parse(&text).expect("valid dimacs");
        let eqs = crate::lyapunov::extract_xor(cnf.num_vars, &cnf.clauses);
        let engine = IncXor::new(cnf.num_vars, &eqs);
        let (rows, avgw, maxw) = engine.density();
        eprintln!(
            "{name}: vars={} clauses={} xor_eqs={} | rank={} kernel_dim={} decision_vars={} | rows={} avg_w={:.1} max_w={}",
            cnf.num_vars,
            cnf.clauses.len(),
            eqs.len(),
            engine.rank(),
            engine.kernel_dim(),
            engine.decision_vars().len(),
            rows,
            avgw,
            maxw,
        );
    }
}

#[test]
#[ignore = "diagnostic: measures how far probing collapses par32's GF(2) kernel"]
fn par32_probing_collapses_kernel() {
    use crate::xorsat::XorEquation;
    // Full propagation under a single assumed literal: residual-clause BCP interleaved with live
    // Gaussian, to a fixpoint. Returns the forced assignment, or None on conflict (a failed literal).
    fn probe(
        num_vars: usize,
        clauses: &[Vec<Lit>],
        eqs: &[XorEquation],
        start: Lit,
    ) -> Option<Vec<Option<bool>>> {
        let mut val: Vec<Option<bool>> = vec![None; num_vars];
        let mut in_engine = vec![false; num_vars];
        let mut engine = IncXor::new(num_vars, eqs);
        val[start.var() as usize] = Some(start.is_positive());
        let mut progress = true;
        while progress {
            progress = false;
            for v in 0..num_vars {
                if val[v].is_some() && !in_engine[v] {
                    engine.assign(v, val[v].unwrap());
                    in_engine[v] = true;
                }
            }
            match engine.state() {
                Err(_) => return None,
                Ok(forced) => {
                    for (lit, _) in forced {
                        let v = lit.var() as usize;
                        match val[v] {
                            None => {
                                val[v] = Some(lit.is_positive());
                                progress = true;
                            }
                            Some(b) => {
                                if b != lit.is_positive() {
                                    return None;
                                }
                            }
                        }
                    }
                }
            }
            for c in clauses {
                let mut sat = false;
                let mut unset: Option<Lit> = None;
                let mut n_unset = 0;
                for &l in c {
                    match val[l.var() as usize] {
                        Some(b) if b == l.is_positive() => {
                            sat = true;
                            break;
                        }
                        Some(_) => {}
                        None => {
                            n_unset += 1;
                            unset = Some(l);
                        }
                    }
                }
                if sat {
                    continue;
                }
                if n_unset == 0 {
                    return None;
                }
                if n_unset == 1 {
                    let l = unset.unwrap();
                    if val[l.var() as usize].is_none() {
                        val[l.var() as usize] = Some(l.is_positive());
                        progress = true;
                    }
                }
            }
        }
        Some(val)
    }

    for name in ["par16-1-c", "par32-1-c"] {
        let path = format!("../../benchmarks/arena/instances/sat/crafted_parity/{name}.cnf");
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("{name}: missing");
            continue;
        };
        let cnf = crate::dimacs::parse(&text).expect("valid dimacs");
        let eqs = crate::lyapunov::extract_xor(cnf.num_vars, &cnf.clauses);
        let base = IncXor::new(cnf.num_vars, &eqs);
        let kernel = base.decision_vars();
        let kernel0 = base.kernel_dim();

        let mut folded = eqs.clone();
        let (mut failed, mut backbone, mut equiv) = (0usize, 0usize, 0usize);
        for &x in &kernel {
            let f1 = probe(cnf.num_vars, &cnf.clauses, &eqs, Lit::new(x as u32, true));
            let f0 = probe(cnf.num_vars, &cnf.clauses, &eqs, Lit::new(x as u32, false));
            match (&f1, &f0) {
                (None, None) => {} // both polarities conflict ⇒ the whole instance is UNSAT-under-XOR
                (None, Some(_)) => {
                    failed += 1;
                    folded.push(XorEquation::new(vec![x], false));
                }
                (Some(_), None) => {
                    failed += 1;
                    folded.push(XorEquation::new(vec![x], true));
                }
                (Some(a1), Some(a0)) => {
                    for y in 0..cnf.num_vars {
                        if y == x {
                            continue;
                        }
                        if let (Some(v1), Some(v0)) = (a1[y], a0[y]) {
                            if v1 == v0 {
                                backbone += 1;
                                folded.push(XorEquation::new(vec![y], v1));
                            } else {
                                equiv += 1;
                                folded.push(XorEquation::new(vec![x, y], v0)); // x=F⇒y=v0 ⇒ x⊕y=v0
                            }
                        }
                    }
                }
            }
        }
        let after = IncXor::new(cnf.num_vars, &folded);
        eprintln!(
            "{name}: kernel {} → {} | probes found failed_lits={} backbone_hits={} equiv_pairs={} (raw counts, pre-dedup)",
            kernel0,
            after.kernel_dim(),
            failed,
            backbone,
            equiv,
        );
    }
}

#[test]
fn detects_a_linear_contradiction() {
    // x0⊕x1=0, x1⊕x2=0, x0⊕x2=1 sum to 0=1: UNSAT with no assignment. The certificate is the
    // empty clause (an unconditional contradiction), which is correct.
    let eqs = vec![
        XorEquation::new(vec![0, 1], false),
        XorEquation::new(vec![1, 2], false),
        XorEquation::new(vec![0, 2], true),
    ];
    let eng = XorEngine::new(3, &eqs);
    match eng.analyze(&vec![None, None, None]) {
        XorStep::Conflict(_) => {}
        other => panic!("expected conflict, got {other:?}"),
    }
}

#[test]
fn next_branch_picks_a_free_kernel_var_and_breaking_it_collapses_the_rest() {
    // x0⊕x1=0, x2⊕x3=0: pivots x0,x2; free (kernel) x1,x3. The engine must offer a free var as
    // the break point, and breaking it forces its pivot partner (the collapse).
    let eqs = vec![
        XorEquation::new(vec![0, 1], false),
        XorEquation::new(vec![2, 3], false),
    ];
    let mut inc = IncXor::new(4, &eqs);
    assert_eq!(inc.rank(), 2);
    assert_eq!(inc.kernel_dim(), 2);
    let b = inc.next_branch().expect("a free kernel var to break on");
    assert!(
        b == 1 || b == 3,
        "break point must be a non-pivot free var, got {b}"
    );
    inc.assign(b, true);
    let forced = inc.state().expect("consistent");
    assert!(
        !forced.is_empty(),
        "breaking a kernel var must force its pivot partner"
    );
}

#[test]
fn incremental_engine_matches_the_recompute_oracle_under_random_trails() {
    // The fast IncXor, driven by random assign/unassign (push/pop) sequences, must report the
    // SAME forced-literal set and conflict status as the recompute XorEngine at EVERY step.
    let mut rng: u64 = 0xD1B5_4A32_D192_ED03;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let forced_set = |r: &Result<Vec<(Lit, Vec<Lit>)>, Vec<Lit>>| -> Option<std::collections::BTreeSet<(u32, bool)>> {
        match r {
            Err(_) => None, // conflict
            Ok(f) => Some(f.iter().map(|(l, _)| (l.var(), l.is_positive())).collect()),
        }
    };
    for _ in 0..2000 {
        let num_vars = 3 + (next() % 6) as usize; // 3..=8
        let n_eqs = 1 + (next() % 6) as usize;
        let mut eqs = Vec::new();
        for _ in 0..n_eqs {
            let vars: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
            if !vars.is_empty() {
                eqs.push(XorEquation::new(vars, next() % 2 == 0));
            }
        }
        if eqs.is_empty() {
            continue;
        }
        let oracle = XorEngine::new(num_vars, &eqs);
        let mut inc = IncXor::new(num_vars, &eqs);
        let mut assign: Vec<Option<bool>> = vec![None; num_vars];
        let mut stack: Vec<usize> = Vec::new();

        for _step in 0..40 {
            let free: Vec<usize> = (0..num_vars).filter(|&v| assign[v].is_none()).collect();
            let push = !free.is_empty() && (stack.is_empty() || next() % 2 == 0);
            if push {
                let v = free[(next() % free.len() as u64) as usize];
                let val = next() % 2 == 0;
                inc.assign(v, val);
                assign[v] = Some(val);
                stack.push(v);
            } else if let Some(v) = stack.pop() {
                inc.unassign();
                assign[v] = None;
            }
            inc.check_occ();

            let got = inc.state();
            let truth = oracle.all_consequences(&assign);
            // conflict status must match
            assert_eq!(
                got.is_err(),
                truth.is_err(),
                "conflict mismatch: inc={:?} oracle_err={} (eqs={eqs:?} assign={assign:?})",
                got.is_err(),
                truth.is_err()
            );
            if got.is_ok() {
                assert_eq!(
                    forced_set(&got),
                    forced_set(&truth),
                    "forced-set mismatch (eqs={eqs:?} assign={assign:?})"
                );
                // every incremental reason must be unit on its literal (implied shape).
                if let Ok(f) = &got {
                    for (lit, reason) in f {
                        let un: Vec<_> = reason
                            .iter()
                            .filter(|l| assign[l.var() as usize].is_none())
                            .collect();
                        assert_eq!(un.len(), 1, "reason must be unit");
                        assert_eq!(un[0].var(), lit.var());
                    }
                }
            }
        }
    }
}

#[test]
fn matches_the_brute_force_oracle_exhaustively() {
    // Robustness to absurdity: random small GF(2) systems × random partial assignments, the
    // engine's forced-set and consistency must match the brute-force oracle EXACTLY, and every
    // emitted clause must be implied (true in all solutions) and correctly unit/falsified.
    let mut rng: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for _ in 0..3000 {
        let num_vars = 3 + (next() % 5) as usize; // 3..=7
        let n_eqs = 1 + (next() % 6) as usize;
        let mut eqs = Vec::new();
        for _ in 0..n_eqs {
            let vars: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
            if vars.is_empty() {
                continue;
            }
            eqs.push(XorEquation::new(vars, next() % 2 == 0));
        }
        if eqs.is_empty() {
            continue;
        }
        // random partial assignment
        let assign: Vec<Option<bool>> = (0..num_vars)
            .map(|_| match next() % 3 {
                0 => Some(true),
                1 => Some(false),
                _ => None,
            })
            .collect();

        let eng = XorEngine::new(num_vars, &eqs);
        let got = eng.all_consequences(&assign);
        let truth = oracle(num_vars, &eqs, &assign);

        match (got, truth) {
            (Err(conflict), None) => {
                // A genuine conflict: every literal is false under the current assignment...
                for l in &conflict {
                    assert_eq!(
                        assign[l.var() as usize],
                        Some(!l.is_positive()),
                        "conflict literal must be false under the assignment"
                    );
                }
                // ...and the clause is IMPLIED — true in every full solution of the system.
                for mask in 0u32..(1u32 << num_vars) {
                    let full: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
                    if eqs
                        .iter()
                        .all(|e| e.vars.iter().fold(false, |a, &x| a ^ full[x]) == e.rhs)
                    {
                        assert!(lit_sat(&conflict, &full), "conflict clause must be implied");
                    }
                }
            }
            (Ok(forced), Some(truth_forced)) => {
                // Every engine-forced var must agree with the oracle.
                for (lit, reason) in &forced {
                    let v = lit.var() as usize;
                    assert_eq!(truth_forced[v], Some(lit.is_positive()),
                        "engine forced x{v}={} but oracle disagrees (eqs={eqs:?}, assign={assign:?})", lit.is_positive());
                    // reason must be unit: exactly one unassigned literal, and it is `lit`.
                    let un: Vec<_> = reason
                        .iter()
                        .filter(|l| assign[l.var() as usize].is_none())
                        .collect();
                    assert_eq!(un.len(), 1, "reason must be unit");
                    assert_eq!(un[0].var(), lit.var());
                    // reason must be IMPLIED: true in every consistent completion.
                    for mask in 0u32..(1u32 << num_vars) {
                        let full: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
                        let sat_all = eqs
                            .iter()
                            .all(|e| e.vars.iter().fold(false, |a, &x| a ^ full[x]) == e.rhs);
                        if sat_all {
                            assert!(
                                lit_sat(reason, &full),
                                "reason clause must hold in every solution"
                            );
                        }
                    }
                }
                // Completeness: every oracle-forced unassigned var must be found by the engine.
                let engine_vars: std::collections::HashSet<usize> =
                    forced.iter().map(|(l, _)| l.var() as usize).collect();
                for v in 0..num_vars {
                    if assign[v].is_none() && truth_forced[v].is_some() {
                        assert!(
                            engine_vars.contains(&v),
                            "engine missed forced x{v} (eqs={eqs:?}, assign={assign:?})"
                        );
                    }
                }
            }
            (Ok(forced), None) => panic!(
                "engine missed a contradiction: forced={forced:?} eqs={eqs:?} assign={assign:?}"
            ),
            (Err(c), Some(_)) => {
                panic!("engine reported a false conflict {c:?}: eqs={eqs:?} assign={assign:?}")
            }
        }
    }
}
