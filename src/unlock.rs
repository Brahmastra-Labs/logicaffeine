//! Module unlock logic for the Logicaffeine curriculum.
//!
//! # Unlock Rules
//!
//! 1. **First module** in each era is always unlocked
//! 2. **Sequential unlock**: A module unlocks when the previous module is completed
//! 3. **Last two restriction**: The last 2 modules in each era remain locked until
//!    at least one module in that era is 100% complete
//!
//! # Module States
//!
//! - `Locked`: Cannot access (prerequisites not met)
//! - `Available`: Can start (unlocked but not started)
//! - `Started`: In progress (some exercises done)
//! - `Progressing`: 50-99% complete
//! - `Mastered`: 100% complete
//! - `Perfected`: 100% complete with 90%+ accuracy

use crate::content::ContentEngine;
use crate::progress::{ModuleProgress, UserProgress};

/// The state of a module for display purposes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    /// Cannot access - prerequisites not met
    Locked,
    /// Can start - unlocked but not started
    Available,
    /// In progress - some exercises done (1-49%)
    Started,
    /// Making good progress (50-99%)
    Progressing,
    /// 100% complete
    Mastered,
    /// 100% complete with 90%+ accuracy
    Perfected,
}

impl ModuleState {
    /// Get the icon for this state
    pub fn icon(&self) -> &'static str {
        match self {
            ModuleState::Locked => "\u{1F512}",      // 🔒
            ModuleState::Available => "\u{2591}\u{2591}\u{2591}", // ░░░
            ModuleState::Started => "\u{2B50}\u{2591}\u{2591}",   // ⭐░░
            ModuleState::Progressing => "\u{2B50}\u{2B50}\u{2591}", // ⭐⭐░
            ModuleState::Mastered => "\u{2B50}\u{2B50}\u{2B50}",  // ⭐⭐⭐
            ModuleState::Perfected => "\u{1F451}",   // 👑
        }
    }

    /// Get a label for this state
    pub fn label(&self) -> &'static str {
        match self {
            ModuleState::Locked => "locked",
            ModuleState::Available => "not started",
            ModuleState::Started => "in progress",
            ModuleState::Progressing => "progressing",
            ModuleState::Mastered => "mastered",
            ModuleState::Perfected => "perfected",
        }
    }
}

/// Check if a specific module is unlocked for the user.
///
/// # Arguments
/// * `progress` - The user's progress data
/// * `engine` - The content engine with curriculum structure
/// * `era_id` - The era identifier
/// * `module_id` - The module identifier to check
///
/// # Returns
/// `true` if the module is unlocked, `false` otherwise
pub fn check_module_unlocked(
    progress: &UserProgress,
    engine: &ContentEngine,
    era_id: &str,
    module_id: &str,
) -> bool {
    // Get the era
    let era = match engine.get_era(era_id) {
        Some(e) => e,
        None => return false,
    };

    // Find the module index in this era
    let module_index = match era
        .modules
        .iter()
        .position(|m| m.meta.id == module_id)
    {
        Some(idx) => idx,
        None => return false,
    };

    let module_count = era.modules.len();

    // Rule 1: First module is always unlocked
    if module_index == 0 {
        return true;
    }

    // Rule 3: Check if this is one of the last 2 modules
    let is_last_two = module_index >= module_count.saturating_sub(2);

    if is_last_two {
        // Last 2 modules require at least 1 completed module in the era
        let has_any_complete = era
            .modules
            .iter()
            .any(|m| is_module_complete(progress, &m.meta.id));

        if !has_any_complete {
            return false;
        }
    }

    // Rule 2: Previous module must be complete
    let prev_module_id = &era.modules[module_index - 1].meta.id;
    is_module_complete(progress, prev_module_id)
}

/// Check if a module is marked as complete in user progress
pub fn is_module_complete(progress: &UserProgress, module_id: &str) -> bool {
    progress
        .modules
        .get(module_id)
        .map(|m| m.completed)
        .unwrap_or(false)
}

/// Get a list of all locked module IDs for a given era.
///
/// # Arguments
/// * `progress` - The user's progress data
/// * `engine` - The content engine with curriculum structure
/// * `era_id` - The era identifier
///
/// # Returns
/// A vector of module IDs that are currently locked
pub fn get_locked_module_ids(
    progress: &UserProgress,
    engine: &ContentEngine,
    era_id: &str,
) -> Vec<String> {
    let era = match engine.get_era(era_id) {
        Some(e) => e,
        None => return vec![],
    };

    era.modules
        .iter()
        .filter(|m| !check_module_unlocked(progress, engine, era_id, &m.meta.id))
        .map(|m| m.meta.id.clone())
        .collect()
}

/// Get the display state for a module.
///
/// # Arguments
/// * `progress` - The user's progress data
/// * `engine` - The content engine with curriculum structure
/// * `era_id` - The era identifier
/// * `module_id` - The module identifier
///
/// # Returns
/// The current state of the module for UI display
pub fn get_module_state(
    progress: &UserProgress,
    engine: &ContentEngine,
    era_id: &str,
    module_id: &str,
) -> ModuleState {
    // First check if unlocked
    if !check_module_unlocked(progress, engine, era_id, module_id) {
        return ModuleState::Locked;
    }

    // Check progress
    match progress.modules.get(module_id) {
        None => ModuleState::Available,
        Some(mp) => {
            if mp.completed {
                // Check accuracy for perfected status
                // For now, use stars as a proxy (3 stars = 90%+)
                if mp.stars >= 3 && mp.best_score >= 90 {
                    ModuleState::Perfected
                } else {
                    ModuleState::Mastered
                }
            } else if mp.best_score >= 50 {
                ModuleState::Progressing
            } else if mp.attempts > 0 || mp.best_score > 0 {
                ModuleState::Started
            } else {
                ModuleState::Available
            }
        }
    }
}

/// Count how many modules are completed in an era.
pub fn count_completed_modules(
    progress: &UserProgress,
    engine: &ContentEngine,
    era_id: &str,
) -> usize {
    let era = match engine.get_era(era_id) {
        Some(e) => e,
        None => return 0,
    };

    era.modules
        .iter()
        .filter(|m| is_module_complete(progress, &m.meta.id))
        .count()
}

/// Calculate the completion percentage for a module based on exercises.
///
/// # Arguments
/// * `progress` - The user's progress data
/// * `engine` - The content engine
/// * `era_id` - The era identifier
/// * `module_id` - The module identifier
///
/// # Returns
/// Percentage complete (0-100)
pub fn get_module_completion_percent(
    progress: &UserProgress,
    engine: &ContentEngine,
    era_id: &str,
    module_id: &str,
) -> u32 {
    let module = match engine.get_module(era_id, module_id) {
        Some(m) => m,
        None => return 0,
    };

    let total_exercises = module.exercises.len();
    if total_exercises == 0 {
        return 0;
    }

    // Count exercises with at least one correct answer
    let completed = module
        .exercises
        .iter()
        .filter(|ex| {
            progress
                .exercises
                .get(&ex.id)
                .map(|p| p.correct_count > 0)
                .unwrap_or(false)
        })
        .count();

    ((completed as f64 / total_exercises as f64) * 100.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_state_icons() {
        assert!(!ModuleState::Locked.icon().is_empty());
        assert!(!ModuleState::Available.icon().is_empty());
        assert!(!ModuleState::Mastered.icon().is_empty());
    }

    #[test]
    fn test_module_state_labels() {
        assert_eq!(ModuleState::Locked.label(), "locked");
        assert_eq!(ModuleState::Available.label(), "not started");
    }
}
