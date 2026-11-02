use super::*;

fn number(n: i64) -> Expr<'static> {
    Expr::Literal(Literal::Number(n))
}

#[test]
fn a_copy_that_runs_over_proven_reads_is_a_plain_slice() {
    assert!(
        copy_is_a_plain_slice(&number(2), &number(4), false, true),
        "items 2 through 4"
    );
    assert!(
        copy_is_a_plain_slice(&number(3), &number(3), false, true),
        "item 3 alone"
    );
    assert!(
        copy_is_a_plain_slice(&number(2), &number(4), true, true),
        "items 2 and 3"
    );
}

#[test]
fn a_copy_that_never_runs_is_no_slice_though_its_reads_are_proven() {
    assert!(
        !copy_is_a_plain_slice(&number(4), &number(2), false, true),
        "4 through 2 reads nothing"
    );
    assert!(
        !copy_is_a_plain_slice(&number(3), &number(2), false, true),
        "3 through 2 reads nothing"
    );
    assert!(
        !copy_is_a_plain_slice(&number(3), &number(3), true, true),
        "3 up to 3 reads nothing"
    );
    assert!(
        !copy_is_a_plain_slice(&number(5), &number(3), true, true),
        "5 up to 3 reads nothing"
    );
}

#[test]
fn a_copy_with_an_unproven_read_or_a_computed_bound_is_no_slice() {
    assert!(!copy_is_a_plain_slice(&number(2), &number(4), false, false));
    let mut interner = Interner::new();
    let n = Expr::Identifier(interner.intern("n"));
    assert!(!copy_is_a_plain_slice(&number(1), &n, false, true));
    assert!(!copy_is_a_plain_slice(&n, &number(4), false, true));
}
