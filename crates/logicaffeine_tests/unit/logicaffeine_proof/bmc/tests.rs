use super::*;

fn atom(s: &str) -> ProofExpr {
    ProofExpr::Atom(s.to_string())
}
fn iff(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Iff(Box::new(a), Box::new(b))
}

// ── A latched-true register: x@0 = true, x@(t+1) ↔ x@t. Property: x is always true.
fn latched_init() -> ProofExpr {
    atom("x@0")
}
fn latched_trans(t: u32) -> ProofExpr {
    iff(atom(&format!("x@{}", t + 1)), atom(&format!("x@{}", t)))
}
fn latched_prop(t: u32) -> ProofExpr {
    atom(&format!("x@{}", t))
}

// ── A toggle: q@0 = false, q@(t+1) ↔ ¬q@t. The (false) property "q is always false".
fn toggle_init() -> ProofExpr {
    not(atom("q@0"))
}
fn toggle_trans(t: u32) -> ProofExpr {
    iff(
        atom(&format!("q@{}", t + 1)),
        not(atom(&format!("q@{}", t))),
    )
}
fn toggle_always_false(t: u32) -> ProofExpr {
    not(atom(&format!("q@{}", t)))
}

#[test]
fn bmc_finds_toggle_violation_at_step_one() {
    // "q always false" is violated: q@1 = ¬q@0 = true. BMC must find it at depth 1.
    let out = find_counterexample(&toggle_init(), &toggle_trans, &toggle_always_false, 5);
    match out {
        BmcOutcome::CounterexampleAt { k, trace } => {
            assert_eq!(k, 1, "shallowest violation is at step 1");
            assert!(
                trace.iter().any(|(n, v)| n == "q@1" && *v),
                "trace must show q@1 high: {trace:?}"
            );
        }
        other => panic!("expected a counterexample, got {other:?}"),
    }
}

#[test]
fn bmc_no_counterexample_for_latched_invariant() {
    let out = find_counterexample(&latched_init(), &latched_trans, &latched_prop, 6);
    assert_eq!(out, BmcOutcome::NoneWithin(6));
}

#[test]
fn k_induction_proves_latched_invariant() {
    // x is latched true and starts true ⇒ always true. 1-inductive.
    let out = prove_invariant(&latched_init(), &latched_trans, &latched_prop, 1);
    assert_eq!(out, InductionOutcome::Proven);
}

#[test]
fn k_induction_finds_toggle_counterexample_in_base() {
    // "q always false" is false; the base case must surface the violation, not "Proven".
    let out = prove_invariant(&toggle_init(), &toggle_trans, &toggle_always_false, 3);
    match out {
        InductionOutcome::CounterexampleAt { k, .. } => assert_eq!(k, 1),
        other => panic!("expected a base-case counterexample, got {other:?}"),
    }
}

#[test]
fn vacuity_detects_dead_trigger() {
    // An antecedent that contradicts itself never fires → vacuous.
    let dead = ProofExpr::And(Box::new(atom("req@0")), Box::new(not(atom("req@0"))));
    assert_eq!(check_vacuity(&dead), VacuityOutcome::Vacuous);
}

#[test]
fn vacuity_accepts_a_live_trigger() {
    match check_vacuity(&atom("req@0")) {
        VacuityOutcome::Reachable(w) => {
            assert!(w.iter().any(|(n, v)| n == "req@0" && *v));
        }
        other => panic!("a live trigger must be Reachable, got {other:?}"),
    }
}

// ── Incremental BMC (reuses learned clauses across depths via assumptions). ──

#[test]
fn incremental_bmc_finds_toggle_violation_at_step_one() {
    match find_counterexample_incremental(&toggle_init(), &toggle_trans, &toggle_always_false, 5) {
        BmcOutcome::CounterexampleAt { k, trace } => {
            assert_eq!(k, 1);
            assert!(
                trace.iter().any(|(n, v)| n == "q@1" && *v),
                "trace: {trace:?}"
            );
        }
        other => panic!("expected a counterexample, got {other:?}"),
    }
}

#[test]
fn temporal_symmetry_detected_and_broken_in_bmc() {
    // Two interchangeable latches s0, s1. init: at least one on. property: not both on. The system is
    // symmetric under swapping s0 ↔ s1 at EVERY time frame.
    let sym_init = ProofExpr::Or(Box::new(atom("s0@0")), Box::new(atom("s1@0")));
    let sym_trans = |t: u32| {
        conj(vec![
            iff(atom(&format!("s0@{}", t + 1)), atom(&format!("s0@{}", t))),
            iff(atom(&format!("s1@{}", t + 1)), atom(&format!("s1@{}", t))),
        ])
    };
    let not_both = |t: u32| {
        not(ProofExpr::And(
            Box::new(atom(&format!("s0@{}", t))),
            Box::new(atom(&format!("s1@{}", t))),
        ))
    };

    // The temporal symmetry is detected: s0, s1 are interchangeable across time.
    let pairs = temporal_symmetry_pairs(&sym_init, &sym_trans(0), &not_both(0));
    assert_eq!(
        pairs,
        vec![("s0".to_string(), "s1".to_string())],
        "s0 ↔ s1 is a temporal symmetry"
    );

    // Counterexample (both latches on at t=0 violates "not both") — symmetric BMC agrees with plain BMC.
    let plain = find_counterexample(&sym_init, &sym_trans, &not_both, 4);
    let broken = find_counterexample_symmetric(&sym_init, &sym_trans, &not_both, 4);
    assert!(
        matches!(plain, BmcOutcome::CounterexampleAt { k: 0, .. }),
        "plain: {plain:?}"
    );
    assert!(
        matches!(broken, BmcOutcome::CounterexampleAt { k: 0, .. }),
        "broken: {broken:?}"
    );

    // No counterexample for the same symmetric system with a satisfied property ("at least one on"):
    // the symmetry break must not turn a NoneWithin into a spurious counterexample.
    let at_least_one = |t: u32| {
        ProofExpr::Or(
            Box::new(atom(&format!("s0@{}", t))),
            Box::new(atom(&format!("s1@{}", t))),
        )
    };
    assert_eq!(
        find_counterexample_symmetric(&sym_init, &sym_trans, &at_least_one, 5),
        find_counterexample(&sym_init, &sym_trans, &at_least_one, 5),
        "symmetric BMC matches plain BMC on a held invariant"
    );

    // An asymmetric system (the single-signal toggle) has no interchangeable signals, and the symmetric
    // search then coincides exactly with plain BMC.
    assert!(
        temporal_symmetry_pairs(&toggle_init(), &toggle_trans(0), &toggle_always_false(0))
            .is_empty(),
        "a one-signal system has no signal-swap symmetry"
    );
    assert_eq!(
        find_counterexample_symmetric(&toggle_init(), &toggle_trans, &toggle_always_false, 5),
        find_counterexample(&toggle_init(), &toggle_trans, &toggle_always_false, 5),
        "with no symmetry, symmetric BMC = plain BMC"
    );
}

#[test]
fn incremental_bmc_no_counterexample_for_latched_invariant() {
    assert_eq!(
        find_counterexample_incremental(&latched_init(), &latched_trans, &latched_prop, 6),
        BmcOutcome::NoneWithin(6)
    );
}

fn sig(i: usize, t: u32) -> ProofExpr {
    ProofExpr::Atom(format!("s{i}@{t}"))
}
fn lit_b(b: bool, e: ProofExpr) -> ProofExpr {
    if b {
        e
    } else {
        ProofExpr::Not(Box::new(e))
    }
}

#[test]
fn incremental_matches_nonincremental_and_simulation() {
    // Random DETERMINISTIC functional systems over `n` boolean signals: a fixed initial
    // state and `next_i = (maybe ¬) current_{src_i}`. The unique trace is computed by
    // direct simulation (the oracle); BOTH BMC variants must report the first depth it
    // violates the property — and agree with each other.
    let mut state = 0xb5ad_4ece_da1c_e2a9u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let n = 3usize;
    let max_k = 5u32;
    for _trial in 0..120 {
        let init_bits: Vec<bool> = (0..n).map(|_| next() & 1 == 0).collect();
        let src: Vec<usize> = (0..n).map(|_| (next() % n as u64) as usize).collect();
        let inv: Vec<bool> = (0..n).map(|_| next() & 1 == 0).collect();
        let p = (next() % n as u64) as usize;
        let want = next() & 1 == 0;

        let init_expr = conj((0..n).map(|i| lit_b(init_bits[i], sig(i, 0))).collect());
        let (src_t, inv_t) = (src.clone(), inv.clone());
        let trans = move |t: u32| {
            conj(
                (0..n)
                    .map(|i| {
                        // s_i@(t+1) ↔ (inv_i ? : ¬) s_{src_i}@t
                        let rhs = lit_b(inv_t[i], sig(src_t[i], t));
                        ProofExpr::Iff(Box::new(sig(i, t + 1)), Box::new(rhs))
                    })
                    .collect(),
            )
        };
        let prop = move |t: u32| lit_b(want, sig(p, t));

        // Oracle: simulate the unique trace, find the first violating depth.
        let mut cur = init_bits.clone();
        let mut expected: Option<u32> = None;
        for k in 0..=max_k {
            if (cur[p] == want) == false {
                expected = Some(k);
                break;
            }
            let nxt: Vec<bool> = (0..n)
                .map(|i| if inv[i] { cur[src[i]] } else { !cur[src[i]] })
                .collect();
            cur = nxt;
        }

        let k_of = |o: &BmcOutcome| match o {
            BmcOutcome::CounterexampleAt { k, .. } => Some(*k),
            BmcOutcome::NoneWithin(_) => None,
            BmcOutcome::Unsupported => panic!("unexpected Unsupported"),
        };
        let ni = find_counterexample(&init_expr, &trans, &prop, max_k);
        let inc = find_counterexample_incremental(&init_expr, &trans, &prop, max_k);
        assert_eq!(
            k_of(&ni),
            expected,
            "non-incremental BMC disagrees with simulation"
        );
        assert_eq!(
            k_of(&inc),
            expected,
            "incremental BMC disagrees with simulation"
        );
    }
}
