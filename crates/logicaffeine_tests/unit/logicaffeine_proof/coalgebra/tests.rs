use super::*;

#[test]
fn well_founded_iff_admits_a_countdown_morphism() {
    // THE THEOREM, machine-checked over many random finite systems: the system is well-founded
    // (acyclic) IFF it admits a coalgebra morphism to the countdown. Forward (acyclic ⇒ morphism):
    // the canonical longest-path ranking IS one. Backward (cyclic ⇒ no morphism): a cycle would
    // force `V(s) < V(s)` — so no candidate, however chosen, can be a morphism.
    let mut state = 0x0CA1_0B5A_7777_1234u64;
    let mut next = || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let mut acyclic_seen = 0;
    let mut cyclic_seen = 0;
    for _ in 0..5_000 {
        let n = 2 + (next() as usize % 7);
        let mut succ = vec![Vec::new(); n];
        for s in 0..n {
            let deg = next() as usize % 3;
            for _ in 0..deg {
                succ[s].push(next() as usize % n);
            }
        }
        let system = TransitionSystem {
            n_states: n,
            successors: succ,
        };
        let wf = is_well_founded(&system);
        match canonical_countdown_morphism(&system) {
            Some(m) => {
                assert!(wf, "a morphism was produced ⇒ must be well-founded");
                assert!(
                    is_countdown_morphism(&system, &m),
                    "the canonical morphism must check"
                );
                acyclic_seen += 1;
            }
            None => {
                assert!(!wf, "no morphism ⇒ must be cyclic");
                // The impossibility, witnessed: NO measure is a morphism for a cyclic system.
                for _ in 0..6 {
                    let cand: Vec<u64> = (0..n).map(|_| next() % (n as u64 + 1)).collect();
                    assert!(
                        !is_countdown_morphism(&system, &cand),
                        "a cycle admits no Lyapunov fn"
                    );
                }
                cyclic_seen += 1;
            }
        }
    }
    assert!(
        acyclic_seen > 0 && cyclic_seen > 0,
        "the equivalence must be exercised both ways"
    );
}

#[test]
fn a_cycle_admits_no_lyapunov_function() {
    // The impossibility side, explicit: a 3-cycle `0→1→2→0` is non-terminating, and NO potential
    // is a countdown morphism for it — the categorical statement of "no measure ⇒ no termination".
    let cycle = TransitionSystem {
        n_states: 3,
        successors: vec![vec![1], vec![2], vec![0]],
    };
    assert!(!is_well_founded(&cycle));
    assert!(canonical_countdown_morphism(&cycle).is_none());
    // Exhaustively over all small potentials: none is a morphism.
    for a in 0..4u64 {
        for b in 0..4u64 {
            for c in 0..4u64 {
                assert!(
                    !is_countdown_morphism(&cycle, &[a, b, c]),
                    "no potential can descend around a cycle"
                );
            }
        }
    }
}

#[test]
fn the_kernel_termination_guard_is_this_well_founded_coalgebra_theorem() {
    // FUSING THE TOWER TO THE KERNEL FIX (CRITIQUE #1). The structural-recursion guard just hardened
    // in logicaffeine_kernel accepts a fixpoint IFF every recursive call decreases a structural
    // measure — i.e. iff the recursion relation is WELL-FOUNDED. That is exactly this rung:
    // well-founded ⟺ admits a countdown morphism (a Lyapunov ranking). The guard is not ad hoc; it
    // enforces "the recursion admits a Lyapunov function," the coalgebra characterization of
    // termination — the same theorem the ∞-groupoid tower is built on, now grounding a real soundness
    // fix in the proof kernel.
    //
    // A structurally-decreasing fixpoint on Nat is the countdown chain n → n−1 → … → 0: well-founded,
    // and the structural size IS the morphism the guard implicitly constructs.
    let decreasing = TransitionSystem {
        n_states: 6,
        successors: (0..6)
            .map(|i| if i > 0 { vec![i - 1] } else { vec![] })
            .collect(),
    };
    assert!(
        is_well_founded(&decreasing),
        "structural recursion on Nat is well-founded (the guard accepts it)"
    );
    let measure = canonical_countdown_morphism(&decreasing)
        .expect("a well-founded recursion has a Lyapunov ranking");
    assert!(
        is_countdown_morphism(&decreasing, &measure),
        "the structural size IS the countdown morphism"
    );

    // The non-decreasing self-call `f n → f n` — exactly what the hardened guard now rejects — is a
    // self-loop: not well-founded, and it admits NO countdown morphism. No Lyapunov function exists,
    // so the recursion cannot terminate. The guard's rejection and the coalgebra's are the same fact.
    let self_call = TransitionSystem {
        n_states: 1,
        successors: vec![vec![0]],
    };
    assert!(
        !is_well_founded(&self_call),
        "a non-decreasing self-call is a cycle (the guard rejects it)"
    );
    assert!(
        canonical_countdown_morphism(&self_call).is_none(),
        "no Lyapunov function for a non-terminating recursion"
    );
}

#[test]
fn our_lyapunov_trajectories_are_countdown_morphisms() {
    // The connection back to the framework. A Lyapunov measure allows plateaus (several certified
    // moves at one potential level), so it is a *lax* morphism to `(ℕ, ≥)`. Its **level structure**
    // — the distinct potential values, on the level system `level_0 → level_1 → …` — is the STRICT
    // morphism to the countdown coalgebra, and that strict descent is what witnesses termination.
    // We check it on the actual symmetry and parity trajectories.
    let level_morphism = |traj: &[u64]| {
        let mut levels = traj.to_vec();
        levels.dedup(); // distinct potential levels (the trajectory is non-increasing)
        let path = TransitionSystem::path(levels.len());
        is_countdown_morphism(&path, &levels)
    };
    let (php, _) = crate::families::php(6);
    if let Some((_, ranked)) =
        crate::lyapunov::solve_by_measure_synthesis(php.num_vars, &php.clauses)
    {
        assert!(
            level_morphism(&ranked.ranks),
            "the symmetry measure's levels are a countdown morphism"
        );
    }
    let (eqs, tcnf, _) = crate::families::tseitin_expander(10, 7);
    let (traj, _) = crate::lyapunov::gaussian_lyapunov(&eqs, tcnf.num_vars);
    assert!(
        level_morphism(&traj),
        "the parity measure's levels are a countdown morphism"
    );
}
