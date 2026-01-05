include!(concat!(env!("OUT_DIR"), "/lexicon_data.rs"));

/// Get canonical verb form and whether it's lexically negative.
/// Used at parse time to transform "lacks" → ("Have", true).
/// Returns (canonical_lemma, is_negative).
pub fn get_canonical_verb(lemma: &str) -> Option<(&'static str, bool)> {
    lookup_canonical(lemma).map(|m| (m.lemma, m.polarity == Polarity::Negative))
}

/// Feature-based lexical properties
/// Words can have multiple overlapping features
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    // Verb Transitivity
    Transitive,
    Intransitive,
    Ditransitive,

    // Control Theory
    SubjectControl, // "I want to run"
    ObjectControl,  // "I persuaded him to run"
    Raising,        // "He seems to run"

    // Semantics
    Opaque,      // "I seek a unicorn" (De Dicto/De Re ambiguity)
    Factive,     // "I know that..." (Presupposes truth)
    Performative, // "I promise"
    Collective,  // "The group gathered"
    Mixed,       // "Lift" - can be collective or distributive
    Weather,     // "Rain", "Snow" - weather verbs with expletive "it"
    Unaccusative, // "The door opens" - intransitive subject is Theme, not Agent

    // Noun Features
    Count,
    Mass,
    Proper, // Proper Name

    // Gender
    Masculine,
    Feminine,
    Neuter,

    // Animacy
    Animate,
    Inanimate,

    // Adjective Features
    Intersective,    // "Red ball" -> Red(x) AND Ball(x)
    NonIntersective, // "Fake gun" -> Fake(Gun)
    Subsective,      // "Small elephant" -> Small(x, ^Elephant)
    Gradable,        // "Tall", "Taller"
    EventModifier,   // "Beautiful dancer" -> can modify dancing event
}

impl Feature {
    pub fn from_str(s: &str) -> Option<Feature> {
        match s {
            "Transitive" => Some(Feature::Transitive),
            "Intransitive" => Some(Feature::Intransitive),
            "Ditransitive" => Some(Feature::Ditransitive),
            "SubjectControl" => Some(Feature::SubjectControl),
            "ObjectControl" => Some(Feature::ObjectControl),
            "Raising" => Some(Feature::Raising),
            "Opaque" => Some(Feature::Opaque),
            "Factive" => Some(Feature::Factive),
            "Performative" => Some(Feature::Performative),
            "Collective" => Some(Feature::Collective),
            "Weather" => Some(Feature::Weather),
            "Unaccusative" => Some(Feature::Unaccusative),
            "Count" => Some(Feature::Count),
            "Mass" => Some(Feature::Mass),
            "Proper" => Some(Feature::Proper),
            "Masculine" => Some(Feature::Masculine),
            "Feminine" => Some(Feature::Feminine),
            "Neuter" => Some(Feature::Neuter),
            "Animate" => Some(Feature::Animate),
            "Inanimate" => Some(Feature::Inanimate),
            "Intersective" => Some(Feature::Intersective),
            "NonIntersective" => Some(Feature::NonIntersective),
            "Subsective" => Some(Feature::Subsective),
            "Gradable" => Some(Feature::Gradable),
            "EventModifier" => Some(Feature::EventModifier),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sort {
    Entity,
    Physical,
    Animate,
    Human,
    Plant,
    Place,
    Time,
    Abstract,
    Information,
    Event,
    Celestial,
    Value,
    Group,
}

impl Sort {
    pub fn is_compatible_with(self, other: Sort) -> bool {
        if self == other {
            return true;
        }
        match (self, other) {
            (Sort::Human, Sort::Animate) => true,
            (Sort::Plant, Sort::Animate) => true,
            (Sort::Animate, Sort::Physical) => true,
            (Sort::Human, Sort::Physical) => true,
            (Sort::Plant, Sort::Physical) => true,
            (_, Sort::Entity) => true,
            _ => false,
        }
    }
}

/// Vendler's Lexical Aspect Classes (Aktionsart)
///
/// Classification based on three binary features:
/// - Static: Does the predicate involve change?
/// - Durative: Does the predicate extend over time?
/// - Telic: Does the predicate have a natural endpoint?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum VerbClass {
    /// +static, +durative, -telic: know, love, exist
    State,
    /// -static, +durative, -telic: run, swim, drive
    #[default]
    Activity,
    /// -static, +durative, +telic: build, draw, write
    Accomplishment,
    /// -static, -durative, +telic: win, find, die
    Achievement,
    /// -static, -durative, -telic: knock, cough, blink
    Semelfactive,
}

impl VerbClass {
    /// Returns true if this verb class is stative (+static)
    pub fn is_stative(&self) -> bool {
        matches!(self, VerbClass::State)
    }

    /// Returns true if this verb class is durative (+durative)
    pub fn is_durative(&self) -> bool {
        matches!(
            self,
            VerbClass::State | VerbClass::Activity | VerbClass::Accomplishment
        )
    }

    /// Returns true if this verb class is telic (+telic)
    pub fn is_telic(&self) -> bool {
        matches!(self, VerbClass::Accomplishment | VerbClass::Achievement)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Time {
    Past,
    Present,
    Future,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    Simple,
    Progressive,
    Perfect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Definiteness {
    Definite,
    Indefinite,
    Proximal,
    Distal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Number {
    Singular,
    Plural,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbEntry {
    pub lemma: String,
    pub time: Time,
    pub aspect: Aspect,
    pub class: VerbClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbMetadata {
    pub lemma: &'static str,
    pub class: VerbClass,
    pub time: Time,
    pub aspect: Aspect,
    pub features: &'static [Feature],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NounMetadata {
    pub lemma: &'static str,
    pub number: Number,
    pub features: &'static [Feature],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdjectiveMetadata {
    pub lemma: &'static str,
    pub features: &'static [Feature],
}

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

            if last == second_last && !"aeiou".contains(last) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_from_str_ditransitive() {
        assert_eq!(
            Feature::from_str("Ditransitive"),
            Some(Feature::Ditransitive)
        );
    }

    #[test]
    fn feature_from_str_subject_control() {
        assert_eq!(
            Feature::from_str("SubjectControl"),
            Some(Feature::SubjectControl)
        );
    }

    #[test]
    fn feature_from_str_opaque() {
        assert_eq!(Feature::from_str("Opaque"), Some(Feature::Opaque));
    }

    #[test]
    fn feature_from_str_unknown() {
        assert_eq!(Feature::from_str("Unknown"), None);
    }

    #[test]
    fn irregular_past_ran() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("ran").unwrap();
        assert_eq!(entry.lemma, "Run");
        assert_eq!(entry.time, Time::Past);
        assert_eq!(entry.aspect, Aspect::Simple);
    }

    #[test]
    fn irregular_progressive_running() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("running").unwrap();
        assert_eq!(entry.lemma, "Run");
        assert_eq!(entry.time, Time::None);
        assert_eq!(entry.aspect, Aspect::Progressive);
    }

    #[test]
    fn regular_past_jumped() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("jumped").unwrap();
        assert_eq!(entry.lemma, "Jump");
        assert_eq!(entry.time, Time::Past);
    }

    #[test]
    fn regular_present_runs() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("runs").unwrap();
        assert_eq!(entry.lemma, "Run");
        assert_eq!(entry.time, Time::Present);
    }

    #[test]
    fn present_silent_e_hopes() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("hopes").unwrap();
        assert_eq!(entry.lemma, "Hope");
        assert_eq!(entry.time, Time::Present);
    }

    #[test]
    fn present_silent_e_decides() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("decides").unwrap();
        assert_eq!(entry.lemma, "Decide");
        assert_eq!(entry.time, Time::Present);
    }

    #[test]
    fn present_silent_e_convinces() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("convinces").unwrap();
        assert_eq!(entry.lemma, "Convince");
        assert_eq!(entry.time, Time::Present);
    }

    #[test]
    fn past_silent_e_decided() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("decided").unwrap();
        assert_eq!(entry.lemma, "Decide");
        assert_eq!(entry.time, Time::Past);
    }

    #[test]
    fn regular_progressive_jumping() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("jumping").unwrap();
        assert_eq!(entry.lemma, "Jump");
        assert_eq!(entry.aspect, Aspect::Progressive);
    }

    #[test]
    fn regular_present_barks() {
        let lex = Lexicon::new();
        let entry = lex.lookup_verb("barks").unwrap();
        assert_eq!(entry.lemma, "Bark");
        assert_eq!(entry.time, Time::Present);
    }

    #[test]
    fn verb_db_returns_metadata_with_features() {
        let meta = lookup_verb_db("give").unwrap();
        assert_eq!(meta.lemma, "Give");
        assert_eq!(meta.class, VerbClass::Achievement);
        assert!(meta.features.contains(&Feature::Ditransitive));
    }

    #[test]
    fn verb_db_irregular_past() {
        let meta = lookup_verb_db("ran").unwrap();
        assert_eq!(meta.lemma, "Run");
        assert_eq!(meta.time, Time::Past);
    }

    #[test]
    fn verb_db_opaque_verb_has_feature() {
        let meta = lookup_verb_db("seek").unwrap();
        assert_eq!(meta.lemma, "Seek");
        assert!(meta.features.contains(&Feature::Opaque));
    }

    #[test]
    fn noun_db_returns_metadata() {
        let meta = lookup_noun_db("dog").unwrap();
        assert_eq!(meta.lemma, "Dog");
        assert_eq!(meta.number, Number::Singular);
    }

    #[test]
    fn noun_db_plural_form() {
        let meta = lookup_noun_db("men").unwrap();
        assert_eq!(meta.lemma, "Man");
        assert_eq!(meta.number, Number::Plural);
    }

    #[test]
    fn noun_db_proper_name_has_features() {
        let meta = lookup_noun_db("john").unwrap();
        assert_eq!(meta.lemma, "John");
        assert!(meta.features.contains(&Feature::Proper));
        assert!(meta.features.contains(&Feature::Masculine));
    }

    #[test]
    fn adjective_db_returns_metadata() {
        let meta = lookup_adjective_db("fake").unwrap();
        assert_eq!(meta.lemma, "Fake");
        assert!(meta.features.contains(&Feature::NonIntersective));
    }

    #[test]
    fn adjective_db_gradable() {
        let meta = lookup_adjective_db("tall").unwrap();
        assert_eq!(meta.lemma, "Tall");
        assert!(meta.features.contains(&Feature::Gradable));
    }
}
