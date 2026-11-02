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
