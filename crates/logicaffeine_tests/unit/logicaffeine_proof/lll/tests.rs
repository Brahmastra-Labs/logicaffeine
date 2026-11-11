use super::*;

fn brute_sat(nv: usize, clauses: &[Vec<Lit>]) -> bool {
    (0u64..(1u64 << nv)).any(|x| {
        clauses.iter().all(|c| {
            c.iter()
                .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
        })
    })
}

// A decorrelated instance seed — never seed along SplitMix64's own increment γ (see the
// seed-collapse lesson: `s·γ` makes consecutive trials the same stream shifted by one).
fn seed_of(tag: u64, i: u64) -> u64 {
    let mut z = tag
        .wrapping_mul(0xD1B5_4A32_D192_ED03)
        .wrapping_add(i)
        .wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// **The LLL certificate never lies — soundness against brute force.** Over a sweep of sparse-to-dense
/// random k-SAT, whenever `lll_certifies_sat` fires, brute force must confirm the instance is
/// satisfiable. The certificate is a *sufficient* condition, so we only check the one direction it
/// claims (Some ⟹ SAT); and we require it to fire at least once so the test is not vacuous.
#[test]
fn lll_certificate_is_sound_against_brute_force() {
    let n = 14usize;
    let mut fired = 0usize;
    for k in 3..=5usize {
        // very low densities — this is where local sparsity holds
        for &num in &[2usize, 3, 4, 6, 8, 10, 14, 20] {
            for t in 0..12u64 {
                let tag = (k as u64) << 40 ^ (num as u64) << 20;
                let cnf = crate::families::random_ksat(k, n, num, seed_of(tag, t));
                if let Some(cert) = lll_certifies_sat(&cnf.clauses) {
                    fired += 1;
                    assert!(
                        brute_sat(n, &cnf.clauses),
                        "LLL certified SAT but brute force says UNSAT: k={k} m={num} cert={cert:?}"
                    );
                    // the certificate re-checks: recomputing degree and width reproduces it
                    assert_eq!(lll_dependency_degree(&cnf.clauses), cert.max_degree);
                }
            }
        }
    }
    assert!(
        fired >= 5,
        "the LLL certificate fired on too few instances to be meaningful: {fired}"
    );
}

/// **Moser–Tardos constructs the witness whenever the LLL certifies.** The certificate is not just an
/// existence proof — when it fires, resampling reaches an actual model, and that model is verified to
/// satisfy every clause. This makes the LLL the constructive, witness-producing lower bookend.
#[test]
fn moser_tardos_constructs_a_model_under_the_lll_condition() {
    let n = 16usize;
    let mut constructed = 0usize;
    for k in 3..=5usize {
        for &num in &[2usize, 3, 4, 6, 8, 12] {
            for t in 0..10u64 {
                let tag = (k as u64) << 40 ^ (num as u64) << 20 ^ 0xA5;
                let cnf = crate::families::random_ksat(k, n, num, seed_of(tag, t));
                if lll_certifies_sat(&cnf.clauses).is_some() {
                    let model = moser_tardos_witness(
                        n,
                        &cnf.clauses,
                        seed_of(tag ^ 0xF00D, t),
                        100 * num.max(1),
                    )
                    .expect("under the LLL condition Moser–Tardos must reach a model");
                    assert!(
                        cnf.clauses
                            .iter()
                            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                        "the constructed witness must satisfy every clause"
                    );
                    constructed += 1;
                }
            }
        }
    }
    assert!(
        constructed >= 5,
        "constructed too few witnesses to be meaningful: {constructed}"
    );
}

/// **The two bookends are consistent and complementary.** A sparse instance the LLL certifies SAT sits
/// far *below* the first-moment density `α*(k)` (which only forbids satisfiability *above* it) — so the
/// SAT certificate and the UNSAT bound never disagree, they bracket the threshold from opposite sides.
#[test]
fn lll_sat_region_sits_below_the_first_moment_unsat_bound() {
    let n = 16usize;
    let k = 4usize;
    let alpha_star = crate::families::ksat_threshold_first_moment_upper(k as u32);
    let mut checked = 0usize;
    for &num in &[3usize, 4, 5] {
        for t in 0..8u64 {
            let cnf =
                crate::families::random_ksat(k, n, num, seed_of(0xBEEF, (num as u64) << 8 ^ t));
            if lll_certifies_sat(&cnf.clauses).is_some() {
                let alpha = num as f64 / n as f64;
                assert!(alpha < alpha_star, "an LLL-certified instance must lie below α*({k})={alpha_star:.3}, got α={alpha:.3}");
                assert!(brute_sat(n, &cnf.clauses), "and it is genuinely SAT");
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 1,
        "expected at least one LLL-certified sparse instance"
    );
}

fn pairwise_degree(clauses: &[Vec<Lit>]) -> usize {
    (0..clauses.len())
        .map(|i| {
            (0..clauses.len())
                .filter(|&j| j != i && clauses[i].iter().any(|a| clauses[j].iter().any(|b| a.var() == b.var())))
                .count()
        })
        .max()
        .unwrap_or(0)
}

/// Clauses of width `lo..=hi` over `n` variables, a literal sometimes repeated (a repeat leaves the clause's
/// neighbours as they are but counts toward its width).
fn random_formula(next: &mut impl FnMut() -> u64, n: usize, m: usize, lo: usize, hi: usize) -> Vec<Vec<Lit>> {
    (0..m)
        .map(|_| {
            let width = lo + (next() % (hi - lo + 1) as u64) as usize;
            let mut c: Vec<Lit> = Vec::with_capacity(width);
            while c.len() < width {
                let l = if !c.is_empty() && next() % 9 == 0 {
                    c[(next() % c.len() as u64) as usize]
                } else {
                    Lit::new((next() % n as u64) as u32, next() & 1 == 0)
                };
                c.push(l);
            }
            c
        })
        .collect()
}

fn stream(seed: u64) -> impl FnMut() -> u64 {
    let mut i = 0u64;
    move || {
        i += 1;
        seed_of(seed, i)
    }
}

/// The dependency degree is its definition: for each clause, the other clauses sharing a variable with it, the most
/// of any clause.
#[test]
fn the_dependency_degree_is_the_pairwise_definition() {
    let mut next = stream(0xDE6);
    assert_eq!(lll_dependency_degree(&[]), 0);
    for round in 0..2000u64 {
        let n = 1 + (next() % 24) as usize;
        let m = (next() % 40) as usize;
        let clauses = random_formula(&mut next, n, m, 1, 5);
        assert_eq!(lll_dependency_degree(&clauses), pairwise_degree(&clauses), "round {round}: n={n} m={m}");
    }
}

/// The certificate is exactly the symmetric condition `e · 2^{−w_min} · (d+1) ≤ 1` on the formula's own minimum
/// width and degree: it fires when the condition holds, with those two numbers, and declines when it does not.
#[test]
fn the_certificate_is_exactly_the_symmetric_condition() {
    let mut next = stream(0x5EC7);
    let (mut fired, mut declined) = (0usize, 0usize);
    for round in 0..3000u64 {
        let lo = 1 + (next() % 7) as usize;
        let n = lo + (next() % 40) as usize;
        let m = 1 + (next() % 30) as usize;
        let clauses = random_formula(&mut next, n, m, lo, lo + 2);
        let width = clauses.iter().map(|c| c.len()).min().unwrap();
        let degree = pairwise_degree(&clauses);
        let holds = std::f64::consts::E * 2f64.powi(-(width as i32)) * (degree as f64 + 1.0) <= 1.0;
        let expected = holds.then_some(LllSatCert { min_width: width, max_degree: degree });
        assert_eq!(lll_certifies_sat(&clauses), expected, "round {round}: width {width}, degree {degree}");
        if holds {
            fired += 1;
        } else {
            declined += 1;
        }
    }
    assert!(fired >= 200 && declined >= 200, "both sides of the condition are exercised: {fired} fired, {declined} declined");
}

/// On a star — `m` clauses of width `w` through one shared variable, each otherwise on its own variables — every
/// clause has degree `m − 1`, so the certificate holds up to the largest `d` with `d + 1 ≤ 2ʷ/e` and not one clause
/// further: `d ≤ 4` at width 4 (2⁴/e ≈ 5.89), `d ≤ 10` at 5 (≈ 11.77), `d ≤ 22` at 6 (≈ 23.54), `d ≤ 93` at 8
/// (≈ 94.18). At width 1 nothing is certified (e/2 > 1), and width 2 admits only clauses that share no variable.
#[test]
fn the_certificate_boundary_is_sharp_on_a_star() {
    let star = |w: usize, m: usize| -> Vec<Vec<Lit>> {
        (0..m)
            .map(|i| {
                let mut c = vec![Lit::pos(0)];
                c.extend((1..w).map(|k| Lit::neg((1 + i * (w - 1) + k - 1) as u32)));
                c
            })
            .collect()
    };
    for (w, largest) in [(2usize, 0usize), (3, 1), (4, 4), (5, 10), (6, 22), (8, 93)] {
        assert_eq!(
            lll_certifies_sat(&star(w, largest + 1)),
            Some(LllSatCert { min_width: w, max_degree: largest }),
            "width {w}: degree {largest} is certified"
        );
        assert_eq!(lll_certifies_sat(&star(w, largest + 2)), None, "width {w}: degree {} is not", largest + 1);
    }
    assert_eq!(lll_certifies_sat(&[vec![Lit::pos(0)]]), None, "a unit clause alone fails e/2 ≤ 1");
    assert_eq!(lll_certifies_sat(&[vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(2), Lit::pos(3)]]), Some(LllSatCert { min_width: 2, max_degree: 0 }));
}

/// Declining a formula whose variables occur densely costs one pass over it, not a neighbour union per clause: a
/// variable in more clauses than the certificate's degree bound allows declines it outright. Each formula here would
/// take ~10¹¹ union steps clause by clause; the check must answer within a minute even unoptimized on a loaded box.
#[test]
fn declining_a_dense_formula_reads_it_once() {
    let dense: Vec<(usize, Vec<Vec<Lit>>)> = vec![
        // Binary clauses through one variable: width 2 admits no shared variable at all.
        (2, (1..=400_000u32).map(|v| vec![Lit::pos(0), Lit::neg(v)]).collect()),
        // Width-8 clauses through one variable: at most 93 neighbours are admitted, and it has 199,999.
        (8, (0..200_000u32).map(|i| std::iter::once(Lit::pos(0)).chain((1..8).map(|k| Lit::neg(1 + 7 * i + k))).collect()).collect()),
    ];
    for (w, clauses) in dense {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(lll_certifies_sat(&clauses));
        });
        let got = rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .unwrap_or_else(|_| panic!("width {w}: declining a dense formula reads it once, not every clause's neighbours"));
        assert_eq!(got, None, "width {w}");
    }
}

/// Moser–Tardos as stated: from a seeded assignment, resample the first violated clause, scanning the formula for it
/// at every step.
fn moser_tardos_by_scanning(num_vars: usize, clauses: &[Vec<Lit>], seed: u64, max_resamples: usize) -> Option<Vec<bool>> {
    let mut rng = seed;
    let mut next = move || {
        rng = rng.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let mut assign: Vec<bool> = (0..num_vars).map(|_| next() & 1 == 0).collect();
    let violated = |a: &[bool], c: &[Lit]| c.iter().all(|l| a[l.var() as usize] != l.is_positive());
    for _ in 0..max_resamples {
        match clauses.iter().find(|c| violated(&assign, c)) {
            None => return Some(assign),
            Some(c) => {
                for l in c {
                    assign[l.var() as usize] = next() & 1 == 0;
                }
            }
        }
    }
    clauses.iter().all(|c| !violated(&assign, c)).then_some(assign)
}

/// The witness search resamples exactly the clauses the stated algorithm does, in the same order, so it reaches the
/// same assignment — or runs out of resamples on the same formulas — whether or not the LLL condition holds.
#[test]
fn moser_tardos_follows_the_stated_algorithm_step_for_step() {
    let mut next = stream(0x307E);
    let (mut found, mut exhausted) = (0usize, 0usize);
    for round in 0..1500u64 {
        let lo = 1 + (next() % 5) as usize;
        let n = lo + (next() % 20) as usize;
        let m = (next() % 50) as usize;
        let mut clauses = random_formula(&mut next, n, m, lo, lo + 2);
        if next() % 40 == 0 {
            clauses.push(Vec::new());
        }
        let seed = next();
        let budget = (next() % 200) as usize;
        let got = moser_tardos_witness(n, &clauses, seed, budget);
        assert_eq!(got, moser_tardos_by_scanning(n, &clauses, seed, budget), "round {round}: n={n} m={} budget {budget}", clauses.len());
        if got.is_some() {
            found += 1;
        } else {
            exhausted += 1;
        }
    }
    assert!(found >= 200 && exhausted >= 100, "both outcomes are exercised: {found} found, {exhausted} exhausted");
}

/// A resample touches only the clauses sharing a variable with the one resampled, so the search costs the resamples'
/// neighbourhoods, not a scan of the formula per resample: 1,000,000 disjoint width-4 clauses (degree 0, certified)
/// start with ~62,500 violated, a ~3·10¹⁰-step search by scanning, and the witness must come within a minute even
/// unoptimized on a loaded box.
#[test]
fn moser_tardos_costs_the_resamples_neighbourhoods() {
    let clauses: Vec<Vec<Lit>> = (0..1_000_000u32).map(|i| (0..4).map(|k| Lit::new(4 * i + k, (i + k) % 3 == 0)).collect()).collect();
    assert!(lll_certifies_sat(&clauses).is_some());
    let (tx, rx) = std::sync::mpsc::channel();
    let formula = clauses.clone();
    std::thread::spawn(move || {
        let _ = tx.send(moser_tardos_witness(4_000_000, &formula, 0x5EED, 1000 + 64 * formula.len()));
    });
    let model = rx
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the witness search costs its resamples' neighbourhoods")
        .expect("under the LLL condition the search reaches a model");
    assert!(clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())));
}
