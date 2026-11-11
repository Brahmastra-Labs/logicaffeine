use super::*;
use crate::lrat::{LratStep, Lrat};

fn p(v: u32) -> Lit {
    Lit::pos(v)
}
fn n(v: u32) -> Lit {
    Lit::neg(v)
}

/// The value of a clause set under a full assignment.
fn holds(clauses: &[Vec<Lit>], x: &[bool]) -> bool {
    clauses.iter().all(|c| c.iter().any(|l| x[l.var() as usize] == l.is_positive()))
}

fn assignment(bits: u64, num_vars: usize) -> Vec<bool> {
    (0..num_vars).map(|v| bits >> v & 1 == 1).collect()
}

/// Every defining property of a Craig interpolant, by enumeration of all `2^num_vars` assignments: `A ⊨ I`,
/// `I ∧ B ⊨ ⊥`, and the circuit's inputs are exactly the shared variables.
fn assert_interpolant(num_vars: usize, a: &[Vec<Lit>], b: &[Vec<Lit>], itp: &Interpolant, what: &str) {
    let locality = localities(num_vars, a, b);
    let shared: Vec<Var> = (0..num_vars as Var).filter(|&v| locality[v as usize] == Locality::Shared).collect();
    assert_eq!(itp.shared, shared, "{what}: the circuit's inputs are exactly the shared variables");
    assert_eq!(itp.aig.inputs().len(), shared.len(), "{what}: one input per shared variable");
    for bits in 0..1u64 << num_vars {
        let x = assignment(bits, num_vars);
        let i = itp.eval(&x);
        if holds(a, &x) {
            assert!(i, "{what}: A ⊨ I fails at {x:?}");
        }
        if holds(b, &x) {
            assert!(!i, "{what}: I ∧ B ⊨ ⊥ fails at {x:?}");
        }
    }
}

fn expect_interpolant(num_vars: usize, a: &[Vec<Lit>], b: &[Vec<Lit>], what: &str) -> Interpolant {
    match interpolate(num_vars, a, b) {
        Interpolation::Interpolant(itp) => {
            assert_interpolant(num_vars, a, b, &itp, what);
            itp
        }
        other => panic!("{what}: an unsatisfiable A ∧ B interpolates, got {other:?}"),
    }
}

/// The textbook case: `A = a ∧ (¬a ∨ b)`, `B = ¬b`. The shared vocabulary is `{b}`, and the only interpolant over
/// it is `b` itself.
#[test]
fn the_textbook_interpolant_is_the_shared_variable() {
    let a = vec![vec![p(0)], vec![n(0), p(1)]];
    let b = vec![vec![n(1)]];
    let itp = expect_interpolant(2, &a, &b, "textbook");
    assert_eq!(itp.shared, vec![1]);
    assert!(itp.eval(&[false, true]), "b = 1 ⟹ I");
    assert!(!itp.eval(&[false, false]), "b = 0 ⟹ ¬I");
}

/// A deterministic generator: `m` clauses of width 2–3 over `num_vars` variables, each clause on the `A` side or
/// the `B` side. `A` draws its variables from the lower two thirds and `B` from the upper two thirds, so every
/// instance has `A`-local, `B`-local and shared variables.
fn random_split(seed: u64, num_vars: usize, m: usize) -> (Vec<Vec<Lit>>, Vec<Vec<Lit>>) {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let third = num_vars.div_ceil(3);
    for _ in 0..m {
        let to_a = next() & 1 == 0;
        let (lo, hi) = if to_a { (0, num_vars - third) } else { (third, num_vars) };
        let width = (2 + (next() % 2) as usize).min(hi - lo);
        let mut c: Vec<Lit> = Vec::new();
        while c.len() < width {
            let v = lo as u32 + (next() % (hi - lo) as u64) as u32;
            if c.iter().any(|l| l.var() == v) {
                continue;
            }
            c.push(Lit::new(v, next() & 1 == 0));
        }
        if to_a {
            a.push(c);
        } else {
            b.push(c);
        }
    }
    (a, b)
}

/// The interpolant is checked against the definition on every unsatisfiable random split, by enumeration: `A ⊨ I`,
/// `I ∧ B ⊨ ⊥`, inputs exactly the shared variables. The sweep must meet enough unsatisfiable instances to mean
/// something, and on the satisfiable ones the reported model satisfies both sides.
#[test]
fn random_splits_interpolate_by_the_definition() {
    let mut unsat = 0;
    let mut nontrivial = 0;
    let mut sat = 0;
    for seed in 0..400u64 {
        let num_vars = 4 + (seed % 7) as usize;
        let m = num_vars * 4 + (seed % 5) as usize;
        let (a, b) = random_split(seed, num_vars, m);
        let models = |cs: &[Vec<Lit>]| (0..1u64 << num_vars).any(|bits| holds(cs, &assignment(bits, num_vars)));
        let brute_sat = (0..1u64 << num_vars).any(|bits| {
            let x = assignment(bits, num_vars);
            holds(&a, &x) && holds(&b, &x)
        });
        match interpolate(num_vars, &a, &b) {
            Interpolation::Interpolant(itp) => {
                assert!(!brute_sat, "seed {seed}: an interpolant was produced for a satisfiable A ∧ B");
                assert_interpolant(num_vars, &a, &b, &itp, &format!("seed {seed}"));
                unsat += 1;
                if models(&a) && models(&b) {
                    nontrivial += 1;
                }
            }
            Interpolation::Satisfiable(model) => {
                assert!(brute_sat, "seed {seed}: a model was reported for an unsatisfiable A ∧ B");
                assert!(holds(&a, &model) && holds(&b, &model), "seed {seed}: the model satisfies A and B");
                sat += 1;
            }
            Interpolation::Unknown => panic!("seed {seed}: every refutation of the solver interpolates"),
        }
    }
    assert!(unsat >= 150, "the sweep met {unsat} unsatisfiable splits — too few to mean anything");
    assert!(
        nontrivial >= 40,
        "only {nontrivial} splits had both sides satisfiable alone — the interpolant was rarely forced to be nontrivial"
    );
    assert!(sat >= 20, "the sweep met {sat} satisfiable splits — the SAT side went unexercised");
}

/// A side that is contradictory on its own: an empty clause in `A` makes the interpolant false, one in `B` makes it
/// true; an unsatisfiable `A` with a satisfiable `B` forces `I ≡ false`, and the mirror forces `I` true on `A`.
#[test]
fn a_side_contradictory_alone_fixes_the_interpolant() {
    let itp = expect_interpolant(2, &[vec![], vec![p(0), p(1)]], &[vec![n(1)]], "empty clause in A");
    for bits in 0..4 {
        assert!(!itp.eval(&assignment(bits, 2)), "an empty A clause gives I ≡ false");
    }
    let itp = expect_interpolant(2, &[vec![p(0), p(1)]], &[vec![], vec![n(1)]], "empty clause in B");
    for bits in 0..4 {
        let x = assignment(bits, 2);
        if x[0] || x[1] {
            assert!(itp.eval(&x), "an empty B clause gives I true on A");
        }
    }
    let a = vec![vec![p(0), p(1)], vec![n(0), p(1)], vec![n(1)]];
    let b = vec![vec![p(1), p(2)]];
    let itp = expect_interpolant(3, &a, &b, "A unsatisfiable alone");
    for bits in 0..8 {
        if holds(&b, &assignment(bits, 3)) {
            assert!(!itp.eval(&assignment(bits, 3)), "B satisfiable ⟹ I false on B's models");
        }
    }
    let a = vec![vec![p(0), p(1)]];
    let b = vec![vec![p(1), p(2)], vec![n(1), p(2)], vec![n(2)]];
    expect_interpolant(3, &a, &b, "B unsatisfiable alone");
}

/// Pigeonhole split along its two halves — the pigeons' at-least-one rows in `A`, the holes' at-most-one pairs in
/// `B`. Every variable is shared, so the interpolant lives on the whole cube; it is checked there.
#[test]
fn pigeonhole_rows_against_holes_interpolate() {
    for m in [2usize, 3] {
        let (php, _) = crate::families::php(m);
        let (a, b): (Vec<Vec<Lit>>, Vec<Vec<Lit>>) =
            php.clauses.iter().cloned().partition(|c| c.iter().all(|l| l.is_positive()));
        let itp = expect_interpolant(php.num_vars, &a, &b, &format!("PHP({m})"));
        assert_eq!(itp.shared.len(), php.num_vars, "PHP({m}): rows and holes share every variable");
    }
}

/// The Tseitin encoding agrees with the circuit: with the formula variables fixed, the encoded clauses plus the
/// returned literal are satisfiable exactly when the interpolant evaluates true.
#[test]
fn the_encoded_interpolant_agrees_with_its_circuit() {
    let mut checked = 0;
    for seed in 0..40u64 {
        let num_vars = 7;
        let (a, b) = random_split(seed, num_vars, 36);
        let Interpolation::Interpolant(itp) = interpolate(num_vars, &a, &b) else { continue };
        checked += 1;
        if checked > 6 {
            break;
        }
        let mut clauses = Vec::new();
        let mut next_var = num_vars;
        let root = itp.encode(&mut clauses, &mut next_var);
        for bits in 0..1u64 << num_vars {
            let x = assignment(bits, num_vars);
            for (want, lit) in [(true, root), (false, root.negated())] {
                let mut s = Solver::new(next_var);
                for c in &clauses {
                    s.add_clause(c.clone());
                }
                for v in 0..num_vars {
                    s.add_clause(vec![Lit::new(v as u32, x[v])]);
                }
                s.add_clause(vec![lit]);
                let sat = matches!(s.solve(), SolveResult::Sat(_));
                assert_eq!(sat, itp.eval(&x) == want, "seed {seed}: encoding vs circuit at {x:?}");
            }
        }
    }
    assert!(checked >= 5, "only {checked} interpolants were encoded and compared");
}

/// The trimmed LRAT refutation of `a ++ b` the solver's learned clauses give.
fn refutation_of(num_vars: usize, a: &[Vec<Lit>], b: &[Vec<Lit>]) -> Lrat {
    let mut s = Solver::new(num_vars);
    for c in a.iter().chain(b) {
        s.add_clause(c.clone());
    }
    assert_eq!(s.solve(), SolveResult::Unsat);
    let original: Vec<Vec<Lit>> = a.iter().chain(b).cloned().collect();
    let learned: Vec<Vec<Lit>> = s.learned().iter().map(|c| c.lits.clone()).collect();
    crate::lrat::refutation(num_vars, &original, &learned).expect("the solver's refutation trims to LRAT")
}

/// A proof that does not refute is no interpolant: a step whose conflict hint is not falsified, a step claiming a
/// clause its chain does not reach, a hint naming no clause, a step out of id order, and a proof that never derives
/// the empty clause are each refused — `None`, never a formula.
#[test]
fn a_proof_that_does_not_refute_yields_no_interpolant() {
    let a = vec![vec![p(0), p(1)], vec![n(0), p(1)]];
    let b = vec![vec![n(1), p(2)], vec![n(1), n(2)]];
    let good = refutation_of(3, &a, &b);
    assert!(interpolant(3, &a, &b, &good).is_some(), "the untouched proof interpolates");

    let mut no_conflict = good.clone();
    let last = no_conflict.steps.last_mut().unwrap();
    assert!(last.hints.len() >= 2, "deriving ⊥ from no assumption takes a unit and a conflict");
    last.hints.truncate(1);
    assert!(interpolant(3, &a, &b, &no_conflict).is_none(), "a chain that never conflicts is refused");

    let mut dangling = good.clone();
    dangling.steps[0].hints[0] = 999;
    assert!(interpolant(3, &a, &b, &dangling).is_none(), "a hint naming no clause is refused");

    let mut truncated = good.clone();
    truncated.steps.pop();
    assert!(interpolant(3, &a, &b, &truncated).is_none(), "a proof without the empty clause is refused");

    let mut misordered = good.clone();
    misordered.steps[0].id = 1;
    assert!(interpolant(3, &a, &b, &misordered).is_none(), "a step id at or below the formula's is refused");

    // The empty clause claimed from one hint that, with nothing assumed, is neither unit nor a conflict.
    let overclaim = Lrat {
        steps: vec![LratStep { id: 5, clause: vec![], hints: vec![1] }],
    };
    assert!(interpolant(3, &a, &b, &overclaim).is_none(), "an empty clause no chain derives is refused");
}

/// Literals beyond the declared variables are refused rather than read out of bounds.
#[test]
fn literals_beyond_the_variables_are_refused() {
    let a = vec![vec![p(5)]];
    let b = vec![vec![n(5)]];
    let proof = Lrat { steps: vec![LratStep { id: 3, clause: vec![], hints: vec![1, 2] }] };
    assert!(interpolant(2, &a, &b, &proof).is_none());
    assert!(interpolant(6, &a, &b, &proof).is_some(), "with the variable declared, the same proof interpolates");
}
