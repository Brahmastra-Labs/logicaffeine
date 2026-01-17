//! Struggle Detection Logic
//!
//! Detects when a user is struggling with an exercise based on:
//! - Inactivity (no answer attempt after threshold time)
//! - Wrong attempts (incorrect answers)

/// Reasons why a user might be struggling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StruggleReason {
    Inactivity,
    WrongAttempt,
}

impl StruggleReason {
    /// Get a message describing the struggle reason
    pub fn message(&self) -> &'static str {
        match self {
            StruggleReason::Inactivity => "Taking your time? Here's a hint to help you along.",
            StruggleReason::WrongAttempt => "Not quite! Let me help you think through this.",
        }
    }
}

/// Configuration for struggle detection
#[derive(Debug, Clone, Copy)]
pub struct StruggleConfig {
    /// Seconds of inactivity before considering the user stuck
    pub inactivity_threshold_secs: u64,
    /// Number of wrong attempts before showing help
    pub wrong_attempt_threshold: u32,
}

impl Default for StruggleConfig {
    fn default() -> Self {
        Self {
            inactivity_threshold_secs: 5,
            wrong_attempt_threshold: 1,
        }
    }
}

/// Tracks struggle state for an exercise
#[derive(Debug, Clone, Default)]
pub struct StruggleDetector {
    pub config: StruggleConfig,
    pub is_struggling: bool,
    pub reason: Option<StruggleReason>,
    pub wrong_attempts: u32,
    pub inactivity_triggered: bool,
}

impl StruggleDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: StruggleConfig) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    /// Record a wrong attempt - may trigger struggle state
    pub fn record_wrong_attempt(&mut self) {
        self.wrong_attempts += 1;
        if self.wrong_attempts >= self.config.wrong_attempt_threshold {
            self.is_struggling = true;
            self.reason = Some(StruggleReason::WrongAttempt);
        }
    }

    /// Record a correct attempt - resets inactivity but keeps struggle state for hints
    pub fn record_correct_attempt(&mut self) {
        // User got it right - they're no longer struggling
        self.is_struggling = false;
        self.inactivity_triggered = false;
    }

    /// Record user activity (typing, clicking) - resets inactivity timer
    pub fn record_activity(&mut self) {
        // Activity resets inactivity detection
        self.inactivity_triggered = false;
    }

    /// Called when inactivity threshold is reached
    pub fn trigger_inactivity(&mut self) {
        if !self.inactivity_triggered {
            self.inactivity_triggered = true;
            self.is_struggling = true;
            // Only set reason if not already struggling from wrong attempts
            if self.reason.is_none() {
                self.reason = Some(StruggleReason::Inactivity);
            }
        }
    }

    /// Reset struggle state (e.g., when moving to next exercise)
    pub fn reset(&mut self) {
        self.is_struggling = false;
        self.reason = None;
        self.wrong_attempts = 0;
        self.inactivity_triggered = false;
    }

    /// Check if we should show hints
    pub fn should_show_hints(&self) -> bool {
        self.is_struggling
    }

    /// Get the current struggle reason
    pub fn reason(&self) -> Option<StruggleReason> {
        self.reason
    }

    /// Get the current struggle reason for display
    pub fn struggle_message(&self) -> Option<&'static str> {
        match self.reason {
            Some(StruggleReason::Inactivity) => Some("Taking your time? Here's a hint to help you along."),
            Some(StruggleReason::WrongAttempt) => Some("Not quite! Let me help you think through this."),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
