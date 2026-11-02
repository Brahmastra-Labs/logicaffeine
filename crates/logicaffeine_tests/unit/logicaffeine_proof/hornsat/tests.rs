use super::*;

#[test]
fn facts_and_rules_chain_to_a_least_model() {
    // ⇒a, ⇒b, (a∧b)⇒c, c⇒d  ⇒  {a,b,c,d} all true.
    let cs = vec![
        HornClause::fact(0),
        HornClause::fact(1),
        HornClause::rule([0, 1], 2),
        HornClause::rule([2], 3),
    ];
    match solve(&cs, 4) {
        HornOutcome::Sat(m) => {
            assert_eq!(m, vec![true, true, true, true]);
            assert!(satisfies(&cs, &m));
        }
        o => panic!("expected Sat, got {o:?}"),
    }
}

#[test]
fn least_model_leaves_unforced_variables_false() {
    // Only a is a fact; b is never forced ⇒ least model {a}, not {a,b}.
    let cs = vec![HornClause::fact(0), HornClause::rule([1], 0)];
    match solve(&cs, 2) {
        HornOutcome::Sat(m) => {
            assert_eq!(m, vec![true, false]);
            assert!(satisfies(&cs, &m));
        }
        o => panic!("expected Sat, got {o:?}"),
    }
}

#[test]
fn forced_goal_is_refuted_with_a_derivation() {
    // ⇒a, a⇒b, (a∧b)⇒false  — the goal's body is forced, so UNSAT.
    let cs = vec![
        HornClause::fact(0),
        HornClause::rule([0], 1),
        HornClause::goal([0, 1]),
    ];
    match solve(&cs, 2) {
        HornOutcome::Unsat(r) => {
            assert!(is_refutation(&cs, 2, &r), "refutation must re-check: {r:?}");
        }
        o => panic!("expected Unsat, got {o:?}"),
    }
}

#[test]
fn an_unforced_goal_is_satisfiable() {
    // The goal needs b, which is never forced ⇒ SAT (least model {a}).
    let cs = vec![HornClause::fact(0), HornClause::goal([0, 1])];
    match solve(&cs, 2) {
        HornOutcome::Sat(m) => assert!(satisfies(&cs, &m)),
        o => panic!("expected Sat, got {o:?}"),
    }
}

#[test]
fn matches_brute_force_on_random_horn_systems() {
    let mut s: u64 = 0xA0761D6478BD642F;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..400 {
        let num_vars = (next() % 6) as usize + 1;
        let m = (next() % 8) as usize + 1;
        let cs: Vec<HornClause> = (0..m)
            .map(|_| {
                let body: Vec<usize> = (0..num_vars).filter(|_| next() % 3 == 0).collect();
                // ~1/4 goal clauses, else a definite rule with a random head.
                if next() % 4 == 0 {
                    HornClause::goal(body)
                } else {
                    HornClause::rule(body, (next() as usize) % num_vars)
                }
            })
            .collect();
        let brute_sat = (0..(1u32 << num_vars)).any(|mask| {
            let a: Vec<bool> = (0..num_vars).map(|i| (mask >> i) & 1 == 1).collect();
            satisfies(&cs, &a)
        });
        match solve(&cs, num_vars) {
            HornOutcome::Sat(m) => {
                assert!(brute_sat, "we said SAT, brute force UNSAT: {cs:?}");
                assert!(satisfies(&cs, &m), "least model is wrong: {m:?}");
            }
            HornOutcome::Unsat(r) => {
                assert!(!brute_sat, "we said UNSAT, brute force SAT: {cs:?}");
                assert!(is_refutation(&cs, num_vars, &r), "bogus refutation {r:?}");
            }
        }
    }
}
