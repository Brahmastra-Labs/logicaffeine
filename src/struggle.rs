//! Struggle detection for triggering Socratic hints.
//!
//! The struggle detector monitors user activity and triggers hints when:
//! - The user has been inactive for 5+ seconds (configurable)
//! - The user submits a wrong answer
//!
//! This implements a gentle pedagogical approach: hints appear when
//! students need them most, not as punishments but as support.

use std::time::Duration;

/// Default inactivity threshold before triggering a hint
const DEFAULT_INACTIVITY_THRESHOLD: Duration = Duration::from_secs(5);

/// The reason why the user is struggling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StruggleReason {
    /// User hasn't typed anything for a while
    Inactivity,
    /// User submitted an incorrect answer
    WrongAttempt,
}

impl StruggleReason {
    /// Get a human-readable message for this reason
    pub fn message(&self) -> &'static str {
        match self {
            StruggleReason::Inactivity => "Taking your time? Here's a hint.",
            StruggleReason::WrongAttempt => "Not quite. Let me help you.",
        }
    }
}

/// Detects when a user is struggling and should receive a hint.
#[derive(Debug, Clone)]
pub struct StruggleDetector {
    /// Inactivity threshold before triggering
    threshold: Duration,
    /// Whether the user is currently struggling
    is_struggling: bool,
    /// The reason for struggling (if any)
    reason: Option<StruggleReason>,
    /// Number of wrong attempts on current exercise
    wrong_attempts: u32,
    /// Whether a hint has been shown for this struggle
    hint_shown: bool,
}

impl StruggleDetector {
    /// Create a new detector with default 5-second threshold
    pub fn new() -> Self {
        Self {
            threshold: DEFAULT_INACTIVITY_THRESHOLD,
            is_struggling: false,
            reason: None,
            wrong_attempts: 0,
            hint_shown: false,
        }
    }

    /// Create a detector with a custom inactivity threshold
    pub fn with_threshold(threshold: Duration) -> Self {
        Self {
            threshold,
            is_struggling: false,
            reason: None,
            wrong_attempts: 0,
            hint_shown: false,
        }
    }

    /// Get the current inactivity threshold
    pub fn threshold(&self) -> Duration {
        self.threshold
    }

    /// Check if the user is currently struggling
    pub fn is_struggling(&self) -> bool {
        self.is_struggling
    }

    /// Get the reason for struggling (if any)
    pub fn reason(&self) -> Option<StruggleReason> {
        self.reason
    }

    /// Get the number of wrong attempts on the current exercise
    pub fn wrong_attempts(&self) -> u32 {
        self.wrong_attempts
    }

    /// Check if a hint has been shown for this struggle
    pub fn hint_shown(&self) -> bool {
        self.hint_shown
    }

    /// Record a period of inactivity
    ///
    /// If the duration exceeds the threshold, triggers struggling state.
    pub fn record_inactivity(&mut self, duration: Duration) {
        if duration >= self.threshold {
            self.is_struggling = true;
            self.reason = Some(StruggleReason::Inactivity);
        }
    }

    /// Record that the user did something (typed, clicked, etc.)
    ///
    /// This clears inactivity-based struggling but not wrong-attempt struggling.
    pub fn record_activity(&mut self) {
        if self.reason == Some(StruggleReason::Inactivity) {
            self.is_struggling = false;
            self.reason = None;
            self.hint_shown = false;
        }
    }

    /// Record a wrong answer attempt
    pub fn record_wrong_attempt(&mut self) {
        self.wrong_attempts += 1;
        self.is_struggling = true;
        self.reason = Some(StruggleReason::WrongAttempt);
        self.hint_shown = false;
    }

    /// Record a correct answer attempt
    ///
    /// This clears the struggling state since the user succeeded.
    pub fn record_correct_attempt(&mut self) {
        // Correct answers clear struggling but we keep wrong_attempts
        // for statistics purposes
        self.is_struggling = false;
        self.reason = None;
        self.hint_shown = false;
    }

    /// Mark that a hint has been shown to the user
    pub fn mark_hint_shown(&mut self) {
        self.hint_shown = true;
    }

    /// Reset all state for a new exercise
    pub fn reset(&mut self) {
        self.is_struggling = false;
        self.reason = None;
        self.wrong_attempts = 0;
        self.hint_shown = false;
    }

    /// Check if we should show a hint now
    ///
    /// Returns true if struggling and hint hasn't been shown yet.
    pub fn should_show_hint(&self) -> bool {
        self.is_struggling && !self.hint_shown
    }
}

impl Default for StruggleDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_struggle_reason_messages() {
        assert!(!StruggleReason::Inactivity.message().is_empty());
        assert!(!StruggleReason::WrongAttempt.message().is_empty());
    }

    #[test]
    fn test_should_show_hint() {
        let mut detector = StruggleDetector::new();

        // Initially: not struggling
        assert!(!detector.should_show_hint());

        // After wrong attempt: should show
        detector.record_wrong_attempt();
        assert!(detector.should_show_hint());

        // After hint shown: should not show again
        detector.mark_hint_shown();
        assert!(!detector.should_show_hint());
    }

    #[test]
    fn test_activity_only_clears_inactivity_struggle() {
        let mut detector = StruggleDetector::new();

        // Wrong attempt struggle should NOT be cleared by activity
        detector.record_wrong_attempt();
        assert!(detector.is_struggling());

        detector.record_activity();
        // Still struggling because it was wrong-attempt, not inactivity
        assert!(detector.is_struggling());
    }
}
