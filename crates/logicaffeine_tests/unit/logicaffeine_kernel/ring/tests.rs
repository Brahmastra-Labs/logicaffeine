use super::*;

#[test]
fn test_polynomial_constant() {
    let p = Polynomial::constant(42);
    assert_eq!(p, Polynomial::constant(42));
}

#[test]
fn test_polynomial_add() {
    let x = Polynomial::var(0);
    let y = Polynomial::var(1);
    let sum1 = x.add(&y);
    let sum2 = y.add(&x);
    assert!(sum1.canonical_eq(&sum2), "x+y should equal y+x");
}

#[test]
fn test_polynomial_mul() {
    let x = Polynomial::var(0);
    let y = Polynomial::var(1);
    let prod1 = x.mul(&y);
    let prod2 = y.mul(&x);
    assert!(prod1.canonical_eq(&prod2), "x*y should equal y*x");
}

#[test]
fn test_polynomial_distributivity() {
    let x = Polynomial::var(0);
    let y = Polynomial::var(1);
    let z = Polynomial::var(2);

    // x*(y+z) should equal x*y + x*z
    let lhs = x.mul(&y.add(&z));
    let rhs = x.mul(&y).add(&x.mul(&z));
    assert!(lhs.canonical_eq(&rhs));
}

#[test]
fn test_polynomial_subtraction() {
    let x = Polynomial::var(0);
    let result = x.sub(&x);
    assert!(result.canonical_eq(&Polynomial::zero()));
}

#[test]
fn test_collatz_algebra() {
    // 3(2k+1) + 1 = 6k + 4
    let k = Polynomial::var(0);
    let two = Polynomial::constant(2);
    let three = Polynomial::constant(3);
    let one = Polynomial::constant(1);
    let four = Polynomial::constant(4);
    let six = Polynomial::constant(6);

    // LHS: 3*(2*k + 1) + 1
    let two_k = two.mul(&k);
    let two_k_plus_1 = two_k.add(&one);
    let three_times = three.mul(&two_k_plus_1);
    let lhs = three_times.add(&one);

    // RHS: 6*k + 4
    let six_k = six.mul(&k);
    let rhs = six_k.add(&four);

    assert!(lhs.canonical_eq(&rhs), "3(2k+1)+1 should equal 6k+4");
}
