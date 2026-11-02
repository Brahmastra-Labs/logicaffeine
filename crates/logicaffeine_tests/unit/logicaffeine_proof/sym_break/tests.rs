use super::*;
use crate::cdcl::{SolveResult, Solver};
use std::collections::BTreeSet;

/// All satisfying assignments of `clauses` over `num_vars`, by brute force.
fn models(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<bool>> {
    (0u64..(1u64 << num_vars))
        .filter_map(|x| {
            let a: Vec<bool> = (0..num_vars).map(|i| (x >> i) & 1 == 1).collect();
            clauses
                .iter()
                .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
                .then_some(a)
        })
        .collect()
}

/// The number of orbits of `models` under `group` (action `(a∘g)[j] = a[g[j]]`).
fn orbit_count(group: &[Perm], models: &[Vec<bool>]) -> usize {
    let mut seen: BTreeSet<Vec<bool>> = BTreeSet::new();
    let mut count = 0;
    for m in models {
        if seen.contains(m) {
            continue;
        }
        count += 1;
        for g in group {
            seen.insert((0..m.len()).map(|j| m[g[j]]).collect());
        }
    }
    count
}

/// Count the distinct projections onto the first `num_orig` variables among the models of `clauses`
/// over `total_vars`, by CDCL model-enumeration with blocking clauses.
fn count_projected_models(total_vars: usize, num_orig: usize, clauses: &[Vec<Lit>]) -> usize {
    let mut seen: BTreeSet<Vec<bool>> = BTreeSet::new();
    loop {
        let mut s = Solver::new(total_vars);
        for c in clauses {
            s.add_clause(c.clone());
        }
        for proj in &seen {
            s.add_clause(
                (0..num_orig)
                    .map(|v| Lit::new(v as u32, !proj[v]))
                    .collect(),
            );
        }
        match s.solve() {
            SolveResult::Sat(m) => {
                seen.insert((0..num_orig).map(|v| m[v]).collect());
            }
            SolveResult::Unsat => break,
        }
    }
    seen.len()
}

fn all_s_n(n: usize) -> Vec<Perm> {
    let mut out = Vec::new();
    let mut p: Perm = (0..n).collect();
    loop {
        out.push(p.clone());
        let Some(i) = (0..n.saturating_sub(1)).rev().find(|&i| p[i] < p[i + 1]) else {
            break;
        };
        let j = (i + 1..n).rev().find(|&j| p[j] > p[i]).unwrap();
        p.swap(i, j);
        p[i + 1..].reverse();
    }
    out
}

/// **The semantic leader test keeps exactly one per orbit.** Under the full `S_n` permuting `n`
/// coordinates, the orbits of `{0,1}ⁿ` are the Hamming-weight classes (`n+1` of them), and the
/// lex-leaders are exactly one per orbit.
#[test]
fn lex_leaders_are_one_per_orbit_under_s_n() {
    for n in 2..=5usize {
        let group = all_s_n(n);
        let all: Vec<Vec<bool>> = (0u64..(1u64 << n))
            .map(|x| (0..n).map(|i| (x >> i) & 1 == 1).collect())
            .collect();
        let leaders = all.iter().filter(|a| is_lex_leader(&group, a)).count();
        assert_eq!(
            leaders,
            n + 1,
            "S_{n} on the cube has n+1 weight-orbits, one leader each"
        );
        assert_eq!(leaders, orbit_count(&group, &all), "leaders == orbit count");
    }
}

/// **The CNF predicate accepts exactly the semantic leaders.** Enumerating the models of the SBP
/// (projected to the original variables) by CDCL reproduces the semantic leader count — the Tseitin
/// lex encoding is correct.
#[test]
fn the_cnf_sbp_accepts_exactly_the_lex_leaders() {
    for n in 2..=4usize {
        let group = all_s_n(n);
        let (sbp, total) = lex_leader_sbp(n, &group);
        let semantic = (0u64..(1u64 << n))
            .filter(|&x| {
                is_lex_leader(
                    &group,
                    &(0..n).map(|i| (x >> i) & 1 == 1).collect::<Vec<_>>(),
                )
            })
            .count();
        assert_eq!(
            count_projected_models(total, n, &sbp),
            semantic,
            "CNF SBP accepts exactly the leaders"
        );
        assert_eq!(semantic, n + 1, "and there are n+1 of them");
    }
}

/// **The affine-map SBP encodes the lex predicate exactly.** For an affine map `α` whose image bits are
/// XORs of *several* variables, the SBP (extended with its Tseitin/prefix-equality aux) is satisfiable
/// over `x` iff `x ≤_lex α(x)` — the machinery that breaks affine symmetries a permutation SBP cannot.
#[test]
fn affine_lex_leader_encodes_the_lex_predicate() {
    let n = 3usize;
    // α(x)[0] = x0 ⊕ x1, α(x)[1] = x1, α(x)[2] = x2 ⊕ x0.
    let map = vec![
        (vec![0usize, 1], false),
        (vec![1], false),
        (vec![2, 0], false),
    ];
    let (sbp, total) = affine_lex_leader_sbp(n, &[map]);
    let alpha = |x: u64| -> u64 {
        let a0 = (x & 1) ^ ((x >> 1) & 1);
        let a1 = (x >> 1) & 1;
        let a2 = ((x >> 2) & 1) ^ (x & 1);
        a0 | (a1 << 1) | (a2 << 2)
    };
    let lex_le = |x: u64, y: u64| -> bool {
        for j in 0..n {
            let (xj, yj) = ((x >> j) & 1, (y >> j) & 1);
            if xj != yj {
                return xj < yj;
            }
        }
        true
    };
    for x in 0u64..(1 << n) {
        let accepted = (0u64..(1u64 << (total - n))).any(|aux| {
            let full = x | (aux << n);
            sbp.iter().all(|c| {
                c.iter()
                    .any(|l| ((full >> l.var()) & 1 == 1) == l.is_positive())
            })
        });
        assert_eq!(
            accepted,
            lex_le(x, alpha(x)),
            "SBP must accept x={x:03b} iff x ≤_lex α(x)={:03b}",
            alpha(x)
        );
    }
}

/// **Partial (per-generator) breaking is sound but weaker than complete.** Passing only a generating
/// set keeps a *superset* of the canonical representatives — at least one per orbit (sound,
/// satisfiability-preserving), but generally more than the complete break's exactly-one. It is the
/// polynomial, scalable fallback for groups too large to enumerate.
#[test]
fn partial_generator_breaking_is_sound_but_weaker_than_complete() {
    let n = 3;
    let full = all_s_n(n); // 6 elements
    let gens: Vec<Perm> = vec![vec![1, 0, 2], vec![1, 2, 0]]; // (0 1) and (0 1 2) generate S_3
    let (complete, ct) = lex_leader_sbp(n, &full);
    let (partial, pt) = lex_leader_sbp(n, &gens);
    let complete_survivors = count_projected_models(ct, n, &complete);
    let partial_survivors = count_projected_models(pt, n, &partial);
    assert_eq!(
        complete_survivors,
        n + 1,
        "complete keeps exactly one per orbit (n+1 weight classes)"
    );
    assert!(
        partial_survivors >= complete_survivors && partial_survivors <= (1 << n),
        "partial keeps a superset (≥ complete, ≤ all): {partial_survivors} vs {complete_survivors}"
    );
    // Soundness on a satisfiable symmetric formula: at-least-one-true keeps a model under both.
    let f = vec![vec![Lit::pos(0), Lit::pos(1), Lit::pos(2)]];
    let mut with_partial = f.clone();
    with_partial.extend(partial);
    assert!(
        count_projected_models(pt, n, &with_partial) >= 1,
        "partial preserves satisfiability"
    );
}

/// **The stabilizer-chain break sits between complete and generators — stronger than the bare
/// generators, still polynomial.** Generators ∪ transversal coset reps is a *superset* of the
/// generators, so its lex-leader break is at least as strong (fewer-or-equal survivors), and it is a
/// subset of the whole group, so the complete break is at least as strong as it: complete ≤ chain ≤
/// generators. All are sound (every survivor count ≥ the orbit count).
#[test]
fn stabilizer_chain_break_is_between_complete_and_generators() {
    let n = 4;
    let gens: Vec<Perm> = vec![vec![1, 0, 2, 3], vec![1, 2, 3, 0]]; // (0 1), (0 1 2 3) generate S_4
    let bsgs = crate::permgroup::schreier_sims(n, &gens);
    let complete = bsgs.elements(100_000).unwrap();
    let mut chain = gens.clone();
    chain.extend(bsgs.transversal_elements());
    let survivors = |group: &[Perm]| {
        let (sbp, t) = lex_leader_sbp(n, group);
        count_projected_models(t, n, &sbp)
    };
    let (c, ch, g) = (survivors(&complete), survivors(&chain), survivors(&gens));
    assert_eq!(
        c,
        n + 1,
        "complete keeps exactly one per orbit (n+1 weight classes)"
    );
    assert!(
        c <= ch && ch <= g,
        "complete ≤ stabilizer-chain ≤ generators: {c} ≤ {ch} ≤ {g}"
    );
    assert!(g <= (1usize << n), "all sound (≤ total assignments)");
}

/// **Value (phase) symmetry is broken — what the variable-only scheme cannot see.** `F = (x₀∨x₁) ∧
/// (¬x₀∨x₁)` is invariant under *flipping* `x₀` (`x₀ ↦ ¬x₀`); its two models `(0,1),(1,1)` form one
/// orbit under the flip, and the literal lex-leader keeps exactly one. The phase-free variable scheme
/// is blind to it (its only generator flips a phase).
#[test]
fn value_phase_symmetry_is_broken() {
    let f = vec![
        vec![Lit::pos(0), Lit::pos(1)],
        vec![Lit::neg(0), Lit::pos(1)],
    ];
    assert!(
        variable_automorphism_generators(2, &f).is_none_or(|g| g.is_empty()),
        "the phase-free variable scheme is blind to the value symmetry"
    );
    let gens = literal_automorphism_generators(2, &f);
    assert!(
        !gens.is_empty(),
        "the value symmetry x₀ ↦ ¬x₀ is detected as a literal symmetry"
    );
    // It must satisfy the negation-respecting round-trip through the literal points.
    for s in &gens {
        assert_eq!(
            litsym_from_points(&litsym_to_points(s, 2), 2),
            *s,
            "litsym ↔ points round-trips"
        );
    }
    let (sbp, total) = lex_leader_sbp_lit(2, &gens);
    let mut broken = f.clone();
    broken.extend(sbp);
    assert_eq!(
        count_projected_models(total, 2, &broken),
        1,
        "value symmetry broken to one model"
    );
    // Sanity: F itself has two models, so the symmetry genuinely halved them.
    assert_eq!(
        models(2, &f).len(),
        2,
        "F has two models, one orbit under the flip"
    );
}

/// **Complete symmetry breaking on a real formula: one model per orbit, satisfiability preserved.**
/// `clique_coloring(3,3)` (proper 3-colourings of K₃) has 6 models in a single orbit under
/// `S₃(vertices) × S₃(colours)`; the complete lex-leader SBP leaves exactly one, and the formula stays
/// SAT. PHP(3) is UNSAT and stays UNSAT under the SBP (soundness on the unsatisfiable side).
#[test]
fn complete_lex_leader_keeps_one_model_per_orbit_end_to_end() {
    // SAT, symmetric.
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let group = variable_automorphism_group(cnf.num_vars, &cnf.clauses, 100_000)
        .expect("clique colouring has a phase-free, small automorphism group");
    let ms = models(cnf.num_vars, &cnf.clauses);
    let orbits = orbit_count(&group, &ms);
    let (sbp, total) = lex_leader_sbp(cnf.num_vars, &group);
    let mut broken = cnf.clauses.clone();
    broken.extend(sbp);
    let surviving = count_projected_models(total, cnf.num_vars, &broken);
    assert_eq!(
        surviving, orbits,
        "complete SBP leaves exactly one model per orbit"
    );
    assert!(
        orbits >= 1 && surviving < ms.len(),
        "and it strictly breaks the symmetry"
    );

    // UNSAT, symmetric: the SBP must preserve unsatisfiability.
    let (php, _) = crate::families::php(3);
    let pg = variable_automorphism_group(php.num_vars, &php.clauses, 100_000).expect("PHP group");
    let (psbp, ptotal) = lex_leader_sbp(php.num_vars, &pg);
    let mut pbroken = php.clauses.clone();
    pbroken.extend(psbp);
    assert_eq!(
        count_projected_models(ptotal, php.num_vars, &pbroken),
        0,
        "UNSAT stays UNSAT"
    );
}

/// **Conditional (local) symmetry emerges under a partial assignment.** `F` is globally asymmetric —
/// the clause `x₀∨x₁` singles out `x₁` — so there is no global `x₁↔x₂` symmetry. But under `x₀ = true`
/// the residual is `(x₁∨x₂) ∧ (¬x₁∨¬x₂)`, symmetric under swapping `x₁ ↔ x₂`: a symmetry that exists
/// only down that branch. A different symmetry source — the residual's, not the formula's.
#[test]
fn conditional_symmetry_emerges_under_a_partial_assignment() {
    let f = vec![
        vec![Lit::neg(0), Lit::pos(1), Lit::pos(2)], // ¬x0 ∨ x1 ∨ x2
        vec![Lit::neg(0), Lit::neg(1), Lit::neg(2)], // ¬x0 ∨ ¬x1 ∨ ¬x2
        vec![Lit::pos(0), Lit::pos(1)],              // x0 ∨ x1   (breaks the global x1↔x2 symmetry)
    ];
    let swaps_12 = |gens: &[Vec<Lit>]| {
        gens.iter()
            .any(|img| img[1] == Lit::pos(2) && img[2] == Lit::pos(1))
    };
    // No global x1↔x2 symmetry.
    assert!(
        !swaps_12(&literal_automorphism_generators(3, &f)),
        "F has no global x1↔x2 symmetry"
    );
    // Conditionally on x0 = true, the residual is symmetric under x1↔x2.
    let local = conditional_symmetry_generators(3, &f, &[Lit::pos(0)]);
    assert!(
        swaps_12(&local),
        "the residual under x0=true has the x1↔x2 symmetry: {local:?}"
    );
}

/// **Hierarchical (block-wise) breaking: a polynomial break of an exponential grid symmetry.** On
/// `clique_coloring(3,3)` (`S₃×S₃`, `|G| = 36`), the block-wise break uses only the adjacent
/// vertex-row swaps and uniform colour swaps — a handful of structured generators — yet is sound
/// (keeps ≥ one model per orbit) and genuinely breaks the symmetry (fewer survivors than models).
#[test]
fn hierarchical_block_wise_breaking_is_sound_and_polynomial() {
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let nv = cnf.num_vars;
    let (sbp, total) =
        hierarchical_break(nv, &cnf.clauses).expect("clique colouring is a grid symmetry");
    let ms = models(nv, &cnf.clauses);
    let var_group = variable_automorphism_group(nv, &cnf.clauses, 100_000).unwrap();
    let orbits = orbit_count(&var_group, &ms);
    let mut broken = cnf.clauses.clone();
    broken.extend(sbp);
    let surviving = count_projected_models(total, nv, &broken);
    assert!(
        surviving >= orbits,
        "hierarchical break is sound: ≥ one model per orbit ({surviving} ≥ {orbits})"
    );
    assert!(
        surviving < ms.len(),
        "and it breaks the symmetry: fewer survivors than the {} models",
        ms.len()
    );
}

/// **PHP's symmetry is an imprimitive grid — symmetry within the symmetry.** PHP(n)'s variable group
/// `S_n × S_{n-1}` acts on the `n × (n-1)` grid of variables `x_{p,h}`; it is **imprimitive**, and the
/// pigeon rows are the minimal block system (`n` blocks of size `n-1`). The block structure lays the
/// grid bare — the internal structure of the formula's symmetry, recovered group-theoretically.
#[test]
fn php_symmetry_is_an_imprimitive_grid() {
    let n = 4;
    let (cnf, _) = crate::families::php(n);
    let gens = variable_automorphism_generators(cnf.num_vars, &cnf.clauses).expect("phase-free");
    assert!(
        !crate::permgroup::is_primitive(cnf.num_vars, &gens),
        "PHP's symmetry is imprimitive — it is a grid, not an atom"
    );
    let blocks = crate::permgroup::minimal_block_system(cnf.num_vars, &gens)
        .expect("PHP has a block system");
    assert!(
        blocks.iter().all(|b| b.len() == n - 1),
        "blocks are pigeon-rows of size n-1: {blocks:?}"
    );
    assert_eq!(blocks.len(), n, "there are n pigeon-rows");
    for b in &blocks {
        let pigeon = b[0] / (n - 1);
        assert!(
            b.iter().all(|&v| v / (n - 1) == pigeon),
            "each block is exactly one pigeon's row: {b:?}"
        );
    }
}

/// **Counting up to symmetry = the orbit count, three ways.** The number of essentially-distinct
/// solutions equals the complete-SBP model count, the brute-force orbit count, AND Burnside's lemma
/// `(1/|G|)·Σ_σ #{models fixed by σ}` — the canonical orbit-counting theorem. On `clique_coloring(3,3)`
/// all six proper colourings lie in a single orbit, so there is essentially one solution.
#[test]
fn count_modulo_symmetry_equals_burnside_and_brute_orbit_count() {
    let (cnf, _) = crate::families::clique_coloring(3, 3);
    let nv = cnf.num_vars;
    let group = literal_automorphism_group(nv, &cnf.clauses, 100_000).unwrap();
    let ms = models(nv, &cnf.clauses);
    let image = |img: &[Lit], a: &[bool]| -> Vec<bool> {
        (0..a.len())
            .map(|j| {
                if img[j].is_positive() {
                    a[img[j].var() as usize]
                } else {
                    !a[img[j].var() as usize]
                }
            })
            .collect()
    };
    // Brute-force orbit count under the literal action.
    let brute = {
        let mut seen: BTreeSet<Vec<bool>> = BTreeSet::new();
        let mut c = 0;
        for m in &ms {
            if seen.contains(m) {
                continue;
            }
            c += 1;
            for s in &group {
                seen.insert(image(s, m));
            }
        }
        c
    };
    // Burnside: the average number of fixed models over the group.
    let fixed: usize = group
        .iter()
        .map(|s| ms.iter().filter(|&m| image(s, m) == *m).count())
        .sum();
    let burnside = fixed / group.len();
    let counted = count_models_modulo_symmetry(nv, &cnf.clauses).unwrap();
    assert_eq!(counted, brute, "complete-SBP count == brute orbit count");
    assert_eq!(counted, burnside, "== Burnside count");
    assert_eq!(
        counted, 1,
        "clique_coloring(3,3): all six proper colourings are one orbit"
    );
    assert_eq!(ms.len(), 6, "and there are six of them");
}
