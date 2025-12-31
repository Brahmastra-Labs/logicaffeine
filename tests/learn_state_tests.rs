/// Tests for Tab and Focus state management
///
/// Tab Modes: LESSON | EXAMPLES | PRACTICE | TEST
/// Focus Mode: Collapse other eras when one is expanded

use logos::learn_state::{TabMode, ModuleTabState, FocusState};

// ═══════════════════════════════════════════════════════════════════
// TabMode Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_tab_modes_all_four_exist() {
    let tabs = TabMode::all();
    assert_eq!(tabs.len(), 4);
}

#[test]
fn test_tab_mode_labels() {
    assert_eq!(TabMode::Lesson.label(), "LESSON");
    assert_eq!(TabMode::Examples.label(), "EXAMPLES");
    assert!(TabMode::Practice.label().contains("PRACTICE"));
    assert!(TabMode::Test.label().contains("TEST"));
}

#[test]
fn test_tab_mode_default_is_lesson() {
    assert_eq!(TabMode::default(), TabMode::Lesson);
}

// ═══════════════════════════════════════════════════════════════════
// ModuleTabState Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_initial_tab_is_lesson() {
    let state = ModuleTabState::new("introduction");
    assert_eq!(state.current_tab, TabMode::Lesson);
}

#[test]
fn test_tab_switch() {
    let mut state = ModuleTabState::new("introduction");
    state.set_tab(TabMode::Practice);
    assert_eq!(state.current_tab, TabMode::Practice);
}

#[test]
fn test_tab_switch_resets_exercise_index() {
    let mut state = ModuleTabState::new("introduction");
    state.exercise_index = 5;
    state.set_tab(TabMode::Test);
    assert_eq!(state.exercise_index, 0, "Exercise index should reset on tab change");
}

#[test]
fn test_tab_switch_resets_submitted() {
    let mut state = ModuleTabState::new("introduction");
    state.submitted = true;
    state.set_tab(TabMode::Practice);
    assert!(!state.submitted, "Submitted flag should reset on tab change");
}

#[test]
fn test_same_tab_no_reset() {
    let mut state = ModuleTabState::new("introduction");
    state.exercise_index = 5;
    state.submitted = true;
    state.set_tab(TabMode::Lesson); // Same as current
    assert_eq!(state.exercise_index, 5, "Same tab should not reset index");
    assert!(state.submitted, "Same tab should not reset submitted");
}

#[test]
fn test_module_tab_state_module_id() {
    let state = ModuleTabState::new("syllogistic");
    assert_eq!(state.module_id, "syllogistic");
}

// ═══════════════════════════════════════════════════════════════════
// FocusState Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_initial_focus_is_none() {
    let focus = FocusState::new();
    assert!(focus.focused_era.is_none());
    assert!(!focus.is_focused());
}

#[test]
fn test_focus_era() {
    let mut focus = FocusState::new();
    focus.set_focus("logic-caffeine");
    assert_eq!(focus.focused_era, Some("logic-caffeine".to_string()));
    assert!(focus.is_focused());
}

#[test]
fn test_toggle_focus_same_era_clears() {
    let mut focus = FocusState::new();
    focus.set_focus("logic-caffeine");
    focus.toggle_focus("logic-caffeine");
    assert!(focus.focused_era.is_none());
}

#[test]
fn test_toggle_focus_different_era_switches() {
    let mut focus = FocusState::new();
    focus.set_focus("logic-caffeine");
    focus.toggle_focus("other-era");
    assert_eq!(focus.focused_era, Some("other-era".to_string()));
}

#[test]
fn test_is_era_visible_when_no_focus() {
    let focus = FocusState::new();
    assert!(focus.is_era_visible("logic-caffeine"));
    assert!(focus.is_era_visible("other-era"));
    assert!(focus.is_era_visible("any-era"));
}

#[test]
fn test_is_era_visible_when_focused() {
    let mut focus = FocusState::new();
    focus.set_focus("logic-caffeine");
    assert!(focus.is_era_visible("logic-caffeine"));
    assert!(!focus.is_era_visible("other-era"));
}

#[test]
fn test_clear_focus() {
    let mut focus = FocusState::new();
    focus.set_focus("logic-caffeine");
    focus.clear_focus();
    assert!(focus.focused_era.is_none());
}

#[test]
fn test_focus_state_default() {
    let focus = FocusState::default();
    assert!(focus.focused_era.is_none());
}

// ═══════════════════════════════════════════════════════════════════
// Expanded Module State Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_focus_module() {
    let mut focus = FocusState::new();
    focus.set_focus_module("logic-caffeine", "introduction");
    assert!(focus.is_module_focused("logic-caffeine", "introduction"));
    assert!(!focus.is_module_focused("logic-caffeine", "syllogistic"));
}

#[test]
fn test_toggle_focus_module_same_clears() {
    let mut focus = FocusState::new();
    focus.set_focus_module("logic-caffeine", "introduction");
    focus.toggle_focus_module("logic-caffeine", "introduction");
    assert!(!focus.is_module_focused("logic-caffeine", "introduction"));
}

#[test]
fn test_toggle_focus_module_different_switches() {
    let mut focus = FocusState::new();
    focus.set_focus_module("logic-caffeine", "introduction");
    focus.toggle_focus_module("logic-caffeine", "syllogistic");
    assert!(!focus.is_module_focused("logic-caffeine", "introduction"));
    assert!(focus.is_module_focused("logic-caffeine", "syllogistic"));
}
