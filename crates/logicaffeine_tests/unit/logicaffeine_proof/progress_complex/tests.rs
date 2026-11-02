use super::*;

#[test]
fn no_contention_is_a_contractible_square_determinism() {
    // No shared resource ⇒ no forbidden cells ⇒ the full filled grid, contractible: β₀ = 1, β₁ = 0.
    // This is the deterministic case — the same contractible execution space as an independent trace.
    let pc = ProgressComplex::new(4, 4, &[]);
    let (b0, b1, b2) = pc.betti();
    assert_eq!(
        (b0, b1, b2),
        (1, 0, 0),
        "an uncontended execution space is contractible"
    );
    assert_eq!(
        b0 as i64 - b1 as i64 + b2 as i64,
        pc.euler(),
        "Euler–Poincaré holds"
    );
    assert_eq!(pc.euler(), 1);
}

#[test]
fn one_mutex_opens_a_hole_pi_one_is_Z_the_deadlock() {
    // GENUINE HIGHER HOMOTOPY. Both processes run [.., acquire, CS, CS, release, ..]; the forbidden
    // region is the 2×2 block where both are inside the critical section. Removing it opens ONE
    // hole: β₁ = 1, so the execution space is homotopy-equivalent to a circle (π₁ = Z). The two
    // directed paths around the hole are "P finishes its CS before Q" vs "Q before P" — genuinely
    // inequivalent schedules. The hole IS the synchronization obstruction.
    let forbidden = [(1, 1), (1, 2), (2, 1), (2, 2)];
    let pc = ProgressComplex::new(4, 4, &forbidden);
    let (b0, b1, b2) = pc.betti();
    assert_eq!(b0, 1, "the allowed region is still connected");
    assert_eq!(
        b1, 1,
        "ONE hole — π₁ = Z — the mutex carves real higher homotopy"
    );
    assert_eq!(b2, 0, "no enclosed void");
    assert_eq!(
        b0 as i64 - b1 as i64 + b2 as i64,
        pc.euler(),
        "Euler–Poincaré: β₀−β₁+β₂ = χ"
    );
    assert_eq!(pc.euler(), 0, "χ = 0, the signature of a single hole");
}

#[test]
fn two_critical_sections_give_two_holes_beta_one_counts_them() {
    // Two disjoint forbidden blocks (two contended resources) ⇒ β₁ = 2. The first Betti number
    // genuinely COUNTS the independent obstructions — the homology is real, not a yes/no flag.
    let forbidden = [
        (1, 1),
        (1, 2),
        (2, 1),
        (2, 2),
        (4, 4),
        (4, 5),
        (5, 4),
        (5, 5),
    ];
    let pc = ProgressComplex::new(7, 7, &forbidden);
    let (b0, b1, b2) = pc.betti();
    assert_eq!(
        (b0, b1, b2),
        (1, 2, 0),
        "two critical sections ⇒ two independent holes"
    );
    assert_eq!(
        b0 as i64 - b1 as i64 + b2 as i64,
        pc.euler(),
        "Euler–Poincaré holds with two holes"
    );
}

#[test]
fn three_processes_solid_is_contractible() {
    // No forbidden cell ⇒ a solid 3×3×3 block of cubes, contractible: β = (1,0,0,0). The 3D
    // determinism baseline, and a check that the ∂₃ homology machinery is calibrated.
    let pc = ProgressComplex3::new(3, 3, 3, &[]);
    assert_eq!(
        pc.betti(),
        (1, 0, 0, 0),
        "a solid 3-process execution is contractible"
    );
}

#[test]
fn a_forbidden_core_opens_a_2_sphere_pi_two_is_Z() {
    // CLIMBING A RUNG: genuine π₂. Forbid the single center cell of a 3×3×3 grid — the state where
    // all three processes are jointly in the forbidden core. Its six faces remain (they bound the
    // surrounding allowed cells), forming a 2-CYCLE that no longer bounds anything: a hollow
    // 2-SPHERE. So β₂ = 1 — the execution space has π₂ = Z. This is the first π₂ the tower PRODUCES
    // from a real system, not merely admits via the crossed-module machinery. The 2-void is the
    // higher-dimensional synchronization obstruction: a sphere of schedules that cannot be contracted.
    let pc = ProgressComplex3::new(3, 3, 3, &[(1, 1, 1)]);
    let (b0, b1, b2, b3) = pc.betti();
    assert_eq!(b0, 1, "still connected");
    assert_eq!(b1, 0, "no 1-holes");
    assert_eq!(b2, 1, "a hollow 2-sphere — π₂ = Z, genuine higher homotopy");
    assert_eq!(b3, 0, "no enclosed 3-void");
    // Euler–Poincaré in 3D: β₀ − β₁ + β₂ − β₃ = χ, and χ = 2 for a 2-sphere shell.
    assert_eq!(
        b0 as i64 - b1 as i64 + b2 as i64 - b3 as i64,
        2,
        "χ = 2, the signature of a 2-sphere"
    );
}

#[test]
fn the_hole_is_the_obstruction_to_breaking_the_scheduler_symmetry() {
    // THE THROUGH-LINE, closed. For a contractible execution (β₁ = 0) the scheduler symmetry breaks
    // cleanly — one canonical schedule per trace, the π₀ collapse of `trace_determinism`. The instant
    // a mutex opens a hole (β₁ = 1), that collapse is OBSTRUCTED: there are β₁ independent classes of
    // directed paths the commutation 2-cells cannot merge, because merging them would cross the
    // forbidden region. So β₁ measures, exactly, how far the scheduler symmetry FAILS to be breakable.
    let clean = ProgressComplex::new(4, 4, &[]).betti().1;
    let contended = ProgressComplex::new(4, 4, &[(1, 1), (1, 2), (2, 1), (2, 2)])
        .betti()
        .1;
    assert_eq!(
        clean, 0,
        "no hole ⇒ scheduler symmetry fully breakable (determinism)"
    );
    assert!(
        contended > clean,
        "a hole ⇒ symmetry breaking is obstructed — β₁ counts the obstruction"
    );
}
