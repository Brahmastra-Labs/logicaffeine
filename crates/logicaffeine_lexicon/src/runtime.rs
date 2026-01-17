//! Runtime lexicon loading for development builds.
//!
//! This module provides dynamic JSON-based lexicon loading as an alternative
//! to compile-time code generation. Enable with the `dynamic-lexicon` feature.
//!
//! # Architecture
//!
//! The runtime lexicon trades compile-time safety for faster iteration during
//! development. Instead of generating Rust code from `lexicon.json` at build time,
//! this module embeds the JSON and parses it once at runtime when `LexiconIndex::new()`
//! is called.
//!
//! # When to Use
//!
//! - **Development**: Use `dynamic-lexicon` for faster edit-compile cycles when
//!   modifying the lexicon.
//! - **Production**: Disable this feature for compile-time validation and
//!   slightly faster startup.
//!
//! # JSON Format
//!
//! The lexicon file must contain three top-level arrays:
//!
//! - `nouns`: Array of `NounEntry` objects with `lemma`, optional `forms`, `features`, and `sort`
//! - `verbs`: Array of `VerbEntry` objects with `lemma`, `class`, optional `forms`, and `features`
//! - `adjectives`: Array of `AdjectiveEntry` objects with `lemma`, `regular`, and `features`
//!
//! # Example
//!
//! ```
//! use logicaffeine_lexicon::runtime::LexiconIndex;
//!
//! let lexicon = LexiconIndex::new();
//! let proper_nouns = lexicon.proper_nouns();
//! assert!(!proper_nouns.is_empty());
//! ```
//!
//! # Type Disambiguation
//!
//! This module defines its own `VerbEntry`, `NounEntry`, and `AdjectiveEntry` types
//! for JSON deserialization. These are distinct from `crate::VerbEntry` and other types
//! in the parent `crate::types` module, which are used for compile-time generated lookups.

use rand::seq::SliceRandom;
use serde::Deserialize;
use std::collections::HashMap;

const LEXICON_JSON: &str = include_str!("../../logicaffeine_language/assets/lexicon.json");

/// Deserialized lexicon data from lexicon.json.
#[derive(Deserialize, Debug)]
pub struct LexiconData {
    /// All noun entries including proper nouns and common nouns.
    pub nouns: Vec<NounEntry>,
    /// All verb entries with Vendler class and features.
    pub verbs: Vec<VerbEntry>,
    /// All adjective entries with gradability info.
    pub adjectives: Vec<AdjectiveEntry>,
}

/// A noun entry from the lexicon database.
#[derive(Deserialize, Debug, Clone)]
pub struct NounEntry {
    /// Base form of the noun (e.g., "dog", "Mary").
    pub lemma: String,
    /// Irregular inflected forms: "plural" → "mice", etc.
    #[serde(default)]
    pub forms: HashMap<String, String>,
    /// Grammatical/semantic features: "Animate", "Proper", "Countable".
    #[serde(default)]
    pub features: Vec<String>,
    /// Semantic sort for type checking: "Human", "Physical", "Abstract".
    #[serde(default)]
    pub sort: Option<String>,
}

/// A verb entry from the lexicon database.
#[derive(Deserialize, Debug, Clone)]
pub struct VerbEntry {
    /// Base/infinitive form of the verb (e.g., "run", "give").
    pub lemma: String,
    /// Vendler Aktionsart class: "State", "Activity", "Accomplishment", "Achievement".
    pub class: String,
    /// Irregular inflected forms: "past" → "ran", "participle" → "run".
    #[serde(default)]
    pub forms: HashMap<String, String>,
    /// Grammatical/semantic features: "Transitive", "Ditransitive", "Control".
    #[serde(default)]
    pub features: Vec<String>,
}

/// An adjective entry from the lexicon database.
#[derive(Deserialize, Debug, Clone)]
pub struct AdjectiveEntry {
    /// Base/positive form of the adjective (e.g., "tall", "happy").
    pub lemma: String,
    /// Whether comparative/superlative follow regular -er/-est pattern.
    #[serde(default)]
    pub regular: bool,
    /// Semantic features: "Gradable", "Subsective", "NonIntersective".
    #[serde(default)]
    pub features: Vec<String>,
}

/// Index for querying the lexicon by features, sorts, and classes.
pub struct LexiconIndex {
    data: LexiconData,
}

impl LexiconIndex {
    /// Load and parse the lexicon from the embedded JSON file.
    pub fn new() -> Self {
        let data: LexiconData = serde_json::from_str(LEXICON_JSON)
            .expect("Failed to parse lexicon.json");
        Self { data }
    }

    /// Get all nouns marked with the "Proper" feature (names).
    pub fn proper_nouns(&self) -> Vec<&NounEntry> {
        self.data.nouns.iter()
            .filter(|n| n.features.iter().any(|f| f == "Proper"))
            .collect()
    }

    /// Get all nouns NOT marked as proper (common nouns).
    pub fn common_nouns(&self) -> Vec<&NounEntry> {
        self.data.nouns.iter()
            .filter(|n| !n.features.iter().any(|f| f == "Proper"))
            .collect()
    }

    /// Get all nouns with a specific feature (case-insensitive).
    pub fn nouns_with_feature(&self, feature: &str) -> Vec<&NounEntry> {
        self.data.nouns.iter()
            .filter(|n| n.features.iter().any(|f| f.eq_ignore_ascii_case(feature)))
            .collect()
    }

    /// Get all nouns with a specific semantic sort (case-insensitive).
    pub fn nouns_with_sort(&self, sort: &str) -> Vec<&NounEntry> {
        self.data.nouns.iter()
            .filter(|n| n.sort.as_ref().map(|s| s.eq_ignore_ascii_case(sort)).unwrap_or(false))
            .collect()
    }

    /// Get all verbs with a specific feature (case-insensitive).
    pub fn verbs_with_feature(&self, feature: &str) -> Vec<&VerbEntry> {
        self.data.verbs.iter()
            .filter(|v| v.features.iter().any(|f| f.eq_ignore_ascii_case(feature)))
            .collect()
    }

    /// Get all verbs with a specific Vendler class (case-insensitive).
    pub fn verbs_with_class(&self, class: &str) -> Vec<&VerbEntry> {
        self.data.verbs.iter()
            .filter(|v| v.class.eq_ignore_ascii_case(class))
            .collect()
    }

    /// Get all verbs that are intransitive (no Transitive/Ditransitive feature).
    pub fn intransitive_verbs(&self) -> Vec<&VerbEntry> {
        self.data.verbs.iter()
            .filter(|v| {
                !v.features.iter().any(|f|
                    f.eq_ignore_ascii_case("Transitive") ||
                    f.eq_ignore_ascii_case("Ditransitive")
                )
            })
            .collect()
    }

    /// Returns all verbs that take a direct object.
    ///
    /// Includes both transitive verbs (two-place predicates) and ditransitive verbs
    /// (three-place predicates). Verbs are matched if they have either the `"Transitive"`
    /// or `"Ditransitive"` feature (case-insensitive).
    pub fn transitive_verbs(&self) -> Vec<&VerbEntry> {
        self.data.verbs.iter()
            .filter(|v| {
                v.features.iter().any(|f| f.eq_ignore_ascii_case("Transitive")) ||
                v.features.iter().any(|f| f.eq_ignore_ascii_case("Ditransitive"))
            })
            .collect()
    }

    /// Returns all adjectives with a specific feature (case-insensitive).
    ///
    /// Common features include `"Intersective"`, `"Subsective"`, `"NonIntersective"`,
    /// and `"Gradable"`. See [`crate::Feature`] for the full list.
    pub fn adjectives_with_feature(&self, feature: &str) -> Vec<&AdjectiveEntry> {
        self.data.adjectives.iter()
            .filter(|a| a.features.iter().any(|f| f.eq_ignore_ascii_case(feature)))
            .collect()
    }

    /// Returns all adjectives with intersective semantics.
    ///
    /// Intersective adjectives combine with nouns via set intersection:
    /// "red ball" denotes things that are both red and balls. This is a convenience
    /// method equivalent to `adjectives_with_feature("Intersective")`.
    pub fn intersective_adjectives(&self) -> Vec<&AdjectiveEntry> {
        self.adjectives_with_feature("Intersective")
    }

    /// Selects a random proper noun from the lexicon.
    ///
    /// Returns `None` if the lexicon contains no proper nouns.
    pub fn random_proper_noun(&self, rng: &mut impl rand::Rng) -> Option<&NounEntry> {
        self.proper_nouns().choose(rng).copied()
    }

    /// Selects a random common noun from the lexicon.
    ///
    /// Returns `None` if the lexicon contains no common nouns.
    pub fn random_common_noun(&self, rng: &mut impl rand::Rng) -> Option<&NounEntry> {
        self.common_nouns().choose(rng).copied()
    }

    /// Selects a random verb from the lexicon.
    ///
    /// Returns `None` if the lexicon contains no verbs.
    pub fn random_verb(&self, rng: &mut impl rand::Rng) -> Option<&VerbEntry> {
        self.data.verbs.choose(rng)
    }

    /// Selects a random intransitive verb from the lexicon.
    ///
    /// Returns `None` if the lexicon contains no intransitive verbs.
    pub fn random_intransitive_verb(&self, rng: &mut impl rand::Rng) -> Option<&VerbEntry> {
        self.intransitive_verbs().choose(rng).copied()
    }

    /// Selects a random transitive or ditransitive verb from the lexicon.
    ///
    /// Returns `None` if the lexicon contains no transitive verbs.
    pub fn random_transitive_verb(&self, rng: &mut impl rand::Rng) -> Option<&VerbEntry> {
        self.transitive_verbs().choose(rng).copied()
    }

    /// Selects a random adjective from the lexicon.
    ///
    /// Returns `None` if the lexicon contains no adjectives.
    pub fn random_adjective(&self, rng: &mut impl rand::Rng) -> Option<&AdjectiveEntry> {
        self.data.adjectives.choose(rng)
    }

    /// Selects a random intersective adjective from the lexicon.
    ///
    /// Returns `None` if the lexicon contains no intersective adjectives.
    pub fn random_intersective_adjective(&self, rng: &mut impl rand::Rng) -> Option<&AdjectiveEntry> {
        self.intersective_adjectives().choose(rng).copied()
    }
}

/// Creates a [`LexiconIndex`] by loading and parsing the embedded lexicon JSON.
///
/// Equivalent to calling [`LexiconIndex::new()`].
impl Default for LexiconIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes the plural form of a noun.
///
/// Returns the irregular plural if one is defined in the noun's `forms` map under
/// the `"plural"` key. Otherwise, applies English pluralization rules:
///
/// - Sibilants (`-s`, `-x`, `-ch`, `-sh`) → append `-es` ("box" → "boxes")
/// - Consonant + `y` → replace `y` with `-ies` ("city" → "cities")
/// - Vowel + `y` (`-ay`, `-ey`, `-oy`, `-uy`) → append `-s` ("day" → "days")
/// - Default → append `-s` ("dog" → "dogs")
///
/// # Arguments
///
/// * `noun` - The noun entry containing the lemma and optional irregular forms.
///
/// # Examples
///
/// ```
/// use logicaffeine_lexicon::runtime::{NounEntry, pluralize};
/// use std::collections::HashMap;
///
/// // Regular noun
/// let dog = NounEntry {
///     lemma: "dog".to_string(),
///     forms: HashMap::new(),
///     features: vec![],
///     sort: None,
/// };
/// assert_eq!(pluralize(&dog), "dogs");
///
/// // Irregular noun
/// let mouse = NounEntry {
///     lemma: "mouse".to_string(),
///     forms: [("plural".to_string(), "mice".to_string())].into(),
///     features: vec![],
///     sort: None,
/// };
/// assert_eq!(pluralize(&mouse), "mice");
/// ```
pub fn pluralize(noun: &NounEntry) -> String {
    if let Some(plural) = noun.forms.get("plural") {
        plural.clone()
    } else {
        let lemma = noun.lemma.to_lowercase();
        if lemma.ends_with('s') || lemma.ends_with('x') ||
           lemma.ends_with("ch") || lemma.ends_with("sh") {
            format!("{}es", lemma)
        } else if lemma.ends_with('y') && !lemma.ends_with("ay") &&
                  !lemma.ends_with("ey") && !lemma.ends_with("oy") && !lemma.ends_with("uy") {
            format!("{}ies", &lemma[..lemma.len()-1])
        } else {
            format!("{}s", lemma)
        }
    }
}

/// Computes the third-person singular present tense form of a verb.
///
/// Returns the irregular form if one is defined in the verb's `forms` map under
/// the `"present3s"` key. Otherwise, applies English conjugation rules:
///
/// - Sibilants and `-o` (`-s`, `-x`, `-ch`, `-sh`, `-o`) → append `-es` ("go" → "goes")
/// - Consonant + `y` → replace `y` with `-ies` ("fly" → "flies")
/// - Vowel + `y` (`-ay`, `-ey`, `-oy`, `-uy`) → append `-s` ("play" → "plays")
/// - Default → append `-s` ("run" → "runs")
///
/// # Arguments
///
/// * `verb` - The verb entry containing the lemma and optional irregular forms.
///
/// # Examples
///
/// ```
/// use logicaffeine_lexicon::runtime::{VerbEntry, present_3s};
/// use std::collections::HashMap;
///
/// let run = VerbEntry {
///     lemma: "run".to_string(),
///     class: "Activity".to_string(),
///     forms: HashMap::new(),
///     features: vec![],
/// };
/// assert_eq!(present_3s(&run), "runs");
///
/// let go = VerbEntry {
///     lemma: "go".to_string(),
///     class: "Activity".to_string(),
///     forms: [("present3s".to_string(), "goes".to_string())].into(),
///     features: vec![],
/// };
/// assert_eq!(present_3s(&go), "goes");
/// ```
pub fn present_3s(verb: &VerbEntry) -> String {
    if let Some(form) = verb.forms.get("present3s") {
        form.clone()
    } else {
        let lemma = verb.lemma.to_lowercase();
        if lemma.ends_with('s') || lemma.ends_with('x') ||
           lemma.ends_with("ch") || lemma.ends_with("sh") || lemma.ends_with('o') {
            format!("{}es", lemma)
        } else if lemma.ends_with('y') && !lemma.ends_with("ay") &&
                  !lemma.ends_with("ey") && !lemma.ends_with("oy") && !lemma.ends_with("uy") {
            format!("{}ies", &lemma[..lemma.len()-1])
        } else {
            format!("{}s", lemma)
        }
    }
}

/// Computes the past tense form of a verb.
///
/// Returns the irregular form if one is defined in the verb's `forms` map under
/// the `"past"` key. Otherwise, applies English past tense rules:
///
/// - Ends in `-e` → append `-d` ("love" → "loved")
/// - Consonant + `y` → replace `y` with `-ied` ("carry" → "carried")
/// - Vowel + `y` (`-ay`, `-ey`, `-oy`, `-uy`) → append `-ed` ("play" → "played")
/// - Default → append `-ed` ("walk" → "walked")
///
/// # Arguments
///
/// * `verb` - The verb entry containing the lemma and optional irregular forms.
///
/// # Examples
///
/// ```
/// use logicaffeine_lexicon::runtime::{VerbEntry, past_tense};
/// use std::collections::HashMap;
///
/// let walk = VerbEntry {
///     lemma: "walk".to_string(),
///     class: "Activity".to_string(),
///     forms: HashMap::new(),
///     features: vec![],
/// };
/// assert_eq!(past_tense(&walk), "walked");
///
/// let run = VerbEntry {
///     lemma: "run".to_string(),
///     class: "Activity".to_string(),
///     forms: [("past".to_string(), "ran".to_string())].into(),
///     features: vec![],
/// };
/// assert_eq!(past_tense(&run), "ran");
/// ```
pub fn past_tense(verb: &VerbEntry) -> String {
    if let Some(form) = verb.forms.get("past") {
        form.clone()
    } else {
        let lemma = verb.lemma.to_lowercase();
        if lemma.ends_with('e') {
            format!("{}d", lemma)
        } else if lemma.ends_with('y') && !lemma.ends_with("ay") &&
                  !lemma.ends_with("ey") && !lemma.ends_with("oy") && !lemma.ends_with("uy") {
            format!("{}ied", &lemma[..lemma.len()-1])
        } else {
            format!("{}ed", lemma)
        }
    }
}

/// Computes the gerund (present participle) form of a verb.
///
/// Returns the irregular form if one is defined in the verb's `forms` map under
/// the `"gerund"` key. Otherwise, applies English gerund formation rules:
///
/// - Ends in `-e` (but not `-ee`) → drop `e` and append `-ing` ("make" → "making")
/// - Ends in `-ee` → append `-ing` without dropping ("see" → "seeing")
/// - Default → append `-ing` ("run" → "running")
///
/// Note: This implementation does not handle consonant doubling (e.g., "run" → "running"
/// should double the 'n', but this produces "runing"). For accurate results with such
/// verbs, provide an irregular form in the `forms` map.
///
/// # Arguments
///
/// * `verb` - The verb entry containing the lemma and optional irregular forms.
///
/// # Examples
///
/// ```
/// use logicaffeine_lexicon::runtime::{VerbEntry, gerund};
/// use std::collections::HashMap;
///
/// let make = VerbEntry {
///     lemma: "make".to_string(),
///     class: "Activity".to_string(),
///     forms: HashMap::new(),
///     features: vec![],
/// };
/// assert_eq!(gerund(&make), "making");
///
/// let see = VerbEntry {
///     lemma: "see".to_string(),
///     class: "Activity".to_string(),
///     forms: HashMap::new(),
///     features: vec![],
/// };
/// assert_eq!(gerund(&see), "seeing");
/// ```
pub fn gerund(verb: &VerbEntry) -> String {
    if let Some(form) = verb.forms.get("gerund") {
        form.clone()
    } else {
        let lemma = verb.lemma.to_lowercase();
        if lemma.ends_with('e') && !lemma.ends_with("ee") {
            format!("{}ing", &lemma[..lemma.len()-1])
        } else {
            format!("{}ing", lemma)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexicon_loads() {
        let index = LexiconIndex::new();
        assert!(!index.proper_nouns().is_empty());
        assert!(!index.common_nouns().is_empty());
        assert!(!index.intersective_adjectives().is_empty());
    }

    #[test]
    fn test_proper_nouns() {
        let index = LexiconIndex::new();
        let proper = index.proper_nouns();
        assert!(proper.iter().any(|n| n.lemma == "John"));
        assert!(proper.iter().any(|n| n.lemma == "Mary"));
    }

    #[test]
    fn test_intersective_adjectives() {
        let index = LexiconIndex::new();
        let adj = index.intersective_adjectives();
        assert!(adj.iter().any(|a| a.lemma == "Happy"));
        assert!(adj.iter().any(|a| a.lemma == "Red"));
    }

    #[test]
    fn test_pluralize() {
        let noun = NounEntry {
            lemma: "Dog".to_string(),
            forms: HashMap::new(),
            features: vec![],
            sort: None,
        };
        assert_eq!(pluralize(&noun), "dogs");

        let noun_irregular = NounEntry {
            lemma: "Man".to_string(),
            forms: [("plural".to_string(), "men".to_string())].into(),
            features: vec![],
            sort: None,
        };
        assert_eq!(pluralize(&noun_irregular), "men");
    }

    #[test]
    fn test_present_3s() {
        let verb = VerbEntry {
            lemma: "Run".to_string(),
            class: "Activity".to_string(),
            forms: HashMap::new(),
            features: vec![],
        };
        assert_eq!(present_3s(&verb), "runs");
    }
}
