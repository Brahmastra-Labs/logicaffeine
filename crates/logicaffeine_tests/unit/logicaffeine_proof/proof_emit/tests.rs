use super::*;
use crate::cdcl::Lit;
use crate::pr::check_pr_refutation;

fn p(v: u32) -> Lit {
    Lit::pos(v)
}
fn n(v: u32) -> Lit {
    Lit::neg(v)
}

/// The canonical 2-variable all-clauses UNSAT instance, refuted by the resolvents `p`, `¬p`.
fn pq_unsat() -> (usize, Vec<Vec<Lit>>, Vec<ProofStep>) {
    let f = vec![
        vec![p(0), p(1)],
        vec![p(0), n(1)],
        vec![n(0), p(1)],
        vec![n(0), n(1)],
    ];
    let steps = vec![ProofStep::Rup(vec![p(0)]), ProofStep::Rup(vec![n(0)])];
    (2, f, steps)
}

#[test]
fn drat_round_trips_through_our_checker() {
    let (nv, f, steps) = pq_unsat();
    let text = emit_drat(nv, &f, &steps).expect("emits");
    let parsed = parse_dpr(&text).expect("parses");
    assert!(
        check_pr_refutation(nv, &f, &parsed),
        "round-tripped DRAT must still refute"
    );
}

#[test]
fn drat_rejects_a_corrupted_addition() {
    // Over a SATISFIABLE base `(a ∨ b)`, a bolted-on unit `¬a` is not entailed (assuming `a`
    // satisfies the clause with no propagation), so it is not RUP — the parsed-back "proof"
    // must fail to verify. (In the full 4-clause UNSAT formula every clause is RUP, so the
    // base must be satisfiable for the corruption to bite.)
    let f = vec![vec![p(0), p(1)]];
    let bogus = parse_dpr("-1 0\n").expect("parses"); // the un-entailed unit `¬a`
    assert!(!check_pr_refutation(2, &f, &bogus));
}

#[test]
fn lrat_hints_validate_and_reject_corruption() {
    let (nv, f, steps) = pq_unsat();
    let lrat = emit_lrat(nv, &f, &steps).expect("emits LRAT");
    assert!(
        check_lrat(nv, &f, &lrat),
        "our own LRAT checker accepts our hints"
    );

    // Corrupt a hint id → the chain no longer conflicts where claimed → rejected.
    let mangled = lrat.replace(" 0\n", " 999 0\n");
    assert!(
        !check_lrat(nv, &f, &mangled),
        "a corrupted hint chain must be rejected"
    );
}

#[test]
fn lrat_checker_rejects_a_proof_with_no_empty_clause() {
    let (nv, f, _) = pq_unsat();
    // A single learned unit `p` with a valid hint, but never closing to the empty clause.
    let db: Vec<(u64, Vec<Lit>)> = f
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u64 + 1, c.clone()))
        .collect();
    let hints = rup_hints(nv, &db, &[p(0)]).expect("p is RUP");
    let mut line = String::from("5 1 ");
    for h in hints {
        line.push_str(&h.to_string());
        line.push(' ');
    }
    line.push_str("0\n");
    assert!(
        !check_lrat(nv, &f, &line),
        "no empty-clause line ⇒ not a refutation"
    );
}

#[test]
fn dpr_assignment_witness_line_round_trips_and_reverifies() {
    // The textbook model-removing PR clause: F=(a∨b), C=(¬a∨b) with ω={¬a,b}. Emit it as a
    // DPR witness line, parse it back, and confirm the parsed clause+witness still PR-checks.
    let (a, b) = (0u32, 1u32);
    let f = vec![vec![p(a), p(b)]];
    let c = vec![n(a), p(b)];
    let omega = vec![n(a), p(b)];
    let witness = Witness::Assignment(omega);
    assert!(crate::pr::is_pr(2, &f, &c, &witness));

    // emit_dpr requires a full refutation; here we test just the witness-line machinery, so
    // hand-format one line and parse it back.
    let w = try_assignment_witness(2, &f, &c, &witness).expect("reduces");
    let pivot = c.iter().copied().find(|l| w.contains(l)).unwrap();
    let mut out = String::new();
    let mut c_ord = vec![pivot];
    c_ord.extend(c.iter().copied().filter(|&l| l != pivot));
    push_clause(&mut out, &c_ord);
    out.push(' ');
    let mut w_ord = vec![pivot];
    w_ord.extend(w.iter().copied().filter(|&l| l != pivot));
    for &l in &w_ord {
        out.push_str(&dimacs(l).to_string());
        out.push(' ');
    }
    out.push_str("0\n");

    let parsed = parse_dpr(&out).expect("parses");
    match &parsed[0] {
        ProofStep::Pr { clause, witness } => {
            assert!(
                crate::pr::is_pr(2, &f, clause, witness),
                "parsed DPR line re-verifies"
            );
        }
        other => panic!("expected a PR step, got {other:?}"),
    }
}

#[test]
fn lrat_certifies_a_real_cdcl_refutation_of_php() {
    // Not a hand-built trace: the actual learned clauses from our CDCL solver refuting PHP(3),
    // emitted as LRAT and replayed by our independent (cake_lpr-style) checker.
    use crate::cdcl::{SolveResult, Solver};
    let (cnf, _) = crate::families::php(3);
    let nv = cnf.num_vars;
    let mut solver = Solver::new(nv);
    for c in &cnf.clauses {
        solver.add_clause(c.clone());
    }
    let steps: Vec<ProofStep> = match solver.solve() {
        SolveResult::Unsat => solver
            .learned()
            .iter()
            .map(|c| ProofStep::Rup(c.lits.clone()))
            .collect(),
        SolveResult::Sat(_) => panic!("PHP(3) is UNSAT"),
    };
    let lrat = emit_lrat(nv, &cnf.clauses, &steps).expect("a real CDCL refutation is RUP → LRAT");
    assert!(
        check_lrat(nv, &cnf.clauses, &lrat),
        "LRAT replay of a real CDCL refutation validates"
    );

    // The same trace also round-trips as DRAT through our checker.
    let drat = emit_drat(nv, &cnf.clauses, &steps).expect("emits DRAT");
    let parsed = parse_dpr(&drat).expect("parses");
    assert!(check_pr_refutation(nv, &cnf.clauses, &parsed));
}

#[test]
fn the_universal_lrat_guarantee_via_plain_cdcl() {
    // The fail-closed fallback is pure RUP, so EVERY UNSAT instance we close carries a
    // cake_lpr-checkable LRAT proof — independent of any symmetry/PR machinery.
    let (cnf, _) = crate::families::php(4);
    let nv = cnf.num_vars;
    let steps = crate::sdcl::plain_cdcl_refutation(nv, &cnf.clauses);
    let lrat = emit_lrat(nv, &cnf.clauses, &steps).expect("plain CDCL ⇒ RUP ⇒ LRAT");
    assert!(
        check_lrat(nv, &cnf.clauses, &lrat),
        "universal LRAT certificate validates"
    );
}

#[test]
fn external_sr2drat_drat_trim_certifies_our_php_symmetry_proof() {
    // The marquee cross-check: our *substitution-redundancy* refutation of PHP(n) — the symmetry
    // proof Kissat/CaDiCaL time out on — emitted as `.sr`, expanded by Marijn Heule's `sr2drat`,
    // and VERIFIED by the community's `drat-trim`. Independent of our solver and our SR checker.
    // Gated on `$SR2DRAT` + `$DRAT_TRIM` so the default suite stays green when the binaries are
    // absent (mirrors the Z3-optional pattern).
    let (Ok(sr2drat), Ok(drat_trim)) = (std::env::var("SR2DRAT"), std::env::var("DRAT_TRIM"))
    else {
        eprintln!("SR2DRAT/DRAT_TRIM unset — skipping external sr2drat→drat-trim cross-check");
        return;
    };
    for n in [4usize, 8] {
        let (cnf, _) = crate::families::php(n);
        let cert = crate::sym_certify::heule_php_refutation(n);
        assert!(cert.refuted, "PHP({n}) symmetry proof closes internally");
        let sr = emit_sr(cnf.num_vars, &cnf.clauses, &cert.steps).expect("emits .sr");
        let cnf_path = temp_write(&format!("logos_sr_php{n}.cnf"), &crate::dimacs::print(&cnf));
        let sr_path = temp_write(&format!("logos_sr_php{n}.sr"), &sr);
        let drat_path = cnf_path.with_extension("drat");
        // sr2drat expands the .sr into a plain DRAT proof (stdout → file).
        let drat = std::process::Command::new(&sr2drat)
            .arg(&cnf_path)
            .arg(&sr_path)
            .output()
            .expect("run sr2drat");
        std::fs::write(&drat_path, &drat.stdout).expect("write expanded DRAT");
        // drat-trim independently verifies the expansion against the original CNF.
        let out = std::process::Command::new(&drat_trim)
            .arg(&cnf_path)
            .arg(&drat_path)
            .output()
            .expect("run drat-trim");
        let s = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            s.contains("VERIFIED") && !s.contains("NOT VERIFIED"),
            "sr2drat→drat-trim must VERIFY our PHP({n}) SR proof; got:\n{s}"
        );
    }
}

#[test]
fn size_sink_measures_the_sr_proof_lazily_and_matches_the_string_length() {
    // The benchmarks page reports our OWN proof size. It must be measured by streaming the SR
    // proof through a byte counter — never materializing a giant String — so the streamed count
    // equals the real serialized length exactly, and a tight cap aborts emission early
    // (fail-closed) instead of running to completion: the memory-safety guarantee for a
    // pathologically large proof.
    let (cnf, _) = crate::families::php(4);
    let cert = crate::sym_certify::heule_php_refutation(4);
    assert!(cert.refuted, "PHP(4) symmetry proof closes internally");
    let full = emit_sr(cnf.num_vars, &cnf.clauses, &cert.steps).expect("emits .sr");
    assert!(
        full.len() > 8,
        "the SR proof is more than a handful of bytes"
    );

    let mut sink = SizeSink::new(64 << 20);
    write_sr(&mut sink, cnf.num_vars, &cnf.clauses, &cert.steps).expect("streams .sr");
    assert!(!sink.overflowed());
    assert_eq!(
        sink.bytes(),
        full.len() as u64,
        "streamed byte count = serialized length"
    );

    let mut capped = SizeSink::new(8);
    let err = write_sr(&mut capped, cnf.num_vars, &cnf.clauses, &cert.steps);
    assert_eq!(
        err,
        Err(EmitError::SizeCapExceeded),
        "a tight cap aborts emission early"
    );
    assert!(capped.overflowed());
    assert!(capped.bytes() >= 8, "counted up to the cap before bailing");
}

#[test]
fn dpr_emits_or_honestly_declines_the_sdcl_proof() {
    // The SaDiCaL-grade path: solve a symmetric UNSAT instance with the certified SDCL solver
    // and export the proof as DPR. Either it is fully PR (a dpr-trim-checkable proof that
    // round-trips), or a step is irreducibly SR and the emitter declines honestly — never a
    // bogus line. In both cases the internal certificate holds.
    use crate::sdcl::{solve_certified, CertifiedOutcome};
    let (cnf, _) = crate::families::php(3);
    let nv = cnf.num_vars;
    match solve_certified(nv, &cnf.clauses) {
        CertifiedOutcome::Unsat { steps, .. } => {
            assert!(
                check_pr_refutation(nv, &cnf.clauses, &steps),
                "the SDCL proof self-checks"
            );
            match emit_dpr(nv, &cnf.clauses, &steps) {
                Ok(text) => {
                    let parsed = parse_dpr(&text).expect("parses");
                    assert!(
                        check_pr_refutation(nv, &cnf.clauses, &parsed),
                        "round-tripped DPR refutes"
                    );
                }
                Err(EmitError::RequiresSubstitutionRedundancy { .. }) => {}
                Err(e) => panic!("unexpected emit error: {e:?}"),
            }
        }
        CertifiedOutcome::Sat(_) => panic!("PHP(3) is UNSAT"),
    }
}

/// Write `text` to a uniquely-named temp file and return its path.
fn temp_write(name: &str, text: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, text).expect("write temp file");
    path
}

#[test]
fn external_drat_trim_accepts_our_drat_proof() {
    // Gold-standard cross-check: the SAT community's own `drat-trim` must accept the DRAT we
    // emit for a real CDCL refutation of PHP(5). Gated on `$DRAT_TRIM` so the default suite
    // stays green when the binary is absent (mirrors the Z3-optional pattern).
    let Ok(bin) = std::env::var("DRAT_TRIM") else {
        eprintln!("DRAT_TRIM unset — skipping external drat-trim cross-check");
        return;
    };
    let (cnf, _) = crate::families::php(5);
    let nv = cnf.num_vars;
    let steps = crate::sdcl::plain_cdcl_refutation(nv, &cnf.clauses);
    let drat = emit_drat(nv, &cnf.clauses, &steps).expect("emits DRAT");
    let cnf_path = temp_write("logos_drat_php5.cnf", &crate::dimacs::print(&cnf));
    let drat_path = temp_write("logos_drat_php5.drat", &drat);
    let out = std::process::Command::new(&bin)
        .arg(&cnf_path)
        .arg(&drat_path)
        .output()
        .expect("run drat-trim");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        s.contains("VERIFIED") && !s.contains("NOT VERIFIED"),
        "drat-trim must VERIFY our DRAT; got:\n{s}"
    );
}

#[test]
fn external_lrat_check_accepts_our_lrat_proof() {
    // The formally-verified-style checker `lrat-check` must accept the hint-carrying LRAT we
    // synthesise for a real CDCL refutation of PHP(5). Gated on `$LRAT_CHECK`.
    let Ok(bin) = std::env::var("LRAT_CHECK") else {
        eprintln!("LRAT_CHECK unset — skipping external lrat-check cross-check");
        return;
    };
    let (cnf, _) = crate::families::php(5);
    let nv = cnf.num_vars;
    let steps = crate::sdcl::plain_cdcl_refutation(nv, &cnf.clauses);
    let lrat = emit_lrat(nv, &cnf.clauses, &steps).expect("emits LRAT");
    let cnf_path = temp_write("logos_lrat_php5.cnf", &crate::dimacs::print(&cnf));
    let lrat_path = temp_write("logos_lrat_php5.lrat", &lrat);
    let out = std::process::Command::new(&bin)
        .arg(&cnf_path)
        .arg(&lrat_path)
        .output()
        .expect("run lrat-check");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        s.contains("VERIFIED") && !s.contains("NOT VERIFIED"),
        "lrat-check must accept our LRAT; got:\n{s}"
    );
}

#[test]
fn try_assignment_witness_is_sound_on_php_substitutions() {
    // Over the real PHP crusher's substitution steps: whenever the reducer hands back an
    // assignment witness, pr::is_pr must independently bless it (the reducer never invents an
    // unsound witness). We also report how many genuinely reduce vs. stay irreducibly SR.
    let cr = crate::sym_certify::heule_php_refutation(4);
    let (cnf, _) = crate::families::php(4);
    let nv = cnf.num_vars;
    let mut db = cnf.clauses.clone();
    let mut reduced = 0usize;
    let mut sr_only = 0usize;
    for step in &cr.steps {
        match step {
            ProofStep::Pr { clause, witness } => {
                if let Some(w) = try_assignment_witness(nv, &db, clause, witness) {
                    assert!(
                        crate::pr::is_pr(nv, &db, clause, &Witness::Assignment(w)),
                        "the reducer returned an unsound assignment witness"
                    );
                    reduced += 1;
                } else {
                    sr_only += 1;
                }
                db.push(clause.clone());
            }
            ProofStep::Rup(c) => db.push(c.clone()),
            ProofStep::Delete(c) => {
                let key = canon(c);
                if let Some(pos) = db.iter().position(|d| canon(d) == key) {
                    db.swap_remove(pos);
                }
            }
        }
    }
    // The honest classification: PHP's swap clauses are SR. The test's contract is soundness
    // of the reducer, not that everything reduces — but it must have actually run on PR steps.
    assert!(
        reduced + sr_only > 0,
        "the PHP refutation must contain PR steps to classify"
    );
    eprintln!("PHP(4): {reduced} steps reduced to PR, {sr_only} remained irreducibly SR");
}

/// A refutation carrying partial-symmetry steps, emitted as `.sr`, expanded by `sr2drat` and verified by `drat-trim`
/// against the original formula. Gated on `$SR2DRAT` and `$DRAT_TRIM`.
#[test]
fn external_sr2drat_drat_trim_certifies_partial_symmetry_steps() {
    let (Ok(sr2drat), Ok(drat_trim)) = (std::env::var("SR2DRAT"), std::env::var("DRAT_TRIM")) else {
        eprintln!("SR2DRAT/DRAT_TRIM unset — skipping external partial-symmetry cross-check");
        return;
    };
    let mut verified = 0;
    for seed in 0..400u64 {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let nv = 5usize;
        let clauses: Vec<Vec<Lit>> = (0..16)
            .map(|_| {
                let mut c: Vec<Lit> = Vec::new();
                while c.len() < 2 {
                    let l = Lit::new((next() % nv as u64) as u32, next() & 1 == 0);
                    if !c.iter().any(|m| m.var() == l.var()) {
                        c.push(l);
                    }
                }
                c
            })
            .collect();
        let r = crate::sym_certify::certified_unsat_conditional(nv, &clauses, 2);
        let carries_partial = r.steps.iter().any(|s| {
            matches!(s, ProofStep::Pr { witness: Witness::PartialSymmetry(_), .. })
        });
        if !r.refuted || !carries_partial {
            continue;
        }
        let cnf = crate::dimacs::DimacsCnf { num_vars: nv, clauses: clauses.clone() };
        let sr = emit_sr(nv, &clauses, &r.steps).expect("emits .sr");
        let cnf_path = temp_write(&format!("logos_partial_sr_{seed}.cnf"), &crate::dimacs::print(&cnf));
        let sr_path = temp_write(&format!("logos_partial_sr_{seed}.sr"), &sr);
        let drat_path = cnf_path.with_extension("drat");
        let drat = std::process::Command::new(&sr2drat).arg(&cnf_path).arg(&sr_path).output().expect("run sr2drat");
        std::fs::write(&drat_path, &drat.stdout).expect("write expanded DRAT");
        let out = std::process::Command::new(&drat_trim).arg(&cnf_path).arg(&drat_path).output().expect("run drat-trim");
        let s = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(
            s.contains("VERIFIED") && !s.contains("NOT VERIFIED"),
            "seed {seed}: sr2drat→drat-trim must VERIFY the partial-symmetry proof; got:\n{s}"
        );
        verified += 1;
        if verified == 5 {
            break;
        }
    }
    assert!(verified >= 1, "no refutation in the sweep carried a partial-symmetry step");
}

/// A refutation whose preprocessing introduced variables is emitted as DPR over them: each PR line leads its clause
/// and its witness with the same pivot, the introduced variable.
#[test]
fn a_dpr_proof_naming_introduced_variables_is_emitted_pivot_first() {
    use crate::cdcl::{SolveResult, Solver};
    let mut formula: Vec<Vec<Lit>> = Vec::new();
    for a in 0..8u32 {
        for b in a + 1..8 {
            formula.push(vec![Lit::neg(a), Lit::neg(b)]);
        }
    }
    formula.push(vec![Lit::pos(0), Lit::pos(1), Lit::pos(2)]);
    formula.push(vec![Lit::pos(3), Lit::pos(4), Lit::pos(5)]);
    let options = crate::preprocess::Options { factor: true, eliminate: false, ..Default::default() };
    let pre = crate::preprocess::preprocess(8, &formula, &options);
    let mut proof = pre.steps.clone();
    if !pre.unsat {
        let mut s = Solver::new(pre.num_vars);
        for c in &pre.clauses {
            s.add_clause(c.clone());
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
        proof.extend(s.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())));
    }
    let text = emit_dpr(8, &formula, &proof).expect("a factoring refutation emits as DPR");
    let mut pr_lines = 0;
    for line in text.lines().filter(|l| !l.starts_with('d')) {
        let fields: Vec<i64> = line.split_whitespace().map(|t| t.parse().unwrap()).collect();
        let zeros: Vec<usize> = fields.iter().enumerate().filter(|(_, &x)| x == 0).map(|(i, _)| i).collect();
        if zeros.len() == 2 {
            pr_lines += 1;
            assert_eq!(fields[0], fields[zeros[0] + 1], "the pivot leads both the clause and the witness: {line}");
            assert!(fields[0].unsigned_abs() > 8, "the pivot is an introduced variable: {line}");
        }
    }
    assert!(pr_lines >= 2, "{pr_lines} PR lines");
}

/// Emitting a long refutation costs its propagations: the DPR text of a factored at-most-one's refutation over 250
/// variables comes within a minute unoptimized on a loaded box.
#[test]
fn emitting_a_long_refutation_costs_its_propagations() {
    use crate::cdcl::{SolveResult, Solver};
    let mut formula: Vec<Vec<Lit>> = Vec::new();
    for a in 0..250u32 {
        for b in a + 1..250 {
            formula.push(vec![Lit::neg(a), Lit::neg(b)]);
        }
    }
    formula.push((0..125).map(Lit::pos).collect());
    formula.push((125..250).map(Lit::pos).collect());
    let options = crate::preprocess::Options { factor: true, ..Default::default() };
    let pre = crate::preprocess::preprocess(250, &formula, &options);
    let mut proof = pre.steps.clone();
    if !pre.unsat {
        let mut s = Solver::new(pre.num_vars);
        for c in &pre.clauses {
            s.add_clause(c.clone());
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
        proof.extend(s.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(emit_dpr(250, &formula, &proof).is_ok());
    });
    assert!(rx.recv_timeout(std::time::Duration::from_secs(60)).expect("emission costs its propagations"));
}
