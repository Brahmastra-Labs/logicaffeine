use super::*;

fn binom(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut num = 1u64;
    let mut den = 1u64;
    for i in 0..k {
        num *= n - i;
        den *= i + 1;
    }
    num / den
}

#[test]
fn the_constant_monomial_is_degree_zero_and_first() {
    for &ml in &[true, false] {
        let ms = monomials_up_to_degree(5, 3, ml);
        assert_eq!(ms[0], one(), "the constant 1 must be listed first");
        assert_eq!(degree(&ms[0]), 0);
    }
}

#[test]
fn multilinear_counts_are_binomial_and_squarefree() {
    // Squarefree monomials of degree d in n vars: C(n, d). Up to degree D: Σ C(n,d).
    for n in 1u32..=7 {
        for d in 0..=n as usize {
            let got = monomials_of_degree(n, d, true);
            assert_eq!(got.len() as u64, binom(n as u64, d as u64), "C({n},{d})");
            for m in &got {
                let mut sorted = m.clone();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(
                    &sorted, m,
                    "multilinear monomial must be strictly increasing"
                );
                assert!(m.iter().all(|&v| v < n), "variable index in range");
            }
        }
    }
}

#[test]
fn non_multilinear_counts_are_multiset_coefficients() {
    // Monomials with repetition of degree d in n vars: C(n + d − 1, d).
    for n in 1u32..=6 {
        for d in 0..=5usize {
            let got = monomials_of_degree(n, d, false);
            let want = binom(n as u64 + d as u64 - if d == 0 { 0 } else { 1 }, d as u64);
            let want = if d == 0 { 1 } else { want };
            assert_eq!(got.len() as u64, want, "multiset C({}+{d}-1,{d})", n);
            for m in &got {
                let mut s = m.clone();
                s.sort_unstable();
                assert_eq!(&s, m, "must be nondecreasing");
            }
        }
    }
}

#[test]
fn all_generated_monomials_are_distinct() {
    for &ml in &[true, false] {
        let ms = monomials_up_to_degree(6, 4, ml);
        let mut sorted = ms.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ms.len(),
            "no duplicate monomials (multilinear={ml})"
        );
    }
}

#[test]
fn multiplication_respects_degree_and_idempotence() {
    // Non-multilinear: x_i · x_i = x_i² (degree grows).
    assert_eq!(mul(&vec![1], &vec![1], false), vec![1, 1]);
    // Multilinear: x_i · x_i = x_i (degree stays).
    assert_eq!(mul(&vec![1], &vec![1], true), vec![1]);
    // Product is sorted and has additive degree (non-multilinear).
    let p = mul(&vec![2, 0], &vec![1, 0], false);
    assert_eq!(p, vec![0, 0, 1, 2]);
    assert_eq!(degree(&p), 4);
    // 1 · m = m.
    assert_eq!(mul(&one(), &vec![3, 5], false), vec![3, 5]);
}
