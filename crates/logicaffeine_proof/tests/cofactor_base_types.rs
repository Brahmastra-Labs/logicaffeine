//! **Is the cofactor collapse a property of the base type?**
//!
//! `work/PAPER.md` §8.5 collapses the 42,263 `n = 4` orbits to ~403 structural signatures (base types), and
//! the Uniform Transfer Theorem carries certificates along morphs — so if the CofactorIso collapse is
//! determined by the base-type signature, measuring the few hundred base types captures the whole
//! hypercube (a ×105 reduction of the `3-SAT ∈ coNP` target). This measures exactly that: over a
//! sample of instance-rigid residue cores, group by `hypercube::abstract_signature`, and report
//! whether the collapse (distinct − iso) is *consistent within a signature*. A measurement — the
//! within-signature spread says whether the signature determines the collapse or not.

use logicaffeine_proof::cofactor::{canon, distinct_width, quotient_class_count, CofactorIso};
use logicaffeine_proof::hypercube::{abstract_signature, automorphism_group_size, minimal_cover_orbits};
use std::collections::BTreeMap;

#[test]
fn the_cofactor_collapse_distribution_across_base_types_is_measured() {
    let n = 4usize;
    // signature string -> collapses (distinct − iso) of the rigid cores carrying it.
    let mut by_sig: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut rigid = 0usize;
    for cover in minimal_cover_orbits(n).iter().step_by(60) {
        let clauses = cover.clauses();
        if automorphism_group_size(n, &clauses) != 1 {
            continue; // instance-symmetric — not a residue core (fast proxy)
        }
        rigid += 1;
        let cc = canon(&clauses);
        let distinct = distinct_width(n, &cc) as i64;
        let iso = quotient_class_count(n, &cc, &CofactorIso { cap: 4 }) as i64;
        let sig = format!("{:?}", abstract_signature(n, &clauses));
        by_sig.entry(sig).or_default().push(distinct - iso);
    }
    assert!(rigid > 0, "rigid residue cores sampled");
    // How many signatures, and is the collapse consistent within a signature?
    let sigs = by_sig.len();
    let mut consistent = 0usize;
    let mut spread_total = 0i64;
    for collapses in by_sig.values() {
        let lo = *collapses.iter().min().unwrap();
        let hi = *collapses.iter().max().unwrap();
        if lo == hi {
            consistent += 1;
        }
        spread_total += hi - lo;
    }
    let mean_spread = spread_total as f64 / sigs.max(1) as f64;
    eprintln!(
        "base-type collapse (n=4, {rigid} rigid cores, {sigs} distinct signatures): \
         {consistent}/{sigs} signatures have IDENTICAL collapse across their cores; \
         mean within-signature spread {mean_spread:.2} classes"
    );
    eprintln!(
        "  reading: low spread ⟹ the base-type signature largely determines the cofactor collapse, so \
         measuring the ~309 base types captures the hypercube (the Uniform-Transfer ×105 reduction); \
         high spread ⟹ the signature is too coarse and collapse needs the finer morph structure"
    );
}
