//! What a learner has done, kept in the browser's local storage under `logos_user_progress`.
//!
//! Two kinds of record share the one blob: the Learn roadmap's lessons (which are done, which was
//! open last), and every graded exercise's attempts with its SM-2 schedule, which the Syntax Guide's
//! review reads too. A blob saved by an older page still loads: serde drops the fields it no longer
//! knows (the scores the Learn page once kept) and fills in the ones it lacks.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UserProgress {
    /// Every exercise attempted, by id.
    pub exercises: HashMap<String, ExerciseProgress>,
    /// The roadmap lessons finished, by lesson id.
    pub lessons: BTreeSet<String>,
    /// The lesson open when the learner left.
    pub open_lesson: Option<String>,
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

impl UserProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            crate::storage::load_raw()
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default()
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

    pub fn record_attempt(&mut self, exercise_id: &str, correct: bool) {
        let entry = self
            .exercises
            .entry(exercise_id.to_string())
            .or_insert_with(|| ExerciseProgress {
                exercise_id: exercise_id.to_string(),
                attempts: 0,
                correct_count: 0,
                last_attempt: None,
                srs: SrsData::default(),
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

    /// Record a roadmap lesson as finished.
    pub fn finish_lesson(&mut self, lesson_id: &str) {
        self.lessons.insert(lesson_id.to_string());
    }

    pub fn is_lesson_done(&self, lesson_id: &str) -> bool {
        self.lessons.contains(lesson_id)
    }
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_web/progress/tests.rs"]
mod tests;
