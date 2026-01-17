//! Lexicon: Vocabulary lookup functions
//!
//! This module includes the compile-time generated lexicon lookup code
//! from build.rs. It provides ~56 lookup functions for classifying words.

// Include the generated lexicon lookup functions
include!(concat!(env!("OUT_DIR"), "/lexicon_data.rs"));

// Re-export types from lexicon crate that aren't defined in generated code
// Note: Polarity, CanonicalMapping are defined in lexicon_data.rs
pub use logicaffeine_lexicon::{
    Aspect, Case, Definiteness, Feature, Gender, Number, Sort, Time, VerbClass,
    AdjectiveMetadata, MorphologicalRule, NounMetadata, VerbEntry, VerbMetadata,
};

/// Get canonical verb form and whether it's lexically negative.
/// Used at parse time to transform "lacks" → ("Have", true).
/// Returns (canonical_lemma, is_negative).
pub fn get_canonical_verb(lemma: &str) -> Option<(&'static str, bool)> {
    lookup_canonical(lemma).map(|m| (m.lemma, m.polarity == Polarity::Negative))
}

/// Lexicon trait for abstracting over static and dynamic lexicons
pub trait LexiconTrait {
    fn lookup_verb(&self, word: &str) -> Option<VerbMetadata>;
    fn lookup_noun(&self, word: &str) -> Option<NounMetadata>;
    fn lookup_adjective(&self, word: &str) -> Option<AdjectiveMetadata>;
}

/// Static lexicon implementation using compile-time generated data
pub struct StaticLexicon;

impl LexiconTrait for StaticLexicon {
    fn lookup_verb(&self, word: &str) -> Option<VerbMetadata> {
        lookup_verb_db(word)
    }

    fn lookup_noun(&self, word: &str) -> Option<NounMetadata> {
        lookup_noun_db(word)
    }

    fn lookup_adjective(&self, word: &str) -> Option<AdjectiveMetadata> {
        lookup_adjective_db(word)
    }
}

/// Lexicon struct for verb lookup with inflection handling
pub struct Lexicon {}

impl Lexicon {
    pub fn new() -> Self {
        Lexicon {}
    }

    pub fn lookup_verb(&self, word: &str) -> Option<VerbEntry> {
        let lower = word.to_lowercase();

        if let Some(entry) = lookup_irregular_verb(&lower) {
            return Some(entry);
        }

        if lower.ends_with("ing") {
            let stem = self.strip_ing(&lower);
            let lemma = Self::capitalize(&stem);
            let class = self.lookup_verb_class(&lemma.to_lowercase());
            return Some(VerbEntry {
                lemma,
                time: Time::None,
                aspect: Aspect::Progressive,
                class,
            });
        }

        if lower.ends_with("ed") {
            let stem = self.strip_ed(&lower);
            // Only treat as verb if the stem is a known base verb
            // This prevents "doomed" → "Doom" when "doom" isn't in lexicon
            if !is_base_verb(&stem) {
                return None;
            }
            let lemma = Self::capitalize(&stem);
            let class = self.lookup_verb_class(&lemma.to_lowercase());
            return Some(VerbEntry {
                lemma,
                time: Time::Past,
                aspect: Aspect::Simple,
                class,
            });
        }

        let is_third_person = if lower.ends_with("es") && lower.len() > 2 {
            true
        } else if lower.ends_with("s") && !lower.ends_with("ss") && lower.len() > 2 {
            true
        } else {
            false
        };

        if is_third_person {
            if is_stemming_exception(&lower) {
                return None;
            }

            let stem = self.strip_s(&lower);
            if !is_base_verb(&stem) {
                return None;
            }
            let lemma = Self::capitalize(&stem);
            let class = self.lookup_verb_class(&lemma.to_lowercase());
            return Some(VerbEntry {
                lemma,
                time: Time::Present,
                aspect: Aspect::Simple,
                class,
            });
        }

        // Check if this is a base verb form
        if is_base_verb(&lower) {
            let lemma = Self::capitalize(&lower);
            let class = self.lookup_verb_class(&lower);
            return Some(VerbEntry {
                lemma,
                time: Time::Present,
                aspect: Aspect::Simple,
                class,
            });
        }

        None
    }

    fn lookup_verb_class(&self, lemma: &str) -> VerbClass {
        lookup_verb_class(lemma)
    }

    fn strip_ing(&self, word: &str) -> String {
        let base = &word[..word.len() - 3];

        if base.len() >= 2 {
            let chars: Vec<char> = base.chars().collect();
            let last = chars[chars.len() - 1];
            let second_last = chars[chars.len() - 2];

            if last == second_last && !"aeiou".contains(last) {
                return base[..base.len() - 1].to_string();
            }
        }

        if needs_e_ing(base) {
            return format!("{}e", base);
        }

        base.to_string()
    }

    fn strip_ed(&self, word: &str) -> String {
        let base = &word[..word.len() - 2];

        if base.ends_with("i") {
            return format!("{}y", &base[..base.len() - 1]);
        }

        if base.len() >= 2 {
            let chars: Vec<char> = base.chars().collect();
            let last = chars[chars.len() - 1];
            let second_last = chars[chars.len() - 2];

            // Doubled consonant handling for verbs like "stopped" → "stop"
            // BUT: first check if the base WITH doubled consonant is already a verb
            // This handles words like "passed" → "pass" (natural double 's')
            if last == second_last && !"aeiou".contains(last) {
                // First try the base as-is (handles "pass", "miss", "kiss", etc.)
                if is_base_verb(base) {
                    return base.to_string();
                }
                // Otherwise strip the doubled consonant (handles "stopped" → "stop")
                return base[..base.len() - 1].to_string();
            }

            // Consonant clusters that typically come from silent-e verbs:
            // "tabled" → "tabl" needs "e", "googled" → "googl" needs "e"
            // Pattern: consonant + l/r at end, with vowel before the consonant
            if (last == 'l' || last == 'r') && !"aeiou".contains(second_last) {
                if chars.len() >= 3 && "aeiou".contains(chars[chars.len() - 3]) {
                    return format!("{}e", base);
                }
            }
        }

        if needs_e_ed(base) {
            return format!("{}e", base);
        }

        // Fallback: try adding 'e' and check if that's a valid verb
        // This handles all silent-e verbs not explicitly in needs_e_ed
        // e.g., "escaped" → "escap" → "escape" (valid verb)
        let with_e = format!("{}e", base);
        if is_base_verb(&with_e) {
            return with_e;
        }

        base.to_string()
    }

    fn strip_s(&self, word: &str) -> String {
        if word.ends_with("ies") {
            return format!("{}y", &word[..word.len() - 3]);
        }
        // For verbs ending in silent 'e': hopes → hope, decides → decide
        // These add "s" not "es", so stripping just "s" gives correct lemma
        if word.ends_with("es") {
            let base_minus_es = &word[..word.len() - 2];
            let base_minus_s = &word[..word.len() - 1];
            // If base-1 ends in 'e', probably a silent-e verb: hopes → hope
            if base_minus_s.ends_with('e') {
                return base_minus_s.to_string();
            }
            // Otherwise it's a sibilant ending: watches → watch, fixes → fix
            return base_minus_es.to_string();
        }
        word[..word.len() - 1].to_string()
    }

    fn capitalize(s: &str) -> String {
        let mut chars = s.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    }
}

impl Default for Lexicon {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of smart word analysis for derivational morphology
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordAnalysis {
    /// A dictionary entry (exact match or derived plural)
    Noun(NounMetadata),
    /// A word derived via morphological rules (agentive nouns like "blogger")
    DerivedNoun {
        lemma: String,
        number: Number,
    },
}

/// Smart word analysis with derivational morphology support.
///
/// Three-step resolution:
/// 1. **Exact Match** - Check if word exists in lexicon (handles irregulars like "mice")
/// 2. **Plural Derivation** - Strip 's'/'es' and check if stem exists (farmers → farmer)
/// 3. **Morphological Rules** - Apply suffix rules for unknown agentive nouns
pub fn analyze_word(word: &str) -> Option<WordAnalysis> {
    let lower = word.to_lowercase();

    // 1. EXACT MATCH (Fast Path)
    // Handles explicit entries like "farmer", "mice", "children"
    if let Some(meta) = lookup_noun_db(&lower) {
        return Some(WordAnalysis::Noun(meta));
    }

    // 2. PLURAL DERIVATION (Smart Path)
    // "farmers" → stem "farmer" → lookup
    if lower.ends_with('s') && lower.len() > 2 {
        // Try simple 's' stripping: "farmers" -> "farmer"
        let stem = &lower[..lower.len() - 1];
        if let Some(meta) = lookup_noun_db(stem) {
            // Found the singular base - return as plural
            return Some(WordAnalysis::Noun(NounMetadata {
                lemma: meta.lemma,
                number: Number::Plural,
                features: meta.features,
            }));
        }

        // Try 'es' stripping: "boxes" -> "box", "churches" -> "church"
        if lower.ends_with("es") && lower.len() > 3 {
            let stem_es = &lower[..lower.len() - 2];
            if let Some(meta) = lookup_noun_db(stem_es) {
                return Some(WordAnalysis::Noun(NounMetadata {
                    lemma: meta.lemma,
                    number: Number::Plural,
                    features: meta.features,
                }));
            }
        }

        // Try 'ies' -> 'y': "cities" -> "city"
        if lower.ends_with("ies") && lower.len() > 4 {
            let stem_ies = format!("{}y", &lower[..lower.len() - 3]);
            if let Some(meta) = lookup_noun_db(&stem_ies) {
                return Some(WordAnalysis::Noun(NounMetadata {
                    lemma: meta.lemma,
                    number: Number::Plural,
                    features: meta.features,
                }));
            }
        }
    }

    // 3. MORPHOLOGICAL RULES (Data-driven from lexicon.json)
    // Handle agentive nouns like "blogger", "vlogger" even if not in lexicon
    for rule in get_morphological_rules() {
        // Check plural form first (e.g., "vloggers" -> "vlogger" -> rule match)
        let (is_plural, check_word) = if lower.ends_with('s') && !rule.suffix.ends_with('s') {
            (true, &lower[..lower.len() - 1])
        } else {
            (false, lower.as_str())
        };

        if check_word.ends_with(rule.suffix) {
            return Some(WordAnalysis::DerivedNoun {
                lemma: check_word.to_string(),
                number: if is_plural { Number::Plural } else { Number::Singular },
            });
        }
    }

    None
}

/// Check if a word is a known common noun or derivable from one.
/// This is used for sentence-initial capitalization disambiguation.
pub fn is_derivable_noun(word: &str) -> bool {
    analyze_word(word).is_some()
}

/// Check if a word is a proper name (has Feature::Proper in the lexicon).
/// Used to distinguish "Socrates fears death" from "Birds fly" (bare plurals).
/// Names like "Socrates", "James", "Chris" end in 's' but aren't plural nouns.
pub fn is_proper_name(word: &str) -> bool {
    let lower = word.to_lowercase();
    if let Some(meta) = lookup_noun_db(&lower) {
        return meta.features.contains(&Feature::Proper);
    }
    false
}

/// Get the canonical lemma for a noun.
///
/// Maps inflected forms to their dictionary headword:
/// - "men" → "Man"
/// - "children" → "Child"
/// - "farmers" → "Farmer"
///
/// This is used for predicate canonicalization in the proof engine,
/// ensuring "All men are mortal" and "Socrates is a man" produce
/// matching predicates.
pub fn get_canonical_noun(word: &str) -> Option<&'static str> {
    match analyze_word(word) {
        Some(WordAnalysis::Noun(meta)) => Some(meta.lemma),
        Some(WordAnalysis::DerivedNoun { .. }) => {
            // Derived nouns (e.g., "blogger") return owned Strings,
            // so we can't return a static reference.
            // Fall back to raw word handling in the caller.
            None
        }
        _ => None,
    }
}
