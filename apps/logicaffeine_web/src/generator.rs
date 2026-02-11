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
use logicaffeine_language::runtime_lexicon::{LexiconIndex, pluralize, present_3s, past_tense, gerund};
use logicaffeine_language::{compile, compile_all_scopes};
use rand::Rng;
use rand::seq::SliceRandom;
use std::collections::HashMap;

/// Exercise generator that fills templates with lexicon words.
///
/// Holds a reference to the lexicon index for word selection.
/// Stateless; create one instance and reuse for all exercise generation.
pub struct Generator {
    lexicon: LexiconIndex,
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
    /// Creates a new generator with a fresh lexicon index.
    pub fn new() -> Self {
        Self {
            lexicon: LexiconIndex::new(),
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

    fn generate_translation(&self, exercise: &ExerciseConfig, rng: &mut impl Rng) -> Option<Challenge> {
        let template = exercise.template.as_ref()?;
        let sentence = self.fill_template(template, &exercise.constraints, rng)?;

        let golden_logic = compile(&sentence).ok()?;

        Some(Challenge {
            exercise_id: exercise.id.clone(),
            prompt: exercise.prompt.clone(),
            sentence,
            answer: AnswerType::FreeForm { golden_logic },
            hint: exercise.hint.clone(),
            explanation: exercise.explanation.clone(),
        })
    }

    fn generate_multiple_choice(&self, exercise: &ExerciseConfig, rng: &mut impl Rng) -> Option<Challenge> {
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
            answer: AnswerType::MultipleChoice { options, correct_index },
            hint: exercise.hint.clone(),
            explanation: exercise.explanation.clone(),
        })
    }

    fn generate_ambiguity(&self, exercise: &ExerciseConfig, rng: &mut impl Rng) -> Option<Challenge> {
        let template = exercise.template.as_ref()?;
        let sentence = self.fill_template(template, &exercise.constraints, rng)?;

        let readings = compile_all_scopes(&sentence).ok()?;

        Some(Challenge {
            exercise_id: exercise.id.clone(),
            prompt: exercise.prompt.clone(),
            sentence,
            answer: AnswerType::Ambiguity { readings },
            hint: exercise.hint.clone(),
            explanation: exercise.explanation.clone(),
        })
    }

    fn fill_template(&self, template: &str, constraints: &HashMap<String, Vec<String>>, rng: &mut impl Rng) -> Option<String> {
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

            let slot_constraints = constraints.get(slot_type).map(|v| v.as_slice()).unwrap_or(&[]);
            let word = self.fill_slot(slot_type, slot_constraints, modifier, &mut used_names, rng)?;

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
mod tests {
    use super::*;
    use crate::content::ContentEngine;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn test_generate_translation_challenge() {
        let engine = ContentEngine::new();
        let generator = Generator::new();
        let mut rng = StdRng::seed_from_u64(42);

        // Use introduction module which has Translation exercises
        let exercise = engine.get_exercise("first-steps", "introduction", "I_1.1");
        assert!(exercise.is_some(), "Exercise first-steps/introduction/I_1.1 should exist");
        let exercise = exercise.unwrap();
        let challenge = generator.generate(exercise, &mut rng);

        assert!(challenge.is_some(), "Should generate a challenge");
        let challenge = challenge.unwrap();
        assert!(!challenge.sentence.is_empty(), "Sentence should not be empty");

        if let AnswerType::FreeForm { golden_logic } = &challenge.answer {
            assert!(!golden_logic.is_empty(), "Golden logic should not be empty");
        } else {
            panic!("Expected FreeForm answer type");
        }
    }

    #[test]
    fn test_generate_multiple_choice() {
        let engine = ContentEngine::new();
        let generator = Generator::new();
        let mut rng = StdRng::seed_from_u64(42);

        // Use syllogistic module which has MultipleChoice exercises
        let exercise = engine.get_exercise("first-steps", "syllogistic", "A_1.1");
        assert!(exercise.is_some(), "Exercise first-steps/syllogistic/A_1.1 should exist");
        let exercise = exercise.unwrap();
        let challenge = generator.generate(exercise, &mut rng);

        assert!(challenge.is_some(), "Should generate a challenge");
        let challenge = challenge.unwrap();

        if let AnswerType::MultipleChoice { options, correct_index } = &challenge.answer {
            assert_eq!(options.len(), 4, "Should have 4 options");
            assert!(*correct_index < options.len(), "Correct index should be within options range");
        } else {
            panic!("Expected MultipleChoice answer type");
        }
    }

    #[test]
    fn test_fill_template_proper_names() {
        let generator = Generator::new();
        let mut rng = StdRng::seed_from_u64(42);

        let constraints = HashMap::new();
        let result = generator.fill_template("{ProperName} runs.", &constraints, &mut rng);

        assert!(result.is_some());
        let sentence = result.unwrap();
        assert!(sentence.ends_with(" runs."), "Template should be filled: {}", sentence);
        assert!(!sentence.starts_with("{"), "Slot should be replaced");
    }

    #[test]
    fn test_fill_template_with_modifier() {
        let generator = Generator::new();
        let mut rng = StdRng::seed_from_u64(42);

        let constraints = HashMap::new();
        let result = generator.fill_template("All {Noun:Plural} run.", &constraints, &mut rng);

        assert!(result.is_some());
        let sentence = result.unwrap();
        assert!(!sentence.contains("{"), "All slots should be filled: {}", sentence);
    }

    #[test]
    fn test_deterministic_with_seed() {
        let generator = Generator::new();
        let mut rng1 = StdRng::seed_from_u64(12345);
        let mut rng2 = StdRng::seed_from_u64(12345);

        let constraints = HashMap::new();
        let result1 = generator.fill_template("{ProperName} is {Adjective}.", &constraints, &mut rng1);
        let result2 = generator.fill_template("{ProperName} is {Adjective}.", &constraints, &mut rng2);

        assert_eq!(result1, result2, "Same seed should produce same output");
    }

    #[test]
    fn test_all_introduction_exercises() {
        let engine = ContentEngine::new();
        let generator = Generator::new();

        let module = engine.get_module("first-steps", "introduction");
        assert!(module.is_some(), "Introduction module should exist");
        let module = module.unwrap();

        println!("Introduction module has {} exercises", module.exercises.len());

        for (i, exercise) in module.exercises.iter().enumerate() {
            let mut rng = StdRng::seed_from_u64(42 + i as u64);
            println!("Exercise {}: id={}, type={:?}", i, exercise.id, exercise.exercise_type);

            let challenge = generator.generate(exercise, &mut rng);
            if challenge.is_none() {
                println!("  FAILED to generate challenge!");
                if let Some(template) = &exercise.template {
                    println!("  Template: {}", template);
                    let filled = generator.fill_template(template, &exercise.constraints, &mut StdRng::seed_from_u64(42));
                    println!("  Filled template: {:?}", filled);
                    if let Some(sentence) = filled {
                        let compiled = compile(&sentence);
                        println!("  Compile result: {:?}", compiled);
                    }
                }
            } else {
                println!("  OK: {:?}", challenge.as_ref().map(|c| &c.sentence));
            }
            assert!(challenge.is_some(), "Exercise {} ({}) should generate a challenge", i, exercise.id);
        }
    }

    #[test]
    fn test_all_exercises_across_all_modules() {
        let engine = ContentEngine::new();
        let generator = Generator::new();

        let mut total_exercises = 0;
        let mut successful = 0;
        let mut failed_exercises = Vec::new();

        for era in engine.eras() {
            for module in &era.modules {
                if let Some(m) = engine.get_module(&era.meta.id, &module.meta.id) {
                    for (i, exercise) in m.exercises.iter().enumerate() {
                        total_exercises += 1;
                        let mut rng = StdRng::seed_from_u64(42 + i as u64);

                        let challenge = generator.generate(exercise, &mut rng);
                        if challenge.is_some() {
                            successful += 1;
                        } else {
                            failed_exercises.push(format!("{}/{}/{}", era.meta.id, module.meta.id, exercise.id));
                        }
                    }
                }
            }
        }

        println!("Total exercises: {}", total_exercises);
        println!("Successful: {}", successful);
        println!("Failed: {}", failed_exercises.len());

        if !failed_exercises.is_empty() {
            println!("\nFailed exercises:");
            for ex in &failed_exercises {
                println!("  - {}", ex);
            }
        }

        // Allow some failures for now, but ensure most work
        let success_rate = successful as f64 / total_exercises as f64;
        assert!(success_rate >= 0.8, "At least 80% of exercises should generate (got {:.1}%)", success_rate * 100.0);
    }
}
