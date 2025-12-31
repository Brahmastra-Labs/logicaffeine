//! Tab & Focus State Management for Learn Page
//!
//! Manages the state for the integrated learn page experience:
//! - Tab modes (Lesson, Examples, Practice, Test)
//! - Focus state (which era/module is expanded)
//! - Exercise navigation within modes

/// The four tab modes available for each module
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabMode {
    #[default]
    Lesson,
    Examples,
    Practice,
    Test,
}

impl TabMode {
    /// Get display label for the tab
    pub fn label(&self) -> &'static str {
        match self {
            TabMode::Lesson => "LESSON",
            TabMode::Examples => "EXAMPLES",
            TabMode::Practice => "PRACTICE",
            TabMode::Test => "TEST",
        }
    }

    /// Get all tab modes in order
    pub fn all() -> [TabMode; 4] {
        [TabMode::Lesson, TabMode::Examples, TabMode::Practice, TabMode::Test]
    }
}

/// State for a single module's tab interface
#[derive(Debug, Clone, Default)]
pub struct ModuleTabState {
    pub module_id: String,
    pub current_tab: TabMode,
    pub exercise_index: usize,
    pub submitted: bool,
}

impl ModuleTabState {
    pub fn new(module_id: &str) -> Self {
        Self {
            module_id: module_id.to_string(),
            current_tab: TabMode::Lesson,
            exercise_index: 0,
            submitted: false,
        }
    }

    /// Switch to a new tab, resetting exercise state
    pub fn switch_tab(&mut self, tab: TabMode) {
        self.current_tab = tab;
        self.exercise_index = 0;
        self.submitted = false;
    }

    /// Reset exercise state without changing tab
    pub fn reset_exercise(&mut self) {
        self.exercise_index = 0;
        self.submitted = false;
    }
}

/// Tracks which era is currently focused (expanded)
#[derive(Debug, Clone, Default)]
pub struct FocusState {
    /// The currently focused era (None = no focus, all eras visible)
    pub focused_era: Option<String>,
    /// The currently expanded module within the focused era
    pub expanded_module: Option<(String, String)>, // (era_id, module_id)
}

impl FocusState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Focus on a specific era
    pub fn focus_era(&mut self, era_id: &str) {
        self.focused_era = Some(era_id.to_string());
    }

    /// Expand a module within an era
    pub fn expand_module(&mut self, era_id: &str, module_id: &str) {
        self.focused_era = Some(era_id.to_string());
        self.expanded_module = Some((era_id.to_string(), module_id.to_string()));
    }

    /// Collapse the current module (but keep era focused)
    pub fn collapse_module(&mut self) {
        self.expanded_module = None;
    }

    /// Unfocus completely (show all eras)
    pub fn unfocus(&mut self) {
        self.focused_era = None;
        self.expanded_module = None;
    }

    /// Check if a specific era is visible (either focused or no focus)
    pub fn is_era_visible(&self, era_id: &str) -> bool {
        match &self.focused_era {
            None => true,
            Some(focused) => focused == era_id,
        }
    }

    /// Check if a specific module is expanded
    pub fn is_module_expanded(&self, era_id: &str, module_id: &str) -> bool {
        match &self.expanded_module {
            None => false,
            Some((e, m)) => e == era_id && m == module_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_modes_all_four_exist() {
        let tabs = TabMode::all();
        assert_eq!(tabs.len(), 4);
        assert_eq!(tabs[0], TabMode::Lesson);
        assert_eq!(tabs[1], TabMode::Examples);
        assert_eq!(tabs[2], TabMode::Practice);
        assert_eq!(tabs[3], TabMode::Test);
    }

    #[test]
    fn test_initial_tab_is_lesson() {
        let state = ModuleTabState::new("test-module");
        assert_eq!(state.current_tab, TabMode::Lesson);
    }

    #[test]
    fn test_tab_switch_resets_exercise_index() {
        let mut state = ModuleTabState::new("test-module");
        state.exercise_index = 5;
        state.submitted = true;

        state.switch_tab(TabMode::Practice);

        assert_eq!(state.current_tab, TabMode::Practice);
        assert_eq!(state.exercise_index, 0);
        assert!(!state.submitted);
    }

    #[test]
    fn test_focus_state_toggles_era() {
        let mut focus = FocusState::new();
        assert!(focus.focused_era.is_none());

        focus.focus_era("first-steps");
        assert_eq!(focus.focused_era, Some("first-steps".to_string()));

        focus.unfocus();
        assert!(focus.focused_era.is_none());
    }

    #[test]
    fn test_is_era_visible_when_focused() {
        let mut focus = FocusState::new();

        // No focus = all eras visible
        assert!(focus.is_era_visible("first-steps"));
        assert!(focus.is_era_visible("mastery"));

        // Focus on one era = only that era visible
        focus.focus_era("first-steps");
        assert!(focus.is_era_visible("first-steps"));
        assert!(!focus.is_era_visible("mastery"));
    }

    #[test]
    fn test_module_expansion() {
        let mut focus = FocusState::new();

        focus.expand_module("first-steps", "introduction");
        assert!(focus.is_module_expanded("first-steps", "introduction"));
        assert!(!focus.is_module_expanded("first-steps", "syllogistic"));

        focus.collapse_module();
        assert!(!focus.is_module_expanded("first-steps", "introduction"));
        // Era should still be focused
        assert!(focus.is_era_visible("first-steps"));
    }
}
