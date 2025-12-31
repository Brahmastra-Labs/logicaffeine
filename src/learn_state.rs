//! State management for the integrated Learn page.
//!
//! This module provides state types for:
//! - Tab navigation within modules (LESSON | EXAMPLES | PRACTICE | TEST)
//! - Focus mode (collapse other eras when one is expanded)
//! - Module expansion state

use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════
// Tab Mode
// ═══════════════════════════════════════════════════════════════════

/// The available tabs within a module section
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TabMode {
    /// Reading content from lesson.md
    #[default]
    Lesson,
    /// Interactive code blocks with symbol dictionary
    Examples,
    /// Infinite flashcard mode, earn XP per correct answer
    Practice,
    /// 17-question assessment with final score
    Test,
}

impl TabMode {
    /// Get the display label for this tab
    pub fn label(&self) -> &'static str {
        match self {
            TabMode::Lesson => "LESSON",
            TabMode::Examples => "EXAMPLES",
            TabMode::Practice => "PRACTICE \u{221E}",  // ∞ infinity symbol
            TabMode::Test => "TEST \u{1F4DD}",          // 📝 memo emoji
        }
    }

    /// Get all tab modes in display order
    pub fn all() -> [TabMode; 4] {
        [TabMode::Lesson, TabMode::Examples, TabMode::Practice, TabMode::Test]
    }

    /// Check if this is a practice or test mode (earns XP)
    pub fn is_exercise_mode(&self) -> bool {
        matches!(self, TabMode::Practice | TabMode::Test)
    }

    /// Check if hints are allowed in this mode
    pub fn allows_hints(&self) -> bool {
        !matches!(self, TabMode::Test)
    }
}

// ═══════════════════════════════════════════════════════════════════
// Module Tab State
// ═══════════════════════════════════════════════════════════════════

/// State for a single module's tab navigation and exercise progress
#[derive(Debug, Clone)]
pub struct ModuleTabState {
    /// The module ID this state belongs to
    pub module_id: String,
    /// Currently selected tab
    pub current_tab: TabMode,
    /// Current exercise index (for Practice/Test modes)
    pub exercise_index: usize,
    /// Whether the current exercise has been submitted
    pub submitted: bool,
    /// User's answer for the current exercise
    pub user_answer: Option<String>,
    /// Combo counter for Practice mode
    pub combo: u32,
    /// Session statistics
    pub session_correct: u32,
    pub session_wrong: u32,
}

impl ModuleTabState {
    /// Create a new module tab state
    pub fn new(module_id: &str) -> Self {
        Self {
            module_id: module_id.to_string(),
            current_tab: TabMode::Lesson,
            exercise_index: 0,
            submitted: false,
            user_answer: None,
            combo: 0,
            session_correct: 0,
            session_wrong: 0,
        }
    }

    /// Switch to a different tab, resetting exercise state if changing tabs
    pub fn set_tab(&mut self, tab: TabMode) {
        if self.current_tab != tab {
            self.current_tab = tab;
            self.exercise_index = 0;
            self.submitted = false;
            self.user_answer = None;
            // Reset combo when leaving Practice mode
            if !matches!(tab, TabMode::Practice) {
                self.combo = 0;
            }
        }
    }

    /// Move to the next exercise
    pub fn next_exercise(&mut self) {
        self.exercise_index += 1;
        self.submitted = false;
        self.user_answer = None;
    }

    /// Record an answer result
    pub fn record_answer(&mut self, correct: bool) {
        self.submitted = true;
        if correct {
            self.session_correct += 1;
            self.combo += 1;
        } else {
            self.session_wrong += 1;
            self.combo = 0;
        }
    }

    /// Get session accuracy as a percentage
    pub fn accuracy(&self) -> f64 {
        let total = self.session_correct + self.session_wrong;
        if total == 0 {
            0.0
        } else {
            (self.session_correct as f64 / total as f64) * 100.0
        }
    }

    /// Reset session statistics
    pub fn reset_session(&mut self) {
        self.session_correct = 0;
        self.session_wrong = 0;
        self.combo = 0;
        self.exercise_index = 0;
        self.submitted = false;
        self.user_answer = None;
    }
}

// ═══════════════════════════════════════════════════════════════════
// Focus State
// ═══════════════════════════════════════════════════════════════════

/// State for focus mode - which era/module is expanded
#[derive(Debug, Clone, Default)]
pub struct FocusState {
    /// Currently focused era (if any)
    pub focused_era: Option<String>,
    /// Currently focused module within the era (if any)
    pub focused_module: Option<String>,
}

impl FocusState {
    /// Create a new focus state with nothing focused
    pub fn new() -> Self {
        Self {
            focused_era: None,
            focused_module: None,
        }
    }

    /// Check if any era is focused
    pub fn is_focused(&self) -> bool {
        self.focused_era.is_some()
    }

    /// Set focus to a specific era
    pub fn set_focus(&mut self, era_id: &str) {
        self.focused_era = Some(era_id.to_string());
        self.focused_module = None;
    }

    /// Toggle focus on an era - if already focused, clear; otherwise focus
    pub fn toggle_focus(&mut self, era_id: &str) {
        if self.focused_era.as_deref() == Some(era_id) {
            self.focused_era = None;
            self.focused_module = None;
        } else {
            self.focused_era = Some(era_id.to_string());
            self.focused_module = None;
        }
    }

    /// Clear all focus
    pub fn clear_focus(&mut self) {
        self.focused_era = None;
        self.focused_module = None;
    }

    /// Check if a specific era is visible (not hidden by focus mode)
    pub fn is_era_visible(&self, era_id: &str) -> bool {
        match &self.focused_era {
            None => true, // No focus means all visible
            Some(focused) => focused == era_id,
        }
    }

    /// Set focus to a specific module within an era
    pub fn set_focus_module(&mut self, era_id: &str, module_id: &str) {
        self.focused_era = Some(era_id.to_string());
        self.focused_module = Some(module_id.to_string());
    }

    /// Toggle focus on a module - if already focused, clear; otherwise focus
    pub fn toggle_focus_module(&mut self, era_id: &str, module_id: &str) {
        if self.is_module_focused(era_id, module_id) {
            self.focused_module = None;
        } else {
            self.focused_era = Some(era_id.to_string());
            self.focused_module = Some(module_id.to_string());
        }
    }

    /// Check if a specific module is focused
    pub fn is_module_focused(&self, era_id: &str, module_id: &str) -> bool {
        self.focused_era.as_deref() == Some(era_id)
            && self.focused_module.as_deref() == Some(module_id)
    }

    /// Check if a module is expanded (either focused or in a focused era with no module focus)
    pub fn is_module_expanded(&self, era_id: &str, module_id: &str) -> bool {
        match (&self.focused_era, &self.focused_module) {
            (None, _) => false, // Nothing focused = nothing expanded
            (Some(e), None) => e == era_id, // Era focused, all modules in era are "semi-expanded"
            (Some(e), Some(m)) => e == era_id && m == module_id, // Specific module focused
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_mode_is_exercise_mode() {
        assert!(!TabMode::Lesson.is_exercise_mode());
        assert!(!TabMode::Examples.is_exercise_mode());
        assert!(TabMode::Practice.is_exercise_mode());
        assert!(TabMode::Test.is_exercise_mode());
    }

    #[test]
    fn test_tab_mode_allows_hints() {
        assert!(TabMode::Lesson.allows_hints());
        assert!(TabMode::Examples.allows_hints());
        assert!(TabMode::Practice.allows_hints());
        assert!(!TabMode::Test.allows_hints());
    }

    #[test]
    fn test_module_tab_state_accuracy() {
        let mut state = ModuleTabState::new("test");
        assert_eq!(state.accuracy(), 0.0);

        state.record_answer(true);
        state.record_answer(true);
        state.record_answer(false);
        assert!((state.accuracy() - 66.67).abs() < 0.1);
    }

    #[test]
    fn test_module_tab_state_combo() {
        let mut state = ModuleTabState::new("test");
        state.set_tab(TabMode::Practice);

        state.record_answer(true);
        assert_eq!(state.combo, 1);
        state.record_answer(true);
        assert_eq!(state.combo, 2);
        state.record_answer(false);
        assert_eq!(state.combo, 0);
    }
}
