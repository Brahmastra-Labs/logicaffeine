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

/// A symbol definition for the glossary
#[derive(Debug, Clone, Deserialize)]
pub struct SymbolDef {
    pub symbol: String,
    pub name: String,
    pub meaning: String,
    #[serde(default)]
    pub example: Option<String>,
}

/// A content block within a section (paragraph, definition, example, etc.)
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Paragraph {
        text: String,
    },
    Definition {
        term: String,
        definition: String,
    },
    Example {
        title: String,
        #[serde(default)]
        premises: Vec<String>,
        #[serde(default)]
        conclusion: Option<String>,
        #[serde(default)]
        note: Option<String>,
    },
    /// Symbol glossary block - shows relevant symbols for this section
    Symbols {
        title: String,
        symbols: Vec<SymbolDef>,
    },
    /// Quiz question embedded in the lesson
    Quiz {
        question: String,
        options: Vec<String>,
        correct: usize,
        #[serde(default)]
        explanation: Option<String>,
    },
}

/// A lesson section with structured content
#[derive(Debug, Clone, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub order: u32,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub key_symbols: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub meta: ModuleMeta,
    pub exercises: Vec<ExerciseConfig>,
    pub sections: Vec<Section>,
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
        let mut sections = Vec::new();

        for file in module_dir.files() {
            if let Some(name) = file.path().file_name() {
                let name_str = name.to_string_lossy();
                if name_str.starts_with("ex_") && name_str.ends_with(".json") {
                    // Load exercise
                    if let Some(content) = file.contents_utf8() {
                        if let Ok(exercise) = serde_json::from_str::<ExerciseConfig>(content) {
                            exercises.push(exercise);
                        }
                    }
                } else if name_str.starts_with("sec_") && name_str.ends_with(".json") {
                    // Load section
                    if let Some(content) = file.contents_utf8() {
                        if let Ok(section) = serde_json::from_str::<Section>(content) {
                            sections.push(section);
                        }
                    }
                }
            }
        }

        exercises.sort_by(|a, b| a.id.cmp(&b.id));
        sections.sort_by_key(|s| s.order);
        Some(Module { meta, exercises, sections })
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
        assert!(engine.era_count() >= 4, "Should have at least 4 eras (got {})", engine.era_count());
    }

    #[test]
    fn test_first_steps_era_exists() {
        let engine = ContentEngine::new();
        let era = engine.get_era("first-steps");
        assert!(era.is_some(), "First Steps era should exist");
        assert_eq!(era.unwrap().meta.title, "First Steps");
    }

    #[test]
    fn test_building_blocks_era_exists() {
        let engine = ContentEngine::new();
        let era = engine.get_era("building-blocks");
        assert!(era.is_some(), "Building Blocks era should exist");
        assert_eq!(era.unwrap().meta.title, "Building Blocks");
    }

    #[test]
    fn test_expanding_horizons_era_exists() {
        let engine = ContentEngine::new();
        let era = engine.get_era("expanding-horizons");
        assert!(era.is_some(), "Expanding Horizons era should exist");
        assert_eq!(era.unwrap().meta.title, "Expanding Horizons");
    }

    #[test]
    fn test_mastery_era_exists() {
        let engine = ContentEngine::new();
        let era = engine.get_era("mastery");
        assert!(era.is_some(), "Mastery era should exist");
        assert_eq!(era.unwrap().meta.title, "Mastery");
    }

    #[test]
    fn test_introduction_module_exists() {
        let engine = ContentEngine::new();
        let module = engine.get_module("first-steps", "introduction");
        assert!(module.is_some(), "Introduction module should exist");
        assert_eq!(module.unwrap().meta.title, "Introduction");
    }

    #[test]
    fn test_syllogistic_module_exists() {
        let engine = ContentEngine::new();
        let module = engine.get_module("first-steps", "syllogistic");
        assert!(module.is_some(), "Syllogistic module should exist");
        let m = module.unwrap();
        assert_eq!(m.meta.title, "Syllogistic Logic");
        assert!(m.exercises.len() >= 90, "Should have at least 90 exercises (got {})", m.exercises.len());
    }

    #[test]
    fn test_propositional_module_exists() {
        let engine = ContentEngine::new();
        let module = engine.get_module("building-blocks", "propositional");
        assert!(module.is_some(), "Propositional module should exist");
        let m = module.unwrap();
        assert_eq!(m.meta.title, "Basic Propositional Logic");
        assert!(m.exercises.len() >= 100, "Should have at least 100 exercises (got {})", m.exercises.len());
    }

    #[test]
    fn test_exercises_load() {
        let engine = ContentEngine::new();
        let count = engine.exercise_count("first-steps", "syllogistic");
        assert!(count >= 90, "Syllogistic module should have at least 90 exercises");
    }

    #[test]
    fn test_exercise_has_explanation() {
        let engine = ContentEngine::new();
        let ex = engine.get_exercise("first-steps", "syllogistic", "A_1.1");
        assert!(ex.is_some(), "Exercise A_1.1 should exist");
        let exercise = ex.unwrap();
        assert!(exercise.explanation.is_some(), "Exercise should have explanation");
        assert!(exercise.options.is_some(), "Exercise should have options");
        assert_eq!(exercise.exercise_type, ExerciseType::MultipleChoice);
    }

    #[test]
    fn test_all_eras_have_modules() {
        let engine = ContentEngine::new();

        // First Steps: 5 modules
        let first_steps_modules = ["introduction", "syllogistic", "definitions", "fallacies", "inductive"];
        for module in first_steps_modules {
            assert!(engine.get_module("first-steps", module).is_some(), "first-steps/{} should exist", module);
        }

        // Building Blocks: 2 modules
        let building_blocks_modules = ["propositional", "proofs"];
        for module in building_blocks_modules {
            assert!(engine.get_module("building-blocks", module).is_some(), "building-blocks/{} should exist", module);
        }

        // Expanding Horizons: 6 modules
        let expanding_modules = ["quantificational", "relations", "modal", "further_modal", "deontic", "belief"];
        for module in expanding_modules {
            assert!(engine.get_module("expanding-horizons", module).is_some(), "expanding-horizons/{} should exist", module);
        }

        // Mastery: 5 modules
        let mastery_modules = ["ethics", "metalogic", "history", "deviant", "philosophy"];
        for module in mastery_modules {
            assert!(engine.get_module("mastery", module).is_some(), "mastery/{} should exist", module);
        }
    }
}
