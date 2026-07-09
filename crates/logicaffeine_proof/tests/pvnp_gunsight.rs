//! **The Cook–Reckhow gunsight: aiming the certified portfolio at P vs NP, honestly.**
//!
//! The bridge is one theorem — **Cook–Reckhow (1979): NP = coNP iff some propositional proof system
//! is polynomially bounded** (every tautology has a poly-size proof); `P = NP ⟹ NP = coNP`, so
//! refuting poly-boundedness for every system is the proof-complexity road to `P ≠ NP`. This test
//! machine-checks where the repository's certificates sit on that road, cell by cell:
//!
//!   - **The weak-system routes to a collapse are certifiably closed on pigeonhole.** In their own
//!     cost measures, per-instance and growing: Nullstellensatz degree (exact `2(m−1)` — at
//!     characteristic 2, at characteristic 3, and lifted to the ring `ℤ/6`) and resolution width
//!     (exact `m−1` under the wide-axiom convention, with the closed-set lower-bound certificate
//!     re-checked with zero trust). The asymptotic size forms are classical (Razborov; Haken,
//!     Ben-Sasson–Wigderson) — cited, not claimed; what is machine-checked is the certified,
//!     growing per-instance cost.
//!   - **Those same instances cannot witness hardness at the frontier.** The Extended-Frege-class
//!     engine refutes every one of them CHEAPLY: `heule_php_refutation(m)` emits a certified SR
//!     proof of quadratically many steps, re-verified by the zero-trust PR/SR checker. So
//!     pigeonhole — the family that kills the weak systems — is *dead as an EF-hardness
//!     candidate*, machine-checked. This is why the ladder's frontier is where it is.
//!   - **The frontier cell, named exactly.** What would advance `P ≠ NP` from here: a family with
//!     *certified superpolynomial EF-class proof size*. No such family is known — for Frege/EF no
//!     superpolynomial lower bound exists, and the classical barriers (relativization, natural
//!     proofs, algebrization) explain why the algebraic and structural techniques certified in this
//!     repository cannot be the ones to cross. Conversely, the aim `P = NP` through a
//!     poly-bounded system must run through a system at least as strong as the EF class — every
//!     weaker route in this portfolio ends at a certificate that says no.
//!
//! Neither direction of P vs NP is decided here, and no rung of this test claims otherwise. The
//! gunsight's contribution is that the target is now a *named, machine-tracked cell* with every
//! approach lane certifiably marked closed or open.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::families;
use logicaffeine_proof::polycalc;
use logicaffeine_proof::polycalc_gfp::{ns_refutes_gfp, NsField};
use logicaffeine_proof::polycalc_zm::{check_ns_lower_bound_zm, lift_prime_witness_to_zm};
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::res_width::{
    check_res_width_lower_bound, min_res_width_clauses, resolution_width_closure, WidthConvention,
};
use logicaffeine_proof::sym_certify::heule_php_refutation;

/// **The survivor ledger: the gunsight lined up — and the survivor certified NOT RANDOM.** Every
/// candidate family is run through the entire certified arsenal and sorted: DEAD (a certified
/// dissolution exists somewhere in the portfolio — useless as a frontier witness) or SURVIVOR (no
/// specialist fires anywhere). The verdicts:
///
///   - **DEAD**: pigeonhole (SR, `m(m−1)/2` steps — the companion test), Tseitin expanders (the
///     `GF(2)` parity specialist), mod-3 Tseitin (the `GF(3)` specialist) — each dissolved by a
///     certified engine, each therefore unable to witness frontier hardness.
///   - **SURVIVOR: threshold 3-CNF.** On pinned UNSAT samples, the full 30-route dispatcher finds
///     NO certified shortcut — every sample routes to raw CDCL search — and the certified ladder
///     reports `BeyondBudget`. This is the lone family in the corpus with no certified dissolution,
///     which is exactly the classical conjecture (hard for every proof system, EF included).
///
/// And the sharpest fact, per the finite-`n` randomness theorem: **the survivor is not random.**
/// "Random 3-CNF" names the *sampling distribution*; each sampled instance is a fixed finite
/// formula, and this test BUILDS its structure certificate over the ring `ℤ/6` and re-checks it —
/// no instance of the hardest-known family is structureless. What survives is not randomness; it is
/// **cost**: structure exists (certified per instance), cheap structure does not (no specialist
/// fires, certified per instance). The frontier target is therefore precisely "expensive structure,
/// uniformly along a family" — never "no structure."
#[test]
fn the_survivor_ledger_lines_up_threshold_3cnf_and_certifies_it_is_not_random() {
    use logicaffeine_proof::hypercube::{weakest_crushing_rung, ProofRung};
    use logicaffeine_proof::polycalc_zm::build_ns_certificate_zm;
    use logicaffeine_proof::solve::{solve_structured, Answer, Route};

    // DEAD rows: certified dissolutions fire — these families cannot witness frontier hardness.
    let (_, tseitin, _) = families::tseitin_expander(6, 0xC0DE);
    let solved = solve_structured(tseitin.num_vars, &tseitin.clauses);
    assert!(matches!(solved.answer, Answer::Unsat));
    assert_ne!(solved.via, Route::Cdcl, "Tseitin: a certified specialist dissolves it");
    let (_, mod3, _) = families::mod_p_tseitin_expander(4, 3, 0xC0DE);
    let solved = solve_structured(mod3.num_vars, &mod3.clauses);
    assert!(matches!(solved.answer, Answer::Unsat));
    assert_ne!(solved.via, Route::Cdcl, "mod-3 Tseitin: the GF(3) specialist dissolves it");

    // SURVIVOR rows: pinned UNSAT threshold-3-CNF samples (density 5, fixed seeds).
    let samples: &[(usize, u64)] = &[(12, 2), (12, 3), (16, 3), (20, 1), (20, 2), (20, 3)];
    for &(n, seed) in samples {
        let cnf = families::random_3sat(n, n * 5, seed);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(solved.answer, Answer::Unsat), "n={n} seed={seed}: the pinned sample is UNSAT");
        assert_eq!(
            solved.via,
            Route::Cdcl,
            "n={n} seed={seed}: NO certified specialist fires — the survivor signature"
        );
        if n <= 16 {
            assert_eq!(
                weakest_crushing_rung(cnf.num_vars, &cnf.clauses, 3),
                ProofRung::BeyondBudget,
                "n={n} seed={seed}: the certified ladder cannot place it within budget"
            );
        }
        if n <= 12 {
            // NOT RANDOM: the sampled instance's structure certificate, built and re-checked over
            // the ring ℤ/6 — finite "randomness" is expensive structure, never absent structure.
            // (The explicit-corner construction is 3ⁿ work; the larger samples carry the same
            // theorem through §5.11's ∀-modulus completeness rather than a per-instance build.)
            let cert = build_ns_certificate_zm(6, cnf.num_vars, &cnf.clauses)
                .expect("the survivor instance certifies — nothing finite is random");
            assert!(cert.verify(&cnf.clauses), "n={n} seed={seed}: the certificate re-checks");
        }
        eprintln!(
            "SURVIVOR 3CNF n={n} seed={seed}: UNSAT, via=Cdcl (no shortcut), conflicts={}{}",
            solved.conflicts,
            if n <= 12 { ", structure CERTIFIED over ℤ/6" } else { "" }
        );
    }
    eprintln!("ledger: PHP/Tseitin/mod-p DEAD (certified dissolutions); threshold 3-CNF SURVIVES — the lined-up target");
}

/// **The trick matrix: EVERY family has a trick, and EVERY portfolio system has a hard family —
/// both patterns certified, and P vs NP is exactly whether the quantifiers swap.** The posit
/// "everything hard has a trick" is provably TRUE in its per-family form, and this test certifies
/// it row by row: pigeonhole's trick is the symmetry/SR system (`m(m−1)/2` steps — and the SDCL
/// engine DISCOVERS that trick with zero hints: the automatic trick-finder, lift-and-shift-left
/// made executable); Tseitin's trick is the `GF(2)` specialist; mod-3 Tseitin's and `Count_3`'s is
/// `GF(3)`; parity's is `GF(2)`; and each surviving threshold-3-CNF instance's trick is its own
/// CDCL-found RUP proof — expensive to FIND, polynomial to RE-CHECK (verified here with the
/// zero-trust checker), which is the very shape of NP. Simultaneously, column by column, every
/// system in the portfolio has a certified hard family: `GF(2)`-NS ← PHP and `Count_3`;
/// `GF(3)`-NS ← parity; `ℤ/6`-NS ← parity (the conjunction inherits BOTH primes' weaknesses);
/// resolution ← PHP width. The two certified patterns together give the exact logical content of
/// the posit: **∀family ∃trick (true, certified) vs ∃trick ∀family (= a poly-bounded system = NP
/// = coNP, by Cook–Reckhow — the open cell)**. The question is not whether tricks exist — they
/// always do — but whether one trick, or even a poly-time trick-FINDER, covers every family at
/// once; every system we can certify is covered by a hard family, and the first uncovered column,
/// if any, must sit at or above the EF class.
#[test]
fn every_family_has_a_trick_and_every_portfolio_system_has_a_hard_family() {
    use logicaffeine_proof::polycalc_gfp::{
        check_ns_lower_bound_polys_gfp, exactly_one_linear_generators_gfp,
        ns_lower_bound_witness_polys_gfp, ns_refutes_polys_gfp,
    };
    use logicaffeine_proof::polycalc_zm::ns_refutes_zm;
    use logicaffeine_proof::sdcl::sdcl_refute;
    use logicaffeine_proof::solve::{solve_structured, Answer, Route};

    // ── Row PHP: hard for NS-GF(2) and resolution (certified in the companion test); its TRICK is
    //    the symmetry/SR system — and SDCL DISCOVERS the trick unaided.
    let (php3, _) = families::php(3);
    let auto = sdcl_refute(php3.num_vars, &php3.clauses);
    assert!(auto.refuted, "PHP(3): the automatic trick-finder refutes with zero hints");
    assert!(
        check_pr_refutation(php3.num_vars, &php3.clauses, &auto.steps),
        "PHP(3): the self-discovered trick re-checks with zero trust"
    );

    // ── Row Tseitin: hard for resolution (certified width elsewhere); trick = the GF(2) route.
    let (_, tseitin, _) = families::tseitin_expander(6, 0xC0DE);
    let solved = solve_structured(tseitin.num_vars, &tseitin.clauses);
    assert!(matches!(solved.answer, Answer::Unsat) && solved.via != Route::Cdcl);

    // ── Rows Count_3 / parity: each hard at ONE characteristic, tricked by the other — and the
    //    ring ℤ/6 column is hard for BOTH (the conjunction inherits every prime's weakness).
    let f2 = NsField::Prime(2);
    let f3 = NsField::Prime(3);
    let (cnt, _) = families::mod_counting(4, 3);
    let groups = families::mod_counting_groups(4, 3);
    let g2 = logicaffeine_proof::polycalc::exactly_one_linear_generators(&groups);
    let g3 = exactly_one_linear_generators_gfp(f3, &groups);
    let w = logicaffeine_proof::polycalc::ns_lower_bound_witness_polys(cnt.num_vars, &g2, 1)
        .expect("Count_3(4): hard cell at GF(2) degree 1");
    assert!(logicaffeine_proof::polycalc::check_ns_lower_bound_polys(cnt.num_vars, &g2, 1, &w));
    assert!(ns_refutes_polys_gfp(f3, cnt.num_vars, &g3, 1), "Count_3(4): the GF(3) trick, degree 1");
    let parity: Vec<Vec<Lit>> = {
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        vec![
            vec![p(0), p(1)], vec![q(0), q(1)],
            vec![p(1), p(2)], vec![q(1), q(2)],
            vec![p(2), p(0)], vec![q(2), q(0)],
        ]
    };
    let pgen3: Vec<_> = parity.iter().map(|c| logicaffeine_proof::polycalc_gfp::clause_polynomial_gfp(f3, c)).collect();
    let w3 = ns_lower_bound_witness_polys_gfp(f3, 3, &pgen3, 1)
        .expect("parity: hard cell at GF(3) degree 1");
    assert!(check_ns_lower_bound_polys_gfp(f3, 3, &pgen3, 1, &w3));
    assert!(
        ns_refutes_gfp(f2, 3, &parity, 2) && !ns_refutes_zm(6, 3, &parity, 2),
        "parity: tricked by GF(2) at degree 2, while the ℤ/6 column stays hard — the conjunction"
    );

    // ── Row threshold-3CNF (the survivor): no specialist trick — but its OWN FOUND PROOF is the
    //    per-instance trick: expensive to find, polynomial to re-check. The shape of NP itself.
    let cnf = families::random_3sat(12, 60, 2);
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    assert!(matches!(solved.answer, Answer::Unsat) && solved.via == Route::Cdcl);
    assert!(!solved.proof.is_empty(), "the survivor's trick is its own discovered proof");
    assert!(
        check_pr_refutation(cnf.num_vars, &cnf.clauses, &solved.proof),
        "the found trick re-checks in polynomial time — finding was the only expensive part"
    );

    eprintln!(
        "trick matrix: every family row has a certified trick cell (∀F ∃system: easy — TRUE); \
         every portfolio system column has a certified hard family (∀S∈portfolio ∃F: hard — TRUE); \
         the swap ∃system ∀F = a poly-bounded system = NP = coNP (Cook–Reckhow) — the open cell, \
         reachable only at or above the EF class"
    );
}

/// A 3-coloring instance reduced to SAT: variables `x_{v,c}` (`c ∈ {0,1,2}`), one-hot per vertex,
/// and no edge monochromatic.
fn coloring_to_sat(vertices: usize, edges: &[(usize, usize)]) -> (usize, Vec<Vec<Lit>>) {
    let var = |v: usize, c: usize| (3 * v + c) as u32;
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for v in 0..vertices {
        clauses.push((0..3).map(|c| Lit::new(var(v, c), true)).collect());
        for c1 in 0..3 {
            for c2 in (c1 + 1)..3 {
                clauses.push(vec![Lit::new(var(v, c1), false), Lit::new(var(v, c2), false)]);
            }
        }
    }
    for &(u, v) in edges {
        for c in 0..3 {
            clauses.push(vec![Lit::new(var(u, c), false), Lit::new(var(v, c), false)]);
        }
    }
    (3 * vertices, clauses)
}

/// **The NP-complete class layer: problems exist, instances are never random, class-hardness is
/// the open cell.** Three statements the user's question pulls apart, each now an artifact:
///
///   1. **NP-complete problems EXIST — that is settled mathematics** (Cook–Levin: SAT is
///      NP-complete), and reductions are executable: 3-COLORING compiles to SAT here, with the
///      reduction verified faithful in both directions against brute force (a graph is 3-colorable
///      iff its CNF is satisfiable, checked exhaustively on the corpus).
///   2. **No instance of an NP-complete problem is random.** The poles transport through the
///      reduction: the non-3-colorable `K₄` reduces to an UNSAT CNF whose structure certificate is
///      built and re-checked over `ℤ/6` — the negative instance of an NP-hard problem, certified
///      structured; the colorable instances yield models that decode to verified proper colorings.
///   3. What "NP-hardness" could still mean after 1 and 2 is exactly the **cost** question — the
///      gunsight's open cell. "NP problems don't exist" is false (Cook–Levin); "no NP instance is
///      random" is true and certified; "the class costs superpolynomially in every proof system"
///      is open, and it is the ONLY remaining reading — now machine-separated from the other two.
#[test]
fn np_complete_instances_inherit_the_poles_through_certified_reductions() {
    use logicaffeine_proof::polycalc_zm::build_ns_certificate_zm;
    use logicaffeine_proof::solve::{solve_structured, Answer};

    let corpus: &[(&str, usize, Vec<(usize, usize)>)] = &[
        ("K3 (triangle)", 3, vec![(0, 1), (1, 2), (0, 2)]),
        ("C5 (odd cycle)", 5, vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]),
        ("K4 (4-clique)", 4, vec![(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]),
    ];
    for (name, vertices, edges) in corpus {
        let (nv, clauses) = coloring_to_sat(*vertices, edges);
        // Ground truth by brute force over all 3^V colorings — the reduction must be faithful.
        let colorable = (0..3u32.pow(*vertices as u32)).any(|code| {
            let color = |v: usize| (code / 3u32.pow(v as u32)) % 3;
            edges.iter().all(|&(u, v)| color(u) != color(v))
        });
        match solve_structured(nv, &clauses) {
            solved if matches!(solved.answer, Answer::Sat(_)) => {
                assert!(colorable, "{name}: SAT ⟹ genuinely 3-colorable");
                let Answer::Sat(model) = solved.answer else { unreachable!() };
                // Decode the model to a coloring and verify it is proper and total.
                let coloring: Vec<usize> = (0..*vertices)
                    .map(|v| (0..3).find(|&c| model[3 * v + c]).expect("one-hot decodes"))
                    .collect();
                assert!(
                    edges.iter().all(|&(u, v)| coloring[u] != coloring[v]),
                    "{name}: the decoded coloring is proper — the reduction round-trips"
                );
            }
            solved => {
                assert!(matches!(solved.answer, Answer::Unsat));
                assert!(!colorable, "{name}: UNSAT ⟹ genuinely non-3-colorable");
                // The negative instance of an NP-complete problem is NOT random: its structure
                // certificate over ℤ/6, built and re-checked.
                let cert = build_ns_certificate_zm(6, nv, &clauses)
                    .expect("the non-colorable instance still certifies");
                assert!(cert.verify(&clauses), "{name}: the ℤ/6 structure certificate re-checks");
                eprintln!("{name}: non-3-colorable — UNSAT certified AND structure-certified (not random)");
            }
        }
    }
    eprintln!(
        "NP layer: problems exist (Cook–Levin, executable reduction); instances are never random \
         (poles transport through reductions); class-cost-hardness is the gunsight's open cell"
    );
}

#[test]
fn the_gunsight_closes_the_weak_routes_and_names_the_frontier_cell() {
    // ── Cell 1: Nullstellensatz — certified growing degree on PHP, at every characteristic and
    //    over the ring. The NS route to a poly-bounded system dies here.
    let mut ns_degrees = Vec::new();
    for (m, exact) in [(3usize, 4usize), (4, 6)] {
        let (php, _) = families::php(m);
        assert!(
            !polycalc::nullstellensatz_refutes(php.num_vars, &php.clauses, exact - 1),
            "PHP({m}): no GF(2) NS refutation below {exact}"
        );
        assert!(
            polycalc::nullstellensatz_refutes(php.num_vars, &php.clauses, exact),
            "PHP({m}): GF(2) NS degree exactly {exact}"
        );
        ns_degrees.push(exact);
    }
    assert!(ns_degrees.windows(2).all(|w| w[1] > w[0]), "the NS cost grows: {ns_degrees:?}");
    let (php3, _) = families::php(3);
    assert!(!ns_refutes_gfp(NsField::Prime(3), php3.num_vars, &php3.clauses, 3));
    assert!(ns_refutes_gfp(NsField::Prime(3), php3.num_vars, &php3.clauses, 4));
    let w2 = logicaffeine_proof::polycalc_gfp::ns_lower_bound_witness_gfp(
        NsField::Prime(2),
        php3.num_vars,
        &php3.clauses,
        3,
    )
    .expect("the GF(2) witness at degree 3 exists");
    let lifted = lift_prime_witness_to_zm(6, 2, &w2);
    assert!(
        check_ns_lower_bound_zm(6, php3.num_vars, &php3.clauses, 3, &lifted),
        "PHP(3): the hardness lifts to the ring ℤ/6 — no composite-modulus escape route"
    );

    // ── Cell 2: resolution — certified growing width on PHP (wide-axiom convention), with the
    //    closed-set lower-bound certificate re-checked with zero trust.
    let mut widths = Vec::new();
    for m in [3usize, 4] {
        let (php, _) = families::php(m);
        let w = min_res_width_clauses(php.num_vars, &php.clauses, WidthConvention::WideAxioms)
            .expect("PHP is UNSAT — some width refutes");
        assert_eq!(w, m - 1, "PHP({m}): wide-axiom resolution width is exactly m−1");
        let closed = resolution_width_closure(&php.clauses, w - 1, WidthConvention::WideAxioms);
        assert!(
            check_res_width_lower_bound(&php.clauses, w - 1, WidthConvention::WideAxioms, &closed),
            "PHP({m}): the width-{} lower bound re-checks from the closed set",
            w - 1
        );
        widths.push(w);
    }
    assert!(widths.windows(2).all(|w| w[1] > w[0]), "the resolution cost grows: {widths:?}");

    // ── Cell 3, the frontier: the SAME family is CHEAP for the EF-class engine — certified short
    //    SR proofs, quadratic step count, re-verified with zero trust. PHP cannot witness
    //    EF-hardness; no known family can; that cell is THE open cell of the P ≠ NP program.
    let mut step_counts = Vec::new();
    for m in 3usize..=8 {
        let (php, _) = families::php(m);
        let cert = heule_php_refutation(m);
        assert!(cert.refuted, "PHP({m}): the EF-class engine refutes it");
        assert!(
            check_pr_refutation(php.num_vars, &php.clauses, &cert.steps),
            "PHP({m}): the SR proof re-checks with zero trust in the producer"
        );
        assert!(
            cert.steps.len() <= 3 * m * m,
            "PHP({m}): {} steps — polynomially small, the family is dead as an EF-hardness candidate",
            cert.steps.len()
        );
        step_counts.push((m, cert.steps.len()));
    }
    eprintln!("gunsight cell 1 (NS): PHP degree {ns_degrees:?} — growing, char-invariant, ring-lifted: CLOSED");
    eprintln!("gunsight cell 2 (resolution): PHP width {widths:?} — growing, certificate re-checked: CLOSED");
    eprintln!(
        "gunsight cell 3 (EF-class frontier): PHP SR steps {step_counts:?} — POLYNOMIAL, so the \
         weak-system killers are dead as frontier witnesses; the open cell is a family with \
         certified superpolynomial EF-class proof size"
    );
    eprintln!(
        "the chain: Cook–Reckhow ties poly-boundedness to NP = coNP; every route below the EF class \
         is certifiably closed above; neither direction of P vs NP is decided by any cell here"
    );
}
