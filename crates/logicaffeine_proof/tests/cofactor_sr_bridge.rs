//! **The SR rung — the open cell's tool, above the decidable cofactor congruences.**
//!
//! The cofactor lens exhausts its *decidable* congruences at the wall (CofactorIso does not collapse
//! the residue). The rung above is SR/PR — extension variables / substitution-redundancy — which is
//! exactly the mechanism that can relate cofactors CNF-isomorphism cannot. This file is the honest
//! first stone of that bridge: the existing SR engine (`sdcl_refute`) refuted with a zero-trust-checked
//! PR certificate, shown operating (i) on pigeonhole — the certified precedent that an SR-poly family
//! is resolution-exponential (§8.4) — and (ii) on a *cofactor-iso-rigid* residue core, where the
//! strongest decidable cofactor congruence gives no collapse at all. What is **not** claimed: an
//! asymptotic separation at these scales (at `n = 4` plain RUP already refutes everything). What **is**
//! localized: the open cell is whether the SR certificate size stays *polynomial along a family* — the
//! §8.3 mirror curve, equivalent to `3-SAT ∈ coNP`.

use logicaffeine_proof::cofactor::{canon, distinct_width, quotient_class_count, CofactorIso};
use logicaffeine_proof::hypercube::minimal_cover_orbits;
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::sdcl::sdcl_refute;

#[test]
fn the_sr_engine_operates_above_the_decidable_cofactor_congruence() {
    // (i) Pigeonhole — the certified precedent (SR-poly, resolution-exponential). CofactorIso collapses
    //     it, the group-cofactor certificate is poly (§ cofactor_lens), and the SR engine refutes it
    //     with a re-checked PR certificate: the three rungs agree on the same family.
    let (php, _) = logicaffeine_proof::families::php(4);
    let php_cc = canon(&php.clauses);
    let php_distinct = distinct_width(php.num_vars, &php_cc);
    let php_iso = quotient_class_count(php.num_vars, &php_cc, &CofactorIso { cap: 4 });
    let php_cert = sdcl_refute(php.num_vars, &php.clauses);
    assert!(php_cert.refuted, "PHP(4) refuted by the SR engine");
    assert!(
        check_pr_refutation(php.num_vars, &php.clauses, &php_cert.steps),
        "PHP(4)'s PR certificate re-checks with zero trust"
    );
    eprintln!(
        "SR bridge (i) PHP(4): cofactor-iso {php_iso} ≤ distinct {php_distinct}; SR refutation \
         {} PR steps, re-checked — the precedent that escalation collapses happen (Haken-exp → SR-poly)",
        php_cert.sbp_clauses
    );

    // (ii) A cofactor-iso-RIGID residue core: CofactorIso gives NO collapse (iso == distinct), the
    //      decidable rung is exhausted on it — yet the SR engine still refutes it, zero-trust re-checked.
    let n = 4usize;
    let core = minimal_cover_orbits(n)
        .into_iter()
        .find(|cover| {
            let cc = canon(&cover.clauses());
            quotient_class_count(n, &cc, &CofactorIso { cap: 4 }) == distinct_width(n, &cc)
        })
        .expect("a cofactor-iso-rigid core exists at n=4");
    let clauses = core.clauses();
    let cc = canon(&clauses);
    let iso = quotient_class_count(n, &cc, &CofactorIso { cap: 4 });
    let distinct = distinct_width(n, &cc);
    assert_eq!(iso, distinct, "the core is cofactor-iso-rigid — the decidable rung is exhausted on it");
    let cert = sdcl_refute(n, &clauses);
    assert!(cert.refuted, "the SR engine refutes the cofactor-iso-rigid core");
    assert!(
        check_pr_refutation(n, &clauses, &cert.steps),
        "the cofactor-iso-rigid core's PR certificate re-checks with zero trust"
    );
    eprintln!(
        "SR bridge (ii) rigid core: cofactor-iso {iso} == distinct {distinct} (no collapse), SR refutation \
         {} PR steps + RUP, re-checked — the SR rung operates where the decidable cofactor congruence stops",
        cert.sbp_clauses
    );
    eprintln!(
        "  THE OPEN CELL: whether the SR certificate size stays POLYNOMIAL along the residue family \
         (the §8.3 mirror curve) — that, and only that, is 3-SAT ∈ coNP. Everything below it is certified; \
         this rung is where the fight is, and no formal barrier forbids it."
    );
}
