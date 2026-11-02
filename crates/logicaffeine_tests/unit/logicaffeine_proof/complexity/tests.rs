use super::*;

#[test]
fn a_valid_descent_certifies_its_size() {
    // ranks 3,3,2,2,2,1 → 3 levels, max width 3 (the two-and-three runs), bound 9, actual 6.
    let ranks = vec![3, 3, 2, 2, 2, 1];
    let b = certify_size_bound(&ranks).expect("non-increasing ⇒ valid");
    assert_eq!(b.levels, 3);
    assert_eq!(b.max_width, 3);
    assert_eq!(b.bound, 9);
    assert_eq!(b.actual, 6);
    assert!(b.actual <= b.bound);
}

#[test]
fn an_increasing_measure_certifies_nothing() {
    // A measure that goes back up is not a termination measure — reject it.
    assert!(certify_size_bound(&[3, 2, 3]).is_none());
    assert!(certify_size_bound(&[1, 2]).is_none());
}

#[test]
fn empty_and_flat_measures() {
    assert_eq!(certify_size_bound(&[]).unwrap().bound, 0);
    let flat = certify_size_bound(&[5, 5, 5]).unwrap();
    assert_eq!(
        (flat.levels, flat.max_width, flat.bound, flat.actual),
        (1, 3, 3, 3)
    );
}
