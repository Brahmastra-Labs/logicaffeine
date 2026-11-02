use super::*;
use crate::sat::{find_model, prove_unsat, ModelOutcome, UnsatOutcome};

fn a(s: &str) -> ProofExpr {
    atom(s.to_string())
}

fn bits(mask: u32, n: usize) -> Vec<bool> {
    (0..n).map(|i| (mask >> i) & 1 == 1).collect()
}
fn lex_le_oracle(x: &[bool], y: &[bool]) -> bool {
    for k in 0..x.len() {
        if x[k] != y[k] {
            return !x[k] && y[k];
        }
    }
    true
}

#[test]
fn lex_le_matches_brute_force() {
    // Soundness foundation: the encoded `X ≤ₗₑₓ Y` must agree with the oracle on EVERY pinned
    // assignment (auxiliaries free) — a wrong SBP would delete models and forge UNSAT.
    for m in 1..=5 {
        let xs: Vec<ProofExpr> = (0..m).map(|i| a(&format!("x{i}"))).collect();
        let ys: Vec<ProofExpr> = (0..m).map(|i| a(&format!("y{i}"))).collect();
        let f = lex_le(&xs, &ys, "L");
        for xm in 0..(1u32 << m) {
            for ym in 0..(1u32 << m) {
                let (xa, ya) = (bits(xm, m), bits(ym, m));
                let mut pinned = f.clone();
                for (v, &b) in xs.iter().zip(&xa).chain(ys.iter().zip(&ya)) {
                    pinned = and(pinned, if b { v.clone() } else { not(v.clone()) });
                }
                let sat = matches!(find_model(&pinned), ModelOutcome::Sat(_));
                assert_eq!(
                    sat,
                    lex_le_oracle(&xa, &ya),
                    "lex_le m={m} x={xa:?} y={ya:?} encoded={sat} oracle={}",
                    lex_le_oracle(&xa, &ya)
                );
            }
        }
    }
}

/// Pairwise PHP(n) — used to confirm symmetry breaking PRESERVES the UNSAT verdict.
fn php(n: usize) -> ProofExpr {
    let holes = n - 1;
    let p = |i: usize, h: usize| a(&format!("p_{i}_{h}"));
    let mut clauses = Vec::new();
    for i in 0..n {
        clauses.push((0..holes).map(|h| p(i, h)).reduce(or).unwrap());
    }
    for h in 0..holes {
        for i in 0..n {
            for j in (i + 1)..n {
                clauses.push(not(and(p(i, h), p(j, h))));
            }
        }
    }
    clauses.into_iter().reduce(and).unwrap()
}

#[test]
fn breaking_preserves_unsat_on_php() {
    // Adding verified SBPs to an UNSAT formula keeps it UNSAT (and stays refutable).
    for n in 3..=6 {
        let broken = break_symmetries(&php(n));
        assert!(
            matches!(prove_unsat(&broken), UnsatOutcome::Refuted),
            "PHP({n}) with symmetry breaking must stay Refuted"
        );
    }
}

#[test]
fn breaking_detects_the_pigeon_symmetry() {
    // PHP rows (pigeons) are interchangeable → at least one verified row-swap SBP is added, so
    // the formula grows. (If detection silently found nothing, the formula would be unchanged.)
    let before = php(4);
    let after = break_symmetries(&before);
    assert_ne!(
        before, after,
        "PHP(4) pigeon symmetry must be detected and broken"
    );
}

#[test]
fn breaking_preserves_sat_models() {
    // Soundness-critical: on a SATISFIABLE symmetric formula, breaking must keep ≥1 model.
    // "each of 3 items takes one of 3 distinct slots" (a feasible bipartite matching) is SAT and
    // row-symmetric; after breaking it must remain SAT.
    let p = |i: usize, h: usize| a(&format!("q_{i}_{h}"));
    let mut clauses = Vec::new();
    for i in 0..3 {
        clauses.push((0..3).map(|h| p(i, h)).reduce(or).unwrap());
    }
    for h in 0..3 {
        for i in 0..3 {
            for j in (i + 1)..3 {
                clauses.push(not(and(p(i, h), p(j, h))));
            }
        }
    }
    let f = clauses.into_iter().reduce(and).unwrap();
    assert!(
        matches!(find_model(&f), ModelOutcome::Sat(_)),
        "feasible must be SAT to begin"
    );
    let broken = break_symmetries(&f);
    assert!(
        matches!(find_model(&broken), ModelOutcome::Sat(_)),
        "symmetry breaking must NOT delete every model of a SAT formula"
    );
}

fn raw_cdcl_is_unsat(e: &ProofExpr) -> bool {
    // Drive the bare CDCL core — NO matching fast-path, NO symmetry breaking inside — so this
    // measures the effect of the SBPs we pass in, isolated from `prove_unsat`'s shortcuts.
    use crate::cdcl::SolveResult;
    use crate::cnf::Cnf;
    let mut cnf = Cnf::new();
    cnf.assert(e).expect("clausifiable");
    let (mut solver, _) = cnf.into_solver_with_atoms();
    matches!(solver.solve(), SolveResult::Unsat)
}

#[test]
fn symmetry_breaking_tames_raw_cdcl_on_pigeonhole() {
    // The PROOF symmetry breaking earns its place: pairwise PHP is syntactically symmetric, so
    // raw CDCL re-derives the same conflict once per pigeon permutation and bogs down. The
    // verified lex-leader SBPs collapse those orbits — the broken formula is decided much faster.
    // (`prove_unsat` would short-circuit PHP via the matching reasoner; here we bypass it to
    // isolate the symmetry-breaking effect on the general CDCL engine.)
    use std::time::Instant;
    let n = 8;
    let plain = php(n);
    let broken = break_symmetries(&plain);
    assert_ne!(
        plain, broken,
        "PHP({n}) must have a verified pigeon symmetry to break"
    );

    let t = Instant::now();
    assert!(raw_cdcl_is_unsat(&broken), "broken PHP({n}) must be UNSAT");
    let with = t.elapsed();
    let t = Instant::now();
    assert!(raw_cdcl_is_unsat(&plain), "plain PHP({n}) must be UNSAT");
    let without = t.elapsed();

    eprintln!(
        "raw CDCL PHP({n}): plain={without:?} broken={with:?} speedup={:.1}x",
        without.as_secs_f64() / with.as_secs_f64().max(f64::MIN_POSITIVE)
    );
    assert!(
        with < without,
        "symmetry breaking must speed up raw CDCL on pigeonhole: broken={with:?} plain={without:?}"
    );
}

/// THE POWER, pulled out and measured to its bound. `PHP(n)` is the canonical exponential-hard SAT
/// instance: every resolution/CDCL refutation has size `2^Ω(n)` (Haken, 1985) — the wall that stops raw
/// CDCL *and* Z3 cold. We measure the **conflict count** (machine-independent search work) for plain vs
/// symmetry-broken CDCL as `n` grows: plain explodes, the broken count stays small, and crucially the
/// **collapse ratio widens with `n`** — the hallmark of an exponential-vs-polynomial separation, made
/// empirical. Then the matching reasoner decides the SAME family with **zero search** at a scale where
/// CDCL is exponentially dead — the polynomial bound, realized and certified.
#[test]
fn pigeonhole_exponential_collapses_to_polynomial_certified() {
    use crate::cdcl::SolveResult;
    use crate::cnf::Cnf;
    let conflicts_of = |e: &ProofExpr| -> u64 {
        let mut cnf = Cnf::new();
        cnf.assert(e).expect("clausifiable");
        let (mut solver, _) = cnf.into_solver_with_atoms();
        assert!(
            matches!(solver.solve(), SolveResult::Unsat),
            "PHP must be UNSAT"
        );
        solver.conflicts()
    };

    let mut plains = Vec::new();
    let mut ratios = Vec::new();
    for n in 4..=8 {
        let plain = conflicts_of(&php(n));
        let broken = conflicts_of(&break_symmetries(&php(n)));
        eprintln!(
            "PHP({n}): plain conflicts={plain}  broken={broken}  collapse={:.1}x",
            plain as f64 / broken.max(1) as f64
        );
        plains.push(plain);
        ratios.push(plain as f64 / broken.max(1) as f64);
    }

    // Plain CDCL conflicts explode super-linearly with n — the Haken wall, empirically.
    assert!(
        *plains.last().unwrap() >= plains[0].saturating_mul(plains.len() as u64),
        "plain CDCL conflicts must explode with n: {plains:?}"
    );
    // The symmetry-breaking collapse WIDENS with n: the win grows (polynomial pulling away from exponential).
    assert!(
        *ratios.last().unwrap() > ratios[0] * 1.5,
        "the symmetry-breaking collapse must widen with n: {ratios:?}"
    );

    // The matching reasoner decides the WHOLE family in polynomial time with a re-verified Hall witness —
    // certified UNSAT, zero search, at sizes where every CDCL/resolution refutation is astronomically large.
    for n in [10usize, 16, 24, 32] {
        assert!(
            crate::pigeonhole::decide_pigeonhole_unsat(&php(n)),
            "matching: PHP({n}) certified UNSAT with no search"
        );
    }
    eprintln!("matching reasoner: PHP(10..32) all certified UNSAT, polynomial, zero search — CDCL is exponentially dead here");
}

#[test]
fn non_symmetric_formula_is_left_alone() {
    // A formula whose "rows" are NOT interchangeable (asymmetric extra clause) gets no SBP for
    // that pair — the automorphism check must reject it. Here row 0 and row 1 differ because an
    // extra unit clause pins one of row 0's variables, so the swap is not an automorphism.
    let f = and(
        and(or(a("p_0_0"), a("p_0_1")), or(a("p_1_0"), a("p_1_1"))),
        a("p_0_0"), // breaks the row-0/row-1 symmetry
    );
    // With no verified symmetry the formula is returned unchanged.
    assert_eq!(
        break_symmetries(&f),
        f,
        "asymmetric formula must be left unchanged"
    );
}

/// The collapse curve, charted out to `PHP(10)`. Heavy because plain CDCL grows `~×5` per pigeon, so
/// `PHP(10)` costs ~`10⁵` conflicts — the very point: it is the exponential wall, rendered. We measure
/// the conflict count plain vs symmetry-broken for `n = 4..10`, draw a `log₂` bar chart of both curves
/// (plain a rising staircase, broken nearly flat), and bank it. `#[ignore]` by default; run on demand.
#[test]
#[ignore = "heavy (~seconds): plain CDCL on PHP(9..10) is exponential — that's the point. Charts the curve."]
fn pigeonhole_collapse_curve_to_php10() {
    use crate::cdcl::SolveResult;
    use crate::cnf::Cnf;
    let conflicts_of = |e: &ProofExpr| -> u64 {
        let mut cnf = Cnf::new();
        cnf.assert(e).expect("clausifiable");
        let (mut solver, _) = cnf.into_solver_with_atoms();
        assert!(matches!(solver.solve(), SolveResult::Unsat));
        solver.conflicts()
    };
    let bar = |v: u64| "█".repeat((64.0 * (v.max(1) as f64).log2() / (200000f64).log2()) as usize);
    let mut rows = Vec::new();
    rows.push("  n |   plain |  broken |  collapse | log₂ plain (█) vs broken (░)".to_string());
    rows.push("----+---------+---------+-----------+-----------------------------".to_string());
    for n in 4..=10 {
        let plain = conflicts_of(&php(n));
        let broken = conflicts_of(&break_symmetries(&php(n)));
        rows.push(format!(
            "{n:3} | {plain:7} | {broken:7} | {:7.1}× | {}\n    |         |         |           | {}",
            plain as f64 / broken.max(1) as f64,
            bar(plain),
            "░".repeat(bar(broken).chars().count())
        ));
    }
    let chart = rows.join("\n");
    eprintln!("\nPIGEONHOLE COLLAPSE — exponential plain vs polynomial broken\n{chart}\n");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("pigeonhole_collapse_curve.txt"), format!("PIGEONHOLE COLLAPSE — exponential plain CDCL vs polynomial certified symmetry breaking\n\n{chart}\n"));
    }
    // The wall is real: PHP(10) plain conflicts dwarf PHP(4), while broken barely moved.
    let p10 = conflicts_of(&php(10));
    let b10 = conflicts_of(&break_symmetries(&php(10)));
    assert!(
        p10 > 50_000,
        "PHP(10) plain CDCL hits the exponential wall: {p10} conflicts"
    );
    assert!(
        b10 < 100,
        "broken PHP(10) stays polynomial: {b10} conflicts"
    );
}
