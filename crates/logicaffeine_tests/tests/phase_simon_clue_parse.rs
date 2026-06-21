//! PARSE-LEVEL lock for the six Simon clues, read verbatim from the puzzle text.
//! Each clue must compile and retain EVERY entity/relation it mentions — no dropped
//! disjunct, no lost modifier. This pins the parse independently of the solve, so a
//! parser regression surfaces here even if the over-determined grid still solves.

use logicaffeine_language::compile;

fn ok(s: &str) -> String {
    compile(s).unwrap_or_else(|e| panic!("expected OK for {s:?}, got {e:?}"))
}

/// Clue 1 — of-pair XOR over an activity head and a fused year label.
#[test]
fn clue1_hunting_vacation_and_2004_holiday_xor() {
    let o = ok("Of the hunting vacation and the 2004 holiday, one was with Neal and the other was in Connecticut.");
    for needle in ["Hunt", "2004", "Neal", "Connecticut"] {
        assert!(o.contains(needle), "clue1 lost {needle}; got: {o}");
    }
    // Both assignments of the XOR (Neal↔one / the-other↔Connecticut) must appear.
    assert!(o.contains('∨'), "clue1 must be an exclusive disjunction; got: {o}");
    assert_eq!(o.matches("Neal").count(), 2, "clue1 XOR names Neal in both arms; got: {o}");
}

/// Clue 2 — definite "the Florida trip" identified with "the hunting trip".
#[test]
fn clue2_florida_trip_is_hunting_trip() {
    let o = ok("The Florida trip was the hunting trip.");
    for needle in ["Florida", "Hunt", "Trip"] {
        assert!(o.contains(needle), "clue2 lost {needle}; got: {o}");
    }
}

/// Clue 3 — "Neither A nor B is C" must deny BOTH subjects (two statements).
#[test]
fn clue3_neither_nor_keeps_both_subjects() {
    let o = ok("Neither the holiday with Bill nor the Florida vacation is the 2001 trip.");
    assert!(o.contains("Bill"), "clue3 lost Bill; got: {o}");
    assert!(o.contains("Florida"), "clue3 lost the Florida vacation; got: {o}");
    assert_eq!(o.matches("2001_trip").count(), 2, "clue3 denies BOTH ≠ 2001 trip; got: {o}");
}

/// Clue 4 — definite "the holiday with Yvonne" + contraction "wasn't" → negation.
#[test]
fn clue4_yvonne_holiday_not_in_kentucky() {
    let o = ok("The holiday with Yvonne wasn't in Kentucky.");
    assert!(o.contains("Yvonne"), "clue4 lost Yvonne; got: {o}");
    assert!(o.contains("Kentucky"), "clue4 lost Kentucky; got: {o}");
    assert!(o.contains('¬'), "clue4 'wasn't' must negate; got: {o}");
}

/// Clue 5 — of-pair XOR over a skydiving trip and a Maine holiday.
#[test]
fn clue5_skydiving_and_maine_xor() {
    let o = ok("Of the skydiving trip and the Maine holiday, one was in 2003 and the other was with Bill.");
    for needle in ["Skydive", "Maine", "2003", "Bill"] {
        assert!(o.contains(needle), "clue5 lost {needle}; got: {o}");
    }
    assert!(o.contains('∨'), "clue5 must be an exclusive disjunction; got: {o}");
}

/// Clue 6 — fused "the 2003 holiday" + contraction "wasn't" → negated cycling.
#[test]
fn clue6_2003_holiday_not_cycling() {
    let o = ok("The 2003 holiday wasn't the cycling trip.");
    assert!(o.contains("2003"), "clue6 lost 2003; got: {o}");
    assert!(o.contains("Cycle"), "clue6 lost cycling; got: {o}");
    assert!(o.contains('¬'), "clue6 'wasn't' must negate; got: {o}");
}
