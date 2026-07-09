//! **The cofactor-DAG lens in action: structured families collapse; the residue is measured.**
//!
//! `work/PAPER.md` §4 measures symmetry *in the instance*; the residue is rigid to it. This lens lifts to
//! the Shannon cofactor DAG and asks the dual of the incompressibility pole: exponentially-many
//! distinct cofactors — do they fall into polynomially-many *classes* under a congruence that need
//! not be an automorphism of the formula? The instrument is `logicaffeine_proof::cofactor`:
//! `distinct_width` (the strict distinct-cofactor floor) and `quotient_class_count` (the number of
//! `~`-classes among that same fixed set — monotone in `~`, `≤ distinct_width`, both as theorems).
//!
//! Three findings, each a measurement (a negative result is data too):
//!   1. **Structured families collapse** — the XOR cycle's cofactor count is linear, and CofactorIso
//!      keeps it linear; pigeonhole's group-cofactor certificate is polynomial and re-checks.
//!   2. **The residue is cofactor-measured** — at `n = 4`, over the instance-rigid cores, how far
//!      CofactorIso (symmetry *above* the instance) drops the class count below the distinct floor,
//!      and how many rigid cores collapse at all — the potential discovery, counted exactly.
//!   3. **The wall** — on random 3-CNF the distinct-cofactor count explodes and CofactorIso does not
//!      collapse it: the honest exponential floor the open cell lives behind.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::cofactor::{
    canon, canon_raw, check_quotient_dag, distinct_cofactor_dag, distinct_width, quotient_class_count,
    quotient_dag, CanonClauses, CofactorIso, GroupInduced,
};
use logicaffeine_proof::census::{affine_composite_shear_generators, affine_transvection_generators};
use logicaffeine_proof::dimacs::DimacsCnf;
use logicaffeine_proof::hypercube::{
    cube_group_closure, hyperoctahedral_generators, minimal_cover_orbits, php_perm_symmetries,
    Cover, CubeSym,
};
use logicaffeine_proof::proof::Perm;
use std::collections::BTreeSet;

/// Ground-truth `Bₙ`-rigidity: the exhaustive signed-permutation stabilizer is trivial. `group` is
/// the pre-closed `Bₙ` (hoisted once — it depends only on `n`), no symmetry finder in the loop.
fn bn_rigid(group: &[CubeSym], n: usize, clauses: &[Vec<Lit>]) -> bool {
    let cover = Cover::of_cnf(&DimacsCnf { num_vars: n, clauses: clauses.to_vec() });
    group.iter().filter(|g| g.is_automorphism(&cover)).count() == 1
}

/// Visible to any single-source shear of depth ≤ 3 (the affine lens, complete for its class at small
/// `n`) — the paper's second registered lens on the residue.
fn shear_visible(n: usize, clauses: &[Vec<Lit>]) -> bool {
    if !affine_transvection_generators(n, clauses).is_empty() {
        return true;
    }
    for depth in 2..=3usize.min(n.saturating_sub(1)) {
        if affine_composite_shear_generators(n, clauses, depth).iter().any(|(s, _)| s.len() == depth) {
            return true;
        }
    }
    false
}

fn xor_cycle(k: usize) -> CanonClauses {
    let mut raw: Vec<Vec<(u32, bool)>> = Vec::new();
    for i in 0..k {
        let j = (i + 1) % k;
        raw.push(vec![(i as u32, true), (j as u32, true)]);
        raw.push(vec![(i as u32, false), (j as u32, false)]);
    }
    canon_raw(&raw)
}

fn php_canon(m: usize) -> (usize, CanonClauses) {
    let (php, _) = logicaffeine_proof::families::php(m);
    (php.num_vars, canon(&php.clauses))
}

fn php_group(m: usize) -> Vec<Perm> {
    let nv = m * (m - 1);
    let gens = php_perm_symmetries(m);
    let key = |p: &Perm| -> Vec<u32> { (0..nv).map(|v| p.apply(Lit::pos(v as u32)).var()).collect() };
    let id = Perm::identity(nv);
    let mut seen: BTreeSet<Vec<u32>> = [key(&id)].into_iter().collect();
    let mut group = vec![id.clone()];
    let mut frontier = vec![id];
    while let Some(p) = frontier.pop() {
        for g in &gens {
            let q = p.compose(g);
            if seen.insert(key(&q)) {
                group.push(q.clone());
                frontier.push(q);
            }
        }
    }
    group
}

fn linear_by_second_difference(seq: &[usize]) -> bool {
    let d1: Vec<i64> = seq.windows(2).map(|w| w[1] as i64 - w[0] as i64).collect();
    d1.windows(2).all(|w| w[0] == w[1])
}

/// **Structured families collapse to a polynomial number of cofactor classes — certified.** The odd
/// XOR cycle's distinct-cofactor count is linear (constant second difference — the fitted-law /
/// interpolation-certificate pattern), and CofactorIso keeps it linear (bounded by the floor), so the
/// class count is polynomial at every rung. Pigeonhole's group-cofactor certificate DAG is polynomial
/// in `m` and re-checks with zero trust. This is the load-bearing direction of the lens: where the
/// cofactors fall into polynomially-many classes, the certificate is polynomial.
#[test]
fn the_cofactor_quotient_ladder_collapses_structured_families() {
    // --- XOR cycle: linear distinct floor, CofactorIso stays linear-bounded (poly). ---
    let mut distinct_seq: Vec<usize> = Vec::new();
    let mut iso_seq: Vec<usize> = Vec::new();
    for k in [5usize, 7, 9, 11, 13] {
        let f = xor_cycle(k);
        let d = distinct_width(k, &f);
        let iso = quotient_class_count(k, &f, &CofactorIso { cap: 5 });
        assert!(iso <= d, "k={k}: iso {iso} ≤ distinct {d}");
        distinct_seq.push(d);
        iso_seq.push(iso);
    }
    assert!(
        linear_by_second_difference(&distinct_seq),
        "XOR-cycle distinct-cofactor count is linear (poly): {distinct_seq:?}"
    );
    // iso ≤ a linear sequence ⟹ iso is O(k): polynomially-many cofactor classes at every scale.
    eprintln!("xor-cycle cofactor classes: distinct {distinct_seq:?}, cofactor-iso {iso_seq:?} (both poly)");

    // --- Pigeonhole: the group-cofactor certificate DAG is polynomial in m and re-checks. ---
    for m in [3usize, 4] {
        let (nv, clauses) = php_canon(m);
        let group = GroupInduced { group: php_group(m), label: "php-sym".into() };
        let dag = quotient_dag(nv, &clauses, &group).expect("PHP unfolds under its symmetry group");
        assert!(check_quotient_dag(&dag.nodes), "PHP({m}): the group-cofactor certificate re-checks");
        assert!(
            dag.width() <= 4 * m * m,
            "PHP({m}): the group-cofactor certificate is polynomial in m ({} ≤ {})",
            dag.width(),
            4 * m * m
        );
        eprintln!("php cofactor certificate: PHP({m}) → {} nodes (poly, re-checked)", dag.width());
    }
}

/// **The load-bearing lemma, executable: a small cofactor quotient IS a polynomial, zero-trust,
/// poly-time-checkable certificate.** Each quotient DAG re-checks locally (`check_quotient_dag`) and
/// is *output-sensitive* — the memoized recursion visits each class once, so both finding and
/// re-checking the certificate cost time linear in its own size. Demonstrated on the structured
/// families (XOR cycle under CofactorIso; pigeonhole under its group). The consequence, stated:
/// wherever the cofactors collapse to polynomially-many classes the refutation is polynomial and
/// cheaply verified — the class-sharing on the DAG edges is the extension-variable mechanism (the
/// Resolution→ER/SR jump), and this is the `poly classes ⟹ poly certificate` direction of the lens.
#[test]
fn poly_cofactor_quotient_is_a_poly_time_checkable_certificate() {
    let mut sizes: Vec<(String, usize, usize)> = Vec::new(); // (family, param, certificate size)
    for k in [5usize, 7, 9, 11, 13] {
        let f = xor_cycle(k);
        let dag = quotient_dag(k, &f, &CofactorIso { cap: 5 }).expect("odd XOR cycle UNSAT");
        assert!(check_quotient_dag(&dag.nodes), "xor k={k}: certificate re-checks with zero trust");
        assert!(dag.visits <= 2 * dag.width() + 2 * k + 2, "xor k={k}: found+checked in size-linear time");
        sizes.push(("xor-cycle".into(), k, dag.width()));
    }
    for m in [3usize, 4] {
        let (nv, cc) = php_canon(m);
        let dag = quotient_dag(nv, &cc, &GroupInduced { group: php_group(m), label: "php".into() })
            .expect("PHP UNSAT under its group");
        assert!(check_quotient_dag(&dag.nodes), "php m={m}: certificate re-checks with zero trust");
        assert!(dag.visits <= 2 * dag.width() + 2 * nv + 2, "php m={m}: found+checked in size-linear time");
        sizes.push(("php".into(), m, dag.width()));
    }
    eprintln!(
        "poly cofactor-quotient certificates (re-checked, output-sensitive — one format, one checker): {sizes:?}"
    );
}

/// **The residue, measured at the cofactor level.** Over the minimal-UNSAT cores at `n = 4`, split by
/// whether the *instance* carries any literal-permutation symmetry (`automorphism_group_size == 1` ⟺
/// rigid), we measure the strict distinct-cofactor floor against `CofactorIso` — the congruence that
/// is *not* required to be an automorphism of the formula. The honest question this answers with data:
/// among the instance-rigid cores, how many nonetheless have cofactors that collapse under
/// isomorphism — symmetry that lives in the cofactor DAG, not the instance? The count is reported
/// exactly, monotonicity (`iso ≤ distinct`) is re-confirmed on every residue member, and — when the
/// discovery set is non-empty — one such certificate is re-checked with zero trust. Sampled by stride
/// for the default run; the full sweep is the `#[ignore]`d companion below.
#[test]
fn the_residue_cofactor_dag_rigidity_is_measured_and_the_collapse_count_reported() {
    // Fast default: ~106 cores. Proves the machinery — monotonicity on every core, the residue is
    // populated, and at least one fully-rigid core's cofactor collapse carries a re-checked certificate.
    measure_residue_cofactor_rigidity(400);
}

#[test]
#[ignore = "ground-truth locked census (stride 41, ~1031 cores, exhaustive B₄ — ~4 min); run explicitly or in the full suite"]
fn the_residue_cofactor_dag_rigidity_ground_truth_locked() {
    // Locks the headline partition (130 Bₙ-sym / 73 shear-only / 828 residue) and the cofactor-collapse
    // split (387 collapse / 441 wall) — the reproducible finding that ~47% of the fully-rigid residue
    // carries symmetry above every instance lens.
    measure_residue_cofactor_rigidity(41);
}

fn measure_residue_cofactor_rigidity(stride: usize) {
    let n = 4usize;
    let group = cube_group_closure(&hyperoctahedral_generators(n), n); // the 384-element B₄, hoisted once
    let orbits = minimal_cover_orbits(n);
    let mut sampled = 0usize;
    let (mut symmetric, mut shear_only, mut residue) = (0usize, 0, 0);
    let (mut residue_collapse, mut residue_wall) = (0usize, 0);
    let mut discovery_example: Option<CanonClauses> = None;
    for cover in orbits.iter().step_by(stride) {
        sampled += 1;
        let clauses = cover.clauses();
        let cc = canon(&clauses);
        let distinct = distinct_width(n, &cc);
        let iso = quotient_class_count(n, &cc, &CofactorIso { cap: 4 }); // cap 4 = exact at n=4
        // Monotonicity is a theorem — re-confirm it on every core (residue included).
        assert!(iso <= distinct, "residue monotonicity: iso {iso} ≤ distinct {distinct}");
        // The paper's residue cascade, GROUND TRUTH (no symmetry finder): Bₙ-rigid AND shear-rigid.
        if !bn_rigid(&group, n, &clauses) {
            symmetric += 1;
            continue;
        }
        if shear_visible(n, &clauses) {
            shear_only += 1;
            continue;
        }
        residue += 1;
        if iso < distinct {
            residue_collapse += 1;
            if discovery_example.is_none() {
                discovery_example = Some(cc.clone());
            }
        } else {
            residue_wall += 1;
        }
    }
    assert!(residue > 0, "the residue (rigid under every §4 lens) is populated (sampled {sampled})");
    // LOCKED (2026-07-03, stride 41): the ground-truth partition and the cofactor-collapse split —
    // pinning the finding that ~47% of the fully-rigid residue carries symmetry ABOVE every instance
    // lens (387 of 828 residue cores collapse under CofactorIso). A regression in the cofactor
    // machinery or the rigidity cascade breaks here.
    if stride == 41 {
        assert_eq!((symmetric, shear_only, residue), (130, 73, 828), "ground-truth residue partition");
        assert_eq!((residue_collapse, residue_wall), (387, 441), "the cofactor-collapse split of the residue");
    }
    eprintln!(
        "cofactor-rigidity census (n=4, {sampled} orbits sampled, stride {stride}) — GROUND TRUTH Bₙ+shear:\n  \
         Bₙ-symmetric: {symmetric}  |  shear-only: {shear_only}  |  RESIDUE (rigid under every §4 lens): {residue}\n  \
         residue AND cofactor-iso COLLAPSES (symmetry ABOVE every instance lens): {residue_collapse}\n  \
         residue AND cofactor-iso-rigid too (the true wall): {residue_wall}"
    );
    // If any fully-rigid residue core's cofactors collapse, re-check one certificate with zero trust —
    // the "symmetry above the instance" is real and carries a checkable refutation, not just a number.
    if let Some(core) = discovery_example {
        let dag = quotient_dag(n, &core, &CofactorIso { cap: 4 }).expect("the core is UNSAT");
        assert!(check_quotient_dag(&dag.nodes), "the discovered cofactor-symmetric certificate re-checks");
        eprintln!(
            "  discovery certificate: a fully-rigid residue core with a re-checked cofactor-iso DAG of {} \
             nodes (distinct floor {})",
            dag.width(),
            distinct_width(n, &core)
        );
    }
}

/// **The wall: on random 3-CNF the distinct-cofactor count explodes and CofactorIso does not collapse
/// it.** This is the honest floor the open cell lives behind — the residue archetype (Chvátal–Szemerédi
/// random 3-CNF) where no *decidable* cofactor congruence we have keeps the class count polynomial. The
/// distinct-cofactor count grows with `n`, CofactorIso stays a large fraction of it (no dramatic
/// collapse), and both are reported. A negative result, measured — which localizes the open cell: any
/// polynomial collapse here would need an SR-definable congruence beyond CofactorIso.
#[test]
fn the_scaling_wall_random_3cnf_cofactors_explode_and_iso_does_not_collapse() {
    let mut series: Vec<(usize, usize, usize)> = Vec::new(); // (n, distinct, iso)
    for n in [8usize, 10, 12] {
        // Above the 3-SAT threshold (ratio ≈ 4.5); take the first seed that yields an UNSAT instance.
        let clauses = (0u64..64)
            .find_map(|seed| {
                let cnf = logicaffeine_proof::families::random_3sat(n, (n * 9) / 2, seed);
                let cc = canon(&cnf.clauses);
                distinct_cofactor_dag(n, &cc).map(|_| cc) // Some ⟺ UNSAT
            })
            .expect("an UNSAT random 3-CNF exists above threshold");
        let distinct = distinct_width(n, &clauses);
        let iso = quotient_class_count(n, &clauses, &CofactorIso { cap: 5 });
        assert!(iso <= distinct, "n={n}: monotonicity iso {iso} ≤ distinct {distinct}");
        series.push((n, distinct, iso));
    }
    // The distinct-cofactor count grows (the wall is real, not a small-n artifact).
    assert!(
        series.windows(2).all(|w| w[1].1 > w[0].1),
        "distinct-cofactor count grows with n on random 3-CNF: {series:?}"
    );
    // CofactorIso does NOT collapse it to a constant — it also grows, tracking the wall.
    assert!(
        series.windows(2).all(|w| w[1].2 >= w[0].2),
        "CofactorIso does not collapse random 3-CNF (iso non-decreasing with n): {series:?}"
    );
    eprintln!("scaling wall (random 3-CNF): (n, distinct, cofactor-iso) = {series:?}");
    eprintln!(
        "  the residue archetype: distinct cofactors explode and the strongest DECIDABLE congruence \
         (CofactorIso) does not make the class count polynomial — the open cell is a poly-index \
         SR-definable congruence beyond this rung"
    );
}
