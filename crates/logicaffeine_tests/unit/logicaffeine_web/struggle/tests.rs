use super::*;

#[test]
fn test_no_struggle_initially() {
    let detector = StruggleDetector::new();
    assert!(!detector.is_struggling);
    assert!(detector.reason.is_none());
}

#[test]
fn test_struggle_after_5s_inactivity() {
    let mut detector = StruggleDetector::new();
    assert!(!detector.is_struggling);

    detector.trigger_inactivity();

    assert!(detector.is_struggling);
    assert_eq!(detector.reason, Some(StruggleReason::Inactivity));
}

#[test]
fn test_struggle_after_wrong_attempt() {
    let mut detector = StruggleDetector::new();
    assert!(!detector.is_struggling);

    detector.record_wrong_attempt();

    assert!(detector.is_struggling);
    assert_eq!(detector.reason, Some(StruggleReason::WrongAttempt));
}

#[test]
fn test_reset_clears_struggle() {
    let mut detector = StruggleDetector::new();
    detector.record_wrong_attempt();
    assert!(detector.is_struggling);

    detector.reset();

    assert!(!detector.is_struggling);
    assert!(detector.reason.is_none());
    assert_eq!(detector.wrong_attempts, 0);
}

#[test]
fn test_configurable_threshold() {
    let config = StruggleConfig {
        inactivity_threshold_secs: 10,
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

#[test]
fn test_inactivity_only_triggers_once() {
    let mut detector = StruggleDetector::new();

    detector.trigger_inactivity();
    assert!(detector.inactivity_triggered);

    // Triggering again shouldn't change the reason
    detector.reason = None;
    detector.trigger_inactivity();
    assert!(detector.reason.is_none()); // Didn't set it again
}

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
