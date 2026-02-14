#![cfg(feature = "web-tests")]
/// Tests for Struggle Detection Logic
///
/// Struggle detection triggers Socratic hints when:
/// 1. User is inactive for threshold time (trigger_inactivity called)
/// 2. User submits a wrong answer

use logicaffeine_web::struggle::{StruggleDetector, StruggleReason, StruggleConfig};

// ═══════════════════════════════════════════════════════════════════
// Basic State Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_no_struggle_initially() {
    let detector = StruggleDetector::new();
    assert!(!detector.is_struggling);
    assert!(detector.reason().is_none());
}

#[test]
fn test_default_threshold_is_5_seconds() {
    let detector = StruggleDetector::new();
    assert_eq!(detector.config.inactivity_threshold_secs, 5);
}

// ═══════════════════════════════════════════════════════════════════
// Inactivity Detection Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_struggle_after_inactivity_triggered() {
    let mut detector = StruggleDetector::new();
    detector.trigger_inactivity();
    assert!(detector.is_struggling);
    assert_eq!(detector.reason(), Some(StruggleReason::Inactivity));
}

#[test]
fn test_inactivity_threshold_configurable() {
    let config = StruggleConfig {
        inactivity_threshold_secs: 3,
        wrong_attempt_threshold: 1,
    };
    let detector = StruggleDetector::with_config(config);
    assert_eq!(detector.config.inactivity_threshold_secs, 3);
}

// ═══════════════════════════════════════════════════════════════════
// Wrong Attempt Detection Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_struggle_after_wrong_attempt() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    assert!(detector.is_struggling);
    assert_eq!(detector.reason(), Some(StruggleReason::WrongAttempt));
}

#[test]
fn test_correct_attempt_not_struggling() {
    let mut detector = StruggleDetector::new();
    detector.record_correct_attempt();
    assert!(!detector.is_struggling);
}

#[test]
fn test_wrong_attempt_count_tracked() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    detector.record_wrong_attempt();
    detector.record_wrong_attempt();
    assert_eq!(detector.wrong_attempts, 3);
}

// ═══════════════════════════════════════════════════════════════════
// Reset Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_reset_clears_struggle() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    assert!(detector.is_struggling);

    detector.reset();
    assert!(!detector.is_struggling);
    assert!(detector.reason().is_none());
}

#[test]
fn test_reset_clears_wrong_attempts() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    detector.record_wrong_attempt();
    assert_eq!(detector.wrong_attempts, 2);

    detector.reset();
    assert_eq!(detector.wrong_attempts, 0);
}

#[test]
fn test_activity_resets_inactivity_flag() {
    let mut detector = StruggleDetector::new();
    detector.trigger_inactivity();
    assert!(detector.inactivity_triggered);

    detector.record_activity();
    assert!(!detector.inactivity_triggered);
}

// ═══════════════════════════════════════════════════════════════════
// Hint Display Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_should_show_hints() {
    let mut detector = StruggleDetector::new();
    assert!(!detector.should_show_hints());

    detector.record_wrong_attempt();
    assert!(detector.should_show_hints());
}

#[test]
fn test_struggle_message() {
    let mut detector = StruggleDetector::new();
    assert!(detector.struggle_message().is_none());

    detector.trigger_inactivity();
    assert!(detector.struggle_message().is_some());
    assert!(detector.struggle_message().unwrap().contains("hint"));
}

// ═══════════════════════════════════════════════════════════════════
// Edge Cases
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_multiple_struggle_reasons() {
    let mut detector = StruggleDetector::new();

    // First: inactivity
    detector.trigger_inactivity();
    assert_eq!(detector.reason(), Some(StruggleReason::Inactivity));

    // Then: wrong attempt (should update reason)
    detector.record_wrong_attempt();
    assert_eq!(detector.reason(), Some(StruggleReason::WrongAttempt));
}

#[test]
fn test_inactivity_only_triggers_once() {
    let mut detector = StruggleDetector::new();

    detector.trigger_inactivity();
    assert!(detector.inactivity_triggered);

    // Clear reason to test that triggering again doesn't set it
    detector.reason = None;
    detector.trigger_inactivity();
    assert!(detector.reason.is_none()); // Didn't set it again
}

#[test]
fn test_default_trait() {
    let detector = StruggleDetector::default();
    assert!(!detector.is_struggling);
}

#[test]
fn test_configurable_wrong_attempt_threshold() {
    let config = StruggleConfig {
        inactivity_threshold_secs: 5,
        wrong_attempt_threshold: 2,
    };
    let mut detector = StruggleDetector::with_config(config);

    // First wrong attempt shouldn't trigger with threshold of 2
    detector.record_wrong_attempt();
    assert!(!detector.is_struggling);

    // Second wrong attempt should trigger
    detector.record_wrong_attempt();
    assert!(detector.is_struggling);
}
