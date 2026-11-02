//! Exercise generation from templates.
//!
//! Transforms exercise configurations from the curriculum into concrete
//! challenges by filling template slots with words from the lexicon.
//!
//! # Template Syntax
//!
//! Templates use curly braces for slots: `{SlotType}` or `{SlotType:Modifier}`.
//!
//! ## Slot Types
//!
//! | Slot | Description | Example Output |
//! |------|-------------|----------------|
//! | `{ProperName}` | Random proper noun | "Alice", "Bob" |
//! | `{Noun}` | Random common noun (singular) | "cat", "dog" |
//! | `{Noun:Plural}` | Pluralized common noun | "cats", "dogs" |
//! | `{Verb}` | Base form verb | "run", "eat" |
//! | `{Verb:Past}` | Past tense | "ran", "ate" |
//! | `{Verb:Present3s}` | Third person singular | "runs", "eats" |
//! | `{Verb:Gerund}` | Present participle | "running", "eating" |
//! | `{Adjective}` | Intersective adjective | "tall", "red" |
//!
//! ## Constraints
//!
//! Constraints filter word selection by semantic features. They're specified
//! in the exercise config's `constraints` map:
//!
//! ```json
//! {
//!   "Verb": ["Intransitive"],
//!   "Noun": ["Animate"]
//! }
//! ```
//!
//! ## Example
//!
//! Template: `"{ProperName} {Verb:Present3s}."`
//! With constraint `Verb: ["Intransitive"]`
//! Output: `"Alice runs."`
//!
//! # Usage
//!
//! ```no_run
//! use logicaffeine_web::generator::Generator;
//! use logicaffeine_web::content::ExerciseConfig;
//! use rand::SeedableRng;
//!
//! let generator = Generator::new();
//! let mut rng = rand::rngs::StdRng::seed_from_u64(42);
//! # let exercise_config: ExerciseConfig = serde_json::from_str(r#"{"id":"test","type":"Translation","difficulty":1,"prompt":"test"}"#).unwrap();
//!
//! if let Some(challenge) = generator.generate(&exercise_config, &mut rng) {
//!     println!("Sentence: {}", challenge.sentence);
//! }
//! ```

use crate::content::{ExerciseConfig, ExerciseType};
use logicaffeine_language::runtime_lexicon::{
    gerund, past_tense, pluralize, present_3s, LexiconIndex,
};
use logicaffeine_language::{compile, compile_all_scopes};
use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::LazyLock;
#[cfg(target_arch = "wasm32")]
use std::sync::OnceLock;

// The process-wide lexicon, parsed exactly once. Native builds parse the embedded
// JSON on first use; wasm pins the copy fetched from /data/lexicon.json (staged by
// scripts/web/stage-web-data.sh) so the 279 KB document never rides in the binary.
#[cfg(not(target_arch = "wasm32"))]
static LEXICON: LazyLock<LexiconIndex> = LazyLock::new(LexiconIndex::new);
#[cfg(target_arch = "wasm32")]
static LEXICON: OnceLock<LexiconIndex> = OnceLock::new();

/// The pinned lexicon. `None` only on wasm before [`ensure_lexicon`] resolves —
/// the `LexiconGate` component holds Learn content back until it has.
pub fn lexicon() -> Option<&'static LexiconIndex> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&LEXICON)
    }
    #[cfg(target_arch = "wasm32")]
    {
        LEXICON.get()
    }
}

/// Resolves once the lexicon is pinned — instant on native and on repeat calls.
pub async fn ensure_lexicon() -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    if LEXICON.get().is_none() {
        let text = crate::ui::data_fetch::fetch_static_text("/data/lexicon.json").await?;
        let index = LexiconIndex::from_json(&text)
            .map_err(|e| format!("parsing /data/lexicon.json: {e}"))?;
        let _ = LEXICON.set(index);
    }
    Ok(())
}

/// Exercise generator that fills templates with lexicon words.
///
/// Holds a reference to the lexicon index for word selection.
/// Stateless; create one instance and reuse for all exercise generation.
pub struct Generator {
    lexicon: &'static LexiconIndex,
}

/// A generated exercise instance ready for display and grading.
///
/// Contains the filled template, expected answer(s), and optional hints.
#[derive(Debug, Clone)]
pub struct Challenge {
    /// Unique identifier linking back to the exercise config.
    pub exercise_id: String,
    /// Instructions shown to the user (e.g., "Translate this sentence").
    pub prompt: String,
    /// The filled template sentence for the user to translate.
    pub sentence: String,
    /// The expected answer format and content.
    pub answer: AnswerType,
    /// Optional hint revealed on request.
    pub hint: Option<String>,
    /// Detailed explanation shown after answering.
    pub explanation: Option<String>,
}

/// Expected answer format for a challenge.
#[derive(Debug, Clone)]
pub enum AnswerType {
    /// User enters free-form FOL expression, compared against golden answer.
    FreeForm {
        /// The correct FOL translation, pre-compiled from the sentence.
        golden_logic: String,
    },
    /// User selects from multiple predefined options.
    MultipleChoice {
        /// List of answer choices to display.
        options: Vec<String>,
        /// Zero-based index of the correct option.
        correct_index: usize,
    },
    /// Sentence has multiple valid readings; user must provide all.
    Ambiguity {
        /// All valid FOL translations for the ambiguous sentence.
        readings: Vec<String>,
    },
}

impl Generator {
    /// How many sentences a generator may draw before giving up on an
    /// exercise. Lexicon pools contain words the compiler rejects in some
    /// frames, so generation rejection-samples; the bound keeps a
    /// misconfigured template from looping forever.
    const MAX_DRAWS: usize = 32;

    /// Creates a new generator over the pinned lexicon index.
    pub fn new() -> Self {
        Self {
            lexicon: lexicon()
                .expect("gated: LexiconGate resolves the lexicon before Learn content renders"),
        }
    }

    /// Generates a challenge from an exercise configuration.
    ///
    /// Returns `None` if:
    /// - The template cannot be filled (missing word categories)
    /// - The generated sentence fails to compile (invalid grammar)
    /// - Required fields are missing from the exercise config
    pub fn generate(&self, exercise: &ExerciseConfig, rng: &mut impl Rng) -> Option<Challenge> {
        match exercise.exercise_type {
            ExerciseType::Translation => self.generate_translation(exercise, rng),
            ExerciseType::MultipleChoice => self.generate_multiple_choice(exercise, rng),
            ExerciseType::Ambiguity => self.generate_ambiguity(exercise, rng),
        }
    }

    fn generate_translation(
        &self,
        exercise: &ExerciseConfig,
        rng: &mut impl Rng,
    ) -> Option<Challenge> {
        let template = exercise.template.as_ref()?;

        // Rejection sampling: a draw whose sentence the compiler rejects is
        // discarded and redrawn, so one unparseable word in a pool can never
        // sink the exercise.
        for _ in 0..Self::MAX_DRAWS {
            let Some(sentence) = self.fill_template(template, &exercise.constraints, rng) else {
                return None;
            };
            let Ok(golden_logic) = compile(&sentence) else {
                continue;
            };

            return Some(Challenge {
                exercise_id: exercise.id.clone(),
                prompt: exercise.prompt.clone(),
                sentence,
                answer: AnswerType::FreeForm { golden_logic },
                hint: exercise.hint.clone(),
                explanation: exercise.explanation.clone(),
            });
        }
        None
    }

    fn generate_multiple_choice(
        &self,
        exercise: &ExerciseConfig,
        rng: &mut impl Rng,
    ) -> Option<Challenge> {
        let options = exercise.options.clone()?;
        let correct_index = exercise.correct?;

        let sentence = if let Some(template) = &exercise.template {
            self.fill_template(template, &exercise.constraints, rng)?
        } else {
            exercise.prompt.clone()
        };

        Some(Challenge {
            exercise_id: exercise.id.clone(),
            prompt: exercise.prompt.clone(),
            sentence,
            answer: AnswerType::MultipleChoice {
                options,
                correct_index,
            },
            hint: exercise.hint.clone(),
            explanation: exercise.explanation.clone(),
        })
    }

    fn generate_ambiguity(
        &self,
        exercise: &ExerciseConfig,
        rng: &mut impl Rng,
    ) -> Option<Challenge> {
        let template = exercise.template.as_ref()?;

        for _ in 0..Self::MAX_DRAWS {
            let Some(sentence) = self.fill_template(template, &exercise.constraints, rng) else {
                return None;
            };
            let Ok(readings) = compile_all_scopes(&sentence) else {
                continue;
            };

            return Some(Challenge {
                exercise_id: exercise.id.clone(),
                prompt: exercise.prompt.clone(),
                sentence,
                answer: AnswerType::Ambiguity { readings },
                hint: exercise.hint.clone(),
                explanation: exercise.explanation.clone(),
            });
        }
        None
    }

    fn fill_template(
        &self,
        template: &str,
        constraints: &HashMap<String, Vec<String>>,
        rng: &mut impl Rng,
    ) -> Option<String> {
        let mut result = template.to_string();
        let mut used_names: HashMap<String, String> = HashMap::new();

        while let Some(start) = result.find('{') {
            let end = result[start..].find('}')? + start;
            let slot = &result[start + 1..end];

            let (slot_type, modifier) = if let Some(colon_pos) = slot.find(':') {
                (&slot[..colon_pos], Some(&slot[colon_pos + 1..]))
            } else {
                (slot, None)
            };

            let slot_constraints = constraints
                .get(slot_type)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let word =
                self.fill_slot(slot_type, slot_constraints, modifier, &mut used_names, rng)?;

            result = format!("{}{}{}", &result[..start], word, &result[end + 1..]);
        }

        Some(result)
    }

    fn fill_slot(
        &self,
        slot_type: &str,
        constraints: &[String],
        modifier: Option<&str>,
        used_names: &mut HashMap<String, String>,
        rng: &mut impl Rng,
    ) -> Option<String> {
        match slot_type {
            "ProperName" => {
                let key = format!("ProperName_{}", used_names.len());
                if let Some(existing) = used_names.get(&key) {
                    return Some(existing.clone());
                }

                let proper_nouns = self.lexicon.proper_nouns();
                let available: Vec<_> = proper_nouns
                    .iter()
                    .filter(|n| !used_names.values().any(|v| v == &n.lemma))
                    .copied()
                    .collect();

                let entry = if !available.is_empty() {
                    available.choose(rng)?
                } else {
                    proper_nouns.choose(rng)?
                };
                let name = entry.lemma.clone();
                used_names.insert(key, name.clone());
                Some(name)
            }
            "Noun" => {
                let nouns = if constraints.is_empty() {
                    self.lexicon.common_nouns()
                } else {
                    let mut filtered = Vec::new();
                    for constraint in constraints {
                        filtered.extend(self.lexicon.nouns_with_feature(constraint));
                    }
                    filtered
                };

                let entry = nouns.choose(rng)?;
                let word = entry.lemma.to_lowercase();

                match modifier {
                    Some("Plural") => Some(pluralize(entry)),
                    _ => Some(word),
                }
            }
            "Verb" => {
                let verbs = if constraints.contains(&"Intransitive".to_string()) {
                    self.lexicon.intransitive_verbs()
                } else if constraints.contains(&"Transitive".to_string()) {
                    self.lexicon.transitive_verbs()
                } else {
                    let mut result = Vec::new();
                    for constraint in constraints {
                        result.extend(self.lexicon.verbs_with_feature(constraint));
                    }
                    if result.is_empty() {
                        self.lexicon.intransitive_verbs()
                    } else {
                        result
                    }
                };

                let entry = verbs.choose(rng)?;

                match modifier {
                    Some("Past") => Some(past_tense(entry)),
                    Some("Gerund") => Some(gerund(entry)),
                    Some("Present3s") => Some(present_3s(entry)),
                    _ => Some(entry.lemma.to_lowercase()),
                }
            }
            "Adjective" => {
                let adjectives = if constraints.contains(&"Intersective".to_string()) {
                    self.lexicon.intersective_adjectives()
                } else if constraints.is_empty() {
                    self.lexicon.intersective_adjectives()
                } else {
                    let mut result = Vec::new();
                    for constraint in constraints {
                        result.extend(self.lexicon.adjectives_with_feature(constraint));
                    }
                    result
                };

                let entry = adjectives.choose(rng)?;
                Some(entry.lemma.to_lowercase())
            }
            _ => Some("thing".to_string()),
        }
    }
}

impl Default for Generator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_web/generator/tests.rs"]
mod tests;
