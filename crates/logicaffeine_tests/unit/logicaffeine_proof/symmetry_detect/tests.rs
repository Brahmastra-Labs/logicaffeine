use super::*;
use crate::cdcl::Lit;
use crate::families;
use crate::proof::Perm;

/// Build the permutation that swaps two pigeon rows of PHP(n): for every hole, exchange the
/// two pigeons' variables. This is a textbook automorphism of the pigeonhole formula.
fn swap_pigeon_rows(n: usize, p0: usize, p1: usize) -> Perm {
    let holes = n - 1;
    let images: Vec<Lit> = (0..n * holes)
        .map(|v| {
            let (p, h) = (v / holes, v % holes);
            let np = if p == p0 {
                p1
            } else if p == p1 {
                p0
            } else {
                p
            };
            Lit::pos((np * holes + h) as u32)
        })
        .collect();
    Perm::from_images(images)
}

#[test]
fn identity_is_always_an_automorphism() {
    let (cnf, _) = families::php(4);
    assert!(perm_is_automorphism(
        &cnf.clauses,
        &Perm::identity(cnf.num_vars)
    ));
}

#[test]
fn swapping_pigeon_rows_is_an_automorphism_of_php() {
    let (cnf, _) = families::php(4);
    for (p0, p1) in [(0, 1), (1, 2), (0, 3), (2, 3)] {
        let sigma = swap_pigeon_rows(4, p0, p1);
        assert!(
            perm_is_automorphism(&cnf.clauses, &sigma),
            "swapping pigeons {p0},{p1} must preserve PHP(4)"
        );
    }
}

#[test]
fn support_restricted_check_matches_brute_force_set_equality() {
    // The independent oracle: σ is an automorphism iff `{σ(C):C∈F}` equals `{C:C∈F}` as sets.
    // Over many seeded random small formulas and random literal permutations, the fast
    // support-restricted `perm_is_automorphism` must return the IDENTICAL verdict — the
    // soundness net for the speedup, robust to absurdity.
    let mut state = 0xA5A5_5A5A_DEAD_BEEFu64;
    let mut next = || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let num_vars = 5usize;
    let brute = |clauses: &[Vec<Lit>], sigma: &Perm| -> bool {
        use std::collections::BTreeSet;
        let orig: BTreeSet<Vec<u32>> = clauses.iter().map(|c| clause_key(c)).collect();
        let mapped: BTreeSet<Vec<u32>> = clauses
            .iter()
            .map(|c| clause_key(&sigma.apply_clause(c)))
            .collect();
        orig == mapped
    };
    let mut accepts = 0;
    for _ in 0..20_000 {
        let nclauses = next() as usize % 6;
        let clauses: Vec<Vec<Lit>> = (0..nclauses)
            .map(|_| {
                let len = 1 + (next() as usize % 3);
                let mut c = Vec::new();
                for _ in 0..len {
                    let v = (next() as u32) % num_vars as u32;
                    let lit = Lit::new(v, next() & 1 == 0);
                    if !c.contains(&lit) && !c.contains(&lit.negated()) {
                        c.push(lit);
                    }
                }
                c
            })
            .filter(|c| !c.is_empty())
            .collect();
        let sigma = {
            let mut order: Vec<u32> = (0..num_vars as u32).collect();
            for i in (1..num_vars).rev() {
                let j = next() as usize % (i + 1);
                order.swap(i, j);
            }
            Perm::from_images(
                (0..num_vars)
                    .map(|v| Lit::new(order[v], next() & 1 == 0))
                    .collect(),
            )
        };
        let fast = perm_is_automorphism(&clauses, &sigma);
        assert_eq!(
            fast,
            brute(&clauses, &sigma),
            "fast vs brute disagree: clauses={clauses:?}"
        );
        if fast {
            accepts += 1;
        }
    }
    assert!(
        accepts > 0,
        "the differential must exercise genuine acceptances, not just rejects"
    );
}

#[test]
fn incremental_index_matches_stateless_automorphism_check() {
    // The incremental `AutomorphismIndex` must give the IDENTICAL verdict to the stateless
    // `perm_is_automorphism` — over many seeded random formulas built up one clause at a time
    // (exercising `insert`) and many random permutations. The soundness net for the speedup.
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    let mut next = || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let num_vars = 5usize;
    let mut agree = 0;
    for _ in 0..20_000 {
        let nclauses = next() as usize % 7;
        let clauses: Vec<Vec<Lit>> = (0..nclauses)
            .map(|_| {
                let len = 1 + (next() as usize % 3);
                let mut c = Vec::new();
                for _ in 0..len {
                    let v = (next() as u32) % num_vars as u32;
                    let lit = Lit::new(v, next() & 1 == 0);
                    if !c.contains(&lit) && !c.contains(&lit.negated()) {
                        c.push(lit);
                    }
                }
                c
            })
            .filter(|c| !c.is_empty())
            .collect();
        // Build the index incrementally to exercise insert().
        let mut ix = AutomorphismIndex::new(num_vars);
        for c in &clauses {
            ix.insert(c.clone());
        }
        let sigma = {
            let mut order: Vec<u32> = (0..num_vars as u32).collect();
            for i in (1..num_vars).rev() {
                let j = next() as usize % (i + 1);
                order.swap(i, j);
            }
            Perm::from_images(
                (0..num_vars)
                    .map(|v| Lit::new(order[v], next() & 1 == 0))
                    .collect(),
            )
        };
        assert_eq!(
            ix.is_automorphism(&sigma),
            perm_is_automorphism(&clauses, &sigma),
            "incremental index disagrees with stateless check on {clauses:?}"
        );
        agree += 1;
    }
    assert_eq!(agree, 20_000);
}

#[test]
fn occurrence_propagation_matches_full_scan_propagation() {
    // The occurrence-driven `propagate_to_conflict` must reach the IDENTICAL conflict verdict
    // as the trusted full-scan `rup::propagate` — over many seeded random formulas and random
    // assumption sets (including duplicate-literal and tautological clauses). The soundness
    // net for routing the SR check's propagation through the index.
    let mut state = 0x0F0F_F0F0_1357_9BDFu64;
    let mut next = || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let num_vars = 6usize;
    let reference = |clauses: &[Vec<Lit>], assume: &[Lit]| -> bool {
        let mut assign: Vec<Option<bool>> = vec![None; num_vars];
        for &l in assume {
            if !crate::rup::set_true(&mut assign, l) {
                return true;
            }
        }
        crate::rup::propagate(clauses, &mut assign)
    };
    let mut conflicts = 0;
    for _ in 0..20_000 {
        let nclauses = next() as usize % 8;
        let clauses: Vec<Vec<Lit>> = (0..nclauses)
            .map(|_| {
                let len = 1 + (next() as usize % 4);
                (0..len)
                    .map(|_| Lit::new((next() as u32) % num_vars as u32, next() & 1 == 0))
                    .collect()
            })
            .filter(|c: &Vec<Lit>| !c.is_empty())
            .collect();
        let mut ix = AutomorphismIndex::new(num_vars);
        for c in &clauses {
            ix.insert(c.clone());
        }
        let nassume = next() as usize % 4;
        let assume: Vec<Lit> = (0..nassume)
            .map(|_| Lit::new((next() as u32) % num_vars as u32, next() & 1 == 0))
            .collect();
        let got = ix.propagate_to_conflict(num_vars, &assume);
        assert_eq!(
            got,
            reference(&clauses, &assume),
            "clauses={clauses:?} assume={assume:?}"
        );
        if got {
            conflicts += 1;
        }
    }
    assert!(
        conflicts > 0,
        "the differential must exercise genuine conflicts"
    );
}

#[test]
fn a_non_symmetry_is_rejected() {
    // Map only pigeon 0's row to pigeon 1's row but NOT vice-versa: this collapses two rows
    // onto one, so it is not a bijection-preserving automorphism of the clause set.
    let (cnf, _) = families::php(3);
    let holes = 2;
    let images: Vec<Lit> = (0..cnf.num_vars)
        .map(|v| {
            let (p, h) = (v / holes, v % holes);
            let np = if p == 0 { 1 } else { p };
            Lit::pos((np * holes + h) as u32)
        })
        .collect();
    assert!(!perm_is_automorphism(
        &cnf.clauses,
        &Perm::from_images(images)
    ));
}

// --- the general finder ---

use std::collections::HashSet;

fn p(v: u32) -> Lit {
    Lit::pos(v)
}
fn neg(v: u32) -> Lit {
    Lit::neg(v)
}

/// All permutations of `0..n`.
fn all_var_perms(n: usize) -> Vec<Vec<usize>> {
    fn rec(cur: &mut Vec<usize>, rem: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if rem.is_empty() {
            out.push(cur.clone());
            return;
        }
        for i in 0..rem.len() {
            let x = rem.remove(i);
            cur.push(x);
            rec(cur, rem, out);
            cur.pop();
            rem.insert(i, x);
        }
    }
    let mut out = Vec::new();
    rec(&mut Vec::new(), &mut (0..n).collect(), &mut out);
    out
}

/// The TRUE symmetry group of `clauses`: every negation-respecting literal permutation
/// (variable bijection × per-variable phase) that is an automorphism, as a set of keys.
fn brute_force_group(num_vars: usize, clauses: &[Vec<Lit>]) -> HashSet<Vec<u32>> {
    let mut set = HashSet::new();
    for vp in all_var_perms(num_vars) {
        for phase in 0..(1u32 << num_vars) {
            let images: Vec<Lit> = (0..num_vars)
                .map(|v| Lit::new(vp[v] as u32, (phase >> v) & 1 == 0))
                .collect();
            let perm = Perm::from_images(images);
            if perm_is_automorphism(clauses, &perm) {
                set.insert(perm_key(num_vars, &perm));
            }
        }
    }
    set
}

/// The set of element keys generated by `gens` (closure under composition, with identity).
fn generated_group(num_vars: usize, gens: &[Perm]) -> HashSet<Vec<u32>> {
    let id = Perm::identity(num_vars);
    let mut set = HashSet::new();
    set.insert(perm_key(num_vars, &id));
    let mut frontier = vec![id];
    while let Some(x) = frontier.pop() {
        for g in gens {
            let y = g.compose(&x);
            let k = perm_key(num_vars, &y);
            if set.insert(k) {
                frontier.push(y);
            }
        }
    }
    set
}

#[test]
fn finds_the_full_php3_symmetry_group() {
    let (cnf, _) = families::php(3);
    let gens = find_generators(cnf.num_vars, &cnf.clauses);
    let group = generated_group(cnf.num_vars, &gens);
    // PHP(3): permute 3 pigeons × 2 holes = S_3 × S_2, order 12.
    assert_eq!(
        group.len(),
        12,
        "discovered generators must generate the full S_3 × S_2"
    );
}

#[test]
fn discovered_generators_match_brute_force_across_cases() {
    // (num_vars, clauses): PHP(2), PHP(3), a swap-symmetric pair, and an asymmetric formula.
    let php2 = families::php(2).0;
    let php3 = families::php(3).0;
    let cases: Vec<(usize, Vec<Vec<Lit>>)> = vec![
        (php2.num_vars, php2.clauses),
        (php3.num_vars, php3.clauses),
        (2, vec![vec![p(0), p(1)], vec![neg(0), neg(1)]]), // exactly-one(a,b): a↔b symmetric
        (3, vec![vec![p(0)], vec![p(0), p(1)]]),           // asymmetric: only identity
        (3, vec![vec![p(0), p(1), p(2)]]),                 // a single symmetric clause: S_3
    ];
    for (num_vars, clauses) in cases {
        let gens = find_generators(num_vars, &clauses);
        let found = generated_group(num_vars, &gens);
        let truth = brute_force_group(num_vars, &clauses);
        assert_eq!(
            found, truth,
            "finder must generate exactly Aut(F) for {clauses:?}"
        );
    }
}

#[test]
fn every_discovered_generator_is_a_verified_automorphism() {
    let (cnf, _) = families::php(4);
    let gens = find_generators(cnf.num_vars, &cnf.clauses);
    assert!(
        !gens.is_empty(),
        "PHP(4) is highly symmetric — generators must be found"
    );
    for g in &gens {
        assert!(
            perm_is_automorphism(&cnf.clauses, g),
            "every returned generator is sound"
        );
    }
}

#[test]
fn an_asymmetric_formula_yields_no_nontrivial_generators() {
    let clauses = vec![vec![p(0)], vec![p(0), p(1)], vec![neg(1), p(2)]];
    let gens = find_generators(3, &clauses);
    assert!(
        gens.iter().all(|g| g.is_identity()),
        "no non-trivial symmetry to find"
    );
}

/// **Schreier–Sims as the symmetry backend computes `|Aut(F)|` exactly.** PHP(n) has automorphism
/// group `S_n` (pigeons) × `S_{n−1}` (holes) ⟹ `|Aut| = n!·(n−1)!`; complete-graph `k`-colouring has
/// `S_n × S_k` ⟹ `|Aut| = n!·k!`. The backend reproduces both from the detected generators — a group
/// computation the symmetry layer could not previously perform.
#[test]
fn the_bsgs_backend_computes_the_automorphism_group_order() {
    let fact = |k: u128| (1..=k).product::<u128>();
    for n in 3..=5usize {
        let (cnf, _) = crate::families::php(n);
        assert_eq!(
            aut_order(cnf.num_vars, &cnf.clauses),
            fact(n as u128) * fact((n - 1) as u128),
            "|Aut(PHP({n}))| = n!·(n-1)!"
        );
    }
    for &(n, k) in &[(4usize, 3usize), (5, 3)] {
        let (cnf, _) = crate::families::clique_coloring(n, k);
        assert_eq!(
            aut_order(cnf.num_vars, &cnf.clauses),
            fact(n as u128) * fact(k as u128),
            "|Aut(clique_coloring({n},{k}))| = n!·k!"
        );
    }
}

/// The backend decides membership / cosets in the automorphism group: every detected generator and
/// the identity are members; a variable swap that is not a global symmetry is rejected.
#[test]
fn the_bsgs_backend_decides_automorphism_membership() {
    let (cnf, _) = crate::families::php(3); // |Aut| = 12, S_3 × S_2
    let nv = cnf.num_vars;
    let bsgs = automorphism_group(nv, &cnf.clauses);
    for g in find_generators(nv, &cnf.clauses) {
        assert!(
            bsgs.contains(&literal_perm_to_points(nv, &g)),
            "a detected generator is a member"
        );
    }
    assert!(
        bsgs.contains(&(0..2 * nv).collect::<Vec<_>>()),
        "the identity is a member"
    );
    // Swapping only hole 0 / hole 1 of pigeon 0 (variables 0 and 1) is not a global symmetry.
    let mut bad: Vec<usize> = (0..2 * nv).collect();
    bad.swap(0, 2); // +x0 ↔ +x1
    bad.swap(1, 3); // ¬x0 ↔ ¬x1
    assert!(
        !bsgs.contains(&bad),
        "a non-automorphism variable swap must be rejected"
    );
}

#[test]
fn a_rigid_random_formula_refines_to_discrete_and_has_no_generators() {
    let cnf = crate::families::random_3sat(3000, 12_000, 0x51D);
    let start = std::time::Instant::now();
    let gens = find_generators(cnf.num_vars, &cnf.clauses);
    assert!(gens.is_empty(), "a random formula of this size has no symmetry");
    assert!(start.elapsed().as_secs() < 60, "refinement alone decides it ({:?})", start.elapsed());
}

#[test]
fn disjoint_copies_of_a_formula_have_the_wreath_product_group() {
    // PHP(3) (3 pigeons, 2 holes) has |Aut| = 3!·2! = 12; k disjoint copies have 12^k · k!.
    let (base_vars, base) = {
        let (cnf, _) = crate::families::php(3);
        (cnf.num_vars, cnf.clauses)
    };
    assert_eq!(aut_order(base_vars, &base), 12);
    for k in 2..=4usize {
        let mut clauses: Vec<Vec<Lit>> = Vec::new();
        for copy in 0..k {
            let shift = (copy * base_vars) as u32;
            for c in &base {
                clauses.push(c.iter().map(|l| Lit::new(l.var() + shift, l.is_positive())).collect());
            }
        }
        let factorial: u128 = (1..=k as u128).product();
        assert_eq!(aut_order(k * base_vars, &clauses), 12u128.pow(k as u32) * factorial, "{k} copies");
        for g in find_generators(k * base_vars, &clauses) {
            assert!(perm_is_automorphism(&clauses, &g));
        }
    }
}
