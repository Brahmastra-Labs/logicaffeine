use super::*;
use crate::lrat::LratStep;

fn p(v: u32) -> Lit {
    Lit::pos(v)
}
fn n(v: u32) -> Lit {
    Lit::neg(v)
}

fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    (0u32..1 << num_vars).any(|m| clauses.iter().all(|c| c.iter().any(|l| ((m >> l.var()) & 1 == 1) == l.is_positive())))
}

fn refuted(num_vars: usize, clauses: &[Vec<Lit>], what: &str) -> (KernelRefutation, KernelCheck) {
    match refute(num_vars, clauses) {
        KernelVerdict::Refuted(r, how) => (r, how),
        other => panic!("{what}: expected a kernel-checked refutation, got {other:?}"),
    }
}

/// `x ∧ ¬x`: the smallest refutation, and both kernels accept it.
#[test]
fn a_contradiction_is_refuted_and_both_kernels_agree() {
    let (r, how) = refuted(1, &[vec![p(0)], vec![n(0)]], "x ∧ ¬x");
    assert_eq!(how, KernelCheck::TwoKernels, "the independent kernel agrees on a unit refutation");
    assert_eq!(check(&r), Ok(KernelCheck::TwoKernels), "re-checking gives the same verdict");
}

/// The context is exactly the formula: one `Prop` atom per variable, then one hypothesis per clause stating it; the
/// development ends in `False`.
#[test]
fn the_context_declares_every_variable_and_assumes_every_clause() {
    let clauses = vec![vec![p(0), p(1)], vec![n(0)], vec![n(1), p(2)], vec![n(2)]];
    let context = hypotheses(3, &clauses);
    let names: Vec<&str> = context.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["SAT.p0", "SAT.p1", "SAT.p2", "SAT.h0", "SAT.h1", "SAT.h2", "SAT.h3"]);
    assert!(context[..3].iter().all(|(_, ty)| *ty == Term::Sort(Universe::Prop)), "the atoms are propositions");
    assert_eq!(context[4].1, holds(n(0)), "a unit clause is its literal");
    assert_eq!(context[3].1, disjunction(&clauses[0]), "a clause is the disjunction of its literals");
    let (r, _) = refuted(3, &clauses, "a resolution chain through a binary clause");
    assert_eq!(r.lemmas.last().map(|l| l.statement.clone()), Some(Term::Global("False".to_string())));
}

/// Pigeonhole: every clause shape the kernel term meets — wide positive rows eliminated through nested `Or`, binary
/// negative columns, propagated negative units — in a refutation the kernel checks.
#[test]
fn pigeonhole_is_refuted_by_a_kernel_checked_term() {
    for m in [2usize, 3] {
        let (php, _) = crate::families::php(m);
        refuted(php.num_vars, &php.clauses, &format!("PHP({m})"));
    }
}

/// Random formulas, decided against brute force: every unsatisfiable one is refuted by a term the kernel checks,
/// every satisfiable one reports a model of every clause.
#[test]
fn random_formulas_are_refuted_by_the_kernel_or_satisfied() {
    let (mut unsat, mut sat) = (0, 0);
    for seed in 0..60u64 {
        let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let nv = 4 + (seed % 4) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nv * 5)
            .map(|_| {
                let mut c: Vec<Lit> = Vec::new();
                while c.len() < 3 {
                    let l = Lit::new((next() % nv as u64) as u32, next() & 1 == 0);
                    if !c.iter().any(|m| m.var() == l.var()) {
                        c.push(l);
                    }
                }
                c
            })
            .collect();
        match refute(nv, &clauses) {
            KernelVerdict::Refuted(..) => {
                assert!(!sat_brute(nv, &clauses), "seed {seed}: a satisfiable formula was refuted");
                unsat += 1;
            }
            KernelVerdict::Satisfiable(model) => {
                assert!(clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())));
                sat += 1;
            }
            KernelVerdict::Unknown(why) => panic!("seed {seed}: every refutation becomes a checked term: {why}"),
        }
    }
    assert!(unsat >= 20 && sat >= 5, "{unsat} refuted, {sat} satisfied");
}

/// The kernel checks against the formula, not the refutation's say-so: a development of one formula checked against
/// another is rejected — with a clause dropped, or with a literal's sign flipped.
#[test]
fn a_proof_does_not_check_against_another_formulas_statement() {
    let clauses = vec![vec![p(0), p(1)], vec![n(0), p(1)], vec![p(0), n(1)], vec![n(0), n(1)]];
    let (r, _) = refuted(2, &clauses, "the four clauses over two variables");
    let dropped = KernelRefutation { clauses: clauses[..3].to_vec(), ..r.clone() };
    assert!(check(&dropped).is_err(), "the formula without its last clause is satisfiable — no proof of it");
    let mut flipped = clauses.clone();
    flipped[0] = vec![p(0), n(1)];
    let other = KernelRefutation { clauses: flipped, ..r.clone() };
    assert!(check(&other).is_err(), "a hypothesis with a different clause is not what the lemmas use");
}

/// The development must end in `False` and declare nothing twice: a refutation that stops short, one whose last lemma
/// states something else, and one that redeclares a hypothesis or the standard library are refused.
#[test]
fn a_development_that_stops_short_or_redeclares_is_refused() {
    let clauses = vec![vec![p(0), p(1)], vec![n(0), p(1)], vec![p(0), n(1)], vec![n(0), n(1)]];
    let (r, _) = refuted(2, &clauses, "the four clauses over two variables");
    let mut short = r.clone();
    short.lemmas.pop();
    assert!(check(&short).is_err(), "without its last lemma the development proves no False");
    let mut restated = r.clone();
    let last = restated.lemmas.last_mut().unwrap();
    last.statement = arrow(falsum(), falsum());
    assert!(check(&restated).is_err(), "a last lemma stating False → False is no refutation");
    for taken in ["SAT.h0", "False", "Or"] {
        let mut renamed = r.clone();
        renamed.lemmas[0].name = taken.to_string();
        assert!(check(&renamed).is_err(), "{taken} is declared already");
    }
    assert!(check(&KernelRefutation { lemmas: Vec::new(), ..r.clone() }).is_err(), "no lemmas, no refutation");
}

/// A proof whose chains do not replay gives no development: a conflict hint replaced by a clause that is not
/// falsified, a hint naming no clause, and literals beyond the variables.
#[test]
fn an_lrat_proof_that_does_not_replay_gives_no_development() {
    let clauses = vec![vec![p(0), p(1)], vec![n(0), p(1)], vec![p(0), n(1)], vec![n(0), n(1)]];
    let mut s = Solver::new(2);
    for c in &clauses {
        s.add_clause(c.clone());
    }
    assert_eq!(s.solve(), SolveResult::Unsat);
    let learned: Vec<Vec<Lit>> = s.learned().iter().map(|c| c.lits.clone()).collect();
    let good = crate::lrat::refutation(2, &clauses, &learned).expect("an LRAT refutation");
    assert!(lemmas(2, &clauses, &good).is_some());

    let mut no_conflict = good.clone();
    no_conflict.steps.last_mut().unwrap().hints.truncate(1);
    assert!(lemmas(2, &clauses, &no_conflict).is_none(), "a chain that never conflicts");

    let mut dangling = good.clone();
    dangling.steps[0].hints[0] = 77;
    assert!(lemmas(2, &clauses, &dangling).is_none(), "a hint naming no clause");

    let bogus = Lrat { steps: vec![LratStep { id: 5, clause: vec![], hints: vec![1, 2] }] };
    assert!(lemmas(2, &clauses, &bogus).is_none(), "under nothing assumed, (x₀ ∨ x₁) is not unit");

    assert!(lemmas(1, &clauses, &good).is_none(), "a literal beyond the declared variables");
}

/// The empty clause among the formula's own clauses is refuted directly by its hypothesis.
#[test]
fn an_empty_clause_in_the_formula_is_its_own_refutation() {
    refuted(2, &[vec![p(0), p(1)], vec![]], "the formula holding the empty clause");
}

/// The route verdicts that carry a clausal proof become kernel theorems without a fresh search: the proof
/// `solve::certified_proof` gives — 2-SAT's two units, Horn's empty stream, the parity and GF(3) refutations compiled
/// to resolution — is turned into a development of the formula itself, and both kernels check it.
#[test]
fn route_proofs_become_kernel_theorems() {
    use crate::solve::{certified_proof, solve_structured, Route};
    let (_, tseitin, _) = crate::families::tseitin_expander(8, 7);
    let (_, mod3, _) = crate::families::mod_p_tseitin_expander(6, 3, 5);
    let cases: Vec<(usize, Vec<Vec<Lit>>, Route)> = vec![
        (2, vec![vec![p(0), p(1)], vec![n(0), p(1)], vec![p(0), n(1)], vec![n(0), n(1)]], Route::TwoSat),
        (3, vec![vec![p(0)], vec![n(0), p(1)], vec![n(0), n(1), p(2)], vec![n(2), n(1)]], Route::Horn),
        (tseitin.num_vars, tseitin.clauses.clone(), Route::Parity),
        (mod3.num_vars, mod3.clauses.clone(), Route::ModP),
    ];
    for (nv, f, route) in cases {
        let solved = solve_structured(nv, &f);
        assert_eq!(solved.via, route, "the family is decided by the route under test");
        let proof = certified_proof(nv, &f, &solved).unwrap_or_else(|| panic!("{route:?} gives a clausal proof"));
        let (r, how) = certify(nv, &f, &proof).unwrap_or_else(|why| panic!("{route:?}: {why}"));
        assert_eq!(how, KernelCheck::TwoKernels, "{route:?}: both kernels check every lemma");
        assert_eq!((r.num_vars, &r.clauses), (nv, &f), "{route:?}: the refutation is of the formula itself");
        assert_eq!(check(&r), Ok(KernelCheck::TwoKernels), "{route:?}: re-checking gives the same verdict");
    }
}

/// A PR step preserves satisfiability but is no consequence of the formula, so no term proves it: a genuine PR
/// refutation is refused, naming the step, while a resolution proof of the same verdict is accepted.
#[test]
fn a_stream_with_a_pr_step_is_refused() {
    use crate::proof::Witness;
    let f = vec![vec![p(0), p(1)], vec![n(0), p(1)], vec![p(0), n(1)], vec![n(0), n(1)]];
    let pr = vec![ProofStep::Pr { clause: vec![p(0)], witness: Witness::Assignment(vec![p(0)]) }, ProofStep::Rup(vec![])];
    assert!(crate::pr::check_pr_refutation(2, &f, &pr), "a genuine PR refutation");
    let why = certify(2, &f, &pr).expect_err("a PR stream is no kernel theorem");
    assert!(why.contains("PR step"), "the refusal names the step: {why}");
    let rup = vec![ProofStep::Rup(vec![p(1)]), ProofStep::Rup(vec![])];
    let (_, how) = certify(2, &f, &rup).expect("the resolution proof of the same verdict is");
    assert_eq!(how, KernelCheck::TwoKernels);
}

/// A stream that does not refute is refused, never turned into a term: one after which unit propagation does not
/// conflict, one whose lemma is not RUP, and any stream for a satisfiable formula. As in DRAT and LRAT the empty clause
/// need not be written. Deletions are skipped, so a refutation that deletes a clause it no longer needs is still
/// accepted.
#[test]
fn certify_refuses_a_stream_that_does_not_refute() {
    let f = vec![vec![p(0), p(1)], vec![n(0), p(1)], vec![p(0), n(1)], vec![n(0), n(1)]];
    assert!(certify(2, &f, &[]).is_err(), "unit propagation alone does not refute the four clauses");
    assert!(certify(2, &f, &[ProofStep::Rup(vec![p(0), p(1)])]).is_err(), "after (x₀ ∨ x₁) nothing conflicts");
    assert!(certify(2, &f, &[ProofStep::Rup(vec![p(1)])]).is_ok(), "after (x₁) propagation conflicts: closed");
    // `x₀` forces `x₁ ∧ ¬x₁`, so `(¬x₀)` is RUP and the formula is unsatisfiable; `(x₀)` is not RUP, though the empty
    // clause would follow from it.
    let g = vec![
        vec![n(0), p(1)],
        vec![n(0), n(1)],
        vec![p(0), p(2), p(3)],
        vec![p(0), n(2), p(3)],
        vec![p(0), p(2), n(3)],
        vec![p(0), n(2), n(3)],
    ];
    assert!(certify(4, &g, &[ProofStep::Rup(vec![p(0)]), ProofStep::Rup(vec![])]).is_err(), "(x₀) is not RUP");
    assert!(certify(4, &g, &[ProofStep::Rup(vec![n(0)]), ProofStep::Rup(vec![p(2)]), ProofStep::Rup(vec![])]).is_ok(),
        "the refutation through (¬x₀) and (x₂) is");
    assert!(certify(2, &[vec![p(0), p(1)]], &[ProofStep::Rup(vec![n(0)]), ProofStep::Rup(vec![])]).is_err(), "a satisfiable formula");
    let with_deletion = vec![ProofStep::Rup(vec![p(1)]), ProofStep::Delete(vec![p(0), p(1)]), ProofStep::Rup(vec![])];
    assert!(certify(2, &f, &with_deletion).is_ok(), "a deletion is skipped");
}
