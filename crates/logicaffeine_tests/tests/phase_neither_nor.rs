//! "Neither X nor Y is P" must negate the predicate of BOTH coordinated subjects:
//! ¬P(X) ∧ ¬P(Y). The parser was dropping the second disjunct (Y) entirely, so a
//! real clue like "Neither the holiday with Bill nor the Florida vacation is the
//! 2001 trip" lost the Florida-vacation constraint.

use logicaffeine_language::compile;

fn ok(s: &str) -> String {
    compile(s).unwrap_or_else(|e| panic!("expected OK for {s:?}, got {e:?}"))
}

#[test]
fn neither_nor_proper_names_negates_both() {
    let out = ok("Neither Bill nor Neal is happy.");
    assert!(out.contains("Bill"), "must keep first subject; got: {out}");
    assert!(out.contains("Neal"), "must keep SECOND subject; got: {out}");
    assert_eq!(out.matches("Happy").count(), 2, "predicate negated over both; got: {out}");
}

#[test]
fn neither_nor_definite_descriptions_negates_both() {
    let out = ok("Neither the dog nor the cat is hungry.");
    assert!(out.contains("Dog"), "must keep first subject; got: {out}");
    assert!(out.contains("Cat"), "must keep SECOND subject; got: {out}");
    assert_eq!(out.matches("Hungry").count(), 2, "predicate negated over both; got: {out}");
}

#[test]
fn neither_nor_simon_clue_keeps_florida_vacation() {
    let out = ok("Neither the holiday with Bill nor the Florida vacation is the 2001 trip.");
    assert!(out.contains("Bill"), "first subject (Bill) present; got: {out}");
    assert!(out.contains("Florida"), "SECOND subject (Florida vacation) must survive; got: {out}");
    assert_eq!(
        out.matches("2001_trip").count(),
        2,
        "both subjects must be denied identity with the 2001 trip; got: {out}"
    );
}
