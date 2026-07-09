//! **The non-resolution rung: do GF(2)/mod-p specialists fire on residue cofactors?**
//!
//! `reduce` (unit-prop + pure + subsumption) is resolution-simulatable, so Chvátal–Szemerédi caps it.
//! The rung that *beats* resolution is the non-resolution one — parity/GF(2), mod-`p`, exact-cover —
//! polynomial exactly where resolution is exponential. `StructuredReduceIso` fuses those specialists
//! (via `structured_leaf`, restricted to non-resolution routes) onto `reduce` as terminals. This
//! measures whether they add collapse on the residue beyond `reduce`. Honest either way:
//!   - `struct-reduce ≪ reduce` ⟹ the non-resolution crushers fire on residue cofactors — a real
//!     collapse past the resolution cap.
//!   - `struct-reduce ≈ reduce` ⟹ they rarely fire on the genuine residue, so the residue is beyond
//!     *both* resolution and the non-resolution specialists, and only the SR rung remains (the open cell).
//! Bounded sample (`solve_structured` per cofactor is expensive), measured — a negative result is data.

use logicaffeine_proof::cdcl::{Lit, SolveResult, Solver};
use logicaffeine_proof::cofactor::{
    canon, distinct_width, quotient_class_count, CanonClauses, ReduceIso, StructuredReduceIso,
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
fn the_non_resolution_rung_is_measured_against_reduce_on_the_residue() {
    let mut rows: Vec<(usize, usize, f64, f64, f64)> = Vec::new(); // (n, cores, distinct, reduce, struct-reduce)
    for n in [5usize, 6, 7] {
        let cores = sample_rigid_cores(n, 12, 0xC0FFEE ^ (n as u64) << 24);
        let cnt = cores.len();
        let (mut sd, mut srd, mut sst) = (0.0f64, 0.0f64, 0.0f64);
        for cc in &cores {
            let dist = distinct_width(n, cc) as f64;
            let rd = quotient_class_count(n, cc, &ReduceIso { cap: 6 }) as f64;
            let st = quotient_class_count(n, cc, &StructuredReduceIso { cap: 6 }) as f64;
            // The non-resolution rung is coarser than reduce — a theorem, re-confirmed per core.
            assert!(st <= rd, "n={n}: struct-reduce {st} ≤ reduce {rd}");
            sd += dist;
            srd += rd;
            sst += st;
        }
        let c = cnt.max(1) as f64;
        rows.push((n, cnt, sd / c, srd / c, sst / c));
    }
    assert!(rows.iter().all(|r| r.1 > 0), "rigid cores found at every scale");
    for r in &rows {
        eprintln!(
            "non-res rung n={} ({} cores): mean class counts — distinct {:.1}, reduce {:.1}, struct-reduce {:.1} \
             | non-res gain {:.2}",
            r.0, r.1, r.2, r.3, r.4, r.3 - r.4
        );
    }
    eprintln!(
        "  reading: 'non-res gain' = reduce − struct-reduce = classes the GF(2)/mod-p crushers remove \
         beyond resolution. Large ⟹ the non-resolution rung beats the resolution cap on the residue; \
         ~0 ⟹ the specialists don't fire on the genuine residue, and the SR rung is the only one left (open cell)"
    );
}
