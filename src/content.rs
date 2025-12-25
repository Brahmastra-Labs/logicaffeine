use include_dir::{include_dir, Dir};
use serde::Deserialize;
use std::collections::HashMap;

static CURRICULUM_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/curriculum");

#[derive(Debug, Clone, Deserialize)]
pub struct EraMeta {
    pub id: String,
    pub title: String,
    pub description: String,
    pub order: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModuleMeta {
    pub id: String,
    pub title: String,
    pub pedagogy: String,
    pub order: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExerciseConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub exercise_type: ExerciseType,
    pub difficulty: u32,
    pub prompt: String,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub constraints: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub explanation: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub correct: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExerciseType {
    Translation,
    MultipleChoice,
    Ambiguity,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub meta: ModuleMeta,
    pub exercises: Vec<ExerciseConfig>,
}

#[derive(Debug, Clone)]
pub struct Era {
    pub meta: EraMeta,
    pub modules: Vec<Module>,
}

#[derive(Debug, Clone)]
pub struct Curriculum {
    pub eras: Vec<Era>,
}

pub struct ContentEngine {
    pub curriculum: Curriculum,
}

impl ContentEngine {
    pub fn new() -> Self {
        let curriculum = Self::load_curriculum();
        Self { curriculum }
    }

    fn load_curriculum() -> Curriculum {
        let mut eras = Vec::new();

        for era_entry in CURRICULUM_DIR.dirs() {
            if let Some(era) = Self::load_era(era_entry) {
                eras.push(era);
            }
        }

        eras.sort_by_key(|e| e.meta.order);
        Curriculum { eras }
    }

    fn load_era(era_dir: &Dir) -> Option<Era> {
        // era_dir.path() returns "01_trivium", file paths are "01_trivium/meta.json"
        let era_path = era_dir.path().to_string_lossy();
        let meta_path = format!("{}/meta.json", era_path);
        let meta_file = era_dir.get_file(&meta_path)?;
        let meta_content = meta_file.contents_utf8()?;
        let meta: EraMeta = serde_json::from_str(meta_content).ok()?;

        let mut modules = Vec::new();
        for module_entry in era_dir.dirs() {
            if let Some(module) = Self::load_module(module_entry) {
                modules.push(module);
            }
        }

        modules.sort_by_key(|m| m.meta.order);
        Some(Era { meta, modules })
    }

    fn load_module(module_dir: &Dir) -> Option<Module> {
        // module_dir.path() returns "01_trivium/01_atomic"
        let module_path = module_dir.path().to_string_lossy();
        let meta_path = format!("{}/meta.json", module_path);
        let meta_file = module_dir.get_file(&meta_path)?;
        let meta_content = meta_file.contents_utf8()?;
        let meta: ModuleMeta = serde_json::from_str(meta_content).ok()?;

        let mut exercises = Vec::new();
        for file in module_dir.files() {
            if let Some(name) = file.path().file_name() {
                let name_str = name.to_string_lossy();
                if name_str.starts_with("ex_") && name_str.ends_with(".json") {
                    if let Some(content) = file.contents_utf8() {
                        if let Ok(exercise) = serde_json::from_str::<ExerciseConfig>(content) {
                            exercises.push(exercise);
                        }
                    }
                }
            }
        }

        exercises.sort_by(|a, b| a.id.cmp(&b.id));
        Some(Module { meta, exercises })
    }

    pub fn get_era(&self, era_id: &str) -> Option<&Era> {
        self.curriculum.eras.iter().find(|e| e.meta.id == era_id)
    }

    pub fn get_module(&self, era_id: &str, module_id: &str) -> Option<&Module> {
        self.get_era(era_id)?
            .modules
            .iter()
            .find(|m| m.meta.id == module_id)
    }

    pub fn get_exercise(&self, era_id: &str, module_id: &str, exercise_id: &str) -> Option<&ExerciseConfig> {
        self.get_module(era_id, module_id)?
            .exercises
            .iter()
            .find(|e| e.id == exercise_id)
    }

    pub fn eras(&self) -> &[Era] {
        &self.curriculum.eras
    }

    pub fn era_count(&self) -> usize {
        self.curriculum.eras.len()
    }

    pub fn module_count(&self, era_id: &str) -> usize {
        self.get_era(era_id).map(|e| e.modules.len()).unwrap_or(0)
    }

    pub fn exercise_count(&self, era_id: &str, module_id: &str) -> usize {
        self.get_module(era_id, module_id)
            .map(|m| m.exercises.len())
            .unwrap_or(0)
    }
}

impl Default for ContentEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dir_contents() {
        // Debug: print what's in the embedded directory
        println!("Files in CURRICULUM_DIR:");
        for f in CURRICULUM_DIR.files() {
            println!("  file: {:?}", f.path());
        }
        println!("Dirs in CURRICULUM_DIR:");
        for d in CURRICULUM_DIR.dirs() {
            println!("  era dir: {:?}", d.path());
            for f in d.files() {
                println!("    era file: {:?}", f.path());
            }
            for module_d in d.dirs() {
                println!("    module dir: {:?}", module_d.path());
                for f in module_d.files() {
                    println!("      module file: {:?}", f.path());
                }
            }
        }
        assert!(!CURRICULUM_DIR.dirs().collect::<Vec<_>>().is_empty(), "Should have embedded directories");
    }

    #[test]
    fn test_curriculum_loads() {
        let engine = ContentEngine::new();
        assert!(engine.era_count() >= 3, "Should have at least 3 eras (got {})", engine.era_count());
    }

    #[test]
    fn test_era_trivium_exists() {
        let engine = ContentEngine::new();
        let trivium = engine.get_era("trivium");
        assert!(trivium.is_some(), "Trivium era should exist");
        assert_eq!(trivium.unwrap().meta.title, "Basics");
    }

    #[test]
    fn test_module_atomic_exists() {
        let engine = ContentEngine::new();
        let atomic = engine.get_module("trivium", "atomic");
        assert!(atomic.is_some(), "Atomic module should exist");
        assert_eq!(atomic.unwrap().meta.title, "The Atomic World");
    }

    #[test]
    fn test_exercises_load() {
        let engine = ContentEngine::new();
        let count = engine.exercise_count("trivium", "atomic");
        assert!(count >= 2, "Atomic module should have at least 2 exercises");
    }

    #[test]
    fn test_exercise_has_template() {
        let engine = ContentEngine::new();
        let ex = engine.get_exercise("trivium", "atomic", "ex_01");
        assert!(ex.is_some(), "Exercise ex_01 should exist");
        assert!(ex.unwrap().template.is_some(), "Exercise should have template");
    }

    #[test]
    fn test_logicaffeine_era_exists() {
        let engine = ContentEngine::new();
        let logicaffeine = engine.get_era("logicaffeine");
        assert!(logicaffeine.is_some(), "Logicaffeine era should exist");
        assert_eq!(logicaffeine.unwrap().meta.title, "Practice");
    }

    #[test]
    fn test_logicaffeine_syllogistic_module() {
        let engine = ContentEngine::new();
        let module = engine.get_module("logicaffeine", "syllogistic");
        assert!(module.is_some(), "Syllogistic module should exist");
        let m = module.unwrap();
        assert_eq!(m.meta.title, "The Syllogism");
        assert!(m.exercises.len() >= 90, "Should have at least 90 exercises (got {})", m.exercises.len());
    }

    #[test]
    fn test_logicaffeine_exercise_has_explanation() {
        let engine = ContentEngine::new();
        let ex = engine.get_exercise("logicaffeine", "syllogistic", "A_1.1");
        assert!(ex.is_some(), "Exercise A_1.1 should exist");
        let exercise = ex.unwrap();
        assert!(exercise.explanation.is_some(), "Exercise should have explanation");
        assert!(exercise.options.is_some(), "Exercise should have options");
        assert_eq!(exercise.exercise_type, ExerciseType::MultipleChoice);
    }
}
