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
            Some(StruggleReason::Inactivity) => {
                Some("Taking your time? Here's a hint to help you along.")
            }
            Some(StruggleReason::WrongAttempt) => {
                Some("Not quite! Let me help you think through this.")
            }
            None => None,
        }
    }
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_web/struggle/tests.rs"]
mod tests;
