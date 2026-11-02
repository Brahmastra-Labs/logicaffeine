use super::*;

fn ri(x: i64) -> Rational {
    Rational::from_i64(x)
}
fn rr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).expect("nonzero denominator")
}
fn mat(rows: &[&[i64]]) -> Vec<Vec<Rational>> {
    rows.iter()
        .map(|r| r.iter().map(|&x| ri(x)).collect())
        .collect()
}
fn vecr(xs: &[i64]) -> Vec<Rational> {
    xs.iter().map(|&x| ri(x)).collect()
}

#[test]
fn solves_2x2_integer_system() {
    // [[2,1],[1,3]] x = [3,5] → x = [4/5, 7/5]
    let a = mat(&[&[2, 1], &[1, 3]]);
    let b = vecr(&[3, 5]);
    let x = solve(&a, &b).unwrap();
    assert_eq!(x, vec![rr(4, 5), rr(7, 5)]);
    assert!(residual_is_zero(&a, &x, &b));
}

#[test]
fn solution_is_exact_fraction_not_rounded() {
    // 3·x = 1 → x = 1/3 EXACTLY (a float would give 0.333…).
    let a = mat(&[&[3]]);
    let b = vecr(&[1]);
    let x = solve(&a, &b).unwrap();
    assert_eq!(x, vec![rr(1, 3)]);
    assert!(residual_is_zero(&a, &x, &b));
}

#[test]
fn hilbert_3x3_solved_exactly_zero_residual() {
    // H[i][j] = 1/(i+j+1), x = [1,1,1], b = H·x. Hilbert is famously ill-conditioned:
    // floating-point LAPACK returns x with a nonzero residual. Exact rationals nail it.
    let n = 3;
    let h: Vec<Vec<Rational>> = (0..n)
        .map(|i| (0..n).map(|j| rr(1, (i + j + 1) as i64)).collect())
        .collect();
    let x_true = vecr(&[1, 1, 1]);
    let b = matvec(&h, &x_true);
    let cert = certify_solve(&h, &b).unwrap();
    assert_eq!(cert.x, x_true, "exact Hilbert solve");
    assert!(cert.residual_zero, "residual must be EXACTLY zero");
    assert!(cert.nonsingular, "Hilbert is nonsingular");
}

#[test]
fn singular_matrix_has_no_solve() {
    // rows are linearly dependent → singular.
    let a = mat(&[&[1, 2], &[2, 4]]);
    let b = vecr(&[1, 1]);
    assert!(solve(&a, &b).is_none());
    assert!(det(&a).is_zero());
}

#[test]
fn determinant_is_exact() {
    // det([[1,2],[3,4]]) = -2
    assert_eq!(det(&mat(&[&[1, 2], &[3, 4]])), ri(-2));
    // det of a rational matrix
    let a = vec![vec![rr(1, 2), ri(0)], vec![ri(0), rr(2, 3)]];
    assert_eq!(det(&a), rr(1, 3)); // (1/2)(2/3)
}

#[test]
fn lu_factorization_is_exact_with_pivoting() {
    // A needs a row swap (zero leading pivot) — the certificate P·A = L·U must still hold exactly.
    let a = mat(&[&[0, 1, 2], &[1, 2, 3], &[2, 1, 1]]);
    let f = lu(&a).unwrap();
    assert!(lu_is_exact(&a, &f), "P·A must equal L·U exactly");
    // L is unit lower-triangular, U is upper-triangular — structurally.
    let n = a.len();
    for i in 0..n {
        assert_eq!(f.l[i][i], ri(1));
        for j in (i + 1)..n {
            assert!(f.l[i][j].is_zero(), "L strictly upper must be 0");
            assert!(f.u[j][i].is_zero(), "U strictly lower must be 0");
        }
    }
}

#[test]
fn inverse_times_a_is_identity_exactly() {
    let a = mat(&[&[4, 3], &[6, 3]]);
    let ainv = inverse(&a).unwrap();
    assert!(is_inverse(&a, &ainv), "A·A⁻¹ = I exactly");
    assert!(is_inverse(&ainv, &a), "A⁻¹·A = I exactly");
    // exact entries: inv([[4,3],[6,3]]) = [[-1/2, 1/2],[1, -2/3]]
    assert_eq!(
        ainv,
        vec![vec![rr(-1, 2), rr(1, 2)], vec![ri(1), rr(-2, 3)]]
    );
}

#[test]
fn inverse_of_hilbert_is_exact() {
    let n = 3;
    let h: Vec<Vec<Rational>> = (0..n)
        .map(|i| (0..n).map(|j| rr(1, (i + j + 1) as i64)).collect())
        .collect();
    let hinv = inverse(&h).unwrap();
    assert!(
        is_inverse(&h, &hinv),
        "exact inverse of the ill-conditioned Hilbert matrix"
    );
}

#[test]
fn rank_is_exact() {
    assert_eq!(rank(&mat(&[&[1, 2], &[2, 4]])), 1, "rank-deficient");
    assert_eq!(rank(&mat(&[&[1, 0], &[0, 1]])), 2, "full rank");
    assert_eq!(
        rank(&mat(&[&[1, 2, 3], &[2, 4, 6], &[1, 1, 1]])),
        2,
        "rectangular-ish"
    );
    assert_eq!(rank(&mat(&[&[0, 0], &[0, 0]])), 0, "zero matrix");
}

#[test]
fn lu_is_exact_on_hilbert() {
    let n = 4;
    let h: Vec<Vec<Rational>> = (0..n)
        .map(|i| (0..n).map(|j| rr(1, (i + j + 1) as i64)).collect())
        .collect();
    let f = lu(&h).unwrap();
    assert!(
        lu_is_exact(&h, &f),
        "exact LU of an ill-conditioned Hilbert matrix"
    );
}

#[test]
fn a_wrong_x_is_not_certified() {
    // Certificate is a real re-check: perturbing the true x by anything makes residual_is_zero false.
    let a = mat(&[&[2, 1], &[1, 3]]);
    let b = vecr(&[3, 5]);
    let x = solve(&a, &b).unwrap();
    let mut bad = x.clone();
    bad[0] = bad[0].add(&rr(1, 1000000)); // tiny perturbation
    assert!(residual_is_zero(&a, &x, &b));
    assert!(
        !residual_is_zero(&a, &bad, &b),
        "perturbed x must fail the exact residual check"
    );
}
