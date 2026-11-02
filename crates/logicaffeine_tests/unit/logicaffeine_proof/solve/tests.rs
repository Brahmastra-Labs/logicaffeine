use super::*;
use crate::families::{clique_coloring, php, tseitin_expander};
use crate::rup::check_refutation;

#[test]
fn pigeonhole_is_crushed_by_a_specialist_not_cdcl() {
    // PHP(20): 20 pigeons into 19 holes — exponential for resolution, but a polynomial matching
    // refutation. It must be decided by a structural route, never the CDCL fallback.
    let (cnf, _) = php(20);
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    assert!(matches!(solved.answer, Answer::Unsat));
    assert_ne!(
        solved.via,
        Route::Cdcl,
        "PHP must be crushed structurally, got CDCL"
    );
}

#[test]
fn massive_pigeonhole_does_not_fall_to_search() {
    // The headline: a large PHP a CDCL solver cannot touch is decided structurally and fast
    // (the test simply completing proves it did not enter exponential search).
    let (cnf, _) = php(50);
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    assert!(matches!(solved.answer, Answer::Unsat));
    assert_ne!(solved.via, Route::Cdcl);
}

#[test]
fn clique_colouring_is_crushed_structurally() {
    let (cnf, _) = clique_coloring(8, 7);
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    assert!(matches!(solved.answer, Answer::Unsat));
    assert_ne!(
        solved.via,
        Route::Cdcl,
        "clique-colouring must be crushed structurally"
    );
}

#[test]
fn sparse_sat_is_certified_by_lll_not_search() {
    // Four width-4 clauses over DISJOINT variable sets: dependency degree 0, so the Lovász Local
    // Lemma guarantees a model. The dispatcher must certify SAT via the LLL / Moser–Tardos route
    // with a genuine, re-checked model — the SAT-side specialist, not the CDCL fallback.
    let cl = |vs: [u32; 4]| vs.iter().map(|&v| Lit::pos(v)).collect::<Vec<_>>();
    let clauses = vec![
        cl([0, 1, 2, 3]),
        cl([4, 5, 6, 7]),
        cl([8, 9, 10, 11]),
        cl([12, 13, 14, 15]),
    ];
    let solved = solve_structured(16, &clauses);
    assert_eq!(
        solved.via,
        Route::Lll,
        "a locally-sparse SAT formula must route to LLL"
    );
    match &solved.answer {
        Answer::Sat(model) => assert!(
            clauses
                .iter()
                .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
            "the LLL/Moser–Tardos witness must satisfy every clause"
        ),
        Answer::Unsat => panic!("a satisfiable sparse formula must not be reported UNSAT"),
    }
}

#[test]
fn tseitin_parity_is_crushed_structurally() {
    let (_, cnf, _) = tseitin_expander(40, 7);
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    assert!(matches!(solved.answer, Answer::Unsat));
    assert!(
        matches!(solved.via, Route::Parity | Route::Collapse),
        "Tseitin parity must go through the GF(2) route, got {:?}",
        solved.via
    );
}

#[test]
fn two_sat_is_decided_with_a_model() {
    // (x0 ∨ x1) ∧ (¬x0 ∨ x1) ∧ (x0 ∨ ¬x1) — 2-SAT, satisfiable (x0=x1=true).
    let clauses = vec![
        vec![Lit::new(0, true), Lit::new(1, true)],
        vec![Lit::new(0, false), Lit::new(1, true)],
        vec![Lit::new(0, true), Lit::new(1, false)],
    ];
    let solved = solve_structured(2, &clauses);
    assert_eq!(solved.via, Route::TwoSat);
    match solved.answer {
        Answer::Sat(m) => {
            for c in &clauses {
                assert!(c.iter().any(|l| m[l.var() as usize] == l.is_positive()));
            }
        }
        Answer::Unsat => panic!("instance is SAT"),
    }
}

#[test]
fn horn_is_decided_with_its_least_model() {
    // facts a, b; and a ∧ b → c (a 3-literal Horn clause, so it is NOT 2-SAT and reaches the
    // Horn route). Satisfiable, least model {a,b,c}.
    let clauses = vec![
        vec![Lit::new(0, true)],
        vec![Lit::new(1, true)],
        vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, true)],
    ];
    let solved = solve_structured(3, &clauses);
    assert_eq!(solved.via, Route::Horn);
    match solved.answer {
        Answer::Sat(m) => assert!(m[0] && m[1] && m[2]),
        Answer::Unsat => panic!("instance is SAT"),
    }
}

#[test]
fn unstructured_sat_returns_a_model_via_cdcl() {
    // A 3-literal clause keeps it out of the 2-SAT / Horn classes; it is satisfiable.
    let clauses = vec![
        vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)],
        vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, false)],
    ];
    let solved = solve_structured(3, &clauses);
    match solved.answer {
        Answer::Sat(m) => {
            for c in &clauses {
                assert!(c.iter().any(|l| m[l.var() as usize] == l.is_positive()));
            }
        }
        Answer::Unsat => panic!("instance is SAT"),
    }
}

#[test]
fn cdcl_route_carries_a_valid_rup_certificate() {
    // All 8 clauses over 3 vars → UNSAT. Whichever route fires, the CDCL fallback's proof must
    // be a valid RUP refutation; a polynomial specialist certifies internally.
    let clauses = vec![
        vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)],
        vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, false)],
        vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, true)],
        vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, false)],
        vec![Lit::new(0, true), Lit::new(1, false), Lit::new(2, true)],
        vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, false)],
        vec![Lit::new(0, true), Lit::new(1, false), Lit::new(2, false)],
        vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, true)],
    ];
    let solved = solve_structured(3, &clauses);
    assert!(matches!(solved.answer, Answer::Unsat));
    if solved.via == Route::Cdcl {
        let learned: Vec<Vec<Lit>> = solved.proof.iter().map(|s| s.clause().to_vec()).collect();
        assert!(check_refutation(3, &clauses, &learned));
    }
}

/// The 2^(k-1) gadget clauses encoding XOR(vars)=rhs — each forbids one wrong-parity row.
fn xor_gadget(vars: &[u32], rhs: bool) -> Vec<Vec<Lit>> {
    let k = vars.len();
    let mut clauses = Vec::new();
    for mask in 0u32..(1 << k) {
        if ((mask.count_ones() % 2) == 1) != rhs {
            clauses.push(
                (0..k)
                    .map(|i| Lit::new(vars[i], (mask >> i) & 1 == 0))
                    .collect(),
            );
        }
    }
    clauses
}

#[test]
fn hybrid_xor_solves_an_xor_heavy_sat_instance_with_a_valid_model() {
    // XOR gadgets x0⊕x1⊕x2=0, x2⊕x3=1, unit x0; plus a genuine residual clause (x1∨x3) that is
    // NOT a complete gadget — so CDCL must repair the seeded GF(2) witness to satisfy it.
    let mut clauses = xor_gadget(&[0, 1, 2], false);
    clauses.extend(xor_gadget(&[2, 3], true));
    clauses.push(vec![Lit::new(0, true)]);
    clauses.push(vec![Lit::new(1, true), Lit::new(3, true)]);
    let solved = solve_structured(4, &clauses);
    assert_eq!(
        solved.via,
        Route::HybridXor,
        "XOR-heavy SAT must take the hybrid route"
    );
    match solved.answer {
        Answer::Sat(m) => {
            for c in &clauses {
                assert!(
                    c.iter().any(|l| m[l.var() as usize] == l.is_positive()),
                    "model fails {c:?}"
                );
            }
        }
        Answer::Unsat => panic!("instance is SAT"),
    }
}

#[test]
fn fused_route_decides_a_mixed_parity_cardinality_instance() {
    // Exactly-one of {0,1,2} linked by equalities to {3,4,5} under even parity: exactly-one forces an
    // ODD count, the parity forces EVEN — UNSAT, a MIX neither the parity route nor the covering cut
    // refutes alone. The fused rung (recovered XOR + recovered at-most-one, reasoned together) decides
    // it; verdict must be UNSAT and the fused route must actually fire (not fall through to plain CDCL).
    let mut clauses: Vec<Vec<Lit>> = vec![
        vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)],
        vec![Lit::new(0, false), Lit::new(1, false)],
        vec![Lit::new(0, false), Lit::new(2, false)],
        vec![Lit::new(1, false), Lit::new(2, false)],
    ];
    for i in 0..3u32 {
        clauses.extend(xor_gadget(&[i, i + 3], false)); // x_i = x_{i+3}
    }
    clauses.extend(xor_gadget(&[3, 4, 5], false)); // x3 ⊕ x4 ⊕ x5 = 0
    let solved = solve_comprehensive(6, &clauses);
    assert!(
        matches!(solved.answer, Answer::Unsat),
        "the mixed instance is UNSAT (via {:?})",
        solved.via
    );
    assert_eq!(
        solved.via,
        Route::HybridXor,
        "the fused parity+cardinality route must fire"
    );
}

#[test]
fn xor_inconsistent_system_is_refuted_structurally() {
    // x0⊕x1=0 ∧ x1⊕x2=0 ∧ x0⊕x2=1 sums to 0=1 — UNSAT by Gaussian alone, never CDCL search.
    let mut clauses = xor_gadget(&[0, 1], false);
    clauses.extend(xor_gadget(&[1, 2], false));
    clauses.extend(xor_gadget(&[0, 2], true));
    let solved = solve_structured(3, &clauses);
    assert!(matches!(solved.answer, Answer::Unsat));
    assert_ne!(
        solved.via,
        Route::Cdcl,
        "a contradictory linear system must collapse structurally"
    );
}

#[test]
fn mod_p_tseitin_cnf_is_lifted_to_gf_p_not_left_to_cdcl() {
    // The mod-3 Tseitin obstruction encoded as opaque Boolean CNF. It is invisible to the GF(2)
    // parity cut (the whole point of the family), so the dispatcher must RECOVER the one-hot GF(p)
    // system from the raw clauses and crush it by Gaussian elimination over the right field — never
    // fall to CDCL, which (like Z3 and Kissat) needs exponential resolution here.
    for &p in &[3u64, 5] {
        let (_, cnf, _) = crate::families::mod_p_tseitin_expander(6, p, 0xC0FFEE);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(
            matches!(solved.answer, Answer::Unsat),
            "mod-{p} Tseitin is UNSAT"
        );
        assert_eq!(
            solved.via,
            Route::ModP,
            "mod-{p} CNF must be lifted to the GF(p) route"
        );
        assert_eq!(solved.conflicts, 0, "the GF(p) collapse spends no search");
    }
}

#[test]
fn the_gf_p_route_returns_a_verified_model_on_a_satisfiable_mod_p_cnf() {
    // A *consistent* mod-p divergence system (total charge 0) is satisfiable; the recovered GF(p)
    // route must report SAT with a Boolean model that genuinely satisfies every original clause.
    let p = 3u64;
    let (_, cnf, _) = crate::families::mod_p_consistent_onehot(6, p, 0xABCD);
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    assert_eq!(
        solved.via,
        Route::ModP,
        "a consistent mod-p one-hot CNF must take the GF(p) route"
    );
    match &solved.answer {
        Answer::Sat(m) => assert!(
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the GF(p) model must satisfy every clause"
        ),
        Answer::Unsat => panic!("a consistent mod-p system must be SAT"),
    }
}

#[test]
fn the_gf_p_route_never_misfires_on_unstructured_or_non_linear_cnf() {
    // Soundness of the lift: it must decline (recover → None) on inputs that are NOT a clean mod-p
    // one-hot encoding, leaving them to the honest routes. Random 3-SAT and plain PHP must not be
    // misrouted to ModP.
    let rnd = crate::families::random_3sat(30, 120, 0x5EED);
    let rnd_via = solve_structured(rnd.num_vars, &rnd.clauses).via;
    assert_ne!(rnd_via, Route::ModP);
    assert_ne!(
        rnd_via,
        Route::ModM,
        "random must not be misrouted to the composite lift either"
    );
    let (php_cnf, _) = php(6);
    let php_via = solve_structured(php_cnf.num_vars, &php_cnf.clauses).via;
    assert_ne!(php_via, Route::ModP);
    assert_ne!(php_via, Route::ModM);
}

#[test]
fn the_gf_p_route_agrees_with_boolean_brute_force_on_tiny_instances() {
    // The ultimate oracle: on instances small enough to enumerate every Boolean assignment, the GF(p)
    // lift's verdict matches exhaustive search — on both the UNSAT (Tseitin) and SAT (consistent)
    // forms. K₄ at p=3 is 6 edges × 3 bits = 18 variables, a 2¹⁸ sweep.
    for (cnf, want_sat) in [
        (crate::families::mod_p_tseitin_expander(4, 3, 1).1, false),
        (crate::families::mod_p_consistent_onehot(4, 3, 1).1, true),
    ] {
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert_eq!(
            solved.via,
            Route::ModP,
            "a tiny mod-3 instance must take the GF(p) route"
        );
        let brute = (0u64..(1u64 << cnf.num_vars)).any(|code| {
            let asg: Vec<bool> = (0..cnf.num_vars).map(|i| (code >> i) & 1 == 1).collect();
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| asg[l.var() as usize] == l.is_positive()))
        });
        assert_eq!(
            matches!(solved.answer, Answer::Sat(_)),
            brute,
            "GF(p) verdict must match brute force"
        );
        assert_eq!(brute, want_sat, "family verdict sanity");
    }
}

#[test]
fn composite_modulus_onehot_cnf_is_lifted_to_zmod_m_not_left_to_cdcl() {
    // A mod-6 one-hot Tseitin system: COMPOSITE modulus. It is invisible to GF(2) and the obstruction
    // lives in the GF(3) factor. The dispatcher must recover the ℤ/6 system from the raw clauses and
    // decide it by CRT over the prime-power components — the composite lift — never fall to CDCL.
    let (_, cnf, _) = crate::families::mod_p_tseitin_expander(6, 6, 0xC0FFEE);
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    assert!(
        matches!(solved.answer, Answer::Unsat),
        "mod-6 Tseitin is UNSAT"
    );
    assert_eq!(
        solved.via,
        Route::ModM,
        "a composite mod-6 CNF must be lifted to the ℤ/m route"
    );
    assert_eq!(solved.conflicts, 0, "the ℤ/m collapse spends no search");
}

#[test]
fn the_zmod_m_route_returns_a_verified_model_on_a_satisfiable_composite_cnf() {
    let (_, cnf, _) = crate::families::mod_p_consistent_onehot(6, 6, 0xABCD);
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    assert_eq!(
        solved.via,
        Route::ModM,
        "a consistent composite one-hot CNF must take the ℤ/m route"
    );
    match &solved.answer {
        Answer::Sat(m) => assert!(
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the ℤ/m model must satisfy every clause"
        ),
        Answer::Unsat => panic!("a consistent composite system must be SAT"),
    }
}

#[test]
fn the_sos_route_is_sound_in_the_dispatcher() {
    // SoS is wired as the last specialist before CDCL (`Route::Sos`). This pins its CONTRACT in the
    // dispatcher: whatever it routes is genuinely UNSAT, and a satisfiable instance is never routed
    // to it. (Empirically it rarely *fires* — small instances are decided faster by CDCL and its
    // degree-2 niche is caught by the cheaper specialists — but the wiring is always sound.)
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    fn brute_sat(nv: usize, cl: &[Vec<Lit>]) -> bool {
        (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        })
    }
    let mut state = 0x5005_7777u64;
    for _ in 0..120 {
        let nv = 4usize;
        let m = 6 + (sm(&mut state) % 8) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut vs: Vec<u32> = Vec::new();
            while vs.len() < 3 {
                let v = (sm(&mut state) % nv as u64) as u32;
                if !vs.contains(&v) {
                    vs.push(v);
                }
            }
            cl.push(
                vs.iter()
                    .map(|&v| Lit::new(v, sm(&mut state) % 2 == 0))
                    .collect(),
            );
        }
        let solved = solve_structured(nv, &cl);
        let sat = brute_sat(nv, &cl);
        if solved.via == Route::Sos {
            assert!(
                matches!(solved.answer, Answer::Unsat),
                "the SoS route only ever refutes"
            );
            assert!(
                !sat,
                "the SoS route must never fire on a satisfiable instance: {cl:?}"
            );
        }
        if sat {
            assert_ne!(
                solved.via,
                Route::Sos,
                "a satisfiable instance must not be routed to SoS"
            );
        }
    }
}

#[test]
fn solve_comprehensive_matches_brute_force() {
    // The full-arsenal power solver must decide correctly across a fuzz (the CDCL fallback is
    // complete; the heavy Nullstellensatz / symmetry-breaking routes must never change the verdict),
    // and every reported model must satisfy the formula.
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    fn brute_sat(nv: usize, cl: &[Vec<Lit>]) -> bool {
        (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        })
    }
    let mut state = 0xC0DE_5A1Du64;
    for _ in 0..80 {
        let nv = 3 + (sm(&mut state) % 3) as usize; // 3..5
        let m = 2 + (sm(&mut state) % 10) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for v in 0..nv {
                if sm(&mut state) % 2 == 0 {
                    c.push(Lit::new(v as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        let solved = solve_comprehensive(nv, &cl);
        assert_eq!(
            matches!(solved.answer, Answer::Sat(_)),
            brute_sat(nv, &cl),
            "solve_comprehensive verdict must match brute force via {:?}: {cl:?}",
            solved.via
        );
        if let Answer::Sat(m) = &solved.answer {
            assert!(
                cl.iter()
                    .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "a reported model must satisfy every clause: {cl:?}"
            );
        }
    }
}

#[test]
fn the_symmetry_break_route_solves_a_symmetric_instance() {
    // clique_coloring(3,3) is SAT and richly symmetric (S₃ vertices × S₃ colours); it slips past the
    // cheap specialists, so the comprehensive solver's complete-symmetry-breaking route decides it
    // with a re-checked model.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let s = symmetry_break_solve(cnf.num_vars, &cnf.clauses)
        .expect("clique colouring has a usable, phase-free symmetry group");
    assert_eq!(s.via, Route::SymmetryBreak);
    match s.answer {
        Answer::Sat(m) => assert!(
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the symmetry-break route returns a valid model"
        ),
        Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
    }
}

#[test]
fn dynamic_sel_refutes_a_symmetric_instance_in_search() {
    // Dynamic in-CDCL symmetry breaking (SEL): refute PHP by amplifying learned clauses with the
    // symmetry group during the budgeted search — the dynamic complement to the static lex-leader,
    // with a proof checked internally before it is returned. (In the full dispatcher PHP is caught
    // earlier by the pigeonhole specialist; this exercises the SEL route directly.)
    let (cnf, _) = crate::families::php(5);
    let solved =
        dynamic_sel(cnf.num_vars, &cnf.clauses).expect("PHP is symmetric — dynamic SEL engages");
    assert_eq!(solved.via, Route::Sel);
    assert!(matches!(solved.answer, Answer::Unsat), "PHP(5) is UNSAT");
    assert!(!solved.proof.is_empty(), "SEL returns a refutation proof");
}

#[test]
fn orbital_branch_collapses_symmetric_branches_and_is_correct() {
    // SAT, richly symmetric: clique_coloring(3,3). The whole variable grid is one orbit under
    // S₃(vertices)×S₃(colours), so orbital branching collapses every "some-cell-true" branch into the
    // single representative branch and decides it with a re-checked model.
    let (sat_cnf, _) = crate::families::clique_coloring(3, 3);
    let s = orbital_branch_solve(sat_cnf.num_vars, &sat_cnf.clauses)
        .expect("a large variable orbit drives orbital branching");
    assert_eq!(s.via, Route::OrbitalBranch);
    match &s.answer {
        Answer::Sat(m) => assert!(
            sat_cnf
                .clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "orbital branching returns a valid model"
        ),
        Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
    }

    // UNSAT, richly symmetric: clique_coloring(4,3) — K₄ cannot be 3-coloured. Orbital concludes UNSAT
    // only because BOTH branches (rep-true and all-orbit-false) are genuinely UNSAT.
    let (unsat_cnf, _) = crate::families::clique_coloring(4, 3);
    let u = orbital_branch_solve(unsat_cnf.num_vars, &unsat_cnf.clauses)
        .expect("a large variable orbit drives orbital branching");
    assert_eq!(u.via, Route::OrbitalBranch);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "clique_coloring(4,3) is UNSAT"
    );

    // The RECURSION: clique_coloring(4,4) (16 vars) is SAT and far too symmetric to dispatch in one
    // split — the representative branch is itself symmetric, so orbital branching descends the
    // residual group level by level and still returns a re-checked model.
    let (big_cnf, _) = crate::families::clique_coloring(4, 4);
    let big = orbital_branch_solve(big_cnf.num_vars, &big_cnf.clauses)
        .expect("recursive orbital branching engages on the larger grid");
    assert_eq!(big.via, Route::OrbitalBranch);
    match &big.answer {
        Answer::Sat(m) => assert!(
            big_cnf
                .clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the recursive orbital model is valid"
        ),
        Answer::Unsat => panic!("clique_coloring(4,4) is SAT"),
    }

    // Soundness: the orbital verdict matches an independent brute-force decision across all instances.
    for (cnf, _) in [
        crate::families::clique_coloring(3, 3),
        crate::families::clique_coloring(4, 3),
        crate::families::clique_coloring(4, 4),
    ] {
        let nv = cnf.num_vars;
        let brute = (0u64..(1u64 << nv)).any(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        let got = orbital_branch_solve(nv, &cnf.clauses).expect("orbital fires");
        assert_eq!(
            matches!(got.answer, Answer::Sat(_)),
            brute,
            "orbital-branch verdict matches brute force (nv={nv})"
        );
    }
}

#[test]
fn solve_by_symmetry_breaking_matches_brute_force() {
    let instances = [
        crate::families::clique_coloring(3, 3), // SAT
        crate::families::clique_coloring(4, 3), // UNSAT (K₄ not 3-colourable)
        crate::families::clique_coloring(4, 4), // SAT
    ];
    for (cnf, _) in instances {
        let nv = cnf.num_vars;
        let brute = (0u64..(1u64 << nv)).any(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        let solved = solve_by_symmetry_breaking(nv, &cnf.clauses);
        assert_eq!(
            matches!(solved.answer, Answer::Sat(_)),
            brute,
            "break-then-solve matches brute (nv={nv})"
        );
        if let Answer::Sat(m) = &solved.answer {
            assert!(
                cnf.clauses
                    .iter()
                    .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the projected model satisfies the original formula"
            );
        }
    }
}

#[test]
fn break_all_symmetry_complete_leaves_one_model_per_orbit() {
    // Count the distinct ORIGINAL-variable models of a (possibly aux-laden) formula by blocking each
    // original projection in turn.
    let count_original = |total: usize, cl: &[Vec<Lit>], nv: usize| -> usize {
        let mut working = cl.to_vec();
        let mut count = 0;
        loop {
            match solve_comprehensive(total, &working).answer {
                Answer::Unsat => break,
                Answer::Sat(m) => {
                    count += 1;
                    working.push((0..nv).map(|v| Lit::new(v as u32, !m[v])).collect());
                }
            }
        }
        count
    };
    let n = |v| Lit::new(v, false);

    // at-most-1-of-3 (S₃): models {000,100,010,001} → 2 orbits. Complete breaking ⇒ exactly 2 survive.
    let amo = vec![vec![n(0), n(1)], vec![n(0), n(2)], vec![n(1), n(2)]];
    let orbits_amo = models_up_to_symmetry(3, &amo, 1000).representatives.len();
    let (b1, t1) = break_all_symmetry_complete(3, &amo);
    assert_eq!(
        count_original(t1, &b1, 3),
        orbits_amo,
        "one model per orbit (at-most-1-of-3)"
    );

    // clique_coloring(3,3) (S₃×S₃): the 6 proper colourings form 1 orbit ⇒ exactly 1 model survives.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let orbits_clq = models_up_to_symmetry(cnf.num_vars, &cnf.clauses, 1000)
        .representatives
        .len();
    let (b2, t2) = break_all_symmetry_complete(cnf.num_vars, &cnf.clauses);
    assert_eq!(
        orbits_clq, 1,
        "clique(3,3)'s colourings form a single orbit"
    );
    assert_eq!(
        count_original(t2, &b2, cnf.num_vars),
        1,
        "complete breaking leaves exactly one model (clique)"
    );

    // No symmetry ⇒ returned unchanged.
    let asym = vec![vec![Lit::new(0, true)], vec![n(1), Lit::new(2, true)]];
    let (b3, t3) = break_all_symmetry_complete(3, &asym);
    assert_eq!((b3.len(), t3), (asym.len(), 3), "no symmetry ⇒ no breaks");
}

#[test]
fn break_all_symmetry_runs_to_a_fixpoint_soundly() {
    // clique_coloring(3,3): rich S₃×S₃ symmetry. The automated breaker drives it down, stays
    // equisatisfiable, and reaches a fixpoint.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let before = symmetry_structure(cnf.num_vars, &cnf.clauses).order;
    let broken = break_all_symmetry(cnf.num_vars, &cnf.clauses);
    let after = symmetry_structure(cnf.num_vars, &broken).order;
    assert!(
        after < before,
        "the breaker reduced the symmetry group: {before} → {after}"
    );

    // Equisatisfiable: same verdict as the original, and the broken model satisfies the ORIGINAL.
    let orig = solve_comprehensive(cnf.num_vars, &cnf.clauses).answer;
    let brk = solve_comprehensive(cnf.num_vars, &broken).answer;
    assert_eq!(
        matches!(orig, Answer::Sat(_)),
        matches!(brk, Answer::Sat(_)),
        "breaking preserves the verdict"
    );
    if let Answer::Sat(m) = &brk {
        assert!(
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the broken-formula model is a genuine model of the original"
        );
    }

    // Fixpoint: breaking the already-broken formula adds nothing.
    let twice = break_all_symmetry(cnf.num_vars, &broken);
    assert_eq!(
        twice.len(),
        broken.len(),
        "already at the fixpoint — re-breaking is a no-op"
    );

    // An asymmetric formula is returned unchanged (nothing to break).
    let asym = vec![
        vec![Lit::new(0, true)],
        vec![Lit::new(1, false), Lit::new(2, true)],
    ];
    assert_eq!(
        break_all_symmetry(3, &asym).len(),
        asym.len(),
        "no symmetry ⇒ no breaks"
    );
}

#[test]
fn class_algebra_constants_wrapper_has_the_right_shape() {
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let k = symmetry_structure(cnf.num_vars, &cnf.clauses)
        .conjugacy_classes
        .unwrap(); // 9
    let a = class_algebra_constants(cnf.num_vars, &cnf.clauses).expect("enumerable group");
    assert_eq!(a.len(), k, "the class-algebra tensor is k × k × k");
    assert!(a
        .iter()
        .all(|m| m.len() == k && m.iter().all(|r| r.len() == k)));
}

#[test]
fn character_table_wrapper_is_the_grid_groups_table() {
    // The variable symmetry of clique_coloring(3,3) is S₃×S₃; its character table has one row per
    // conjugacy class with the product degrees, satisfies Σ dᵢ² = |G|, and re-checks orthogonality.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let t = character_table(cnf.num_vars, &cnf.clauses).expect("enumerable group");
    assert_eq!(
        t.degrees,
        vec![1, 1, 1, 1, 2, 2, 2, 2, 4],
        "S₃×S₃ irreducible degrees"
    );
    assert_eq!(
        t.degrees.iter().map(|d| d * d).sum::<u128>(),
        36,
        "Σ dᵢ² = |S₃×S₃|"
    );
    assert_eq!(
        t.values.len(),
        t.degrees.len(),
        "one character per irreducible"
    );
    assert!(
        t.values.iter().all(|row| row.len() == t.class_sizes.len()),
        "a value per conjugacy class"
    );
    assert!(
        t.values.iter().any(|row| row.iter().all(|&x| x == 1)),
        "the trivial character is present"
    );
}

#[test]
fn frobenius_schur_wrapper_reports_a_real_grid_group() {
    // S₃×S₃ is totally real, so every indicator is +1, one per irreducible.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let fs = frobenius_schur_indicators(cnf.num_vars, &cnf.clauses).expect("enumerable group");
    assert_eq!(fs, vec![1; 9], "S₃×S₃: nine real irreducibles");
}

#[test]
fn isotypic_decomposition_wrapper_bridges_action_and_representation() {
    // clique_coloring(3,3): S₃×S₃ on 9 cells. The permutation character decomposes with the trivial
    // multiplicity equal to the orbit count (1, transitive) and Σ m·d = 9.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let iso = isotypic_multiplicities(cnf.num_vars, &cnf.clauses).expect("enumerable group");
    let degs = symmetry_structure(cnf.num_vars, &cnf.clauses)
        .irreducible_degrees
        .unwrap();
    assert_eq!(
        iso.iter().zip(&degs).map(|(m, d)| m * d).sum::<u128>(),
        9,
        "Σ m·d = 9 cells"
    );
    // The permutation character: the identity fixes all 9 cells.
    let pi = permutation_character(cnf.num_vars, &cnf.clauses).expect("enumerable group");
    assert!(pi.contains(&9), "the identity class fixes all 9 variables");
}

#[test]
fn automorphism_group_wrapper_measures_the_symmetry_of_the_symmetry() {
    // clique_coloring(3,3): the variable symmetry is S₃×S₃, whose automorphism group has order 72.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    assert_eq!(
        automorphism_group_order(cnf.num_vars, &cnf.clauses),
        Some(72),
        "|Aut(S₃×S₃)| = 72"
    );
}

#[test]
fn table_of_marks_wrapper_classifies_a_grid_groups_g_sets() {
    // clique_coloring(3,3): the variable symmetry is S₃×S₃; its table of marks is triangular with the
    // trivial-subgroup row giving the coset indices [G:H] and the full-group column all ones.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let (orders, marks) =
        table_of_marks(cnf.num_vars, &cnf.clauses).expect("subgroup lattice in range");
    let k = orders.len();
    assert_eq!(
        *orders.last().unwrap(),
        36,
        "the whole group S₃×S₃ has order 36"
    );
    assert_eq!(
        marks[0][0], 36,
        "m(1, 1) = [G:1] = |G| = 36 (the regular action)"
    );
    for j in 0..k {
        assert_eq!(marks[0][j], 36 / orders[j], "m(1, H_j) = [G : H_j]");
        assert_eq!(
            marks[j][k - 1],
            1,
            "every subgroup fixes the one coset of G"
        );
    }
}

#[test]
fn galois_class_orbits_wrapper_partitions_a_rational_grid_group() {
    // S₃×S₃ is rational, so every Galois orbit on its 9 classes is a singleton.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let orbits = galois_class_orbits(cnf.num_vars, &cnf.clauses).expect("enumerable group");
    assert_eq!(orbits.len(), 9, "S₃×S₃ has 9 conjugacy classes");
    assert!(
        orbits.iter().all(|o| o.len() == 1),
        "all rational ⇒ every orbit is a singleton"
    );
    let prof = symmetry_structure(cnf.num_vars, &cnf.clauses);
    assert_eq!(prof.rational_classes, Some(9));
}

#[test]
fn tensor_decomposition_wrapper_is_a_valid_fusion_ring() {
    // clique_coloring(3,3): S₃×S₃, 9 irreducibles. The fusion coefficients form a valid representation
    // ring — every product has the right dimension and the trivial character is the unit.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let degs = symmetry_structure(cnf.num_vars, &cnf.clauses)
        .irreducible_degrees
        .unwrap();
    let n = tensor_decomposition(cnf.num_vars, &cnf.clauses).expect("enumerable group");
    let k = degs.len();
    assert_eq!(n.len(), k, "a k×k×k fusion tensor");
    for i in 0..k {
        for j in 0..k {
            assert_eq!(
                (0..k).map(|c| n[i][j][c] * degs[c]).sum::<u128>(),
                degs[i] * degs[j],
                "dim(χ_i ⊗ χ_j) = d_i·d_j"
            );
        }
    }
}

#[test]
fn equivalence_symmetry_matches_brute_force_and_reduces() {
    let p = |v: u32| Lit::new(v, true);
    let nl = |v: u32| Lit::new(v, false);
    let sat = |m: &[bool], cls: &[Vec<Lit>]| {
        cls.iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
    };
    let brute_equiv = |nv: usize, f: &[Vec<Lit>], s: &[Vec<Lit>]| -> bool {
        (0u32..(1u32 << nv)).all(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            sat(&a, f) == sat(&a, s)
        })
    };

    // 1. THE REDUCTION FIRES: at-most-one-of-4 is fully S₄-symmetric, so its 6 pair clauses fuse into
    //    ONE orbit — equivalence to itself is decided with far fewer entailment checks than the naive 12.
    let amo: Vec<Vec<Lit>> = (0..4u32)
        .flat_map(|i| ((i + 1)..4).map(move |j| vec![nl(i), nl(j)]))
        .collect();
    assert_eq!(amo.len(), 6);
    let gens = common_automorphism_generators(4, &amo, &amo);
    assert!(
        !gens.is_empty(),
        "at-most-one-of-4 has a nontrivial common symmetry group"
    );
    let (reps, naive) = equivalence_check_counts(4, &amo, &amo);
    assert!(
        reps < naive,
        "the common symmetry must reduce the work: {reps} < {naive}"
    );
    assert_eq!(
        equivalent_modulo_symmetry(4, &amo, &amo),
        EquivVerdict::Equivalent
    );

    // 2. EQUIVALENT but syntactically different: add a clause F already entails (resolvent x1). Still ≡.
    let f2 = vec![vec![p(0), p(1)], vec![nl(0), p(1)]];
    let mut s2 = f2.clone();
    s2.push(vec![p(1)]);
    assert!(
        brute_equiv(2, &f2, &s2),
        "sanity: the resolvent makes them equivalent"
    );
    assert_eq!(
        equivalent_modulo_symmetry(2, &f2, &s2),
        EquivVerdict::Equivalent
    );

    // 3. INEQUIVALENT: a dropped constraint changes the function; the witness must truly distinguish.
    let f3 = vec![vec![p(0), p(1)], vec![nl(0), nl(1)]];
    let s3 = vec![vec![p(0), p(1)]];
    match equivalent_modulo_symmetry(2, &f3, &s3) {
        EquivVerdict::Differ(m) => {
            assert_ne!(sat(&m, &f3), sat(&m, &s3), "witness must distinguish")
        }
        EquivVerdict::Equivalent => panic!("f3 and s3 are NOT equivalent"),
    }

    // 4. ASYMMETRIC pair: no usable common symmetry ⇒ no orbit fusion (and the verdict is still right).
    let fa = vec![vec![p(0), p(1)]];
    let sa = vec![vec![p(0), p(1)], vec![p(1), p(2)]];
    let (r, t) = equivalence_check_counts(3, &fa, &sa);
    assert_eq!(r, t, "no common symmetry ⇒ one check per clause");

    // 5. FUZZ: the symmetry-reduced verdict ALWAYS equals brute force, and every witness distinguishes.
    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let nv = 4usize;
    let mk = |s: &mut u64| -> Vec<Vec<Lit>> {
        let m = (xs(s) % 5) as usize; // 0..=4 clauses
        (0..m)
            .map(|_| {
                let w = 1 + (xs(s) % 2) as usize; // width 1 or 2
                (0..w)
                    .map(|_| Lit::new((xs(s) % nv as u64) as u32, xs(s) & 1 == 0))
                    .collect()
            })
            .collect()
    };
    let mut seed = 0x1234_5678_9abc_def0u64;
    for _ in 0..400 {
        let f = mk(&mut seed);
        let s = mk(&mut seed);
        let want = brute_equiv(nv, &f, &s);
        match equivalent_modulo_symmetry(nv, &f, &s) {
            EquivVerdict::Equivalent => {
                assert!(
                    want,
                    "claimed equivalent but brute says differ: F={f:?} S={s:?}"
                )
            }
            EquivVerdict::Differ(m) => {
                assert!(
                    !want,
                    "claimed differ but brute says equivalent: F={f:?} S={s:?}"
                );
                assert_ne!(sat(&m, &f), sat(&m, &s), "witness must distinguish F and S");
            }
        }
    }
}

#[test]
fn optimization_symmetry_matches_brute_force_and_reduces() {
    let p = |v: u32| Lit::new(v, true);
    let sat = |m: &[bool], cls: &[Vec<Lit>]| {
        cls.iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
    };
    let brute_min = |nv: usize, f: &[Vec<Lit>], w: &[i64]| -> Option<i64> {
        (0u32..(1u32 << nv))
            .filter_map(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                sat(&a, f).then(|| (0..nv).filter(|&i| a[i]).map(|i| w[i]).sum::<i64>())
            })
            .min()
    };

    // 1. THE REDUCTION FIRES: "at least one of 4", minimise the count (all weights 1). Fully S₄-symmetric
    //    (clauses AND weights), so the optimal "exactly one true" models form one orbit — far fewer
    //    candidate models to enumerate than the naive 15.
    let amo1 = vec![vec![p(0), p(1), p(2), p(3)]];
    let w1 = vec![1i64; 4];
    let (opt, m) = optimize_modulo_symmetry(4, &amo1, &w1).expect("F is satisfiable");
    assert_eq!(opt, 1, "minimum is one variable true");
    assert!(sat(&m, &amo1), "witness satisfies F");
    assert_eq!(m.iter().filter(|&&b| b).count(), 1, "witness has weight 1");
    let (with, without) = optimize_enumeration_counts(4, &amo1, &w1);
    assert!(
        with < without,
        "symmetry must shrink the enumeration: {with} < {without}"
    );

    // 2. UNSAT hard clauses ⇒ no optimum.
    assert_eq!(
        optimize_modulo_symmetry(2, &[vec![p(0)], vec![Lit::new(0, false)]], &[1, 1]),
        None
    );

    // 3. ASYMMETRIC weights kill the symmetry ⇒ no reduction (distinct weights ⇒ no objective-preserving
    //    permutation), and the optimum is still correct.
    let w_distinct = vec![1i64, 2, 3, 4];
    assert!(
        optimization_symmetry_generators(4, &amo1, &w_distinct).is_empty(),
        "distinct weights ⇒ no sym"
    );
    let (wa, wo) = optimize_enumeration_counts(4, &amo1, &w_distinct);
    assert_eq!(wa, wo, "no usable symmetry ⇒ no reduction");

    // 4. FUZZ: the symmetry-reduced optimum ALWAYS equals brute force, and the witness is a real model of
    //    F achieving exactly that weight.
    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let nv = 4usize;
    let mut seed = 0xC0FFEE_1234_5678u64;
    for _ in 0..400 {
        let m = (xs(&mut seed) % 5) as usize;
        let f: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let wclause = 1 + (xs(&mut seed) % 2) as usize;
                (0..wclause)
                    .map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0))
                    .collect()
            })
            .collect();
        // Weights in -2..=2 (symmetric instances arise when repeats let an automorphism preserve them).
        let weights: Vec<i64> = (0..nv).map(|_| (xs(&mut seed) % 5) as i64 - 2).collect();
        let want = brute_min(nv, &f, &weights);
        match optimize_modulo_symmetry(nv, &f, &weights) {
            None => assert!(want.is_none(), "claimed UNSAT but a model exists: F={f:?}"),
            Some((opt, wit)) => {
                assert_eq!(
                    Some(opt),
                    want,
                    "optimum must match brute force: F={f:?} w={weights:?}"
                );
                assert!(sat(&wit, &f), "witness must satisfy F");
                let ww: i64 = (0..nv).filter(|&i| wit[i]).map(|i| weights[i]).sum();
                assert_eq!(ww, opt, "witness must achieve the optimum");
            }
        }
    }
}

#[test]
fn fractional_automorphism_is_a_doubly_stochastic_commuting_relaxation() {
    let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)];

    // The canonical (block-averaging over the coarsest equitable partition) matrix is ALWAYS a valid
    // fractional automorphism — a doubly-stochastic matrix commuting with the co-occurrence graph.
    let (php, _) = crate::families::php(4);
    let (clq, _) = crate::families::clique_coloring(3, 3);
    let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();
    for (nv, cl) in [
        (php.num_vars, php.clauses.clone()),
        (clq.num_vars, clq.clauses.clone()),
        (6, c6.clone()),
    ] {
        let part = fractional_automorphism(nv, &cl);
        assert!(
            is_fractional_automorphism(nv, &cl, &part),
            "the equitable partition commutes with A"
        );
        // Non-trivial: symmetric formulas admit a fractional automorphism that is NOT a permutation.
        let cells = part.iter().copied().max().map_or(0, |m| m + 1);
        assert!(
            cells < nv,
            "a non-discrete partition ⇒ a genuine (non-permutation) fractional automorphism"
        );
        // Fractional symmetry contains integer symmetry: every automorphism orbit lies within one cell.
        let gens = crate::sym_break::variable_automorphism_generators(nv, &cl).unwrap_or_default();
        for orbit in crate::permgroup::orbits(nv, &gens) {
            assert!(
                orbit.iter().all(|&v| part[v] == part[orbit[0]]),
                "orbit ⊆ fractional-automorphism cell"
            );
        }
    }

    // The identity partition (each variable its own cell) is the trivial fractional automorphism (B = I).
    let (nv, _) = (clq.num_vars, ());
    assert!(
        is_fractional_automorphism(nv, &clq.clauses, &(0..nv).collect::<Vec<_>>()),
        "identity is a fractional automorphism"
    );

    // REJECTION: on the path 0–1–2–3, {0,1} | {2,3} is NOT equitable (vertex 0 and vertex 1 see different
    // cell-distributions), so it is not a fractional automorphism — but the reflection {0,3} | {1,2} is.
    let p4 = vec![edge(0, 1), edge(1, 2), edge(2, 3)];
    assert!(
        !is_fractional_automorphism(4, &p4, &[0, 0, 1, 1]),
        "a non-equitable partition is rejected"
    );
    assert!(
        is_fractional_automorphism(4, &p4, &[0, 1, 1, 0]),
        "the reflection partition is equitable"
    );
    assert_eq!(
        fractional_automorphism(4, &p4),
        vec![0, 1, 1, 0],
        "P₄'s coarsest equitable partition is the reflection"
    );

    // FUZZ: the canonical partition is always a valid fractional automorphism.
    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let n = 5usize;
    let mut seed = 0x0FAC_0000_1234_9999u64;
    for _ in 0..200 {
        let m = (xs(&mut seed) % 6) as usize;
        let cl: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let w = 1 + (xs(&mut seed) % 3) as usize;
                (0..w)
                    .map(|_| Lit::new((xs(&mut seed) % n as u64) as u32, xs(&mut seed) & 1 == 0))
                    .collect()
            })
            .collect();
        let part = fractional_automorphism(n, &cl);
        assert!(
            is_fractional_automorphism(n, &cl, &part),
            "canonical partition must always commute: {cl:?}"
        );
    }
}

#[test]
fn color_refinement_over_approximates_the_orbit_partition() {
    let p = |v: u32| Lit::new(v, true);

    // 1. Vertex-transitive symmetric families collapse to ONE cell (one orbit ⇒ one colour).
    let (php, _) = crate::families::php(4);
    assert_eq!(
        color_refinement_cells(php.num_vars, &php.clauses),
        1,
        "PHP(4) variables are all alike"
    );
    let (clq, _) = crate::families::clique_coloring(3, 3);
    assert_eq!(
        color_refinement_cells(clq.num_vars, &clq.clauses),
        1,
        "clique(3,3) variables are all alike"
    );

    // 2. A structurally-distinguished variable gets its own cell — a polynomial asymmetry certificate.
    let f = vec![vec![p(0)], vec![p(1), p(2)]]; // x0 in a unit, x1/x2 symmetric in a binary
    let cells = color_refinement(3, &f);
    assert_ne!(
        cells[0], cells[1],
        "the unit variable is distinguished from the binary pair"
    );
    assert_eq!(cells[1], cells[2], "x1 and x2 are interchangeable");
    assert_eq!(
        provably_asymmetric_variables(3, &f),
        vec![0],
        "x0 is provably fixed by every automorphism"
    );
    // …and the detected symmetry indeed fixes the singleton-cell variable.
    for g in crate::sym_break::variable_automorphism_generators(3, &f).unwrap_or_default() {
        assert_eq!(
            g[0], 0,
            "an automorphism must fix the provably-asymmetric variable"
        );
    }

    // 3. THE THEOREM, on every instance incl. a fuzz: orbit(v) ⊆ cell(v) (each orbit is monochromatic),
    //    and the equitable partition is never finer than the orbit partition (cells ≤ orbits).
    let check = |nv: usize, cl: &[Vec<Lit>]| {
        let gens = crate::sym_break::variable_automorphism_generators(nv, cl).unwrap_or_default();
        let orbits = crate::permgroup::orbits(nv, &gens);
        let cells = color_refinement(nv, cl);
        for orbit in &orbits {
            let c0 = cells[orbit[0]];
            assert!(
                orbit.iter().all(|&v| cells[v] == c0),
                "orbit {orbit:?} must be monochromatic: {cells:?}"
            );
        }
        assert!(
            color_refinement_cells(nv, cl) <= orbits.len(),
            "the equitable partition is coarser than the orbit partition"
        );
    };
    check(php.num_vars, &php.clauses);
    check(clq.num_vars, &clq.clauses);
    check(3, &f);

    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let nv = 5usize;
    let mut seed = 0xA5A5_1234_DEAD_BEEFu64;
    for _ in 0..200 {
        let m = (xs(&mut seed) % 6) as usize;
        let cl: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let w = 1 + (xs(&mut seed) % 3) as usize;
                (0..w)
                    .map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0))
                    .collect()
            })
            .collect();
        check(nv, &cl);
    }
}

#[test]
fn two_wl_over_approximates_orbitals_and_beats_one_wl() {
    let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)]; // an edge as a 2-clause

    // THE HEADLINE: a 6-cycle vs two triangles. Both are 2-regular, so 1-WL sees a single cell for each
    // and CANNOT tell them apart — but 2-WL distinguishes them by their pair structure.
    let c6 = vec![
        edge(0, 1),
        edge(1, 2),
        edge(2, 3),
        edge(3, 4),
        edge(4, 5),
        edge(5, 0),
    ];
    let two_tri = vec![
        edge(0, 1),
        edge(1, 2),
        edge(0, 2),
        edge(3, 4),
        edge(4, 5),
        edge(3, 5),
    ];
    assert_eq!(color_refinement_cells(6, &c6), 1, "1-WL: C₆ is one cell");
    assert_eq!(
        color_refinement_cells(6, &two_tri),
        1,
        "1-WL: 2·C₃ is one cell — same as C₆"
    );
    assert_ne!(
        two_wl_fingerprint(6, &c6),
        two_wl_fingerprint(6, &two_tri),
        "2-WL SEPARATES C₆ from 2·C₃ where 1-WL cannot"
    );

    // 2-WL refines 1-WL: the diagonal pair-coloring is at least as fine as the 1-WL vertex coloring.
    let diag_refines_1wl = |nv: usize, cl: &[Vec<Lit>]| {
        let wl1 = color_refinement(nv, cl);
        let pc = two_wl_pair_colors(nv, cl);
        for a in 0..nv {
            for b in 0..nv {
                if pc[a][a] == pc[b][b] {
                    assert_eq!(wl1[a], wl1[b], "2-WL diagonal must refine 1-WL");
                }
            }
        }
    };
    diag_refines_1wl(6, &c6);
    diag_refines_1wl(6, &two_tri);

    // THE THEOREM (the 2-orbit analogue of #42): orbital(i,j) ⊆ paircell(i,j) — every orbital is
    // monochromatic under 2-WL — and pair-cells are never finer than orbitals. Checked incl. a fuzz.
    let check = |nv: usize, cl: &[Vec<Lit>]| {
        let gens = crate::sym_break::variable_automorphism_generators(nv, cl).unwrap_or_default();
        let orbitals = crate::permgroup::orbitals(nv, &gens);
        let pc = two_wl_pair_colors(nv, cl);
        for orbital in &orbitals {
            let (i0, j0) = orbital[0];
            let c0 = pc[i0][j0];
            assert!(
                orbital.iter().all(|&(i, j)| pc[i][j] == c0),
                "orbital must be monochromatic"
            );
        }
        assert!(
            two_wl_pair_cells(nv, cl) <= orbitals.len(),
            "pair-cells ≤ orbitals"
        );
    };
    let (php, _) = crate::families::php(4);
    let (clq, _) = crate::families::clique_coloring(3, 3);
    check(php.num_vars, &php.clauses);
    check(clq.num_vars, &clq.clauses);
    check(6, &c6);
    check(6, &two_tri);
    // clique(3,3): 1-WL is one cell, but 2-WL recovers the rank-4 orbital structure 1-WL is blind to.
    assert_eq!(color_refinement_cells(clq.num_vars, &clq.clauses), 1);
    assert!(
        two_wl_pair_cells(clq.num_vars, &clq.clauses) >= 4,
        "2-WL sees the 4 orbitals of the grid"
    );

    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let nv = 5usize;
    let mut seed = 0x2B0C_1A7E_55AA_F00Du64;
    for _ in 0..120 {
        let m = (xs(&mut seed) % 6) as usize;
        let cl: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let w = 1 + (xs(&mut seed) % 3) as usize;
                (0..w)
                    .map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0))
                    .collect()
            })
            .collect();
        check(nv, &cl);
    }
}

#[test]
fn association_scheme_multiplicities_are_the_eigenspace_dimensions() {
    let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)];
    let (clq, _) = crate::families::clique_coloring(3, 3);
    let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();

    // clique(3,3) is the multiplicity-free S₃×S₃ action on 9 cells: its scheme multiplicities are the
    // degrees of the four constituent irreducibles — triv⊗triv=1, triv⊗std=2, std⊗triv=2, std⊗std=4.
    assert_eq!(
        association_scheme_multiplicities(clq.num_vars, &clq.clauses),
        Some(vec![1, 2, 2, 4]),
        "clique(3,3): eigenspace dimensions = S₃×S₃ constituent degrees"
    );

    // In general the multiplicities partition the space and include the trivial eigenspace (dim 1).
    for (nv, cl) in [(clq.num_vars, clq.clauses.clone()), (6, c6.clone())] {
        let m = association_scheme_multiplicities(nv, &cl)
            .expect("commutative scheme has multiplicities");
        assert_eq!(
            m.iter().sum::<u128>(),
            nv as u128,
            "Σ multiplicities = number of vertices"
        );
        assert!(m.iter().all(|&x| x >= 1), "each eigenspace is non-empty");
        assert_eq!(m[0], 1, "the smallest (trivial) eigenspace has dimension 1");
        // #multiplicities = #eigenspaces = scheme rank.
        assert_eq!(
            m.len(),
            coherent_rank(nv, &cl).unwrap(),
            "one multiplicity per eigenspace"
        );
    }
}

#[test]
fn association_scheme_eigenmatrix_is_the_scheme_character_table() {
    let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)];
    let (clq, _) = crate::families::clique_coloring(3, 3);
    let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();

    // clique(3,3) is the rook's-graph scheme (rank 4); C₆ is the cyclic scheme (rank 4). Both are
    // commutative, so each has an eigenmatrix P — the scheme's character table.
    assert_eq!(
        coherent_rank(clq.num_vars, &clq.clauses),
        Some(4),
        "clique(3,3): 4 relations"
    );

    for (nv, cl) in [(clq.num_vars, clq.clauses.clone()), (6, c6.clone())] {
        let d = coherent_rank(nv, &cl).unwrap();
        let (p, pm) = association_scheme_eigenmatrix(nv, &cl)
            .expect("a commutative scheme has an eigenmatrix");
        assert_eq!(pm.len(), d, "P has one row per common eigenspace (d × d)");
        assert!(pm.iter().all(|r| r.len() == d));

        // Each row is a 1-dimensional representation of the coherent algebra:
        // P[m][i]·P[m][j] = Σ_k a_{ijk}·P[m][k]  (re-verified against the intersection numbers).
        let (_, a) = coherent_configuration_constants(nv, &cl).unwrap();
        for row in &pm {
            for i in 0..d {
                for j in 0..d {
                    let lhs = row[i] as u128 * row[j] as u128 % p as u128;
                    let rhs = (0..d)
                        .map(|k| (a[i][j][k] % p as u128) * row[k] as u128 % p as u128)
                        .sum::<u128>()
                        % p as u128;
                    assert_eq!(lhs, rhs, "row must be an algebra homomorphism");
                }
            }
        }

        // The valencies (all-ones eigenvalue) partition the vertices and include the diagonal (valency 1),
        // and that valency vector is one of P's rows.
        let valency: Vec<u128> = (0..d).map(|i| (0..d).map(|j| a[i][j][0]).sum()).collect();
        assert_eq!(
            valency.iter().sum::<u128>(),
            nv as u128,
            "Σ valencies = number of vertices"
        );
        assert!(valency.contains(&1), "the diagonal relation has valency 1");
        assert!(
            pm.iter()
                .any(|row| (0..d).all(|i| row[i] as u128 == valency[i] % p as u128)),
            "the valency vector is a row of P (the trivial eigenspace)"
        );
    }
}

#[test]
fn coherent_configuration_is_a_genuine_association_scheme() {
    let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)];

    // The stabilized 2-WL coloring is a coherent configuration: its intersection numbers are
    // well-defined (the function returns Some only after checking EVERY pair), and they satisfy the
    // basic algebra identities. Verified on several structures + a fuzz.
    let (php, _) = crate::families::php(4);
    let (clq, _) = crate::families::clique_coloring(3, 3);
    let c6 = vec![
        edge(0, 1),
        edge(1, 2),
        edge(2, 3),
        edge(3, 4),
        edge(4, 5),
        edge(5, 0),
    ];

    let check = |nv: usize, cl: &[Vec<Lit>]| {
        let (d, p) = coherent_configuration_constants(nv, cl).expect("2-WL is coherent");
        assert_eq!(
            d,
            two_wl_pair_cells(nv, cl),
            "rank = number of basis relations"
        );
        // Σ_{i,j} p[i][j][k] = n for every relation k (every intermediate z lands in exactly one cell).
        for k in 0..d {
            let total: u128 = (0..d)
                .flat_map(|i| (0..d).map(move |j| (i, j)))
                .map(|(i, j)| p[i][j][k])
                .sum();
            assert_eq!(
                total, nv as u128,
                "Σ_ij p[i][j][k] counts all n intermediate points"
            );
        }
        // Transpose closure: the reverse of a basis relation is again a single basis relation.
        let pc = two_wl_pair_colors(nv, cl);
        let mut transpose = vec![usize::MAX; d];
        for i in 0..nv {
            for j in 0..nv {
                let (r, rt) = (pc[i][j], pc[j][i]);
                if transpose[r] == usize::MAX {
                    transpose[r] = rt;
                } else {
                    assert_eq!(transpose[r], rt, "R_r^T must be a single relation");
                }
            }
        }
        d
    };
    check(php.num_vars, &php.clauses);
    let dclq = check(clq.num_vars, &clq.clauses);
    check(6, &c6);

    // clique(3,3) is the 3×3 rook's graph: S₃×S₃ is transitive with rank 4, so the coherent
    // configuration has exactly 4 relations (diagonal, same-row, same-column, different-both) — and it
    // matches the group's orbital count (Schurian).
    assert_eq!(dclq, 4, "the rook's graph scheme has 4 relations");
    let clq_gens =
        crate::sym_break::variable_automorphism_generators(clq.num_vars, &clq.clauses).unwrap();
    assert_eq!(
        dclq,
        crate::permgroup::orbitals(clq.num_vars, &clq_gens).len(),
        "Schurian: relations = orbitals"
    );

    // A concrete intersection number on C₆ (vertices 0..5 in a cycle): the adjacency relation is the
    // colour of an edge pair (0,1) and the distance-2 relation that of (0,2). Two vertices at distance 2
    // share exactly ONE common neighbour, so p[adjacency][adjacency][distance-2] = 1.
    let pc6 = two_wl_pair_colors(6, &c6);
    let (_d6, p6) = coherent_configuration_constants(6, &c6).unwrap();
    let adj = pc6[0][1];
    let dist2 = pc6[0][2];
    assert_ne!(adj, dist2, "2-WL separates adjacency from distance-2 in C₆");
    assert_eq!(
        p6[adj][adj][dist2], 1,
        "C₆: vertices at distance 2 share exactly one common neighbour"
    );

    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let nv = 5usize;
    let mut seed = 0x0C0E_4E17_C0FF_EE00u64;
    for _ in 0..100 {
        let m = (xs(&mut seed) % 6) as usize;
        let cl: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let w = 1 + (xs(&mut seed) % 3) as usize;
                (0..w)
                    .map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0))
                    .collect()
            })
            .collect();
        // The key invariant: 2-WL always yields a coherent configuration (Some, not None).
        assert!(
            coherent_configuration_constants(nv, &cl).is_some(),
            "2-WL must always be coherent"
        );
    }
}

#[test]
fn three_wl_over_approximates_3_orbits_and_beats_two_wl() {
    let edge = |a: usize, b: usize| vec![Lit::new(a as u32, true), Lit::new(b as u32, true)];

    // Build a 6-regular Cayley graph on ℤ₄×ℤ₄ (16 vertices v = 4*r + c) from a connection set.
    let cayley = |conn: &[(i32, i32)]| -> Vec<Vec<Lit>> {
        let idx = |r: i32, c: i32| (((r.rem_euclid(4)) * 4 + c.rem_euclid(4)) as usize);
        let mut edges = std::collections::BTreeSet::new();
        for r in 0..4 {
            for c in 0..4 {
                for &(dr, dc) in conn {
                    let (a, b) = (idx(r, c), idx(r + dr, c + dc));
                    if a < b {
                        edges.insert((a, b));
                    }
                }
            }
        }
        edges.into_iter().map(|(a, b)| edge(a, b)).collect()
    };
    // The 4×4 rook's graph (= K₄□K₄): same row or same column. SRG(16,6,2,2).
    let rook = cayley(&[(1, 0), (2, 0), (3, 0), (0, 1), (0, 2), (0, 3)]);
    // The Shrikhande graph: connection set ±(1,0), ±(0,1), ±(1,1). Also SRG(16,6,2,2).
    let shrikhande = cayley(&[(1, 0), (3, 0), (0, 1), (0, 3), (1, 1), (3, 3)]);

    // THE HEADLINE: both are strongly regular with the SAME parameters, so 2-WL produces identical
    // colorings (same fingerprint) and CANNOT tell them apart — but 3-WL distinguishes them.
    assert_eq!(
        two_wl_fingerprint(16, &rook),
        two_wl_fingerprint(16, &shrikhande),
        "2-WL cannot separate two SRG(16,6,2,2) graphs"
    );
    assert_ne!(
        three_wl_fingerprint(16, &rook),
        three_wl_fingerprint(16, &shrikhande),
        "3-WL SEPARATES the rook's graph from the Shrikhande graph"
    );

    // THE THEOREM (the 3-orbit analogue of #42/#43): every 3-orbit is monochromatic under 3-WL.
    let check = |nv: usize, cl: &[Vec<Lit>]| {
        let gens = crate::sym_break::variable_automorphism_generators(nv, cl).unwrap_or_default();
        let tw = three_wl_colors(nv, cl);
        for orbit in crate::permgroup::orbits_on_tuples(nv, &gens, 3) {
            let t0 = &orbit[0];
            let c0 = tw[t0[0]][t0[1]][t0[2]];
            assert!(
                orbit.iter().all(|t| tw[t[0]][t[1]][t[2]] == c0),
                "3-orbit must be monochromatic"
            );
        }
    };
    let (clq, _) = crate::families::clique_coloring(3, 3);
    let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();
    check(clq.num_vars, &clq.clauses);
    check(6, &c6);

    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let nv = 5usize;
    let mut seed = 0x33C0_FFEE_0033_0033u64;
    for _ in 0..60 {
        let m = (xs(&mut seed) % 6) as usize;
        let cl: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let w = 1 + (xs(&mut seed) % 3) as usize;
                (0..w)
                    .map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0))
                    .collect()
            })
            .collect();
        check(nv, &cl);
    }
}

#[test]
fn canonical_form_decides_formula_isomorphism() {
    let edge = |a: usize, b: usize| vec![Lit::new(a as u32, true), Lit::new(b as u32, true)];
    let relabel = |cl: &[Vec<Lit>], perm: &[usize]| -> Vec<Vec<Lit>> {
        cl.iter()
            .map(|c| {
                c.iter()
                    .map(|l| Lit::new(perm[l.var() as usize] as u32, l.is_positive()))
                    .collect()
            })
            .collect()
    };

    // THE DEFINING PROPERTY: canonical form is an isomorphism invariant. Relabelling the variables by any
    // permutation leaves the canonical form unchanged.
    let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();
    let perm = [3usize, 5, 0, 2, 4, 1];
    assert_eq!(
        canonical_form(6, &c6),
        canonical_form(6, &relabel(&c6, &perm)),
        "isomorphic formulas share a canonical form"
    );
    assert_eq!(
        formulas_isomorphic(6, &c6, &relabel(&c6, &perm)),
        Some(true)
    );

    // COMPLETENESS: canonical form separates C₆ from two triangles — where 1-WL is blind (one cell each).
    let two_tri = vec![
        edge(0, 1),
        edge(1, 2),
        edge(0, 2),
        edge(3, 4),
        edge(4, 5),
        edge(3, 5),
    ];
    assert_eq!(
        color_refinement_cells(6, &c6),
        color_refinement_cells(6, &two_tri),
        "1-WL cannot tell them apart"
    );
    assert_eq!(
        formulas_isomorphic(6, &c6, &two_tri),
        Some(false),
        "but canonical form CAN"
    );
    assert_ne!(canonical_form(6, &c6), canonical_form(6, &two_tri));

    // A path and a star on 4 vertices are non-isomorphic trees; canonical form distinguishes them.
    let path4 = vec![edge(0, 1), edge(1, 2), edge(2, 3)];
    let star4 = vec![edge(0, 1), edge(0, 2), edge(0, 3)];
    assert_eq!(formulas_isomorphic(4, &path4, &star4), Some(false));

    // FUZZ: for random formulas and random permutations, F and π(F) ALWAYS share a canonical form, and a
    // structural change (dropping a clause) is detected as non-isomorphic when it truly changes the form.
    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let nv = 6usize;
    let mut seed = 0xCA90_F00D_1234_5678u64;
    for _ in 0..120 {
        let m = (xs(&mut seed) % 7) as usize;
        let f: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let w = 1 + (xs(&mut seed) % 2) as usize;
                (0..w)
                    .map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0))
                    .collect()
            })
            .collect();
        // A random permutation via Fisher–Yates.
        let mut perm: Vec<usize> = (0..nv).collect();
        for i in (1..nv).rev() {
            let j = (xs(&mut seed) % (i as u64 + 1)) as usize;
            perm.swap(i, j);
        }
        let fp = relabel(&f, &perm);
        assert_eq!(
            canonical_form(nv, &f),
            canonical_form(nv, &fp),
            "F ≅ π(F): F={f:?} perm={perm:?}"
        );
        assert_eq!(formulas_isomorphic(nv, &f, &fp), Some(true));
    }
}

#[test]
fn weighted_model_count_is_exact_and_symmetry_accelerated() {
    let p = |v: u32| Lit::new(v, true);
    let nl = |v: u32| Lit::new(v, false);
    let sat = |m: &[bool], cls: &[Vec<Lit>]| {
        cls.iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
    };
    let brute = |nv: usize, f: &[Vec<Lit>], w: &[(i64, i64)]| -> i128 {
        (0u32..(1u32 << nv))
            .filter_map(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                sat(&a, f).then(|| {
                    (0..nv)
                        .map(|i| if a[i] { w[i].1 as i128 } else { w[i].0 as i128 })
                        .product::<i128>()
                })
            })
            .sum()
    };

    // Unit weights ⇒ the weighted count is just the model count.
    let amo = vec![vec![p(0), p(1), p(2)]]; // at-least-one-of-3: 7 models
    let ones = vec![(1i64, 1i64); 3];
    assert_eq!(
        weighted_model_count(3, &amo, &ones),
        7,
        "unit weights ⇒ #models"
    );
    assert_eq!(weighted_model_count(3, &amo, &ones), brute(3, &amo, &ones));

    // THE ACCELERATION: a fully S₃-symmetric instance fuses its models into few orbits, so the count
    // takes far fewer solves than there are models.
    let (solves, models) = weighted_model_count_solve_counts(3, &amo);
    assert_eq!(models, 7, "7 models of at-least-one-of-3");
    assert!(
        solves < models,
        "symmetry must reduce the solves: {solves} < {models}"
    );

    // Weighted, symmetric weights: Z must match brute force exactly.
    let w_sym = vec![(2i64, 3i64); 3];
    assert_eq!(
        weighted_model_count(3, &amo, &w_sym),
        brute(3, &amo, &w_sym)
    );

    // UNSAT formula ⇒ empty product sum = 0.
    assert_eq!(
        weighted_model_count(2, &[vec![p(0)], vec![nl(0)]], &[(1, 1), (1, 1)]),
        0
    );

    // FUZZ: arbitrary formulas and arbitrary (asymmetric) weights — the symmetry-accelerated count is
    // ALWAYS the exact weighted model count (every model's true weight is summed; symmetry only groups
    // the search).
    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    let nv = 4usize;
    let mut seed = 0x5EED_4321_FACE_0001u64;
    for _ in 0..200 {
        let m = (xs(&mut seed) % 5) as usize;
        let f: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let wd = 1 + (xs(&mut seed) % 2) as usize;
                (0..wd)
                    .map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0))
                    .collect()
            })
            .collect();
        let w: Vec<(i64, i64)> = (0..nv)
            .map(|_| ((xs(&mut seed) % 4) as i64, (xs(&mut seed) % 4) as i64))
            .collect();
        assert_eq!(
            weighted_model_count(nv, &f, &w),
            brute(nv, &f, &w),
            "F={f:?} w={w:?}"
        );
    }
}

#[test]
fn assignment_weight_inventory_splits_orbits_by_weight() {
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let inv = assignment_weight_inventory(cnf.num_vars, &cnf.clauses).expect("enumerable group");
    assert_eq!(inv.len(), 10, "weights 0..=9");
    assert_eq!(
        inv.iter().sum::<u128>(),
        36,
        "sums to the assignment-orbit count (36 binary 3×3 matrices)"
    );
    assert_eq!(inv[0], 1, "one all-false assignment");
    assert_eq!(
        inv[1], 1,
        "all single-one matrices are row/column equivalent"
    );
    assert_eq!(inv[9], 1, "one all-true assignment");
    // The sum agrees with the profile's scalar assignment_orbits.
    assert_eq!(
        Some(inv.iter().sum::<u128>()),
        symmetry_structure(cnf.num_vars, &cnf.clauses).assignment_orbits,
        "the inventory sums to the profile's assignment_orbits"
    );

    // No symmetry ⇒ the binomial distribution (every assignment is its own orbit).
    let asym = vec![
        vec![Lit::new(0, true)],
        vec![Lit::new(1, false), Lit::new(2, true)],
    ];
    assert_eq!(
        assignment_weight_inventory(3, &asym),
        Some(vec![1, 3, 3, 1]),
        "no symmetry ⇒ C(3,w)"
    );
}

#[test]
fn pb_coefficient_symmetry_profiles_through_the_same_ladder() {
    use crate::pseudo_boolean::PbConstraint;
    // 3·x0 + 3·x1 + 3·x2 + 5·x3 ≥ 6: {x0,x1,x2} share coefficient 3 (interchangeable, S₃), x3 (coeff 5)
    // is alone. The coefficient-symmetry group is S₃ on {0,1,2}.
    let c =
        PbConstraint::new_weighted(&[(0, 3, true), (1, 3, true), (2, 3, true), (3, 5, true)], 6);
    let prof = pb_symmetry_profile(4, &[c]);
    assert_eq!(prof.order, 6, "S₃ on the three weight-3 variables");
    assert_eq!(
        prof.num_orbits, 2,
        "two orbits: {{x0,x1,x2}} and the fixed point x3"
    );
    assert!(!prof.abelian, "S₃ is non-abelian");
    assert_eq!(prof.solvable, Some(true), "S₃ is solvable");
    assert_eq!(
        prof.coherent_rank, None,
        "the coefficient profile has no clauses ⇒ no scheme rank"
    );

    // All-distinct coefficients ⇒ no coefficient symmetry ⇒ the trivial profile.
    let distinct = PbConstraint::new_weighted(&[(0, 1, true), (1, 2, true), (2, 3, true)], 3);
    assert_eq!(
        pb_symmetry_profile(3, &[distinct]).order,
        1,
        "distinct weights ⇒ trivial group"
    );
}

#[test]
fn symmetry_structure_profiles_the_variable_group() {
    let p = |v| Lit::new(v, true);
    let n = |v| Lit::new(v, false);

    // exactly-one-of-3: the variable group is S₃ on 3 points — sharply 3-transitive, primitive, rank 2.
    let exactly1 = vec![
        vec![p(0), p(1), p(2)],
        vec![n(0), n(1)],
        vec![n(0), n(2)],
        vec![n(1), n(2)],
    ];
    let prof = symmetry_structure(3, &exactly1);
    assert_eq!(prof.order, 6, "|S₃| = 6");
    assert_eq!(prof.num_orbits, 1, "transitive on the 3 cells");
    assert_eq!(prof.rank, 2, "S₃ is 2-transitive ⇒ rank 2");
    assert_eq!(
        prof.coherent_rank,
        Some(2),
        "coherent (scheme) rank matches the orbital rank here"
    );
    assert!(
        prof.coherent_rank.unwrap() <= prof.rank,
        "coherent rank ≤ orbital rank"
    );
    assert_eq!(prof.transitivity, 3, "S₃ is 3-transitive on 3 points");
    assert!(prof.primitive, "S₃ on 3 points is primitive");
    assert_eq!(prof.blocks, None, "a primitive group has no block system");
    assert!(!prof.abelian, "S₃ is non-abelian");
    assert_eq!(prof.solvable, Some(true), "S₃ is solvable");
    assert_eq!(
        prof.nilpotent,
        Some(false),
        "S₃ is solvable but NOT nilpotent"
    );
    assert_eq!(prof.derived_length, Some(2), "S₃ has derived length 2");
    assert_eq!(
        prof.nilpotency_class, None,
        "S₃ is not nilpotent ⇒ no nilpotency class"
    );
    assert_eq!(prof.derived_order, 3, "[S₃,S₃] = A₃ has order 3");
    assert_eq!(
        prof.conjugacy_classes,
        Some(3),
        "S₃ has 3 conjugacy classes (= 3 irreps)"
    );
    assert_eq!(prof.center_order, Some(1), "S₃ has a trivial centre");
    assert_eq!(
        prof.exponent,
        Some(6),
        "S₃ has exponent 6 (lcm of orders 1,2,3)"
    );
    assert_eq!(prof.subgroups, Some(6), "S₃ has 6 subgroups");
    assert_eq!(prof.simple, Some(false), "S₃ is not simple (A₃ is normal)");
    assert_eq!(prof.composition_factors, Some(vec![2, 3]), "S₃ = C₂, C₃");
    assert_eq!(
        prof.sylow,
        Some(vec![(2, 3), (3, 1)]),
        "S₃: 3 Sylow-2, 1 Sylow-3"
    );
    assert_eq!(
        prof.real_classes,
        Some(3),
        "S₃: all 3 classes (= irreps) are real"
    );
    assert_eq!(
        prof.rational_classes,
        Some(3),
        "S₃ is rational: all 3 classes rational"
    );
    assert_eq!(
        prof.automorphism_order,
        Some(6),
        "|Aut(S₃)| = 6 (S₃ is complete)"
    );
    assert_eq!(prof.outer_automorphism_order, Some(1), "Out(S₃) = 1");
    assert_eq!(
        prof.irreducible_degrees,
        Some(vec![1, 1, 2]),
        "S₃ irreps: trivial, sign, 2-dim standard"
    );
    assert_eq!(
        prof.frobenius_schur,
        Some(vec![1, 1, 1]),
        "S₃ is totally real ⇒ all indicators +1"
    );
    {
        // The permutation rep of S₃ on its 3 variables = trivial ⊕ standard: Σ m·d = 3, Σ m² = rank,
        // and the trivial multiplicity is the orbit count (Burnside).
        let iso = prof
            .isotypic_multiplicities
            .as_ref()
            .expect("S₃ isotypic decomposition");
        let degs = prof.irreducible_degrees.as_ref().unwrap();
        assert_eq!(
            iso.iter().zip(degs).map(|(m, d)| m * d).sum::<u128>(),
            3,
            "Σ m·d = 3 variables"
        );
        assert_eq!(
            iso.iter().map(|m| m * m).sum::<u128>(),
            prof.rank as u128,
            "Σ m² = rank"
        );
    }

    // clique_coloring(3,3): the variable group is S₃(vertices) × S₃(colours) on the 3×3 grid — order 36,
    // transitive but only 1-transitive, IMPRIMITIVE (rows/cols are blocks), rank 4 (diagonal + same-row
    // + same-col + different-both).
    let (clique, _) = crate::families::clique_coloring(3, 3);
    let cp = symmetry_structure(clique.num_vars, &clique.clauses);
    assert_eq!(cp.order, 36, "|S₃ × S₃| = 36");
    assert_eq!(cp.num_orbits, 1, "transitive on the 9 cells");
    assert_eq!(cp.rank, 4, "the grid action has rank 4");
    assert_eq!(
        cp.coherent_rank,
        Some(4),
        "clique(3,3): the coherent scheme has 4 relations"
    );
    assert_eq!(cp.transitivity, 1, "transitive but not 2-transitive");
    assert!(!cp.primitive, "the grid action is imprimitive");
    assert!(cp.blocks.is_some(), "rows/columns form a block system");
    assert!(!cp.abelian, "S₃ × S₃ is non-abelian");
    assert_eq!(cp.solvable, Some(true), "S₃ × S₃ is solvable");
    assert_eq!(
        cp.nilpotent,
        Some(false),
        "S₃ × S₃ is not nilpotent (S₃ isn't)"
    );
    assert_eq!(cp.derived_order, 9, "[S₃×S₃, S₃×S₃] = A₃ × A₃ has order 9");
    assert_eq!(
        cp.conjugacy_classes,
        Some(9),
        "S₃×S₃ has 3·3 = 9 conjugacy classes"
    );
    assert_eq!(cp.center_order, Some(1), "S₃×S₃ has a trivial centre");
    assert_eq!(
        cp.rational_classes,
        Some(9),
        "S₃×S₃ is rational: all 9 classes rational"
    );
    // Aut(S₃×S₃) = Aut(S₃) ≀ C₂ = (S₃×S₃)⋊C₂, order 72; Inn = 36 (trivial centre) ⇒ Out = C₂.
    assert_eq!(cp.automorphism_order, Some(72), "|Aut(S₃×S₃)| = 72");
    assert_eq!(
        cp.outer_automorphism_order,
        Some(2),
        "Out(S₃×S₃) = C₂ (factor swap)"
    );
    assert_eq!(
        cp.assignment_orbits,
        Some(36),
        "2⁹ assignments up to S₃×S₃ = 36 binary 3×3 matrices"
    );
    assert_eq!(
        cp.abelianization,
        Some((4, 2)),
        "(S₃×S₃)ᵃᵇ = C₂×C₂ (order 4, exponent 2)"
    );
    assert!(
        cp.subgroups.is_some(),
        "the S₃×S₃ subgroup lattice is computed"
    );
    assert_eq!(cp.simple, Some(false), "S₃×S₃ is not simple");
    assert_eq!(
        cp.composition_factors,
        Some(vec![2, 2, 3, 3]),
        "S₃×S₃ = C₂², C₃² (product 36)"
    );
    assert_eq!(
        cp.sylow,
        Some(vec![(2, 9), (3, 1)]),
        "S₃×S₃: 3² = 9 Sylow-2, 1 Sylow-3"
    );
    // Irreducibles of a direct product are the pairwise products of the factors' irreducibles:
    // {1,1,2} ⊗ {1,1,2} = {1,1,1,1,2,2,2,2,4}, and Σ dᵢ² = 4·1 + 4·4 + 16 = 36 = |G|.
    assert_eq!(
        cp.irreducible_degrees,
        Some(vec![1, 1, 1, 1, 2, 2, 2, 2, 4]),
        "S₃×S₃ irreps = products of the S₃ irreps"
    );
    assert_eq!(
        cp.frobenius_schur,
        Some(vec![1, 1, 1, 1, 1, 1, 1, 1, 1]),
        "S₃×S₃ is totally real ⇒ all nine indicators +1"
    );
    {
        // The permutation rep on the 9 grid cells decomposes with Σ m·d = 9 and ⟨π,π⟩ = rank = 4.
        let iso = cp
            .isotypic_multiplicities
            .as_ref()
            .expect("S₃×S₃ isotypic decomposition");
        let degs = cp.irreducible_degrees.as_ref().unwrap();
        assert_eq!(
            iso.iter().zip(degs).map(|(m, d)| m * d).sum::<u128>(),
            9,
            "Σ m·d = 9 cells"
        );
        assert_eq!(
            iso.iter().map(|m| m * m).sum::<u128>(),
            cp.rank as u128,
            "⟨π,π⟩ = rank = 4"
        );
    }

    // No symmetry → the trivial profile.
    let asym = vec![vec![p(0)], vec![n(1), p(2)]];
    let ap = symmetry_structure(3, &asym);
    assert_eq!(ap.order, 1, "no symmetry ⇒ trivial group");
    assert!(
        ap.coherent_rank.is_some(),
        "the scheme rank is always computed from the clauses"
    );
    assert_eq!(
        ap.irreducible_degrees,
        Some(vec![1]),
        "trivial group: one trivial irreducible"
    );
    assert_eq!(
        ap.frobenius_schur,
        Some(vec![1]),
        "trivial group: its character is real"
    );
    assert_eq!(
        ap.isotypic_multiplicities,
        Some(vec![3]),
        "trivial group on 3 vars: the perm rep is 3 copies of the trivial irreducible"
    );
    assert_eq!(
        ap.rational_classes,
        Some(1),
        "trivial group: its one class is rational"
    );
    assert_eq!(
        ap.automorphism_order,
        Some(1),
        "trivial group: Aut is trivial"
    );
    assert_eq!(
        ap.outer_automorphism_order,
        Some(1),
        "trivial group: Out is trivial"
    );
}

#[test]
fn models_up_to_symmetry_enumerates_orbits_and_counts_exactly() {
    let p = |v| Lit::new(v, true);
    let n = |v| Lit::new(v, false);

    // Independent oracle: (#models, #orbits) under the variable-symmetry generators, by brute force.
    let oracle = |nv: usize, cl: &[Vec<Lit>]| -> (u128, usize) {
        let models: Vec<Vec<bool>> = (0u64..(1u64 << nv))
            .filter_map(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cl.iter()
                    .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
                    .then_some(a)
            })
            .collect();
        let gens = crate::sym_break::variable_automorphism_generators(nv, cl).unwrap_or_default();
        let model_set: std::collections::HashSet<Vec<bool>> = models.iter().cloned().collect();
        let mut seen = std::collections::HashSet::new();
        let mut orbits = 0;
        for m in &models {
            if seen.contains(m) {
                continue;
            }
            orbits += 1;
            let mut stack = vec![m.clone()];
            while let Some(cur) = stack.pop() {
                if !seen.insert(cur.clone()) {
                    continue;
                }
                for g in &gens {
                    let mut pm = vec![false; nv];
                    for v in 0..nv {
                        pm[g[v]] = cur[v];
                    }
                    if model_set.contains(&pm) && !seen.contains(&pm) {
                        stack.push(pm);
                    }
                }
            }
        }
        (models.len() as u128, orbits)
    };

    // exactly-one-of-3 (one-hot): 3 models, all one orbit under S₃.
    let exactly1 = vec![
        vec![p(0), p(1), p(2)],
        vec![n(0), n(1)],
        vec![n(0), n(2)],
        vec![n(1), n(2)],
    ];
    // at-most-one-of-3: 4 models {000, 100, 010, 001} → 2 orbits ({000}, the three weight-1).
    let atmost1 = vec![vec![n(0), n(1)], vec![n(0), n(2)], vec![n(1), n(2)]];

    for cl in [&exactly1, &atmost1] {
        let (m_exact, m_orbits) = oracle(3, cl);
        let sc = models_up_to_symmetry(3, cl, 1000);
        assert!(
            sc.exhaustive,
            "the small instance is enumerated to exhaustion"
        );
        assert_eq!(
            sc.total_models, m_exact,
            "exact model count = sum of orbit sizes"
        );
        assert_eq!(
            sc.representatives.len(),
            m_orbits,
            "one representative per orbit"
        );
        // Cross-check the orbit count against Burnside — two independent routes must agree.
        if let Some(burnside) = crate::sym_break::count_models_modulo_symmetry(3, cl) {
            assert_eq!(
                sc.representatives.len(),
                burnside,
                "enumeration agrees with Burnside"
            );
        }
        // Every representative is a genuine model, and they lie in distinct orbits (all distinct).
        for r in &sc.representatives {
            assert!(
                cl.iter()
                    .all(|c| c.iter().any(|l| r[l.var() as usize] == l.is_positive())),
                "valid model"
            );
        }
        let distinct: std::collections::HashSet<&Vec<bool>> = sc.representatives.iter().collect();
        assert_eq!(
            distinct.len(),
            sc.representatives.len(),
            "representatives are distinct"
        );
    }

    // Specific counts, to pin the numbers.
    assert_eq!(models_up_to_symmetry(3, &exactly1, 1000).total_models, 3);
    assert_eq!(
        models_up_to_symmetry(3, &exactly1, 1000)
            .representatives
            .len(),
        1
    );
    assert_eq!(models_up_to_symmetry(3, &atmost1, 1000).total_models, 4);
    assert_eq!(
        models_up_to_symmetry(3, &atmost1, 1000)
            .representatives
            .len(),
        2
    );
}

#[test]
fn declared_symmetry_is_verified_then_broken() {
    let p = |v| Lit::new(v, true);
    let brute = |cl: &[Vec<Lit>], nv: usize| {
        (0u64..(1u64 << nv)).any(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            cl.iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        })
    };

    // clique_coloring(3,3), x[v][c] = 3v+c. A declared vertex swap 0↔1 is a genuine symmetry — verified
    // and used.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let nv = cnf.num_vars;
    let mut vswap: Vec<usize> = (0..nv).collect();
    for c in 0..3 {
        vswap.swap(c, 3 + c);
    }
    assert!(
        is_declared_symmetry(nv, &cnf.clauses, &vswap),
        "a vertex swap is a genuine clique symmetry"
    );
    let s = solve_with_declared_symmetry(nv, &cnf.clauses, &[vswap]);
    assert_eq!(s.via, Route::DeclaredSymmetry);
    match &s.answer {
        Answer::Sat(m) => assert!(cnf
            .clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))),
        Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
    }

    // A BOGUS declared generator (a valid permutation but NOT a symmetry) is rejected — the verdict is
    // unaffected (a wrong declaration cannot corrupt the result).
    let mut bogus: Vec<usize> = (0..nv).collect();
    bogus.swap(0, 5); // x[0][0] ↔ x[1][2] is not a system symmetry
    assert!(
        !is_declared_symmetry(nv, &cnf.clauses, &bogus),
        "a non-symmetry must be rejected"
    );
    let wb = solve_with_declared_symmetry(nv, &cnf.clauses, &[bogus]);
    assert!(
        matches!(wb.answer, Answer::Sat(_)),
        "a bogus declaration must not corrupt the SAT verdict"
    );
    if let Answer::Sat(m) = &wb.answer {
        assert!(cnf
            .clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())));
    }

    // A SEMANTIC declared symmetry (not syntactic): a↔b in (a∨x)(b∨x)(a∨x∨y). Accepted via implication.
    let f = vec![vec![p(0), p(2)], vec![p(1), p(2)], vec![p(0), p(2), p(3)]];
    let mut ab: Vec<usize> = (0..4).collect();
    ab.swap(0, 1);
    assert!(
        is_declared_symmetry(4, &f, &ab),
        "a↔b is a semantic symmetry, verified by implication"
    );
    let sem = solve_with_declared_symmetry(4, &f, &[ab]);
    assert_eq!(sem.via, Route::DeclaredSymmetry);
    assert_eq!(
        matches!(sem.answer, Answer::Sat(_)),
        brute(&f, 4),
        "declared-symmetry verdict matches brute force"
    );

    // A malformed generator (wrong length) is rejected without panicking; the verdict is still correct.
    assert!(
        !is_declared_symmetry(4, &f, &[0usize, 1]),
        "a wrong-length permutation is rejected"
    );
    let mf = solve_with_declared_symmetry(4, &f, &[vec![0usize, 1]]);
    assert_eq!(
        matches!(mf.answer, Answer::Sat(_)),
        matches!(solve_comprehensive(4, &f).answer, Answer::Sat(_)),
        "a malformed declaration is dropped, verdict unchanged"
    );
}

#[test]
fn almost_symmetry_breaks_a_near_miss_conditionally() {
    let p = |v| Lit::new(v, true);
    let brute = |cl: &[Vec<Lit>], nv: usize| {
        (0u64..(1u64 << nv)).any(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            cl.iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        })
    };
    // a=0,b=1,x=2,y=3. (a∨x)(b∨x) is symmetric in a,b; the extra (a∨y) breaks it, and its swap (b∨y)
    // is NOT implied — so a↔b is an ALMOST-symmetry (one broken clause), not a semantic one.
    let f = vec![vec![p(0), p(2)], vec![p(1), p(2)], vec![p(0), p(3)]];

    // It is NOT a semantic symmetry (the broken image is not implied).
    let (sem, _) = semantic_symmetry_pairs(4, &f);
    assert!(
        !sem.contains(&(0, 1)),
        "a,b is not a SEMANTIC symmetry here: {sem:?}"
    );

    // It IS an almost-symmetry: a↔b breaks exactly one clause.
    let almost = almost_symmetry_pairs(4, &f, 2);
    assert!(
        almost
            .iter()
            .any(|(a, b, imgs)| *a == 0 && *b == 1 && imgs.len() == 1),
        "a↔b breaks exactly one clause: {almost:?}"
    );

    // Solved correctly — the conditional (guarded) break is sound.
    let s = almost_symmetry_solve(4, &f)
        .expect("an almost-symmetry is detected and conditionally broken");
    assert_eq!(s.via, Route::AlmostSymmetry);
    assert_eq!(
        matches!(s.answer, Answer::Sat(_)),
        brute(&f, 4),
        "almost-symmetry verdict matches brute force"
    );
    if let Answer::Sat(m) = &s.answer {
        assert!(
            f.iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "valid model"
        );
    }

    // UNSAT path: force every variable off — (a∨x) cannot hold. The conditional break preserves UNSAT.
    let mut un = f.clone();
    un.extend([
        vec![Lit::new(0, false)],
        vec![Lit::new(1, false)],
        vec![Lit::new(2, false)],
        vec![Lit::new(3, false)],
    ]);
    let u = almost_symmetry_solve(4, &un).expect("almost-symmetry still detected");
    assert_eq!(u.via, Route::AlmostSymmetry);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "all variables off makes (a∨x) UNSAT"
    );
    assert_eq!(
        matches!(u.answer, Answer::Sat(_)),
        brute(&un, 4),
        "UNSAT verdict matches brute force"
    );
}

#[test]
fn semantic_symmetry_breaks_a_non_syntactic_interchange() {
    let p = |v| Lit::new(v, true);
    // a=0, b=1, x=2, y=3. (a∨x) ∧ (b∨x) ∧ (a∨x∨y): the third clause is IMPLIED by (a∨x), so it is
    // redundant — F's models are symmetric in a,b. But its swap (b∨x∨y) is not a clause, so swapping
    // a,b does NOT preserve the clause set: a,b are SEMANTICALLY but not SYNTACTICALLY interchangeable.
    let f = vec![vec![p(0), p(2)], vec![p(1), p(2)], vec![p(0), p(2), p(3)]];

    // The semantic detector finds (a,b), flagged non-syntactic.
    let (pairs, non_syntactic) = semantic_symmetry_pairs(4, &f);
    assert!(
        pairs.contains(&(0, 1)) && non_syntactic,
        "a,b are a semantic, non-syntactic symmetry: {pairs:?}"
    );

    // Confirm it really is non-syntactic: swapping a,b changes the clause set.
    let canon_set = |swap: bool| -> std::collections::HashSet<Vec<(u32, bool)>> {
        f.iter()
            .map(|c| {
                let mut k: Vec<(u32, bool)> = c
                    .iter()
                    .map(|l| {
                        let v = l.var() as usize;
                        let nv = if swap && v == 0 {
                            1
                        } else if swap && v == 1 {
                            0
                        } else {
                            v
                        };
                        (nv as u32, l.is_positive())
                    })
                    .collect();
                k.sort_unstable();
                k
            })
            .collect()
    };
    assert_ne!(
        canon_set(true),
        canon_set(false),
        "the a↔b swap changes the clause set — not syntactic"
    );

    // It is broken and solved correctly.
    let s = semantic_symmetry_solve(4, &f).expect("a semantic symmetry is detected and broken");
    assert_eq!(s.via, Route::SemanticSymmetry);
    let brute = (0u64..16).any(|x| {
        let a: Vec<bool> = (0..4).map(|i| (x >> i) & 1 == 1).collect();
        f.iter()
            .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
    });
    assert_eq!(
        matches!(s.answer, Answer::Sat(_)),
        brute,
        "semantic-symmetry verdict matches brute force"
    );
    if let Answer::Sat(m) = &s.answer {
        assert!(
            f.iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "valid model"
        );
    }

    // Declines when the only symmetry is already syntactic: (a∨x)∧(b∨x), a↔b preserves the clause set,
    // so symmetry_break_solve already covers it and the semantic route bows out.
    let syntactic = vec![vec![p(0), p(2)], vec![p(1), p(2)]];
    assert!(
        semantic_symmetry_solve(3, &syntactic).is_none(),
        "a purely syntactic symmetry is left to the syntactic route"
    );
}

#[test]
fn nested_block_tower_breaks_multidimensional_symmetry() {
    let p = |v| Lit::new(v, true);
    let brute = |nv: usize, cl: &[Vec<Lit>]| -> bool {
        (0u64..(1u64 << nv)).any(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            cl.iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        })
    };

    // One block level (2-D), SAT: clique_coloring(3,3) — vertex × colour, phase-free.
    let (sat, _) = crate::families::clique_coloring(3, 3);
    let s = nested_symmetry_solve(sat.num_vars, &sat.clauses)
        .expect("a grid symmetry has a block system");
    assert_eq!(s.via, Route::NestedSymmetry);
    match &s.answer {
        Answer::Sat(m) => assert!(sat
            .clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))),
        Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
    }
    assert_eq!(
        matches!(s.answer, Answer::Sat(_)),
        brute(sat.num_vars, &sat.clauses),
        "verdict matches brute"
    );

    // One block level, UNSAT: clique_coloring(4,3) — K₄ is not 3-colourable.
    let (unsat, _) = crate::families::clique_coloring(4, 3);
    let u = nested_symmetry_solve(unsat.num_vars, &unsat.clauses)
        .expect("a grid symmetry has a block system");
    assert_eq!(u.via, Route::NestedSymmetry);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "clique_coloring(4,3) is UNSAT"
    );
    assert_eq!(
        matches!(u.answer, Answer::Sat(_)),
        brute(unsat.num_vars, &unsat.clauses),
        "verdict matches brute"
    );

    // MULTI-LEVEL TOWER: the cube graph Q₃ (8 vertices, "cover every edge" = xᵤ ∨ xᵥ per edge). It is
    // vertex-transitive, phase-free, and imprimitive with NESTED blocks (antipodal pairs ⊂ … ⊂ whole),
    // so the tower ascends past the single block system a 2-D break would stop at. SAT, broken soundly.
    let mut cube: Vec<Vec<Lit>> = Vec::new();
    for v in 0u32..8 {
        for b in 0..3 {
            let w = v ^ (1 << b);
            if v < w {
                cube.push(vec![p(v), p(w)]);
            }
        }
    }
    let c = nested_symmetry_solve(8, &cube).expect("the cube's nested block tower engages");
    assert_eq!(c.via, Route::NestedSymmetry);
    match &c.answer {
        Answer::Sat(m) => assert!(cube
            .iter()
            .all(|cl| cl.iter().any(|l| m[l.var() as usize] == l.is_positive()))),
        Answer::Unsat => panic!("covering every cube edge is SAT (e.g. all true)"),
    }
    assert_eq!(
        matches!(c.answer, Answer::Sat(_)),
        brute(8, &cube),
        "the nested-tower verdict matches brute force"
    );
}

#[test]
fn symmetry_revealed_by_simplification_is_unlocked_and_broken() {
    let p = |v| Lit::new(v, true);
    let n = |v| Lit::new(v, false);
    // Raw F lacks the x2↔x3 symmetry (the ¬x0 tag sits only on x2's clause); propagating x0=T strips
    // the tag, revealing (x1∨x2)∧(x1∨x3) — symmetric in x2,x3. The route detects the unlocked
    // symmetry, solves the residual with the arsenal, and re-applies x0=T.
    let unlock = vec![vec![p(0)], vec![p(1), p(2), n(0)], vec![p(1), p(3)]];
    let s = symmetry_via_simplification_solve(4, &unlock).expect("simplification unlocks symmetry");
    assert_eq!(s.via, Route::SymmetrySimplify);
    match &s.answer {
        Answer::Sat(m) => {
            assert!(unlock
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())));
            assert!(m[0], "the forced literal x0 is re-applied to the model");
        }
        Answer::Unsat => panic!("the instance is SAT"),
    }
    let brute = (0u64..16).any(|x| {
        let a: Vec<bool> = (0..4).map(|i| (x >> i) & 1 == 1).collect();
        unlock
            .iter()
            .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
    });
    assert_eq!(
        matches!(s.answer, Answer::Sat(_)),
        brute,
        "verdict matches brute force"
    );

    // Declines when the symmetry is already visible on the raw formula (no unit hides it).
    let already = vec![vec![p(0)], vec![p(1), p(2)], vec![p(1), p(3)]];
    assert!(
        symmetry_via_simplification_solve(4, &already).is_none(),
        "symmetry already present on the raw formula ⟹ nothing unlocked"
    );

    // A unit-propagation conflict is reported as UNSAT.
    let conflict = vec![vec![p(0)], vec![p(1)], vec![n(0), n(1)]];
    let u = symmetry_via_simplification_solve(2, &conflict).expect("BCP reaches a conflict");
    assert_eq!(u.via, Route::SymmetrySimplify);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "x0 ∧ x1 ∧ ¬(x0∧x1) is UNSAT"
    );
}

#[test]
fn symmetric_binary_inference_learns_implication_orbits() {
    let p = |v| Lit::new(v, true);
    let n = |v| Lit::new(v, false);
    // Chain xᵢ → a → y with x0,x1,x2 interchangeable (S₃; a=var3, y=var4). BCP under x0 derives the
    // TRANSITIVE x0→y (not a clause), and symmetry adds x1→y and x2→y from that single probe.
    let chain = vec![
        vec![n(0), p(3)],
        vec![n(1), p(3)],
        vec![n(2), p(3)], // xᵢ → a
        vec![n(3), p(4)], // a → y
    ];
    let s = symmetric_binary_inference_solve(5, &chain)
        .expect("a derived implication orbit is learned");
    assert_eq!(s.via, Route::SymmetricBinary);
    match &s.answer {
        Answer::Sat(m) => assert!(
            chain
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the strengthened formula yields a valid model"
        ),
        Answer::Unsat => panic!("the chain is SAT"),
    }
    // Soundness vs brute force.
    let brute = (0u64..32).any(|x| {
        let a: Vec<bool> = (0..5).map(|i| (x >> i) & 1 == 1).collect();
        chain
            .iter()
            .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
    });
    assert_eq!(
        matches!(s.answer, Answer::Sat(_)),
        brute,
        "binary-inference verdict matches brute force"
    );

    // Declines when BCP yields no NEW binary: here xᵢ→y are already clauses, so there is nothing to
    // learn and the route bows out (the cheap routes / CDCL decide).
    let direct = vec![vec![n(0), p(2)], vec![n(1), p(2)]]; // x0→y, x1→y directly (y=var2), S₂
    assert!(
        symmetric_binary_inference_solve(3, &direct).is_none(),
        "no new implication to learn ⟹ declines"
    );
}

#[test]
fn symmetric_component_decomposition_solves_copies_once() {
    let p = |v| Lit::new(v, true);
    let n = |v| Lit::new(v, false);
    // Two interchangeable copies of (x∨y): components {0,1} and {2,3}, swapped by (0↔2, 1↔3). SAT —
    // the representative is solved once and its model is replicated through the symmetry.
    let sat = vec![vec![p(0), p(1)], vec![p(2), p(3)]];
    let s = symmetric_component_solve(4, &sat).expect("two symmetric components decompose");
    assert_eq!(s.via, Route::SymmetricComponent);
    match &s.answer {
        Answer::Sat(m) => assert!(
            sat.iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the assembled model is valid"
        ),
        Answer::Unsat => panic!("two copies of (x∨y) is SAT"),
    }

    // Two interchangeable copies of an UNSAT gadget x∧y∧¬(x∧y). Each copy is UNSAT, and the mixed
    // clause widths give it no phase symmetry, so the variable group (with the copy swap) is seen — so
    // F is UNSAT, detected by solving just ONE copy.
    let unsat = vec![
        vec![p(0)],
        vec![p(1)],
        vec![n(0), n(1)],
        vec![p(2)],
        vec![p(3)],
        vec![n(2), n(3)],
    ];
    let u = symmetric_component_solve(4, &unsat).expect("two symmetric components decompose");
    assert_eq!(u.via, Route::SymmetricComponent);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "an UNSAT component makes F UNSAT"
    );

    // Soundness vs brute force on both.
    for cl in [&sat, &unsat] {
        let brute = (0u64..16).any(|x| {
            let a: Vec<bool> = (0..4).map(|i| (x >> i) & 1 == 1).collect();
            cl.iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        let got = symmetric_component_solve(4, cl).expect("decomposition fires");
        assert_eq!(
            matches!(got.answer, Answer::Sat(_)),
            brute,
            "component-decomposition verdict matches brute force"
        );
    }

    // A single connected component (clique_coloring(3,3)) has nothing to decompose — it declines.
    let (clique, _) = crate::families::clique_coloring(3, 3);
    assert!(
        symmetric_component_solve(clique.num_vars, &clique.clauses).is_none(),
        "a single component is not decomposable"
    );
}

#[test]
fn plain_component_decomposition_solves_asymmetric_independent_parts() {
    let p = |v| Lit::new(v, true);
    let n = |v| Lit::new(v, false);
    // Heterogeneous independent components: A = (x0∨x1) on {0,1}, B = (x2 ≠ x3) on {2,3}. Their different
    // structure means no automorphism maps one onto the other, so the SYMMETRIC route declines — this is
    // exactly the case plain component decomposition exists to cover.
    let sat = vec![vec![p(0), p(1)], vec![p(2), p(3)], vec![n(2), n(3)]];
    assert!(
        symmetric_component_solve(4, &sat).is_none(),
        "asymmetric components: the symmetric route declines"
    );
    let s = solve_by_components(4, &sat);
    assert_eq!(s.via, Route::Component);
    match &s.answer {
        Answer::Sat(m) => assert!(
            sat.iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the assembled model satisfies every clause"
        ),
        Answer::Unsat => panic!("both components are satisfiable, so F is SAT"),
    }

    // One component UNSAT (x2 ∧ ¬x2) ⟹ F is UNSAT, found without ever touching the other component.
    let unsat = vec![vec![p(0), p(1)], vec![p(2)], vec![n(2)]];
    let u = solve_by_components(3, &unsat);
    assert_eq!(u.via, Route::Component);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "an UNSAT component makes F UNSAT"
    );

    // A single connected component has nothing to split — component_solve declines to the fallback.
    let single = vec![vec![p(0), p(1)], vec![n(0), p(1)]];
    assert_ne!(
        solve_by_components(2, &single).via,
        Route::Component,
        "one component is not decomposable"
    );

    // Soundness vs brute force on both decomposable instances.
    for (nv, cl) in [(4usize, &sat), (3, &unsat)] {
        let brute = (0u64..(1 << nv)).any(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            cl.iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        assert_eq!(
            matches!(solve_by_components(nv, cl).answer, Answer::Sat(_)),
            brute,
            "matches brute force"
        );
    }
}

#[test]
fn symmetry_propagation_breaks_during_search_and_is_correct() {
    // clique_coloring(3,3): SAT and variable-symmetric. The lex-leader propagator prunes non-canonical
    // assignments DURING the search (no static clauses, no aux vars) and returns a re-checked model.
    let (sat, _) = crate::families::clique_coloring(3, 3);
    let s = symmetry_propagate_solve(sat.num_vars, &sat.clauses)
        .expect("a phase-free variable symmetry drives the propagator");
    assert_eq!(s.via, Route::SymmetryPropagate);
    match &s.answer {
        Answer::Sat(m) => assert!(
            sat.clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the propagator returns a valid model"
        ),
        Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
    }

    // clique_coloring(4,3): UNSAT (K₄ needs 4 colours). The propagator preserves satisfiability, so the
    // dynamic break decides UNSAT.
    let (unsat, _) = crate::families::clique_coloring(4, 3);
    let u = symmetry_propagate_solve(unsat.num_vars, &unsat.clauses).expect("variable symmetry");
    assert_eq!(u.via, Route::SymmetryPropagate);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "clique_coloring(4,3) is UNSAT"
    );

    // Soundness: the dynamic verdict matches an independent brute-force decision on both.
    for (cnf, _) in [
        crate::families::clique_coloring(3, 3),
        crate::families::clique_coloring(4, 3),
    ] {
        let nv = cnf.num_vars;
        let brute = (0u64..(1u64 << nv)).any(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        let got = symmetry_propagate_solve(nv, &cnf.clauses).expect("propagator fires");
        assert_eq!(
            matches!(got.answer, Answer::Sat(_)),
            brute,
            "symmetry-propagation verdict matches brute force (nv={nv})"
        );
    }
}

#[test]
fn orbit_weight_quotient_collapses_a_full_symmetric_instance() {
    let p = |v| Lit::new(v, true);
    // at-least-2 of {x0,x1,x2}: fully interchangeable (G = S₃, no phase symmetry), SAT at weight 2.
    let at_least_2 = vec![vec![p(0), p(1)], vec![p(0), p(2)], vec![p(1), p(2)]];
    let s = orbit_weight_quotient_solve(3, &at_least_2)
        .expect("a full symmetric group collapses to weight classes");
    assert_eq!(s.via, Route::OrbitWeightQuotient);
    match &s.answer {
        Answer::Sat(m) => {
            assert!(at_least_2
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())));
            assert!(
                m.iter().filter(|&&b| b).count() >= 2,
                "the witness has weight ≥ 2"
            );
        }
        Answer::Unsat => panic!("at-least-2 of 3 is SAT"),
    }

    // at-least-3 ∧ at-most-1 over {x0..x3}: S₄-symmetric, and UNSAT (≥3 and ≤1 true cannot both hold).
    // The clause widths differ (ternary at-least-3, binary at-most-1), so global complement does NOT
    // preserve the set — no phase symmetry — and the quotient sees the full S₄ variable group. It
    // checks weights 0..4 and finds none satisfying — an exact refutation.
    let subsets3 = [[0u32, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];
    let pairs = [[0u32, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]];
    let mut card: Vec<Vec<Lit>> = Vec::new();
    for sub in subsets3 {
        card.push(sub.iter().map(|&v| Lit::new(v, true)).collect()); // at-least-3
    }
    for pr in pairs {
        card.push(pr.iter().map(|&v| Lit::new(v, false)).collect()); // at-most-1
    }
    let u =
        orbit_weight_quotient_solve(4, &card).expect("S₄ is the full symmetric group on the orbit");
    assert_eq!(u.via, Route::OrbitWeightQuotient);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "≥3 and ≤1 true is unsatisfiable"
    );

    // Soundness vs brute force on both.
    for (nv, cl) in [(3usize, &at_least_2), (4, &card)] {
        let brute = (0u64..(1u64 << nv)).any(|x| {
            let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
            cl.iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        let got = orbit_weight_quotient_solve(nv, cl).expect("quotient fires");
        assert_eq!(
            matches!(got.answer, Answer::Sat(_)),
            brute,
            "orbit-weight-quotient verdict matches brute force (nv={nv})"
        );
    }

    // The gate DECLINES when the group is not the FULL product: clique_coloring(3,3)'s symmetry is
    // S₃(vertices)×S₃(colours) on the 3×3 grid — not the full S₉ on the cells — so a pure cell-swap is
    // not in the group and the quotient correctly bows out (the breaking routes handle it instead).
    let (clique, _) = crate::families::clique_coloring(3, 3);
    assert!(
        orbit_weight_quotient_solve(clique.num_vars, &clique.clauses).is_none(),
        "a product symmetry that is not the full symmetric group is not a weight quotient"
    );
}

#[test]
fn symmetric_probe_infers_a_whole_orbit_from_one_failed_literal() {
    let y = |b| Lit::new(3, b);
    let nx = |v| Lit::new(v, false);
    // x0,x1,x2 are interchangeable (S₃) and each is a FAILED literal: xᵢ=true forces y ∧ ¬y.
    let base: Vec<Vec<Lit>> = (0u32..3)
        .flat_map(|v| [vec![nx(v), y(true)], vec![nx(v), y(false)]])
        .collect();

    // SAT — all xᵢ false (y free). A single probe of x0 fails, and symmetry forces ¬x0,¬x1,¬x2 from
    // that one probe (the inference, not a search over the three).
    let s = symmetric_probe_solve(4, &base).expect("a symmetric failed literal engages the probe");
    assert_eq!(s.via, Route::SymmetricProbe);
    match &s.answer {
        Answer::Sat(m) => {
            assert!(base
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())));
            assert!(
                !m[0] && !m[1] && !m[2],
                "the whole orbit was inferred false from one probe"
            );
        }
        Answer::Unsat => panic!("the base instance is SAT"),
    }

    // UNSAT — add x0∨x1∨x2: once the orbit is forced false, the cardinality clause cannot hold.
    let mut unsat = base.clone();
    unsat.push(vec![
        Lit::new(0, true),
        Lit::new(1, true),
        Lit::new(2, true),
    ]);
    let u = symmetric_probe_solve(4, &unsat).expect("the probe engages");
    assert_eq!(u.via, Route::SymmetricProbe);
    assert!(
        matches!(u.answer, Answer::Unsat),
        "forcing the orbit false refutes the cardinality clause"
    );

    // Soundness: the verdict matches an independent brute-force decision on both.
    for cl in [&base, &unsat] {
        let brute = (0u64..16).any(|x| {
            let a: Vec<bool> = (0..4).map(|i| (x >> i) & 1 == 1).collect();
            cl.iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        let got = symmetric_probe_solve(4, cl).expect("probe fires");
        assert_eq!(
            matches!(got.answer, Answer::Sat(_)),
            brute,
            "symmetric-probe verdict matches brute force"
        );
    }
}

#[test]
fn local_symmetry_solve_exploits_branch_symmetry_and_is_correct() {
    // Globally asymmetric (the clause x0∨x1 singles out x1), but branching on x0 reveals an x1↔x2
    // symmetry in the residual. The local-symmetry route splits on x0, breaks each residual's
    // symmetry, and decides it correctly with a re-checked model.
    let f = vec![
        vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, true)], // ¬x0 ∨ x1 ∨ x2
        vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, false)], // ¬x0 ∨ ¬x1 ∨ ¬x2
        vec![Lit::new(0, true), Lit::new(1, true)],                     // x0 ∨ x1
    ];
    let brute = (0u64..8).any(|x| {
        let a: Vec<bool> = (0..3).map(|i| (x >> i) & 1 == 1).collect();
        f.iter()
            .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
    });
    let solved = local_symmetry_solve(3, &f).expect("a branch reveals local symmetry");
    assert_eq!(solved.via, Route::LocalSymmetry);
    assert_eq!(
        matches!(solved.answer, Answer::Sat(_)),
        brute,
        "local-symmetry verdict matches brute force"
    );
    if let Answer::Sat(m) = &solved.answer {
        assert!(
            f.iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the model from a local-symmetry branch satisfies F"
        );
    }
}

#[test]
fn symmetry_breaking_scales_to_a_large_group_via_partial_breaking() {
    // clique_coloring(6,6): |Aut| = 6!·6! = 518400 — too large to enumerate (> the 50k cap), so the
    // symmetry route falls back to sound per-generator PARTIAL breaking and still solves it with a
    // re-checked model. (The point: symmetry breaking runs even where the group cannot be enumerated.)
    let (cnf, _) = crate::families::clique_coloring(6, 6);
    let s = symmetry_break_solve(cnf.num_vars, &cnf.clauses)
        .expect("a large phase-free symmetry group still drives partial breaking");
    assert_eq!(s.via, Route::SymmetryBreak);
    match s.answer {
        Answer::Sat(m) => assert!(
            cnf.clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "partial breaking returns a valid model"
        ),
        Answer::Unsat => panic!("clique_coloring(6,6) is SAT"),
    }
}

#[test]
fn the_composite_lift_verdict_matches_the_supplied_equations() {
    // A composite one-hot CNF has m bits per group, so a Boolean brute force (2^(edges·m)) is
    // intractable — the right oracle is the supplied ℤ/m system the family also returns: the lifted
    // CNF verdict must agree with running modm::solve directly on those equations, and with the
    // family's declared expectation. (The equation-level engine is itself brute-force-verified in
    // modm's own tests.)
    use crate::modm::{solve as modm_solve, ModmOutcome};
    for (eqs, cnf, want) in [
        crate::families::mod_p_tseitin_expander(6, 6, 7),
        crate::families::mod_p_consistent_onehot(6, 6, 7),
    ] {
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert_eq!(
            solved.via,
            Route::ModM,
            "a composite instance must take the ℤ/m route"
        );
        let num_gf_vars = cnf.num_vars / 6; // one ℤ/6 variable (edge) per 6 one-hot bits
        let supplied_unsat = matches!(
            modm_solve(&eqs, num_gf_vars, 6),
            Some(ModmOutcome::Unsat { .. })
        );
        assert_eq!(
            matches!(solved.answer, Answer::Unsat),
            supplied_unsat,
            "the lifted-CNF verdict must match modm::solve on the supplied equations"
        );
        assert_eq!(
            matches!(solved.answer, Answer::Unsat),
            matches!(want, crate::families::ExpectedVerdict::Unsat),
            "and the family's declared expectation"
        );
    }
}

#[test]
fn mine_clauses_derives_an_implied_unit_via_probing() {
    // {a} ∧ {¬a ∨ b} ⇒ b is implied (a forces b); failed-literal probing must mine the unit b.
    let clauses = vec![
        vec![Lit::new(0, true)],
        vec![Lit::new(0, false), Lit::new(1, true)],
    ];
    let mined = mine_clauses(2, &clauses);
    assert!(
        mined
            .iter()
            .any(|c| c.len() == 1 && c[0].var() == 1 && c[0].is_positive()),
        "expected mined implied unit b; got {mined:?}"
    );
}

#[test]
fn every_mined_clause_is_implied() {
    // Soundness: each mined clause must hold in EVERY model of the formula (brute force).
    let clauses = vec![
        xor_gadget(&[0, 1, 2], false),
        vec![vec![Lit::new(0, true)]],
        vec![vec![Lit::new(1, true)]],
    ]
    .concat();
    let n = 3;
    let mined = mine_clauses(n, &clauses);
    assert!(
        !mined.is_empty(),
        "this instance has implied structure to mine"
    );
    for mask in 0u32..(1 << n) {
        let asg: Vec<bool> = (0..n).map(|v| (mask >> v) & 1 == 1).collect();
        let is_model = clauses
            .iter()
            .all(|c| c.iter().any(|l| asg[l.var() as usize] == l.is_positive()));
        if is_model {
            for mc in &mined {
                assert!(
                    mc.iter().any(|l| asg[l.var() as usize] == l.is_positive()),
                    "mined clause {mc:?} is not implied (fails model {asg:?})"
                );
            }
        }
    }
}

/// **The exact-cover lift crushes modular counting and the chessboard — one harvest, every
/// modulus, certified.** Exactly-one groups (an all-positive clause whose variable pairs all
/// carry at-most-one clauses) yield the equations `Σ_{v∈g} x_v = 1`, valid consequences over
/// EVERY modulus; Gaussian elimination over `GF(2)`, `GF(3)`, `GF(5)` then finds whatever
/// counting obstruction exists — the parity sum for `Count_2`, the black-minus-white
/// combination for the mutilated chessboard over `GF(3)`, the mod-3/5 sums for `Count_{3,5}` —
/// each refutation re-checked fail-closed before the route reports. This turns the satbench
/// CONTROL rows (`Count_2` odd-matching fell through to CDCL at ~40–60 ms) into µs crushes with
/// zero search. Soundness: the route only ever reports UNSAT (with a re-checked certificate);
/// satisfiable coverings decline through to the rest of the chain, and a SAT control is pinned.
#[test]
fn exact_cover_lift_crushes_modular_counting_and_the_chessboard() {
    // Count_2 on odd n — previously a CONTROL row (CDCL); now the GF(2) point-sum, no search.
    for n in [7usize, 9, 11] {
        let (cnf, _) = crate::families::mod_counting(n, 2);
        let s = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(s.answer, Answer::Unsat), "Count_2({n}) is UNSAT");
        assert_eq!(
            s.via,
            Route::ExactCover,
            "Count_2({n}): the exact-cover lift fires"
        );
        assert_eq!(s.conflicts, 0, "Count_2({n}): zero search");
    }
    // Count_3 and Count_5 — the same harvest, higher rungs of the modulus ladder.
    for (n, q) in [(7usize, 3usize), (8, 3), (6, 5), (7, 5)] {
        let (cnf, _) = crate::families::mod_counting(n, q);
        let s = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(s.answer, Answer::Unsat), "Count_{q}({n}) is UNSAT");
        assert_eq!(
            s.via,
            Route::ExactCover,
            "Count_{q}({n}): the exact-cover lift fires"
        );
    }
    // The mutilated chessboard: the ±1 (black/white) combination lives in GF(3) — Gaussian
    // finds it from the raw covering equations, no bipartite reasoning supplied.
    let (cb, _) = crate::families::mutilated_chessboard(4);
    let s = solve_structured(cb.num_vars, &cb.clauses);
    assert!(matches!(s.answer, Answer::Unsat), "chessboard(4) is UNSAT");
    assert_eq!(
        s.conflicts, 0,
        "chessboard(4): a specialist decides — zero search"
    );
    assert_ne!(s.via, Route::Cdcl, "chessboard(4): never the fallback");
    // SAT control: a satisfiable covering never yields a false UNSAT from the lift.
    let (sat6, _) = crate::families::mod_counting(6, 3);
    let s = solve_structured(sat6.num_vars, &sat6.clauses);
    match s.answer {
        Answer::Sat(model) => {
            assert!(
                sat6.clauses
                    .iter()
                    .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                "Count_3(6): the SAT model re-checks"
            );
        }
        Answer::Unsat => panic!("Count_3(6) is satisfiable — the lift must decline"),
    }
}

/// **Measurement: the symmetry arsenal against Ramsey.** Ramsey formulas carry the huge `Sₙ`
/// vertex symmetry induced on edge variables — clique geometry, no counting or parity to lift,
/// so the exact-cover and algebraic routes rightly decline and the question is whether the
/// symmetry routes (detection + orbital branching / SEL / lex-leader, `solve_comprehensive`)
/// beat the raw CDCL fallback the fast chain uses. Wall time, route, and conflicts per engine,
/// on `ramsey(3,3;6)` and `ramsey(3,4;9)` — the measured groundwork for the crushable frontier
/// (`ramsey(3,5;14)`, where plain CDCL walls). Verdict correctness asserted; the comparison is
/// reported, not presumed.
#[test]
#[ignore = "scale measurement — symmetry-arsenal wall time on Ramsey; run explicitly"]
fn ramsey_symmetric_attack_is_measured() {
    for (s, t, n) in [(3usize, 3usize, 6usize), (3, 4, 9)] {
        let (cnf, _) = crate::families::ramsey(s, t, n);
        let t0 = std::time::Instant::now();
        let fast = solve_structured(cnf.num_vars, &cnf.clauses);
        let fast_ms = t0.elapsed().as_millis();
        assert!(
            matches!(fast.answer, Answer::Unsat),
            "ramsey({s},{t};{n}) is UNSAT"
        );
        let t1 = std::time::Instant::now();
        let full = solve_comprehensive(cnf.num_vars, &cnf.clauses);
        let full_ms = t1.elapsed().as_millis();
        assert!(
            matches!(full.answer, Answer::Unsat),
            "ramsey({s},{t};{n}) is UNSAT (arsenal)"
        );
        eprintln!(
            "RAMSEY | ({s},{t};{n}) [{} vars]: fast={:?} {}ms {} conflicts | arsenal={:?} {}ms {} conflicts",
            cnf.num_vars, fast.via, fast_ms, fast.conflicts, full.via, full_ms, full.conflicts
        );
    }
}
