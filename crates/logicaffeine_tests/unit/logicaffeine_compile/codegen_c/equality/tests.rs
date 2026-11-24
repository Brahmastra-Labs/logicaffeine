use super::*;

/// A literal has nothing left to compute; a name is read and a call is made, as the VM reads and makes them.
#[test]
fn only_a_literal_is_inert() {
    for literal in ["0LL", "42LL", "1.5", "0x1p63", "true", "false", "\"\"", "\"abc\""] {
        assert!(inert(literal), "{literal} is a literal");
    }
    for computed in ["items", "a.b", "noisy()", "-1LL", "(x)", "\"a\" \"b\"", "\"a\\\"b\""] {
        assert!(!inert(computed), "{computed} is computed");
    }
}

/// `is not equal to` of a decided answer is the other answer; of anything else, its negation.
#[test]
fn unequal_decides_what_is_decided_and_negates_the_rest() {
    assert_eq!(unequal("true".to_string()), "false");
    assert_eq!(unequal("false".to_string()), "true");
    assert_eq!(unequal("str_equals(a, b)".to_string()), "(!str_equals(a, b))");
    assert_eq!(unequal("((x).tag == Option__Int_None)".to_string()), "(!((x).tag == Option__Int_None))");
}

/// The left operand is computed before a test that reads only the right, the right after a test that reads only the
/// left, and a literal on either side is left out.
#[test]
fn an_operand_the_test_does_not_read_is_still_computed_in_order() {
    assert_eq!(first_then("noisy()", "t".to_string()), "((void)(noisy()), t)");
    assert_eq!(first_then("0LL", "t".to_string()), "t");
    assert_eq!(first_then("", "t".to_string()), "t", "the nothing literal has no code");
    assert_eq!(
        then_computed("t".to_string(), "noisy()", 3),
        "({ bool __tested3 = t; (void)(noisy()); __tested3; })"
    );
    assert_eq!(then_computed("t".to_string(), "false", 3), "t");
    assert_eq!(then_computed("t".to_string(), "", 3), "t");
}
