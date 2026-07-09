//! **Does the cofactor-collapse magnitude grow with `n`?**
//!
//! At `n = 4` the residue's CofactorIso collapse is order-1 (distinct 9 → iso 8). The question with
//! asymptotic teeth: does the collapse *grow* with scale? This samples instance-rigid minimal-UNSAT
//! cores at `n = 4, 5, 6` (random generation + deletion-minimization, filtered by the production
//! automorphism finder — a fast rigidity proxy, which can under-report, adequate for a trend), and
//! measures the distinct-cofactor floor against the CofactorIso class count. A measurement, reported
//! honestly: the mean collapse and mean ratio across scales, so the trend is visible whatever it is.

use logicaffeine_proof::cdcl::{Lit, SolveResult, Solver};
use logicaffeine_proof::cofactor::{canon, distinct_width, quotient_class_count, CanonClauses, CofactorIso};
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

/// Sample up to `want` instance-rigid minimal-UNSAT cores at `n`: random 2–3-CNF, filtered UNSAT,
/// deletion-minimized to a core, kept only if the production automorphism finder sees no symmetry.
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
            continue; // instance-symmetric — not a residue core
        }
        let cc = canon(&core);
        if seen.insert(cc.clone()) {
            out.push(cc);
        }
    }
    out
}

#[test]
fn the_cofactor_collapse_magnitude_across_scales_is_measured() {
    // (n, samples, mean distinct, mean iso, mean absolute collapse, mean ratio)
    let mut rows: Vec<(usize, usize, f64, f64, f64, f64)> = Vec::new();
    for n in [4usize, 5, 6] {
        let cores = sample_rigid_cores(n, 24, 0x5CE_2CE ^ (n as u64) << 20);
        let cnt = cores.len();
        let (mut td, mut ti, mut tc, mut tr) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for cc in &cores {
            let d = distinct_width(n, cc);
            let iso = quotient_class_count(n, cc, &CofactorIso { cap: 6 });
            assert!(iso <= d, "n={n}: monotonicity iso {iso} ≤ distinct {d}");
            td += d as f64;
            ti += iso as f64;
            tc += (d - iso) as f64;
            tr += d as f64 / iso.max(1) as f64;
        }
        let c = cnt.max(1) as f64;
        rows.push((n, cnt, td / c, ti / c, tc / c, tr / c));
    }
    assert!(rows.iter().all(|r| r.1 > 0), "rigid minimal-UNSAT cores found at every scale");
    for r in &rows {
        eprintln!(
            "cofactor-collapse scale n={}: {} rigid cores, mean-distinct {:.1}, mean-iso {:.1}, \
             mean-collapse {:.2} classes, mean-ratio {:.3}",
            r.0, r.1, r.2, r.3, r.4, r.5
        );
    }
    eprintln!(
        "  reading: whether the mean absolute collapse (distinct − iso) grows across n=4→6 says \
         whether symmetry-above-the-instance has asymptotic teeth or stays order-1 — measured, not assumed"
    );
}
