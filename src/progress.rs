use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserProgress {
    pub xp: u64,
    pub level: u32,
    pub streak_days: u32,
    pub last_session: Option<String>,
    pub exercises: HashMap<String, ExerciseProgress>,
    pub modules: HashMap<String, ModuleProgress>,
    #[serde(default)]
    pub combo: u32,
    #[serde(default)]
    pub best_combo: u32,
    #[serde(default)]
    pub streak_freezes: u8,
    #[serde(default)]
    pub last_streak_date: Option<String>,
    #[serde(default)]
    pub achievements: HashSet<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub last_weekly_freeze_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExerciseProgress {
    pub exercise_id: String,
    pub attempts: u32,
    pub correct_count: u32,
    pub last_attempt: Option<String>,
    pub srs: SrsData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrsData {
    pub ease_factor: f64,
    pub interval: u32,
    pub repetitions: u32,
    pub next_review: Option<String>,
}

impl Default for SrsData {
    fn default() -> Self {
        Self {
            ease_factor: 2.5,
            interval: 1,
            repetitions: 0,
            next_review: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleProgress {
    pub module_id: String,
    pub unlocked: bool,
    pub completed: bool,
    pub stars: u8,
    pub best_score: u32,
    pub attempts: u32,
}

impl Default for ModuleProgress {
    fn default() -> Self {
        Self {
            module_id: String::new(),
            unlocked: false,
            completed: false,
            stars: 0,
            best_score: 0,
            attempts: 0,
        }
    }
}

impl UserProgress {
    pub fn new() -> Self {
        Self {
            level: 1,
            ..Default::default()
        }
    }

    pub fn load() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            crate::storage::load_raw()
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_else(Self::new)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::new()
        }
    }

    pub fn save(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            if let Ok(json) = serde_json::to_string(self) {
                crate::storage::save_raw(&json);
            }
        }
    }

    pub fn add_xp(&mut self, amount: u64) {
        self.xp += amount;
        self.level = calculate_level(self.xp);
        self.save();
    }

    pub fn record_attempt(&mut self, exercise_id: &str, correct: bool) {
        let entry = self.exercises.entry(exercise_id.to_string()).or_insert_with(|| {
            ExerciseProgress {
                exercise_id: exercise_id.to_string(),
                attempts: 0,
                correct_count: 0,
                last_attempt: None,
                srs: SrsData::default(),
            }
        });

        entry.attempts += 1;
        if correct {
            entry.correct_count += 1;
        }

        self.save();
    }

    pub fn get_exercise_progress(&self, exercise_id: &str) -> Option<&ExerciseProgress> {
        self.exercises.get(exercise_id)
    }

    pub fn get_module_progress(&self, module_id: &str) -> Option<&ModuleProgress> {
        self.modules.get(module_id)
    }

    pub fn update_module_score(&mut self, module_id: &str, score: u32) {
        let entry = self.modules.entry(module_id.to_string()).or_insert_with(|| {
            ModuleProgress {
                module_id: module_id.to_string(),
                ..Default::default()
            }
        });

        entry.attempts += 1;
        if score > entry.best_score {
            entry.best_score = score;
        }

        self.save();
    }
}

pub fn calculate_level(xp: u64) -> u32 {
    ((xp as f64).sqrt() / 10.0).floor() as u32 + 1
}

pub fn xp_for_level(level: u32) -> u64 {
    let l = level as u64;
    l * l * 100
}

pub fn calculate_xp_reward(difficulty: u32, first_try: bool, streak_days: u32) -> u64 {
    let base: u64 = 10;
    let difficulty_bonus = (difficulty.saturating_sub(1) as u64) * 5;
    let first_try_bonus = if first_try { 5 } else { 0 };
    let streak_bonus = (streak_days.min(7) as u64) * 2;

    base + difficulty_bonus + first_try_bonus + streak_bonus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_calculation() {
        assert_eq!(calculate_level(0), 1);
        assert_eq!(calculate_level(100), 2);
        assert_eq!(calculate_level(400), 3);
        assert_eq!(calculate_level(900), 4);
    }

    #[test]
    fn test_xp_reward() {
        assert_eq!(calculate_xp_reward(1, false, 0), 10);
        assert_eq!(calculate_xp_reward(1, true, 0), 15);
        assert_eq!(calculate_xp_reward(2, false, 0), 15);
        assert_eq!(calculate_xp_reward(1, false, 3), 16);
        assert_eq!(calculate_xp_reward(3, true, 5), 10 + 10 + 5 + 10);
    }

    #[test]
    fn test_user_progress_record() {
        let mut progress = UserProgress::new();
        progress.record_attempt("test_q1", true);
        progress.record_attempt("test_q1", false);

        let ex = progress.get_exercise_progress("test_q1").unwrap();
        assert_eq!(ex.attempts, 2);
        assert_eq!(ex.correct_count, 1);
    }
}
