use super::*;

fn big(x: i64) -> BigInt {
    BigInt::from_i64(x)
}

/// The value of `r` at `point`, exactly.
fn eval(r: &Relation, point: &[i64]) -> BigInt {
    r.terms.iter().fold(BigInt::zero(), |acc, (c, m)| {
        let mut term = c.clone();
        for (x, &e) in point.iter().zip(m) {
            for _ in 0..e {
                term = term.mul(&big(*x));
            }
        }
        acc.add(&term)
    })
}

/// The loop-head states of `total = 0; i = 1; while i ≤ n { total += f(i); i += 1 }` for every `n` in `ns`, as rows
/// `(i, total, n)`.
fn sum_rows(f: impl Fn(i64) -> i64, ns: impl IntoIterator<Item = i64>) -> Vec<Vec<BigInt>> {
    let mut rows = Vec::new();
    for n in ns {
        let (mut i, mut total) = (1, 0);
        loop {
            rows.push(vec![big(i), big(total), big(n)]);
            if i > n {
                break;
            }
            total += f(i);
            i += 1;
        }
    }
    rows
}

/// The Gauss sum's head states satisfy one relation, `2·total + i = i²`, found at degree two and not offered again
/// times a monomial at degrees three and four.
#[test]
fn the_sum_of_the_counter_is_one_quadratic_relation() {
    let rs = relations(&sum_rows(|i| i, 0..=12), 4, 70);
    assert_eq!(rs.len(), 1, "{rs:?}");
    let r = &rs[0];
    assert_eq!(r.degree(), 2);
    for i in -50..=50 {
        let total = i * (i - 1) / 2;
        assert!(eval(r, &[i, total, 999]).is_zero(), "it is the Gauss relation: {r:?}");
    }
    assert!(!eval(r, &[3, 4, 0]).is_zero(), "and not one every point satisfies");
}

/// Runs long enough to pin each relation down: a counter reaching 25 gives more points on the curve than there are
/// monomials of degree four in the counter and the total, so no polynomial but the sum's vanishes on all of them.
#[test]
fn the_sums_of_squares_and_cubes_are_one_relation_each_at_their_degree() {
    let squares = relations(&sum_rows(|i| i * i, 0..=24), 4, 70);
    assert_eq!(squares.len(), 1, "{squares:?}");
    assert_eq!(squares[0].degree(), 3);
    let cubes = relations(&sum_rows(|i| i * i * i, 0..=24), 4, 70);
    assert_eq!(cubes.len(), 1, "{cubes:?}");
    assert_eq!(cubes[0].degree(), 4);
    for i in 1..=40 {
        let t = (i - 1) * i / 2;
        assert!(eval(&cubes[0], &[i, t * t, 7]).is_zero(), "Σ k³ for k < i is (i(i−1)/2)²: {:?}", cubes[0]);
    }
}

/// A variable a linear relation determines is not searched again: `m` always equals `n`, so `m = n` is found, and the
/// quadratic relation is stated without `m`.
#[test]
fn a_variable_a_linear_relation_determines_is_dropped_from_the_higher_degrees() {
    let rows: Vec<Vec<BigInt>> =
        sum_rows(|i| i, 0..=10).into_iter().map(|r| vec![r[2].clone(), r[0].clone(), r[1].clone(), r[2].clone()]).collect();
    let rs = relations(&rows, 4, 70);
    assert_eq!(rs.len(), 2, "{rs:?}");
    assert_eq!(rs[0].degree(), 1);
    assert_eq!(rs[0].terms, vec![(big(1), vec![1, 0, 0, 0]), (big(-1), vec![0, 0, 0, 1])], "m − n = 0");
    assert!(rs[1].terms.iter().all(|(_, m)| m[0] == 0), "the quadratic relation does not mention m: {:?}", rs[1]);
}

/// Rows with no relation among them give none: the null space is found exactly, not by a tolerance.
#[test]
fn rows_with_no_relation_give_none() {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % 2001) as i64 - 1000
    };
    let rows: Vec<Vec<BigInt>> = (0..200).map(|_| vec![big(next()), big(next()), big(next())]).collect();
    assert_eq!(relations(&rows, 4, 70), Vec::<Relation>::new());
}

/// A relation's coefficients are coprime integers, the first positive, however the elimination scaled them.
#[test]
fn coefficients_are_coprime_integers_the_first_positive() {
    let rows: Vec<Vec<BigInt>> = (0..30).map(|t| vec![big(6 * t + 4), big(-9 * t - 6)]).collect();
    let rs = relations(&rows, 2, 70);
    assert_eq!(rs, vec![Relation { terms: vec![(big(3), vec![1, 0]), (big(2), vec![0, 1])] }], "3x + 2y = 0");
}

/// A run that holds a constant gives `x − c = 0`; a search past `max_monomials` stops at the last degree that fits.
#[test]
fn a_constant_is_a_linear_relation_and_the_monomial_budget_bounds_the_degree() {
    let rows: Vec<Vec<BigInt>> = (0..20).map(|t| vec![big(t), big(5)]).collect();
    assert_eq!(relations(&rows, 4, 70), vec![Relation { terms: vec![(big(1), vec![0, 1]), (big(-5), vec![0, 0])] }]);
    let gauss = sum_rows(|i| i, 0..=12);
    assert!(relations(&gauss, 4, 9).is_empty(), "three variables have ten monomials of degree two");
    assert_eq!(relations(&gauss, 4, 10).len(), 1);
}

/// Too short a run leaves room for a polynomial that vanishes on its points by accident: thirteen points on the cube
/// sum's curve, against fifteen monomials of degree four in the counter and the total. The search reports what the
/// rows satisfy; it is the proof that keeps only the inductive relations.
#[test]
fn too_few_points_on_a_curve_admit_an_accidental_relation() {
    let short = relations(&sum_rows(|i| i * i * i, 0..=12), 4, 70);
    assert!(short.len() > 1, "{short:?}");
    let rows = sum_rows(|i| i * i * i, 0..=12);
    for r in &short {
        assert!(rows.iter().all(|row| {
            let point: Vec<i64> = row.iter().map(|x| x.to_i64().unwrap()).collect();
            eval(r, &point).is_zero()
        }));
    }
}
