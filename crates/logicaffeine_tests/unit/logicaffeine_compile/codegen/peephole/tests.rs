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

/// An end handed over as code is still a literal where the code is one: the count folds, and no untyped literal is
/// left to call a method on (`50.saturating_add(1)` has no integer type, E0689).
#[test]
fn a_literal_handed_over_as_code_folds_the_count() {
    assert_eq!(pass_count(LoopEnd::Literal(0), LoopEnd::Code("50"), true).as_usize(), "51");
    assert_eq!(pass_count(LoopEnd::Code("1"), LoopEnd::Code("10"), false).as_usize(), "9");
    assert_eq!(pass_count(LoopEnd::Code("(7)"), LoopEnd::Literal(3), true).as_usize(), "0", "a loop that never runs");
    assert_eq!(
        pass_count(LoopEnd::Literal(0), LoopEnd::Code("n"), true).as_usize(),
        "n.saturating_add(1).max(0) as usize",
        "a name stays the code it is"
    );
}

/// A literal capacity is scaled where it is written: no untyped literal is left to call a method on
/// (`1000.saturating_mul(5)`, E0689). Any other capacity keeps its saturating multiply.
#[test]
fn a_literal_capacity_is_scaled_where_it_is_written() {
    assert_eq!(scale_capacity("1000", 5), "5000");
    assert_eq!(scale_capacity("(1000)", 2), "2000");
    assert_eq!(scale_capacity("1000", 1), "1000");
    assert_eq!(scale_capacity(&usize::MAX.to_string(), 2), usize::MAX.to_string(), "it saturates");
    assert_eq!(scale_capacity("n.max(0) as usize", 3), "(n.max(0) as usize).saturating_mul(3)");
}
