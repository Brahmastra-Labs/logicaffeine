use rand::seq::SliceRandom;
use serde::Deserialize;
use std::collections::HashMap;

const LEXICON_JSON: &str = include_str!("../assets/lexicon.json");

#[derive(Deserialize, Debug)]
pub struct LexiconData {
    pub nouns: Vec<NounEntry>,
    pub verbs: Vec<VerbEntry>,
    pub adjectives: Vec<AdjectiveEntry>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NounEntry {
    pub lemma: String,
    #[serde(default)]
    pub forms: HashMap<String, String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub sort: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct VerbEntry {
    pub lemma: String,
    pub class: String,
    #[serde(default)]
    pub forms: HashMap<String, String>,
    #[serde(default)]
    pub features: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AdjectiveEntry {
    pub lemma: String,
    #[serde(default)]
    pub regular: bool,
    #[serde(default)]
    pub features: Vec<String>,
}

pub struct LexiconIndex {
    data: LexiconData,
}

impl LexiconIndex {
    pub fn new() -> Self {
        let data: LexiconData = serde_json::from_str(LEXICON_JSON)
            .expect("Failed to parse lexicon.json");
        Self { data }
    }

    pub fn proper_nouns(&self) -> Vec<&NounEntry> {
        self.data.nouns.iter()
            .filter(|n| n.features.iter().any(|f| f == "Proper"))
            .collect()
    }

    pub fn common_nouns(&self) -> Vec<&NounEntry> {
        self.data.nouns.iter()
            .filter(|n| !n.features.iter().any(|f| f == "Proper"))
            .collect()
    }

    pub fn nouns_with_feature(&self, feature: &str) -> Vec<&NounEntry> {
        self.data.nouns.iter()
            .filter(|n| n.features.iter().any(|f| f.eq_ignore_ascii_case(feature)))
            .collect()
    }

    pub fn nouns_with_sort(&self, sort: &str) -> Vec<&NounEntry> {
        self.data.nouns.iter()
            .filter(|n| n.sort.as_ref().map(|s| s.eq_ignore_ascii_case(sort)).unwrap_or(false))
            .collect()
    }

    pub fn verbs_with_feature(&self, feature: &str) -> Vec<&VerbEntry> {
        self.data.verbs.iter()
            .filter(|v| v.features.iter().any(|f| f.eq_ignore_ascii_case(feature)))
            .collect()
    }

    pub fn verbs_with_class(&self, class: &str) -> Vec<&VerbEntry> {
        self.data.verbs.iter()
            .filter(|v| v.class.eq_ignore_ascii_case(class))
            .collect()
    }

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

    pub fn transitive_verbs(&self) -> Vec<&VerbEntry> {
        self.data.verbs.iter()
            .filter(|v| {
                v.features.iter().any(|f| f.eq_ignore_ascii_case("Transitive")) ||
                v.features.iter().any(|f| f.eq_ignore_ascii_case("Ditransitive"))
            })
            .collect()
    }

    pub fn adjectives_with_feature(&self, feature: &str) -> Vec<&AdjectiveEntry> {
        self.data.adjectives.iter()
            .filter(|a| a.features.iter().any(|f| f.eq_ignore_ascii_case(feature)))
            .collect()
    }

    pub fn intersective_adjectives(&self) -> Vec<&AdjectiveEntry> {
        self.adjectives_with_feature("Intersective")
    }

    pub fn random_proper_noun(&self, rng: &mut impl rand::Rng) -> Option<&NounEntry> {
        self.proper_nouns().choose(rng).copied()
    }

    pub fn random_common_noun(&self, rng: &mut impl rand::Rng) -> Option<&NounEntry> {
        self.common_nouns().choose(rng).copied()
    }

    pub fn random_verb(&self, rng: &mut impl rand::Rng) -> Option<&VerbEntry> {
        self.data.verbs.choose(rng)
    }

    pub fn random_intransitive_verb(&self, rng: &mut impl rand::Rng) -> Option<&VerbEntry> {
        self.intransitive_verbs().choose(rng).copied()
    }

    pub fn random_transitive_verb(&self, rng: &mut impl rand::Rng) -> Option<&VerbEntry> {
        self.transitive_verbs().choose(rng).copied()
    }

    pub fn random_adjective(&self, rng: &mut impl rand::Rng) -> Option<&AdjectiveEntry> {
        self.data.adjectives.choose(rng)
    }

    pub fn random_intersective_adjective(&self, rng: &mut impl rand::Rng) -> Option<&AdjectiveEntry> {
        self.intersective_adjectives().choose(rng).copied()
    }
}

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
