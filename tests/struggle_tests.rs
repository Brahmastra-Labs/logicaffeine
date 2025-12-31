/// Tests for Struggle Detection Logic
///
/// Struggle detection triggers Socratic hints when:
/// 1. User is inactive for 5+ seconds
/// 2. User submits a wrong answer

use logos::struggle::{StruggleDetector, StruggleReason};
use std::time::Duration;

// ═══════════════════════════════════════════════════════════════════
// Basic State Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_no_struggle_initially() {
    let detector = StruggleDetector::new();
    assert!(!detector.is_struggling());
    assert!(detector.reason().is_none());
}

#[test]
fn test_default_threshold_is_5_seconds() {
    let detector = StruggleDetector::new();
    assert_eq!(detector.threshold(), Duration::from_secs(5));
}

// ═══════════════════════════════════════════════════════════════════
// Inactivity Detection Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_struggle_after_inactivity_threshold() {
    let mut detector = StruggleDetector::new();
    detector.record_inactivity(Duration::from_secs(6));
    assert!(detector.is_struggling());
    assert_eq!(detector.reason(), Some(StruggleReason::Inactivity));
}

#[test]
fn test_no_struggle_under_threshold() {
    let mut detector = StruggleDetector::new();
    detector.record_inactivity(Duration::from_secs(4));
    assert!(!detector.is_struggling());
}

#[test]
fn test_struggle_at_exact_threshold() {
    let mut detector = StruggleDetector::new();
    detector.record_inactivity(Duration::from_secs(5));
    assert!(detector.is_struggling());
}

#[test]
fn test_inactivity_threshold_configurable() {
    let mut detector = StruggleDetector::with_threshold(Duration::from_secs(3));
    assert_eq!(detector.threshold(), Duration::from_secs(3));

    detector.record_inactivity(Duration::from_secs(4));
    assert!(detector.is_struggling());
}

// ═══════════════════════════════════════════════════════════════════
// Wrong Attempt Detection Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_struggle_after_wrong_attempt() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    assert!(detector.is_struggling());
    assert_eq!(detector.reason(), Some(StruggleReason::WrongAttempt));
}

#[test]
fn test_correct_attempt_not_struggling() {
    let mut detector = StruggleDetector::new();
    detector.record_correct_attempt();
    assert!(!detector.is_struggling());
}

#[test]
fn test_wrong_attempt_count_tracked() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    detector.record_wrong_attempt();
    detector.record_wrong_attempt();
    assert_eq!(detector.wrong_attempts(), 3);
}

// ═══════════════════════════════════════════════════════════════════
// Reset Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_reset_clears_struggle() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    assert!(detector.is_struggling());

    detector.reset();
    assert!(!detector.is_struggling());
    assert!(detector.reason().is_none());
}

#[test]
fn test_reset_clears_wrong_attempts() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    detector.record_wrong_attempt();
    assert_eq!(detector.wrong_attempts(), 2);

    detector.reset();
    assert_eq!(detector.wrong_attempts(), 0);
}

#[test]
fn test_activity_resets_inactivity_struggle() {
    let mut detector = StruggleDetector::new();
    detector.record_inactivity(Duration::from_secs(10));
    assert!(detector.is_struggling());

    detector.record_activity();
    assert!(!detector.is_struggling());
}

// ═══════════════════════════════════════════════════════════════════
// Hint Request Tracking
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_hint_shown_tracked() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    assert!(!detector.hint_shown());

    detector.mark_hint_shown();
    assert!(detector.hint_shown());
}

#[test]
fn test_hint_shown_resets_with_new_exercise() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    detector.mark_hint_shown();
    assert!(detector.hint_shown());

    detector.reset();
    assert!(!detector.hint_shown());
}

// ═══════════════════════════════════════════════════════════════════
// Edge Cases
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_multiple_struggle_reasons() {
    let mut detector = StruggleDetector::new();

    // First: inactivity
    detector.record_inactivity(Duration::from_secs(6));
    assert_eq!(detector.reason(), Some(StruggleReason::Inactivity));

    // Then: wrong attempt (should update reason)
    detector.record_wrong_attempt();
    assert_eq!(detector.reason(), Some(StruggleReason::WrongAttempt));
}

#[test]
fn test_zero_duration_not_struggling() {
    let mut detector = StruggleDetector::new();
    detector.record_inactivity(Duration::from_secs(0));
    assert!(!detector.is_struggling());
}

#[test]
fn test_default_trait() {
    let detector = StruggleDetector::default();
    assert!(!detector.is_struggling());
}
