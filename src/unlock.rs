//! Module Unlock Logic
//!
//! Rules:
//! - First module in each era is always unlocked
//! - Subsequent modules unlock when the previous module is completed
//! - Last two modules in each era are locked until at least one module has 100% completion

use crate::content::ContentEngine;
use crate::progress::UserProgress;

/// State of a module for the user
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    /// Module is locked and cannot be accessed
    Locked,
    /// Module is unlocked but not started
    Available,
    /// Module has been started (score < 50%)
    Started,
    /// Module is in progress (50-89% score)
    Progressing,
    /// Module has been completed but not perfected (90%+ score)
    Completed,
    /// Module has been perfected (100% or 3 stars)
    Perfected,
}

/// Get the current state of a module for the user
pub fn get_module_state(
    progress: &UserProgress,
    engine: &ContentEngine,
    era_id: &str,
    module_id: &str,
) -> ModuleState {
    // Check if locked
    if !check_module_unlocked(progress, engine, era_id, module_id) {
        return ModuleState::Locked;
    }

    // Check progress
    match progress.modules.get(module_id) {
        None => ModuleState::Available,
        Some(mp) => {
            if mp.completed && (mp.best_score >= 90 || mp.stars >= 3) {
                ModuleState::Perfected
            } else if mp.completed {
                ModuleState::Completed
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

/// Check if a specific module is unlocked for the user
pub fn check_module_unlocked(
    progress: &UserProgress,
    engine: &ContentEngine,
    era_id: &str,
    module_id: &str,
) -> bool {
    let Some(era) = engine.get_era(era_id) else {
        return false;
    };

    let module_ids: Vec<&str> = era.modules.iter().map(|m| m.meta.id.as_str()).collect();
    let Some(module_index) = module_ids.iter().position(|&id| id == module_id) else {
        return false;
    };

    let total_modules = module_ids.len();

    // First module is always unlocked
    if module_index == 0 {
        return true;
    }

    // Check if this is one of the last two modules
    let is_final_module = total_modules >= 2 && module_index >= total_modules - 2;

    if is_final_module {
        // Last two modules require at least one module to be 100% complete
        let has_perfect_completion = module_ids.iter().take(total_modules.saturating_sub(2)).any(|&mid| {
            progress.modules.get(mid).map_or(false, |mp| mp.completed && mp.best_score >= 100)
        });

        if !has_perfect_completion {
            return false;
        }
    }

    // Check if previous module is completed
    let prev_module_id = module_ids[module_index - 1];
    progress.modules.get(prev_module_id).map_or(false, |mp| mp.completed)
}

/// Get list of locked module IDs for an era
pub fn get_locked_module_ids(
    progress: &UserProgress,
    engine: &ContentEngine,
    era_id: &str,
) -> Vec<String> {
    let Some(era) = engine.get_era(era_id) else {
        return Vec::new();
    };

    era.modules
        .iter()
        .filter(|m| !check_module_unlocked(progress, engine, era_id, &m.meta.id))
        .map(|m| m.meta.id.clone())
        .collect()
}

/// Check if any module in the era has 100% completion
pub fn has_perfect_module(progress: &UserProgress, engine: &ContentEngine, era_id: &str) -> bool {
    let Some(era) = engine.get_era(era_id) else {
        return false;
    };

    era.modules.iter().any(|m| {
        progress.modules.get(&m.meta.id).map_or(false, |mp| mp.completed && mp.best_score >= 100)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::ModuleProgress;

    fn make_progress_with_completed(completed_modules: &[(&str, bool, u32)]) -> UserProgress {
        let mut progress = UserProgress::new();
        for (id, completed, score) in completed_modules {
            progress.modules.insert(id.to_string(), ModuleProgress {
                module_id: id.to_string(),
                unlocked: true,
                completed: *completed,
                stars: 0,
                best_score: *score,
                attempts: 1,
            });
        }
        progress
    }

    #[test]
    fn test_first_module_always_unlocked() {
        let progress = UserProgress::new();
        let engine = ContentEngine::new();

        // First module of first era should always be unlocked
        if let Some(era) = engine.eras().first() {
            if let Some(module) = era.modules.first() {
                assert!(check_module_unlocked(&progress, &engine, &era.meta.id, &module.meta.id));
            }
        }
    }

    #[test]
    fn test_module_locked_until_previous_complete() {
        let engine = ContentEngine::new();

        if let Some(era) = engine.eras().first() {
            if era.modules.len() >= 2 {
                let first_id = &era.modules[0].meta.id;
                let second_id = &era.modules[1].meta.id;

                // Without completing first module, second should be locked
                let progress = UserProgress::new();
                assert!(!check_module_unlocked(&progress, &engine, &era.meta.id, second_id));

                // After completing first module, second should be unlocked
                let progress = make_progress_with_completed(&[(first_id.as_str(), true, 80)]);
                assert!(check_module_unlocked(&progress, &engine, &era.meta.id, second_id));
            }
        }
    }

    #[test]
    fn test_last_two_locked_until_one_module_100_complete() {
        let engine = ContentEngine::new();

        // Find an era with at least 4 modules
        for era in engine.eras() {
            if era.modules.len() >= 4 {
                let module_ids: Vec<&str> = era.modules.iter().map(|m| m.meta.id.as_str()).collect();
                let last_module_id = module_ids[module_ids.len() - 1];
                let second_last_id = module_ids[module_ids.len() - 2];

                // Complete all modules except last two, but none at 100%
                let mut completed: Vec<(&str, bool, u32)> = module_ids[..module_ids.len()-2]
                    .iter()
                    .map(|id| (*id, true, 80u32))
                    .collect();

                let progress = make_progress_with_completed(&completed);

                // Last two should still be locked (no 100% completion)
                assert!(!check_module_unlocked(&progress, &engine, &era.meta.id, second_last_id),
                    "Second-to-last module should be locked without 100% completion");
                assert!(!check_module_unlocked(&progress, &engine, &era.meta.id, last_module_id),
                    "Last module should be locked without 100% completion");

                // Now complete one module at 100%
                completed[0].2 = 100;
                let progress = make_progress_with_completed(&completed);

                // Second-to-last should now be unlocked
                assert!(check_module_unlocked(&progress, &engine, &era.meta.id, second_last_id),
                    "Second-to-last module should be unlocked with 100% completion");

                break;
            }
        }
    }

    #[test]
    fn test_get_locked_module_ids() {
        let progress = UserProgress::new();
        let engine = ContentEngine::new();

        if let Some(era) = engine.eras().first() {
            let locked = get_locked_module_ids(&progress, &engine, &era.meta.id);

            // All modules except the first should be locked initially
            assert_eq!(locked.len(), era.modules.len() - 1);

            // First module should NOT be in the locked list
            if let Some(first_module) = era.modules.first() {
                assert!(!locked.contains(&first_module.meta.id));
            }
        }
    }
}
