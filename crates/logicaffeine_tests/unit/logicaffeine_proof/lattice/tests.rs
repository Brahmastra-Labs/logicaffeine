use super::*;

fn as_i64(v: &[Vec<BigInt>]) -> Vec<Vec<i64>> {
    v.iter()
        .map(|row| {
            row.iter()
                .map(|x| x.to_i64().expect("small entry"))
                .collect()
        })
        .collect()
}

fn norm_sq(row: &[i64]) -> i64 {
    row.iter().map(|&x| x * x).sum()
}

#[test]
fn lll_reduces_a_skewed_basis_to_the_standard_one() {
    // The lattice ℤ² given by a badly skewed basis: [10,1] is long, but 10·[1,0] shears it to [0,1].
    let reduced = as_i64(&lll_reduce(&[vec![1, 0], vec![10, 1]]));
    assert_eq!(
        reduced,
        vec![vec![1, 0], vec![0, 1]],
        "LLL recovers the standard basis"
    );
}

#[test]
fn lll_finds_the_shortest_vector_of_a_flat_lattice() {
    // det = 15·17 − 23·11 = 2, so this lattice is very flat and its shortest vector is tiny ([1,1],
    // norm² = 2); both input rows have norm² in the hundreds. LLL surfaces it.
    let reduced = as_i64(&lll_reduce(&[vec![15, 23], vec![11, 17]]));
    let shortest = reduced.iter().map(|r| norm_sq(r)).min().unwrap();
    assert_eq!(shortest, 2, "LLL finds the shortest vector, norm² = 2");
    assert!(
        reduced.iter().any(|r| r == &[1, 1] || r == &[-1, -1]),
        "and it is ±[1,1]"
    );
}

#[test]
fn exact_integer_lll_matches_the_known_reductions() {
    let big = |rows: &[&[i64]]| -> Vec<Vec<BigInt>> {
        rows.iter()
            .map(|r| r.iter().map(|&x| BigInt::from_i64(x)).collect())
            .collect()
    };
    let out = as_i64(&lll_reduce_bigint_exact(&big(&[&[1, 0], &[10, 1]])));
    assert_eq!(
        out,
        vec![vec![1, 0], vec![0, 1]],
        "exact LLL recovers the standard basis"
    );

    let out = as_i64(&lll_reduce_bigint_exact(&big(&[&[15, 23], &[11, 17]])));
    let shortest = out.iter().map(|r| norm_sq(r)).min().unwrap();
    assert_eq!(
        shortest, 2,
        "exact LLL finds the shortest vector, norm² = 2"
    );
}

#[test]
fn l2_fplll_matches_the_known_reductions() {
    let big = |rows: &[&[i64]]| -> Vec<Vec<BigInt>> {
        rows.iter()
            .map(|r| r.iter().map(|&x| BigInt::from_i64(x)).collect())
            .collect()
    };
    let out = as_i64(&lll_reduce_bigint_fp(&big(&[&[1, 0], &[10, 1]])));
    assert_eq!(
        out,
        vec![vec![1, 0], vec![0, 1]],
        "L² recovers the standard basis"
    );

    let out = as_i64(&lll_reduce_bigint_fp(&big(&[&[15, 23], &[11, 17]])));
    let shortest = out.iter().map(|r| norm_sq(r)).min().unwrap();
    assert_eq!(shortest, 2, "L² finds the shortest vector, norm² = 2");
}

#[test]
fn lll_output_is_size_reduced() {
    // A 3-D lattice: after reduction every off-diagonal Gram–Schmidt coefficient satisfies |μ| ≤ ½.
    let reduced = lll_reduce(&[vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 10]]);
    let b: Vec<Vec<Rational>> = reduced
        .iter()
        .map(|row| {
            row.iter()
                .map(|x| Rational::from_bigint(x.clone()))
                .collect()
        })
        .collect();
    let (_, mu) = gram_schmidt(&b);
    for i in 0..b.len() {
        for j in 0..i {
            assert!(
                mu[i][j].abs() <= rat(1, 2),
                "size-reduced: |μ[{i}][{j}]| ≤ ½"
            );
        }
    }
}
