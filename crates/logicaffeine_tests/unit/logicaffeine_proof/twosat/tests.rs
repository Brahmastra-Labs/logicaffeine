use super::*;

fn cl(a: Lit, b: Lit) -> (Lit, Lit) {
    (a, b)
}

#[test]
fn simple_satisfiable_instance() {
    // (x0 ∨ x1) ∧ (¬x0 ∨ x1) ∧ (¬x1 ∨ x2) — satisfiable.
    let cs = vec![
        cl(Lit::pos(0), Lit::pos(1)),
        cl(Lit::neg(0), Lit::pos(1)),
        cl(Lit::neg(1), Lit::pos(2)),
    ];
    match solve(&cs, 3) {
        TwoSatOutcome::Sat(a) => assert!(satisfies(&cs, &a), "model must satisfy: {a:?}"),
        o => panic!("expected Sat, got {o:?}"),
    }
}

#[test]
fn forced_contradiction_is_unsat() {
    // (x0)∧(¬x0) as units: (x0∨x0) and (¬x0∨¬x0) ⇒ x0 forced both ways.
    let cs = vec![cl(Lit::pos(0), Lit::pos(0)), cl(Lit::neg(0), Lit::neg(0))];
    match solve(&cs, 1) {
        TwoSatOutcome::Unsat(v) => {
            assert_eq!(v, 0);
            assert!(is_refutation(&cs, 1, v), "refutation must re-check");
        }
        o => panic!("expected Unsat, got {o:?}"),
    }
}

#[test]
fn implication_cycle_is_unsat() {
    // x0→x1, x1→¬x0, ¬x0→x0 collapses {x0,¬x0,x1} — classic 2-SAT contradiction.
    // (¬x0∨x1)=x0→x1 ; (¬x1∨¬x0)=x1→¬x0 ; (x0∨x0)=¬x0→x0.
    let cs = vec![
        cl(Lit::neg(0), Lit::pos(1)),
        cl(Lit::neg(1), Lit::neg(0)),
        cl(Lit::pos(0), Lit::pos(0)),
    ];
    match solve(&cs, 2) {
        TwoSatOutcome::Unsat(v) => assert!(is_refutation(&cs, 2, v)),
        o => panic!("expected Unsat, got {o:?}"),
    }
}

#[test]
fn empty_is_satisfiable() {
    assert!(matches!(solve(&[], 3), TwoSatOutcome::Sat(_)));
}

#[test]
fn matches_brute_force_on_random_2sat() {
    let mut s: u64 = 0x2545F4914F6CDD1D;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..500 {
        let num_vars = (next() % 6) as usize + 1;
        let m = (next() % 10) as usize + 1;
        let lit = |r: u64, nv: usize| Lit {
            var: (r as usize) % nv,
            pos: (r >> 8) & 1 == 1,
        };
        let cs: Vec<(Lit, Lit)> = (0..m)
            .map(|_| (lit(next(), num_vars), lit(next(), num_vars)))
            .collect();
        let brute_sat = (0..(1u32 << num_vars)).any(|mask| {
            let a: Vec<bool> = (0..num_vars).map(|i| (mask >> i) & 1 == 1).collect();
            satisfies(&cs, &a)
        });
        match solve(&cs, num_vars) {
            TwoSatOutcome::Sat(a) => {
                assert!(brute_sat, "we said SAT, brute force UNSAT: {cs:?}");
                assert!(satisfies(&cs, &a), "model is wrong: {a:?} for {cs:?}");
            }
            TwoSatOutcome::Unsat(v) => {
                assert!(!brute_sat, "we said UNSAT, brute force SAT: {cs:?}");
                assert!(is_refutation(&cs, num_vars, v), "bogus refutation var={v}");
            }
        }
    }
}
