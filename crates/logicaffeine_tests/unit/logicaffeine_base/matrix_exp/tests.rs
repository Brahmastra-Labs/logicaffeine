use super::*;

fn r(n: i64) -> Rational {
    Rational::from_i64(n)
}

fn m(rows: &[&[i64]]) -> Vec<Vec<Rational>> {
    rows.iter().map(|row| row.iter().map(|v| r(*v)).collect()).collect()
}

fn shown(e: &[Vec<ExactReal>]) -> Vec<Vec<String>> {
    e.iter().map(|row| row.iter().map(|x| x.to_string()).collect()).collect()
}

#[test]
fn the_characteristic_polynomial_of_small_matrices() {
    assert_eq!(characteristic_polynomial(&m(&[&[0, 1], &[-1, 0]])), Some(Poly::from_i64s(&[1, 0, 1])));
    assert_eq!(characteristic_polynomial(&m(&[&[2, 1], &[0, 3]])), Some(Poly::from_i64s(&[6, -5, 1])));
    assert_eq!(characteristic_polynomial(&m(&[&[1, 2], &[3]])), None);
    assert_eq!(characteristic_polynomial(&[]), None);
}

#[test]
fn a_scalar_exponential_is_its_exponential() {
    assert_eq!(shown(&exp_matrix(&m(&[&[1]]), &r(1)).unwrap()), [["e"]]);
    assert_eq!(shown(&exp_matrix(&m(&[&[-1]]), &r(1)).unwrap()), [["e^(-1)"]]);
    assert_eq!(shown(&exp_matrix(&m(&[&[0]]), &r(1)).unwrap()), [["1"]]);
}

#[test]
fn a_rotation_generator_exponentiates_to_cosines_and_sines() {
    let e = exp_matrix(&m(&[&[0, 1], &[-1, 0]]), &r(1)).unwrap();
    assert_eq!(shown(&e), [["cos(1)", "sin(1)"], ["-sin(1)", "cos(1)"]]);
    let two = exp_matrix(&m(&[&[0, 1], &[-1, 0]]), &r(2)).unwrap();
    assert_eq!(two[0][0].to_string(), "cos(2)");
}

#[test]
fn a_jordan_block_brings_a_polynomial_factor() {
    let e = exp_matrix(&m(&[&[1, 1], &[0, 1]]), &r(1)).unwrap();
    assert_eq!(shown(&e), [["e", "e"], ["0", "e"]], "the exponential of a unit Jordan block is e times it");
    let nil = exp_matrix(&m(&[&[0, 1], &[0, 0]]), &r(3)).unwrap();
    assert_eq!(shown(&nil), [["1", "3"], ["0", "1"]]);
}

#[test]
fn a_damped_oscillator_carries_its_decay_into_both_terms() {
    let e = exp_matrix(&m(&[&[-1, 1], &[-1, -1]]), &r(1)).unwrap();
    for (got, want) in [
        (&e[0][0], 0.36787944117144233 * 1f64.cos()),
        (&e[0][1], 0.36787944117144233 * 1f64.sin()),
        (&e[1][0], -0.36787944117144233 * 1f64.sin()),
    ] {
        let d: f64 = got.to_decimal(15).parse().unwrap();
        assert!((d - want).abs() < 1e-12, "{got} ≈ {want}");
    }
}

#[test]
fn every_exponential_satisfies_the_semigroup_law_numerically() {
    for rows in [
        m(&[&[0, 1], &[-1, 0]]),
        m(&[&[1, 2], &[3, 4]]),
        m(&[&[0, 1, 0], &[0, 0, 1], &[1, 0, 0]]),
        m(&[&[2, 0, 0], &[1, 2, 0], &[0, 1, 2]]),
    ] {
        let half = Rational::from_ratio_i64(1, 2).unwrap();
        let whole = exp_matrix(&rows, &r(1)).unwrap();
        let part = exp_matrix(&rows, &half).unwrap();
        let n = rows.len();
        for i in 0..n {
            for j in 0..n {
                let product = (0..n).fold(ExactReal::from_i64(0), |acc, k| acc.add(&part[i][k].mul(&part[k][j])));
                let d1: f64 = whole[i][j].to_decimal(20).parse().unwrap();
                let d2: f64 = product.to_decimal(20).parse().unwrap();
                assert!((d1 - d2).abs() <= 1e-9 * (1.0 + d1.abs()), "e^M = (e^{{M/2}})² at [{i}][{j}] for {rows:?}");
            }
        }
    }
}
