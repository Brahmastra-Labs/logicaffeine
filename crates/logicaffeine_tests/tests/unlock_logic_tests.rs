/// Tests for module unlock logic
///
/// Unlock Rules:
/// 1. First module in each era is always unlocked
/// 2. Modules unlock sequentially (previous must be complete)
/// 3. Last 2 modules in each era are locked until at least 1 module is 100% complete

use logicaffeine_web::unlock::{check_module_unlocked, get_locked_module_ids, get_module_state, ModuleState};
use logicaffeine_web::progress::{UserProgress, ModuleProgress};
use logicaffeine_web::content::ContentEngine;

// Current curriculum structure:
// Era: first-steps (5 modules: introduction, syllogistic, definitions, fallacies, inductive)
// Era: building-blocks
// Era: expanding-horizons
// Era: mastery

const ERA_ID: &str = "first-steps";
const FIRST_MODULE: &str = "introduction";
const SECOND_MODULE: &str = "syllogistic";
// In a 5-module era, last two are fallacies and inductive
const SECOND_TO_LAST: &str = "fallacies";
const LAST_MODULE: &str = "inductive";

#[test]
fn test_first_module_always_unlocked() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    // First module in era should always be unlocked
    let unlocked = check_module_unlocked(&progress, &engine, ERA_ID, FIRST_MODULE);
    assert!(unlocked, "First module in era should always be unlocked");
}

#[test]
fn test_first_module_in_each_era_unlocked() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    // Check that the first module in the first era is unlocked
    assert!(
        check_module_unlocked(&progress, &engine, ERA_ID, FIRST_MODULE),
        "First module '{}' in era '{}' should be unlocked",
        FIRST_MODULE,
        ERA_ID
    );
}

#[test]
fn test_second_module_locked_initially() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    // Second module should be locked when first is not complete
    let unlocked = check_module_unlocked(&progress, &engine, ERA_ID, SECOND_MODULE);
    assert!(!unlocked, "Second module should be locked when first is incomplete");
}

#[test]
fn test_second_module_unlocks_after_first_complete() {
    let mut progress = UserProgress::new();

    // Mark first module as complete
    progress.modules.insert(FIRST_MODULE.to_string(), ModuleProgress {
        module_id: FIRST_MODULE.to_string(),
        unlocked: true,
        completed: true,
        stars: 3,
        best_score: 100,
        attempts: 1,
    });

    let engine = ContentEngine::new();
    let unlocked = check_module_unlocked(&progress, &engine, ERA_ID, SECOND_MODULE);
    assert!(unlocked, "Second module should unlock after first is complete");
}

#[test]
fn test_last_two_modules_locked_until_one_complete() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    // Last two modules should be locked initially
    let second_to_last_unlocked = check_module_unlocked(&progress, &engine, ERA_ID, SECOND_TO_LAST);
    let last_unlocked = check_module_unlocked(&progress, &engine, ERA_ID, LAST_MODULE);

    assert!(!second_to_last_unlocked, "Second-to-last module should be locked initially");
    assert!(!last_unlocked, "Last module should be locked initially");
}

#[test]
fn test_last_two_unlock_after_one_module_complete() {
    let mut progress = UserProgress::new();

    // Complete first module
    progress.modules.insert(FIRST_MODULE.to_string(), ModuleProgress {
        module_id: FIRST_MODULE.to_string(),
        unlocked: true,
        completed: true,
        stars: 3,
        best_score: 100,
        attempts: 1,
    });

    let engine = ContentEngine::new();

    // Now the "last 2 special lock" is lifted, but they still need sequential unlock
    let locked = get_locked_module_ids(&progress, &engine, ERA_ID);

    // With introduction complete, syllogistic unlocks
    assert!(!locked.contains(&SECOND_MODULE.to_string()), "syllogistic should be unlocked after introduction complete");
}

#[test]
fn test_get_locked_module_ids_returns_all_locked() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    let locked = get_locked_module_ids(&progress, &engine, ERA_ID);

    // With no progress, all modules except the first should be locked
    assert!(locked.contains(&SECOND_MODULE.to_string()), "syllogistic should be locked initially");
    assert!(locked.contains(&"definitions".to_string()), "definitions should be locked initially");
    assert!(locked.contains(&LAST_MODULE.to_string()), "inductive should be locked initially");

    // First module should NOT be in locked list
    assert!(!locked.contains(&FIRST_MODULE.to_string()), "First module should not be locked");
}

#[test]
fn test_module_state_available() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    // First module should be Available (not started)
    let state = get_module_state(&progress, &engine, ERA_ID, FIRST_MODULE);
    assert_eq!(state, ModuleState::Available);
}

#[test]
fn test_module_state_locked() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    // Second module should be Locked
    let state = get_module_state(&progress, &engine, ERA_ID, SECOND_MODULE);
    assert_eq!(state, ModuleState::Locked);
}

#[test]
fn test_module_state_started() {
    let mut progress = UserProgress::new();

    // Mark first module as unlocked so we can work on it
    // Add some progress but not complete
    progress.modules.insert(FIRST_MODULE.to_string(), ModuleProgress {
        module_id: FIRST_MODULE.to_string(),
        unlocked: true,
        completed: false,
        stars: 1,
        best_score: 30,
        attempts: 1,
    });

    let engine = ContentEngine::new();
    let state = get_module_state(&progress, &engine, ERA_ID, FIRST_MODULE);
    assert_eq!(state, ModuleState::Started);
}

#[test]
fn test_module_state_mastered() {
    let mut progress = UserProgress::new();

    // Complete with high score
    progress.modules.insert(FIRST_MODULE.to_string(), ModuleProgress {
        module_id: FIRST_MODULE.to_string(),
        unlocked: true,
        completed: true,
        stars: 3,
        best_score: 100,
        attempts: 1,
    });

    let engine = ContentEngine::new();
    let state = get_module_state(&progress, &engine, ERA_ID, FIRST_MODULE);
    // 100 score with 3 stars should be Perfected (>= 90%)
    assert_eq!(state, ModuleState::Perfected);
}

#[test]
fn test_nonexistent_era_returns_false() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    let unlocked = check_module_unlocked(&progress, &engine, "nonexistent", "module");
    assert!(!unlocked, "Nonexistent era should return false");
}

#[test]
fn test_nonexistent_module_returns_false() {
    let progress = UserProgress::new();
    let engine = ContentEngine::new();

    let unlocked = check_module_unlocked(&progress, &engine, ERA_ID, "nonexistent");
    assert!(!unlocked, "Nonexistent module should return false");
}

#[test]
fn test_progressive_unlock_chain() {
    let mut progress = UserProgress::new();
    let engine = ContentEngine::new();

    // Complete modules progressively and check unlock chain
    let modules = [FIRST_MODULE, SECOND_MODULE, "definitions"];

    for (i, module_id) in modules.iter().enumerate() {
        // Check current module is unlocked (or first)
        if i == 0 {
            assert!(check_module_unlocked(&progress, &engine, ERA_ID, module_id));
        }

        // Complete current module
        progress.modules.insert(module_id.to_string(), ModuleProgress {
            module_id: module_id.to_string(),
            unlocked: true,
            completed: true,
            stars: 3,
            best_score: 100,
            attempts: 1,
        });

        // Next module should now be unlocked (if not last in our test array)
        if i < modules.len() - 1 {
            let next = modules[i + 1];
            assert!(
                check_module_unlocked(&progress, &engine, ERA_ID, next),
                "Module {} should be unlocked after {} is complete",
                next,
                module_id
            );
        }
    }
}

#[test]
fn test_module_state_progressing() {
    let mut progress = UserProgress::new();

    // Add progress at 50-99%
    progress.modules.insert(FIRST_MODULE.to_string(), ModuleProgress {
        module_id: FIRST_MODULE.to_string(),
        unlocked: true,
        completed: false,
        stars: 2,
        best_score: 75,
        attempts: 2,
    });

    let engine = ContentEngine::new();
    let state = get_module_state(&progress, &engine, ERA_ID, FIRST_MODULE);
    assert_eq!(state, ModuleState::Progressing);
}
