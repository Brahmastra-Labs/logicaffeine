use include_dir::{include_dir, Dir};
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::LazyLock;

// Curriculum content lives under assets/curriculum/ (01_first-steps/, 02_building-blocks/, etc.)
// — scoped so images and other assets never ride into the wasm binary.
static CURRICULUM_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/curriculum");

// Parsed once per process; every render path borrows it through `ContentEngine::shared()`.
static SHARED: LazyLock<ContentEngine> = LazyLock::new(ContentEngine::new);

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
    /// The module's line on the Learn roadmap.
    pub summary: String,
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
    /// The lesson this exercise practises: the id of a section of its own module.
    #[serde(default)]
    pub section: Option<String>,
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

/// A block of a lesson's teaching. Its examples and its question have fields of their own
/// ([`Section::examples`], [`Section::question`]).
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
    /// A table of symbols or words, each with its name and meaning.
    Symbols {
        title: String,
        symbols: Vec<SymbolDef>,
    },
}

/// A section of a module: one lesson on the Learn roadmap.
#[derive(Debug, Clone, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub order: u32,
    /// The lesson's short teaching.
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    /// The lesson's examples: the first two on the main path, the rest behind "More examples".
    #[serde(default)]
    pub examples: Vec<LessonExample>,
    /// The one question that closes the lesson.
    #[serde(default)]
    pub question: Option<LessonQuestion>,
}

/// One example of a lesson.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LessonExample {
    /// An English sentence and exactly the formula LOGOS prints for it.
    Formula {
        sentence: String,
        formula: String,
        #[serde(default)]
        note: Option<String>,
    },
    /// A worked case in prose: its lines, then what follows from them.
    Worked {
        title: String,
        lines: Vec<String>,
        #[serde(default)]
        conclusion: Option<String>,
        #[serde(default)]
        note: Option<String>,
    },
}

/// The question that closes a lesson.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LessonQuestion {
    /// Write the sentence as a formula; graded against exactly what LOGOS prints for it.
    Translate {
        sentence: String,
        answer: String,
        hint: String,
        explanation: String,
    },
    /// Pick the one right option.
    Choice {
        prompt: String,
        options: Vec<String>,
        correct: usize,
        hint: String,
        explanation: String,
    },
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

/// One lesson on the Learn roadmap: a section, with the module and era it belongs to.
#[derive(Debug, Clone, Copy)]
pub struct Lesson<'a> {
    pub era: &'a Era,
    pub module: &'a Module,
    pub section: &'a Section,
}

impl ContentEngine {
    pub fn new() -> Self {
        let (curriculum, _) = Self::load_curriculum();
        Self { curriculum }
    }

    /// The process-wide curriculum, parsed on first use. Render paths borrow this instead of
    /// re-parsing every curriculum file.
    pub fn shared() -> &'static ContentEngine {
        &SHARED
    }

    /// Every curriculum file the loader skipped, with the reason. A file that is skipped
    /// disappears from the page and from every content audit, so this must stay empty.
    pub fn load_issues() -> Vec<String> {
        Self::load_curriculum().1
    }

    fn load_curriculum() -> (Curriculum, Vec<String>) {
        Self::load_from(&CURRICULUM_DIR)
    }

    fn load_from(root: &Dir) -> (Curriculum, Vec<String>) {
        let mut eras = Vec::new();
        let mut issues = Vec::new();

        for file in root.files() {
            issues.push(format!(
                "{}: stray file at the curriculum root",
                file.path().display()
            ));
        }
        for era_entry in root.dirs() {
            if let Some(era) = Self::load_era(era_entry, &mut issues) {
                eras.push(era);
            }
        }

        eras.sort_by_key(|e| e.meta.order);
        (Curriculum { eras }, issues)
    }

    fn load_era(era_dir: &Dir, issues: &mut Vec<String>) -> Option<Era> {
        let meta_path = era_dir.path().join("meta.json");
        let meta: EraMeta = parse_file(era_dir, &meta_path, issues)?;

        for file in era_dir.files() {
            if file.path() != meta_path {
                issues.push(format!(
                    "{}: stray file in an era directory",
                    file.path().display()
                ));
            }
        }

        let mut modules = Vec::new();
        for module_entry in era_dir.dirs() {
            if let Some(module) = Self::load_module(module_entry, issues) {
                modules.push(module);
            }
        }

        modules.sort_by_key(|m| m.meta.order);
        Some(Era { meta, modules })
    }

    fn load_module(module_dir: &Dir, issues: &mut Vec<String>) -> Option<Module> {
        let meta_path = module_dir.path().join("meta.json");
        let meta: ModuleMeta = parse_file(module_dir, &meta_path, issues)?;

        let mut exercises = Vec::new();
        let mut sections = Vec::new();

        for file in module_dir.files() {
            let path = file.path();
            if path == meta_path {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            if !name.ends_with(".json") {
                issues.push(format!(
                    "{}: stray file in a module directory",
                    path.display()
                ));
            } else if name.starts_with("ex_") {
                exercises.extend(parse_file::<ExerciseConfig>(module_dir, path, issues));
            } else if name.starts_with("sec_") {
                sections.extend(parse_file::<Section>(module_dir, path, issues));
            } else {
                issues.push(format!(
                    "{}: not loaded — module files are meta.json, ex_*.json or sec_*.json",
                    path.display()
                ));
            }
        }
        for dir in module_dir.dirs() {
            issues.push(format!(
                "{}: stray directory in a module directory",
                dir.path().display()
            ));
        }

        exercises.sort_by(|a, b| natural_cmp(&a.id, &b.id));
        sections.sort_by_key(|s| s.order);
        Some(Module {
            meta,
            exercises,
            sections,
        })
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

    pub fn get_exercise(
        &self,
        era_id: &str,
        module_id: &str,
        exercise_id: &str,
    ) -> Option<&ExerciseConfig> {
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

    /// Every lesson, in roadmap order: era by era, module by module.
    pub fn lessons(&self) -> Vec<Lesson<'_>> {
        self.curriculum
            .eras
            .iter()
            .flat_map(|era| era.modules.iter().map(move |module| (era, module)))
            .flat_map(|(era, module)| {
                module.sections.iter().map(move |section| Lesson {
                    era,
                    module,
                    section,
                })
            })
            .collect()
    }

    /// The lesson with this id.
    pub fn lesson(&self, id: &str) -> Option<Lesson<'_>> {
        self.lessons()
            .into_iter()
            .find(|lesson| lesson.section.id == id)
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

fn parse_file<T: serde::de::DeserializeOwned>(
    dir: &Dir,
    path: &std::path::Path,
    issues: &mut Vec<String>,
) -> Option<T> {
    let Some(file) = dir.get_file(path) else {
        issues.push(format!("{}: missing", path.display()));
        return None;
    };
    let Some(text) = file.contents_utf8() else {
        issues.push(format!("{}: not UTF-8", path.display()));
        return None;
    };
    serde_json::from_str(text)
        .map_err(|e| issues.push(format!("{}: {e}", path.display())))
        .ok()
}

/// Orders ids the way they count: runs of digits compare as numbers, so `A_1.2` precedes
/// `A_1.10` and `C_6.99` precedes `C_6.100`, and everything else compares as text, ignoring
/// case. Ids that still tie (`01` vs `1`, `a` vs `A`) fall back to plain text order so the
/// ordering stays total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.as_bytes(), b.as_bytes());
    loop {
        match (x.first(), y.first()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let (dx, rx) = split_digits(x);
                let (dy, ry) = split_digits(y);
                let (tx, ty) = (trim_zeros(dx), trim_zeros(dy));
                let by_value = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                if by_value != Ordering::Equal {
                    return by_value;
                }
                x = rx;
                y = ry;
            }
            (Some(p), Some(q)) => {
                let by_text = p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase());
                if by_text != Ordering::Equal {
                    return by_text;
                }
                x = &x[1..];
                y = &y[1..];
            }
        }
    }
}

fn split_digits(s: &[u8]) -> (&[u8], &[u8]) {
    let n = s.iter().take_while(|c| c.is_ascii_digit()).count();
    s.split_at(n)
}

fn trim_zeros(digits: &[u8]) -> &[u8] {
    let n = digits.iter().take_while(|&&c| c == b'0').count();
    &digits[n..]
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_web/content/tests.rs"]
mod tests;
