use super::*;

fn eq(vars: &[usize], rhs: bool) -> XorEquation {
    XorEquation::new(vars.to_vec(), rhs)
}

/// CNF-encode a list of XOR equations into a `ProofExpr` over atoms `x{v}`: each equation becomes
/// its `2^{k-1}` wrong-parity clauses (the inverse of [`refute_via_parity`]'s recognition).
fn xor_system_to_expr(system: &[(Vec<usize>, bool)]) -> crate::ProofExpr {
    use crate::ProofExpr;
    let lit = |v: usize, positive: bool| {
        let a = ProofExpr::Atom(format!("x{v}"));
        if positive {
            a
        } else {
            ProofExpr::Not(Box::new(a))
        }
    };
    let mut clauses = Vec::new();
    for (vars, rhs) in system {
        let k = vars.len();
        for mask in 0u32..(1 << k) {
            let parity = mask.count_ones() % 2 == 1;
            if parity != *rhs {
                let mut it = vars.iter().enumerate();
                let (i0, &v0) = it.next().unwrap();
                let first = lit(v0, mask & (1 << i0) == 0);
                let clause = it.fold(first, |acc, (i, &v)| {
                    ProofExpr::Or(Box::new(acc), Box::new(lit(v, mask & (1 << i) == 0)))
                });
                clauses.push(clause);
            }
        }
    }
    let mut it = clauses.into_iter();
    let first = it.next().expect("non-empty system");
    it.fold(first, |acc, c| {
        crate::ProofExpr::And(Box::new(acc), Box::new(c))
    })
}

/// The GF(2) shadow, stacked into the default prover: an inconsistent parity cover that resolution
/// refutes only exponentially is now decided in polynomial time by Gaussian elimination — recovered
/// straight from the CNF, with no CDCL search, and `prove_unsat` returns a certified `Refuted`.
#[test]
fn parity_shadow_refutes_inconsistent_xor_through_prove_unsat() {
    // x0⊕x1=1, x1⊕x2=1, x0⊕x2=1 — the three sum to 0 = 1, an odd cycle: UNSAT.
    let system = vec![(vec![0, 1], true), (vec![1, 2], true), (vec![0, 2], true)];
    let e = xor_system_to_expr(&system);
    assert!(
        refute_via_parity(&e),
        "the parity shadow must recognize and refute the odd cycle"
    );
    assert_eq!(
        crate::sat::prove_unsat(&e),
        crate::sat::UnsatOutcome::Refuted,
        "prove_unsat now routes the parity cover to Gaussian elimination"
    );
}

/// Soundness: a *satisfiable* parity cover is never falsely refuted — the shadow declines and the
/// prover finds a model.
#[test]
fn parity_shadow_is_sound_on_satisfiable_xor() {
    // x0⊕x1=1, x1⊕x2=1 — consistent (x0=0,x1=1,x2=0).
    let system = vec![(vec![0, 1], true), (vec![1, 2], true)];
    let e = xor_system_to_expr(&system);
    assert!(
        !refute_via_parity(&e),
        "a satisfiable parity system must not be refuted"
    );
    assert!(
        matches!(
            crate::sat::prove_unsat(&e),
            crate::sat::UnsatOutcome::Sat(_)
        ),
        "prove_unsat must find a model for the satisfiable parity cover"
    );
}

/// The shadows do not interfere: pigeonhole is a *counting* obstruction, not a parity one, so the
/// GF(2) recognizer declines it (leaving the counting shadow to decide it). No gadget, no guess.
#[test]
fn parity_shadow_declines_non_parity_pigeonhole() {
    use crate::cdcl::Lit;
    use crate::ProofExpr;
    let (cnf, _) = crate::families::php(4);
    let clause_expr = |c: &[Lit]| {
        let mut it = c.iter().map(|l| {
            let a = ProofExpr::Atom(format!("x{}", l.var()));
            if l.is_positive() {
                a
            } else {
                ProofExpr::Not(Box::new(a))
            }
        });
        let first = it.next().expect("non-empty clause");
        it.fold(first, |acc, l| ProofExpr::Or(Box::new(acc), Box::new(l)))
    };
    let mut it = cnf.clauses.iter().map(|c| clause_expr(c));
    let first = it.next().unwrap();
    let e = it.fold(first, |acc, c| ProofExpr::And(Box::new(acc), Box::new(c)));
    assert!(
        !refute_via_parity(&e),
        "pigeonhole is not a parity cover — the GF(2) shadow declines"
    );
}

#[test]
fn simple_consistent_system_is_solved() {
    // x0 ⊕ x1 = 1, x1 = 1  ⇒  x1 = 1, x0 = 0.
    let sys = vec![eq(&[0, 1], true), eq(&[1], true)];
    match solve(&sys, 2) {
        XorOutcome::Sat(a) => {
            assert!(satisfies(&sys, &a), "assignment must satisfy: {a:?}");
            assert_eq!(a, vec![false, true]);
        }
        o => panic!("expected Sat, got {o:?}"),
    }
}

#[test]
fn direct_contradiction_is_refuted() {
    // x0 ⊕ x1 = 0 and x0 ⊕ x1 = 1 — summing them gives 0 = 1.
    let sys = vec![eq(&[0, 1], false), eq(&[0, 1], true)];
    match solve(&sys, 2) {
        XorOutcome::Unsat(r) => {
            assert!(
                is_refutation(&sys, 2, &r),
                "refutation must re-check: {r:?}"
            );
            assert_eq!(r.len(), 2, "both equations are needed");
        }
        o => panic!("expected Unsat, got {o:?}"),
    }
}

#[test]
fn parity_chain_summing_to_one_is_refuted() {
    // x_i ⊕ x_{i+1} = 0 for a chain, plus x0 ⊕ x_{n-1} = 1 — all equal yet endpoints differ.
    let n = 8;
    let mut sys: Vec<XorEquation> = (0..n - 1).map(|i| eq(&[i, i + 1], false)).collect();
    sys.push(eq(&[0, n - 1], true));
    match solve(&sys, n) {
        XorOutcome::Unsat(r) => assert!(is_refutation(&sys, n, &r), "refutation invalid: {r:?}"),
        o => panic!("inconsistent chain must be Unsat, got {o:?}"),
    }
}

#[test]
fn duplicate_variables_cancel() {
    // x0 ⊕ x0 ⊕ x1 = 1  ≡  x1 = 1.
    let sys = vec![eq(&[0, 0, 1], true)];
    match solve(&sys, 2) {
        XorOutcome::Sat(a) => assert!(a[1], "x1 must be true: {a:?}"),
        o => panic!("expected Sat, got {o:?}"),
    }
}

#[test]
fn empty_system_is_trivially_sat() {
    assert!(matches!(solve(&[], 3), XorOutcome::Sat(_)));
}

#[test]
fn matches_brute_force_on_random_systems() {
    // Independent oracle: enumerate all 2^num_vars assignments; the system is SAT iff some
    // assignment satisfies every equation. Cross-check verdict + re-check every witness.
    let mut s: u64 = 0xD1B54A32D192ED03;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..400 {
        let num_vars = (next() % 6) as usize + 1; // 1..=6
        let m = (next() % 8) as usize + 1; // 1..=8 equations
        let sys: Vec<XorEquation> = (0..m)
            .map(|_| {
                let vars: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
                eq(&vars, next() % 2 == 0)
            })
            .collect();
        let brute_sat = (0..(1u32 << num_vars)).any(|mask| {
            let a: Vec<bool> = (0..num_vars).map(|i| (mask >> i) & 1 == 1).collect();
            satisfies(&sys, &a)
        });
        match solve(&sys, num_vars) {
            XorOutcome::Sat(a) => {
                assert!(brute_sat, "we said SAT but brute force says UNSAT: {sys:?}");
                assert!(satisfies(&sys, &a), "returned assignment is wrong: {a:?}");
            }
            XorOutcome::Unsat(r) => {
                assert!(
                    !brute_sat,
                    "we said UNSAT but brute force found a model: {sys:?}"
                );
                assert!(is_refutation(&sys, num_vars, &r), "bogus refutation {r:?}");
            }
        }
    }
}

#[test]
fn a_bad_refutation_is_rejected() {
    let sys = vec![eq(&[0, 1], false), eq(&[0, 1], true)];
    assert!(!is_refutation(&sys, 2, &[]), "empty is not a refutation");
    assert!(
        !is_refutation(&sys, 2, &[0]),
        "one consistent equation is not 0=1"
    );
    assert!(is_refutation(&sys, 2, &[0, 1]), "the pair sums to 0=1");
}

use crate::cdcl::Lit;

fn mix(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The gadgets by their definition: group the clauses of distinct in-range variables by variable set, and keep a set
/// of k ≤ `max_width` variables whose group is exactly 2^{k−1} clauses forbidding pairwise distinct assignments of one
/// parity, as the equation that parity forbids.
fn gadgets_by_grouping(num_vars: usize, clauses: &[Vec<Lit>], max_width: usize) -> Vec<XorEquation> {
    use std::collections::{HashMap, HashSet};
    let mut groups: HashMap<Vec<usize>, Vec<&Vec<Lit>>> = HashMap::new();
    for c in clauses {
        let mut vars: Vec<usize> = c.iter().map(|l| l.var() as usize).collect();
        vars.sort_unstable();
        vars.dedup();
        if vars.len() == c.len() && vars.iter().all(|&v| v < num_vars) {
            groups.entry(vars).or_default().push(c);
        }
    }
    let mut eqs = Vec::new();
    for (vars, group) in groups {
        let k = vars.len();
        if k == 0 || k > max_width || group.len() != 1 << (k - 1) {
            continue;
        }
        let forbidden: HashSet<u32> = group
            .iter()
            .map(|c| c.iter().filter(|l| !l.is_positive()).map(|l| 1u32 << vars.binary_search(&(l.var() as usize)).unwrap()).sum())
            .collect();
        let parities: HashSet<u32> = forbidden.iter().map(|a| a.count_ones() % 2).collect();
        if forbidden.len() == group.len() && parities.len() == 1 {
            let p = *parities.iter().next().unwrap();
            eqs.push(XorEquation::new(vars, p == 0));
        }
    }
    eqs
}

fn sorted(mut eqs: Vec<XorEquation>) -> Vec<XorEquation> {
    eqs.sort_by(|a, b| (&a.vars, a.rhs).cmp(&(&b.vars, b.rhs)));
    eqs
}

/// The clauses of `⊕ vars = rhs`: one forbidding each assignment of the wrong parity.
fn whole_gadget(vars: &[u32], rhs: bool) -> Vec<Vec<Lit>> {
    (0u32..1 << vars.len())
        .filter(|mask| (mask.count_ones() % 2 == 1) != rhs)
        .map(|mask| vars.iter().enumerate().map(|(i, &v)| Lit::new(v, mask & (1 << i) == 0)).collect())
        .collect()
}

/// A formula built to probe every rule of gadget recognition: whole gadgets of random equations (some shuffled
/// in, some of one variable), gadgets with a clause missing, duplicated, or carrying the wrong parity, two gadgets
/// of opposite parity on one variable set, clauses with a repeated or complementary literal, and noise clauses —
/// and in half the formulas an inconsistent system: two whole gadgets and a third over their variables' symmetric
/// difference whose parity contradicts their sum.
fn gadget_soup(next: &mut impl FnMut() -> u64, num_vars: usize) -> Vec<Vec<Lit>> {
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    if num_vars >= 2 && next() % 2 == 0 {
        let mut pick = |next: &mut dyn FnMut() -> u64| -> Vec<u32> {
            let k = 1 + (next() % 3.min(num_vars as u64)) as usize;
            let mut vars: Vec<u32> = Vec::new();
            while vars.len() < k {
                let v = (next() % num_vars as u64) as u32;
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            vars
        };
        let (a, b) = (pick(&mut *next), pick(&mut *next));
        let (ra, rb) = (next() & 1 == 0, next() & 1 == 0);
        let mut sum: Vec<u32> = a.iter().copied().filter(|v| !b.contains(v)).chain(b.iter().copied().filter(|v| !a.contains(v))).collect();
        sum.sort_unstable();
        if !sum.is_empty() {
            clauses.extend(whole_gadget(&a, ra));
            clauses.extend(whole_gadget(&b, rb));
            clauses.extend(whole_gadget(&sum, !(ra ^ rb)));
        }
    }
    let pieces = 1 + next() % 8;
    for _ in 0..pieces {
        let k = 1 + (next() % 5.min(num_vars as u64)) as usize;
        let mut vars: Vec<u32> = Vec::new();
        while vars.len() < k {
            let v = (next() % num_vars as u64) as u32;
            if !vars.contains(&v) {
                vars.push(v);
            }
        }
        let rhs = next() & 1 == 0;
        let mut gadget: Vec<Vec<Lit>> = (0u32..1 << k)
            .filter(|mask| (mask.count_ones() % 2 == 1) != rhs)
            .map(|mask| vars.iter().enumerate().map(|(i, &v)| Lit::new(v, mask & (1 << i) == 0)).collect())
            .collect();
        match next() % 8 {
            0 if gadget.len() > 1 => {
                gadget.pop();
            }
            1 => {
                let c = gadget[0].clone();
                gadget.push(c);
            }
            2 => {
                let c = gadget.pop().unwrap();
                gadget.push(c.iter().enumerate().map(|(i, l)| if i == 0 { l.negated() } else { *l }).collect());
            }
            3 => {
                let opposite: Vec<Vec<Lit>> = gadget.iter().map(|c| c.iter().enumerate().map(|(i, l)| if i == 0 { l.negated() } else { *l }).collect()).collect();
                gadget.extend(opposite);
            }
            _ => {}
        }
        clauses.extend(gadget);
    }
    for _ in 0..next() % 6 {
        let width = 1 + (next() % 4) as usize;
        let mut c: Vec<Lit> = (0..width).map(|_| Lit::new((next() % num_vars as u64) as u32, next() & 1 == 0)).collect();
        if next() % 4 == 0 {
            c.push(c[0]);
        } else if next() % 4 == 0 {
            c.push(c[0].negated());
        }
        clauses.push(c);
    }
    for i in (1..clauses.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        clauses.swap(i, j);
    }
    for c in clauses.iter_mut() {
        for i in (1..c.len()).rev() {
            let j = (next() % (i as u64 + 1)) as usize;
            c.swap(i, j);
        }
    }
    clauses
}

/// The gadget extraction is its definition at every width bound, on formulas that probe each of its rules.
#[test]
fn xor_gadgets_are_the_grouping_definition() {
    let mut next = mix(0x6AD6);
    let mut recovered = 0usize;
    for round in 0..3000u64 {
        let num_vars = 1 + (next() % 10) as usize;
        let clauses = gadget_soup(&mut next, num_vars);
        for max_width in [1usize, 2, 3, 12, 31] {
            let got = xor_gadgets(num_vars, &clauses, max_width);
            assert_eq!(sorted(got.clone()), sorted(gadgets_by_grouping(num_vars, &clauses, max_width)), "round {round}, width ≤ {max_width}");
            recovered += got.len();
        }
        let in_range = num_vars.saturating_sub(2).max(1);
        assert_eq!(sorted(xor_gadgets(in_range, &clauses, 12)), sorted(gadgets_by_grouping(in_range, &clauses, 12)), "round {round}: a clause over a variable past the range is no gadget's");
    }
    assert!(recovered >= 10_000, "the soup carries whole gadgets: {recovered}");
}

fn clauses_to_expr(clauses: &[Vec<Lit>]) -> Option<crate::ProofExpr> {
    use crate::ProofExpr;
    let clause = |c: &Vec<Lit>| {
        c.iter()
            .map(|l| {
                let a = ProofExpr::Atom(format!("v{}", l.var()));
                if l.is_positive() { a } else { ProofExpr::Not(Box::new(a)) }
            })
            .reduce(|a, b| ProofExpr::Or(Box::new(a), Box::new(b)))
    };
    clauses.iter().map(clause).collect::<Option<Vec<_>>>()?.into_iter().reduce(|a, b| ProofExpr::And(Box::new(a), Box::new(b)))
}

/// Refuting the clauses directly decides exactly what the `ProofExpr` route decides, refuting and declining alike.
#[test]
fn refuting_the_clauses_agrees_with_the_proof_expr_route() {
    let mut next = mix(0x9A71);
    let (mut refuted, mut declined) = (0usize, 0usize);
    for round in 0..2500u64 {
        let num_vars = 1 + (next() % 8) as usize;
        let clauses = gadget_soup(&mut next, num_vars);
        let Some(expr) = clauses_to_expr(&clauses) else { continue };
        let direct = refute_via_parity_clauses(num_vars, &clauses);
        assert_eq!(direct, refute_via_parity(&expr), "round {round}");
        if direct {
            refuted += 1;
        } else {
            declined += 1;
        }
    }
    assert!(refuted >= 100 && declined >= 100, "both verdicts are exercised: {refuted} refuted, {declined} declined");
}

/// A refutation by clauses is a refutation: brute force finds no model of a formula it refutes.
#[test]
fn a_clause_parity_refutation_is_never_wrong() {
    let mut next = mix(0x50D);
    let mut refuted = 0usize;
    for round in 0..2500u64 {
        let num_vars = 1 + (next() % 10) as usize;
        let clauses = gadget_soup(&mut next, num_vars);
        if refute_via_parity_clauses(num_vars, &clauses) {
            refuted += 1;
            let model = (0u32..1 << num_vars).find(|bits| clauses.iter().all(|c| c.iter().any(|l| (bits >> l.var() & 1 == 1) == l.is_positive())));
            assert_eq!(model, None, "round {round}: a refuted formula has no model");
        }
    }
    assert!(refuted >= 100, "{refuted} refutations");
}

/// Gauss–Jordan elimination over dense bit rows, as the solver was first written: whether the system is consistent,
/// and its rank. The reference for the sparse eliminator.
fn dense_consistency_and_rank(equations: &[XorEquation], num_vars: usize) -> (bool, usize) {
    let mut rows: Vec<(Vec<bool>, bool)> = equations
        .iter()
        .map(|eq| {
            let mut lhs = vec![false; num_vars];
            for &v in &eq.vars {
                if v < num_vars {
                    lhs[v] ^= true;
                }
            }
            (lhs, eq.rhs)
        })
        .collect();
    let mut rank = 0;
    for c in 0..num_vars {
        let Some(p) = (rank..rows.len()).find(|&i| rows[i].0[c]) else { continue };
        rows.swap(rank, p);
        let pivot = rows[rank].clone();
        for (i, row) in rows.iter_mut().enumerate() {
            if i != rank && row.0[c] {
                for (a, b) in row.0.iter_mut().zip(&pivot.0) {
                    *a ^= *b;
                }
                row.1 ^= pivot.1;
            }
        }
        rank += 1;
    }
    (rows.iter().all(|(lhs, rhs)| lhs.iter().any(|&b| b) || !rhs), rank)
}

/// Parity systems of every shape the eliminator meets: equations of no to six variables (most of one to three), a
/// variable sometimes repeated or past `num_vars`, and runs of chained equivalences and gates that close on
/// themselves.
fn random_system(next: &mut impl FnMut() -> u64, num_vars: usize, m: usize) -> Vec<XorEquation> {
    let mut eqs = Vec::with_capacity(m);
    while eqs.len() < m {
        if next() % 6 == 0 {
            let len = 2 + (next() % 6) as usize;
            let start = (next() % num_vars as u64) as usize;
            for i in 0..len {
                let (a, b) = ((start + i) % num_vars, (start + i + 1) % num_vars);
                let mut vars = vec![a, b];
                if next() % 2 == 0 {
                    vars.push((next() % num_vars as u64) as usize);
                }
                eqs.push(XorEquation::new(vars, next() & 1 == 0));
            }
            continue;
        }
        let width = [0usize, 1, 1, 2, 2, 2, 3, 3, 3, 4, 5, 6][(next() % 12) as usize];
        let mut vars: Vec<usize> = (0..width).map(|_| (next() % num_vars as u64) as usize).collect();
        if !vars.is_empty() && next() % 10 == 0 {
            vars.push(vars[0]);
        }
        if next() % 12 == 0 {
            vars.push(num_vars + (next() % 3) as usize);
        }
        eqs.push(XorEquation::new(vars, next() & 1 == 0));
    }
    eqs.truncate(m);
    eqs
}

/// The eliminator decides every system as dense Gauss–Jordan does, at the same rank: a consistent one with an
/// assignment of every variable that satisfies each equation, an inconsistent one with a refutation that re-checks.
#[test]
fn elimination_decides_as_dense_gauss_jordan_at_its_rank() {
    let mut next = mix(0x6A055);
    let (mut sat, mut unsat) = (0usize, 0usize);
    for round in 0..5000u64 {
        let num_vars = 1 + (next() % 30) as usize;
        let m = (next() % 50) as usize;
        let eqs = random_system(&mut next, num_vars, m);
        let (consistent, dense_rank) = dense_consistency_and_rank(&eqs, num_vars);
        assert_eq!(rank(&eqs, num_vars), dense_rank, "round {round}: n={num_vars} m={m}");
        match solve(&eqs, num_vars) {
            XorOutcome::Sat(assignment) => {
                assert!(consistent, "round {round}: a satisfiable verdict on an inconsistent system");
                assert_eq!(assignment.len(), num_vars, "round {round}");
                assert!(satisfies(&eqs, &assignment), "round {round}: the assignment satisfies every equation");
                sat += 1;
            }
            XorOutcome::Unsat(refutation) => {
                assert!(!consistent, "round {round}: a refutation of a consistent system");
                assert!(is_refutation(&eqs, num_vars, &refutation), "round {round}: the refutation re-checks");
                assert!(refutation.windows(2).all(|w| w[0] < w[1]), "round {round}: a refutation names each equation once, in order");
                unsat += 1;
            }
        }
    }
    assert!(sat >= 1000 && unsat >= 1000, "both verdicts are exercised: {sat} consistent, {unsat} inconsistent");
}

/// A dense random system fills in under elimination — the dense phase's ground — and is still decided as Gauss–Jordan
/// decides it.
#[test]
fn a_system_that_fills_in_is_decided_as_dense_gauss_jordan() {
    let mut next = mix(0xF111);
    for round in 0..6u64 {
        let num_vars = 150 + (next() % 100) as usize;
        let m = num_vars - 20 + (next() % 40) as usize;
        let eqs: Vec<XorEquation> = (0..m)
            .map(|_| XorEquation::new((0..3 + next() % 4).map(|_| (next() % num_vars as u64) as usize).collect::<Vec<usize>>(), next() & 1 == 0))
            .collect();
        let (consistent, dense_rank) = dense_consistency_and_rank(&eqs, num_vars);
        assert_eq!(rank(&eqs, num_vars), dense_rank, "round {round}");
        match solve(&eqs, num_vars) {
            XorOutcome::Sat(a) => assert!(consistent && satisfies(&eqs, &a), "round {round}"),
            XorOutcome::Unsat(r) => assert!(!consistent && is_refutation(&eqs, num_vars, &r), "round {round}"),
        }
    }
}

fn within_a_minute<T: Send + 'static>(what: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap_or_else(|_| panic!("{what}"))
}

/// Elimination costs what the system's sparsity allows, not the square of its size: a 300,000-equation chain of
/// equivalences closed into a contradiction, and a 200,000-gate parity circuit with its output pinned, each
/// ~10¹⁶ dense steps, are decided — refuted with a re-checked refutation, solved with a checked assignment — within a
/// minute unoptimized on a loaded box.
#[test]
fn elimination_costs_the_sparsity_of_the_system() {
    let chain = within_a_minute("a chain of equivalences is eliminated along the chain", || {
        let n = 300_000usize;
        let mut eqs: Vec<XorEquation> = (0..n - 1).map(|i| XorEquation::new(vec![i, i + 1], i % 3 == 0)).collect();
        let parity = (0..n - 1).filter(|i| i % 3 == 0).count() % 2 == 1;
        eqs.push(XorEquation::new(vec![0, n - 1], !parity));
        let outcome = solve(&eqs, n);
        (eqs, n, outcome)
    });
    let (eqs, n, outcome) = chain;
    match outcome {
        XorOutcome::Unsat(r) => assert!(is_refutation(&eqs, n, &r), "the chain's refutation re-checks"),
        XorOutcome::Sat(_) => panic!("the closed chain is inconsistent"),
    }
    let (eqs, n, outcome) = within_a_minute("a parity circuit is eliminated gate by gate", || {
        let gates = 200_000usize;
        // g_i = g_{i−1} ⊕ x_i over g_0 … g_gates (variables 0 … gates) and x_1 … x_gates (gates+1 …), the output pinned.
        let x = |i: usize| gates + i;
        let mut eqs: Vec<XorEquation> = (1..=gates).map(|i| XorEquation::new(vec![i, i - 1, x(i)], false)).collect();
        eqs.push(XorEquation::new(vec![0], false));
        eqs.push(XorEquation::new(vec![gates], true));
        let n = 2 * gates + 1;
        let outcome = solve(&eqs, n);
        (eqs, n, outcome)
    });
    match outcome {
        XorOutcome::Sat(a) => assert!(satisfies(&eqs, &a), "the circuit's assignment satisfies every gate"),
        XorOutcome::Unsat(_) => panic!("the circuit is consistent"),
    }
    assert_eq!(within_a_minute("ranking the circuit costs its sparsity", move || rank(&eqs, n)), 200_002);
}
