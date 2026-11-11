use super::*;
use crate::cdcl::Theory;
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

/// The clauses of `e`'s gadget: every assignment of its variables of the wrong parity, excluded.
fn gadget(e: &XorEquation) -> Vec<Vec<Lit>> {
    let k = e.vars.len();
    (0u32..1 << k)
        .filter(|mask| (mask.count_ones() % 2 == 1) != e.rhs)
        .map(|mask| (0..k).map(|i| Lit::new(e.vars[i] as u32, mask >> i & 1 == 0)).collect())
        .collect()
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Random equations over `num_vars` variables: random supports, now and then a unit, a repeated variable, a copy of an
/// earlier equation or the sum of two, so components of every rank and inconsistent systems all occur.
fn random_equations(rng: &mut u64, num_vars: usize, count: usize) -> Vec<XorEquation> {
    let mut eqs: Vec<XorEquation> = Vec::new();
    for _ in 0..count {
        let e = match xorshift(rng) % 8 {
            0 => XorEquation::new(vec![(xorshift(rng) % num_vars as u64) as usize], xorshift(rng) % 2 == 0),
            1 if !eqs.is_empty() => {
                let a = &eqs[(xorshift(rng) % eqs.len() as u64) as usize];
                XorEquation::new(a.vars.clone(), xorshift(rng) % 2 == 0)
            }
            2 if eqs.len() >= 2 => {
                let a = eqs[(xorshift(rng) % eqs.len() as u64) as usize].clone();
                let b = eqs[(xorshift(rng) % eqs.len() as u64) as usize].clone();
                let mut vars = a.vars.clone();
                vars.extend(b.vars.iter().copied());
                XorEquation::new(vars, a.rhs ^ b.rhs ^ (xorshift(rng) % 4 == 0))
            }
            3 => {
                let v = (xorshift(rng) % num_vars as u64) as usize;
                let w = (xorshift(rng) % num_vars as u64) as usize;
                XorEquation::new(vec![v, w, v], xorshift(rng) % 2 == 0)
            }
            _ => {
                let width = 2 + (xorshift(rng) % 4) as usize;
                let mut vars: Vec<usize> = (0..width).map(|_| (xorshift(rng) % num_vars as u64) as usize).collect();
                vars.sort_unstable();
                vars.dedup();
                XorEquation::new(vars, xorshift(rng) % 2 == 0)
            }
        };
        eqs.push(e);
    }
    eqs
}

fn satisfies(eqs: &[XorEquation], mask: u32) -> bool {
    eqs.iter().all(|e| e.vars.iter().fold(false, |a, &v| a ^ (mask >> v & 1 == 1)) == e.rhs)
}

fn agrees(trail: &[Lit], mask: u32) -> bool {
    trail.iter().all(|l| (mask >> l.var() & 1 == 1) == l.is_positive())
}

/// The watched matrix's invariants against `trail`: the values and the assigned columns mirror it; each basic column
/// lies in its row alone; a row with a watch watches a free column of its own and is among that column's watchers,
/// which name no other row, at the place it records; a row without one has no free column and is a unit or on the
/// unwatched stack, whose trail lengths never decrease; and a row holding an unassigned column has its basic column
/// unassigned.
fn watched_invariants(x: &WatchedXor, trail: &[Lit]) {
    let mut value = vec![0i8; x.place.len()];
    for l in trail {
        value[l.var() as usize] = if l.is_positive() { 1 } else { -1 };
    }
    assert_eq!(x.value, value, "the values mirror the trail");
    assert_eq!(x.synced, trail, "the synced literals are the trail");
    assert!(x.unwatched.windows(2).all(|p| p[0].0 <= p[1].0), "the unwatched stack is in trail order");
    assert!(x.unwatched.iter().all(|&(at, _, _)| at <= trail.len()), "every unwatched row lost its watch on this trail");
    for (m, matrix) in x.matrices.iter().enumerate() {
        let (rank, columns) = (matrix.basic.len(), matrix.vars.len());
        let unassigned = |c: usize| value[matrix.vars[c] as usize] == 0;
        for c in 0..columns {
            assert_eq!(bit_get(&matrix.assigned, c), !unassigned(c), "the assigned columns mirror the trail");
            assert_eq!(x.place[matrix.vars[c] as usize], (m as u32, c as u32), "a column's variable is placed there");
            for (i, &r) in matrix.watchers[c].iter().enumerate() {
                assert_eq!(matrix.watch[r as usize], c as u32, "a watcher list names only rows watching its column");
                assert_eq!(matrix.slot[r as usize], i as u32, "a watching row knows its place among its column's watchers");
            }
        }
        let watching: usize = matrix.watchers.iter().map(Vec::len).sum();
        assert_eq!(watching, matrix.watch.iter().filter(|&&w| w != NONE).count(), "each watch is listed once");
        for r in 0..rank {
            let b = matrix.basic[r] as usize;
            assert_eq!(matrix.row_of[b], r as u32);
            assert!(bit_get(&matrix.basic_mask, b));
            for other in 0..rank {
                assert_eq!(matrix.holds(other, b as u32), other == r, "a basic column lies in its row alone");
            }
            let free: Vec<u32> = (0..columns as u32)
                .filter(|&c| matrix.holds(r, c) && unassigned(c as usize) && !bit_get(&matrix.basic_mask, c as usize))
                .collect();
            match matrix.watch[r] {
                NONE => {
                    assert!(free.is_empty(), "a row without a watch has no free column");
                    let listed = x.units.contains(&(m as u32, r as u32))
                        || x.unwatched.iter().filter(|&&(_, mm, rr)| (mm, rr) == (m as u32, r as u32)).count() == 1;
                    assert!(listed, "a row without a watch is a unit or on the unwatched stack once");
                }
                w => {
                    assert!(free.contains(&w), "a row watches a free column of its own");
                    assert!(!x.unwatched.iter().any(|&(_, mm, rr)| (mm, rr) == (m as u32, r as u32)));
                }
            }
            if (0..columns).any(|c| matrix.holds(r, c as u32) && unassigned(c)) {
                assert!(unassigned(b), "a row holding an unassigned column has its basic column unassigned");
            }
        }
        assert_eq!(matrix.basic_mask.iter().map(|w| w.count_ones() as usize).sum::<usize>(), rank);
    }
}

/// Drive a theory the way the solver does, on levels: a decision opens a level with a few more literals beside it, the
/// theory's propagations join the level they were found on, a conflict backjumps below it, and a backtrack drops to a
/// random level. After every call: each clause is implied by the equations and unit, false or satisfied; a call on a
/// trail no solution extends returns a false clause, and only then; and at the theory's fixpoint no unassigned
/// variable is forced. `feed` says how the theory is called: told the kept prefix or finding it, its forced literals
/// as clauses or as tokens explained at once — and every token explained again after each later call while its literal
/// stays assigned, to the same clause. `make` builds the theory from the equations and their solutions, `inspect`
/// looks inside it after each call. Returns the literals propagated, the conflicts and the fixpoints reached.
fn drive<T: Theory>(
    seed: u64,
    cases: usize,
    feed: Feed,
    make: impl Fn(usize, &[XorEquation], &[u32]) -> T,
    inspect: impl Fn(&T, &[Lit]),
) -> (usize, usize, usize) {
    let mut rng = seed;
    let (mut propagated, mut conflicts, mut fixpoints) = (0, 0, 0);
    for _ in 0..cases {
        let num_vars = 2 + (xorshift(&mut rng) % 9) as usize;
        let count = 1 + (xorshift(&mut rng) % 9) as usize;
        let eqs = random_equations(&mut rng, num_vars, count);
        let solutions: Vec<u32> = (0u32..1 << num_vars).filter(|&m| satisfies(&eqs, m)).collect();
        let mut x = make(num_vars, &eqs, &solutions);
        let (mut trail, mut lim, mut kept): (Vec<Lit>, Vec<usize>, usize) = (Vec::new(), Vec::new(), 0);
        // The lazily forced literals on the trail with their tokens and clauses, and those not yet pushed.
        let mut tokens: Vec<(usize, u32, Vec<Lit>)> = Vec::new();
        let mut pending: Vec<(u32, Vec<Lit>)> = Vec::new();
        let assigned = |trail: &[Lit], v: usize| trail.iter().any(|l| l.var() as usize == v);
        for _ in 0..30 {
            let free: Vec<usize> = (0..num_vars).filter(|&v| !assigned(&trail, v)).collect();
            if !free.is_empty() && (lim.is_empty() || xorshift(&mut rng) % 3 != 0) {
                if !trail.is_empty() || xorshift(&mut rng) % 2 == 0 {
                    lim.push(trail.len());
                }
                let extra = (xorshift(&mut rng) % 3) as usize;
                let chosen: Vec<usize> = free.iter().copied().filter(|_| xorshift(&mut rng) % 3 == 0).take(1 + extra).collect();
                for v in chosen {
                    trail.push(Lit::new(v as u32, xorshift(&mut rng) % 2 == 0));
                }
            } else if !lim.is_empty() {
                let level = (xorshift(&mut rng) % lim.len() as u64) as usize;
                trail.truncate(lim[level]);
                lim.truncate(level);
                kept = kept.min(trail.len());
            }
            loop {
                let given = kept;
                let mut forced: Vec<(Lit, u32)> = Vec::new();
                let mut out = match feed {
                    Feed::Trail => x.propagate(&trail),
                    Feed::Kept => x.propagate_kept(&trail, kept),
                    Feed::Lazy => x.propagate_lazily(&trail, kept, &mut forced),
                };
                kept = trail.len();
                inspect(&x, &trail);
                tokens.retain(|&(at, _, _)| at < trail.len());
                pending.clear();
                for &(_, token, ref clause) in &tokens {
                    let mut again = Vec::new();
                    x.explain(token, &mut again);
                    assert_eq!(&again, clause, "a token explains to the same clause while its literal stays assigned");
                }
                for &(lit, token) in &forced {
                    let mut clause = Vec::new();
                    x.explain(token, &mut clause);
                    assert_eq!(clause[0], lit, "a token's clause starts with the literal it forced");
                    if let Some(at) = trail.iter().position(|t| t.var() == lit.var()) {
                        tokens.push((at, token, clause.clone()));
                    } else {
                        pending.push((token, clause.clone()));
                    }
                    out.push(clause);
                }
                let val = |l: &Lit| trail.iter().find(|t| t.var() == l.var()).map(|t| t == l);
                for clause in &out {
                    assert!(
                        solutions.iter().all(|&s| clause.iter().any(|l| (s >> l.var() & 1 == 1) == l.is_positive())),
                        "a returned clause is implied: {clause:?} by {eqs:?}"
                    );
                    let open = clause.iter().filter(|l| val(l).is_none()).count();
                    assert!(open <= 1 || clause.iter().any(|l| val(l) == Some(true)), "a returned clause is unit, false or satisfied");
                }
                let extendable = solutions.iter().any(|&s| agrees(&trail, s));
                let refuted = out.iter().any(|c| c.iter().all(|l| val(l) == Some(false)));
                assert_eq!(refuted, !extendable, "a call returns a false clause exactly when no solution extends the trail: {eqs:?} {trail:?}");
                if refuted {
                    conflicts += 1;
                    if lim.is_empty() {
                        break;
                    }
                    let level = (xorshift(&mut rng) % lim.len() as u64) as usize;
                    trail.truncate(lim[level]);
                    lim.truncate(level);
                    kept = kept.min(trail.len());
                    assert!(trail.len() <= given, "a conflict backjumps below the literals it was found on");
                    continue;
                }
                let mut forced: Vec<Lit> = Vec::new();
                for clause in &out {
                    if clause.iter().any(|l| val(l) == Some(true)) {
                        continue;
                    }
                    let l = *clause.iter().find(|l| val(l).is_none()).expect("a unit clause has its literal open");
                    if !forced.iter().any(|f| f.var() == l.var()) {
                        forced.push(l);
                    }
                }
                if forced.is_empty() {
                    fixpoints += 1;
                    for v in (0..num_vars).filter(|&v| !assigned(&trail, v)) {
                        let mut values = solutions.iter().filter(|&&s| agrees(&trail, s)).map(|&s| s >> v & 1);
                        let first = values.next().expect("an extendable trail");
                        assert!(values.any(|b| b != first), "x{v} is forced at the fixpoint yet unassigned: {eqs:?} {trail:?}");
                    }
                    break;
                }
                propagated += forced.len();
                for l in forced {
                    if let Some(i) = pending.iter().position(|(_, c)| c[0] == l) {
                        let (token, clause) = pending.swap_remove(i);
                        tokens.push((trail.len(), token, clause));
                    }
                    trail.push(l);
                }
            }
            if lim.is_empty() && !solutions.iter().any(|&s| agrees(&trail, s)) {
                break;
            }
        }
    }
    (propagated, conflicts, fixpoints)
}

/// How the driver calls a theory.
#[derive(Clone, Copy)]
enum Feed {
    Trail,
    Kept,
    Lazy,
}

/// A watched matrix for the driver, its construction checked: inconsistency is found at once, and every row is
/// implied by the equations.
fn watched(num_vars: usize, eqs: &[XorEquation], solutions: &[u32]) -> WatchedXor {
    let x = WatchedXor::new(num_vars, eqs);
    assert_eq!(x.inconsistent, solutions.is_empty(), "inconsistency is found on construction: {eqs:?}");
    for matrix in &x.matrices {
        for r in 0..matrix.basic.len() {
            let vars: Vec<usize> =
                (0..matrix.vars.len()).filter(|&c| matrix.holds(r, c as u32)).map(|c| matrix.vars[c] as usize).collect();
            let row = [XorEquation::new(vars, matrix.rhs[r])];
            assert!(solutions.iter().all(|&s| satisfies(&row, s)), "every row is implied by the equations");
        }
    }
    x
}

#[test]
fn watched_matrix_propagates_exactly_what_the_equations_force_told_the_kept_prefix() {
    let (propagated, conflicts, fixpoints) = drive(0x5DEE_CE66_D1CE_4E5B, 6000, Feed::Kept, watched, watched_invariants);
    assert!(propagated > 20_000 && conflicts > 5_000 && fixpoints > 50_000, "{propagated} {conflicts} {fixpoints}");
}

#[test]
fn watched_matrix_propagates_exactly_what_the_equations_force_finding_the_kept_prefix() {
    let (propagated, conflicts, fixpoints) = drive(0x2F69_3B11_7C3D_A4E9, 6000, Feed::Trail, watched, watched_invariants);
    assert!(propagated > 20_000 && conflicts > 5_000 && fixpoints > 50_000, "{propagated} {conflicts} {fixpoints}");
}

#[test]
fn incremental_engine_propagates_exactly_what_the_equations_force_on_solver_shaped_trails() {
    for (seed, feed) in [(0x8CB9_2BA7_2F3D_8DD7, Feed::Kept), (0x6A09_E667_F3BC_C908, Feed::Trail), (0x510E_527F_ADE6_82D1, Feed::Lazy)] {
        let (propagated, conflicts, fixpoints) =
            drive(seed, 3000, feed, |n, eqs, _| IncXor::new(n, eqs), |_: &IncXor, _: &[Lit]| {});
        assert!(propagated > 10_000 && conflicts > 2_500 && fixpoints > 25_000, "{propagated} {conflicts} {fixpoints}");
    }
}

#[test]
fn incremental_engine_follows_a_variable_assigned_the_other_way_in_the_same_place() {
    // x0 ⊕ x1 = 0: told x0 = 1 it forces x1 = 1; given x0 = 0 in the same trail position next, it forces x1 = 0.
    let unit = |clauses: &[Vec<Lit>], trail: &[Lit]| -> Vec<Lit> {
        clauses
            .iter()
            .filter(|c| c.iter().all(|l| !trail.contains(l)))
            .filter_map(|c| {
                let open: Vec<Lit> = c.iter().copied().filter(|l| !trail.contains(&l.negated())).collect();
                (open.len() == 1).then(|| open[0])
            })
            .collect()
    };
    let mut inc = IncXor::new(2, &[XorEquation::new(vec![0, 1], false)]);
    let trail = [Lit::pos(0)];
    assert_eq!(unit(&inc.propagate(&trail), &trail), vec![Lit::pos(1)]);
    let trail = [Lit::neg(0)];
    assert_eq!(unit(&inc.propagate(&trail), &trail), vec![Lit::neg(1)]);
}

#[test]
fn watched_matrix_propagates_exactly_what_the_equations_force_explaining_on_demand() {
    let (propagated, conflicts, fixpoints) = drive(0x9B05_688C_2B3E_6C1F, 6000, Feed::Lazy, watched, watched_invariants);
    assert!(propagated > 20_000 && conflicts > 5_000 && fixpoints > 50_000, "{propagated} {conflicts} {fixpoints}");
}

#[test]
fn watched_matrix_reduces_each_component_and_holds_every_equation() {
    // Two components: {0,1,2} of rank 2 and {3,4} of rank 1, its second equation a copy; x5 in none.
    let eqs = vec![
        XorEquation::new(vec![0, 1], true),
        XorEquation::new(vec![1, 2], false),
        XorEquation::new(vec![3, 4], true),
        XorEquation::new(vec![4, 3], true),
    ];
    let x = WatchedXor::new(6, &eqs);
    assert_eq!(x.matrices.len(), 2);
    assert_eq!(x.matrices.iter().map(|m| m.basic.len()).collect::<Vec<_>>(), vec![2, 1]);
    assert!(x.place[..3].iter().all(|&(m, _)| m == 0));
    assert!(x.place[3..5].iter().all(|&(m, _)| m == 1));
    assert_eq!(x.place[5], (NONE, NONE));
    assert!(!x.inconsistent && x.is_active());
    assert_eq!(x.decision_vars(), vec![2, 4, 5], "three basic columns of six variables");
    // A repeated variable cancels: x0 ⊕ x0 = 1 is 0 = 1.
    assert!(WatchedXor::new(2, &[XorEquation::new(vec![0, 0], true)]).inconsistent);
    // A sum of rank one, inconsistent: x3 ⊕ x4 = 1 and x4 ⊕ x3 = 0.
    let clash = [XorEquation::new(vec![3, 4], true), XorEquation::new(vec![4, 3], false)];
    let x = WatchedXor::new(5, &clash);
    assert!(x.inconsistent && x.is_active());
    // An equation naming a variable past the count is left out.
    let x = WatchedXor::new(2, &[XorEquation::new(vec![0, 2], true), XorEquation::new(vec![0, 1], false)]);
    assert_eq!(x.matrices.len(), 1);
    assert_eq!(x.matrices[0].vars, vec![0, 1]);
    assert!(!WatchedXor::new(2, &[XorEquation::new(vec![0, 2], true)]).is_active());
}

#[test]
fn watched_matrix_reports_its_units_until_they_are_assigned() {
    // x0 ⊕ x1 = 1 and x1 = 1 reduce to rows x0 = 0 and x1 = 1, both units.
    let mut x = WatchedXor::new(2, &[XorEquation::new(vec![0, 1], true), XorEquation::new(vec![1], true)]);
    assert_eq!(x.units.len(), 2);
    let first = x.propagate_kept(&[], 0);
    assert_eq!(first.len(), 2);
    assert!(first.contains(&vec![Lit::neg(0)]) && first.contains(&vec![Lit::pos(1)]));
    // Assigned at a level later undone, x1 = 1 is reported again.
    assert_eq!(x.propagate_kept(&[Lit::neg(0), Lit::pos(1)], 0), Vec::<Vec<Lit>>::new());
    assert_eq!(x.propagate_kept(&[Lit::neg(0)], 1), vec![vec![Lit::pos(1)]]);
    // Against its unit, x1 = 0 is refuted.
    assert_eq!(x.propagate_kept(&[Lit::neg(0), Lit::neg(1)], 1), vec![vec![Lit::pos(1)]]);
}

/// A random formula of XOR gadgets and residual clauses of width one to three, with its equations.
fn xor_formula(rng: &mut u64, num_vars: usize, count: usize, residual: usize) -> (Vec<XorEquation>, Vec<Vec<Lit>>) {
    let eqs: Vec<XorEquation> = random_equations(rng, num_vars, count)
        .into_iter()
        .map(|e| {
            let mut vars = e.vars.clone();
            vars.sort_unstable();
            vars.dedup();
            XorEquation::new(vars, e.rhs)
        })
        .filter(|e| !e.vars.is_empty() && e.vars.len() <= 6)
        .collect();
    let mut clauses: Vec<Vec<Lit>> = eqs.iter().flat_map(gadget).collect();
    for _ in 0..residual {
        let width = 1 + (xorshift(rng) % 3) as usize;
        clauses.push((0..width).map(|_| Lit::new((xorshift(rng) % num_vars as u64) as u32, xorshift(rng) % 2 == 0)).collect());
    }
    (eqs, clauses)
}

/// A formula of equations consistent with a planted assignment, width three to five, and `residual` random clauses of
/// width three: the equations alone are satisfiable, the clauses decide the verdict.
fn planted_xor_formula(rng: &mut u64, num_vars: usize, count: usize, residual: usize) -> (Vec<XorEquation>, Vec<Vec<Lit>>) {
    let planted: Vec<bool> = (0..num_vars).map(|_| xorshift(rng) % 2 == 0).collect();
    let eqs: Vec<XorEquation> = (0..count)
        .map(|_| {
            let width = 3 + (xorshift(rng) % 3) as usize;
            let mut vars: Vec<usize> = (0..width).map(|_| (xorshift(rng) % num_vars as u64) as usize).collect();
            vars.sort_unstable();
            vars.dedup();
            let rhs = vars.iter().fold(false, |a, &v| a ^ planted[v]);
            XorEquation::new(vars, rhs)
        })
        .collect();
    let mut clauses: Vec<Vec<Lit>> = eqs.iter().flat_map(gadget).collect();
    for _ in 0..residual {
        clauses.push((0..3).map(|_| Lit::new((xorshift(rng) % num_vars as u64) as u32, xorshift(rng) % 2 == 0)).collect());
    }
    (eqs, clauses)
}

fn models(clauses: &[Vec<Lit>], m: &[bool]) -> bool {
    clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
}

#[test]
fn watched_matrix_under_cdcl_decides_as_brute_force_does() {
    use crate::cdcl::{SolveResult, Solver};
    let mut rng: u64 = 0xA076_1D64_78BD_642F;
    let (mut sat, mut unsat) = (0, 0);
    for case in 0..3000 {
        let num_vars = 3 + (xorshift(&mut rng) % 9) as usize;
        let count = 1 + (xorshift(&mut rng) % num_vars as u64) as usize;
        let residual = (xorshift(&mut rng) % 6) as usize;
        let (eqs, clauses) = xor_formula(&mut rng, num_vars, count, residual);
        let truth = (0u32..1 << num_vars).any(|m| models(&clauses, &(0..num_vars).map(|v| m >> v & 1 == 1).collect::<Vec<_>>()));
        let x = WatchedXor::new(num_vars, &eqs);
        let kernel = case % 2 == 1;
        let decisions = x.decision_vars();
        let mut solver = Solver::new(num_vars);
        for c in &clauses {
            solver.add_clause(c.clone());
        }
        if kernel {
            solver.set_decision_vars(&decisions);
        }
        let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(x)];
        match solver.solve_with(&mut theories) {
            SolveResult::Sat(m) => {
                assert!(truth, "SAT on an unsatisfiable formula: {eqs:?} {clauses:?}");
                assert!(models(&clauses, &m), "the model satisfies every clause");
                sat += 1;
            }
            SolveResult::Unsat => {
                assert!(!truth, "UNSAT on a satisfiable formula (kernel decisions {kernel}): {eqs:?} {clauses:?}");
                unsat += 1;
            }
        }
    }
    assert!(sat > 500 && unsat > 500, "{sat} SAT, {unsat} UNSAT");
}

#[test]
fn watched_matrix_under_cdcl_agrees_with_plain_cdcl_at_scale() {
    use crate::cdcl::{SolveResult, Solver};
    let mut rng: u64 = 0x1B87_3593_CC9E_2D51;
    let (mut sat, mut unsat) = (0, 0);
    for _ in 0..300 {
        let num_vars = 30 + (xorshift(&mut rng) % 50) as usize;
        let count = num_vars / 2 + (xorshift(&mut rng) % (num_vars as u64 / 2)) as usize;
        let residual = (xorshift(&mut rng) % (4 * num_vars as u64)) as usize;
        let (eqs, clauses) = planted_xor_formula(&mut rng, num_vars, count, residual);
        let mut plain = Solver::new(num_vars);
        let mut live = Solver::new(num_vars);
        for c in &clauses {
            plain.add_clause(c.clone());
            live.add_clause(c.clone());
        }
        let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(WatchedXor::new(num_vars, &eqs))];
        match (plain.solve(), live.solve_with(&mut theories)) {
            (SolveResult::Sat(_), SolveResult::Sat(m)) => {
                assert!(models(&clauses, &m));
                sat += 1;
            }
            (SolveResult::Unsat, SolveResult::Unsat) => unsat += 1,
            (p, l) => panic!("plain {p:?}, watched {l:?}: {eqs:?} {clauses:?}"),
        }
    }
    assert!(sat > 50 && unsat > 50, "{sat} SAT, {unsat} UNSAT");
}

/// Random 3-XOR equations over `n` variables, every variable named, consistent with a planted assignment and of rank
/// `n − defect`; and, for each of the `2^defect` solutions past the first `keep`, a clause over three of its variables
/// that excludes it and no kept solution (one of the three is a variable where it differs from each). With `keep = 0`
/// the formula is unsatisfiable, and only through the equations: each clause alone excludes an eighth of all
/// assignments, and full-rank random 3-XOR is exponential for resolution (Ben-Sasson & Wigderson 2001).
fn excluded_affine_space(n: usize, defect: usize, keep: u64, seed: u64) -> (Vec<XorEquation>, Vec<Vec<Lit>>) {
    let mut rng = seed;
    let planted: Vec<bool> = (0..n).map(|_| xorshift(&mut rng) % 2 == 0).collect();
    let words = n.div_ceil(64);
    let mut basis: Vec<Option<(Vec<u64>, bool)>> = vec![None; n];
    let mut order: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        order.swap(i, (xorshift(&mut rng) % (i as u64 + 1)) as usize);
    }
    let mut covering = order.chunks_exact(3).map(<[usize]>::to_vec).collect::<Vec<_>>().into_iter();
    let (mut eqs, mut rank) = (Vec::new(), 0);
    while rank < n - defect {
        let vars = covering.next().unwrap_or_else(|| {
            let mut vars = Vec::new();
            while vars.len() < 3 {
                let v = (xorshift(&mut rng) % n as u64) as usize;
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            vars
        });
        let rhs = vars.iter().fold(false, |a, &v| a ^ planted[v]);
        let (mut row, mut sum) = (vec![0u64; words], rhs);
        for &v in &vars {
            row[v / 64] ^= 1 << (v % 64);
        }
        while let Some(lead) = (0..n).find(|&c| row[c / 64] >> (c % 64) & 1 == 1) {
            match &basis[lead] {
                Some((b, r)) => {
                    row.iter_mut().zip(b).for_each(|(x, y)| *x ^= y);
                    sum ^= r;
                }
                None => {
                    basis[lead] = Some((row, sum));
                    rank += 1;
                    eqs.push(XorEquation::new(vars, rhs));
                    break;
                }
            }
        }
    }
    let mut clauses: Vec<Vec<Lit>> = eqs.iter().flat_map(gadget).collect();
    let free: Vec<usize> = (0..n).filter(|&c| basis[c].is_none()).collect();
    let solution = |f: u64| {
        let mut s = vec![false; n];
        for (i, &c) in free.iter().enumerate() {
            s[c] = f >> i & 1 == 1;
        }
        for lead in (0..n).rev() {
            if let Some((row, rhs)) = &basis[lead] {
                s[lead] = (lead + 1..n).filter(|&c| row[c / 64] >> (c % 64) & 1 == 1).fold(*rhs, |a, c| a ^ s[c]);
            }
        }
        s
    };
    let kept: Vec<Vec<bool>> = (0..keep).map(solution).collect();
    for f in keep..1u64 << free.len() {
        let s = solution(f);
        let mut vars: Vec<usize> = kept
            .iter()
            .map(|k| {
                let differs: Vec<usize> = (0..n).filter(|&v| k[v] != s[v]).collect();
                differs[(xorshift(&mut rng) % differs.len() as u64) as usize]
            })
            .collect();
        vars.dedup();
        while vars.len() < 3 {
            let v = (xorshift(&mut rng) % n as u64) as usize;
            if !vars.contains(&v) {
                vars.push(v);
            }
        }
        clauses.push(vars.iter().map(|&v| Lit::new(v as u32, !s[v])).collect());
    }
    (eqs, clauses)
}

#[test]
fn watched_matrix_decides_an_excluded_affine_space_within_its_dimension() {
    use crate::cdcl::{SolveResult, Solver};
    let (n, defect) = (600, 8);
    for seed in 0..3u64 {
        for keep in [0, 1] {
            let (eqs, clauses) = excluded_affine_space(n, defect, keep, 0x9E37_79B9 ^ seed.wrapping_mul(0x1000_0001));
            let mut solver = Solver::new(n);
            for c in &clauses {
                solver.add_clause(c.clone());
            }
            let mut theories: Vec<Box<dyn Theory>> = vec![Box::new(WatchedXor::new(n, &eqs))];
            match (keep, solver.solve_with(&mut theories)) {
                (0, SolveResult::Unsat) => {}
                (1, SolveResult::Sat(m)) => assert!(models(&clauses, &m), "the model satisfies every clause"),
                (k, verdict) => panic!("keep {k}: {verdict:?}"),
            }
            assert!(solver.conflicts() <= 1 << defect, "seed {seed} keep {keep}: {} conflicts", solver.conflicts());
        }
    }
}

#[test]
fn a_watched_matrix_counts_its_calls_and_those_that_refute() {
    // x0 ⊕ x1 = 1 and x1 ⊕ x2 = 0: x0 = 1, x2 = 0 contradicts nothing (x1 = 0), while x0 = x1 = 1 violates the first.
    let eqs = [XorEquation::new(vec![0, 1], true), XorEquation::new(vec![1, 2], false)];
    let mut x = WatchedXor::new(3, &eqs);
    assert_eq!((x.calls(), x.refutations()), (0, 0));
    x.propagate(&[Lit::pos(0)]);
    x.propagate(&[Lit::pos(0), Lit::neg(2)]);
    assert_eq!((x.calls(), x.refutations()), (2, 0));
    let refuted = x.propagate(&[Lit::pos(0), Lit::pos(1)]);
    assert!(refuted.iter().any(|c| c.iter().all(|l| [Lit::pos(0), Lit::pos(1)].contains(&l.negated()))));
    assert_eq!((x.calls(), x.refutations()), (3, 1));
    let mut inconsistent = WatchedXor::new(2, &[XorEquation::new(vec![0, 1], true), XorEquation::new(vec![0, 1], false)]);
    inconsistent.propagate(&[]);
    assert_eq!((inconsistent.calls(), inconsistent.refutations()), (1, 1), "inconsistent equations refute every call");
}
