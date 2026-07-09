//! **The congruence-ladder climb: does a richer rung break the CofactorIso plateau?**
//!
//! D3 measured CofactorIso collapsing a roughly-constant ~5–7% of the residue's cofactors (ratio
//! distinct/iso ≈ 1.05–1.07). The climb: stack strictly-coarser *decidable* rungs — `UnitPropIso`
//! (unit propagation, then iso) and `ReduceIso` (unit-prop + pure-literal + subsumption to fixpoint,
//! then iso) — and measure whether the collapse ratio breaks meaningfully past that plateau. The
//! ladder `reduce-iso ≤ unitprop ≤ iso ≤ distinct` holds per core as a theorem; the science is the
//! *ratio* trend. A measurement, honestly reported: if the richer rung climbs, the decidable ladder
//! has more to give; if it plateaus, the SR rung (extension variables) is required — the open cell.

use logicaffeine_proof::cdcl::{Lit, SolveResult, Solver};
use logicaffeine_proof::cofactor::{
    canon, distinct_width, quotient_class_count, CanonClauses, CofactorIso, ReduceIso, UnitPropIso,
};
use logicaffeine_proof::hypercube::automorphism_group_size;
use std::collections::BTreeSet;

fn lcg(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state >> 33
}

fn is_unsat(n: usize, clauses: &[Vec<Lit>]) -> bool {
    let mut s = Solver::new(n);
    for c in clauses {
        s.add_clause(c.clone());
    }
    matches!(s.solve(), SolveResult::Unsat)
}

fn sample_rigid_cores(n: usize, want: usize, seed: u64) -> Vec<CanonClauses> {
    let mut state = seed;
    let mut seen: BTreeSet<CanonClauses> = BTreeSet::new();
    let mut out: Vec<CanonClauses> = Vec::new();
    let mut attempts = 0;
    while out.len() < want && attempts < 4000 {
        attempts += 1;
        let nc = (2 * n) + (lcg(&mut state) % (3 * n as u64)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nc)
            .map(|_| {
                let width = 2 + (lcg(&mut state) % 2) as usize;
                let mut vars: Vec<u32> = Vec::new();
                while vars.len() < width {
                    let v = (lcg(&mut state) % n as u64) as u32;
                    if !vars.contains(&v) {
                        vars.push(v);
                    }
                }
                vars.iter().map(|&v| Lit::new(v, lcg(&mut state) & 1 == 1)).collect()
            })
            .collect();
        if !is_unsat(n, &clauses) {
            continue;
        }
        let mut core = clauses;
        let mut i = 0;
        while i < core.len() {
            let mut trial = core.clone();
            trial.remove(i);
            if is_unsat(n, &trial) {
                core = trial;
            } else {
                i += 1;
            }
        }
        if automorphism_group_size(n, &core) != 1 {
            continue;
        }
        let cc = canon(&core);
        if seen.insert(cc.clone()) {
            out.push(cc);
        }
    }
    out
}

#[test]
fn the_richer_congruence_rungs_are_measured_against_the_cofactor_iso_plateau() {
    // (n, cores, mean class counts: distinct, iso, unitprop, reduce)
    let mut rows: Vec<(usize, usize, f64, f64, f64, f64)> = Vec::new();
    for n in [5usize, 6, 7, 8, 9, 10] {
        let cores = sample_rigid_cores(n, 24, 0xC11B5 ^ (n as u64) << 24);
        let cnt = cores.len();
        let (mut sd, mut si, mut su, mut sr) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for cc in &cores {
            let dist = distinct_width(n, cc) as f64;
            let iso = quotient_class_count(n, cc, &CofactorIso { cap: 6 }) as f64;
            let up = quotient_class_count(n, cc, &UnitPropIso { cap: 6 }) as f64;
            let rd = quotient_class_count(n, cc, &ReduceIso { cap: 6 }) as f64;
            // The ladder is a per-core theorem — re-confirm it on every sampled residue core.
            assert!(rd <= up && up <= iso && iso <= dist, "n={n}: ladder reduce ≤ unitprop ≤ iso ≤ distinct");
            sd += dist;
            si += iso;
            su += up;
            sr += rd;
        }
        let c = cnt.max(1) as f64;
        rows.push((n, cnt, sd / c, si / c, su / c, sr / c));
    }
    assert!(rows.iter().all(|r| r.1 > 0), "rigid cores found at every scale");
    for r in &rows {
        eprintln!(
            "climb n={} ({} rigid cores): mean CLASS COUNTS — distinct {:.1}, iso {:.1}, unitprop {:.1}, \
             reduce {:.1} | ratio distinct/reduce {:.2}",
            r.0, r.1, r.2, r.3, r.4, r.5, r.2 / r.5.max(1.0)
        );
    }
    eprintln!(
        "  reading: the CRUX is whether reduce's mean class count grows POLYNOMIALLY (⟹ toward the poly \
         threshold, a real climb) or SLOW-EXPONENTIALLY (⟹ the Chvátal–Szemerédi resolution cap on the \
         resolution-simulatable reduce rung, and the SR/extension-variable rung is required). Distinct \
         is the exponential floor; watch reduce against it as n grows"
    );
}
