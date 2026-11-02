use super::*;

#[test]
fn test_sm2_first_correct() {
    let mut srs = SrsData::default();
    sm2_update(&mut srs, ResponseQuality::Perfect);

    assert_eq!(srs.repetitions, 1);
    assert_eq!(srs.interval, 1);
    assert!(srs.ease_factor > 2.5);
}

#[test]
fn test_sm2_second_correct() {
    let mut srs = SrsData::default();
    sm2_update(&mut srs, ResponseQuality::Perfect);
    sm2_update(&mut srs, ResponseQuality::Perfect);

    assert_eq!(srs.repetitions, 2);
    assert_eq!(srs.interval, 6);
}

#[test]
fn test_sm2_incorrect_resets() {
    let mut srs = SrsData::default();
    sm2_update(&mut srs, ResponseQuality::Perfect);
    sm2_update(&mut srs, ResponseQuality::Perfect);
    sm2_update(&mut srs, ResponseQuality::Incorrect);

    assert_eq!(srs.repetitions, 0);
    assert_eq!(srs.interval, 1);
}

#[test]
fn test_sm2_ease_factor_minimum() {
    let mut srs = SrsData::default();
    srs.ease_factor = 1.3;
    sm2_update(&mut srs, ResponseQuality::CorrectDifficult);

    assert!(srs.ease_factor >= 1.3);
}

#[test]
fn test_is_due_none() {
    assert!(is_due(None, "2025-01-01"));
}

#[test]
fn test_is_due_past() {
    assert!(is_due(Some("2025-01-01"), "2025-01-02"));
}

#[test]
fn test_is_due_future() {
    assert!(!is_due(Some("2025-01-05"), "2025-01-02"));
}

#[test]
fn test_is_due_today() {
    assert!(is_due(Some("2025-01-01"), "2025-01-01"));
}

#[test]
fn test_calculate_next_review() {
    let next = calculate_next_review("2025-01-15", 6);
    assert_eq!(next, "2025-01-21");
}

#[test]
fn test_calculate_next_review_month_overflow() {
    let next = calculate_next_review("2025-01-28", 6);
    assert_eq!(next, "2025-02-03");
}

#[test]
fn test_response_quality_is_correct() {
    assert!(!ResponseQuality::Blackout.is_correct());
    assert!(!ResponseQuality::Incorrect.is_correct());
    assert!(!ResponseQuality::IncorrectEasy.is_correct());
    assert!(ResponseQuality::CorrectDifficult.is_correct());
    assert!(ResponseQuality::CorrectHesitation.is_correct());
    assert!(ResponseQuality::Perfect.is_correct());
}
