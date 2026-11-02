use super::*;

fn ri(x: i64) -> Rational {
    Rational::from_i64(x)
}
fn rr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}
fn mat(rows: &[&[i64]]) -> Vec<Vec<Rational>> {
    rows.iter()
        .map(|r| r.iter().map(|&x| ri(x)).collect())
        .collect()
}
fn vecr(xs: &[i64]) -> Vec<Rational> {
    xs.iter().map(|&x| ri(x)).collect()
}

/// Every returned proof must type-check in the kernel with type headed by `Eq`.
fn all_kernel_checked(ctx: &Context, proofs: &[Term]) -> bool {
    proofs.iter().all(|p| {
        infer_type(ctx, p)
            .ok()
            .map(|ty| matches!(normalize(ctx, &ty), Term::App(..)))
            .unwrap_or(false)
    })
}

#[test]
fn integer_system_solve_is_kernel_certified() {
    // [[2,1],[1,3]] x = [3,4] → x = [1,1]; each row identity is a real kernel proof.
    let ctx = ring_context();
    let a = mat(&[&[2, 1], &[1, 3]]);
    let b = vecr(&[3, 4]);
    let (x, proofs) = prove_solve(&ctx, &a, &b).expect("kernel-certified solve");
    assert_eq!(x, vecr(&[1, 1]));
    assert_eq!(proofs.len(), 2);
    assert!(
        all_kernel_checked(&ctx, &proofs),
        "every row proof must type-check"
    );
}

#[test]
fn rational_solution_is_kernel_certified() {
    // x = [4/5, 7/5] — denominators cleared, still a kernel proof.
    let ctx = ring_context();
    let a = mat(&[&[2, 1], &[1, 3]]);
    let b = vecr(&[3, 5]);
    let (x, proofs) = prove_solve(&ctx, &a, &b).expect("kernel-certified rational solve");
    assert_eq!(x, vec![rr(4, 5), rr(7, 5)]);
    assert!(all_kernel_checked(&ctx, &proofs));
}

#[test]
fn hilbert_3x3_is_kernel_certified() {
    // The ill-conditioned Hilbert system floating point can't nail — PROVEN in the kernel.
    let ctx = ring_context();
    let n = 3;
    let h: Vec<Vec<Rational>> = (0..n)
        .map(|i| (0..n).map(|j| rr(1, (i + j + 1) as i64)).collect())
        .collect();
    let b = linalg::matvec(&h, &vecr(&[1, 1, 1]));
    let (x, proofs) = prove_solve(&ctx, &h, &b).expect("kernel-certified Hilbert solve");
    assert_eq!(x, vecr(&[1, 1, 1]));
    assert_eq!(proofs.len(), 3);
    assert!(all_kernel_checked(&ctx, &proofs));
}

#[test]
fn wrong_x_is_unprovable_fail_closed() {
    // The oracle must NOT fabricate a proof for a candidate that doesn't solve the system.
    let ctx = ring_context();
    let a = mat(&[&[2, 1], &[1, 3]]);
    let b = vecr(&[3, 4]);
    let true_x = vecr(&[1, 1]);
    assert!(
        prove_solution(&ctx, &a, &true_x, &b).is_some(),
        "true x is provable"
    );
    let wrong_x = vecr(&[2, 0]); // A·[2,0] = [4,2] ≠ [3,4]
    assert!(
        prove_solution(&ctx, &a, &wrong_x, &b).is_none(),
        "a wrong x must be UNPROVABLE (fail-closed)"
    );
}

#[test]
fn lu_factorization_is_kernel_certified() {
    // Every entry of P·A = L·U is proven in the kernel — even with a pivot row-swap.
    let ctx = ring_context();
    let a = mat(&[&[0, 1, 2], &[1, 2, 3], &[2, 1, 1]]);
    let (_f, grid) = prove_lu(&ctx, &a).expect("kernel-certified LU");
    let flat: Vec<Term> = grid.into_iter().flatten().collect();
    assert_eq!(flat.len(), 9);
    assert!(
        all_kernel_checked(&ctx, &flat),
        "every P·A=L·U entry proof must type-check"
    );
}

#[test]
fn inverse_is_kernel_certified_including_off_diagonal_zeros() {
    // Every entry of A·A⁻¹ = I is proven — the off-diagonal `= 0` identities included.
    let ctx = ring_context();
    let a = mat(&[&[4, 3], &[6, 3]]);
    let (ainv, grid) = prove_inverse(&ctx, &a).expect("kernel-certified inverse");
    assert!(linalg::is_inverse(&a, &ainv));
    let flat: Vec<Term> = grid.into_iter().flatten().collect();
    assert_eq!(flat.len(), 4);
    assert!(
        all_kernel_checked(&ctx, &flat),
        "every A·A⁻¹=I entry proof must type-check"
    );
}

#[test]
fn hilbert_lu_and_inverse_are_kernel_certified() {
    let ctx = ring_context();
    let n = 3;
    let h: Vec<Vec<Rational>> = (0..n)
        .map(|i| (0..n).map(|j| rr(1, (i + j + 1) as i64)).collect())
        .collect();
    let (_f, lu_grid) = prove_lu(&ctx, &h).expect("Hilbert LU certified");
    let (_hinv, inv_grid) = prove_inverse(&ctx, &h).expect("Hilbert inverse certified");
    assert!(all_kernel_checked(
        &ctx,
        &lu_grid.into_iter().flatten().collect::<Vec<_>>()
    ));
    assert!(all_kernel_checked(
        &ctx,
        &inv_grid.into_iter().flatten().collect::<Vec<_>>()
    ));
}

#[test]
fn nonsingularity_is_kernel_certified_and_zero_is_not() {
    let ctx = ring_context();
    // det([[2,1],[1,3]]) = 5 ≠ 0 — provable via 5·(1/5) = 1.
    let d = linalg::det(&mat(&[&[2, 1], &[1, 3]]));
    let p = prove_nonzero(&ctx, &d).expect("nonzero det is certifiable");
    assert!(all_kernel_checked(&ctx, std::slice::from_ref(&p)));
    // zero has no inverse — the claim 0 ≠ 0 is correctly unprovable.
    assert!(
        prove_nonzero(&ctx, &ri(0)).is_none(),
        "0 ≠ 0 must be unprovable"
    );
}

#[test]
fn full_unique_solution_certificate() {
    // The complete object: exact x + per-row A·x=b proofs + det≠0 proof, all kernel-checked.
    let ctx = ring_context();
    let a = mat(&[&[2, 1, 0], &[1, 3, 1], &[0, 1, 2]]);
    let b = vecr(&[3, 6, 5]);
    let cert = certify_system(&ctx, &a, &b).expect("unique-solution certificate");
    assert!(
        linalg::residual_is_zero(&a, &cert.x, &b),
        "x really solves it"
    );
    assert_eq!(cert.solve_proofs.len(), 3);
    assert!(
        all_kernel_checked(&ctx, &cert.solve_proofs),
        "row solve proofs check"
    );
    assert!(
        all_kernel_checked(&ctx, std::slice::from_ref(&cert.nonsingular_proof)),
        "det≠0 checks"
    );
    assert!(!cert.det.is_zero());
}

#[test]
fn singular_system_has_no_certificate() {
    let ctx = ring_context();
    let a = mat(&[&[1, 2], &[2, 4]]); // singular
    assert!(certify_system(&ctx, &a, &vecr(&[1, 1])).is_none());
}

#[test]
fn proof_type_is_exactly_the_row_identity() {
    // The proof term's type IS the (cleared) row proposition — not some other Eq.
    let ctx = ring_context();
    let a = mat(&[&[2, 1], &[1, 3]]);
    let b = vecr(&[3, 4]);
    let proofs = prove_solution(&ctx, &a, &vecr(&[1, 1]), &b).unwrap();
    // Row 0: 1·(2·1 + 1·1) = 1·1·3  →  Eq Int (mul 1 (add (mul 2 1) (mul 1 1))) 3
    let lhs = mul(lit(1), add(mul(lit(2), lit(1)), mul(lit(1), lit(1))));
    let expected = eq_int(lhs, lit(3));
    let ty = infer_type(&ctx, &proofs[0]).unwrap();
    assert_eq!(
        normalize(&ctx, &ty),
        normalize(&ctx, &expected),
        "row-0 proof type mismatch"
    );
}
